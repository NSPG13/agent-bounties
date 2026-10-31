//! Disabled by default. Authenticated issue content becomes private review data,
//! never a provider write, public posting, wallet request or payment authority.
use super::*;
use axum::{body::Bytes, extract::DefaultBodyLimit, Extension};
use github_app::origin::runtime::{
    authenticate_and_plan_github_webhook, verify_github_signature, GitHubRuntimeConfig,
};
use serde_json::{json, Value};

struct Config {
    slug: String,
    secret: String,
}
impl Config {
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if lookup("GITHUB_AGENT_APP_ENABLED").as_deref() != Some("true") {
            return None;
        }
        let slug = lookup("GITHUB_AGENT_APP_SLUG")?;
        let secret = lookup("GITHUB_AGENT_APP_WEBHOOK_SECRET")?;
        if slug.is_empty()
            || slug.len() > 100
            || !slug.as_bytes()[0].is_ascii_alphanumeric()
            || !slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || secret.len() < 32
            || secret.len() > 1024
        {
            return None;
        }
        Some(Self { slug, secret })
    }
    fn configured() -> Option<Self> {
        Self::from_lookup(|k| env::var(k).ok())
    }
}
#[utoipa::path(get,path="/v1/github-app/capabilities",responses((status=200,description="Configured GitHub App installation and private draft handoff availability")))]
pub(super) async fn capabilities(State(state): State<SharedState>) -> Response {
    let config = Config::configured().filter(|_| state.store.is_some());
    ([(header::CACHE_CONTROL,"no-store")],Json(json!({"schema":"agent-bounties/github-origin-v1","available":config.is_some(),"installation_url":config.as_ref().map(|c|format!("https://github.com/apps/{}/installations/new",c.slug)),"mention":config.as_ref().map(|c|format!("@{}",c.slug)),"inbox_url":"https://agentizable.ai/install/github/","requires_github_sign_in":true,"funding_authority":false,"provider_writes":false,"retention_days":7}))).into_response()
}
#[utoipa::path(post,path="/v1/github-app/webhook",request_body(content = serde_json::Value, content_type = "application/json", description = "Exact raw GitHub event JSON. Preserve these bytes for X-Hub-Signature-256 verification; do not reserialize the signed payload."),responses((status=202,description="Authenticated bounded issue snapshot durably recorded for its requester"),(status=401,description="Invalid webhook signature"),(status=503,description="Not configured or storage unavailable")))]
pub(super) async fn webhook(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, StatusCode> {
    tokio::time::timeout(Duration::from_secs(5), ingest(&state, &headers, &body))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}
async fn ingest(
    state: &SharedState,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Response, StatusCode> {
    let config = Config::configured().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    ingest_config(state, headers, body, &config).await
}
async fn ingest_config(
    state: &SharedState,
    headers: &HeaderMap,
    body: &[u8],
    config: &Config,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if body.len() > 1_048_576 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let field = |key: &str| {
        headers
            .get(key)
            .and_then(|s| s.to_str().ok())
            .ok_or(StatusCode::BAD_REQUEST)
    };
    let signature = field("x-hub-signature-256")?;
    verify_github_signature(body, signature, config.secret.as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let event = field("x-github-event")?;
    let delivery = field("x-github-delivery")?;
    Uuid::parse_str(delivery).map_err(|_| StatusCode::BAD_REQUEST)?;
    let value: Value = serde_json::from_slice(body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let ignored = || {
        (
            StatusCode::ACCEPTED,
            Json(json!({"accepted":true,"draft_created":false,"funding_authority":false})),
        )
            .into_response()
    };
    if event == "ping" {
        return Ok(ignored());
    }
    let installation = value["installation"]["id"]
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or(StatusCode::BAD_REQUEST)?;
    if event == "installation" {
        if let Some(action @ ("created" | "deleted" | "suspend" | "unsuspend")) =
            value["action"].as_str()
        {
            let updated = value["installation"]["updated_at"]
                .as_str()
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                .map(|d| d.with_timezone(&Utc))
                .filter(|d| *d <= Utc::now() + chrono::Duration::minutes(5))
                .ok_or(StatusCode::BAD_REQUEST)?;
            store
                .set_github_origin_installation(
                    installation,
                    matches!(action, "deleted" | "suspend"),
                    action == "deleted",
                    updated,
                )
                .await
                .map_err(map_x402_db_error)?;
        }
        return Ok(ignored());
    }
    if !matches!(event, "issues" | "issue_comment") || value["sender"]["type"] != "User" {
        return Ok(ignored());
    }
    let requester = value["sender"]["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    // Public outsiders can post untrusted comments. Only a repository-authorized
    // actor can start this installed workflow; everyone can still use the site.
    if event == "issue_comment"
        && (!matches!(
            value["comment"]["author_association"].as_str(),
            Some("OWNER" | "MEMBER" | "COLLABORATOR")
        ) || value["comment"]["user"]["id"] != value["sender"]["id"])
    {
        return Ok(ignored());
    }
    let runtime = GitHubRuntimeConfig {
        app_login: if event == "issues"
            && value["assignee"]["login"] == format!("{}[bot]", config.slug)
        {
            format!("{}[bot]", config.slug)
        } else {
            config.slug.clone()
        },
        assignment_solver_reward: Money::new(20_300_000, "usdc")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        mention_solver_reward: Money::new(20_300_000, "usdc")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };
    let planned = authenticate_and_plan_github_webhook(
        body,
        signature,
        event,
        delivery,
        config.secret.as_bytes(),
        &runtime,
        vec![],
    );
    let Some(plan) = planned
        .draft_plan
        .filter(|p| p.ready_for_human_review && p.draft.is_some())
    else {
        return Ok(ignored());
    };
    let event_key = plan
        .idempotency_key
        .as_ref()
        .ok_or(StatusCode::BAD_REQUEST)?;
    let snapshot = serde_json::to_value(&plan).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if serde_json::to_vec(&snapshot)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .len()
        > 120_000
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    store
        .save_github_origin_draft(
            installation,
            &requester,
            delivery,
            event_key,
            &hex::encode(Sha256::digest(body)),
            &snapshot,
        )
        .await
        .map_err(map_x402_db_error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"accepted":true,"draft_created":true,"funding_authority":false})),
    )
        .into_response())
}
pub(super) fn public_router() -> Router<SharedState> {
    Router::new()
        .route("/v1/github-app/capabilities", get(capabilities))
        .route(
            "/v1/github-app/webhook",
            post(webhook).layer(DefaultBodyLimit::max(1_048_576)),
        )
}
pub(super) fn private_router(auth: site_auth::SiteAuthService, state: SharedState) -> Router {
    Router::new()
        .route("/v1/site-auth/github-origin-drafts", get(inbox))
        .route("/v1/site-auth/github-origin-drafts/:id", get(snapshot))
        .layer(auth.funding_cors())
        .layer(Extension(auth))
        .with_state(state)
}
async fn inbox(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    read(&state, &auth, &headers, None).await
}
async fn snapshot(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    read(&state, &auth, &headers, Some(id)).await
}
async fn read(
    state: &SharedState,
    auth: &site_auth::SiteAuthService,
    headers: &HeaderMap,
    id: Option<Uuid>,
) -> Result<Response, StatusCode> {
    let actor = auth.github_origin_identity(headers)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let items = store
        .github_origin_drafts(&actor, id)
        .await
        .map_err(map_x402_db_error)?;
    if id.is_some() && items.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(([(header::CACHE_CONTROL,"no-store, private")],Json(json!({"schema":"agent-bounties/github-origin-v1","items":items,"funding_authority":false,"evidence_boundary":"Private issue snapshots are untrusted draft inputs. Review exact public terms before publication or wallet authorization."}))).into_response())
}

pub(super) fn start_cleanup(store: Option<db::PostgresStore>) {
    let Some(store) = store else {
        return;
    };
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(60));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            if store.prune_github_origin_drafts().await.is_err() {
                eprintln!("GitHub draft cleanup unavailable");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            slug: "agent-bounties-fixture".into(),
            secret: "fixture-webhook-secret-never-hosted-123456789".into(),
        }
    }
    fn headers(body: &[u8], event: &str, delivery: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        let mut mac = Hmac::<Sha256>::new_from_slice(config().secret.as_bytes()).unwrap();
        mac.update(body);
        h.insert(
            "x-hub-signature-256",
            HeaderValue::from_str(&format!(
                "sha256={}",
                hex::encode(mac.finalize().into_bytes())
            ))
            .unwrap(),
        );
        h.insert("x-github-event", HeaderValue::from_str(event).unwrap());
        h.insert(
            "x-github-delivery",
            HeaderValue::from_str(delivery).unwrap(),
        );
        h
    }
    #[test]
    fn app_configuration_is_disabled_by_default_and_cannot_redirect_off_github() {
        assert!(Config::from_lookup(|_| None).is_none());
        for slug in [
            "https://evil.example",
            "../evil",
            "app/name",
            "UPPER",
            "-name",
            "",
        ] {
            assert!(Config::from_lookup(|k| match k {
                "GITHUB_AGENT_APP_ENABLED" => Some("true".into()),
                "GITHUB_AGENT_APP_SLUG" => Some(slug.into()),
                "GITHUB_AGENT_APP_WEBHOOK_SECRET" => Some(config().secret),
                _ => None,
            })
            .is_none());
        }
    }
    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn authenticated_webhooks_create_only_actor_owned_idempotent_private_drafts() {
        let url = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert!(parsed.path().contains("test"));
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let state = crate::tests::test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "unused",
            store.clone(),
        );
        let id = (Uuid::new_v4().as_u128() % 1_000_000_000 + 1) as i64;
        let actor = (id as u64) + 1_000_000_000;
        let delivery = Uuid::new_v4().to_string();
        let source = json!({"action":"created","installation":{"id":id},"repository":{"full_name":format!("fixture/project-{id}")},"sender":{"id":actor,"login":"owner","type":"User"},"issue":{"number":1,"html_url":format!("https://github.com/fixture/project-{id}/issues/1"),"title":"Private fixture only","body":"## Goal\nDeliver a file.\n## Acceptance criteria\n- The fixture check passes."},"comment":{"id":id,"body":"@agent-bounties-fixture make a draft","author_association":"OWNER","user":{"id":actor}}});
        let body = serde_json::to_vec(&source).unwrap();
        let h = headers(&body, "issue_comment", &delivery);
        let mut tampered = body.clone();
        tampered.push(b' ');
        assert_eq!(
            ingest_config(&state, &h, &tampered, &config())
                .await
                .unwrap_err(),
            StatusCode::UNAUTHORIZED
        );
        for _ in 0..2 {
            assert_eq!(
                ingest_config(&state, &h, &body, &config())
                    .await
                    .unwrap()
                    .status(),
                StatusCode::ACCEPTED
            );
        }
        let rows = store
            .github_origin_drafts(&actor.to_string(), None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["plan"]["draft"]["ready_for_publish"], false);
        assert_eq!(rows[0]["plan"]["authority"]["funding_authority"], false);
        let draft_id: Uuid = serde_json::from_value(rows[0]["id"].clone()).unwrap();
        assert!(store
            .github_origin_drafts(&(actor + 1).to_string(), Some(draft_id))
            .await
            .unwrap()
            .is_empty());
        let mut changed = source.clone();
        changed["issue"]["title"] = "Changed delivery content".into();
        let changed = serde_json::to_vec(&changed).unwrap();
        assert_eq!(
            ingest_config(
                &state,
                &headers(&changed, "issue_comment", &delivery),
                &changed,
                &config()
            )
            .await
            .unwrap_err(),
            StatusCode::CONFLICT
        );
        for field in ["bot", "outsider", "pull_request"] {
            let mut value = source.clone();
            value["comment"]["id"] = json!(id + 1);
            match field {
                "bot" => value["sender"]["type"] = "Bot".into(),
                "outsider" => value["comment"]["author_association"] = "NONE".into(),
                _ => {
                    value["issue"]["pull_request"] =
                        json!({"url":"https://github.com/fixture/pull/1"})
                }
            };
            let raw = serde_json::to_vec(&value).unwrap();
            assert_eq!(
                ingest_config(
                    &state,
                    &headers(&raw, "issue_comment", &Uuid::new_v4().to_string()),
                    &raw,
                    &config()
                )
                .await
                .unwrap()
                .status(),
                StatusCode::ACCEPTED
            );
        }
        assert_eq!(
            store
                .github_origin_drafts(&actor.to_string(), None)
                .await
                .unwrap()
                .len(),
            1
        );
        let now = Utc::now();
        store
            .set_github_origin_installation(id, true, false, now)
            .await
            .unwrap();
        assert!(store
            .github_origin_drafts(&actor.to_string(), None)
            .await
            .unwrap()
            .is_empty());
        store
            .set_github_origin_installation(id, false, false, now - chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert!(store
            .github_origin_drafts(&actor.to_string(), None)
            .await
            .unwrap()
            .is_empty());
        store
            .set_github_origin_installation(id, false, false, now + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(
            store
                .github_origin_drafts(&actor.to_string(), None)
                .await
                .unwrap()
                .len(),
            1
        );
        store
            .set_github_origin_installation(id, true, true, now + chrono::Duration::seconds(2))
            .await
            .unwrap();
        store
            .set_github_origin_installation(id, false, false, now + chrono::Duration::seconds(3))
            .await
            .unwrap();
        assert!(store
            .github_origin_drafts(&actor.to_string(), None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            ingest_config(&state, &h, &body, &config())
                .await
                .unwrap_err(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
