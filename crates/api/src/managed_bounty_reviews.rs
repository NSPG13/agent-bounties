//! Customer review of a funded card bounty. The paying customer records a verdict on the exact
//! canonical submission whose public evidence they could inspect. The dedicated, code-pinned
//! review key, the bounty's sole quorum verifier, signs an acceptance at once. A rejection waits
//! for the solver's appeal and is signed when the appeal period closes unused, or the arbiter's
//! ruling is signed instead (see `managed_bounty_appeals`). Recording and signing are not
//! settlement: only a canonical `BountySettled` proves solver payment.
use crate::{
    managed_payments::{self, BuyerRead, BuyerWrite},
    sponsored_creation, sponsored_verdict, SharedState,
};
use alloy::signers::{local::PrivateKeySigner, SignerSync};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chain_base::{
    customer_review_verifiers, validate_attestation_request_against_feed,
    AutonomousBountyEventKind, AutonomousBountyFeedItem, AutonomousSignedAttestation,
    AutonomousVerificationAttestationRequest,
};
use db::managed_service_orders::{
    ManagedCustomerReview, ManagedOrder, ManagedOrderStatus, ManagedOrderStoreError,
    ReviewSignature, ReviewSigner,
};
use domain::AutonomousSubmissionEvidenceRecord;
use managed_orders::review::{appeal_ends_at, CustomerVerdict, ReviewSchedule};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    round: u64,
    submission_hash: String,
    evidence_hash: String,
    passed: bool,
    #[serde(default)]
    reason: String,
}

pub(crate) fn now() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or_default()
}

/// The configured review key, refused unless it is the pinned verifier for `network`.
pub(crate) fn review_signer<'a>(
    state: &'a SharedState,
    network: &str,
) -> Result<&'a PrivateKeySigner, StatusCode> {
    let config = state
        .managed_payments
        .bounty
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let signer = state
        .managed_payments
        .review_signer
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let address = format!("{:#x}", signer.address());
    if config.network != network
        || !config
            .customer_review
            .verifier
            .eq_ignore_ascii_case(&address)
        || !customer_review_verifiers(network)
            .iter()
            .any(|pinned| pinned.eq_ignore_ascii_case(&address))
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(signer)
}

pub(crate) fn store_status(error: ManagedOrderStoreError) -> StatusCode {
    match error {
        ManagedOrderStoreError::NotFound => StatusCode::NOT_FOUND,
        ManagedOrderStoreError::Quote(_) => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// The v2 attestation for `verdict`'s submission with the given outcome.
fn attestation(
    item: &AutonomousBountyFeedItem,
    verdict: &CustomerVerdict,
    verifier: &str,
    passed: bool,
    response_hash: &str,
    deadline: u64,
) -> Result<AutonomousVerificationAttestationRequest, StatusCode> {
    let terms = item.terms.as_ref().ok_or(StatusCode::CONFLICT)?;
    Ok(AutonomousVerificationAttestationRequest {
        bounty_contract: verdict.bounty_contract.clone(),
        bounty_id: item.bounty_id.to_ascii_lowercase(),
        round: verdict.round,
        verifier: verifier.to_owned(),
        submission_hash: verdict.submission_hash.clone(),
        evidence_hash: verdict.evidence_hash.clone(),
        policy_hash: terms.policy_hash.to_ascii_lowercase(),
        passed,
        response_hash: response_hash.to_owned(),
        deadline,
    })
}

/// Signs, inside the store's locked transaction, only an attestation the indexed bounty still
/// accepts at `now` for exactly `verdict`'s submission.
pub(crate) fn signer_for<'a>(
    signer: &'a PrivateKeySigner,
    network: &'a str,
    item: &'a AutonomousBountyFeedItem,
    verdict: &'a CustomerVerdict,
    now: u64,
) -> impl ReviewSigner + 'a {
    move |passed: bool, response_hash: &str, deadline: u64| {
        let verifier = format!("{:#x}", signer.address());
        let request = attestation(item, verdict, &verifier, passed, response_hash, deadline)
            .map_err(|_| ())?;
        validate_attestation_request_against_feed(item, &request, now).map_err(|_| ())?;
        let digest =
            sponsored_verdict::digest_for_version(network, &request, "2").map_err(|_| ())?;
        let signature = signer.sign_hash_sync(&digest).map_err(|_| ())?;
        Ok(ReviewSignature {
            verifier,
            signature: format!("0x{}", hex::encode(signature.as_bytes())),
        })
    }
}

