//! Account-bound coordination. Provider delivery and relay broadcasts are never payment evidence.
use super::*;
use axum::Extension;
use serde_json::{json, Value};

mod providers;

#[utoipa::path(get, path = "/v1/wallet-funding/capabilities", responses((status = 200, description = "Public rollout flags; flags alone never prove sponsorship eligibility")))]
pub(crate) async fn capabilities() -> Json<Value> {
    Json(
        json!({"guided_topup":env_flag("ENABLE_GUIDED_WALLET_TOPUP"),"sponsored_creation":env_flag("ENABLE_SPONSORED_POSTING_CREATION")}),
    )
}

pub fn router(auth: site_auth::SiteAuthService, state: SharedState) -> Router {
    Router::new()
        .route(
            "/v1/site-auth/posting-funding/:operation/readiness",
            post(readiness),
        )
        .route(
            "/v1/site-auth/posting-funding/:operation/creation-relay",
            post(relay_creation).get(creation_status),
        )
        .route(
            "/v1/site-auth/posting-funding/:operation/topup-options",
            post(topup_options),
        )
        .route(
            "/v1/site-auth/posting-funding/:operation/topups",
            post(prepare_topup).get(topup_status),
        )
        .route(
            "/v1/site-auth/posting-funding/:operation/topups/action",
            post(topup_action),
        )
        .layer(auth.funding_cors())
        .layer(Extension(auth))
        .with_state(state)
}

fn response(value: Value) -> Response {
    let mut result = Json(value).into_response();
    result
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    result
}

async fn draft(
    state: &SharedState,
    owner: &str,
    operation: Uuid,
) -> Result<db::SitePostingDraft, StatusCode> {
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .get_site_posting_draft(owner, operation)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)
}

