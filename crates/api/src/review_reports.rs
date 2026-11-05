//! Review explanations are durable before signing. Reports never substitute for
//! the legacy contract's exact signature quorum or canonical settlement event.
use crate::{secure_operations, security_auth, SharedState};
use alloy::{
    primitives::{keccak256, Address, Bytes, Signature, B256, U256},
    sol_types::{SolCall, SolStruct, SolValue},
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chain_base::{AutonomousVerificationAttestationRequest, JsonRpcTransport};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use uuid::Uuid;
alloy::sol! {
    struct VerificationAttestation {address bounty;bytes32 bountyId;uint64 round;address verifier;bytes32 submissionHash;bytes32 evidenceHash;bytes32 policyHash;bool passed;bytes32 responseHash;uint256 deadline;}
    struct Attestation {address verifier;bool passed;bytes32 responseHash;uint256 deadline;bytes signature;}
    function settleWithAttestations(Attestation[] attestations);
}
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/v1/verification/reviews", post(create))
        .route("/v1/verification/reviews/:id", get(read))
        .route("/v1/verification/reviews/:id/signature", post(sign))
        .route("/v1/verification/reviews/:id/confirm", post(confirm))
}
fn bad<E>(_: E) -> StatusCode {
    StatusCode::BAD_REQUEST
}
fn db_status(e: db::verification::EvidenceError) -> StatusCode {
    match e {
        db::verification::EvidenceError::NotFound => StatusCode::NOT_FOUND,
        db::verification::EvidenceError::Conflict => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}
async fn owner(state: &SharedState, headers: &HeaderMap) -> Result<String, StatusCode> {
    secure_operations::authenticated(state, headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)
}
fn store(state: &SharedState) -> Result<&db::PostgresStore, StatusCode> {
    state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Draft {
    network: String,
    attestation: AutonomousVerificationAttestationRequest,
    assessment: Value,
    previous_report_id: Option<Uuid>,
}
#[utoipa::path(post,path="/v1/verification/reviews",request_body=Value,responses((status=200,description="Store an immutable criterion report before signing"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn create(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(draft): Json<Draft>,
) -> Result<Json<Value>, StatusCode> {
    let principal = secure_operations::authenticated(&state, &headers).await?;
    let signer_authority = security_auth::authorize_verified_wallet(
        &state,
        &principal,
        &draft.attestation.verifier,
        &draft.network,
    )
    .await;
    if let Err(status) = signer_authority {
        if status != StatusCode::FORBIDDEN {
            return Err(status);
        }
        let key = principal.key().ok_or(StatusCode::UNAUTHORIZED)?;
        let assigned = store(&state)?
            .assigned_review_agent(
                &key,
                &draft.network,
                &draft.attestation.bounty_contract,
                i64::try_from(draft.attestation.round).map_err(bad)?,
                &draft.attestation.verifier,
            )
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if !assigned {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    let item = crate::indexed_autonomous_bounty(
        &state,
        &draft.network,
        &draft.attestation.bounty_contract,
    )
    .await?;
    chain_base::validate_attestation_request_against_feed(
        &item,
        &draft.attestation,
        chrono::Utc::now().timestamp() as u64,
    )
    .map_err(|_| StatusCode::CONFLICT)?;
    let terms = &item.terms.as_ref().ok_or(StatusCode::CONFLICT)?.document;
    let assessment = &draft.assessment;
    let checks = assessment["checks"]
        .as_array()
        .ok_or(StatusCode::BAD_REQUEST)?;
    if checks.len() != terms.acceptance_criteria.len()
        || checks.iter().zip(&terms.acceptance_criteria).any(|(r, c)| {
            r["criterion"].as_str() != Some(c.as_str())
                || !r["passed"].is_boolean()
                || r["reason"]
                    .as_str()
                    .is_none_or(|s| s.trim().is_empty() || s.len() > 1000)
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let submission = item
        .events
        .iter()
        .rev()
        .find(|e| {
            e.data["round"].as_u64() == Some(draft.attestation.round)
                && e.kind == chain_base::AutonomousBountyEventKind::SubmissionAdded
        })
        .ok_or(StatusCode::CONFLICT)?;
    let on_time = if terms.benchmark["engine"] == "creator_review_v1" {
        chain_base::creator_review_submission_on_time(
            terms,
            submission.data["verification_expires_at"]
                .as_u64()
                .ok_or(StatusCode::CONFLICT)?,
        )
    } else {
        true
    };
    if assessment["round"] != draft.attestation.round
        || assessment["submission_hash"] != draft.attestation.submission_hash
        || assessment["evidence_hash"] != draft.attestation.evidence_hash
        || assessment["on_time"] != on_time
        || assessment["passed"] != (on_time && checks.iter().all(|c| c["passed"] == true))
        || assessment["passed"] != draft.attestation.passed
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let hash = chain_base::keccak256_canonical_json(assessment).map_err(bad)?;
    if !hash.eq_ignore_ascii_case(&draft.attestation.response_hash) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let owner = principal.key().ok_or(StatusCode::UNAUTHORIZED)?;
    let mut record = store(&state)?
        .save_review_report(
            &owner,
            &draft.network,
            &serde_json::to_value(draft.attestation).map_err(bad)?,
            assessment,
            draft.previous_report_id,
        )
        .await
        .map_err(db_status)?;
    // This page currently supports the existing Base-mainnet creator review flow.
    // Other verifier policies keep their own signing/quorum workflow.
    record["review_url"] = if draft.network == "base-mainnet"
        && terms.benchmark["engine"] == "creator_review_v1"
    {
        json!(format!("https://agentizable.ai/participate.html?bountyContract={}&network=base-mainnet&role=verifier&reviewReport={}",
            record["bounty_contract"].as_str().ok_or(StatusCode::CONFLICT)?,
            record["id"].as_str().ok_or(StatusCode::CONFLICT)?))
    } else {
        Value::Null
    };
    Ok(Json(record))
}
#[utoipa::path(get,path="/v1/verification/reviews/{id}",params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Read a report you authored or may sign"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn read(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let owner = security_auth::principal(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    store(&state)?
        .review_report(id, &owner)
        .await
        .map(Json)
        .map_err(db_status)
}
fn digest(network: &str, r: &AutonomousVerificationAttestationRequest) -> Result<B256, StatusCode> {
    let chain = chain_base::base_network_descriptor(network)
        .map_err(bad)?
        .chain_id;
    let message = VerificationAttestation {
        bounty: Address::from_str(&r.bounty_contract).map_err(bad)?,
        bountyId: B256::from_str(&r.bounty_id).map_err(bad)?,
        round: r.round,
        verifier: Address::from_str(&r.verifier).map_err(bad)?,
        submissionHash: B256::from_str(&r.submission_hash).map_err(bad)?,
        evidenceHash: B256::from_str(&r.evidence_hash).map_err(bad)?,
        policyHash: B256::from_str(&r.policy_hash).map_err(bad)?,
        passed: r.passed,
        responseHash: B256::from_str(&r.response_hash).map_err(bad)?,
        deadline: U256::from(r.deadline),
    };
    Ok(message.eip712_signing_hash(&alloy::sol_types::eip712_domain!{name:"Agent Bounties",version:"1",chain_id:chain,verifying_contract:message.bounty,}))
}
async fn rpc(
    state: &SharedState,
    network: &str,
    method: &str,
    params: Value,
) -> Result<Value, StatusCode> {
    let (_, url) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let response = chain_base::ReqwestJsonRpcTransport::default()
        .post_json_value(
            &url,
            &json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if response["id"] != 1 || response["jsonrpc"] != "2.0" || response.get("error").is_some() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    response
        .get("result")
        .cloned()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
pub(crate) async fn verify_signature(
    state: &SharedState,
    network: &str,
    address: &str,
    digest: B256,
    signature: &str,
) -> Result<(), StatusCode> {
    let sig = hex::decode(
        signature
            .strip_prefix("0x")
            .ok_or(StatusCode::BAD_REQUEST)?,
    )
    .map_err(bad)?;
    if sig.is_empty() || sig.len() > 8192 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let chain = chain_base::base_network_descriptor(network)
        .map_err(bad)?
        .chain_id;
    if rpc(state, network, "eth_chainId", json!([]))
        .await?
        .as_str()
        != Some(&format!("0x{chain:x}"))
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let block = rpc(
        state,
        network,
        "eth_getBlockByNumber",
        json!(["safe", false]),
    )
    .await?;
    let block_hash = block["hash"]
        .as_str()
        .and_then(|s| B256::from_str(s).ok())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let reference = json!({"blockHash":format!("{block_hash:#x}"),"requireCanonical":true});
    let code = rpc(state, network, "eth_getCode", json!([address, reference])).await?;
    if code == "0x" {
        let signature = Signature::try_from(sig.as_slice()).map_err(bad)?;
        if signature.normalize_s().is_some()
            || signature
                .recover_address_from_prehash(&digest)
                .map_err(bad)?
                != Address::from_str(address).map_err(bad)?
        {
            return Err(StatusCode::FORBIDDEN);
        }
        return Ok(());
    }
    let code = code
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .and_then(|s| hex::decode(s).ok())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if code.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let mut data = vec![0x16, 0x26, 0xba, 0x7e];
    data.extend((digest, Bytes::from(sig)).abi_encode_params());
    let returned = rpc(
        state,
        network,
        "eth_call",
        json!([{"to":address,"data":format!("0x{}",hex::encode(data)),"gas":"0x30d40"},reference]),
    )
    .await?;
    if returned.as_str() != Some(&format!("0x1626ba7e{}", "00".repeat(28))) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Signed {
    signature: String,
}
#[utoipa::path(post,path="/v1/verification/reviews/{id}/signature",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Verify a signature against the stored report; first-party signing only"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn sign(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(signed): Json<Signed>,
) -> Result<Json<Value>, StatusCode> {
    let owner = owner(&state, &headers).await?;
    let report = store(&state)?
        .review_report(id, &owner)
        .await
        .map_err(db_status)?;
    let request: AutonomousVerificationAttestationRequest =
        serde_json::from_value(report["request"].clone()).map_err(bad)?;
    let network = report["network"].as_str().ok_or(StatusCode::CONFLICT)?;
    let item = crate::indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    chain_base::validate_attestation_request_against_feed(
        &item,
        &request,
        chrono::Utc::now().timestamp() as u64,
    )
    .map_err(|_| StatusCode::CONFLICT)?;
    verify_signature(
        &state,
        network,
        &request.verifier,
        digest(network, &request)?,
        &signed.signature,
    )
    .await?;
    store(&state)?.review_report_phase(id,&owner,"signed",&json!({"verifier":request.verifier,"passed":request.passed,"response_hash":request.response_hash,"deadline":request.deadline,"signature":signed.signature})).await.map(Json).map_err(db_status)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Confirmation {
    transaction_hash: String,
}
#[utoipa::path(post,path="/v1/verification/reviews/{id}/confirm",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Reconcile an exact signed report against a canonical event"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn confirm(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Confirmation>,
) -> Result<Json<Value>, StatusCode> {
    let owner = owner(&state, &headers).await?;
    let report = store(&state)?
        .review_report(id, &owner)
        .await
        .map_err(db_status)?;
    let tx_hash = B256::from_str(&input.transaction_hash).map_err(bad)?;
    let network = report["network"].as_str().ok_or(StatusCode::CONFLICT)?;
    let contract = report["bounty_contract"]
        .as_str()
        .ok_or(StatusCode::CONFLICT)?;
    let item = crate::indexed_autonomous_bounty(&state, network, contract).await?;
    let expected = if report["signed_attestation"]["passed"] == true {
        chain_base::AutonomousBountyEventKind::BountySettled
    } else {
        chain_base::AutonomousBountyEventKind::SubmissionRejected
    };
    let Some(event) = item.events.iter().find(|e| {
        e.kind == expected
            && e.data["round"] == report["round"]
            && e.tx_hash.eq_ignore_ascii_case(&input.transaction_hash)
    }) else {
        return store(&state)?
            .review_report_phase(
                id,
                &owner,
                "submitted",
                &json!({"transaction_hash":format!("{tx_hash:#x}")}),
            )
            .await
            .map(Json)
            .map_err(db_status);
    };
    let transaction = rpc(
        &state,
        network,
        "eth_getTransactionByHash",
        json!([format!("{tx_hash:#x}")]),
    )
    .await?;
    if transaction["to"]
        .as_str()
        .is_none_or(|s| !s.eq_ignore_ascii_case(contract))
    {
        return Err(StatusCode::CONFLICT);
    }
    let bytes = transaction["input"]
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .and_then(|s| hex::decode(s).ok())
        .ok_or(StatusCode::CONFLICT)?;
    let call = settleWithAttestationsCall::abi_decode(&bytes).map_err(bad)?;
    let hash = keccak256((call.attestations.clone(),).abi_encode_params());
    if event.data["verification_hash"]
        .as_str()
        .is_none_or(|s| !s.eq_ignore_ascii_case(&format!("{hash:#x}")))
    {
        return Err(StatusCode::CONFLICT);
    }
    let signed = &report["signed_attestation"];
    if !call.attestations.iter().any(|a| {
        signed["verifier"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(&format!("{:#x}", a.verifier)))
            && signed["passed"] == a.passed
            && signed["response_hash"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(&format!("{:#x}", a.responseHash)))
            && signed["signature"].as_str().is_some_and(|s| {
                s.eq_ignore_ascii_case(&format!("0x{}", hex::encode(&a.signature)))
            })
            && signed["deadline"]
                .as_u64()
                .is_some_and(|n| U256::from(n) == a.deadline)
    }) {
        return Err(StatusCode::CONFLICT);
    }
    store(&state)?.review_report_phase(id,&owner,"confirmed",&json!({"transaction_hash":event.tx_hash,"canonical_event_id":event.id,"verification_hash":format!("{hash:#x}")})).await.map(Json).map_err(db_status)
}
