use crate::{secure_operations, security_auth, SharedState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use db::verification::Artifact;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
fn db(state: &SharedState) -> Result<&db::PostgresStore, StatusCode> {
    state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
fn err(e: db::verification::EvidenceError) -> StatusCode {
    match e {
        db::verification::EvidenceError::NotFound => StatusCode::NOT_FOUND,
        db::verification::EvidenceError::Conflict => StatusCode::CONFLICT,
        db::verification::EvidenceError::Invalid => StatusCode::BAD_REQUEST,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}
pub fn router() -> Router<SharedState> {
    Router::new()
        .route(
            "/v1/verification/artifacts/:id/disputes",
            post(open_dispute),
        )
        .route(
            "/v1/verification/disputes/:id/resolve",
            post(resolve_dispute),
        )
        .route("/v1/verification/artifacts/:id/participants", post(grant))
        .route(
            "/v1/verification/artifacts/:id/participants/revoke",
            post(revoke),
        )
        .route(
            "/v1/verification/artifacts/:id/prepare-submission",
            post(prepare),
        )
        .route(
            "/v1/verification/artifacts/:id/confirm-submission",
            post(confirm),
        )
        .route(
            "/v1/verification/bounties/:network/:contract/submissions",
            get(history),
        )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisputeReason {
    reason: String,
}
#[utoipa::path(post,path="/v1/verification/artifacts/{id}/disputes",request_body=Value,params(("id"=Uuid,Path,description="Artifact ID")),responses((status=200,description="Suspend cleanup during your dispute; existing hold is returned on retry"),(status=404,description="Only the owner or a granted participant may open a hold")))]
pub(crate) async fn open_dispute(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<DisputeReason>,
) -> Result<Json<Value>, StatusCode> {
    let key = secure_operations::authenticated(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    db(&state)?
        .open_artifact_dispute(id, &key, &input.reason)
        .await
        .map(Json)
        .map_err(err)
}
#[utoipa::path(post,path="/v1/verification/disputes/{id}/resolve",request_body=Value,params(("id"=Uuid,Path,description="Dispute ID")),responses((status=200,description="Withdraw your own hold with an explanation; other holds stay active"),(status=404,description="Dispute is not yours")))]
pub(crate) async fn resolve_dispute(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<DisputeReason>,
) -> Result<Json<Value>, StatusCode> {
    let key = secure_operations::authenticated(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    db(&state)?
        .resolve_artifact_dispute(id, &key, &input.reason)
        .await
        .map(Json)
        .map_err(err)
}
fn reference(a: &Artifact) -> String {
    format!("urn:agent-bounties:artifact:{}:{}", a.id, a.manifest_hash)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Submission {
    network: String,
    bounty_contract: String,
    solver_wallet: String,
    evidence: Value,
}
#[utoipa::path(post,path="/v1/verification/artifacts/{id}/prepare-submission",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Verify stored bytes before preparing an unsigned legacy submission"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn prepare(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Submission>,
) -> Result<Json<Value>, StatusCode> {
    let principal = secure_operations::authenticated(&state, &headers).await?;
    security_auth::authorize_verified_wallet(
        &state,
        &principal,
        &input.solver_wallet,
        &input.network,
    )
    .await?;
    let owner = principal.key().ok_or(StatusCode::UNAUTHORIZED)?;
    let artifact = db(&state)?.owned_artifact(id, &owner).await.map_err(err)?;
    if artifact.status != "ready" {
        return Err(StatusCode::CONFLICT);
    }
    let objects = worker::artifact_store::ArtifactStore::from_env()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    objects
        .get(id, &artifact.manifest)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let item =
        crate::indexed_autonomous_bounty(&state, &input.network, &input.bounty_contract).await?;
    let plan = chain_base::build_autonomous_submission_preparation(
        &crate::configured_autonomous_planner(&input.network)?,
        &input.network,
        &item,
        &input.solver_wallet,
        &reference(&artifact),
        input.evidence,
        chrono::Utc::now().timestamp() as u64,
    )
    .map_err(|_| StatusCode::CONFLICT)?;
    db(&state)?.prepare_artifact_scope(id,&owner,&json!({"network":input.network,"bounty_contract":item.bounty_contract,"round":plan.round,"solver":plan.solver})).await.map_err(err)?;
    Ok(Json(
        json!({"artifact":artifact,"submission_preparation":plan,"notice":"Stored files are private until you confirm publication after the on-chain submission. This plan does not submit or pay."}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Confirmation {
    network: String,
    bounty_contract: String,
    round: u64,
    publish_files: bool,
}
#[utoipa::path(post,path="/v1/verification/artifacts/{id}/confirm-submission",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Confirm exact artifact commitment against the indexed chain"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn confirm(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Confirmation>,
) -> Result<Json<Artifact>, StatusCode> {
    let principal = secure_operations::authenticated(&state, &headers).await?;
    let owner = principal.key().ok_or(StatusCode::UNAUTHORIZED)?;
    let artifact = db(&state)?.owned_artifact(id, &owner).await.map_err(err)?;
    let item =
        crate::indexed_autonomous_bounty(&state, &input.network, &input.bounty_contract).await?;
    let event = item
        .events
        .iter()
        .find(|e| {
            e.kind == chain_base::AutonomousBountyEventKind::SubmissionAdded
                && e.data["round"].as_u64() == Some(input.round)
        })
        .ok_or(StatusCode::CONFLICT)?;
    let solver = event.data["solver"].as_str().ok_or(StatusCode::CONFLICT)?;
    security_auth::authorize_verified_wallet(&state, &principal, solver, &input.network).await?;
    let expected = chain_base::sha256_utf8(&reference(&artifact));
    if event.data["submission_hash"].as_str() != Some(expected.as_str()) {
        return Err(StatusCode::CONFLICT);
    }
    let scope = json!({"network":input.network,"bounty_contract":item.bounty_contract.to_lowercase(),"bounty_id":item.bounty_id,"round":input.round,"event_id":event.id,"submission_hash":expected,"solver":solver,"visibility":if input.publish_files {"public"}else{"private"}});
    db(&state)?
        .confirm_artifact(id, &owner, &scope)
        .await
        .map(Json)
        .map_err(err)
}
pub async fn readable(
    state: &SharedState,
    id: Uuid,
    headers: &HeaderMap,
) -> Result<Artifact, StatusCode> {
    let principal = security_auth::principal(state, headers).await?;
    let artifact = db(state)?.artifact_internal(id).await.map_err(err)?;
    if let Some(key) = principal.key() {
        if key == artifact.owner_key
            || db(state)?
                .resource_is_authorized("verification_artifact", &id.to_string(), &key, "read")
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        {
            return Ok(artifact);
        }
    }
    if let Some(scope) = &artifact.canonical_scope {
        if scope["visibility"] == "public" {
            let item = crate::indexed_autonomous_bounty(
                state,
                scope["network"].as_str().ok_or(StatusCode::NOT_FOUND)?,
                scope["bounty_contract"]
                    .as_str()
                    .ok_or(StatusCode::NOT_FOUND)?,
            )
            .await?;
            if item.events.iter().any(|e| {
                e.id.to_string() == scope["event_id"].as_str().unwrap_or_default()
                    && e.kind == chain_base::AutonomousBountyEventKind::SubmissionAdded
                    && e.data["submission_hash"] == scope["submission_hash"]
            }) {
                return Ok(artifact);
            }
        }
    }
    Err(StatusCode::NOT_FOUND)
}
#[utoipa::path(get,path="/v1/verification/bounties/{network}/{contract}/submissions",params(("network"=String,Path,description="Base network"),("contract"=String,Path,description="Canonical contract")),responses((status=200,description="Public canonical submission files and confirmed review explanations"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn history(
    State(state): State<SharedState>,
    Path((network, contract)): Path<(String, String)>,
) -> Result<Json<Value>, StatusCode> {
    let item = crate::indexed_autonomous_bounty(&state, &network, &contract).await?;
    let files = db(&state)?
        .published_artifacts(&network, &item.bounty_contract)
        .await
        .map_err(err)?;
    let reports = db(&state)?
        .published_reviews(&network, &item.bounty_contract)
        .await
        .map_err(err)?;
    let entries:Vec<_>=item.events.iter().filter(|e|e.kind==chain_base::AutonomousBountyEventKind::SubmissionAdded).map(|event| {
        let round=event.data["round"].as_u64().unwrap_or(0);
        let paid=item.events.iter().find(|e|e.kind==chain_base::AutonomousBountyEventKind::BountySettled&&e.data["round"].as_u64()==Some(round)&&e.data["submission_hash"]==event.data["submission_hash"]);
        json!({"round":round,"solver":event.data["solver"],"submitted_at":event.occurred_at,"review_deadline":event.data["verification_expires_at"],"files":files.iter().filter(|f|f.canonical_scope.as_ref().is_some_and(|s|s["round"].as_u64()==Some(round))).collect::<Vec<_>>(),"reviews":reports.iter().filter(|r|r["round"].as_u64()==Some(round)).collect::<Vec<_>>(),"confirmed_payment":paid,"history_notice":"Missing files or reviews mean no durable record was recovered; they do not prove that no work was submitted."})
    }).collect();
    Ok(Json(
        json!({"network":network,"bounty_contract":item.bounty_contract,"submissions":entries,"selection_notice":"Qualification, selection and confirmed payment are separate. Existing bounty rules still apply; a separate creator award does not rewrite the original outcome."}),
    ))
}
pub async fn cleanup_loop(state: SharedState, objects: worker::artifact_store::ArtifactStore) {
    loop {
        if let Ok(store) = db(&state) {
            if let Ok(artifacts) = store.pending_artifacts().await {
                for artifact in artifacts {
                    let Some(scope) = artifact.pending_scope.as_ref() else {
                        continue;
                    };
                    let (Some(network), Some(contract), Some(round), Some(solver)) = (
                        scope["network"].as_str(),
                        scope["bounty_contract"].as_str(),
                        scope["round"].as_u64(),
                        scope["solver"].as_str(),
                    ) else {
                        continue;
                    };
                    // Unknown or unavailable chain state always retains the files.
                    let Ok(item) =
                        crate::indexed_autonomous_bounty(&state, network, contract).await
                    else {
                        continue;
                    };
                    let expected = chain_base::sha256_utf8(&reference(&artifact));
                    if let Some(event) = item.events.iter().find(|e| {
                        e.kind == chain_base::AutonomousBountyEventKind::SubmissionAdded
                            && e.data["round"].as_u64() == Some(round)
                            && e.data["submission_hash"] == expected
                            && e.data["solver"]
                                .as_str()
                                .is_some_and(|s| s.eq_ignore_ascii_case(solver))
                    }) {
                        let confirmed = json!({"network":network,"bounty_contract":item.bounty_contract.to_lowercase(),"bounty_id":item.bounty_id,"round":round,"event_id":event.id,"submission_hash":expected,"solver":event.data["solver"],"visibility":"private"});
                        // Recovery confirms retention only. Publishing files still
                        // requires the solver's explicit authenticated request.
                        let _ = store
                            .confirm_artifact(artifact.id, &artifact.owner_key, &confirmed)
                            .await;
                    } else if item
                        .events
                        .iter()
                        .any(|e| pending_round_has_ended(&e.kind, &e.data, round))
                    {
                        let _ = store.release_unsubmitted_artifact(artifact.id, scope).await;
                    }
                }
            }
            if let Ok(artifacts) = store.archive_candidates().await {
                for artifact in artifacts {
                    let Some(scope) = artifact.canonical_scope else {
                        continue;
                    };
                    let (Some(network), Some(contract)) =
                        (scope["network"].as_str(), scope["bounty_contract"].as_str())
                    else {
                        continue;
                    };
                    if let Ok(item) =
                        crate::indexed_autonomous_bounty(&state, network, contract).await
                    {
                        if let Some(closed) = item.events.iter().rev().find(|e| {
                            matches!(
                                e.kind,
                                chain_base::AutonomousBountyEventKind::BountySettled
                                    | chain_base::AutonomousBountyEventKind::BountyCancelled
                            )
                        }) {
                            let winning = closed.kind
                                == chain_base::AutonomousBountyEventKind::BountySettled
                                && closed.data["round"] == scope["round"]
                                && closed.data["submission_hash"] == scope["submission_hash"]
                                && closed.data["solver"] == scope["solver"];
                            let _ = store
                                .retain_closed_artifact(artifact.id, closed.occurred_at, winning)
                                .await;
                        }
                    }
                }
            }
            if let Ok(expired) = store.expired_artifacts().await {
                for artifact in expired {
                    if objects
                        .delete(artifact.id, &artifact.manifest)
                        .await
                        .is_ok()
                    {
                        let _ = store.artifact_deleted(artifact.id).await;
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

fn pending_round_has_ended(
    kind: &chain_base::AutonomousBountyEventKind,
    data: &Value,
    round: u64,
) -> bool {
    use chain_base::AutonomousBountyEventKind as Kind;
    matches!(kind, Kind::BountyCancelled | Kind::BountySettled)
        || (data["round"].as_u64() == Some(round)
            && matches!(
                kind,
                Kind::ClaimExpired
                    | Kind::SubmissionExpired
                    | Kind::SubmissionRejected
                    | Kind::SubmissionAdded
            ))
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    #[test]
    fn only_canonical_round_completion_releases_unsubmitted_files() {
        use chain_base::AutonomousBountyEventKind as Kind;
        for kind in [
            Kind::ClaimExpired,
            Kind::SubmissionExpired,
            Kind::SubmissionRejected,
            Kind::SubmissionAdded,
        ] {
            assert!(pending_round_has_ended(&kind, &json!({"round":3}), 3));
            assert!(!pending_round_has_ended(&kind, &json!({"round":2}), 3));
        }
        assert!(!pending_round_has_ended(
            &Kind::BountyClaimed,
            &json!({"round":3,"expires_at":1}),
            3
        ));
        assert!(!pending_round_has_ended(&Kind::FundingAdded, &json!({}), 3));
        assert!(pending_round_has_ended(
            &Kind::BountyCancelled,
            &json!({}),
            3
        ));
        assert!(pending_round_has_ended(
            &Kind::BountySettled,
            &json!({"round":2}),
            3
        ));
    }
}

#[utoipa::path(post,path="/v1/verification/artifacts/{id}/participants",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Grant expiring read access to saved files and checks"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn grant(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<secure_operations::ParticipantGrantRequest>,
) -> Result<StatusCode, StatusCode> {
    let key = secure_operations::authenticated(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    secure_operations::participant_key(&input.principal_key)?;
    db(&state)?.owned_artifact(id, &key).await.map_err(err)?;
    let expiry = input.expires_at.ok_or(StatusCode::BAD_REQUEST)?;
    if expiry <= chrono::Utc::now() || expiry > chrono::Utc::now() + chrono::Duration::days(30) {
        return Err(StatusCode::BAD_REQUEST);
    }
    db(&state)?
        .grant_resource_access(
            "verification_artifact",
            &id.to_string(),
            &key,
            &input.principal_key,
            "read",
            expiry,
        )
        .await
        .map_err(|e| security_auth::status(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/v1/verification/artifacts/{id}/participants/revoke",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Revoke participant read access"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn revoke(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<secure_operations::ParticipantGrantRequest>,
) -> Result<StatusCode, StatusCode> {
    let key = secure_operations::authenticated(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    secure_operations::participant_key(&input.principal_key)?;
    db(&state)?.owned_artifact(id, &key).await.map_err(err)?;
    db(&state)?
        .revoke_resource_access(
            "verification_artifact",
            &id.to_string(),
            &key,
            &input.principal_key,
        )
        .await
        .map_err(|e| security_auth::status(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}