/// The public view of one round: the verdict, any appeal and ruling, and the one signed
/// attestation once the review key has released it.
pub(crate) fn review_view(
    item: &AutonomousBountyFeedItem,
    review: &ManagedCustomerReview,
    relay: Value,
    now: u64,
) -> Result<Value, StatusCode> {
    let signed = review
        .signed
        .as_ref()
        .map(|signed| {
            attestation(
                item,
                &review.verdict,
                &signed.verifier,
                signed.passed,
                &signed.response_hash,
                review.deadline,
            )
        })
        .transpose()?;
    let appeal = review.appeal_status(now).map(|status| {
        json!({
            "status": status,
            "window_ends_at": review.appeal_ends_at.unwrap_or(review.deadline),
            "document": review.appeal.as_ref().map(|a| a.appeal.document()),
            "appeal_hash": review.appeal.as_ref().map(|a| &a.appeal_hash),
        })
    });
    Ok(json!({
        "document": review.verdict.document(),
        "response_hash": review.response_hash,
        "decided_by": review.decided_by.as_str(),
        "appeal": appeal,
        "ruling": review.ruling.as_ref().map(|r| json!({
            "document": r.ruling.document(),
            "response_hash": r.response_hash,
        })),
        "attestation": signed,
        "signature": review.signed.as_ref().map(|s| &s.signature),
        "relay": relay,
        "evidence_boundary": "A signed verdict is not settlement. Only a canonical BountySettled proves solver payment; SubmissionRejected keeps the bounty funded for another attempt."
    }))
}

/// The canonical submission awaiting review, with its published preimage and review schedule.
pub(crate) struct PendingSubmission {
    pub round: u64,
    pub solver: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub expires_at: u64,
    pub evidence: Option<AutonomousSubmissionEvidenceRecord>,
    pub schedule: ReviewSchedule,
}

impl PendingSubmission {
    pub(crate) fn view(&self) -> Value {
        json!({
            "round": self.round,
            "solver": self.solver,
            "submission_hash": self.submission_hash,
            "evidence_hash": self.evidence_hash,
            "verification_expires_at": self.expires_at,
            "decide_by": self.schedule.decide_by,
            // Silence until decide_by accepts only when this is true. Before the evidence is
            // published it states what publishing now would give.
            "auto_accept": self.schedule.auto_accept,
            "sign_by": self.schedule.sign_by,
            "evidence": self.evidence.as_ref().map(|record| json!({
                "artifact_reference": record.artifact_reference,
                "artifact_hash": record.artifact_hash,
                "evidence": record.evidence,
            })),
        })
    }

    /// The verdict document for this submission.
    pub(crate) fn verdict(&self, bounty: &str, passed: bool, reason: String) -> CustomerVerdict {
        CustomerVerdict {
            bounty_contract: bounty.to_ascii_lowercase(),
            round: self.round,
            submission_hash: self.submission_hash.clone(),
            evidence_hash: self.evidence_hash.clone(),
            passed,
            reason,
        }
    }
}

/// The latest canonical submission, if the bounty awaits review. The decision period starts
/// when the evidence preimage is published, never before the submission itself.
pub(crate) async fn pending_submission(
    state: &SharedState,
    network: &str,
    item: &AutonomousBountyFeedItem,
    now: u64,
) -> Result<Option<PendingSubmission>, StatusCode> {
    if item.status != "submitted" {
        return Ok(None);
    }
    let Some(event) = item
        .events
        .iter()
        .rev()
        .find(|event| event.kind == AutonomousBountyEventKind::SubmissionAdded)
    else {
        return Ok(None);
    };
    let field = |name: &str| {
        event.data[name]
            .as_str()
            .map(str::to_ascii_lowercase)
            .ok_or(StatusCode::CONFLICT)
    };
    let round = event.data["round"].as_u64().ok_or(StatusCode::CONFLICT)?;
    let expires_at = event.data["verification_expires_at"]
        .as_u64()
        .ok_or(StatusCode::CONFLICT)?;
    let evidence_hash = field("evidence_hash")?;
    let window = item
        .terms
        .as_ref()
        .and_then(|terms| terms.document.contract_terms["verification_window_seconds"].as_u64())
        .ok_or(StatusCode::CONFLICT)?;
    let evidence = managed_payments::store(state)?
        .get_autonomous_submission_evidence(network, &item.bounty_contract, round)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .filter(|record| record.evidence_hash.eq_ignore_ascii_case(&evidence_hash));
    let published_at = evidence.as_ref().map_or(now, |record| {
        u64::try_from(record.created_at.timestamp()).unwrap_or(now)
    });
    Ok(Some(PendingSubmission {
        round,
        solver: field("solver")?,
        submission_hash: field("submission_hash")?,
        evidence_hash,
        expires_at,
        schedule: ReviewSchedule::new(expires_at, window, published_at)
            .ok_or(StatusCode::CONFLICT)?,
        evidence,
    }))
}