fn saved_budget(saved: &db::SitePostingDraft) -> Result<u64, StatusCode> {
    let draft = &saved.draft["draft"];
    providers::parse_units(&draft["solver_reward_usdc"])
        .ok()
        .and_then(|solver| {
            providers::parse_units(&draft["verifier_reward_usdc"])
                .ok()
                .and_then(|verifier| solver.checked_add(verifier))
        })
        .filter(|total| *total > 0)
        .ok_or(StatusCode::CONFLICT)
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadinessInput {
    wallet: String,
    required_usdc_units: String,
}

#[utoipa::path(post, path = "/v1/site-auth/posting-funding/{operation}/readiness", responses((status = 200, description = "Account wallet balances and action-specific sponsorship preflight; exact signed simulation remains required")), params(("operation" = Uuid, Path, description = "Saved account posting operation")), security(("site_account_session" = [])),
    request_body = ReadinessInput
)]
async fn readiness(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(operation): Path<Uuid>,
    Json(input): Json<ReadinessInput>,
) -> Result<Response, StatusCode> {
    let owner = auth.posting_owner(&headers, true)?;
    let saved = draft(&state, &owner, operation).await?;
    let wallet = normalize_evm_address(&input.wallet).map_err(|_| StatusCode::BAD_REQUEST)?;
    let required = input
        .required_usdc_units
        .parse::<u64>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let required = saved_budget(&saved).unwrap_or(required);
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let caps = &state.x402_relayer;
    let blocker = if saved.approved_draft_hash.as_deref() != Some(&saved.draft_hash)
        || saved_budget(&saved).is_err()
    {
        Some("review_required")
    } else if !env_flag("ENABLE_SPONSORED_POSTING_CREATION")
        || !caps.enabled
        || caps.relayer.is_none()
    {
        Some("sponsorship_unavailable")
    } else if required < caps.min_amount || required > caps.max_amount {
        Some("amount_outside_relay_caps")
    } else if !store
        .x402_quota_available(
            "base-mainnet",
            &wallet,
            caps.max_daily_attempts,
            caps.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?
    {
        Some("relay_quota_exhausted")
    } else {
        None
    };
    let balances = read_base_balances(&state, &wallet).await;
    let (usdc, eth, block) = match balances {
        Ok(value) => value,
        Err(_) => {
            return Ok(response(
                json!({"operation_id":operation,"wallet":wallet,"guided_topup_enabled":env_flag("ENABLE_GUIDED_WALLET_TOPUP"),"sponsored_creation_enabled":env_flag("ENABLE_SPONSORED_POSTING_CREATION"),"gas_sponsorship":{"status":"unavailable","eligible":false,"can_request_authorization":false},"blocker":"balance_service_unavailable","next_action":"Retry the balance check. Your saved review is unchanged."}),
            ))
        }
    };
    let reserve_ready = if blocker.is_none() {
        relay_reserve_ready(&state).await.unwrap_or(false)
    } else {
        false
    };
    let contract_wallet = rpc(&state, "eth_getCode", json!([wallet, "latest"]))
        .await
        .map(|v| v.as_str() != Some("0x"))
        .unwrap_or(true);
    let blocker = blocker.or(if contract_wallet {
        Some("wallet_signature_route_unavailable")
    } else {
        None
    });
    let blocker = blocker.or(if !reserve_ready {
        Some("relay_reserve_unavailable")
    } else {
        None
    });
    Ok(response(
        json!({"operation_id":operation,"wallet":wallet,"network":"base-mainnet","chain_id":8453,
        "required_usdc_units":required.to_string(),"usdc_balance_units":usdc.to_string(),"eth_balance_wei":eth.to_string(),"observed_block":block,
        "usdc_shortfall_units":u128::from(required).saturating_sub(usdc).to_string(),
        "guided_topup_enabled":env_flag("ENABLE_GUIDED_WALLET_TOPUP"),"sponsored_creation_enabled":env_flag("ENABLE_SPONSORED_POSTING_CREATION"),
        "gas_sponsorship":{"action":"create_bounty","status":if blocker.is_none(){"awaiting_authorization"}else{"unavailable"},
            "eligible":false,"can_request_authorization":blocker.is_none(),"gas_payer":if blocker.is_none(){"relay"}else{"undecided"},
            "simulation":"requires_exact_signed_authorization","max_gas":sponsored_creation::CREATION_MAX_GAS,"max_fee_per_gas_wei":caps.max_fee_per_gas_wei.to_string(),
            "message":if blocker.is_none(){"The relay can check your exact signed request and pay gas if simulation succeeds. You do not need ETH for this route."}else{"Sponsorship is temporarily unavailable. Keep this review and retry; you will not be charged gas."}},
        "blocker":if usdc < u128::from(required){Some("usdc_shortfall")}else{blocker},
        "next_action":if usdc < u128::from(required){"Add the USDC shortfall to this wallet."}else if blocker.is_none(){"Review and authorize the exact bounty funding; the relay checks simulation before sending."}else{"Keep this saved review and recheck sponsorship."}}),
    ))
}

async fn rpc(state: &SharedState, method: &str, params: Value) -> Result<Value, StatusCode> {
    let (_, url) = state
        .base_rpc_urls
        .resolve("base-mainnet")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let value: Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .error_for_status()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .json()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    value
        .get("result")
        .cloned()
        .filter(|v| !v.is_null())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

fn hex_amount(value: &Value) -> Result<u128, StatusCode> {
    u128::from_str_radix(
        value
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .ok_or(StatusCode::BAD_GATEWAY)?,
        16,
    )
    .map_err(|_| StatusCode::BAD_GATEWAY)
}

async fn read_base_balances(
    state: &SharedState,
    wallet: &str,
) -> Result<(u128, u128, Value), StatusCode> {
    if hex_amount(&rpc(state, "eth_chainId", json!([])).await?)? != 8453 {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let block = rpc(state, "eth_blockNumber", json!([])).await?;
    let usdc = base_network_descriptor("base-mainnet")
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .native_usdc_token_address;
    let balance = rpc(
        state,
        "eth_call",
        json!([{"to":usdc,"data":format!("0x70a08231{:0>64}",&wallet[2..])},block]),
    )
    .await?;
    let eth = rpc(state, "eth_getBalance", json!([wallet, block])).await?;
    Ok((hex_amount(&balance)?, hex_amount(&eth)?, block))
}

async fn relay_reserve_ready(state: &SharedState) -> Result<bool, StatusCode> {
    let caps = &state.x402_relayer;
    let address = caps.address().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let balance = hex_amount(&rpc(state, "eth_getBalance", json!([address, "pending"])).await?)?;
    let gas_price = hex_amount(&rpc(state, "eth_gasPrice", json!([])).await?)?;
    let budget = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .gas_sponsor_budget_status()
        .await
        .map_err(map_x402_db_error)?;
    Ok(caps.relayer.as_ref().is_some_and(|r| r.has_gas_budget())
        && budget.remaining_micro_usdc > 0
        && gas_price <= caps.max_fee_per_gas_wei
        && balance
            >= u128::from(sponsored_creation::CREATION_MAX_GAS)
                .saturating_mul(caps.max_fee_per_gas_wei))
}

#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreationInput {
    draft_hash: String,
    legal_acceptance_id: Uuid,
    create: AutonomousBountyCreate,
    signature: AutonomousBountyAuthorizationSignature,
}

#[utoipa::path(post, path = "/v1/site-auth/posting-funding/{operation}/creation-relay", responses((status = 200, description = "Submit the exact approved creation and person-signed authorization to the bounded relay once")), params(("operation" = Uuid, Path, description = "Saved account posting operation")), security(("site_account_session" = [])),
    request_body = CreationInput
)]
async fn relay_creation(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(operation): Path<Uuid>,
    Json(input): Json<CreationInput>,
) -> Result<Response, StatusCode> {
    let owner = auth.posting_owner(&headers, true)?;
    let saved = draft(&state, &owner, operation).await?;
    if saved.approved_draft_hash.as_deref() != Some(&input.draft_hash)
        || saved.draft_hash != input.draft_hash
    {
        return Err(StatusCode::CONFLICT);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    // An existing attempt is reconciled even after a flag change or expiration.
    if let Some(id) = store
        .posting_creation_relay(&owner, operation)
        .await
        .map_err(map_x402_db_error)?
    {
        if store
            .get_x402_relay_attempt(id)
            .await
            .map_err(map_x402_db_error)?
            .is_some_and(|a| a.status != X402RelayStatus::Prepared)
        {
            return creation_status_inner(&state, &owner, operation).await;
        }
    }
    let caps = &state.x402_relayer;
    if !env_flag("ENABLE_SPONSORED_POSTING_CREATION") || !caps.enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let relayer = caps
        .relayer
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let terms = require_autonomous_creation_terms(&state, "base-mainnet", &input.create).await?;
    if !store
        .has_posting_legal_acceptance(
            input.legal_acceptance_id,
            &input.create.creator,
            LEGAL_TERMS_VERSION,
            LEGAL_PRIVACY_VERSION,
            &legal_statement_hash(),
        )
        .await
        .map_err(map_x402_db_error)?
    {
        return Err(StatusCode::PRECONDITION_REQUIRED);
    }
    validate_review_binding(&saved, &terms, &input.create)?;

    let now = Utc::now().timestamp().max(0) as u64;
    if input.create.funding_deadline <= now {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let amount =
        u64::try_from(input.create.initial_funding.amount).map_err(|_| StatusCode::BAD_REQUEST)?;
    if amount < caps.min_amount
        || amount > caps.max_amount
        || input
            .create
            .solver_reward
            .amount
            .checked_add(input.create.verifier_reward.amount)
            != Some(input.create.initial_funding.amount)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let plan = configured_autonomous_new_risk_planner("base-mainnet")?
        .plan_authorized_creation(
            "base-mainnet",
            &input.create,
            &input.signature,
            Some(&relayer.address()),
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&input.create).map_err(|_| StatusCode::BAD_REQUEST)?,
    ));
    let relay_id = store
        .reserve_posting_creation(
            &owner,
            operation,
            &input.draft_hash,
            &fingerprint,
            &json!(input),
        )
        .await
        .map_err(map_x402_db_error)?;
    let attempt = store
        .reserve_x402_relay_attempt(
            &NewX402RelayAttempt {
                id: relay_id,
                idempotency_key: format!("posting:{owner}:{operation}"),
                network: "base-mainnet".into(),
                bounty_contract: plan.predicted_bounty_contract.clone(),
                contributor: input.create.creator.clone(),
                amount,
                authorization_nonce: input.create.creation_nonce.clone(),
                authorization_valid_before: input.create.funding_deadline,
                request_fingerprint: fingerprint,
                relayer_address: relayer.address(),
            },
            caps.max_daily_attempts,
            caps.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    if attempt.id != relay_id {
        return Err(StatusCode::CONFLICT);
    }
    if attempt.status != X402RelayStatus::Prepared {
        return creation_status_inner(&state, &owner, operation).await;
    }
    process_creation(&state, attempt, &plan, &terms).await?;
    creation_status_inner(&state, &owner, operation).await
}

fn validate_review_binding(
    saved: &db::SitePostingDraft,
    terms: &AutonomousBountyTermsRecord,
    create: &AutonomousBountyCreate,
) -> Result<(), StatusCode> {
    let reviewed = &saved.draft["draft"];
    if reviewed["title"] != terms.document.title
        || reviewed["goal"] != terms.document.goal
        || reviewed["acceptance_criteria"] != json!(terms.document.acceptance_criteria)
        || providers::parse_units(&reviewed["solver_reward_usdc"]).ok()
            != u64::try_from(create.solver_reward.amount).ok()
        || providers::parse_units(&reviewed["verifier_reward_usdc"]).ok()
            != u64::try_from(create.verifier_reward.amount).ok()
    {
        return Err(StatusCode::CONFLICT);
    }
    // Match the complete approved evidence, reference snapshots and deadline.
    // The only frontend-only evidence annotation is the bounded illustration.
    let mut evidence = reviewed
        .get("evidence_schema")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(object) = evidence.as_object_mut() {
        object.remove("x-agent-bounties-draft-visual");
    }
    if evidence != terms.document.evidence_schema
        || reviewed["benchmark"] != terms.document.benchmark
        || reviewed.get("meta_child").is_some_and(|v| !v.is_null())
        || create.claim_window_seconds
            != reviewed["task_window_days"]
                .as_u64()
                .unwrap_or(30)
                .saturating_mul(86400)
        || create.verification_window_seconds != 48 * 3600
        || create.verifier_reward
            != terms
                .document
                .contract_terms
                .get("claim_bond")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .ok_or(StatusCode::CONFLICT)?
    {
        return Err(StatusCode::CONFLICT);
    }
    if let Some(cutoff) = reviewed["delivery_deadline"].as_str() {
        let cutoff = chrono::DateTime::parse_from_rfc3339(cutoff)
            .map_err(|_| StatusCode::CONFLICT)?
            .timestamp();
        if cutoff < 0 || create.funding_deadline != cutoff as u64 {
            return Err(StatusCode::CONFLICT);
        }
    }
    if let Some(reference) = reviewed
        .get("reference_attachment")
        .filter(|v| !v.is_null())
    {
        if evidence.get("x-agent-bounties-reference-attachment") != Some(reference) {
            return Err(StatusCode::CONFLICT);
        }
    }
    // The signed predicted address binds all on-chain commitments; additionally
    // preserve the approved source and image, not just the economic fields.
    let approved_image = reviewed
        .get("image")
        .filter(|v| !v.is_null())
        .map(|value| {
            let mut image: domain::BountyImageReference =
                serde_json::from_value(value.clone()).map_err(|_| StatusCode::CONFLICT)?;
            image.prompt = image.prompt.trim().into();
            image.alt_text = image.alt_text.trim().into();
            image.asset_url = url::Url::parse(image.asset_url.trim())
                .map_err(|_| StatusCode::CONFLICT)?
                .to_string();
            Ok::<_, StatusCode>(image)
        })
        .transpose()?;
    if reviewed.get("source_url").filter(|v| !v.is_null())
        != terms
            .document
            .source_url
            .as_ref()
            .map(|v| json!(v))
            .as_ref()
        || approved_image != terms.document.image
    {
        return Err(StatusCode::CONFLICT);
    }
    Ok(())
}

async fn process_creation(
    state: &SharedState,
    attempt: X402RelayAttempt,
    plan: &AutonomousBountyAuthorizedCreationPlan,
    terms: &AutonomousBountyTermsRecord,
) -> Result<(), StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let caps = &state.x402_relayer;
    let relayer = caps
        .relayer
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !relayer.has_gas_budget() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    store
        .repair_x402_outbox_marker(attempt.id)
        .await
        .map_err(map_x402_db_error)?;
    let attempt = store
        .get_x402_relay_attempt(attempt.id)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if attempt.status == X402RelayStatus::Broadcast {
        sponsored_verdict::retry_x402_saved(state, &attempt).await?;
        return Ok(());
    }
    if attempt.authorization_valid_before <= Utc::now().timestamp().max(0) as u64 {
        store
            .mark_x402_relay_failed(
                attempt.id,
                None,
                false,
                "authorization_expired",
                "The authorization expired before any broadcast.",
            )
            .await
            .map_err(map_x402_db_error)?;
        return Ok(());
    }
    let Some(lease) = store
        .acquire_x402_relayer_lease("base-mainnet", caps.lease_seconds)
        .await
        .map_err(map_x402_db_error)?
    else {
        return Ok(());
    };
    let result = async {
        let Some(_) = store.claim_x402_relay_attempt(attempt.id,lease,caps.lease_seconds).await.map_err(map_x402_db_error)? else { return Ok(()); };
        let (_, url) = state.base_rpc_urls.resolve("base-mainnet").map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        let prepared = tokio::time::timeout(Duration::from_secs(caps.relay_stage_timeout_seconds()),relayer.prepare_signed_transaction(&url,8453,&plan.relay_transaction,sponsored_creation::CREATION_MAX_GAS,caps.max_fee_per_gas_wei)).await;
        let prepared = match prepared {
            Ok(Ok(value)) => value,
            failure => {
                let (code, transient) = match failure {
                    Ok(Err(ChainBaseError::RelayerGasLimitExceeded { .. })) => ("simulated_gas_exceeds_cap", false),
                    Ok(Err(ChainBaseError::RelayerFeeCapExceeded { .. })) => ("gas_price_exceeds_cap", true),
                    Ok(Err(ChainBaseError::RelayerInsufficientBalance { .. })) => ("relay_reserve_unavailable", true),
                    Ok(Err(ChainBaseError::RelayerSimulation(_))) => ("authorization_simulation_rejected", false),
                    Ok(Err(ChainBaseError::RelayerChainMismatch { .. })) => ("relay_network_mismatch", false),
                    _ => ("simulation_service_unavailable", true),
                };
                store.mark_x402_relay_failed(attempt.id,Some(lease),transient && attempt.attempt_count < 4,code,"No transaction was broadcast. The exact operation is preserved; gas payer was not changed.").await.map_err(map_x402_db_error)?;
                return Ok(());
            }
        };
        store.save_relay_outbox(attempt.id,"base-mainnet",&prepared).await.map_err(map_x402_db_error)?;
        store.persist_posting_transaction(attempt.id,"base-mainnet",&prepared).await.map_err(map_x402_db_error)?;
        store.mark_x402_relay_broadcast(attempt.id,lease,&prepared.transaction.tx_hash,prepared.transaction.estimated_gas,prepared.transaction.gas_limit).await.map_err(map_x402_db_error)?;
        // Persist the hash before the first and only submission. Lost replies and
        // crashes are reconciled by hash; no retry allocates another nonce.
        sponsored_verdict::send_saved(state,"base-mainnet",attempt.id,&prepared,false,lease).await?;
        record_opportunity_creation_progress(state,"base-mainnet",terms,"wallet_signed").await?;
        Ok::<(),StatusCode>(())
    }.await;
    store
        .release_x402_relayer_lease("base-mainnet", lease)
        .await
        .map_err(map_x402_db_error)?;
    result?;
    Ok(())
}

