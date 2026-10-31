//! Exact creator-review signatures; the platform pays only the transaction gas.
use super::*;
use alloy::{
    primitives::{B256, U256},
    sol_types::SolStruct,
};

#[derive(Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SponsoredVerdictRequest {
    network: String,
    attestation: AutonomousVerificationAttestationRequest,
    signature: String,
}
alloy::sol! {
    struct VerificationAttestation {
        address bounty; bytes32 bountyId; uint64 round; address verifier;
        bytes32 submissionHash; bytes32 evidenceHash; bytes32 policyHash;
        bool passed; bytes32 responseHash; uint256 deadline;
    }
}
pub(super) fn digest(
    network: &str,
    r: &AutonomousVerificationAttestationRequest,
) -> Result<B256, StatusCode> {
    digest_for_version(network, r, "1")
}
/// Autonomous-v2 bounties use EIP-712 domain version "2" over the same attestation struct.
pub(super) fn digest_for_version(
    network: &str,
    r: &AutonomousVerificationAttestationRequest,
    version: &'static str,
) -> Result<B256, StatusCode> {
    let chain = base_network_descriptor(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .chain_id;
    let bad = |_| StatusCode::BAD_REQUEST;
    let message = VerificationAttestation {
        bounty: r.bounty_contract.parse().map_err(bad)?,
        bountyId: r.bounty_id.parse().map_err(bad)?,
        round: r.round,
        verifier: r.verifier.parse().map_err(bad)?,
        submissionHash: r.submission_hash.parse().map_err(bad)?,
        evidenceHash: r.evidence_hash.parse().map_err(bad)?,
        policyHash: r.policy_hash.parse().map_err(bad)?,
        passed: r.passed,
        responseHash: r.response_hash.parse().map_err(bad)?,
        deadline: U256::from(r.deadline),
    };
    Ok(
        message.eip712_signing_hash(&alloy::sol_types::Eip712Domain {
            name: Some("Agent Bounties".into()),
            version: Some(version.into()),
            chain_id: Some(U256::from(chain)),
            verifying_contract: Some(message.bounty),
            salt: None,
        }),
    )
}

fn outcome<'a>(
    events: &'a [AutonomousBountyEvent],
    r: &AutonomousVerificationAttestationRequest,
) -> Option<&'a AutonomousBountyEvent> {
    let bound = |e: &&AutonomousBountyEvent| {
        e.contract_address.eq_ignore_ascii_case(&r.bounty_contract)
            && e.bounty_id.eq_ignore_ascii_case(&r.bounty_id)
            && e.data["round"].as_u64() == Some(r.round)
    };
    let hash_matches = |e: &AutonomousBountyEvent| {
        e.data["submission_hash"]
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(&r.submission_hash))
            && e.data["evidence_hash"]
                .as_str()
                .is_some_and(|v| v.eq_ignore_ascii_case(&r.evidence_hash))
    };
    let submission = events
        .iter()
        .filter(bound)
        .find(|e| e.kind == AutonomousBountyEventKind::SubmissionAdded && hash_matches(e))?;
    events.iter().filter(bound).find(|e| {
        e.kind
            == if r.passed {
                AutonomousBountyEventKind::BountySettled
            } else {
                AutonomousBountyEventKind::SubmissionRejected
            }
            && e.data["solver"] == submission.data["solver"]
            && (e.block_number, e.log_index) > (submission.block_number, submission.log_index)
            && (!r.passed || hash_matches(e))
    })
}

fn response(
    network: &str,
    r: &AutonomousVerificationAttestationRequest,
    operation: Option<&db::SponsoredAction>,
    event: Option<&AutonomousBountyEvent>,
) -> Response {
    let status = if event.is_some() {
        StatusCode::OK
    } else if operation
        .is_some_and(|v| v.status == "failed" || v.attempts >= 3 && v.transaction_hash.is_none())
    {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::ACCEPTED
    };
    (status, [(header::CACHE_CONTROL,"no-store, private")], Json(serde_json::json!({
        "schema":"agent-bounties/sponsored-verdict-v1", "network":network,"bounty_contract":r.bounty_contract,"round":r.round,
        "confirmed":event.is_some(),"solver_paid":event.is_some_and(|e| e.kind == AutonomousBountyEventKind::BountySettled),
        "customer_gas_wei":"0", "transaction_hash":event.map(|e| e.tx_hash.as_str()).or_else(|| operation.and_then(|v| v.transaction_hash.as_deref())),
        "operation":operation,"canonical_event_id":event.map(|e| e.id),
        "next_step":"Reconcile this exact verdict. Retry the same signed request after an unavailable or lost reply; never sign a different verdict while this one is unresolved. Only canonical BountySettled proves solver payment."
    }))).into_response()
}

