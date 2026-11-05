//! Relay an exact solver-signed submission without custody of solver funds.
use super::*;
use alloy::{
    primitives::{B256, U256},
    sol_types::{SolCall, SolStruct},
};

#[derive(Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SponsoredSubmissionRequest {
    network: String,
    submission: AutonomousBountySubmissionAuthorizationRequest,
    signature: String,
}

alloy::sol! {
    struct Submit {
        address bounty; bytes32 bountyId; address solver; uint64 round;
        bytes32 submissionHash; bytes32 evidenceHash; bytes32 policyHash; uint256 deadline;
    }
    function submitWithSignature(bytes32 submissionHash, bytes32 evidenceHash, uint256 deadline, bytes signature);
}

pub(super) fn digest(
    network: &str,
    r: &AutonomousBountySubmissionAuthorizationRequest,
) -> Result<B256, StatusCode> {
    let chain = base_network_descriptor(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .chain_id;
    let bad = |_| StatusCode::BAD_REQUEST;
    let message = Submit {
        bounty: r.bounty_contract.parse().map_err(bad)?,
        bountyId: r.bounty_id.parse().map_err(bad)?,
        solver: r.solver.parse().map_err(bad)?,
        round: r.round,
        submissionHash: r.submission_hash.parse().map_err(bad)?,
        evidenceHash: r.evidence_hash.parse().map_err(bad)?,
        policyHash: r.policy_hash.parse().map_err(bad)?,
        deadline: U256::from(r.deadline),
    };
    Ok(
        message.eip712_signing_hash(&alloy::sol_types::Eip712Domain {
            name: Some("Agent Bounties".into()),
            version: Some("1".into()),
            chain_id: Some(U256::from(chain)),
            verifying_contract: Some(message.bounty),
            salt: None,
        }),
    )
}

fn outcome<'a>(
    events: &'a [AutonomousBountyEvent],
    r: &AutonomousBountySubmissionAuthorizationRequest,
) -> Option<&'a AutonomousBountyEvent> {
    events.iter().find(|e| {
        e.kind == AutonomousBountyEventKind::SubmissionAdded
            && e.contract_address.eq_ignore_ascii_case(&r.bounty_contract)
            && e.bounty_id.eq_ignore_ascii_case(&r.bounty_id)
            && e.data["round"].as_u64() == Some(r.round)
            && [
                ("solver", &r.solver),
                ("submission_hash", &r.submission_hash),
                ("evidence_hash", &r.evidence_hash),
            ]
            .iter()
            .all(|(key, value)| {
                e.data[*key]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case(value))
            })
    })
}

fn validate_claim(
    item: &AutonomousBountyFeedItem,
    r: &AutonomousBountySubmissionAuthorizationRequest,
    now: u64,
) -> Result<(), StatusCode> {
    if item.status != "claimed"
        || !item.terms_valid
        || !item.verification_ready
        || !item.bounty_id.eq_ignore_ascii_case(&r.bounty_id)
        || !item
            .terms
            .as_ref()
            .is_some_and(|t| t.policy_hash.eq_ignore_ascii_case(&r.policy_hash))
        || r.round == 0
        || r.deadline <= now
        || r.deadline
            > now.saturating_add(chain_base::AUTONOMOUS_SUBMISSION_AUTHORIZATION_TTL_SECONDS)
    {
        return Err(StatusCode::CONFLICT);
    }
    let claim = item
        .events
        .iter()
        .filter(|e| {
            e.kind == AutonomousBountyEventKind::BountyClaimed
                && e.contract_address.eq_ignore_ascii_case(&r.bounty_contract)
                && e.bounty_id.eq_ignore_ascii_case(&r.bounty_id)
        })
        .max_by_key(|e| (e.block_number, e.log_index))
        .ok_or(StatusCode::CONFLICT)?;
    if claim.data["round"].as_u64() != Some(r.round)
        || !claim.data["solver"]
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(&r.solver))
        || claim.data["claim_expires_at"]
            .as_u64()
            .is_none_or(|v| v < r.deadline)
    {
        return Err(StatusCode::CONFLICT);
    }
    Ok(())
}

fn response(
    network: &str,
    r: &AutonomousBountySubmissionAuthorizationRequest,
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
    (status, [(header::CACHE_CONTROL, "no-store, private")], Json(serde_json::json!({
        "schema":"agent-bounties/sponsored-submission-v1", "network":network,
        "bounty_contract":r.bounty_contract, "bounty_id":r.bounty_id, "round":r.round, "solver":r.solver,
        "submission_hash":r.submission_hash, "evidence_hash":r.evidence_hash,
        "confirmed":event.is_some(), "solver_paid":false, "customer_gas_wei":"0",
        "transaction_hash":event.map(|e| e.tx_hash.as_str()).or_else(|| operation.and_then(|v| v.transaction_hash.as_deref())),
        "operation":operation, "canonical_event_id":event.map(|e| e.id),
        "next_step": if event.is_some() { "Publish the exact artifact and evidence preimages. SubmissionAdded proves submission, not payment." }
            else { "Reconcile or retry this same signed submission. Do not sign a replacement while its outcome is unresolved." }
    }))).into_response()
}