#[utoipa::path(get, path = "/v1/site-auth/posting-funding/{operation}/creation-relay", responses((status = 200, description = "Reconcile the durable relay hash; never broadcast from this endpoint")), params(("operation" = Uuid, Path, description = "Saved account posting operation")), security(("site_account_session" = [])))]
async fn creation_status(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(operation): Path<Uuid>,
) -> Result<Response, StatusCode> {
    creation_status_inner(&state, &auth.posting_owner(&headers, false)?, operation).await
}

async fn creation_status_inner(
    state: &SharedState,
    owner: &str,
    operation: Uuid,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(id) = store
        .posting_creation_relay(owner, operation)
        .await
        .map_err(map_x402_db_error)?
    else {
        return Ok(response(
            json!({"operation_id":operation,"status":"not_started","bounty_funded":false}),
        ));
    };
    store
        .repair_x402_outbox_marker(id)
        .await
        .map_err(map_x402_db_error)?;
    store
        .repair_posting_transaction_marker(id)
        .await
        .map_err(map_x402_db_error)?;
    let Some(mut attempt) = store
        .get_x402_relay_attempt(id)
        .await
        .map_err(map_x402_db_error)?
    else {
        // The operation binding can exist before quota admission. Retrying the
        // identical authorization later must not allocate a new operation.
        return Ok(response(
            json!({"operation_id":operation,"status":"not_started","bounty_funded":false,"next_action":"Resume admission of the same saved authorization when the service is available."}),
        ));
    };
    if attempt.status == X402RelayStatus::Broadcast {
        attempt = reconcile_x402_relay(state, attempt.clone())
            .await
            .unwrap_or(attempt);
    }
    Ok(response(
        json!({"operation_id":operation,"relay_id":id,"status":attempt.status,"bounty_contract":attempt.bounty_contract,"tx_hash":attempt.tx_hash,
        "gas_payer":"relay","retryable":attempt.retryable,"simulation":if attempt.tx_hash.is_some(){"passed"}else{"not_confirmed"},"error_code":attempt.error_code,
        "canonical_funding_event":attempt.canonical_event_id,"creation_and_claimability":"check_canonical_inventory","bounty_funded":attempt.status==X402RelayStatus::Confirmed,
        "next_action":if attempt.status==X402RelayStatus::Failed { if attempt.retryable { "Wait: the service will retry only pre-broadcast checks for this same authorization." } else { "Keep this review for recovery. The authorization or gas policy needs review; no gas payer or budget was changed." } } else { "Reconcile canonical creation, funding and claimability for this exact bounty, then check ready-to-earn inventory. Never send a second creation." }}),
    ))
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TopupInput {
    wallet: String,
    required_usdc_units: String,
    country: String,
    #[serde(default)]
    subdivision: String,
    #[serde(default = "usd")]
    payment_currency: String,
    #[serde(default = "card")]
    payment_method: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    metamask_address: Option<String>,
    #[serde(default)]
    legacy_pending: bool,
    #[serde(default)]
    analytics_disabled: bool,
}
fn usd() -> String {
    "USD".into()
}
fn card() -> String {
    "CARD".into()
}

fn validate_topup(input: &TopupInput) -> Result<u64, StatusCode> {
    normalize_evm_address(&input.wallet).map_err(|_| StatusCode::BAD_REQUEST)?;
    let required = input
        .required_usdc_units
        .parse::<u64>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if required == 0
        || required > 10_000_000_000
        || input.country.len() != 2
        || !input.country.chars().all(|c| c.is_ascii_uppercase())
        || input.subdivision.len() > 3
        || !input.subdivision.chars().all(|c| c.is_ascii_uppercase())
        || input.payment_currency.len() != 3
        || !input
            .payment_currency
            .chars()
            .all(|c| c.is_ascii_uppercase())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(required)
}

#[utoipa::path(post, path = "/v1/site-auth/posting-funding/{operation}/topup-options", responses((status = 200, description = "Read current regional purchase options for the saved wallet")), params(("operation" = Uuid, Path, description = "Saved account posting operation")), security(("site_account_session" = [])),
    request_body = TopupInput
)]
async fn topup_options(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(operation): Path<Uuid>,
    Json(input): Json<TopupInput>,
) -> Result<Response, StatusCode> {
    let owner = auth.posting_owner(&headers, true)?;
    draft(&state, &owner, operation).await?;
    validate_topup(&input)?;
    if !env_flag("ENABLE_GUIDED_WALLET_TOPUP") {
        return Ok(response(
            json!({"enabled":false,"next_action":"Guided top-up is not enabled. Your saved review is unchanged."}),
        ));
    }
    let coinbase = if providers::configured("coinbase") {
        providers::coinbase_options(&input.country, &input.subdivision).await
    } else {
        Err("provider_not_configured")
    };
    let moonpay = providers::moonpay_options(&input).await;
    let meta = input
        .metamask_address
        .as_ref()
        .is_some_and(|v| v.eq_ignore_ascii_case(&input.wallet));
    Ok(response(
        json!({"enabled":true,"operation_id":operation,"wallet":input.wallet,"network":"base-mainnet",
        "recommended":if coinbase.is_ok(){"coinbase"}else if moonpay.is_ok(){"moonpay"}else if meta{"metamask"}else{"handoff"},
        "providers":[{"id":"coinbase","available":coinbase.is_ok(),"options":coinbase.as_ref().ok(),"blocker":coinbase.as_ref().err(),"requirements":"Coinbase sign-in and identity verification may be required. Review the provider quote before payment."},
            {"id":"moonpay","available":moonpay.is_ok(),"options":moonpay.as_ref().ok(),"blocker":moonpay.as_ref().err(),"requirements":"MoonPay requires its own account and may request identity and bank verification. Card eligibility is determined by MoonPay and your bank."},
            {"id":"metamask","available":meta,"blocker":if meta{None}else{Some("matching_metamask_connection_required")},"requirements":"Connect this exact wallet in a supported browser or the MetaMask phone app. Review the quote before continuing with Transak."}],
        "attempt":state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?.active_wallet_topup(&owner,operation).await.map_err(map_x402_db_error)?,
        "next_action":"Prepare one compatible provider quote. Do not open a second purchase while an earlier payment is unresolved."}),
    ))
}

