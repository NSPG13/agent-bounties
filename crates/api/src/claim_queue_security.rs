use crate::*;

pub(super) fn pending_next_action(candidate: &ClaimCandidate) -> &'static str {
    if candidate.claim_transaction_hash.is_none() {
        "The relay outcome is unknown and needs operator reconciliation. Poll this same request for canonical recovery; do not sign, fund, or broadcast again."
    } else {
        "The exact claim transaction remains pending. Replay this same signed request; do not sign or fund again."
    }
}

fn definitely_before_broadcast(error: &ChainBaseError) -> bool {
    matches!(
        error,
        ChainBaseError::InvalidRelayIntent(_)
            | ChainBaseError::InvalidAddress(_)
            | ChainBaseError::RelayerChainMismatch { .. }
            | ChainBaseError::RelayerGasLimitExceeded { .. }
            | ChainBaseError::RelayerFeeCapExceeded { .. }
            | ChainBaseError::RelayerInsufficientBalance { .. }
            | ChainBaseError::RelayerSimulation(_)
    )
}

/// Record the outcome before releasing the shared relayer lease. The candidate
/// already has a durable no-hash relaying marker, so crashes and generic provider
/// errors cannot turn an uncertain send into a fresh authorization attempt.
pub(super) async fn finish_broadcast(
    store: &PostgresStore,
    candidate: ClaimCandidate,
    sponsorship_id: Option<Uuid>,
    lease: Uuid,
    result: Result<Result<BaseRelayedTransaction, ChainBaseError>, tokio::time::error::Elapsed>,
) -> Result<ClaimCandidate, AgentClaimProblem> {
    let outcome = match result {
        Ok(Ok(transaction)) => {
            if let Some(sponsorship_id) = sponsorship_id {
                store.mark_atomic_sponsored_claim_broadcast(candidate.id, sponsorship_id, &transaction.tx_hash)
                    .await.map(|(candidate, _)| candidate).map_err(map_agent_claim_db_error)
            } else {
                store.mark_claim_candidate_relaying(candidate.id, &transaction.tx_hash)
                    .await.map_err(map_agent_claim_db_error)
            }
        }
        Ok(Err(error)) if definitely_before_broadcast(&error) => {
            store.fail_owned_claim_before_broadcast(candidate.id).await
                .map_err(map_agent_claim_db_error)
                .and_then(|_| Err(agent_claim_problem(
                    StatusCode::CONFLICT,
                    "claim_prebroadcast_failed",
                    "relay_claim",
                    "The claim failed a check before broadcast; no transaction was sent.",
                    "Check canonical claimability, solver balance, and relay configuration. Start a fresh candidate after the cause is resolved.",
                )))
        }
        // The provider can return an error after accepting a transaction. Preserve
        // the row and sponsorship until exact canonical evidence or operator review.
        Ok(Err(_)) | Err(_) => Ok(candidate.clone()),
    };
    let release = store
        .release_x402_relayer_lease(&candidate.network, lease)
        .await;
    let candidate = outcome?;
    release.map_err(map_agent_claim_db_error)?;
    Ok(candidate)
}

pub(super) fn problem(status: StatusCode) -> AgentClaimProblem {
    agent_claim_problem(status, "claim_authorization_required", "authorize_claim", "A verified caller must control the solver wallet and any supplied agent.", "Sign in on the first-party website or obtain a wallet session. Anonymous preparation remains available through the direct claim planner.")
}

fn stored_principal(value: &str) -> Option<service_runtime::auth::Principal> {
    use service_runtime::auth::Principal;
    if value == "operator" {
        return Some(Principal::Operator);
    }
    if let Some(id) = value.strip_prefix("account:") {
        return Some(Principal::Account {
            account_id: id.to_string(),
        });
    }
    let (chain, address) = value.strip_prefix("wallet:")?.split_once(':')?;
    Some(Principal::Wallet {
        chain_id: chain.parse().ok()?,
        address: service_runtime::auth::normalize_wallet(address).ok()?,
    })
}