async fn funded_order(
    state: &SharedState,
    buyer: &str,
    id: Uuid,
) -> Result<(ManagedOrder, AutonomousBountyFeedItem), StatusCode> {
    let order = managed_payments::store(state)?
        .get_managed_service_order(buyer, id)
        .await
        .map_err(store_status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if order.status != ManagedOrderStatus::Funded {
        return Err(StatusCode::CONFLICT);
    }
    let bounty = &order.quote.input().bounty;
    let item =
        crate::indexed_autonomous_bounty(state, &bounty.network, &bounty.predicted_bounty).await?;
    Ok((order, item))
}

/// The customer's refundable review deposit: held until the bounty is paid, then refunded to
/// the card through signed Stripe events, or owed to the solver when the arbiter reverses the
/// customer's rejection.
async fn deposit(state: &SharedState, order: &ManagedOrder) -> Result<Value, StatusCode> {
    let input = order.quote.input();
    let Some(line) = input
        .additional_costs
        .iter()
        .find(|cost| cost.code == managed_orders::pricing::REVIEW_DEPOSIT_CODE)
    else {
        return Ok(Value::Null);
    };
    let status = managed_payments::store(state)?
        .managed_deposit_state(order.order_id)
        .await
        .map_err(store_status)?
        .as_str();
    Ok(status.map_or(Value::Null, |status| {
        json!({"amount_minor": line.amount_minor, "currency": input.currency, "status": status})
    }))
}

#[utoipa::path(get,path="/v1/managed-service/orders/{id}/review",params(("id"=Uuid,Path,description="Funded card order ID")),responses((status=200,description="Committed bounty state, the submission awaiting the customer's decision with its published evidence and decide-by time, recorded verdicts with appeal state, and the review deposit"),(status=401,description="Website account login required"),(status=404,description="Order not owned by account"),(status=409,description="The order is not funded")))]
pub async fn read_review(
    State(state): State<SharedState>,
    BuyerRead(buyer): BuyerRead,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    managed_payments::quota(&state, &buyer, "managed_read").await?;
    let (order, item) = funded_order(&state, &buyer, id).await?;
    let network = order.quote.input().bounty.network.clone();
    let now = now();
    let recorded = managed_payments::store(&state)?
        .list_managed_customer_reviews(&buyer, id)
        .await
        .map_err(store_status)?;
    let reviews = recorded
        .iter()
        .map(|review| review_view(&item, review, Value::Null, now))
        .collect::<Result<Vec<_>, _>>()?;
    let pending = pending_submission(&state, &network, &item, now).await?;
    Ok(Json(json!({
        "order_id": id,
        "network": network,
        "bounty_contract": item.bounty_contract,
        "bounty_status": item.status,
        "pending_submission": pending.as_ref().map(PendingSubmission::view),
        "reviews": reviews,
        "deposit": deposit(&state, &order).await?,
    })))
}

#[utoipa::path(post,path="/v1/managed-service/orders/{id}/review",params(("id"=Uuid,Path,description="Funded card order ID")),request_body=Value,responses((status=200,description="The recorded verdict. An acceptance carries its signed attestation, relayed when sponsorship is enabled. A rejection waits unsigned for the solver's appeal. Not settlement."),(status=400,description="Verdict or reason invalid"),(status=401,description="Website account login and allowed Origin required"),(status=404,description="Order not owned by account"),(status=409,description="Not the current canonical submission, evidence unpublished, past the decide-by time, or a different verdict is already recorded"),(status=503,description="Review key or v2 indexing unavailable")))]
pub async fn record_review(
    State(state): State<SharedState>,
    BuyerWrite(buyer): BuyerWrite,
    Path(id): Path<Uuid>,
    Json(request): Json<ReviewRequest>,
) -> Result<Json<Value>, StatusCode> {
    managed_payments::quota(&state, &buyer, "managed_review").await?;
    let (order, item) = funded_order(&state, &buyer, id).await?;
    let network = order.quote.input().bounty.network.clone();
    let signer = review_signer(&state, &network)?;
    let verdict = CustomerVerdict {
        bounty_contract: item.bounty_contract.to_ascii_lowercase(),
        round: request.round,
        submission_hash: request.submission_hash.to_ascii_lowercase(),
        evidence_hash: request.evidence_hash.to_ascii_lowercase(),
        passed: request.passed,
        reason: request.reason,
    };
    verdict.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = managed_payments::store(&state)?;
    let now = now();
    let existing = store
        .list_managed_customer_reviews(&buyer, id)
        .await
        .map_err(store_status)?
        .into_iter()
        .find(|review| review.verdict.round == verdict.round);
    let review = match existing {
        Some(review) if review.verdict == verdict => review,
        Some(_) => return Err(StatusCode::CONFLICT),
        None => {
            // The customer must have been able to inspect the exact committed evidence, and
            // decides before the time that still leaves the solver a full appeal.
            let pending = pending_submission(&state, &network, &item, now)
                .await?
                .ok_or(StatusCode::CONFLICT)?;
            if pending.evidence.is_none()
                || pending.verdict(
                    &item.bounty_contract,
                    verdict.passed,
                    verdict.reason.clone(),
                ) != verdict
                || now >= pending.schedule.decide_by
            {
                return Err(StatusCode::CONFLICT);
            }
            let response_hash = verdict
                .response_hash()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            let check = attestation(
                &item,
                &verdict,
                &format!("{:#x}", signer.address()),
                verdict.passed,
                &response_hash,
                pending.expires_at,
            )?;
            validate_attestation_request_against_feed(&item, &check, now)
                .map_err(|_| StatusCode::CONFLICT)?;
            store
                .record_managed_customer_review(
                    &buyer,
                    id,
                    &verdict,
                    pending.expires_at,
                    (!verdict.passed).then(|| appeal_ends_at(now)),
                    signer_for(signer, &network, &item, &verdict, now),
                )
                .await
                .map_err(store_status)?
        }
    };
    let relay = relay(&state, &network, &item, &review).await;
    Ok(Json(review_view(&item, &review, relay, now)?))
}

/// Send `settleWithAttestations` for the round's signed attestation through the sponsored
/// relayer when it is enabled. Anyone may relay the public signed attestation instead. Failure
/// here never changes the recorded review. An unsigned rejection has nothing to relay yet.
pub(crate) async fn relay(
    state: &SharedState,
    network: &str,
    item: &AutonomousBountyFeedItem,
    review: &ManagedCustomerReview,
) -> Value {
    let Some(signed) = &review.signed else {
        return json!({"status": "awaiting_appeal_period", "reason": "A rejection is signed only after the solver's appeal period or the arbiter's ruling."});
    };
    let deferred = |reason: &str| json!({"status": "deferred", "reason": reason});
    if !sponsored_creation::enabled(state, network) {
        return deferred(
            "Sponsored relaying is disabled; anyone may submit the signed attestation.",
        );
    }
    let outcome = async {
        let request = attestation(
            item,
            &review.verdict,
            &signed.verifier,
            signed.passed,
            &signed.response_hash,
            review.deadline,
        )?;
        let digest = sponsored_verdict::digest_for_version(network, &request, "2")?;
        let store = managed_payments::store(state)?;
        let key = format!("customer-verdict:{digest:#x}");
        let mut operation = match store
            .find_sponsored_action(network, &key)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .filter(|operation| operation.transaction_hash.is_some())
        {
            Some(operation) => operation,
            None => store
                .reserve_sponsored_action(
                    network,
                    &key,
                    &signed.verifier,
                    state.x402_relayer.max_daily_attempts,
                    state.x402_relayer.max_daily_attempts_per_contributor,
                )
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        };
        if operation.transaction_hash.is_none() && item.status == "submitted" {
            let relayer = state
                .x402_relayer
                .relayer
                .as_ref()
                .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
            let (planner, _) = crate::configured_autonomous_v2_planner(network)?;
            let intent = planner
                .plan_attestation_settlement(
                    &review.verdict.bounty_contract,
                    Some(&relayer.address()),
                    &[AutonomousSignedAttestation {
                        verifier: signed.verifier.clone(),
                        passed: signed.passed,
                        response_hash: signed.response_hash.clone(),
                        deadline: review.deadline,
                        signature: signed.signature.clone(),
                    }],
                )
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            operation =
                sponsored_verdict::broadcast(state, network, operation, &intent, 700_000).await?;
        }
        if operation.transaction_hash.is_some() {
            operation =
                sponsored_verdict::retry_saved(state, network, operation, review.deadline).await?;
        }
        Ok::<_, StatusCode>(operation)
    }
    .await;
    match outcome {
        Ok(operation) => json!({
            "status": operation.status,
            "transaction_hash": operation.transaction_hash,
        }),
        Err(_) => deferred(
            "Sponsored relay is unavailable; retry this request or submit the signed attestation.",
        ),
    }
}

#[cfg(test)]
mod rehearsal;
