//! Consented, minimal offline conversion measurement. No wallet/payment authority.
use crate::{require_operator, site_analytics_origin_allowed, SharedState};
use axum::{
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use db::google_ads::{Delivery, NewClick};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{env, time::Duration};
use utoipa::ToSchema;
use uuid::Uuid;

const CONSENT: &str = "google-ads-outcomes-v1";
const INGEST: &str = "https://datamanager.googleapis.com/v1/events:ingest";
pub(crate) fn router() -> Router<SharedState> {
    Router::new()
        .route("/v1/distribution/website-acquisitions", post(acquire))
        .route("/v1/distribution/website-handoffs", post(handoff))
        .route("/v1/distribution/website-consent/revoke", post(revoke))
        .route("/v1/operator/google-ads/report", get(report))
        .route("/v1/operator/google-ads/costs", put(cost))
        .layer(DefaultBodyLimit::max(4096))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcquisitionRequest {
    nonce: String,
    acquisition: Option<String>,
    identifier_kind: String,
    click_id: String,
    campaign: String,
    first_touch_source: Option<String>,
    consent_version: String,
    consent_granted: bool,
    canary_kind: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandoffRequest {
    acquisition: String,
    operation_id: Uuid,
    stage: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RevokeRequest {
    acquisition: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CostRequest {
    campaign: String,
    day: NaiveDate,
    cost_micros: i64,
    clicks: i64,
}
#[derive(Deserialize)]
pub(crate) struct ReportQuery {
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn valid_click(kind: &str, value: &str) -> bool {
    matches!(kind, "gclid" | "gbraid" | "wbraid")
        && (10..=512).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-~".contains(&b))
}
fn allowed(headers: &HeaderMap) -> Result<(), StatusCode> {
    if !site_analytics_origin_allowed(headers)
        || headers.get("sec-gpc").is_some_and(|h| h == "1")
        || headers.get("dnt").is_some_and(|h| h == "1")
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
fn signing_secret(state: &SharedState) -> Result<&str, StatusCode> {
    state
        .distribution_attribution_signing_secret
        .as_deref()
        .filter(|s| s.len() >= 32)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
fn private_operator(state: &SharedState, headers: &HeaderMap) -> Result<(), StatusCode> {
    // Generic local-development routes may allow an unset operator secret.
    // Private advertising reports and cost writes must always fail closed.
    if state
        .operator_api_token
        .as_deref()
        .is_none_or(|token| token.trim().is_empty())
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    require_operator(state, headers)
}
fn encryption_key() -> Result<Vec<u8>, StatusCode> {
    let key = hex::decode(env::var("GOOGLE_ADS_MEASUREMENT_KEY").unwrap_or_default())
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if key.len() != 32 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(key)
}
fn encrypt(key: &[u8], value: &str) -> Result<String, StatusCode> {
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    );
    let mut nonce = [0; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut data = value.as_bytes().to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(CONSENT.as_bytes()),
        &mut data,
    )
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(format!("{}.{}", hex::encode(nonce), hex::encode(data)))
}
fn decrypt(key: &[u8], ciphertext: &str) -> Result<String, StatusCode> {
    let (nonce, data) = ciphertext.split_once('.').ok_or(StatusCode::BAD_REQUEST)?;
    let nonce: [u8; 12] = hex::decode(nonce)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .try_into()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut data = hex::decode(data).map_err(|_| StatusCode::BAD_REQUEST)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let plain = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(CONSENT.as_bytes()),
            &mut data,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    String::from_utf8(plain.to_vec()).map_err(|_| StatusCode::BAD_REQUEST)
}
async fn acquisition_id(state: &SharedState, value: &str) -> Result<Uuid, StatusCode> {
    let hash = db::distribution_acquisition_token_hash(value, signing_secret(state)?)
        .ok_or(StatusCode::BAD_REQUEST)?;
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .google_ads_acquisition(&hash)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)
}

#[utoipa::path(post,path="/v1/distribution/website-acquisitions",request_body=AcquisitionRequest,responses((status=200,description="Private opaque acquisition, no wallet authority"),(status=403,description="Consent or origin denied"),(status=503,description="Measurement disabled or unavailable")))]
pub(crate) async fn acquire(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(r): Json<AcquisitionRequest>,
) -> Result<Json<Value>, StatusCode> {
    allowed(&headers)?;
    if env::var("GOOGLE_ADS_MEASUREMENT_ENABLED").as_deref() != Ok("true") {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if !r.consent_granted || r.consent_version != CONSENT {
        return Err(StatusCode::FORBIDDEN);
    }
    if !valid_click(&r.identifier_kind, &r.click_id)
        || !token(&r.campaign)
        || r.nonce.len() != 64
        || !r
            .nonce
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || r.canary_kind
            .as_deref()
            .is_some_and(|s| !matches!(s, "dry-run-v1" | "mainnet-v1"))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let key = encryption_key()?;
    let secret = signing_secret(&state)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let now = Utc::now();
    let (acquisition, id) = if let Some(existing) = r.acquisition {
        let id = acquisition_id(&state, &existing).await?;
        if let Some(kind) = r.canary_kind.as_deref() {
            if !store
                .google_ads_canary_matches(id, kind)
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            {
                return Err(StatusCode::CONFLICT);
            }
        }
        (existing, id)
    } else {
        let acquisition = db::sign_distribution_acquisition_token(&r.nonce, secret)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let hash = db::distribution_acquisition_token_hash(&acquisition, secret)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let first = r.first_touch_source.as_deref().unwrap_or("google-ads");
        let first = db::normalize_distribution_rail(first).unwrap_or("website");
        let record = store
            .observe_distribution_acquisition(first, &hash, r.canary_kind.as_deref(), now)
            .await
            .map_err(|_| StatusCode::CONFLICT)?;
        (acquisition, record.id)
    };
    let encrypted = encrypt(&key, &r.click_id)?;
    let hash = hex::encode(
        ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key),
            format!("{}:{}", r.identifier_kind, r.click_id).as_bytes(),
        )
        .as_ref(),
    );
    let recorded = store
        .google_ads_record_click(NewClick {
            acquisition_id: id,
            hash: &hash,
            kind: &r.identifier_kind,
            ciphertext: &encrypted,
            campaign: &r.campaign,
            now,
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let expires_at = recorded.ok_or(StatusCode::CONFLICT)?;
    Ok(Json(
        json!({"schema_version":"agent-bounties/website-acquisition-v1","acquisition":acquisition,"consent_version":CONSENT,"expires_at":expires_at}),
    ))
}

#[utoipa::path(post,path="/v1/distribution/website-handoffs",request_body=HandoffRequest,responses((status=200,description="Retry-safe private posting attribution")))]
pub(crate) async fn handoff(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(r): Json<HandoffRequest>,
) -> Result<Json<Value>, StatusCode> {
    allowed(&headers)?;
    if !matches!(
        r.stage.as_str(),
        "draft" | "login" | "wallet" | "wallet_review" | "funding_started"
    ) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let id = acquisition_id(&state, &r.acquisition).await?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !store
        .google_ads_consent_active(id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    {
        return Err(StatusCode::GONE);
    }
    let fingerprint = hex::encode(Sha256::digest(format!(
        "google-ads-posting-operation:{}",
        r.operation_id
    )));
    let reserved = store
        .reserve_distribution_handoff(id, &fingerprint, Utc::now())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .google_ads_funnel(id, r.operation_id, &r.stage)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(
        json!({"acquisition":r.acquisition,"handoff":reserved.id}),
    ))
}

#[utoipa::path(post,path="/v1/distribution/website-consent/revoke",request_body=RevokeRequest,responses((status=200,description="Identifiers erased and pending exports suppressed")))]
pub(crate) async fn revoke(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(r): Json<RevokeRequest>,
) -> Result<Json<Value>, StatusCode> {
    // Withdrawal remains available with GPC/DNT enabled and capture disabled.
    if !site_analytics_origin_allowed(&headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    let id = acquisition_id(&state, &r.acquisition).await?;
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .google_ads_revoke(id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({"revoked":true})))
}

#[utoipa::path(get,path="/v1/operator/google-ads/report",responses((status=200,description="Private campaign outcomes, costs, coverage limitations and delivery status")))]
pub(crate) async fn report(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<ReportQuery>,
) -> Result<Json<Value>, StatusCode> {
    private_operator(&state, &headers)?;
    let end = q.end.unwrap_or_else(Utc::now);
    let start = q.start.unwrap_or(end - chrono::Duration::days(30));
    if end <= start || end - start > chrono::Duration::days(366) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let value = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .google_ads_report(start, end)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(value))
}
#[utoipa::path(put,path="/v1/operator/google-ads/costs",request_body=CostRequest,responses((status=200,description="One verified exported campaign-day replaced idempotently")))]
pub(crate) async fn cost(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(r): Json<CostRequest>,
) -> Result<Json<Value>, StatusCode> {
    private_operator(&state, &headers)?;
    if !token(&r.campaign) || r.cost_micros < 0 || r.clicks < 0 || r.day > Utc::now().date_naive() {
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .google_ads_cost(&r.campaign, r.day, r.cost_micros, r.clicks)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({"recorded":true})))
}

fn event(item: &Delivery, click: &str) -> Value {
    json!({"adIdentifiers":{item.kind.clone():click},"eventTimestamp":item.event_at.to_rfc3339(),"transactionId":item.id.to_string(),"eventSource":"WEB","consent":{"adUserData":"CONSENT_GRANTED","adPersonalization":"CONSENT_DENIED"}})
}

async fn access_token(client: &reqwest::Client) -> Result<String, &'static str> {
    let client_id = env::var("GOOGLE_ADS_OAUTH_CLIENT_ID").map_err(|_| "oauth_not_configured")?;
    let client_secret =
        env::var("GOOGLE_ADS_OAUTH_CLIENT_SECRET").map_err(|_| "oauth_not_configured")?;
    let refresh = env::var("GOOGLE_ADS_OAUTH_REFRESH_TOKEN").map_err(|_| "oauth_not_configured")?;
    let response = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token".to_string()),
        ])
        .send()
        .await
        .map_err(|_| "oauth_transport")?;
    if !response.status().is_success() {
        return Err("oauth_rejected");
    }
    let value: Value = response.json().await.map_err(|_| "oauth_response")?;
    value["access_token"]
        .as_str()
        .map(str::to_owned)
        .ok_or("oauth_response")
}

async fn deliver(
    store: &db::PostgresStore,
    client: &reqwest::Client,
    item: &Delivery,
) -> Result<(), &'static str> {
    let customer = env::var("GOOGLE_ADS_CUSTOMER_ID").map_err(|_| "destination_missing")?;
    let action = env::var(if item.outcome == "external_bounty_funded" {
        "GOOGLE_ADS_FUNDED_ACTION_ID"
    } else {
        "GOOGLE_ADS_PAID_ACTION_ID"
    })
    .map_err(|_| "destination_missing")?;
    if customer.len() != 10
        || !customer.bytes().all(|b| b.is_ascii_digit())
        || action.is_empty()
        || !action.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("destination_invalid");
    }
    let key = encryption_key().map_err(|_| "encryption_unavailable")?;
    let click = decrypt(&key, &item.ciphertext).map_err(|_| "ciphertext_invalid")?;
    let bearer = access_token(client).await?;
    if !store
        .google_ads_pin_destination(item, &customer, &action)
        .await
        .map_err(|_| "store_unavailable")?
    {
        return Err("destination_changed");
    }
    if !store
        .google_ads_delivery_valid(item)
        .await
        .map_err(|_| "store_unavailable")?
    {
        return Err("no_longer_eligible");
    }
    let validate_only = env::var("GOOGLE_ADS_EXPORT_ENABLED").as_deref() != Ok("true");
    let body = json!({"destinations":[{"operatingAccount":{"accountType":"GOOGLE_ADS","accountId":customer},"productDestinationId":action}],"encoding":"HEX","events":[event(item,&click)],"validateOnly":validate_only});
    let response = client
        .post(INGEST)
        .bearer_auth(bearer)
        .json(&body)
        .send()
        .await
        .map_err(|_| "google_transport")?;
    let status = response.status();
    if !status.is_success() {
        let retry = status.as_u16() == 429 || status.is_server_error() || status.as_u16() == 401;
        store
            .google_ads_complete(
                item,
                if retry { "pending" } else { "rejected" },
                Some(if retry {
                    "google_retryable"
                } else {
                    "google_rejected"
                }),
                None,
            )
            .await
            .map_err(|_| "store_unavailable")?;
        return Ok(());
    }
    let response: Value = response.json().await.map_err(|_| "google_response")?;
    let request = response["requestId"].as_str().filter(|v| {
        v.len() <= 256
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    });
    if !validate_only && request.is_none() {
        return Err("google_request_id_missing");
    }
    store
        .google_ads_complete(
            item,
            if validate_only { "pending" } else { "accepted" },
            if validate_only {
                Some("validation_only")
            } else {
                None
            },
            request,
        )
        .await
        .map_err(|_| "store_unavailable")?;
    Ok(())
}

