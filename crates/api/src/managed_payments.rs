//! Account-owned managed service orders. Client JSON never proves payment or funding.
use crate::{security_auth, SharedState};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, FromRequestParts, Path, State},
    http::{request::Parts, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use db::managed_service_orders::{ManagedOrderStatus, ManagedOrderStoreError};
use managed_orders::{
    stripe_checkout::{
        verify_managed_checkout_event, CheckoutPolicy, ManagedCheckoutPlan,
        MANAGED_CHECKOUT_PURPOSE,
    },
    stripe_refund::{
        verify_managed_refund_event, RefundMerchant, MANAGED_DEPOSIT_REFUND_PURPOSE,
        MANAGED_REFUND_PURPOSE,
    },
    ManagedQuote, OrderError, QuoteInput,
};
use serde::Deserialize;
use serde_json::{json, Value};
use service_runtime::{
    auth::Principal,
    managed_checkout::{
        execute_managed_checkout, ManagedCheckoutExecutionError, ManagedStripeClient,
    },
    managed_refund::{execute_managed_refund, ManagedRefundClient},
};
use uuid::Uuid;

mod limits;

#[derive(Clone, Default)]
pub struct ManagedPaymentsConfig {
    pub enabled: bool,
    pub public_checkout_enabled: bool,
    pub refunds_enabled: bool,
    pub recovered_refunds_enabled: bool,
    pub policy: Option<CheckoutPolicy>,
    pub maximum_usd_minor: Option<u64>,
    pub canary_account_id: Option<String>,
    pub bounty: Option<crate::managed_bounty_quotes::ManagedBountyConfig>,
    /// The dedicated customer-review key; it must be the configured, code-pinned verifier.
    pub review_signer: Option<alloy::signers::local::PrivateKeySigner>,
}
impl ManagedPaymentsConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let public_checkout_enabled = match std::env::var("ENABLE_MANAGED_SERVICE_PUBLIC_CHECKOUT")
        {
            Ok(value) if value == "true" => true,
            Ok(value) if value == "false" => false,
            Err(std::env::VarError::NotPresent) => false,
            _ => anyhow::bail!("ENABLE_MANAGED_SERVICE_PUBLIC_CHECKOUT must be true or false"),
        };
        let maximum_usd_minor = match std::env::var("MANAGED_SERVICE_MAXIMUM_USD_MINOR") {
            Ok(value) => limits::parse_limit(Some(&value))?,
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => anyhow::bail!("MANAGED_SERVICE_MAXIMUM_USD_MINOR is invalid"),
        };
        let refunds_enabled = match std::env::var("ENABLE_MANAGED_SERVICE_REFUND_EXECUTION")
            .ok()
            .as_deref()
        {
            None | Some("false") => false,
            Some("true") => true,
            _ => anyhow::bail!("ENABLE_MANAGED_SERVICE_REFUND_EXECUTION must be true or false"),
        };
        let recovered_refunds_enabled =
            match std::env::var("ENABLE_MANAGED_RECOVERED_BOUNTY_REFUNDS")
                .ok()
                .as_deref()
            {
                None | Some("false") => false,
                Some("true") => true,
                _ => anyhow::bail!("ENABLE_MANAGED_RECOVERED_BOUNTY_REFUNDS must be true or false"),
            };
        anyhow::ensure!(
            !recovered_refunds_enabled || refunds_enabled,
            "recovered bounty refunds require managed refund execution"
        );
        let enabled = match std::env::var("ENABLE_MANAGED_SERVICE_ORDERS")
            .ok()
            .as_deref()
        {
            None | Some("false") => false,
            Some("true") => true,
            _ => anyhow::bail!("ENABLE_MANAGED_SERVICE_ORDERS must be true or false"),
        };
        let policy = std::env::var("MANAGED_SERVICE_CHECKOUT_POLICY")
            .ok()
            .map(|value| serde_json::from_str::<CheckoutPolicy>(&value))
            .transpose()
            .map_err(|_| anyhow::anyhow!("MANAGED_SERVICE_CHECKOUT_POLICY is invalid"))?;
        let canary_account_id = std::env::var("MANAGED_SERVICE_CANARY_ACCOUNT_ID").ok();
        anyhow::ensure!(
            canary_account_id.as_ref().is_none_or(|id| account_id(id)),
            "MANAGED_SERVICE_CANARY_ACCOUNT_ID is invalid"
        );
        anyhow::ensure!(
            !(enabled || refunds_enabled) || policy.is_some(),
            "managed order intake requires a Checkout policy"
        );
        let bounty = crate::managed_bounty_quotes::ManagedBountyConfig::from_env()?;
        let review_signer = std::env::var("CUSTOMER_REVIEW_SIGNER_PRIVATE_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .map(|key| {
                key.trim()
                    .parse::<alloy::signers::local::PrivateKeySigner>()
                    .map_err(|_| anyhow::anyhow!("CUSTOMER_REVIEW_SIGNER_PRIVATE_KEY is invalid"))
            })
            .transpose()?;
        if let Some(signer) = &review_signer {
            anyhow::ensure!(
                bounty.as_ref().is_some_and(|config| config
                    .customer_review
                    .verifier
                    .eq_ignore_ascii_case(&format!("{:#x}", signer.address()))),
                "the customer review key must be MANAGED_BOUNTY_QUOTES customer_review.verifier"
            );
        }
        Ok(Self {
            enabled,
            public_checkout_enabled,
            refunds_enabled,
            recovered_refunds_enabled,
            policy,
            maximum_usd_minor,
            canary_account_id,
            bounty,
            review_signer,
        })
    }
    pub(crate) fn price_allowed(&self, quote: &ManagedQuote) -> bool {
        limits::allows(self.maximum_usd_minor, quote)
    }
    pub(crate) fn intake_allowed(&self, state: &crate::AppState, buyer: &str) -> bool {
        self.enabled
            && self.policy.is_some()
            && (self.public_checkout_enabled
                || state.stripe_public_checkout_enabled
                || self
                    .canary_account_id
                    .as_ref()
                    .is_some_and(|id| buyer == format!("account:{id}")))
    }
}
fn account_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn status(error: ManagedOrderStoreError) -> StatusCode {
    match error {
        ManagedOrderStoreError::NotFound => StatusCode::NOT_FOUND,
        ManagedOrderStoreError::Quote(OrderError::Conflict) => StatusCode::CONFLICT,
        ManagedOrderStoreError::Quote(OrderError::Expired) => StatusCode::GONE,
        ManagedOrderStoreError::Quote(_)
        | ManagedOrderStoreError::Checkout(_)
        | ManagedOrderStoreError::Refund(_) => StatusCode::BAD_REQUEST,
        ManagedOrderStoreError::StaleLease => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}
pub(crate) fn store(state: &SharedState) -> Result<&db::PostgresStore, StatusCode> {
    state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
pub(crate) fn policy(state: &SharedState) -> Result<&CheckoutPolicy, StatusCode> {
    let policy = state
        .managed_payments
        .policy
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if state.stripe_webhook_livemode != Some(policy.livemode)
        || state
            .stripe_webhook_secret
            .as_ref()
            .is_none_or(|s| s.is_empty())
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(policy)
}
fn unambiguous_headers(headers: &HeaderMap) -> Result<(), StatusCode> {
    for key in ["cookie", "origin", "authorization", "x-operator-token"] {
        if headers.get_all(key).iter().count() > 1 {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    if let Some(cookie) = headers.get("cookie") {
        let cookie = cookie.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
        if cookie
            .split(';')
            .filter_map(|part| part.trim().split_once('='))
            .filter(|(name, _)| *name == "agent_bounties_session")
            .count()
            > 1
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(())
}
async fn buyer(
    state: &SharedState,
    headers: &HeaderMap,
    write: bool,
) -> Result<String, StatusCode> {
    unambiguous_headers(headers)?;
    let principal = if write {
        security_auth::principal_for_write(state, headers).await?
    } else {
        security_auth::principal(state, headers).await?
    };
    match principal {
        Principal::Account { account_id: id } if account_id(&id) => Ok(format!("account:{id}")),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
pub struct BuyerRead(pub(crate) String);
pub struct BuyerWrite(pub(crate) String);
pub struct Merchant;
#[axum::async_trait]
impl FromRequestParts<SharedState> for BuyerRead {
    type Rejection = StatusCode;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, StatusCode> {
        buyer(state, &parts.headers, false).await.map(Self)
    }
}
#[axum::async_trait]
impl FromRequestParts<SharedState> for BuyerWrite {
    type Rejection = StatusCode;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, StatusCode> {
        buyer(state, &parts.headers, true).await.map(Self)
    }
}
#[axum::async_trait]
impl FromRequestParts<SharedState> for Merchant {
    type Rejection = StatusCode;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, StatusCode> {
        unambiguous_headers(&parts.headers)?;
        if security_auth::principal_for_write(state, &parts.headers).await? != Principal::Operator {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(Self)
    }
}
pub(crate) async fn quota(
    state: &SharedState,
    buyer: &str,
    action: &str,
) -> Result<(), StatusCode> {
    // Each request consumes capacity; reusing an order/key cannot bypass this rate bound.
    let request_id = Uuid::new_v4();
    let key = service_runtime::auth::hash_secret(&request_id.to_string());
    store(state)?
        .consume_security_quota_once(buyer, action, &key, &key, 60, 60)
        .await
        .map_err(|e| security_auth::status(e.into()))?;
    if request_id.as_bytes()[0] & 15 == 0 {
        store(state)?
            .prune_security_auth_state()
            .await
            .map_err(|e| security_auth::status(e.into()))?;
    }
    Ok(())
}
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/v1/managed-service/quotes", post(issue_quote))
        .route(
            "/v1/managed-service/bounty-quotes",
            post(crate::managed_bounty_quotes::issue_bounty_quote),
        )
        .route("/v1/managed-service/quotes/:id", get(read_quote))
        .route("/v1/managed-service/quotes/:id/accept", post(accept_quote))
        .route("/v1/managed-service/orders/:id", get(read_order))
        .route("/v1/managed-service/orders/:id/checkout", post(checkout))
        .route(
            "/v1/managed-service/orders/:id/refund",
            post(request_refund),
        )
        .route(
            "/v1/managed-service/orders/:id/review",
            get(crate::managed_bounty_reviews::read_review)
                .post(crate::managed_bounty_reviews::record_review),
        )
        .route(
            "/v1/managed-service/bounties/:network/:contract/review",
            get(crate::managed_bounty_appeals::read_bounty_review),
        )
        .route(
            "/v1/managed-service/bounties/:network/:contract/appeal",
            post(crate::managed_bounty_appeals::appeal),
        )
        .route(
            "/v1/managed-service/appeals",
            get(crate::managed_bounty_appeals::list_appeals),
        )
        .route(
            "/v1/managed-service/appeals/:network/:contract/:round/ruling",
            post(crate::managed_bounty_appeals::rule),
        )
        .route(
            "/v1/managed-service/deposit-payouts",
            get(crate::managed_bounty_appeals::list_deposit_payouts),
        )
        .route(
            "/v1/managed-service/refund-drift",
            get(crate::managed_bounty_appeals::list_refund_drift),
        )
        .route(
            "/v1/managed-service/deposit-payouts/:network/:contract",
            post(crate::managed_bounty_appeals::record_deposit_payout),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
}

#[utoipa::path(post,path="/v1/managed-service/quotes",request_body=Value,responses((status=200,description="Operator-issued immutable quote; no payment or funding"),(status=401,description="Merchant operator credential required"),(status=422,description="The total quote exceeds the configured USD order limit"),(status=503,description="Managed intake or merchant configuration unavailable")))]
pub async fn issue_quote(
    State(state): State<SharedState>,
    _: Merchant,
    Json(input): Json<QuoteInput>,
) -> Result<Json<Value>, StatusCode> {
    let quote = ManagedQuote::new(input).map_err(|_| StatusCode::BAD_REQUEST)?;
    let q = quote.input();
    q.buyer_principal
        .strip_prefix("account:")
        .filter(|id| account_id(id))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let db = store(&state)?;
    if let Some(existing) = db
        .get_managed_service_quote(&q.buyer_principal, q.quote_id)
        .await
        .map_err(status)?
    {
        if existing != quote {
            return Err(StatusCode::CONFLICT);
        }
        return Ok(Json(
            json!({"quote":quote,"quote_digest":quote.digest().map_err(|_|StatusCode::BAD_REQUEST)?}),
        ));
    }
    if !state
        .managed_payments
        .intake_allowed(&state, &q.buyer_principal)
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if !state.managed_payments.price_allowed(&quote) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let now = chrono::Utc::now().timestamp();
    if q.issued_at > now || q.issued_at < now - 300 || q.expires_at > now + 86400 {
        return Err(StatusCode::BAD_REQUEST);
    }
    // The trusted operator supplies offered pricing; the merchant policy still binds account,
    // mode, currency, payment-method configuration and bounds. A buyer cannot invoke this route.
    ManagedCheckoutPlan::new(
        &quote,
        Uuid::new_v4(),
        Uuid::new_v4(),
        policy(&state)?.clone(),
        now,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    quota(&state, "operator", "managed_quote_issue").await?;
    db.put_managed_service_quote(&quote).await.map_err(status)?;
    Ok(Json(
        json!({"quote":quote,"quote_digest":quote.digest().map_err(|_|StatusCode::BAD_REQUEST)?}),
    ))
}

#[utoipa::path(get,path="/v1/managed-service/quotes/{id}",params(("id"=Uuid,Path,description="Immutable quote or order ID")),responses((status=200,description="The signed-in account's immutable itemized offer and digest"),(status=401,description="Website account login required"),(status=404,description="Quote not owned by this account")))]
pub async fn read_quote(
    State(state): State<SharedState>,
    BuyerRead(buyer): BuyerRead,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    quota(&state, &buyer, "managed_read").await?;
    let quote = store(&state)?
        .get_managed_service_quote(&buyer, id)
        .await
        .map_err(status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(
        json!({"quote":quote,"quote_digest":quote.digest().map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    pub quote_digest: String,
    pub idempotency_key: String,
}
#[utoipa::path(post,path="/v1/managed-service/quotes/{id}/accept",params(("id"=Uuid,Path,description="Immutable quote or order ID")),request_body=Value,responses((status=200,description="Exact quote acceptance or committed replay; payment still required"),(status=401,description="Website account login and allowed Origin required"),(status=404,description="Quote not owned by account"),(status=409,description="Digest or retry key conflicts"),(status=503,description="New intake disabled or storage unavailable")))]
pub async fn accept_quote(
    State(state): State<SharedState>,
    BuyerWrite(buyer): BuyerWrite,
    Path(id): Path<Uuid>,
    Json(request): Json<Acceptance>,
) -> Result<Json<Value>, StatusCode> {
    quota(&state, &buyer, "managed_accept").await?;
    let quote = store(&state)?
        .get_managed_service_quote(&buyer, id)
        .await
        .map_err(status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let compatible = policy(&state).is_ok_and(|p| {
        p.stripe_account_id == quote.input().stripe_account_id
            && p.livemode == quote.input().livemode
            && p.currency == quote.input().currency
            && quote.total_minor() >= p.minimum_minor
            && quote.total_minor() <= p.maximum_minor
    });
    let allowed = state.managed_payments.intake_allowed(&state, &buyer)
        && compatible
        && state.managed_payments.price_allowed(&quote);
    let order = store(&state)?
        .accept_managed_service_quote_with_intake(
            &buyer,
            id,
            &request.quote_digest,
            &request.idempotency_key,
            allowed,
        )
        .await
        .map_err(status)?;
    Ok(Json(json!({"order":order})))
}
#[utoipa::path(get,path="/v1/managed-service/orders/{id}",params(("id"=Uuid,Path,description="Immutable quote or order ID")),responses((status=200,description="Owner-scoped order status; paid is distinct from bank-settled and funded"),(status=401,description="Website account login required"),(status=404,description="Order not owned by account")))]
pub async fn read_order(
    State(state): State<SharedState>,
    BuyerRead(buyer): BuyerRead,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    quota(&state, &buyer, "managed_read").await?;
    let order = store(&state)?
        .get_managed_service_order(&buyer, id)
        .await
        .map_err(status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let recovered_refund = store(&state)?
        .managed_recovered_refund_state(id)
        .await
        .map_err(status)?;
    Ok(Json(
        json!({"order":order,"recovered_bounty_refund":recovered_refund}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}
#[utoipa::path(post,path="/v1/managed-service/orders/{id}/checkout",params(("id"=Uuid,Path,description="Immutable quote or order ID")),request_body=Value,responses((status=200,description="Owner Checkout session or bounded readback; response is not payment evidence"),(status=401,description="Website account login and allowed Origin required"),(status=404,description="Order not owned by account"),(status=409,description="Reconciliation required; do not create a replacement order"),(status=503,description="New Checkout disabled or merchant unavailable")))]
pub async fn checkout(
    State(state): State<SharedState>,
    BuyerWrite(buyer): BuyerWrite,
    Path(id): Path<Uuid>,
    Json(_): Json<Empty>,
) -> Result<Json<Value>, StatusCode> {
    let db = store(&state)?;
    let order = db
        .get_managed_service_order(&buyer, id)
        .await
        .map_err(status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    quota(&state, &buyer, "managed_checkout").await?;
    if !matches!(
        order.status,
        ManagedOrderStatus::AwaitingPayment
            | ManagedOrderStatus::PaymentPending
            | ManagedOrderStatus::PaymentFailed
            | ManagedOrderStatus::PaymentExpired
    ) {
        return Ok(Json(
            json!({"status":"payment_received","order_id":id,"order_status":order.status}),
        ));
    }
    let policy = policy(&state)?.clone();
    let allow_create = state.managed_payments.intake_allowed(&state, &buyer)
        && state.stripe_live_execution_enabled
        && state.managed_payments.price_allowed(&order.quote);
    // Reject missing/mismatched credentials before starting the operation's finite lifetime.
    let secret = state
        .stripe_secret_key
        .as_deref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let client = ManagedStripeClient::new(secret, policy.clone())
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if db
        .get_managed_checkout_plan(&buyer, id)
        .await
        .map_err(status)?
        .is_none()
    {
        if !allow_create {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        db.prepare_managed_checkout(&buyer, id, policy.clone())
            .await
            .map_err(status)?;
    }
    let result = execute_managed_checkout(db, &client, id, allow_create)
        .await
        .map_err(|error| match error {
            ManagedCheckoutExecutionError::ReconciliationRequired
            | ManagedCheckoutExecutionError::Response => StatusCode::CONFLICT,
            ManagedCheckoutExecutionError::Store(e) => status(e),
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    Ok(Json(json!({"order_id":id,"checkout":result})))
}

#[utoipa::path(post,path="/v1/managed-service/orders/{id}/refund",params(("id"=Uuid,Path,description="Owned paid order ID")),request_body=Value,responses((status=200,description="Full pre-funding refund request durably reserved; only signed Stripe evidence reports sent"),(status=401,description="Website account and allowed Origin required"),(status=404,description="Order not owned by account"),(status=409,description="Order not eligible for this pre-funding refund"),(status=503,description="Durable order storage unavailable")))]
pub async fn request_refund(
    State(state): State<SharedState>,
    BuyerWrite(buyer): BuyerWrite,
    Path(id): Path<Uuid>,
    Json(_): Json<Empty>,
) -> Result<Json<Value>, StatusCode> {
    quota(&state, &buyer, "managed_refund").await?;
    let db = store(&state)?;
    let mut order = db
        .request_managed_refund(&buyer, id)
        .await
        .map_err(status)?;
    let mut execution = "deferred";
    if state.managed_payments.refunds_enabled
        && matches!(
            order.status,
            ManagedOrderStatus::RefundRequested
                | ManagedOrderStatus::RefundPending
                | ManagedOrderStatus::RefundActionRequired
        )
    {
        if let (Ok(p), Some(secret)) = (policy(&state), state.stripe_secret_key.as_deref()) {
            let merchant = RefundMerchant {
                stripe_account_id: p.stripe_account_id.clone(),
                livemode: p.livemode,
            };
            if let Ok(client) = ManagedRefundClient::new(secret, merchant) {
                match execute_managed_refund(db, &client, id, true).await {
                    Ok(_) => execution = "reconciling",
                    Err(error) => {
                        execution = "needs_attention";
                        eprintln!("managed refund request retained for recovery: order={id} error={error}");
                    }
                }
                order = db
                    .get_managed_service_order(&buyer, id)
                    .await
                    .map_err(status)?
                    .ok_or(StatusCode::NOT_FOUND)?;
            }
        }
    }
    Ok(Json(json!({"order":order,"refund_execution":execution})))
}

/// Existing production destination: authenticate bytes BEFORE inspecting the namespace.
pub async fn stripe_webhook(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, StatusCode> {
    if headers.get_all("stripe-signature").iter().count() > 1 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let event = crate::verified_stripe_webhook(&state, &headers, &body)?;
    if let (true, Ok(managed), Some(db)) = (
        managed_orders::stripe_dispute::is_dispute_event(&event.event_type),
        policy(&state),
        state.store.as_ref(),
    ) {
        let signature = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let dispute = managed_orders::stripe_dispute::verify_managed_dispute_event(
            &body,
            signature,
            state
                .stripe_webhook_secret
                .as_deref()
                .ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
            managed.livemode,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
        // A dispute on a payment that is not a managed order keeps the existing handling.
        if let Some(order_id) =
            db.record_managed_dispute(&dispute)
                .await
                .map_err(|error| match error {
                    ManagedOrderStoreError::Integrity => StatusCode::BAD_REQUEST,
                    other => status(other),
                })?
        {
            return Ok(Json(
                json!({"managed_order_id":order_id,"disposition":"dispute_recorded_funding_stopped"}),
            ));
        }
    }
    // Full refunds and review-deposit refunds reconcile through the same signed-event path.
    if [
        MANAGED_REFUND_PURPOSE,
        MANAGED_DEPOSIT_REFUND_PURPOSE,
        managed_orders::stripe_refund::MANAGED_RECOVERED_REFUND_PURPOSE,
    ]
    .iter()
    .any(|purpose| event.payload["metadata"]["purpose"] == *purpose)
    {
        let p = policy(&state)?;
        let signature = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let merchant = RefundMerchant {
            stripe_account_id: p.stripe_account_id.clone(),
            livemode: p.livemode,
        };
        let authenticated = verify_managed_refund_event(
            &body,
            signature,
            state
                .stripe_webhook_secret
                .as_deref()
                .ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
            &merchant,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
        let disposition = store(&state)?
            .reconcile_managed_refund_event(&authenticated)
            .await
            .map_err(status)?;
        return Ok(Json(
            json!({"managed_order_id":authenticated.data().order_id,"disposition":disposition}),
        ));
    }
    if event.payload["metadata"]["purpose"] == MANAGED_CHECKOUT_PURPOSE {
        let config = policy(&state)?;
        let signature = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let authenticated = verify_managed_checkout_event(
            &body,
            signature,
            state
                .stripe_webhook_secret
                .as_deref()
                .ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
            &config.stripe_account_id,
            config.livemode,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
        let disposition = store(&state)?
            .reconcile_managed_checkout_event(&authenticated)
            .await
            .map_err(status)?;
        return Ok(Json(
            json!({"managed_order_id":authenticated.data().order_id,"disposition":disposition}),
        ));
    }
    let result = crate::reconcile_stripe_checkout_webhook(State(state), headers, body).await?;
    Ok(Json(
        serde_json::to_value(result.0).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ))
}

#[cfg(test)]
mod tests;
