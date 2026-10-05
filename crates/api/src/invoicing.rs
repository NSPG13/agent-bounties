//! Hosted invoiced seller-of-record orders (docs/invoice-seller-of-record.md).
//!
//! Every route is operator-only except the Stripe webhook, which must carry a valid
//! `STRIPE_INVOICE_WEBHOOK_SECRET` signature. The whole path stays off unless
//! `INVOICING_ENABLED=true`, an operator token is configured, and every setting below is present. A
//! Stripe live key is refused unless `INVOICING_LIVEMODE=true`.
//!
//! Order state is never stored directly. It is replayed from the append-only event log through
//! `invoicing::project_order`, and a new event is persisted only when that replay accepts it.

use super::*;
use ::invoicing::{
    contractor_attestation, invoice_contractor_source_hash, keccak_hex, project_order,
    quote_invoice_order, summarize_contractor_payments, ContractorPayment, ContractorRecord,
    ContractorTaxSummary, InvoicePaidEvidence, InvoicePolicy, OrderEvent, OrderState, OrderStatus,
    INVOICE_CONTRACTOR_SOURCE,
};
use chain_base::{
    autonomous_v2_create_from_terms, plan_participant_attestation, plan_participant_registration,
    validate_autonomous_v2_creation_against_terms, AutonomousV2FactoryFee,
    ParticipantAttestationRequest, ParticipantAttestationTypedData,
    BASE_SEPOLIA_USDC_TOKEN_ADDRESS,
};
use payments_stripe::invoicing::{
    parse_invoice_webhook, plan_credit_note_refund, plan_finalize_invoice, plan_retrieve_invoice,
    plan_send_invoice, plan_void_invoice, CreditNoteRefund, InvoiceWebhookEvidence,
    StripeInvoiceLine, StripeInvoiceOrder,
};
use serde_json::{json, Value};

type InvoicingResult<T> = Result<T, AgentActionApiError>;

const INVOICE_TERMS_SCHEMA: &str = "agent-bounties/terms-v1";

fn unavailable(code: &str, message: impl Into<String>) -> AgentActionApiError {
    agent_action_error(
        StatusCode::SERVICE_UNAVAILABLE,
        code,
        message,
        false,
        "Configure invoicing as described in docs/invoice-seller-of-record.md and retry.",
    )
}

fn conflict(code: &str, message: impl Into<String>, next: &str) -> AgentActionApiError {
    agent_action_error(StatusCode::CONFLICT, code, message, false, next)
}

fn bad_request(code: &str, message: impl Into<String>) -> AgentActionApiError {
    agent_action_error(
        StatusCode::BAD_REQUEST,
        code,
        message,
        false,
        "Correct the request and retry.",
    )
}

fn internal(error: impl std::fmt::Display) -> AgentActionApiError {
    agent_action_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "invoicing_storage_failed",
        error.to_string(),
        true,
        "Retry the identical request.",
    )
}

/// Everything the invoiced path needs, resolved from the environment for each request.
#[derive(Debug, Clone)]
pub(crate) struct InvoicingConfig {
    pub policy: InvoicePolicy,
    pub network: String,
    pub planner: AutonomousBountyTxPlanner,
    pub fee: AutonomousV2FactoryFee,
    pub treasury: String,
    pub contractor_registry: String,
    pub contractor_attester: String,
    pub contractor_agreement_sha256: String,
    pub business_terms_url: String,
    pub business_terms_sha256: String,
    pub verification_mode: String,
    pub verifiers: Vec<String>,
    pub verifier_threshold: u8,
    pub funding_window_seconds: u64,
    pub claim_window_seconds: u64,
    pub verification_window_seconds: u64,
}

fn env_setting(name: &str, missing: &mut Vec<String>) -> String {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => {
            missing.push(name.to_string());
            String::new()
        }
    }
}

fn env_number<T: std::str::FromStr>(name: &str, default: T) -> Result<T, AgentActionApiError> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse()
            .map_err(|_| unavailable("invoicing_misconfigured", format!("{name} is not a number"))),
        _ => Ok(default),
    }
}

