//! Appeals of customer rejections on card bounties, the arbiter's ruling, and the finalizer that
//! signs what the review schedule decides. A card bounty's review state is public: documents
//! name the bounty, round, submission and reasons, never the customer or the order.
//!
//! - The solver who made the rejected submission appeals within the appeal period with an
//!   EIP-712 `ReviewAppeal` signature from their wallet (EOA or ERC-1271).
//! - The operator, as arbiter, upholds or reverses an appealed rejection. The review key signs
//!   the ruling, whose document names the rejection and appeal it decides.
//! - The finalizer signs a rejection whose appeal period closed unused, and accepts a submission
//!   whose customer stayed silent for the full decision period after the delivery was
//!   published. An arbitration that is not decided before `sign_by` signs nothing: the
//!   submission then expires on chain, which returns the solver's bond and reopens the bounty.
//! - After a confirmed canonical `BountySettled` with no reversal, the finalizer reserves and
//!   executes the refund of the customer's review deposit. A reversal instead owes the deposit
//!   to the solver; the operator pays it and records the transfer, which is verified on chain.
use crate::{
    managed_bounty_reviews::{
        now, pending_submission, relay, review_signer, review_view, signer_for, store_status,
        PendingSubmission,
    },
    managed_payments::{self, Merchant},
    sponsored_signature, SharedState,
};
use alloy::{
    primitives::{Address, B256, U256},
    sol_types::SolStruct,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chain_base::{base_network_descriptor, AutonomousBountyFeedItem};
use chain_base::{AutonomousBountyEventKind, JsonRpcTransport};
use db::managed_service_orders::{
    DepositRefundReservation, ManagedCustomerReview, ManagedDepositPayout,
};
use managed_orders::review::{ReviewAppeal, AUTO_ACCEPT_REASON};
use managed_orders::stripe_refund::RefundMerchant;
use serde::Deserialize;
use serde_json::{json, Value};
use service_runtime::managed_refund::{
    execute_managed_deposit_refund, read_managed_refund_status, ManagedRefundClient,
};
use uuid::Uuid;

mod typed {
    alloy::sol! {
        struct ReviewAppeal {
            address bounty; uint64 round; bytes32 submissionHash; bytes32 rejectionHash;
            string reason;
        }
    }
}

const FINALIZER_INTERVAL_SECONDS: u64 = 60;

fn chain_id(network: &str) -> Result<u64, StatusCode> {
    Ok(base_network_descriptor(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .chain_id)
}

/// The EIP-712 digest the solver's wallet signs for `appeal`.
pub(crate) fn appeal_digest(network: &str, appeal: &ReviewAppeal) -> Result<B256, StatusCode> {
    let bad = |_| StatusCode::BAD_REQUEST;
    let bounty: Address = appeal.bounty_contract.parse().map_err(bad)?;
    let message = typed::ReviewAppeal {
        bounty,
        round: appeal.round,
        submissionHash: appeal.submission_hash.parse().map_err(bad)?,
        rejectionHash: appeal.rejection_hash.parse().map_err(bad)?,
        reason: appeal.reason.clone(),
    };
    Ok(
        message.eip712_signing_hash(&alloy::sol_types::Eip712Domain {
            name: Some("Agent Bounties".into()),
            version: Some("1".into()),
            chain_id: Some(U256::from(chain_id(network)?)),
            verifying_contract: Some(bounty),
            salt: None,
        }),
    )
}

/// The typed data a solver signs to appeal `review`, with `reason` to be filled in.
fn appeal_typed_data(network: &str, review: &ManagedCustomerReview) -> Result<Value, StatusCode> {
    Ok(json!({
        "domain": {
            "name": "Agent Bounties",
            "version": "1",
            "chainId": chain_id(network)?,
            "verifyingContract": review.verdict.bounty_contract,
        },
        // Listed explicitly: wallets that fill a missing domain type would hash another domain.
        "types": {"EIP712Domain": [
            {"name": "name", "type": "string"},
            {"name": "version", "type": "string"},
            {"name": "chainId", "type": "uint256"},
            {"name": "verifyingContract", "type": "address"},
        ], "ReviewAppeal": [
            {"name": "bounty", "type": "address"},
            {"name": "round", "type": "uint64"},
            {"name": "submissionHash", "type": "bytes32"},
            {"name": "rejectionHash", "type": "bytes32"},
            {"name": "reason", "type": "string"},
        ]},
        "primaryType": "ReviewAppeal",
        "message": {
            "bounty": review.verdict.bounty_contract,
            "round": review.verdict.round,
            "submissionHash": review.verdict.submission_hash,
            "rejectionHash": review.response_hash,
            "reason": "",
        },
    }))
}

/// The card order, its reviews and the indexed bounty for a public bounty address.
async fn card_bounty(
    state: &SharedState,
    network: &str,
    contract: &str,
) -> Result<(Uuid, Vec<ManagedCustomerReview>, AutonomousBountyFeedItem), StatusCode> {
    chain_id(network)?;
    let contract = contract.to_ascii_lowercase();
    if contract.len() != 42 || contract.parse::<Address>().is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (order_id, reviews) = managed_payments::store(state)?
        .managed_reviews_for_bounty(network, &contract)
        .await
        .map_err(store_status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let item = crate::indexed_autonomous_bounty(state, network, &contract).await?;
    Ok((order_id, reviews, item))
}

#[utoipa::path(get,path="/v1/managed-service/bounties/{network}/{contract}/review",params(("network"=String,Path,description="base-mainnet or base-sepolia"),("contract"=String,Path,description="Card bounty contract")),responses((status=200,description="Public review state of a card bounty: the submission awaiting the customer's decision, each recorded verdict with its appeal and ruling, signed attestations, and the typed data a rejected solver signs to appeal"),(status=404,description="Not a funded card bounty")))]
pub async fn read_bounty_review(
    State(state): State<SharedState>,
    Path((network, contract)): Path<(String, String)>,
) -> Result<Json<Value>, StatusCode> {
    let (order_id, reviews, item) = card_bounty(&state, &network, &contract).await?;
    let now = now();
    let pending = pending_submission(&state, &network, &item, now).await?;
    let payout = managed_payments::store(&state)?
        .managed_deposit_payout(order_id)
        .await
        .map_err(store_status)?;
    let views = reviews
        .iter()
        .map(|review| {
            let mut view = review_view(&item, review, Value::Null, now)?;
            if review.appeal_status(now) == Some("window_open") {
                view["appeal_typed_data"] = appeal_typed_data(&network, review)?;
            }
            Ok(view)
        })
        .collect::<Result<Vec<_>, StatusCode>>()?;
    Ok(Json(json!({
        "network": network,
        "bounty_contract": item.bounty_contract,
        "bounty_status": item.status,
        "pending_submission": pending.as_ref().map(PendingSubmission::view),
        "reviews": views,
        "deposit_payout": payout.as_ref().map(payout_view),
        "appeal_rule": "The solver of a rejected submission may appeal until appeal.window_ends_at by signing appeal_typed_data with message.reason set to a public reason of 20 to 2,000 characters. Agentizable rules on the appeal against the published criteria. A reversal pays the solver and the customer's review deposit.",
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppealRequest {
    round: u64,
    solver: String,
    reason: String,
    signature: String,
}

#[utoipa::path(post,path="/v1/managed-service/bounties/{network}/{contract}/appeal",params(("network"=String,Path,description="base-mainnet or base-sepolia"),("contract"=String,Path,description="Card bounty contract")),request_body=Value,responses((status=200,description="The recorded appeal; the rejection stays unsigned until the arbiter rules"),(status=400,description="Appeal or reason invalid"),(status=401,description="Signature is not the solver's"),(status=404,description="Not a funded card bounty or no such review"),(status=409,description="Not an open appeal period for the solver's current submission, or a different appeal is recorded")))]
pub async fn appeal(
    State(state): State<SharedState>,
    Path((network, contract)): Path<(String, String)>,
    Json(request): Json<AppealRequest>,
) -> Result<Json<Value>, StatusCode> {
    let (order_id, reviews, item) = card_bounty(&state, &network, &contract).await?;
    let review = reviews
        .into_iter()
        .find(|review| review.verdict.round == request.round)
        .ok_or(StatusCode::NOT_FOUND)?;
    let appeal = ReviewAppeal {
        bounty_contract: review.verdict.bounty_contract.clone(),
        round: review.verdict.round,
        submission_hash: review.verdict.submission_hash.clone(),
        rejection_hash: review.response_hash.clone(),
        solver: request.solver.to_ascii_lowercase(),
        reason: request.reason,
    };
    appeal.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let now = now();
    // Only the solver of the still-pending rejected submission reaches signature checks.
    let recorded_same = review
        .appeal
        .as_ref()
        .is_some_and(|recorded| recorded.appeal == appeal);
    if !recorded_same {
        let pending = pending_submission(&state, &network, &item, now)
            .await?
            .ok_or(StatusCode::CONFLICT)?;
        if review.appeal_status(now) != Some("window_open")
            || pending.round != appeal.round
            || pending.submission_hash != appeal.submission_hash
            || pending.solver != appeal.solver
        {
            return Err(StatusCode::CONFLICT);
        }
        managed_payments::quota(
            &state,
            &format!("wallet:{}", appeal.solver),
            "managed_appeal",
        )
        .await?;
    }
    let digest = appeal_digest(&network, &appeal)?;
    let signature =
        sponsored_signature::verify(&state, &network, &appeal.solver, digest, &request.signature)
            .await?;
    let review = managed_payments::store(&state)?
        .record_managed_review_appeal(
            order_id,
            &appeal,
            &format!("0x{}", hex::encode(&signature)),
            now,
        )
        .await
        .map_err(store_status)?;
    Ok(Json(review_view(&item, &review, Value::Null, now)?))
}

#[utoipa::path(get,path="/v1/managed-service/appeals",responses((status=200,description="Appealed rejections awaiting the arbiter, earliest signing deadline first, with the rejection, appeal and published evidence"),(status=401,description="Operator credential required")))]
pub async fn list_appeals(
    State(state): State<SharedState>,
    _: Merchant,
) -> Result<Json<Value>, StatusCode> {
    let store = managed_payments::store(&state)?;
    let open = store
        .list_open_managed_appeals(100)
        .await
        .map_err(store_status)?;
    let mut appeals = Vec::with_capacity(open.len());
    for (network, review) in open {
        let evidence = store
            .get_autonomous_submission_evidence(
                &network,
                &review.verdict.bounty_contract,
                review.verdict.round,
            )
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .filter(|record| {
                record
                    .evidence_hash
                    .eq_ignore_ascii_case(&review.verdict.evidence_hash)
            });
        appeals.push(json!({
            "network": network,
            "bounty_contract": review.verdict.bounty_contract,
            "round": review.verdict.round,
            "rejection": review.verdict.document(),
            "appeal": review.appeal.as_ref().map(|a| a.appeal.document()),
            "evidence": evidence.map(|record| json!({
                "artifact_reference": record.artifact_reference,
                "artifact_hash": record.artifact_hash,
                "evidence": record.evidence,
            })),
            "rule_by": review
                .deadline
                .saturating_sub(managed_orders::review::SETTLEMENT_MARGIN_SECONDS),
        }));
    }
    Ok(Json(json!({ "appeals": appeals })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulingRequest {
    reversed: bool,
    reason: String,
}

#[utoipa::path(post,path="/v1/managed-service/appeals/{network}/{contract}/{round}/ruling",params(("network"=String,Path,description="base-mainnet or base-sepolia"),("contract"=String,Path,description="Card bounty contract"),("round"=u64,Path,description="Appealed round")),request_body=Value,responses((status=200,description="The signed ruling, relayed when sponsorship is enabled; not settlement"),(status=401,description="Operator credential required"),(status=404,description="No such appealed review"),(status=409,description="Not appealed, already ruled differently, or the submission can no longer be settled"),(status=503,description="Review key or v2 indexing unavailable")))]
pub async fn rule(
    State(state): State<SharedState>,
    _: Merchant,
    Path((network, contract, round)): Path<(String, String, u64)>,
    Json(request): Json<RulingRequest>,
) -> Result<Json<Value>, StatusCode> {
    let (order_id, reviews, item) = card_bounty(&state, &network, &contract).await?;
    let review = reviews
        .into_iter()
        .find(|review| review.verdict.round == round && review.appeal.is_some())
        .ok_or(StatusCode::NOT_FOUND)?;
    let signer = review_signer(&state, &network)?;
    let now = now();
    if review.ruling.is_none() {
        let pending = pending_submission(&state, &network, &item, now)
            .await?
            .ok_or(StatusCode::CONFLICT)?;
        if pending.round != round || now >= pending.schedule.sign_by {
            return Err(StatusCode::CONFLICT);
        }
    }
    let review = managed_payments::store(&state)?
        .record_managed_review_ruling(
            order_id,
            round,
            request.reversed,
            &request.reason,
            "operator",
            signer_for(signer, &network, &item, &review.verdict, now),
        )
        .await
        .map_err(store_status)?;
    let relay = relay(&state, &network, &item, &review).await;
    let mut view = review_view(&item, &review, relay, now)?;
    view["deposit_payout"] = managed_payments::store(&state)?
        .managed_deposit_payout(order_id)
        .await
        .map_err(store_status)?
        .as_ref()
        .map_or(Value::Null, payout_view);
    Ok(Json(view))
}

/// What the finalizer did for one card bounty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finalized {
    pub bounty: String,
    pub round: Option<u64>,
    pub action: &'static str,
}

/// Signs every review the schedule has decided and retries relaying signed ones whose
/// submission is still pending. Each write is idempotent, so concurrent API instances agree.
pub(crate) async fn finalize_reviews_once(
    state: &SharedState,
    now: u64,
) -> Result<Vec<Finalized>, StatusCode> {
    // Refund recovery must keep running during an idle review queue or a chain/read-signing
    // outage. Each operation retains its own authorization and durable retry identity.
    let (mut done, review_error) = match finalize_review_candidates_once(state, now).await {
        Ok(done) => (done, None),
        Err(error) => (vec![], Some(error)),
    };
    let refund_result = execute_deposit_refunds(state, &mut done).await;
    let observation_result = observe_refunds(state, now, &mut done).await;
    if let Some(error) = review_error {
        return Err(error);
    }
    refund_result?;
    observation_result?;
    Ok(done)
}

async fn finalize_review_candidates_once(
    state: &SharedState,
    now: u64,
) -> Result<Vec<Finalized>, StatusCode> {
    let network = state
        .managed_payments
        .bounty
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .network
        .clone();
    let signer = review_signer(state, &network)?;
    let store = managed_payments::store(state)?;
    let funded = store
        .claim_managed_funded_bounties(&network, 500, FINALIZER_INTERVAL_SECONDS as i64)
        .await
        .map_err(store_status)?;
    if funded.is_empty() {
        return Ok(vec![]);
    }
    // One feed build per pass, not one per bounty.
    let feed: std::collections::HashMap<String, AutonomousBountyFeedItem> =
        crate::indexed_canonical_autonomous_bounties(state, &network)
            .await?
            .into_iter()
            .map(|item| (item.bounty_contract.to_ascii_lowercase(), item))
            .collect();
    let mut done = vec![];
    let mut first_error = None;
    for (order_id, bounty) in funded {
        let Some(item) = feed.get(&bounty) else {
            continue;
        };
        let result: Result<(), StatusCode> = async {
            if item
                .events
                .iter()
                .any(|event| event.kind == AutonomousBountyEventKind::BountySettled)
            {
                // The store re-checks the confirmed settlement, reversals and disputes.
                if let DepositRefundReservation::Reserved(_) = store
                    .reserve_managed_deposit_refund(order_id)
                    .await
                    .map_err(store_status)?
                {
                    done.push(Finalized {
                        bounty: bounty.clone(),
                        round: None,
                        action: "deposit_refund_reserved",
                    });
                }
                return Ok(());
            }
            let Some(pending) = pending_submission(state, &network, item, now).await? else {
                return Ok(());
            };
            let Some((_, reviews)) = store
                .managed_reviews_for_bounty(&network, &bounty)
                .await
                .map_err(store_status)?
            else {
                return Ok(());
            };
            let current = reviews
                .into_iter()
                .find(|review| review.verdict.round == pending.round);
            let open = now < pending.schedule.sign_by;
            let (review, action) = match current {
                None if open
                    && pending.schedule.auto_accept
                    && pending.evidence.is_some()
                    && now >= pending.schedule.decide_by =>
                {
                    let verdict = pending.verdict(&bounty, true, AUTO_ACCEPT_REASON.into());
                    let review = store
                        .record_managed_deadline_acceptance(
                            order_id,
                            &verdict,
                            pending.expires_at,
                            signer_for(signer, &network, item, &verdict, now),
                        )
                        .await
                        .map_err(store_status)?;
                    (review, "auto_accepted")
                }
                Some(review)
                    if open
                        && review.signed.is_none()
                        && review.appeal.is_none()
                        && review.appeal_ends_at.is_some_and(|ends| now >= ends) =>
                {
                    let verdict = review.verdict.clone();
                    let review = store
                        .finalize_managed_rejection(
                            order_id,
                            pending.round,
                            now,
                            signer_for(signer, &network, item, &verdict, now),
                        )
                        .await
                        .map_err(store_status)?;
                    (review, "rejection_signed")
                }
                Some(review) if review.signed.is_some() => (review, "relay_retried"),
                _ => return Ok(()),
            };
            relay(state, &network, item, &review).await;
            done.push(Finalized {
                bounty: bounty.clone(),
                round: Some(pending.round),
                action,
            });
            Ok(())
        }
        .await;
        if let Err(status) = result {
            eprintln!("managed review candidate failed: order={order_id} status={status}");
            first_error.get_or_insert(status);
        }
    }
    if let Some(status) = first_error {
        return Err(status);
    }
    Ok(done)
}

/// Unix seconds of the last refund re-check; it runs at most every `REFUND_CHECK_SECONDS`.
static LAST_REFUND_CHECK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const REFUND_CHECK_SECONDS: u64 = 900;
/// Card refunds are re-checked for this long after they are created.
const REFUND_CHECK_WINDOW_SECONDS: i64 = 30 * 86_400;
/// A refund younger than this may still be waiting for its events; it is not drift yet.
const REFUND_SETTLE_SECONDS: i64 = 3_600;

/// Reads each recent card refund from Stripe and records it beside its signed status, so a lost
/// webhook shows as drift. A read never changes refund state.
pub(crate) async fn observe_refunds(
    state: &SharedState,
    now: u64,
    done: &mut Vec<Finalized>,
) -> Result<(), StatusCode> {
    use std::sync::atomic::Ordering;
    let last = LAST_REFUND_CHECK.load(Ordering::Relaxed);
    if now < last.saturating_add(REFUND_CHECK_SECONDS)
        || LAST_REFUND_CHECK
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return Ok(());
    }
    let Some(client) = refund_client(state) else {
        return Ok(());
    };
    let store = managed_payments::store(state)?;
    let mut first_error = None;
    for (order_id, kind) in store
        .claim_managed_refunds_to_observe(
            REFUND_CHECK_WINDOW_SECONDS,
            100,
            REFUND_CHECK_SECONDS as i64,
        )
        .await
        .map_err(store_status)?
    {
        let target = match store
            .managed_refund_observation_target(order_id, kind)
            .await
        {
            Ok(target) => target,
            Err(error) => {
                let status = store_status(error);
                eprintln!("managed refund target failed: order={order_id} status={status}");
                first_error.get_or_insert(status);
                continue;
            }
        };
        match read_managed_refund_status(&client, &target.plan, &target.refund_id).await {
            Ok(stripe_status) => {
                if let Err(error) = store
                    .record_managed_refund_observation(&target, stripe_status)
                    .await
                {
                    let status = store_status(error);
                    eprintln!(
                        "managed refund observation failed: order={order_id} status={status}"
                    );
                    first_error.get_or_insert(status);
                    continue;
                }
                if target.signed != Some(stripe_status) {
                    eprintln!(
                        "managed refund status differs from its signed events: order={order_id} refund={} stripe={stripe_status:?} signed={:?}",
                        target.refund_id, target.signed
                    );
                    done.push(Finalized {
                        bounty: order_id.to_string(),
                        round: None,
                        action: "refund_status_drift",
                    });
                }
            }
            Err(error) => eprintln!("managed refund read failed: order={order_id} error={error}"),
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[utoipa::path(get,path="/v1/managed-service/refund-drift",responses((status=200,description="Card refunds whose latest Stripe read differs from the status their signed Stripe events prove, with the action that repairs it"),(status=401,description="Operator credential required")))]
pub async fn list_refund_drift(
    State(state): State<SharedState>,
    _: Merchant,
) -> Result<Json<Value>, StatusCode> {
    let drift = managed_payments::store(&state)?
        .list_managed_refund_drift(REFUND_SETTLE_SECONDS)
        .await
        .map_err(store_status)?;
    Ok(Json(json!({
        "refunds": drift.iter().map(|d| json!({
            "kind": d.kind,
            "order_id": d.order_id,
            "refund_id": d.refund_id,
            "stripe_status": d.stripe_status,
            "signed_status": d.signed_status,
            "observed_at": d.observed_at,
        })).collect::<Vec<_>>(),
        "next_action": "In the Stripe dashboard, open Developers > Events, find the latest refund event for each refund_id and resend it to the checkout webhook endpoint. Its signed delivery updates the status. Never create another refund for a drifted one.",
    })))
}

/// The merchant's refund client, when the Checkout policy and Stripe key are configured.
fn refund_client(state: &SharedState) -> Option<ManagedRefundClient> {
    let policy = managed_payments::policy(state).ok()?;
    let merchant = RefundMerchant {
        stripe_account_id: policy.stripe_account_id.clone(),
        livemode: policy.livemode,
    };
    ManagedRefundClient::new(state.stripe_secret_key.as_deref()?, merchant).ok()
}

/// Sends reserved deposit refunds to Stripe when refund execution is enabled. Only signed
/// refund events later mark one sent; an error leaves it reserved for the next pass or an
/// operator.
async fn execute_deposit_refunds(
    state: &SharedState,
    done: &mut Vec<Finalized>,
) -> Result<(), StatusCode> {
    if !state.managed_payments.refunds_enabled {
        return Ok(());
    }
    let Some(client) = refund_client(state) else {
        return Ok(());
    };
    let store = managed_payments::store(state)?;
    for order_id in store
        .list_unbound_managed_deposit_refunds(20)
        .await
        .map_err(store_status)?
    {
        match execute_managed_deposit_refund(store, &client, order_id).await {
            Ok(_) => done.push(Finalized {
                bounty: order_id.to_string(),
                round: None,
                action: "deposit_refund_requested",
            }),
            Err(error) => {
                eprintln!("deposit refund retained for recovery: order={order_id} error={error}")
            }
        }
    }
    Ok(())
}

fn payout_view(payout: &ManagedDepositPayout) -> Value {
    json!({
        "network": payout.network,
        "bounty_contract": payout.bounty_contract,
        "round": payout.round,
        "solver": payout.solver,
        "amount_micro_usdc": payout.amount_micro_usdc,
        "token": base_network_descriptor(&payout.network)
            .map(|network| network.native_usdc_token_address.to_ascii_lowercase())
            .ok(),
        "status": if payout.tx_hash.is_some() { "paid" } else { "owed" },
        "tx_hash": payout.tx_hash,
        "block_number": payout.block_number,
    })
}

#[utoipa::path(get,path="/v1/managed-service/deposit-payouts",responses((status=200,description="Review deposits owed to solvers after reversed rejections, unpaid first: the exact USDC amount, token and solver to pay, and verified transfers"),(status=401,description="Operator credential required")))]
pub async fn list_deposit_payouts(
    State(state): State<SharedState>,
    _: Merchant,
) -> Result<Json<Value>, StatusCode> {
    let payouts = managed_payments::store(&state)?
        .list_managed_deposit_payouts(false, 200)
        .await
        .map_err(store_status)?;
    Ok(Json(json!({
        "payouts": payouts.iter().map(payout_view).collect::<Vec<_>>(),
        "rule": "Send exactly amount_micro_usdc of token to solver on network from any wallet, then record the transaction hash. It counts only after a successful receipt with that transfer and 3 confirmations.",
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayoutRequest {
    tx_hash: String,
}

async fn rpc_result(rpc: &str, method: &str, params: Value) -> Result<Value, StatusCode> {
    chain_base::ReqwestJsonRpcTransport::default()
        .post_json_value(
            rpc,
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .get("result")
        .cloned()
        .filter(|result| !result.is_null())
        .ok_or(StatusCode::CONFLICT)
}

#[utoipa::path(post,path="/v1/managed-service/deposit-payouts/{network}/{contract}",params(("network"=String,Path,description="base-mainnet or base-sepolia"),("contract"=String,Path,description="Card bounty contract")),request_body=Value,responses((status=200,description="The payout recorded after its receipt was verified on chain"),(status=401,description="Operator credential required"),(status=404,description="No deposit is owed for this bounty"),(status=409,description="Not a confirmed USDC transfer of the owed amount to the solver, or the hash already settles another payout")))]
pub async fn record_deposit_payout(
    State(state): State<SharedState>,
    _: Merchant,
    Path((network, contract)): Path<(String, String)>,
    Json(request): Json<PayoutRequest>,
) -> Result<Json<Value>, StatusCode> {
    let (order_id, _, _) = card_bounty(&state, &network, &contract).await?;
    let tx_hash = request.tx_hash.to_ascii_lowercase();
    if tx_hash.len() != 66 || tx_hash.parse::<B256>().is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = managed_payments::store(&state)?;
    let owed = store
        .managed_deposit_payout(order_id)
        .await
        .map_err(store_status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if owed.tx_hash.as_deref() == Some(tx_hash.as_str()) {
        return Ok(Json(payout_view(&owed)));
    }
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(&network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let receipt = rpc_result(&rpc, "eth_getTransactionReceipt", json!([tx_hash])).await?;
    let latest = rpc_result(&rpc, "eth_blockNumber", json!([])).await?;
    let latest = latest
        .as_str()
        .and_then(|hex| u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok())
        .ok_or(StatusCode::BAD_GATEWAY)?;
    let token = base_network_descriptor(&network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .native_usdc_token_address;
    let block = managed_orders::deposit_payout::verify_deposit_payout(
        &receipt,
        &tx_hash,
        latest,
        &token,
        &owed.solver,
        owed.amount_micro_usdc,
    )
    .map_err(|_| StatusCode::CONFLICT)?;
    let paid = store
        .record_managed_deposit_payout(order_id, &tx_hash, block)
        .await
        .map_err(store_status)?;
    Ok(Json(payout_view(&paid)))
}

/// Runs the finalizer when a review key is configured. One failing bounty never stops others
/// on the next pass.
pub(crate) fn start_finalizer(state: SharedState) {
    if state.store.is_none() || state.managed_payments.review_signer.is_none() {
        return;
    }
    tokio::spawn(async move {
        loop {
            if let Err(status) = finalize_reviews_once(&state, now()).await {
                eprintln!("managed review finalizer pass failed: {status}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(FINALIZER_INTERVAL_SECONDS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appeal_digest_is_the_standard_hash_of_the_published_typed_data() {
        // Rebuilt from the EIP-712 definition and the published type list, so any EIP-712
        // wallet signing `appeal_typed_data` produces the digest the server verifies.
        use alloy::{primitives::keccak256, sol_types::SolValue};
        let appeal = ReviewAppeal {
            bounty_contract: format!("0x{}", "ab".repeat(20)),
            round: 2,
            submission_hash: format!("0x{}", "11".repeat(32)),
            rejection_hash: format!("0x{}", "22".repeat(32)),
            solver: format!("0x{}", "5e".repeat(20)),
            reason: "The report includes the benchmark in section 3.".into(),
        };
        let bounty: Address = appeal.bounty_contract.parse().unwrap();
        let review_type = appeal_typed_data_types();
        let struct_hash = keccak256(
            (
                keccak256(review_type.as_bytes()),
                bounty,
                U256::from(2u64),
                appeal.submission_hash.parse::<B256>().unwrap(),
                appeal.rejection_hash.parse::<B256>().unwrap(),
                keccak256(appeal.reason.as_bytes()),
            )
                .abi_encode(),
        );
        let domain = keccak256(
            (
                keccak256(
                    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
                        .as_bytes(),
                ),
                keccak256("Agent Bounties".as_bytes()),
                keccak256("1".as_bytes()),
                U256::from(84_532u64),
                bounty,
            )
                .abi_encode(),
        );
        let expected =
            keccak256([&[0x19, 0x01][..], domain.as_slice(), struct_hash.as_slice()].concat());
        assert_eq!(appeal_digest("base-sepolia", &appeal).unwrap(), expected);
        // scripts/test-solver-v2.cjs hashes the website's typed data to this same vector.
        assert_eq!(
            format!("{expected:#x}"),
            "0x998a9d06ffe125ab4629f6a4ff8abc33be5501977db88f804dddcf1b6f5061a0"
        );
        let mut other = appeal.clone();
        other.reason.push('!');
        assert_ne!(appeal_digest("base-sepolia", &other).unwrap(), expected);
        assert_ne!(appeal_digest("base-mainnet", &appeal).unwrap(), expected);
    }

    /// The EIP-712 type string implied by the published `types.ReviewAppeal` list.
    fn appeal_typed_data_types() -> String {
        let review = ManagedCustomerReview {
            order_id: Uuid::nil(),
            verdict: managed_orders::review::CustomerVerdict {
                bounty_contract: format!("0x{}", "ab".repeat(20)),
                round: 1,
                submission_hash: format!("0x{}", "11".repeat(32)),
                evidence_hash: format!("0x{}", "33".repeat(32)),
                passed: false,
                reason: "The report omits the requested benchmark.".into(),
            },
            response_hash: format!("0x{}", "22".repeat(32)),
            decided_by: db::managed_service_orders::ReviewDecider::Customer,
            deadline: 1,
            recorded_at: 0,
            appeal_ends_at: Some(1),
            appeal: None,
            ruling: None,
            signed: None,
        };
        let typed = appeal_typed_data("base-sepolia", &review).unwrap();
        assert_eq!(typed["domain"]["chainId"], 84_532);
        let fields = typed["types"]["ReviewAppeal"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                format!(
                    "{} {}",
                    f["type"].as_str().unwrap(),
                    f["name"].as_str().unwrap()
                )
            })
            .collect::<Vec<_>>();
        format!(
            "{}({})",
            typed["primaryType"].as_str().unwrap(),
            fields.join(",")
        )
    }
}