fn provider_error(code: &str) -> Response {
    response(
        json!({"status":"unavailable","code":code,"bounty_funded":false,
        "next_action":match code {"region_unavailable"|"payment_method_unavailable"|"currency_unavailable"|"base_usdc_unavailable"=>"Choose another available provider for your confirmed country and payment method.",
        "subdivision_required_or_unavailable"=>"Select your state to check supported payment methods.",
        "provider_rate_limited"=>"Wait briefly, then check this saved operation.",
        "wallet_mismatch"=>"Choose whether to use the bounty wallet or change it on the saved bounty review before buying.",
        _=>"The provider could not prepare this quote. Keep the saved review and retry availability; no payment was requested."}}),
    )
}

#[utoipa::path(post, path = "/v1/site-auth/posting-funding/{operation}/topups", responses((status = 200, description = "Stage a live quote; preparation never buys crypto or funds a bounty")), params(("operation" = Uuid, Path, description = "Saved account posting operation")), security(("site_account_session" = [])),
    request_body = TopupInput
)]
async fn prepare_topup(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(operation): Path<Uuid>,
    Json(input): Json<TopupInput>,
) -> Result<Response, StatusCode> {
    let owner = auth.posting_owner(&headers, true)?;
    let saved = draft(&state, &owner, operation).await?;
    let required = validate_topup(&input)?;
    if !input.legacy_pending && saved_budget(&saved)? != required {
        return Err(StatusCode::CONFLICT);
    }
    if !env_flag("ENABLE_GUIDED_WALLET_TOPUP") {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if !["moonpay", "coinbase", "metamask"].contains(&input.provider.as_str()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if input.provider == "metamask"
        && !input
            .metamask_address
            .as_ref()
            .is_some_and(|s| s.eq_ignore_ascii_case(&input.wallet))
    {
        return Ok(provider_error("wallet_mismatch"));
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (balance, _, _) = read_base_balances(&state, &input.wallet).await?;
    let shortfall = u128::from(required).saturating_sub(balance) as u64;
    if shortfall == 0 && !input.legacy_pending {
        return Ok(response(
            json!({"status":"wallet_ready","operation_id":operation,"bounty_funded":false,"next_action":"Return to the saved bounty review."}),
        ));
    }
    let context = json!({"wallet":input.wallet,"network":"base-mainnet","chain_id":8453,"required_usdc_units":required.to_string(),"shortfall_usdc_units":shortfall.to_string(),"country":input.country,"payment_currency":input.payment_currency,"legacy_pending":input.legacy_pending});
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&context).map_err(|_| StatusCode::BAD_REQUEST)?,
    ));
    let (attempt, fresh) = store
        .reserve_wallet_topup(
            &owner,
            operation,
            &input.wallet,
            &input.provider,
            &fingerprint,
            &context,
        )
        .await
        .map_err(map_x402_db_error)?;
    if !fresh {
        return Ok(response(
            json!({"status":"purchase_already_exists","attempt":attempt,"next_action":"Resume or resolve the existing purchase before changing providers."}),
        ));
    }
    if input.legacy_pending {
        let attempt = store
            .update_wallet_topup(&owner, attempt.id, "preparing", "unknown", None, None)
            .await
            .map_err(map_x402_db_error)?;
        return Ok(response(
            json!({"status":"purchase_already_exists","attempt":attempt,"next_action":"Check your original provider order or receipt before buying again."}),
        ));
    }
    // The trusted reverse proxy must strip and set this header. It is deliberately
    // opt-in; an arbitrary caller-supplied X-Forwarded-For is never accepted.
    let trusted_header = env::var("TOPUP_TRUSTED_CLIENT_IP_HEADER").unwrap_or_default();
    let ip = headers
        .get(trusted_header.as_str())
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<std::net::IpAddr>().ok());
    let website = env::var("WEBSITE_BASE_URL").unwrap_or_else(|_| "https://agentizable.ai".into());
    let return_url=format!("{}/onramp.html?operation_id={operation}&wallet={}&amount={}&guided=1&return=post.html%3Foperation_id%3D{operation}%23bounty-preview{}",website.trim_end_matches('/'),input.wallet,providers::units(required),if input.analytics_disabled { "&analytics=off" } else { "" });
    let result = match input.provider.as_str() {
        "coinbase" => {
            if let Some(ip) = ip {
                providers::prepare_coinbase(
                    &input,
                    attempt.id,
                    shortfall,
                    &return_url,
                    &ip.to_string(),
                )
                .await
            } else {
                Err("trusted_client_ip_unavailable")
            }
        }
        "moonpay" => {
            if let Some(ip) = ip {
                providers::prepare_moonpay(
                    &input,
                    attempt.id,
                    shortfall,
                    &return_url,
                    &ip.to_string(),
                )
                .await
            } else {
                Err("trusted_client_ip_unavailable")
            }
        }
        "metamask" => Ok((
            "https://portfolio.metamask.io/buy/build-quote".into(),
            json!({"wallet":input.wallet,"network":"base-mainnet","received_usdc":Value::Null,"excess_usdc":Value::Null,"payment_total":Value::Null,"message":"MetaMask obtains the live quote after wallet authentication. Verify this address and USDC on Base before paying."}),
        )),
        _ => Err("provider_unavailable"),
    };
    match result {
        Ok((url, quote)) => {
            let latest = Utc::now() + chrono::Duration::minutes(5);
            let expires = quote["expires_at"]
                .as_str()
                .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
                .map(|v| v.with_timezone(&Utc).min(latest))
                .unwrap_or(latest);
            if expires <= Utc::now() {
                store
                    .update_wallet_topup(&owner, attempt.id, "preparing", "failed", None, None)
                    .await
                    .map_err(map_x402_db_error)?;
                return Ok(provider_error("quote_expired"));
            }
            let attempt = store
                .update_wallet_topup(
                    &owner,
                    attempt.id,
                    "preparing",
                    "prepared",
                    Some(&quote),
                    Some(expires),
                )
                .await
                .map_err(map_x402_db_error)?;
            Ok(response(
                json!({"status":"quote_ready","attempt":attempt,"checkout_url":url,"next_action":"Review the quote and destination, then personally open checkout. Buying USDC does not fund the bounty."}),
            ))
        }
        Err(code) => {
            store
                .update_wallet_topup(&owner, attempt.id, "preparing", "failed", None, None)
                .await
                .map_err(map_x402_db_error)?;
            Ok(provider_error(code))
        }
    }
}