fn processing_summary(value: &Value) -> (&'static str, i32, i32, Value) {
    let mut status = "REQUEST_STATUS_UNKNOWN";
    let mut errors = 0i32;
    let mut warnings = 0i32;
    let mut reasons = Vec::new();
    // This worker submits exactly one event to one destination per request.
    if let Some(rows) = value["requestStatusPerDestination"]
        .as_array()
        .filter(|r| r.len() == 1)
    {
        let row = &rows[0];
        status = match row["requestStatus"].as_str() {
            Some("SUCCESS") => "SUCCESS",
            Some("PARTIAL_SUCCESS") => "PARTIAL_SUCCESS",
            Some("FAILED") => "FAILED",
            Some("PROCESSING") => "PROCESSING",
            _ => "REQUEST_STATUS_UNKNOWN",
        };
        for (group, key, total) in [
            ("errorInfo", "errorCounts", &mut errors),
            ("warningInfo", "warningCounts", &mut warnings),
        ] {
            for entry in row[group][key].as_array().into_iter().flatten().take(100) {
                let count = entry["recordCount"]
                    .as_str()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(0)
                    .max(0);
                *total = total.saturating_add(count);
                if let Some(reason) = entry["reason"].as_str().filter(|s| {
                    s.len() <= 128 && s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
                }) {
                    reasons.push(json!({"reason":reason,"count":count}));
                }
            }
        }
    }
    (status, errors, warnings, json!(reasons))
}

