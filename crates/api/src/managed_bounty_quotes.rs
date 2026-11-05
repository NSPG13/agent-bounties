//! Card-paid bounty quotes. The server composes the public autonomous-v2 terms with the funding
//! wallet as creator, prices them in fiat and binds the exact funding-wallet call. Nothing here
//! charges, funds or settles; acceptance, Checkout and signed payment evidence follow unchanged.
use crate::{
    managed_payments::{self, BuyerWrite},
    SharedState,
};
use axum::{extract::State, http::StatusCode, Json};
use chain_base::{
    autonomous_v2_create_from_terms, autonomous_v2_platform_fee,
    build_autonomous_bounty_terms_record, customer_review_verifiers,
    encode_bounded_v2_funding_wallet_create_call, normalize_evm_address,
    validate_autonomous_v2_creation_for_public_earning, AutonomousBountyTxPlanner,
    AutonomousV2FactoryFee, CUSTOMER_REVIEW_ENGINE,
};
use chrono::{DateTime, Utc};
use domain::AutonomousBountyTermsDocument;
use managed_orders::{
    pricing::{price_basis_from_decimal, priced_additional_costs, PricingPolicy},
    stripe_checkout::ManagedCheckoutPlan,
    BountyCommitment, FiatCurrency, ManagedQuote, QuoteInput, ServiceTerms, PLATFORM_FEE_BPS,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha3::{Digest, Keccak256};
use std::time::Duration;
use uuid::Uuid;

const NONCE_DOMAIN: &[u8] = b"agent-bounties/managed-card-bounty/v1\0";
const BITSO_TICKER: &str = "https://api.bitso.com/v3/ticker?book=usd_mxn";
const DRAFT_KEYS: [&str; 7] = [
    "title",
    "goal",
    "acceptance_criteria",
    "benchmark",
    "evidence_schema",
    "source_url",
    "image",
];

/// Merchant configuration for card-paid bounties (`MANAGED_BOUNTY_QUOTES`, JSON).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedBountyConfig {
    pub network: String,
    pub funding_wallet: String,
    pub customer_review: CustomerReviewSetup,
    pub max_funding_window_seconds: u64,
    pub quote_lifetime_seconds: i64,
    pub minimum_open_seconds: i64,
    pub terms_version: String,
    pub remedies: String,
    pub pricing: PricingPolicy,
    pub rate_source: RateSource,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomerReviewSetup {
    pub verifier: String,
    pub verifier_reward_micro_usdc: u64,
    pub claim_window_seconds: u64,
    pub verification_window_seconds: u64,
    pub public_disclosure: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RateSource {
    /// Bitso's public `usd_mxn` ask. MXN quotes only.
    BitsoUsdMxnAsk,
    /// A fixed decimal fiat-per-USD price, accepted only for Base Sepolia rehearsals.
    Fixed { price: String },
}

impl ManagedBountyConfig {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Ok(raw) = std::env::var("MANAGED_BOUNTY_QUOTES") else {
            return Ok(None);
        };
        let mut config: Self = serde_json::from_str(&raw)
            .map_err(|_| anyhow::anyhow!("MANAGED_BOUNTY_QUOTES is invalid"))?;
        config.validate()?;
        config.funding_wallet = normalize_evm_address(&config.funding_wallet)?;
        config.customer_review.verifier = normalize_evm_address(&config.customer_review.verifier)?;
        Ok(Some(config))
    }

    fn validate(&self) -> anyhow::Result<()> {
        let review = &self.customer_review;
        anyhow::ensure!(
            matches!(self.network.as_str(), "base-mainnet" | "base-sepolia"),
            "managed bounty network must be base-mainnet or base-sepolia"
        );
        anyhow::ensure!(
            !matches!(self.rate_source, RateSource::Fixed { .. }) || self.network == "base-sepolia",
            "a fixed reference rate is only allowed for Base Sepolia rehearsals"
        );
        anyhow::ensure!(
            normalize_evm_address(&self.funding_wallet)?
                != normalize_evm_address(&review.verifier)?,
            "the customer-review signer must not be the funding wallet"
        );
        anyhow::ensure!(
            (1_860..=86_400).contains(&self.quote_lifetime_seconds)
                && self.minimum_open_seconds >= 3_600
                && self.max_funding_window_seconds <= 366 * 86_400,
            "managed bounty windows are out of bounds"
        );
        anyhow::ensure!(
            review.verifier_reward_micro_usdc >= 10_000
                && (1..=30 * 86_400).contains(&review.claim_window_seconds)
                && (managed_orders::review::MIN_VERIFICATION_WINDOW_SECONDS..=30 * 86_400)
                    .contains(&review.verification_window_seconds)
                && !review.public_disclosure.trim().is_empty(),
            "customer-review setup is invalid"
        );
        anyhow::ensure!(
            managed_orders::valid_key(&self.terms_version) && !self.remedies.trim().is_empty(),
            "managed bounty terms are invalid"
        );
        self.pricing
            .validate()
            .map_err(|_| anyhow::anyhow!("managed bounty pricing is invalid"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BountyQuoteRequest {
    /// The posting draft: title, goal, acceptance criteria, customer-review benchmark with its
    /// `delivery_deadline`, evidence schema, optional source URL and image. The server owns
    /// `contract_terms` and `verification_policy`.
    pub document: Map<String, Value>,
    pub solver_reward_micro_usdc: u64,
}

/// Derives the bounty creation nonce from the quote id, so one quote can fund one bounty.
pub fn creation_nonce(quote_id: Uuid) -> String {
    let mut hash = Keccak256::new();
    hash.update(NONCE_DOMAIN);
    hash.update(quote_id.as_bytes());
    format!("0x{}", hex::encode(hash.finalize()))
}

pub fn call_keccak256(calldata: &str) -> Result<String, StatusCode> {
    let bytes =
        hex::decode(calldata.trim_start_matches("0x")).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

#[utoipa::path(post,path="/v1/managed-service/bounty-quotes",request_body=Value,responses((status=200,description="Immutable itemized card quote bound to one exact autonomous-v2 bounty; no payment or funding"),(status=400,description="Draft, reward or deadline rejected"),(status=401,description="Website account login and allowed Origin required"),(status=422,description="The bounty exceeds the funding wallet allowance or the total quote exceeds the configured USD order limit"),(status=503,description="Card intake, signer, verifier, v2 factory, reference rate or funding wallet unavailable, revoked or expiring")))]
pub async fn issue_bounty_quote(
    State(state): State<SharedState>,
    BuyerWrite(buyer): BuyerWrite,
    Json(request): Json<BountyQuoteRequest>,
) -> Result<Json<Value>, StatusCode> {
    let config = state
        .managed_payments
        .bounty
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !state.managed_payments.intake_allowed(&state, &buyer) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let checkout_policy = managed_payments::policy(&state)?.clone();
    let network = if checkout_policy.livemode {
        "base-mainnet"
    } else {
        "base-sepolia"
    };
    if config.network != network
        || !customer_review_verifiers(network)
            .iter()
            .any(|pinned| pinned.eq_ignore_ascii_case(&config.customer_review.verifier))
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    managed_payments::quota(&state, &buyer, "managed_bounty_quote").await?;
    let now = Utc::now();
    let price = reference_price(config, checkout_policy.currency).await?;
    let (planner, deployment) = crate::configured_autonomous_v2_planner(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let prepared = prepare(
        config,
        &buyer,
        &checkout_policy,
        request,
        now,
        &price,
        (&planner, &deployment.fee),
    )?;
    if !state.managed_payments.price_allowed(&prepared.quote) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    wallet_caps(&state, network, &config.funding_wallet)
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .admit(
            prepared.target_micro_usdc,
            prepared.quote.input().expires_at,
        )?;
    let store = managed_payments::store(&state)?;
    store
        .upsert_autonomous_bounty_terms(&prepared.terms)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .put_managed_service_quote(&prepared.quote)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({
        "quote": prepared.quote,
        "quote_digest": prepared.quote.digest().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        "terms_hash": prepared.terms.terms_hash,
        "predicted_bounty_contract": prepared.quote.input().bounty.predicted_bounty,
        "funding_call": {"to": config.funding_wallet, "data": prepared.calldata},
    })))
}

pub struct PreparedBountyQuote {
    pub quote: ManagedQuote,
    pub terms: domain::AutonomousBountyTermsRecord,
    pub calldata: String,
    /// What the funding wallet spends: solver reward, verifier reward and platform fee.
    pub target_micro_usdc: u64,
}

/// The funding wallet's owner-set limits at the latest block, read from `policy()`, `revoked()`
/// and `lifetimeSpent()`. The owner changes them on chain, so they are never configured here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletCaps {
    pub revoked: bool,
    pub valid_until: u64,
    pub max_per_bounty: u128,
    pub lifetime_room: u128,
}

impl WalletCaps {
    /// Decodes the three call results; anything malformed is unavailable, never permissive.
    pub fn decode(policy: &str, revoked: &str, lifetime_spent: &str) -> Option<Self> {
        let words = |hex: &str| {
            let bytes = hex::decode(hex.strip_prefix("0x")?).ok()?;
            (!bytes.is_empty() && bytes.len() % 32 == 0).then(|| {
                bytes
                    .chunks(32)
                    .map(alloy::primitives::U256::from_be_slice)
                    .collect::<Vec<_>>()
            })
        };
        let small = |word: alloy::primitives::U256| u128::try_from(word).unwrap_or(u128::MAX);
        // Policy is a static struct of 13 words; caps are words 8 to 10, validUntil word 2.
        let policy = words(policy).filter(|words| words.len() == 13)?;
        let revoked = words(revoked).filter(|words| words.len() == 1)?[0];
        let spent = words(lifetime_spent).filter(|words| words.len() == 1)?[0];
        if revoked > alloy::primitives::U256::from(1u8) || spent > policy[10] {
            return None;
        }
        Some(Self {
            revoked: revoked == alloy::primitives::U256::from(1u8),
            valid_until: u64::try_from(policy[2]).ok()?,
            max_per_bounty: small(policy[8]),
            lifetime_room: small(policy[10] - spent),
        })
    }

    /// A card is charged only for a bounty the wallet may still fund after the quote expires.
    /// The daily cap is not checked here: a paid order waits for the next day's allowance.
    pub fn admit(&self, target_micro_usdc: u64, quote_expires_at: i64) -> Result<(), StatusCode> {
        if self.revoked || i64::try_from(self.valid_until).unwrap_or(i64::MAX) <= quote_expires_at {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let target = u128::from(target_micro_usdc);
        if target > self.max_per_bounty || target > self.lifetime_room {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        Ok(())
    }
}

fn selector(signature: &str) -> String {
    format!(
        "0x{}",
        hex::encode(&Keccak256::digest(signature.as_bytes())[..4])
    )
}

async fn wallet_caps(state: &SharedState, network: &str, wallet: &str) -> Option<WalletCaps> {
    let (_, rpc) = state.base_rpc_urls.resolve(network).ok()?;
    let transport = chain_base::ReqwestJsonRpcTransport::default();
    let mut results = Vec::with_capacity(3);
    for signature in ["policy()", "revoked()", "lifetimeSpent()"] {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "eth_call",
            "params": [{"to": wallet, "data": selector(signature)}, "latest"]});
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            chain_base::JsonRpcTransport::post_json_value(&transport, &rpc, &request),
        )
        .await
        .ok()?
        .ok()?;
        results.push(response.get("result")?.as_str()?.to_string());
    }
    WalletCaps::decode(&results[0], &results[1], &results[2])
}

/// Pure composition: terms, creation, binding and price. Persistence is the caller's job.
pub fn prepare(
    config: &ManagedBountyConfig,
    buyer: &str,
    checkout_policy: &managed_orders::stripe_checkout::CheckoutPolicy,
    request: BountyQuoteRequest,
    now: DateTime<Utc>,
    price: &ReferencePrice,
    (planner, fee): (&AutonomousBountyTxPlanner, &AutonomousV2FactoryFee),
) -> Result<PreparedBountyQuote, StatusCode> {
    let network = config.network.as_str();
    if fee.platform_fee_bps != PLATFORM_FEE_BPS {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let issued_at = now.timestamp();
    let expires_at = issued_at + config.quote_lifetime_seconds;
    let draft = request.document;
    if draft.keys().any(|key| !DRAFT_KEYS.contains(&key.as_str())) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let benchmark = draft.get("benchmark").ok_or(StatusCode::BAD_REQUEST)?;
    let deadline = benchmark["delivery_deadline"]
        .as_i64()
        .filter(|deadline| {
            benchmark["engine"] == CUSTOMER_REVIEW_ENGINE
                && *deadline >= expires_at + config.minimum_open_seconds
                && *deadline <= issued_at + config.max_funding_window_seconds as i64
        })
        .ok_or(StatusCode::BAD_REQUEST)?;
    let review = &config.customer_review;
    let solver_reward = request.solver_reward_micro_usdc;
    let platform_fee = u64::try_from(
        autonomous_v2_platform_fee(u128::from(solver_reward), PLATFORM_FEE_BPS)
            .map_err(|_| StatusCode::BAD_REQUEST)?,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let target = solver_reward
        .checked_add(review.verifier_reward_micro_usdc)
        .and_then(|sum| sum.checked_add(platform_fee))
        .filter(|_| solver_reward > 0)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let quote_id = Uuid::new_v4();
    let nonce = creation_nonce(quote_id);
    let usdc = |amount: u64| json!({"amount": amount, "currency": "usdc"});
    let mut document = draft.clone();
    document.insert("schema_version".into(), json!("agent-bounties/terms-v1"));
    document.insert(
        "benchmark".into(),
        json!({"engine": CUSTOMER_REVIEW_ENGINE, "reviewer": "paying_customer",
               "acceptance": "all_published_criteria", "delivery_deadline": deadline}),
    );
    document.insert(
        "verification_policy".into(),
        json!({"engine": CUSTOMER_REVIEW_ENGINE, "mechanism": "signed_quorum", "threshold": 1,
               "verifiers": [review.verifier], "public_disclosure": review.public_disclosure}),
    );
    document.insert(
        "contract_terms".into(),
        json!({
            "protocol_version": chain_base::AUTONOMOUS_V2_PROTOCOL_VERSION,
            "network": network,
            "settlement_token": chain_base::base_network_descriptor(network)
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
                .native_usdc_token_address,
            "creator_wallet": config.funding_wallet,
            "solver_reward": usdc(solver_reward),
            "verifier_reward": usdc(review.verifier_reward_micro_usdc),
            "claim_bond": usdc(review.verifier_reward_micro_usdc),
            "initial_funding": usdc(target),
            "platform_fee_bps": PLATFORM_FEE_BPS,
            "platform_fee": usdc(platform_fee),
            "platform_fee_recipient": fee.platform_fee_recipient,
            "funding_deadline": deadline,
            "claim_window_seconds": review.claim_window_seconds,
            "verification_window_seconds": review.verification_window_seconds,
            "creation_nonce": nonce,
        }),
    );
    document.entry("source_url").or_insert(Value::Null);
    document.insert("discovery_source".into(), json!("managed_card_quote"));
    let document: AutonomousBountyTermsDocument =
        serde_json::from_value(Value::Object(document)).map_err(|_| StatusCode::BAD_REQUEST)?;
    let terms = build_autonomous_bounty_terms_record(&config.funding_wallet, document, now)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let create =
        autonomous_v2_create_from_terms(&terms, fee).map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_autonomous_v2_creation_for_public_earning(network, &create, &terms, fee)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let plan = planner
        .plan_v2_creation(network, &create, fee)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let calldata = encode_bounded_v2_funding_wallet_create_call(&create)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let criteria = terms
        .document
        .acceptance_criteria
        .iter()
        .map(|criterion| format!("- {criterion}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut input = QuoteInput {
        quote_id,
        buyer_principal: buyer.to_string(),
        stripe_account_id: checkout_policy.stripe_account_id.clone(),
        livemode: checkout_policy.livemode,
        issued_at,
        expires_at,
        terms: ServiceTerms {
            deliverable: bounded_text(
                format!("{}\n\n{}", terms.document.title, terms.document.goal),
                &terms.document.title,
                &terms.terms_hash,
            ),
            acceptance_criteria: bounded_text(criteria, "Acceptance criteria", &terms.terms_hash),
            delivery_deadline: deadline,
            terms_version: config.terms_version.clone(),
            remedies: config.remedies.clone(),
        },
        currency: checkout_policy.currency,
        price_basis: price_basis_from_decimal(&price.price, &price.reference, expires_at)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        solver_reward_micro_usdc: solver_reward,
        verifier_reserve_micro_usdc: review.verifier_reward_micro_usdc,
        additional_costs: Vec::new(),
        bounty: BountyCommitment {
            network: network.to_string(),
            factory: plan.plan.factory_contract.to_ascii_lowercase(),
            funding_wallet: config.funding_wallet.clone(),
            creation_nonce: nonce,
            terms_hash: terms.terms_hash.to_ascii_lowercase(),
            verification_setup: CUSTOMER_REVIEW_ENGINE.to_string(),
            funding_deadline: deadline,
            predicted_bounty: plan.plan.predicted_bounty_contract.to_ascii_lowercase(),
            create_call_keccak256: call_keccak256(&calldata)?,
        },
    };
    let base_total = ManagedQuote::new(input.clone())
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .total_minor();
    let deposit = managed_orders::fiat_amount(
        managed_orders::review::review_deposit_micro_usdc(solver_reward),
        &input.price_basis,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    input.additional_costs = priced_additional_costs(base_total, deposit, &config.pricing)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let quote = ManagedQuote::new(input).map_err(|_| StatusCode::BAD_REQUEST)?;
    // The merchant policy still bounds account, mode, currency, amount and Checkout lifetime.
    ManagedCheckoutPlan::new(
        &quote,
        Uuid::new_v4(),
        Uuid::new_v4(),
        checkout_policy.clone(),
        issued_at,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(PreparedBountyQuote {
        quote,
        terms,
        calldata,
        target_micro_usdc: target,
    })
}

/// Quote text is limited to 16 KiB; longer public terms are referenced by their hash instead.
fn bounded_text(text: String, heading: &str, terms_hash: &str) -> String {
    const LIMIT: usize = 16_000;
    if text.len() <= LIMIT {
        return text;
    }
    format!("{heading}\n\nThe complete text is in the published bounty terms {terms_hash}.")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencePrice {
    pub price: String,
    pub reference: String,
}

async fn reference_price(
    config: &ManagedBountyConfig,
    currency: FiatCurrency,
) -> Result<ReferencePrice, StatusCode> {
    match (&config.rate_source, currency) {
        (_, FiatCurrency::Usd) => Ok(ReferencePrice {
            price: "1".into(),
            reference: "usdc-usd-par".into(),
        }),
        (RateSource::Fixed { price }, FiatCurrency::Mxn) => Ok(ReferencePrice {
            price: price.clone(),
            reference: "fixed-rehearsal-rate".into(),
        }),
        (RateSource::BitsoUsdMxnAsk, FiatCurrency::Mxn) => bitso_usd_mxn_ask().await,
    }
}

/// Bitso's public ticker over a fixed origin, bounded in time and size. Any malformed or
/// unexpected reply fails closed; no quote is issued without a current reference price.
async fn bitso_usd_mxn_ask() -> Result<ReferencePrice, StatusCode> {
    let unavailable = |_| StatusCode::SERVICE_UNAVAILABLE;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(unavailable)?;
    let response = client.get(BITSO_TICKER).send().await.map_err(unavailable)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > 16 * 1024)
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let body = response.bytes().await.map_err(unavailable)?;
    if body.len() > 16 * 1024 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    parse_bitso_ticker(&body, Utc::now()).ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

/// Accepts only a successful `usd_mxn` ticker created within the last five minutes.
pub fn parse_bitso_ticker(body: &[u8], now: DateTime<Utc>) -> Option<ReferencePrice> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let payload = value.get("payload").filter(|_| value["success"] == true)?;
    let created = DateTime::parse_from_rfc3339(payload["created_at"].as_str()?).ok()?;
    let age = now.signed_duration_since(created.with_timezone(&Utc));
    if payload["book"] != "usd_mxn" || age.num_seconds() > 300 || age.num_seconds() < -60 {
        return None;
    }
    let ask = payload["ask"].as_str()?;
    price_basis_from_decimal(ask, "bitso", 1).ok()?;
    Some(ReferencePrice {
        price: ask.to_string(),
        reference: format!("bitso:usd_mxn:ask:{}", created.timestamp()),
    })
}

#[cfg(test)]
mod tests;