#[utoipa::path(get, path = "/v1/site-auth/posting-funding/{operation}/topups", responses((status = 200, description = "Reconcile provider status; a provider receipt never proves bounty funding")), params(("operation" = Uuid, Path, description = "Saved account posting operation")), security(("site_account_session" = [])))]
async fn topup_status(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(operation): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let owner = auth.posting_owner(&headers, false)?;
    draft(&state, &owner, operation).await?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut attempt = store
        .active_wallet_topup(&owner, operation)
        .await
        .map_err(map_x402_db_error)?;
    if let Some(previous) = &attempt {
        if ["pending", "unknown"].contains(&previous.status.as_str())
            && previous.context["legacy_pending"] != true
        {
            if let Ok((mut next, recovery)) = providers::reconcile(previous).await {
                // A provider success without delivered funds stays unresolved.
                if next == "completed" {
                    let required = previous.context["required_usdc_units"]
                        .as_str()
                        .and_then(|v| v.parse::<u128>().ok())
                        .unwrap_or(u128::MAX);
                    if !read_base_balances(&state, &previous.wallet)
                        .await
                        .is_ok_and(|(balance, _, _)| balance >= required)
                    {
                        next = "pending";
                    }
                }
                let mut quote = previous.quote.clone().unwrap_or_else(|| json!({}));
                if let (Some(target), Some(fields)) = (quote.as_object_mut(), recovery.as_object())
                {
                    target.extend(fields.clone());
                }
                if next != previous.status || Some(&quote) != previous.quote.as_ref() {
                    attempt = store
                        .update_wallet_topup(
                            &owner,
                            previous.id,
                            &previous.status,
                            next,
                            Some(&quote),
                            None,
                        )
                        .await
                        .map_err(map_x402_db_error)?;
                }
            }
        }
    }
    Ok(response(
        json!({"operation_id":operation,"attempt":attempt,"bounty_funded":false,"next_action":"Recheck the wallet balance and return to the same bounty review. Provider completion is not funding evidence."}),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TopupAction {
    attempt_id: Uuid,
    action: String,
    #[serde(default)]
    provider_resolution_confirmed: bool,
}
#[utoipa::path(post, path = "/v1/site-auth/posting-funding/{operation}/topups/action", responses((status = 200, description = "Person-initiated checkout opening or explicit recovery acknowledgement; account cookie and allowed Origin required")), params(("operation" = Uuid, Path, description = "Saved account posting operation")), security(("site_account_session" = [])),
    request_body = TopupAction
)]
async fn topup_action(
    State(state): State<SharedState>,
    Extension(auth): Extension<site_auth::SiteAuthService>,
    headers: HeaderMap,
    Path(operation): Path<Uuid>,
    Json(input): Json<TopupAction>,
) -> Result<Response, StatusCode> {
    let owner = auth.posting_owner(&headers, true)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let previous = store
        .active_wallet_topup(&owner, operation)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if previous.id != input.attempt_id {
        return Err(StatusCode::CONFLICT);
    }
    if input.action == "open" {
        let saved = draft(&state, &owner, operation).await?;
        let required = saved_budget(&saved)?;
        if previous.context["required_usdc_units"].as_str() != Some(required.to_string().as_str())
            || read_base_balances(&state, &previous.wallet).await?.0 >= u128::from(required)
        {
            return Err(StatusCode::CONFLICT);
        }
    }
    let next = match (input.action.as_str(), previous.status.as_str()) {
        ("open", "prepared") if previous.expires_at.is_some_and(|v| v > Utc::now()) => "pending",
        ("cancel_unopened", "prepared" | "preparing") => "cancelled",
        ("confirm_resolved", "pending" | "unknown") if input.provider_resolution_confirmed => {
            "cancelled"
        }
        _ => return Err(StatusCode::CONFLICT),
    };
    let attempt = store
        .update_wallet_topup(&owner, previous.id, &previous.status, next, None, None)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::CONFLICT)?;
    Ok(response(
        json!({"attempt":attempt,"bounty_funded":false,"next_action":"Continue this saved operation."}),
    ))
}