async fn reconcile(
    state: &SharedState,
    network: &str,
    r: &AutonomousBountySubmissionAuthorizationRequest,
    op: &db::SponsoredAction,
) -> Result<Option<AutonomousBountyEvent>, StatusCode> {
    let Some(hash) = op.transaction_hash.as_deref() else {
        return Ok(None);
    };
    let Some(events) = sponsored_verdict::confirmed_receipt_events(state, network, op).await?
    else {
        return Ok(None);
    };
    let Some(event) = outcome(&events, r)
        .filter(|e| e.tx_hash.eq_ignore_ascii_case(hash))
        .cloned()
    else {
        return Err(StatusCode::BAD_GATEWAY);
    };
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .upsert_autonomous_bounty_event(network, &event)
        .await
        .map_err(map_x402_db_error)?;
    store
        .finish_sponsored_action(op.id, None, "confirmed", None, None)
        .await
        .map_err(map_x402_db_error)?;
    Ok(Some(event))
}

#[utoipa::path(post, path="/v1/base/autonomous-bounties/submission-relay", request_body = SponsoredSubmissionRequest, responses((status=200,description="Exact canonical submission confirmed; no payment implied"),(status=202,description="Signed submission reserved or broadcast"),(status=401,description="Invalid solver signature"),(status=409,description="Stale or mismatched claim"),(status=503,description="Sponsorship unavailable; preserve the signed request")))]
pub(super) async fn relay(
    State(state): State<SharedState>,
    Json(request): Json<SponsoredSubmissionRequest>,
) -> Result<Response, StatusCode> {
    let network = request.network.as_str();
    if !["base-mainnet", "base-sepolia"].contains(&network) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let r = &request.submission;
    let digest = digest(network, r)?;
    let signature =
        sponsored_signature::verify(&state, network, &r.solver, digest, &request.signature).await?;
    let item = indexed_autonomous_bounty(&state, network, &r.bounty_contract).await?;
    if !item.terms_valid
        || !item.bounty_id.eq_ignore_ascii_case(&r.bounty_id)
        || !item
            .terms
            .as_ref()
            .is_some_and(|t| t.policy_hash.eq_ignore_ascii_case(&r.policy_hash))
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
    // Do not spend quota on invalid or expired fresh requests. Known hashes must
    // remain reconcilable after expiry, disabled sponsorship or a state change.
    let key = format!("submission:{digest:#x}");
    let previous = store
        .find_sponsored_action(network, &key)
        .await
        .map_err(map_x402_db_error)?;
    let mut operation = if let Some(op) = previous.filter(|v| v.transaction_hash.is_some()) {
        op
    } else {
        if !sponsored_creation::enabled(&state, network) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        validate_claim(&item, r, Utc::now().timestamp() as u64)?;
        let operation = store
            .reserve_sponsored_action(
                network,
                &key,
                &r.solver,
                state.x402_relayer.max_daily_attempts,
                state.x402_relayer.max_daily_attempts_per_contributor,
            )
            .await
            .map_err(map_x402_db_error)?;
        let calldata = submitWithSignatureCall {
            submissionHash: r
                .submission_hash
                .parse()
                .map_err(|_| StatusCode::BAD_REQUEST)?,
            evidenceHash: r
                .evidence_hash
                .parse()
                .map_err(|_| StatusCode::BAD_REQUEST)?,
            deadline: U256::from(r.deadline),
            signature: signature.clone(),
        }
        .abi_encode();
        let intent = EvmTransactionIntent {
            from: state.x402_relayer.address(),
            to: r.bounty_contract.clone(),
            value_wei: 0,
            data: format!("0x{}", hex::encode(calldata)),
            function: "submitWithSignature(bytes32,bytes32,uint256,bytes)".into(),
        };
        sponsored_verdict::broadcast(&state, network, operation, &intent, 350_000).await?
    };
    if sponsored_creation::enabled(&state, network) {
        operation = sponsored_verdict::retry_saved(&state, network, operation, r.deadline).await?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    #[test]
    fn submission_signature_binds_every_field_and_chain() {
        let key: PrivateKeySigner = format!("{:064x}", 2).parse().unwrap();
        let r = AutonomousBountySubmissionAuthorizationRequest {
            bounty_contract: format!("0x{}", "11".repeat(20)),
            bounty_id: format!("0x{}", "22".repeat(32)),
            solver: format!("{:#x}", key.address()),
            round: 1,
            submission_hash: format!("0x{}", "33".repeat(32)),
            evidence_hash: format!("0x{}", "44".repeat(32)),
            policy_hash: format!("0x{}", "55".repeat(32)),
            deadline: 2_000_000_000,
        };
        let signed = key
            .sign_hash_sync(&digest("base-mainnet", &r).unwrap())
            .unwrap();
        for field in [
            "bounty_contract",
            "bounty_id",
            "solver",
            "round",
            "submission_hash",
            "evidence_hash",
            "policy_hash",
            "deadline",
        ] {
            let mut v = serde_json::to_value(&r).unwrap();
            v[field] = match field {
                "round" => 2.into(),
                "deadline" => 2_000_000_001_u64.into(),
                "bounty_contract" | "solver" => format!("0x{}", "99".repeat(20)).into(),
                _ => format!("0x{}", "99".repeat(32)).into(),
            };
            let changed = serde_json::from_value(v).unwrap();
            assert_ne!(
                signed
                    .recover_address_from_prehash(&digest("base-mainnet", &changed).unwrap())
                    .unwrap(),
                key.address(),
                "{field}"
            );
        }
        assert_ne!(
            signed
                .recover_address_from_prehash(&digest("base-sepolia", &r).unwrap())
                .unwrap(),
            key.address()
        );
    }
}