/// Reconciliation precedes expiry; only freshly eligible authenticated waiters can be promoted.
pub(super) async fn maintain(
    state: &SharedState,
    network: &str,
    bounty: &str,
    item: &AutonomousBountyFeedItem,
) -> Result<(), AgentClaimProblem> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| problem(StatusCode::SERVICE_UNAVAILABLE))?;
    let candidates = store
        .active_claim_candidates(network, bounty)
        .await
        .map_err(map_agent_claim_db_error)?;
    let Some(terms) = item.terms.as_ref() else {
        return Ok(());
    };
    let policy = terms.document.agent_eligibility.clone().unwrap_or_default();
    let coordination = terms
        .document
        .claim_coordination
        .clone()
        .unwrap_or_default();
    let bond = item
        .claim_bond
        .parse::<u64>()
        .map_err(|_| problem(StatusCode::SERVICE_UNAVAILABLE))?;
    let mut eligible = Vec::new();
    for candidate in candidates {
        if let Some(claim) =
            current_indexed_claim_for_candidate(item, &terms.policy_hash, &candidate, bond)
        {
            let sponsorship = store
                .get_bond_sponsorship_for_candidate(candidate.id)
                .await
                .map_err(map_agent_claim_db_error)?;
            reconcile_indexed_claim_candidate(state, candidate, sponsorship, claim).await?;
            continue;
        }
        if candidate.claim_transaction_hash.is_some() {
            // The regular reconciliation owns canonical evidence; a timeout leaves the pending row intact.
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                reconcile_agent_native_claim(state, candidate, bond),
            )
            .await;
            continue;
        }
        if candidate.status != ClaimCandidateStatus::Waitlisted
            || require_claimable_autonomous_item(item).is_err()
        {
            continue;
        }
        let Some(key) = store
            .claim_candidate_principal(candidate.id)
            .await
            .map_err(map_agent_claim_db_error)?
        else {
            continue;
        };
        let Some(principal) = stored_principal(&key) else {
            continue;
        };
        if security_auth::authorize_verified_wallet(
            state,
            &principal,
            &candidate.solver_wallet,
            network,
        )
        .await
        .is_err()
        {
            continue;
        }
        if let Some(id) = candidate.agent_id {
            if !service_runtime::auth::authorize_resource(
                state.store.as_ref(),
                &principal,
                "agent",
                &id.to_string(),
                "write",
            )
            .await
            .map_err(|e| problem(security_auth::status(e)))?
            {
                continue;
            }
        }
        let (_, decision) = build_agent_eligibility(
            state,
            network,
            &item.creator,
            &candidate.solver_wallet,
            candidate.agent_id,
            &policy,
        )
        .await?;
        if decision.eligible {
            eligible.push(candidate.id);
        }
    }
    store
        .maintain_owned_claim_queue(
            network,
            bounty,
            coordination.exclusive_claim_seconds,
            &eligible,
        )
        .await
        .map_err(map_agent_claim_db_error)
}

pub(super) fn start_worker(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let Some(store) = state.store.as_ref() else {
                continue;
            };
            let Ok(batch) = store.claim_maintenance_batch().await else {
                continue;
            };
            for (network, bounty) in batch {
                let Ok(item) = indexed_autonomous_bounty(&state, &network, &bounty).await else {
                    continue;
                };
                if maintain(&state, &network, &bounty, &item).await.is_err() {
                    eprintln!(
                        "{}",
                        serde_json::json!({"component":"claim_queue","action":"maintenance","outcome":"deferred"})
                    );
                }
            }
        }
    });
}

#[utoipa::path(post,path="/v1/base/autonomous-bounties/claims/{id}/withdraw",params(("id"=Uuid,Path,description="Claim candidate UUID")),responses((status=200,description="Hosted reservation withdrawn; signed chain authorizations remain subject to their original expiry"),(status=401,description="Authentication required"),(status=404,description="Owned candidate not found"),(status=409,description="A relay is already pending")))]
pub(super) async fn withdraw(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<ClaimCandidate>, StatusCode> {
    let principal = secure_operations::authenticated(&state, &headers).await?;
    let key = principal.key().ok_or(StatusCode::UNAUTHORIZED)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if store
        .claim_candidate_principal(id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .as_deref()
        != Some(&key)
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let candidate = store
        .get_claim_candidate(id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)?;
    security_auth::authorize_verified_wallet(
        &state,
        &principal,
        &candidate.solver_wallet,
        &candidate.network,
    )
    .await?;
    let candidate = store
        .withdraw_owned_claim_candidate(id, &key)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    if let Ok(item) =
        indexed_autonomous_bounty(&state, &candidate.network, &candidate.bounty_contract).await
    {
        let _ = maintain(
            &state,
            &candidate.network,
            &candidate.bounty_contract,
            &item,
        )
        .await;
    }
    Ok(Json(candidate))
}