async fn poll_diagnostics(store: &db::PostgresStore, client: &reqwest::Client) {
    let Ok(due) = store.google_ads_diagnostics_due().await else {
        return;
    };
    if due.is_empty() {
        return;
    }
    let token = access_token(client).await;
    for (id, request) in due {
        let mut summary = ("DIAGNOSTICS_UNAVAILABLE", 0, 0, json!([]));
        if let Ok(bearer) = &token {
            if let Ok(response) = client
                .get("https://datamanager.googleapis.com/v1/requestStatus:retrieve")
                .query(&[("requestId", request)])
                .bearer_auth(bearer)
                .send()
                .await
            {
                if response.status().is_success() {
                    if let Ok(value) = response.json::<Value>().await {
                        summary = processing_summary(&value);
                    }
                }
            }
        }
        let _ = store
            .google_ads_diagnostics_save(id, summary.0, summary.1, summary.2, summary.3)
            .await;
    }
}

pub(crate) fn start(store: Option<db::PostgresStore>) {
    let Some(store) = store else { return };
    tokio::spawn(async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        else {
            return;
        };
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            if store.google_ads_reconcile().await.is_err() {
                eprintln!("google_ads_measurement: reconciliation_unavailable");
                continue;
            }
            // Validation is explicit too: never send even a dry-run without configuration.
            if env::var("GOOGLE_ADS_VALIDATE_ENABLED").as_deref() != Ok("true")
                && env::var("GOOGLE_ADS_EXPORT_ENABLED").as_deref() != Ok("true")
            {
                continue;
            }
            poll_diagnostics(&store, &client).await;
            for _ in 0..20 {
                let Ok(Some(item)) = store.google_ads_lease().await else {
                    break;
                };
                if let Err(code) = deliver(&store, &client, &item).await {
                    let _ = store
                        .google_ads_complete(&item, "pending", Some(code), None)
                        .await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn processing_is_not_campaign_credit_and_errors_are_minimized() {
        let value = json!({"requestStatusPerDestination":[{"requestStatus":"PARTIAL_SUCCESS","errorInfo":{"errorCounts":[{"reason":"PROCESSING_ERROR_REASON_CLICK_NOT_FOUND","recordCount":"1"},{"reason":"private data that must not be stored","recordCount":"0"}]},"warningInfo":{"warningCounts":[]}}]});
        let (state, errors, warnings, reasons) = processing_summary(&value);
        assert_eq!(state, "PARTIAL_SUCCESS");
        assert_eq!(errors, 1);
        assert_eq!(warnings, 0);
        assert_eq!(reasons.as_array().unwrap().len(), 1);
        assert_eq!(processing_summary(&json!({})).0, "REQUEST_STATUS_UNKNOWN");
    }
    #[test]
    fn private_clicks_are_encrypted_and_tamper_evident() {
        let key = [7; 32];
        let ciphertext = encrypt(&key, "gclid_fixture_123").unwrap();
        assert!(!ciphertext.contains("gclid_fixture"));
        assert_eq!(decrypt(&key, &ciphertext).unwrap(), "gclid_fixture_123");
        assert!(decrypt(&[8; 32], &ciphertext).is_err());
        assert_ne!(ciphertext, encrypt(&key, "gclid_fixture_123").unwrap());
    }
    #[test]
    fn google_payload_has_no_wallet_content_or_amount() {
        let item = Delivery {
            id: Uuid::new_v4(),
            lease_id: Uuid::new_v4(),
            kind: "gclid".into(),
            ciphertext: "private".into(),
            outcome: "external_bounty_funded".into(),
            event_at: Utc::now(),
        };
        let payload = event(&item, "gclid_fixture_123");
        assert_eq!(payload.as_object().unwrap().len(), 5);
        for field in [
            "userData",
            "userId",
            "conversionValue",
            "currency",
            "bounty_contract",
            "wallet",
        ] {
            assert!(payload.get(field).is_none());
        }
        assert_eq!(payload["consent"]["adPersonalization"], "CONSENT_DENIED");
        assert_eq!(event(&item, "gclid_fixture_123"), payload);
    }
    #[test]
    fn identifiers_and_campaigns_are_bounded() {
        assert!(valid_click("wbraid", "fixture_identifier_1"));
        assert!(!valid_click("email", "person@example.com"));
        assert!(!valid_click("gclid", "a?private=1234"));
        assert!(!token("a private title"));
        assert!(!token(&"x".repeat(65)));
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://agentbounties.app".parse().unwrap());
        assert!(allowed(&headers).is_ok());
        headers.insert("sec-gpc", "1".parse().unwrap());
        assert_eq!(allowed(&headers), Err(StatusCode::FORBIDDEN));
    }
}