// A restart can resume an admitted authorization that has not been signed by
// the relay. Once signed bytes exist, only reconciliation is allowed.
pub(crate) fn start_recovery_worker(state: SharedState) {
    tokio::spawn(async move {
        loop {
            if let Some(store) = &state.store {
                if let Ok(jobs) = store.pending_posting_creations().await {
                    for (owner, operation, hash, payload, relay_id) in jobs {
                        let _ = async {
                            if store
                                .posting_transaction(relay_id)
                                .await
                                .map_err(map_x402_db_error)?
                                .is_some()
                            {
                                creation_status_inner(&state, &owner, operation).await?;
                                return Ok(());
                            }
                            if !env_flag("ENABLE_SPONSORED_POSTING_CREATION")
                                || !state.x402_relayer.enabled
                            {
                                return Ok(());
                            }
                            let saved = draft(&state, &owner, operation).await?;
                            if saved.draft_hash != hash
                                || saved.approved_draft_hash.as_deref() != Some(&hash)
                            {
                                return Err(StatusCode::CONFLICT);
                            }
                            let input: CreationInput = serde_json::from_value(payload)
                                .map_err(|_| StatusCode::BAD_REQUEST)?;
                            let attempt = store
                                .get_x402_relay_attempt(relay_id)
                                .await
                                .map_err(map_x402_db_error)?
                                .ok_or(StatusCode::NOT_FOUND)?;
                            let terms = require_autonomous_creation_terms(
                                &state,
                                "base-mainnet",
                                &input.create,
                            )
                            .await?;
                            validate_review_binding(&saved, &terms, &input.create)?;
                            if !store
                                .has_posting_legal_acceptance(
                                    input.legal_acceptance_id,
                                    &input.create.creator,
                                    LEGAL_TERMS_VERSION,
                                    LEGAL_PRIVACY_VERSION,
                                    &legal_statement_hash(),
                                )
                                .await
                                .map_err(map_x402_db_error)?
                            {
                                return Err(StatusCode::PRECONDITION_REQUIRED);
                            }
                            let relayer = state
                                .x402_relayer
                                .address()
                                .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
                            let plan = configured_autonomous_new_risk_planner("base-mainnet")?
                                .plan_authorized_creation(
                                    "base-mainnet",
                                    &input.create,
                                    &input.signature,
                                    Some(&relayer),
                                )
                                .map_err(|_| StatusCode::BAD_REQUEST)?;
                            process_creation(&state, attempt, &plan, &terms).await
                        }
                        .await;
                    }
                }
            }
            sleep(Duration::from_secs(15)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        db::SitePostingDraft,
        AutonomousBountyTermsRecord,
        AutonomousBountyCreate,
    ) {
        let wallet = "0x1111111111111111111111111111111111111111";
        let reference = json!({"url":"https://example.test/reference","fetched_at":"2026-09-17T12:00:00Z","sha256":format!("sha256:{}","a".repeat(64))});
        let evidence = json!({"type":"object","x-agent-bounties-reference-attachment":reference});
        let reviewed = json!({"title":"Recruit a real company","goal":"Publish a company backlog task naming the recruiting agent","acceptance_criteria":["Public company bounty names the agent"],"solver_reward_usdc":"2.00","verifier_reward_usdc":"0.01","task_window_days":30,"delivery_deadline":"2026-09-18T21:00:00-06:00","benchmark":{"engine":"creator_review_v1","delivery_deadline":1789786800i64},"evidence_schema":evidence,"reference_attachment":reference,"source_url":"https://example.test/reference"});
        let now = Utc::now();
        let saved = db::SitePostingDraft {
            operation_id: Uuid::new_v4(),
            draft: json!({"draft":reviewed}),
            recovery_state: json!({}),
            draft_hash: "approved".into(),
            approved_draft_hash: Some("approved".into()),
            revision: 1,
            updated_at: now,
            expires_at: now + chrono::Duration::days(1),
        };
        let create:AutonomousBountyCreate=serde_json::from_value(json!({"creator":wallet,"solver_reward":{"amount":2000000,"currency":"usdc"},"verifier_reward":{"amount":10000,"currency":"usdc"},"initial_funding":{"amount":2010000,"currency":"usdc"},"terms_hash":"hash","policy_hash":"policy","acceptance_criteria_hash":"acceptance","benchmark_hash":"benchmark","evidence_schema_hash":"schema","funding_deadline":DateTime::parse_from_rfc3339("2026-09-18T21:00:00-06:00").unwrap().timestamp(),"claim_window_seconds":2592000,"verification_window_seconds":172800,"verification_mode":"signed_quorum","verifier_module":null,"verifier_reward_recipient":null,"verifiers":[wallet],"threshold":1,"creation_nonce":"nonce"})).unwrap();
        let document = domain::AutonomousBountyTermsDocument {
            schema_version: "agent-bounties/terms-v1".into(),
            contract_terms: json!({"claim_bond":create.verifier_reward}),
            title: reviewed["title"].as_str().unwrap().into(),
            goal: reviewed["goal"].as_str().unwrap().into(),
            acceptance_criteria: vec!["Public company bounty names the agent".into()],
            benchmark: reviewed["benchmark"].clone(),
            evidence_schema: evidence,
            verification_policy: json!({}),
            source_url: Some("https://example.test/reference".into()),
            discovery_source: None,
            image: None,
            agent_eligibility: None,
            claim_coordination: None,
        };
        let terms = AutonomousBountyTermsRecord {
            terms_hash: "hash".into(),
            policy_hash: "policy".into(),
            acceptance_criteria_hash: "acceptance".into(),
            benchmark_hash: "benchmark".into(),
            evidence_schema_hash: "schema".into(),
            creator_wallet: wallet.into(),
            document,
            created_at: now,
        };
        (saved, terms, create)
    }
    #[test]
    fn relay_binds_deadline_rewards_checks_and_reference_to_approved_draft() {
        let (saved, terms, create) = fixture();
        assert!(validate_review_binding(&saved, &terms, &create).is_ok());
        for key in [
            "title",
            "goal",
            "acceptance_criteria",
            "benchmark",
            "evidence_schema",
            "source_url",
            "image",
        ] {
            let mut changed = serde_json::to_value(&terms).unwrap();
            changed["document"][key] = Value::Null;
            // Required typed fields cannot even deserialize; optional fields still fail binding.
            if let Ok(changed) = serde_json::from_value(changed) {
                if key != "image" {
                    assert!(
                        validate_review_binding(&saved, &changed, &create).is_err(),
                        "{key}"
                    );
                }
            }
        }
        let mut changed = create.clone();
        changed.funding_deadline += 1;
        assert!(validate_review_binding(&saved, &terms, &changed).is_err());
        let mut changed = create.clone();
        changed.solver_reward.amount += 1;
        assert!(validate_review_binding(&saved, &terms, &changed).is_err());
        let mut changed = create;
        changed.claim_window_seconds += 1;
        assert!(validate_review_binding(&saved, &terms, &changed).is_err());
        let mut changed = saved.clone();
        changed.draft["draft"]["reference_attachment"]["sha256"] = json!("different");
        assert!(validate_review_binding(&changed, &terms, &fixture().2).is_err());
        changed = saved;
        changed.draft["draft"]["evidence_schema"]["x-agent-bounties-draft-visual"] =
            json!({"version":1});
        assert!(validate_review_binding(&changed, &terms, &fixture().2).is_ok());
    }
    #[test]
    fn topup_rejects_unknown_fields_and_bad_location_or_amount() {
        let mut input = json!({"wallet":"0x1111111111111111111111111111111111111111","required_usdc_units":"2010000","country":"MX"});
        assert_eq!(
            validate_topup(&serde_json::from_value(input.clone()).unwrap()).unwrap(),
            2010000
        );
        input["required_usdc_units"] = json!("2.01");
        assert!(validate_topup(&serde_json::from_value(input.clone()).unwrap()).is_err());
        input["country"] = json!("MEX");
        assert!(validate_topup(&serde_json::from_value(input.clone()).unwrap()).is_err());
        input["checkout_url"] = json!("https://evil.test");
        assert!(serde_json::from_value::<TopupInput>(input).is_err());
    }
}