fn sha256_setting(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// Stripe mode of the configured secret key: `Some(true)` live, `Some(false)` test.
fn stripe_key_livemode(key: &str) -> Option<bool> {
    let key = key.trim();
    if key.starts_with("sk_live_") || key.starts_with("rk_live_") {
        Some(true)
    } else if key.starts_with("sk_test_") || key.starts_with("rk_test_") {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn invoicing_config(state: &SharedState) -> InvoicingResult<InvoicingConfig> {
    if env::var("INVOICING_ENABLED").ok().as_deref() != Some("true") {
        return Err(unavailable(
            "invoicing_disabled",
            "the invoiced seller-of-record path is disabled",
        ));
    }
    let livemode = env::var("INVOICING_LIVEMODE").ok().as_deref() == Some("true");
    let key_mode = state
        .stripe_secret_key
        .as_deref()
        .and_then(stripe_key_livemode);
    if key_mode != Some(livemode) {
        return Err(unavailable(
            "invoicing_stripe_mode_mismatch",
            "STRIPE_SECRET_KEY must be a test key, or a live key with INVOICING_LIVEMODE=true",
        ));
    }
    let mut missing = Vec::new();
    let network = env::var("INVOICING_NETWORK").unwrap_or_else(|_| "base-sepolia".to_string());
    let treasury = env_setting("INVOICING_TREASURY_WALLET", &mut missing);
    let contractor_registry = env_setting("INVOICING_CONTRACTOR_REGISTRY", &mut missing);
    let contractor_attester = env_setting("INVOICING_CONTRACTOR_ATTESTER", &mut missing);
    let contractor_agreement_sha256 =
        env_setting("INVOICING_CONTRACTOR_AGREEMENT_SHA256", &mut missing);
    let business_terms_url = env_setting("INVOICING_BUSINESS_TERMS_URL", &mut missing);
    let business_terms_sha256 = env_setting("INVOICING_BUSINESS_TERMS_SHA256", &mut missing);
    let verifiers_setting = env_setting("INVOICING_VERIFIERS", &mut missing);
    if !missing.is_empty() {
        return Err(unavailable(
            "invoicing_misconfigured",
            format!("missing settings: {}", missing.join(", ")),
        ));
    }
    let address = |value: &str, name: &str| {
        normalize_evm_address(value).map_err(|_| {
            unavailable(
                "invoicing_misconfigured",
                format!("{name} is not an address"),
            )
        })
    };
    let verifiers = verifiers_setting
        .split(',')
        .map(|verifier| address(verifier.trim(), "INVOICING_VERIFIERS"))
        .collect::<Result<Vec<_>, _>>()?;
    let verifier_threshold: u8 = env_number("INVOICING_VERIFIER_THRESHOLD", 2)?;
    let verification_mode =
        env::var("INVOICING_VERIFICATION_MODE").unwrap_or_else(|_| "signed_quorum".to_string());
    if !matches!(
        verification_mode.as_str(),
        "signed_quorum" | "ai_judge_quorum"
    ) || verifier_threshold == 0
        || usize::from(verifier_threshold) > verifiers.len()
        || !sha256_setting(&contractor_agreement_sha256)
        || !sha256_setting(&business_terms_sha256)
        || !business_terms_url.starts_with("https://")
    {
        return Err(unavailable(
            "invoicing_misconfigured",
            "verification quorum, agreement hash, or business terms are invalid",
        ));
    }
    let (planner, deployment) = configured_autonomous_v2_planner(&network).map_err(|_| {
        unavailable(
            "invoicing_v2_factory_unavailable",
            "invoiced bounties need a configured autonomous-v2 factory on INVOICING_NETWORK",
        )
    })?;
    let policy = InvoicePolicy {
        platform_fee_bps: deployment.fee.platform_fee_bps,
        processing_fee_bps: env_number("INVOICING_PROCESSING_FEE_BPS", 0)?,
        processing_fee_fixed_cents: env_number("INVOICING_PROCESSING_FEE_FIXED_CENTS", 0)?,
        max_invoice_cents: env_number("INVOICING_MAX_INVOICE_CENTS", 1_000_000)?,
        days_until_due: env_number("INVOICING_DAYS_UNTIL_DUE", 14)?,
        livemode,
    };
    policy
        .validate()
        .map_err(|error| unavailable("invoicing_misconfigured", error.to_string()))?;
    Ok(InvoicingConfig {
        policy,
        network,
        planner,
        fee: deployment.fee,
        treasury: address(&treasury, "INVOICING_TREASURY_WALLET")?,
        contractor_registry: address(&contractor_registry, "INVOICING_CONTRACTOR_REGISTRY")?,
        contractor_attester: address(&contractor_attester, "INVOICING_CONTRACTOR_ATTESTER")?,
        contractor_agreement_sha256: contractor_agreement_sha256.to_ascii_lowercase(),
        business_terms_url,
        business_terms_sha256: business_terms_sha256.to_ascii_lowercase(),
        verification_mode,
        verifiers,
        verifier_threshold,
        funding_window_seconds: env_number("INVOICING_FUNDING_WINDOW_SECONDS", 7 * 86_400)?,
        claim_window_seconds: env_number("INVOICING_CLAIM_WINDOW_SECONDS", 7 * 86_400)?,
        verification_window_seconds: env_number(
            "INVOICING_VERIFICATION_WINDOW_SECONDS",
            3 * 86_400,
        )?,
    })
}

/// Invoicing moves real money, so an unset operator token fails closed here rather than opening
/// the routes as other operator routes do in local development.
fn require_invoicing_operator<'a>(
    state: &'a SharedState,
    headers: &HeaderMap,
) -> InvoicingResult<(InvoicingConfig, &'a PostgresStore)> {
    if state.operator_api_token.is_none() {
        return Err(unavailable(
            "invoicing_operator_token_unset",
            "OPERATOR_API_TOKEN must be set before invoicing routes are enabled",
        ));
    }
    require_operator(state, headers).map_err(|status| {
        agent_action_error(
            status,
            "operator_required",
            "invoicing routes require the operator token",
            false,
            "Send the operator token.",
        )
    })?;
    let config = invoicing_config(state)?;
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| unavailable("invoicing_store_unavailable", "DATABASE_URL is required"))?;
    Ok((config, store))
}

fn stripe_key(state: &SharedState) -> InvoicingResult<&str> {
    state.stripe_secret_key.as_deref().ok_or_else(|| {
        unavailable(
            "invoicing_stripe_unconfigured",
            "STRIPE_SECRET_KEY is required",
        )
    })
}

async fn execute_stripe(
    state: &SharedState,
    intent: &StripeRequestIntent,
) -> InvoicingResult<StripeExecutionReport> {
    execute_stripe_request(intent, stripe_key(state)?, &state.stripe_api_base_url)
        .await
        .map_err(|error| {
            agent_action_error(
                StatusCode::BAD_GATEWAY,
                "stripe_request_failed",
                error.to_string(),
                true,
                "Retry the identical request; Stripe idempotency keys make it safe.",
            )
        })
}

fn valid_order_id(value: &str) -> bool {
    value.starts_with("ord_")
        && value.len() <= 64
        && value[4..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        && value.len() > 4
}

fn event_kind(event: &OrderEvent) -> InvoicingResult<String> {
    serde_json::to_value(event)
        .map_err(internal)?
        .get("kind")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| internal("order event has no kind"))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvoiceOrderView {
    pub order: db::InvoiceOrderRecord,
    pub state: OrderState,
    pub refund_due_cents: u64,
    pub events: Vec<db::InvoiceOrderEventRecord>,
}

async fn load_order(
    store: &PostgresStore,
    config: &InvoicingConfig,
    order_id: &str,
) -> InvoicingResult<(InvoiceOrderView, Vec<OrderEvent>)> {
    let order = store
        .get_invoice_order(order_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            agent_action_error(
                StatusCode::NOT_FOUND,
                "invoice_order_not_found",
                "no invoiced order has this id",
                false,
                "Check the order id.",
            )
        })?;
    let records = store
        .list_invoice_order_events(order_id)
        .await
        .map_err(internal)?;
    let events = records
        .iter()
        .map(|record| serde_json::from_value::<OrderEvent>(record.event.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    let state = project_order(order_id, &config.policy, &events)
        .map_err(|error| internal(format!("stored order log no longer replays: {error}")))?;
    Ok((
        InvoiceOrderView {
            order,
            refund_due_cents: state.refund_due_cents(),
            state,
            events: records,
        },
        events,
    ))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvoiceEventOutcome {
    pub applied: bool,
    pub duplicate: bool,
    pub reason: Option<String>,
    pub state: Option<OrderState>,
}

/// Validates `event` by replaying the log with it appended, then persists it at the next
/// sequence. A replayed evidence id is reported as a duplicate, never applied twice.
async fn append_order_event(
    store: &PostgresStore,
    config: &InvoicingConfig,
    order_id: &str,
    event: OrderEvent,
) -> InvoicingResult<InvoiceEventOutcome> {
    for _ in 0..3 {
        let (_, mut events) = load_order(store, config, order_id).await?;
        if let Some(id) = event.evidence_id() {
            if events
                .iter()
                .any(|existing| existing.evidence_id() == Some(id))
            {
                return Ok(InvoiceEventOutcome {
                    applied: false,
                    duplicate: true,
                    reason: None,
                    state: project_order(order_id, &config.policy, &events).ok(),
                });
            }
        }
        let sequence = i32::try_from(events.len()).map_err(internal)?;
        events.push(event.clone());
        let state = match project_order(order_id, &config.policy, &events) {
            Ok(state) => state,
            Err(error) => {
                return Ok(InvoiceEventOutcome {
                    applied: false,
                    duplicate: false,
                    reason: Some(error.to_string()),
                    state: None,
                })
            }
        };
        let value = serde_json::to_value(&event).map_err(internal)?;
        match store
            .append_invoice_order_event(
                order_id,
                sequence,
                &event_kind(&event)?,
                &value,
                event.evidence_id(),
            )
            .await
            .map_err(internal)?
        {
            db::InvoiceEventAppend::Appended => {
                return Ok(InvoiceEventOutcome {
                    applied: true,
                    duplicate: false,
                    reason: None,
                    state: Some(state),
                })
            }
            db::InvoiceEventAppend::DuplicateEvidence => {
                return Ok(InvoiceEventOutcome {
                    applied: false,
                    duplicate: true,
                    reason: Some("evidence already recorded".to_string()),
                    state: None,
                })
            }
            db::InvoiceEventAppend::SequenceConflict => continue,
        }
    }
    Err(conflict(
        "invoice_order_busy",
        "the order changed concurrently three times",
        "Retry the identical request.",
    ))
}

fn require_applied(outcome: InvoiceEventOutcome) -> InvoicingResult<OrderState> {
    match outcome {
        InvoiceEventOutcome {
            applied: true,
            state: Some(state),
            ..
        } => Ok(state),
        InvoiceEventOutcome { reason, .. } => Err(conflict(
            "invoice_order_transition_rejected",
            reason.unwrap_or_else(|| "the event was already recorded".to_string()),
            "Read the order's next_action and follow it.",
        )),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CreateInvoiceOrderRequest {
    pub order_id: Option<String>,
    pub buyer_name: String,
    pub buyer_email: String,
    pub title: String,
    pub solver_reward_cents: u64,
    pub verifier_reward_cents: u64,
}

pub(crate) async fn create_invoice_order(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<CreateInvoiceOrderRequest>,
) -> InvoicingResult<Json<InvoiceOrderView>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let order_id = request
        .order_id
        .clone()
        .unwrap_or_else(|| format!("ord_{}", Uuid::new_v4().simple()));
    if !valid_order_id(&order_id) {
        return Err(bad_request(
            "invalid_order_id",
            "order ids are ord_ plus letters, digits, _ or -",
        ));
    }
    if request.buyer_name.trim().is_empty() || !request.buyer_email.contains('@') {
        return Err(bad_request(
            "invalid_buyer",
            "buyer name and email are required",
        ));
    }
    let quote = quote_invoice_order(
        &config.policy,
        &request.title,
        request.solver_reward_cents,
        request.verifier_reward_cents,
    )
    .map_err(|error| bad_request("invalid_quote", error.to_string()))?;
    if quote.verifier_reward_usdc % u64::from(config.verifier_threshold) != 0 {
        return Err(bad_request(
            "invalid_quote",
            "the verifier reward must divide evenly across the verifier quorum",
        ));
    }
    let quoted = serde_json::to_value(OrderEvent::Quoted {
        quote,
        terms_sha256: config.business_terms_sha256.clone(),
    })
    .map_err(internal)?;
    if !store
        .create_invoice_order(
            &order_id,
            request.buyer_name.trim(),
            request.buyer_email.trim(),
            &quoted,
        )
        .await
        .map_err(internal)?
    {
        return Err(conflict(
            "invoice_order_exists",
            "an order with this id already exists",
            "Read the existing order instead.",
        ));
    }
    Ok(Json(load_order(store, &config, &order_id).await?.0))
}

pub(crate) async fn get_invoice_order(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> InvoicingResult<Json<InvoiceOrderView>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    Ok(Json(load_order(store, &config, &order_id).await?.0))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvoiceOrderSummary {
    pub order_id: String,
    pub buyer_name: String,
    pub status: Option<OrderStatus>,
    pub next_action: Option<String>,
    pub invoice_total_cents: Option<u64>,
    pub replay_error: Option<String>,
}

pub(crate) async fn list_invoice_orders(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> InvoicingResult<Json<Vec<InvoiceOrderSummary>>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let mut summaries = Vec::new();
    for order in store.list_invoice_orders(500).await.map_err(internal)? {
        let view = load_order(store, &config, &order.order_id).await;
        summaries.push(match view {
            Ok((view, _)) => InvoiceOrderSummary {
                order_id: order.order_id,
                buyer_name: order.buyer_name,
                status: Some(view.state.status),
                next_action: Some(view.state.next_action.clone()),
                invoice_total_cents: Some(view.state.quote.invoice_total_cents),
                replay_error: None,
            },
            Err((_, Json(error))) => InvoiceOrderSummary {
                order_id: order.order_id,
                buyer_name: order.buyer_name,
                status: None,
                next_action: None,
                invoice_total_cents: None,
                replay_error: Some(error.message.clone()),
            },
        });
    }
    Ok(Json(summaries))
}

fn response_id(report: &StripeExecutionReport, prefix: &str) -> InvoicingResult<String> {
    report
        .stripe_id
        .clone()
        .filter(|id| id.starts_with(prefix))
        .ok_or_else(|| {
            agent_action_error(
                StatusCode::BAD_GATEWAY,
                "stripe_response_invalid",
                format!("Stripe did not return a {prefix} id"),
                true,
                "Retry the identical request.",
            )
        })
}

/// Creates the customer, draft invoice and line items, finalizes the invoice, records it, and
/// sends it. Each Stripe id is stored before the next step, and a stored invoice is read back
/// before it is changed, so a retry resumes rather than duplicating. The order records the
/// invoice before it is sent, so the buyer can never pay an invoice the order does not know.
pub(crate) async fn issue_invoice(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> InvoicingResult<Json<InvoiceOrderView>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let (view, _) = load_order(store, &config, &order_id).await?;
    let send_error = |error: payments_stripe::StripeIntegrationError| {
        bad_request("invalid_invoice_order", error.to_string())
    };
    if let (OrderStatus::Invoiced, Some(invoice)) =
        (view.state.status, view.state.invoice_id.as_deref())
    {
        // A retry after the invoice was recorded only (re)sends it; sending is idempotent.
        execute_stripe(
            &state,
            &plan_send_invoice(&order_id, invoice).map_err(send_error)?,
        )
        .await?;
        return Ok(Json(load_order(store, &config, &order_id).await?.0));
    }
    if view.state.status != OrderStatus::Quoted {
        return Err(conflict(
            "invoice_already_issued",
            "only a quoted order can be invoiced",
            &view.state.next_action,
        ));
    }
    if view.state.quote.platform_fee_bps != config.policy.platform_fee_bps {
        return Err(conflict(
            "invoice_fee_mismatch",
            "the configured v2 factory fee no longer matches this quote",
            "Create a new order at the current fee.",
        ));
    }
    let order = StripeInvoiceOrder {
        order_id: order_id.clone(),
        buyer_name: view.order.buyer_name.clone(),
        buyer_email: view.order.buyer_email.clone(),
        lines: view
            .state
            .quote
            .line_items
            .iter()
            .map(|line| StripeInvoiceLine {
                code: line.code.clone(),
                description: line.description.clone(),
                amount_cents: line.amount_cents,
            })
            .collect(),
        days_until_due: config.policy.days_until_due,
        terms_url: config.business_terms_url.clone(),
        terms_sha256: config.business_terms_sha256.clone(),
    };
    let plan_error = |error: payments_stripe::StripeIntegrationError| {
        bad_request("invalid_invoice_order", error.to_string())
    };
    let customer = match view.order.stripe_customer_id.clone() {
        Some(customer) => customer,
        None => {
            let report =
                execute_stripe(&state, &order.plan_customer().map_err(plan_error)?).await?;
            let customer = response_id(&report, "cus_")?;
            store
                .set_invoice_order_stripe_refs(&order_id, Some(&customer), None, None)
                .await
                .map_err(internal)?;
            customer
        }
    };
    let stored_invoice = view.order.stripe_invoice_id.clone();
    let invoice = match stored_invoice.clone() {
        Some(invoice) => invoice,
        None => {
            let report = execute_stripe(
                &state,
                &order.plan_draft_invoice(&customer).map_err(plan_error)?,
            )
            .await?;
            let invoice = response_id(&report, "in_")?;
            store
                .set_invoice_order_stripe_refs(&order_id, None, Some(&invoice), None)
                .await
                .map_err(internal)?;
            invoice
        }
    };
    // A stored invoice may already be finalized by an interrupted run; read its real status
    // instead of re-adding items, whose idempotency keys may have expired.
    let existing = match stored_invoice {
        Some(_) => Some(
            execute_stripe(
                &state,
                &plan_retrieve_invoice(&invoice).map_err(plan_error)?,
            )
            .await?,
        ),
        None => None,
    };
    let existing_status = existing
        .as_ref()
        .and_then(|report| report.response.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("draft")
        .to_string();
    let finalized = match (existing_status.as_str(), existing) {
        ("draft", _) => {
            for item in order
                .plan_invoice_items(&customer, &invoice)
                .map_err(plan_error)?
            {
                execute_stripe(&state, &item).await?;
            }
            execute_stripe(
                &state,
                &plan_finalize_invoice(&order_id, &invoice).map_err(plan_error)?,
            )
            .await?
        }
        ("open", Some(report)) => report,
        (status, _) => {
            return Err(conflict(
                "invoice_stray_status",
                format!("the stored Stripe invoice is {status}, not draft or open"),
                "Investigate the invoice in Stripe, then create a new order.",
            ))
        }
    };
    let total = finalized
        .response
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let hosted_url = finalized
        .response
        .get("hosted_invoice_url")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    if total != view.state.quote.invoice_total_cents
        || finalized.livemode != Some(config.policy.livemode)
    {
        return Err(conflict(
            "invoice_total_mismatch",
            format!(
                "Stripe finalized {total} cents but the quote is {} cents; void the invoice and re-quote",
                view.state.quote.invoice_total_cents
            ),
            "Void the Stripe invoice and create a new order.",
        ));
    }
    store
        .set_invoice_order_stripe_refs(&order_id, None, None, hosted_url.as_deref())
        .await
        .map_err(internal)?;
    require_applied(
        append_order_event(
            store,
            &config,
            &order_id,
            OrderEvent::InvoiceIssued {
                invoice_id: invoice.clone(),
                total_cents: total,
            },
        )
        .await?,
    )?;
    execute_stripe(
        &state,
        &plan_send_invoice(&order_id, &invoice).map_err(plan_error)?,
    )
    .await?;
    Ok(Json(load_order(store, &config, &order_id).await?.0))
}

pub(crate) async fn void_invoice(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> InvoicingResult<Json<StripeExecutionReport>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let (view, _) = load_order(store, &config, &order_id).await?;
    let invoice = view
        .state
        .invoice_id
        .clone()
        .filter(|_| view.state.status == OrderStatus::Invoiced)
        .ok_or_else(|| {
            conflict(
                "invoice_not_voidable",
                "only an issued, unpaid invoice can be voided",
                &view.state.next_action,
            )
        })?;
    let intent = plan_void_invoice(&order_id, &invoice)
        .map_err(|error| bad_request("invalid_invoice_order", error.to_string()))?;
    Ok(Json(execute_stripe(&state, &intent).await?))
}

/// Signature-verified Stripe webhook for invoiced orders. Events that are not about an invoiced
/// order are acknowledged and ignored. A rejected transition is acknowledged with its reason, so
/// Stripe does not retry evidence that can never apply.
pub(crate) async fn reconcile_invoice_webhook(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> InvoicingResult<Json<InvoiceEventOutcome>> {
    let secret = env::var("STRIPE_INVOICE_WEBHOOK_SECRET")
        .ok()
        .filter(|secret| !secret.trim().is_empty())
        .ok_or_else(|| {
            unavailable(
                "invoicing_webhook_unconfigured",
                "STRIPE_INVOICE_WEBHOOK_SECRET is required",
            )
        })?;
    let signature = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| bad_request("invalid_signature", "missing stripe-signature"))?;
    verify_webhook_signature(&body, signature, secret.trim().as_bytes())
        .map_err(|_| bad_request("invalid_signature", "Stripe signature did not verify"))?;
    let config = invoicing_config(&state)?;
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| unavailable("invoicing_store_unavailable", "DATABASE_URL is required"))?;
    let Some(evidence) = parse_invoice_webhook(&body)
        .map_err(|error| bad_request("invalid_invoice_event", error.to_string()))?
    else {
        return Ok(Json(InvoiceEventOutcome {
            applied: false,
            duplicate: false,
            reason: Some("not an invoiced-order event".to_string()),
            state: None,
        }));
    };
    let order_id = evidence.order_id().to_string();
    if store
        .get_invoice_order(&order_id)
        .await
        .map_err(internal)?
        .is_none()
    {
        return Ok(Json(InvoiceEventOutcome {
            applied: false,
            duplicate: false,
            reason: Some("unknown order".to_string()),
            state: None,
        }));
    }
    let event = match evidence {
        InvoiceWebhookEvidence::InvoicePaid {
            stripe_event_id,
            invoice_id,
            order_id,
            status,
            amount_paid_cents,
            amount_remaining_cents,
            currency,
            livemode,
            paid_out_of_band,
        } => OrderEvent::InvoicePaid(InvoicePaidEvidence {
            stripe_event_id,
            invoice_id,
            order_id,
            status,
            amount_paid_cents,
            amount_remaining_cents,
            currency,
            livemode,
            paid_out_of_band,
        }),
        InvoiceWebhookEvidence::InvoiceVoided {
            stripe_event_id,
            invoice_id,
            livemode,
            ..
        } => OrderEvent::InvoiceVoided {
            invoice_id,
            stripe_event_id,
            livemode,
        },
        InvoiceWebhookEvidence::CreditNoteCreated {
            stripe_event_id,
            credit_note_id,
            invoice_id,
            status,
            total_cents,
            livemode,
            ..
        } => {
            if status != "issued" {
                return Ok(Json(InvoiceEventOutcome {
                    applied: false,
                    duplicate: false,
                    reason: Some(format!("credit note status is {status}")),
                    state: None,
                }));
            }
            OrderEvent::CreditNoteIssued {
                credit_note_id,
                invoice_id,
                refund_cents: total_cents,
                stripe_event_id,
                livemode,
            }
        }
    };
    let early_payment = matches!(event, OrderEvent::InvoicePaid(_));
    let outcome = append_order_event(store, &config, &order_id, event).await?;
    if early_payment && !outcome.applied && !outcome.duplicate {
        // A payment that arrives before the order records its invoice may apply later: answer
        // non-2xx so Stripe redelivers it instead of the evidence being dropped.
        let (view, _) = load_order(store, &config, &order_id).await?;
        if view.state.status == OrderStatus::Quoted {
            return Err(agent_action_error(
                StatusCode::CONFLICT,
                "invoice_payment_not_yet_applicable",
                "the order has not recorded its invoice yet",
                true,
                "Stripe redelivers this event; finish issuing the invoice.",
            ));
        }
    }
    Ok(Json(outcome))
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct InvoiceFundingPlanRequest {
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvoiceFundingPlan {
    pub state: OrderState,
    pub terms_hash: String,
    pub creation: AutonomousBountyV2CreationPlan,
    pub signer: String,
    pub evidence_boundary: String,
}

/// The published terms for an invoiced bounty: v2 economics equal to the quote, the treasury as
/// creator, the contractor claim gate, and the configured verifier quorum.
pub(crate) fn invoice_bounty_terms_document(
    config: &InvoicingConfig,
    state: &OrderState,
    request: &InvoiceFundingPlanRequest,
    now: u64,
) -> InvoicingResult<AutonomousBountyTermsDocument> {
    let descriptor = base_network_descriptor(&config.network)
        .map_err(|_| unavailable("invoicing_misconfigured", "INVOICING_NETWORK is invalid"))?;
    if descriptor.chain_id == 84_532
        && !descriptor
            .native_usdc_token_address
            .eq_ignore_ascii_case(BASE_SEPOLIA_USDC_TOKEN_ADDRESS)
    {
        return Err(internal("Base Sepolia USDC descriptor changed"));
    }
    let usdc = |amount: u64| json!({"amount": amount, "currency": "usdc"});
    let quote = &state.quote;
    Ok(AutonomousBountyTermsDocument {
        schema_version: INVOICE_TERMS_SCHEMA.to_string(),
        contract_terms: json!({
            "protocol_version": AUTONOMOUS_V2_PROTOCOL_VERSION,
            "network": config.network,
            "settlement_token": descriptor.native_usdc_token_address,
            "creator_wallet": config.treasury,
            "solver_reward": usdc(quote.solver_reward_usdc),
            "verifier_reward": usdc(quote.verifier_reward_usdc),
            "claim_bond": usdc(quote.verifier_reward_usdc),
            "initial_funding": usdc(quote.funding_target_usdc),
            "funding_deadline": now + config.funding_window_seconds,
            "claim_window_seconds": config.claim_window_seconds,
            "verification_window_seconds": config.verification_window_seconds,
            "creation_nonce": keccak_hex(format!("agent-bounties/invoice-order:{}", state.order_id).as_bytes()),
            "platform_fee_bps": config.fee.platform_fee_bps,
            "platform_fee": usdc(quote.platform_fee_usdc),
            "platform_fee_recipient": config.fee.platform_fee_recipient,
            "claim_eligibility_registry": config.contractor_registry,
            "claim_eligibility_source": invoice_contractor_source_hash(),
        }),
        title: quote.title.clone(),
        goal: request.goal.clone(),
        acceptance_criteria: request.acceptance_criteria.clone(),
        benchmark: json!({
            "engine": "invoice-acceptance-review-v1",
            "order_id": state.order_id,
            "business_terms_sha256": state.terms_sha256,
        }),
        evidence_schema: json!({
            "type": "object",
            "required": ["artifact_reference", "summary"],
        }),
        verification_policy: json!({
            "mechanism": config.verification_mode,
            "threshold": config.verifier_threshold,
            "verifiers": config.verifiers,
            "verifier_module": null,
            "verifier_reward_recipient": null,
            "claim_gate": INVOICE_CONTRACTOR_SOURCE,
        }),
        source_url: request.source_url.clone(),
        discovery_source: Some("invoiced-seller-of-record".to_string()),
        image: None,
        agent_eligibility: None,
        claim_coordination: None,
    })
}

/// Publishes the bounty terms and returns the treasury's unsigned v2 creation plan. Funding is
/// recognized only from the canonical `FundingAdded` that reconciliation later observes.
pub(crate) async fn plan_invoice_funding(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
    Json(request): Json<InvoiceFundingPlanRequest>,
) -> InvoicingResult<Json<InvoiceFundingPlan>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let (view, _) = load_order(store, &config, &order_id).await?;
    if view.state.status != OrderStatus::Paid {
        return Err(conflict(
            "invoice_not_fundable",
            "treasury funding can be planned only for a verified paid order",
            &view.state.next_action,
        ));
    }
    let now_time = Utc::now();
    let now = u64::try_from(now_time.timestamp()).map_err(internal)?;
    let document = invoice_bounty_terms_document(&config, &view.state, &request, now)?;
    let record = build_autonomous_bounty_terms_record(&config.treasury, document, now_time)
        .map_err(|error| bad_request("invalid_invoice_terms", error.to_string()))?;
    let create = autonomous_v2_create_from_terms(&record, &config.fee).map_err(|error| {
        conflict(
            "invoice_terms_mismatch",
            error.to_string(),
            "Re-plan funding.",
        )
    })?;
    if create.claim_eligibility_registry.as_deref() != Some(config.contractor_registry.as_str())
        || create.claim_eligibility_source.as_deref()
            != Some(invoice_contractor_source_hash().as_str())
    {
        return Err(internal("invoice terms lost the contractor claim gate"));
    }
    let platform_fee = validate_autonomous_v2_creation_against_terms(
        &config.network,
        &create,
        &record,
        &config.fee,
    )
    .map_err(|error| {
        conflict(
            "invoice_terms_mismatch",
            error.to_string(),
            "Re-plan funding.",
        )
    })?;
    if platform_fee != u128::from(view.state.quote.platform_fee_usdc) {
        return Err(conflict(
            "invoice_fee_mismatch",
            "the configured v2 factory fee no longer matches the quote",
            "Cancel and refund the order, then re-quote.",
        ));
    }
    let creation = config
        .planner
        .plan_v2_creation(&config.network, &create, &config.fee)
        .map_err(|error| bad_request("invalid_invoice_terms", error.to_string()))?;
    store
        .upsert_autonomous_bounty_terms(&record)
        .await
        .map_err(internal)?;
    let state_after = require_applied(
        append_order_event(
            store,
            &config,
            &order_id,
            OrderEvent::FundingPlanned {
                treasury: config.treasury.clone(),
                bounty_id: creation.plan.bounty_id.clone(),
                bounty_contract: creation.plan.predicted_bounty_contract.clone(),
                funding_target_usdc: view.state.quote.funding_target_usdc,
            },
        )
        .await?,
    )?;
    Ok(Json(InvoiceFundingPlan {
        state: state_after,
        terms_hash: record.terms_hash,
        creation,
        signer: config.treasury,
        evidence_boundary: "The treasury signs these calls. A plan, signature, or transaction hash is not funding; reconcile after the canonical FundingAdded is indexed.".to_string(),
    }))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvoiceReconciliation {
    pub state: OrderState,
    pub applied: Vec<String>,
    pub rejected: Vec<String>,
}

/// Applies indexed canonical events for the order's bounty: treasury `FundingAdded`,
/// `BountySettled`, and the treasury's `RefundWithdrawn`.
pub(crate) async fn reconcile_invoice_order(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> InvoicingResult<Json<InvoiceReconciliation>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let (view, _) = load_order(store, &config, &order_id).await?;
    let Some(contract) = view.state.bounty_contract.clone() else {
        return Err(conflict(
            "invoice_not_planned",
            "the order has no planned bounty yet",
            &view.state.next_action,
        ));
    };
    let (applied, rejected) =
        apply_indexed_chain_events(store, &config, &order_id, &contract).await?;
    Ok(Json(InvoiceReconciliation {
        state: load_order(store, &config, &order_id).await?.0.state,
        applied,
        rejected,
    }))
}

/// Appends every indexed canonical event for the order's planned bounty, returning the applied
/// and rejected log keys. Duplicates are skipped.
async fn apply_indexed_chain_events(
    store: &PostgresStore,
    config: &InvoicingConfig,
    order_id: &str,
    contract: &str,
) -> InvoicingResult<(Vec<String>, Vec<String>)> {
    let contract = contract.to_string();
    let events = store
        .list_autonomous_bounty_history_for_contract(
            &config.network,
            &config.planner.factory_contract,
            &contract,
        )
        .await
        .map_err(internal)?;
    let (mut applied, mut rejected) = (Vec::new(), Vec::new());
    for event in events
        .iter()
        .filter(|event| event.contract_address.eq_ignore_ascii_case(&contract))
    {
        let data = &event.data;
        let text = |key: &str| data[key].as_str().unwrap_or_default().to_string();
        let amount = |key: &str| data[key].as_u64().unwrap_or_default();
        let order_event = match event.kind {
            AutonomousBountyEventKind::FundingAdded => OrderEvent::FundingObserved {
                bounty_contract: contract.clone(),
                contributor: text("contributor"),
                amount_usdc: amount("amount"),
                log_key: event.log_key.clone(),
            },
            AutonomousBountyEventKind::BountySettled => OrderEvent::SettlementObserved {
                bounty_contract: contract.clone(),
                solver: text("solver"),
                reportable_usdc: amount("solver_reward")
                    .saturating_add(amount("timeout_bond_bonus")),
                settled_at: u64::try_from(event.occurred_at.timestamp()).unwrap_or_default(),
                log_key: event.log_key.clone(),
            },
            AutonomousBountyEventKind::RefundWithdrawn => OrderEvent::RefundObserved {
                bounty_contract: contract.clone(),
                contributor: text("contributor"),
                amount_usdc: amount("amount"),
                log_key: event.log_key.clone(),
            },
            _ => continue,
        };
        let outcome = append_order_event(store, config, order_id, order_event).await?;
        if outcome.applied {
            applied.push(event.log_key.clone());
        } else if !outcome.duplicate {
            rejected.push(format!(
                "{}: {}",
                event.log_key,
                outcome.reason.unwrap_or_default()
            ));
        }
    }
    Ok((applied, rejected))
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CancelInvoiceOrderRequest {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvoiceCancellation {
    pub state: OrderState,
    /// The treasury's on-chain cancel and refund calls, when the bounty was funded.
    pub treasury_calls: Vec<EvmTransactionIntent>,
}

pub(crate) async fn cancel_invoice_order(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
    Json(request): Json<CancelInvoiceOrderRequest>,
) -> InvoicingResult<Json<InvoiceCancellation>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    // Decide from the latest indexed chain state, never from a stale projection.
    let (view, _) = load_order(store, &config, &order_id).await?;
    if let Some(contract) = view.state.bounty_contract.as_deref() {
        apply_indexed_chain_events(store, &config, &order_id, contract).await?;
    }
    let (view, _) = load_order(store, &config, &order_id).await?;
    // Repeating a cancellation returns the current treasury calls without a second event.
    let state_after = if view.state.status == OrderStatus::CancelRequested {
        view.state
    } else {
        require_applied(
            append_order_event(
                store,
                &config,
                &order_id,
                OrderEvent::CancellationRequested {
                    reason: request.reason,
                },
            )
            .await?,
        )?
    };
    let mut treasury_calls = Vec::new();
    if let (true, false, Some(contract)) = (
        state_after.funded,
        state_after.refunded_to_treasury,
        state_after.bounty_contract.as_deref(),
    ) {
        let plan_error = |error: ChainBaseError| internal(error);
        treasury_calls.push(
            config
                .planner
                .plan_cancel(contract, Some(&config.treasury))
                .map_err(plan_error)?,
        );
        treasury_calls.push(
            config
                .planner
                .plan_refund_withdrawal(contract, &config.treasury)
                .map_err(plan_error)?,
        );
    }
    Ok(Json(InvoiceCancellation {
        state: state_after,
        treasury_calls,
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RefundInvoiceOrderRequest {
    pub refund: CreditNoteRefund,
    #[serde(default)]
    pub memo: String,
}

/// Issues the buyer's credit note once no escrow is at risk. The order becomes `refunded` only
/// when the signed `credit_note.created` webhook arrives.
pub(crate) async fn refund_invoice_order(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
    Json(request): Json<RefundInvoiceOrderRequest>,
) -> InvoicingResult<Json<StripeExecutionReport>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let (view, _) = load_order(store, &config, &order_id).await?;
    if let Some(contract) = view.state.bounty_contract.as_deref() {
        apply_indexed_chain_events(store, &config, &order_id, contract).await?;
    }
    let (view, _) = load_order(store, &config, &order_id).await?;
    let due = view.state.refund_due_cents();
    let invoice = view
        .state
        .invoice_id
        .clone()
        .filter(|_| due > 0)
        .ok_or_else(|| {
            conflict(
                "invoice_refund_not_due",
                "a refund is due only after cancellation with no escrow at risk",
                &view.state.next_action,
            )
        })?;
    let intent = plan_credit_note_refund(&order_id, &invoice, due, request.refund, &request.memo)
        .map_err(|error| bad_request("invalid_refund", error.to_string()))?;
    Ok(Json(execute_stripe(&state, &intent).await?))
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UpsertContractorRequest {
    pub wallet: String,
    pub agreement_sha256: String,
    pub agreement_accepted_at: u64,
    pub tax_form: Option<::invoicing::TaxFormRecord>,
}

fn contractor_record(contractor_id: &str, value: Value) -> InvoicingResult<ContractorRecord> {
    let record: ContractorRecord = serde_json::from_value(value).map_err(internal)?;
    if record.contractor_id != contractor_id {
        return Err(internal("stored contractor id differs from its key"));
    }
    Ok(record)
}

pub(crate) async fn upsert_contractor(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(contractor_id): Path<String>,
    Json(request): Json<UpsertContractorRequest>,
) -> InvoicingResult<Json<ContractorRecord>> {
    let (_, store) = require_invoicing_operator(&state, &headers)?;
    let wallet = ::invoicing::normalize_address(&request.wallet)
        .ok_or_else(|| bad_request("invalid_wallet", "wallet is not an address"))?;
    if contractor_id.is_empty()
        || contractor_id.len() > 128
        || contractor_id.trim() != contractor_id
    {
        return Err(bad_request(
            "invalid_contractor_id",
            "contractor id must be 1 to 128 characters without surrounding whitespace",
        ));
    }
    // A contractor keeps their wallet history, so settlements to an earlier wallet still count
    // toward their 1099-NEC, and no wallet ever moves to a different contractor.
    let others = store
        .list_invoice_contractors()
        .await
        .map_err(internal)?
        .into_iter()
        .map(|value| serde_json::from_value::<ContractorRecord>(value).map_err(internal))
        .collect::<Result<Vec<_>, _>>()?;
    if others.iter().any(|other| {
        other.contractor_id != contractor_id
            && (other.wallet == wallet || other.previous_wallets.contains(&wallet))
    }) {
        return Err(conflict(
            "contractor_wallet_conflict",
            "this wallet belongs or belonged to another contractor",
            "Each wallet belongs to one contractor for good.",
        ));
    }
    let mut previous_wallets = Vec::new();
    if let Some(existing) = others
        .iter()
        .find(|other| other.contractor_id == contractor_id)
    {
        previous_wallets = existing.previous_wallets.clone();
        if existing.wallet != wallet && !previous_wallets.contains(&existing.wallet) {
            previous_wallets.push(existing.wallet.clone());
        }
        previous_wallets.retain(|previous| previous != &wallet);
    }
    let record = ContractorRecord {
        contractor_id: contractor_id.clone(),
        wallet: wallet.clone(),
        previous_wallets,
        agreement_sha256: request.agreement_sha256.to_ascii_lowercase(),
        agreement_accepted_at: request.agreement_accepted_at,
        tax_form: request.tax_form,
    };
    store
        .upsert_invoice_contractor(
            &contractor_id,
            &wallet,
            &serde_json::to_value(&record).map_err(internal)?,
        )
        .await
        .map_err(|error| {
            conflict(
                "contractor_wallet_conflict",
                error.to_string(),
                "Each wallet belongs to one contractor.",
            )
        })?;
    Ok(Json(record))
}

async fn eligible_contractor_attestation(
    store: &PostgresStore,
    config: &InvoicingConfig,
    contractor_id: &str,
    validity_seconds: u64,
    nonce: u64,
) -> InvoicingResult<ParticipantAttestationRequest> {
    let value = store
        .get_invoice_contractor(contractor_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            agent_action_error(
                StatusCode::NOT_FOUND,
                "contractor_not_found",
                "no contractor has this id",
                false,
                "Record the contractor first.",
            )
        })?;
    let record = contractor_record(contractor_id, value)?;
    let now = u64::try_from(Utc::now().timestamp()).map_err(internal)?;
    let attestation = contractor_attestation(
        &record,
        &config.contractor_agreement_sha256,
        now,
        validity_seconds,
    )
    .map_err(|error| {
        conflict(
            "contractor_ineligible",
            error.to_string(),
            "Collect the agreement and tax form first.",
        )
    })?;
    Ok(ParticipantAttestationRequest {
        registry: config.contractor_registry.clone(),
        wallet: attestation.wallet,
        participant_id: attestation.participant_id,
        source_hash: attestation.source_hash,
        valid_until: attestation.valid_until,
        nonce,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ContractorAttestationPlanRequest {
    /// The registry's current `nonces(wallet)`.
    pub nonce: u64,
    pub validity_days: u64,
}

pub(crate) async fn plan_contractor_attestation(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(contractor_id): Path<String>,
    Json(request): Json<ContractorAttestationPlanRequest>,
) -> InvoicingResult<Json<ParticipantAttestationTypedData>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let attestation = eligible_contractor_attestation(
        store,
        &config,
        &contractor_id,
        request.validity_days.saturating_mul(86_400),
        request.nonce,
    )
    .await?;
    plan_participant_attestation(&config.network, &attestation)
        .map(Json)
        .map_err(|error| bad_request("invalid_attestation", error.to_string()))
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ContractorRegistrationPlanRequest {
    pub nonce: u64,
    pub valid_until: u64,
    pub signature: String,
    #[serde(default)]
    pub relayer: Option<String>,
}

/// Re-checks eligibility, then plans the relayed `register` only for an attestation that the
/// configured attester signed.
pub(crate) async fn plan_contractor_registration(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(contractor_id): Path<String>,
    Json(request): Json<ContractorRegistrationPlanRequest>,
) -> InvoicingResult<Json<EvmTransactionIntent>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let now = u64::try_from(Utc::now().timestamp()).map_err(internal)?;
    let mut attestation = eligible_contractor_attestation(
        store,
        &config,
        &contractor_id,
        request.valid_until.saturating_sub(now),
        request.nonce,
    )
    .await?;
    attestation.valid_until = request.valid_until;
    plan_participant_registration(
        &config.network,
        &attestation,
        &request.signature,
        &config.contractor_attester,
        request.relayer.as_deref(),
    )
    .map(Json)
    .map_err(|error| {
        conflict(
            "invalid_attestation_signature",
            error.to_string(),
            "Sign the exact attestation plan with the registry attester.",
        )
    })
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ContractorPaymentsQuery {
    pub year: i64,
    #[serde(default)]
    pub indexed_threshold_cents: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ContractorPaymentsReport {
    pub summary: ContractorTaxSummary,
    /// Settlements whose solver wallet matches no contractor record. They need manual review.
    pub unmatched_settlements: Vec<String>,
    /// Orders whose log no longer replays, so their payments may be missing from `summary`.
    pub replay_errors: Vec<InvoiceReplayError>,
    /// True only when every order replayed and every settlement matched a contractor.
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InvoiceReplayError {
    pub order_id: String,
    pub error: String,
}

pub(crate) async fn contractor_payments(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<ContractorPaymentsQuery>,
) -> InvoicingResult<Json<ContractorPaymentsReport>> {
    let (config, store) = require_invoicing_operator(&state, &headers)?;
    let contractors = store
        .list_invoice_contractors()
        .await
        .map_err(internal)?
        .into_iter()
        .map(|value| serde_json::from_value::<ContractorRecord>(value).map_err(internal))
        .collect::<Result<Vec<_>, _>>()?;
    let mut payments = Vec::new();
    let mut unmatched = Vec::new();
    let mut replay_errors = Vec::new();
    // Every order, uncapped: a missed settlement would under-report a contractor's 1099-NEC.
    for order_id in store.list_invoice_order_ids().await.map_err(internal)? {
        let view = match load_order(store, &config, &order_id).await {
            Ok((view, _)) => view,
            Err(error) => {
                replay_errors.push(InvoiceReplayError {
                    order_id,
                    error: error.1 .0.message.clone(),
                });
                continue;
            }
        };
        let Some(settlement) = view.state.settlement else {
            continue;
        };
        let order = view.order;
        match contractors.iter().find(|contractor| {
            contractor.wallet.eq_ignore_ascii_case(&settlement.solver)
                || contractor
                    .previous_wallets
                    .iter()
                    .any(|wallet| wallet.eq_ignore_ascii_case(&settlement.solver))
        }) {
            Some(contractor) => payments.push(ContractorPayment {
                contractor_id: contractor.contractor_id.clone(),
                tax_form_kind: contractor
                    .tax_form
                    .as_ref()
                    .map(|form| form.kind)
                    .unwrap_or(::invoicing::TaxFormKind::W9),
                order_id: order.order_id.clone(),
                reportable_usdc: settlement.reportable_usdc,
                paid_at: settlement.settled_at,
                log_key: settlement.log_key.clone(),
            }),
            None => unmatched.push(order.order_id.clone()),
        }
    }
    let summary =
        summarize_contractor_payments(&payments, query.year, query.indexed_threshold_cents)
            .map_err(|error| {
                conflict(
                    "contractor_payments_invalid",
                    error.to_string(),
                    "Investigate duplicate settlements.",
                )
            })?;
    Ok(Json(ContractorPaymentsReport {
        summary,
        complete: unmatched.is_empty() && replay_errors.is_empty(),
        unmatched_settlements: unmatched,
        replay_errors,
    }))
}