pub(super) async fn confirmed_receipt(
    state: &SharedState,
    network: &str,
    op: &db::SponsoredAction,
) -> Result<Option<RpcTransactionReceipt>, StatusCode> {
    let Some(hash) = op.transaction_hash.as_deref() else {
        return Ok(None);
    };
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(receipt) = fetch_transaction_receipt(&rpc, hash, 1)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .result
    else {
        return Ok(None);
    };
    let Some(block) = receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
    else {
        return Ok(None);
    };
    if !receipt.transaction_hash.eq_ignore_ascii_case(hash) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let latest = fetch_block_number(&rpc, 2)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if block > latest
        || latest.saturating_sub(block).saturating_add(1) < state.x402_relayer.confirmations
    {
        return Ok(None);
    }
    let canonical = fetch_exact_block_identity(&rpc, block, 3)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if !receipt
        .block_hash
        .as_ref()
        .is_some_and(|h| h.eq_ignore_ascii_case(&canonical.hash))
    {
        return Ok(None);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let success = receipt.succeeded().map_err(|_| StatusCode::BAD_GATEWAY)?;
    if success.is_none() {
        return Ok(None);
    }
    if success == Some(false) {
        store.finish_sponsored_action(op.id,None,"failed",None,Some("The recorded transaction reverted. Reconcile the canonical round before any replacement action.")).await.map_err(map_x402_db_error)?;
        return Ok(None);
    }
    let logs = receipt
        .logs_to_evm_logs()
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if logs
        .iter()
        .any(|log| log.block_number != block || !log.tx_hash.eq_ignore_ascii_case(hash))
    {
        return Err(StatusCode::BAD_GATEWAY);
    }
    Ok(Some(receipt))
}

pub(super) async fn confirmed_receipt_events(
    state: &SharedState,
    network: &str,
    op: &db::SponsoredAction,
) -> Result<Option<Vec<AutonomousBountyEvent>>, StatusCode> {
    let Some(receipt) = confirmed_receipt(state, network, op).await? else {
        return Ok(None);
    };
    decode_autonomous_bounty_logs(
        receipt
            .logs_to_evm_logs()
            .map_err(|_| StatusCode::BAD_GATEWAY)?,
    )
    .map(Some)
    .map_err(|_| StatusCode::BAD_GATEWAY)
}

async fn reconcile(
    state: &SharedState,
    network: &str,
    r: &AutonomousVerificationAttestationRequest,
    op: &db::SponsoredAction,
) -> Result<Option<AutonomousBountyEvent>, StatusCode> {
    let Some(hash) = op.transaction_hash.as_deref() else {
        return Ok(None);
    };
    let Some(events) = confirmed_receipt_events(state, network, op).await? else {
        return Ok(None);
    };
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let item = indexed_autonomous_bounty(state, network, &r.bounty_contract).await?;
    let relevant = events
        .into_iter()
        .filter(|e| {
            e.contract_address.eq_ignore_ascii_case(&r.bounty_contract)
                && e.bounty_id.eq_ignore_ascii_case(&r.bounty_id)
                && e.tx_hash.eq_ignore_ascii_case(hash)
        })
        .collect::<Vec<_>>();
    let combined = item
        .events
        .into_iter()
        .chain(relevant.iter().cloned())
        .collect::<Vec<_>>();
    let Some(result) = outcome(&combined, r)
        .filter(|e| e.tx_hash.eq_ignore_ascii_case(hash))
        .cloned()
    else {
        return Err(StatusCode::BAD_GATEWAY);
    };
    for event in relevant {
        store
            .upsert_autonomous_bounty_event(network, &event)
            .await
            .map_err(map_x402_db_error)?;
    }
    store
        .finish_sponsored_action(op.id, None, "confirmed", None, None)
        .await
        .map_err(map_x402_db_error)?;
    Ok(Some(result))
}

#[utoipa::path(post, path="/v1/base/autonomous-bounties/creator-verdict-relay", request_body = SponsoredVerdictRequest, responses((status=200,description="Exact canonical creator verdict confirmed"),(status=202,description="Signed creator verdict reserved or broadcast"),(status=401,description="Invalid creator signature"),(status=409,description="Stale or mismatched review"),(status=503,description="Sponsorship unavailable; preserve the same signed request")))]
pub(super) async fn relay(
    State(state): State<SharedState>,
    Json(request): Json<SponsoredVerdictRequest>,
) -> Result<Response, StatusCode> {
    let network = request.network.as_str();
    if !["base-mainnet", "base-sepolia"].contains(&network) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let r = &request.attestation;
    let digest = digest(network, r)?;
    let signature =
        sponsored_signature::verify(&state, network, &r.verifier, digest, &request.signature)
            .await?;
    let item = indexed_autonomous_bounty(&state, network, &r.bounty_contract).await?;
    let terms = item.terms.as_ref().ok_or(StatusCode::CONFLICT)?;
    if !item.terms_valid
        || terms.document.benchmark["engine"] != "creator_review_v1"
        || !item.creator.eq_ignore_ascii_case(&r.verifier)
        || !terms.policy_hash.eq_ignore_ascii_case(&r.policy_hash)
    {
        return Err(StatusCode::CONFLICT);
    }
    if let Some(event) = outcome(&item.events, r) {
        return Ok(response(network, r, None, Some(event)));
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let key = format!("creator-verdict:{digest:#x}");
    let previous = store
        .find_sponsored_action(network, &key)
        .await
        .map_err(map_x402_db_error)?;
    let mut operation = if let Some(operation) = previous.filter(|v| v.transaction_hash.is_some()) {
        operation
    } else {
        if !sponsored_creation::enabled(&state, network) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let now = Utc::now().timestamp() as u64;
        validate_attestation_request_against_feed(&item, r, now)
            .map_err(|_| StatusCode::CONFLICT)?;
        if r.deadline > now.saturating_add(600) {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        store
            .reserve_sponsored_action(
                network,
                &key,
                &r.verifier,
                state.x402_relayer.max_daily_attempts,
                state.x402_relayer.max_daily_attempts_per_contributor,
            )
            .await
            .map_err(map_x402_db_error)?
    };
    if operation.transaction_hash.is_none() {
        let now = Utc::now().timestamp() as u64;
        validate_attestation_request_against_feed(&item, r, now)
            .map_err(|_| StatusCode::CONFLICT)?;
        if r.deadline > now.saturating_add(600) {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let relayer = state
            .x402_relayer
            .relayer
            .as_ref()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let intent = configured_autonomous_planner(network)?
            .plan_attestation_settlement(
                &r.bounty_contract,
                Some(&relayer.address()),
                &[AutonomousSignedAttestation {
                    verifier: r.verifier.clone(),
                    passed: r.passed,
                    response_hash: r.response_hash.clone(),
                    deadline: r.deadline,
                    signature: format!("0x{}", hex::encode(&signature)),
                }],
            )
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        operation = broadcast(&state, network, operation, &intent, 700_000).await?;
    }
    if sponsored_creation::enabled(&state, network) {
        operation = retry_saved(&state, network, operation, r.deadline).await?;
    }
    // Once a hash exists, receipt outages must still return that exact hash.
    let event = tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        reconcile(&state, network, r, &operation),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
    operation = store
        .sponsored_action(operation.id)
        .await
        .map_err(map_x402_db_error)?;
    Ok(response(network, r, Some(&operation), event.as_ref()))
}

pub(super) async fn retry_x402_saved(
    state: &SharedState,
    attempt: &X402RelayAttempt,
) -> Result<(), StatusCode> {
    if attempt.status != X402RelayStatus::Broadcast
        || attempt.authorization_valid_before <= Utc::now().timestamp() as u64
        || !state.x402_relayer.enabled
    {
        return Ok(());
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(saved) = store
        .relay_outbox(attempt.id, &attempt.network)
        .await
        .map_err(map_x402_db_error)?
    else {
        return Ok(());
    };
    let Some(lease) = store
        .acquire_saved_relayer_lease(
            &attempt.network,
            attempt.id,
            state.x402_relayer.lease_seconds,
        )
        .await
        .map_err(map_x402_db_error)?
    else {
        return Ok(());
    };
    let result = send_saved(state, &attempt.network, attempt.id, &saved, true, lease).await;
    let released = store
        .release_x402_relayer_lease(&attempt.network, lease)
        .await
        .map_err(map_x402_db_error);
    result?;
    released?;
    Ok(())
}

/// Explicit POST retry only. Status reads never submit transactions.
pub(super) async fn retry_saved(
    state: &SharedState,
    network: &str,
    operation: db::SponsoredAction,
    valid_before: u64,
) -> Result<db::SponsoredAction, StatusCode> {
    if valid_before <= Utc::now().timestamp() as u64
        || operation.status == "confirmed"
        || operation.status == "failed"
    {
        return Ok(operation);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(saved) = store
        .relay_outbox(operation.id, network)
        .await
        .map_err(map_x402_db_error)?
    else {
        return Ok(operation);
    };
    let Some(lease) = store
        .acquire_saved_relayer_lease(network, operation.id, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?
    else {
        return Ok(operation);
    };
    let result = send_saved(state, network, operation.id, &saved, true, lease).await;
    let released = store
        .release_x402_relayer_lease(network, lease)
        .await
        .map_err(map_x402_db_error);
    result?;
    released?;
    store
        .sponsored_action(operation.id)
        .await
        .map_err(map_x402_db_error)
}
pub(super) async fn send_saved(
    state: &SharedState,
    network: &str,
    id: Uuid,
    saved: &chain_base::PreparedBaseRelayTransaction,
    retransmission: bool,
    lease: Uuid,
) -> Result<(), StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !store
        .relay_outbox_send_ready(id)
        .await
        .map_err(map_x402_db_error)?
    {
        return Ok(());
    }
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|r| r.has_gas_budget())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !saved
        .transaction
        .relayer
        .eq_ignore_ascii_case(&relayer.address())
        || saved.transaction.max_fee_per_gas_wei > state.x402_relayer.max_fee_per_gas_wei
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let raw = hex::decode(
        saved
            .signed_transaction
            .strip_prefix("0x")
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if format!("{:#x}", alloy::primitives::keccak256(raw)) != saved.transaction.tx_hash {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let chain = base_network_descriptor(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .chain_id;
    let result = tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.relay_stage_timeout_seconds()),
        async {
            use chain_base::JsonRpcTransport;
            let response = chain_base::ReqwestJsonRpcTransport::default()
                .post_json_value(
                    &rpc,
                    &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}),
                )
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            if response["jsonrpc"] != "2.0"
                || response["id"] != 1
                || response.get("error").is_some()
                || response["result"] != format!("0x{chain:x}")
            {
                return Err(StatusCode::BAD_GATEWAY);
            }
            if fetch_transaction_receipt(&rpc, &saved.transaction.tx_hash, 2)
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
                .result
                .is_some()
            {
                return Ok(());
            }
            if retransmission {
                relayer
                    .reserve_prepared_retransmission(&rpc, chain, saved)
                    .await
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            }
            if !store
                .claim_relay_outbox_send(id)
                .await
                .map_err(map_x402_db_error)?
            {
                return Ok(());
            }
            if !store
                .relayer_lease_has_time(
                    network,
                    lease,
                    state.x402_relayer.relay_stage_timeout_seconds() + 2,
                )
                .await
                .map_err(map_x402_db_error)?
            {
                return Ok(());
            }
            // Hash and exact bytes already exist durably. A lost RPC response cannot
            // change the nonce or turn the request into an untracked failure.
            let _ =
                chain_base::broadcast_signed_transaction(&rpc, &saved.signed_transaction, 3).await;
            Ok::<(), StatusCode>(())
        },
    )
    .await;
    match result {
        Ok(result) => result,
        Err(_) => Ok(()),
    }
}
pub(super) async fn broadcast(
    state: &SharedState,
    network: &str,
    operation: db::SponsoredAction,
    intent: &EvmTransactionIntent,
    gas_limit: u64,
) -> Result<db::SponsoredAction, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|r| r.has_gas_budget())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let lease = store
        .acquire_x402_relayer_lease(network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let result:Result<db::SponsoredAction,StatusCode>=async {
        if let Some(saved)=store.relay_outbox(operation.id,network).await.map_err(map_x402_db_error)? {
            store.repair_sponsored_outbox_marker(operation.id).await.map_err(map_x402_db_error)?;
            send_saved(state,network,operation.id,&saved,true,lease).await?;
            return store.sponsored_action(operation.id).await.map_err(map_x402_db_error);
        }
        if operation.transaction_hash.is_some() || !store.claim_sponsored_action(operation.id,lease,state.x402_relayer.lease_seconds).await.map_err(map_x402_db_error)? {
            return store.sponsored_action(operation.id).await.map_err(map_x402_db_error);
        }
        let prepared=tokio::time::timeout(Duration::from_secs(state.x402_relayer.relay_stage_timeout_seconds()),relayer.prepare_signed_transaction(&rpc,base_network_descriptor(network).unwrap().chain_id,intent,gas_limit,state.x402_relayer.max_fee_per_gas_wei)).await;
        let saved=match prepared {
            Ok(Ok(saved))=>saved,
            _=>return store.finish_sponsored_action(operation.id,Some(lease),"failed",None,Some("Gas sponsorship preparation failed. Preserve and retry this exact request; no transaction was broadcast.")).await.map_err(map_x402_db_error),
        };
        store.save_relay_outbox(operation.id,network,&saved).await.map_err(map_x402_db_error)?;
        store.finish_sponsored_action(operation.id,Some(lease),"broadcast",Some(&saved.transaction.tx_hash),None).await.map_err(map_x402_db_error)?;
        send_saved(state,network,operation.id,&saved,false,lease).await?;
        store.sponsored_action(operation.id).await.map_err(map_x402_db_error)
    }.await;
    let released = store
        .release_x402_relayer_lease(network, lease)
        .await
        .map_err(map_x402_db_error);
    let operation = result?;
    released?;
    Ok(operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    fn request() -> AutonomousVerificationAttestationRequest {
        AutonomousVerificationAttestationRequest {
            bounty_contract: format!("0x{}", "11".repeat(20)),
            bounty_id: format!("0x{}", "22".repeat(32)),
            round: 1,
            verifier: format!("0x{}", "33".repeat(20)),
            submission_hash: format!("0x{}", "44".repeat(32)),
            evidence_hash: format!("0x{}", "55".repeat(32)),
            policy_hash: format!("0x{}", "66".repeat(32)),
            passed: true,
            response_hash: format!("0x{}", "77".repeat(32)),
            deadline: 2_000_000_000,
        }
    }
    #[test]
    fn review_authority_binds_every_field_and_chain() {
        let key: PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
        let mut r = request();
        r.verifier = format!("{:#x}", key.address());
        let signature = key
            .sign_hash_sync(&digest("base-mainnet", &r).unwrap())
            .unwrap();
        for field in [
            "bounty_contract",
            "bounty_id",
            "round",
            "verifier",
            "submission_hash",
            "evidence_hash",
            "policy_hash",
            "passed",
            "response_hash",
            "deadline",
        ] {
            let mut value = serde_json::to_value(&r).unwrap();
            value[field] = match field {
                "round" => 2.into(),
                "deadline" => 2_000_000_001_u64.into(),
                "passed" => false.into(),
                "bounty_contract" | "verifier" => format!("0x{}", "99".repeat(20)).into(),
                _ => format!("0x{}", "99".repeat(32)).into(),
            };
            let changed = serde_json::from_value(value).unwrap();
            assert_ne!(
                signature
                    .recover_address_from_prehash(&digest("base-mainnet", &changed).unwrap())
                    .unwrap(),
                key.address(),
                "{field}"
            );
        }
        assert_ne!(
            signature
                .recover_address_from_prehash(&digest("base-sepolia", &r).unwrap())
                .unwrap(),
            key.address()
        );
    }
    #[tokio::test]
    async fn stored_confirmation_and_unrelated_rounds_never_prove_payment() {
        let r = request();
        let mut submitted = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: "fixture".into(),
            tx_hash: format!("0x{}", "88".repeat(32)),
            block_number: 1,
            log_index: 0,
            contract_address: r.bounty_contract.clone(),
            bounty_id: r.bounty_id.clone(),
            kind: AutonomousBountyEventKind::SubmissionAdded,
            data: serde_json::json!({"round":1,"solver":format!("0x{}","ab".repeat(20)),"submission_hash":r.submission_hash,"evidence_hash":r.evidence_hash}),
            occurred_at: Utc::now(),
        };
        let mut settled = submitted.clone();
        settled.block_number = 2;
        settled.kind = AutonomousBountyEventKind::BountySettled;
        assert!(outcome(&[submitted.clone(), settled.clone()], &r).is_some());
        settled.data["round"] = 2.into();
        assert!(outcome(&[submitted.clone(), settled.clone()], &r).is_none());
        settled.data["round"] = 1.into();
        submitted.data["evidence_hash"] = format!("0x{}", "cd".repeat(32)).into();
        assert!(outcome(&[submitted, settled], &r).is_none());
        let record = db::SponsoredAction {
            id: Uuid::new_v4(),
            status: "confirmed".into(),
            attempts: 1,
            transaction_hash: Some(format!("0x{}", "88".repeat(32))),
            error_message: None,
        };
        let reply = response("base-mainnet", &r, Some(&record), None);
        let json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(reply.into_body(), 10000)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(json["confirmed"], false);
        assert_eq!(json["solver_paid"], false);
    }
}
