use anyhow::Context;
use app::{
    stripe_secret_key_mode_from_secret, AddFundingContributionRequest, AppResult,
    ApproveRiskBountyRequest, ApproveRiskPayoutRequest, BountyNetwork, BountyStatusResponse,
    ClaimBountyRequest, CreateFundingIntentRequest, CreateHelpRequestRequest, FundQuoteRequest,
    FundingIntentReport, LiveMoneyReadinessConfig, OpenPooledBountyRequest, PooledFundingReport,
    PostBountyRequest, QuoteSet, RegisterAgentRequest, RegisterCapabilityRequest,
    RejectRiskEventRequest, RequestQuotesRequest, ReviewedBountyApproval, SubmitResultRequest,
    VerifySubmissionRequest,
};
use chain_base::{base_network_descriptor, BaseNetworkDescriptor};
use chrono::Utc;
use db::{BountyStatusScope, PostgresStore};
use domain::{
    Agent, Bounty, Capability, EvalRun, FundingContribution, HelpRequest, Id, ProofRecord,
    ReputationEvent, RiskReviewRecord, Settlement, Submission, TemplateSignal, VerifierResult,
};
use eval_harness::{EvalSuiteResult, LoopSuiteResult};
use ledger::Ledger;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::sync::Mutex;
use uuid::Uuid;

pub const CANONICAL_BASE_MAINNET_BOUNTY_FACTORY: &str =
    "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
pub const CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION: &str =
    "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerAddressError {
    UnsupportedNetwork,
    NoncanonicalMainnet,
    MissingFactory,
    MissingImplementation,
}

pub fn autonomous_planner_addresses(
    chain_id: u64,
    configured_factory: Option<String>,
    configured_implementation: Option<String>,
) -> Result<(String, String), PlannerAddressError> {
    let configured = |value: Option<String>| {
        value
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
    };
    let factory = configured(configured_factory);
    let implementation = configured(configured_implementation);
    match chain_id {
        8_453
            if factory.as_deref().is_some_and(|address| {
                !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
            }) || implementation.as_deref().is_some_and(|address| {
                !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION)
            }) =>
        {
            Err(PlannerAddressError::NoncanonicalMainnet)
        }
        8_453 => Ok((
            CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_string(),
            CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_string(),
        )),
        84_532 => Ok((
            factory.ok_or(PlannerAddressError::MissingFactory)?,
            implementation.ok_or(PlannerAddressError::MissingImplementation)?,
        )),
        _ => Err(PlannerAddressError::UnsupportedNetwork),
    }
}

pub fn canonical_mainnet_factory(
    configured_factory: Option<String>,
    configured_implementation: Option<String>,
) -> Option<String> {
    if configured_factory
        .as_deref()
        .is_some_and(|address| !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY))
        || configured_implementation.as_deref().is_some_and(|address| {
            !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION)
        })
    {
        None
    } else {
        Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_string())
    }
}

pub fn autonomous_factory_for_chain(chain_id: u64) -> Option<String> {
    match chain_id {
        84_532 => env_nonempty("BASE_SEPOLIA_BOUNTY_FACTORY"),
        8_453 => canonical_mainnet_factory(
            env_nonempty("BASE_MAINNET_BOUNTY_FACTORY"),
            env_nonempty("BASE_MAINNET_BOUNTY_IMPLEMENTATION"),
        ),
        _ => None,
    }
}

pub fn base_usdc_token_for_chain(descriptor: &BaseNetworkDescriptor) -> Option<String> {
    let configured = match descriptor.chain_id {
        84_532 => env_nonempty("BASE_SEPOLIA_USDC_TOKEN"),
        8_453 => env_nonempty("BASE_MAINNET_USDC_TOKEN"),
        _ => None,
    };
    configured.or_else(|| Some(descriptor.native_usdc_token_address.clone()))
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .and_then(|value| non_empty(&value).map(str::to_string))
}

pub struct LiveMoneyRuntimeSettings<'a> {
    pub stripe_secret_key: Option<&'a str>,
    pub stripe_live_execution_enabled: bool,
    pub stripe_payment_method_configuration_configured: bool,
    pub stripe_webhook_secret_configured: bool,
    pub allow_unsigned_stripe_webhooks: bool,
    pub operator_auth_configured: bool,
    pub base_rpc_url_configured: bool,
    pub base_broadcast_enabled: bool,
}

pub fn live_money_readiness_config(
    network: &str,
    settings: LiveMoneyRuntimeSettings<'_>,
) -> LiveMoneyReadinessConfig {
    let descriptor = base_network_descriptor(network).ok();
    LiveMoneyReadinessConfig {
        network: network.to_string(),
        escrow_contract: descriptor
            .as_ref()
            .and_then(|value| autonomous_factory_for_chain(value.chain_id)),
        usdc_token: descriptor
            .as_ref()
            .and_then(base_usdc_token_for_chain)
            .or_else(|| descriptor.map(|value| value.native_usdc_token_address)),
        stripe_secret_key_mode: stripe_secret_key_mode_from_secret(settings.stripe_secret_key),
        stripe_live_execution_enabled: settings.stripe_live_execution_enabled,
        stripe_payment_method_configuration_configured: settings
            .stripe_payment_method_configuration_configured,
        stripe_webhook_secret_configured: settings.stripe_webhook_secret_configured,
        allow_unsigned_stripe_webhooks: settings.allow_unsigned_stripe_webhooks,
        operator_auth_configured: settings.operator_auth_configured,
        base_rpc_url_configured: settings.base_rpc_url_configured,
        base_broadcast_enabled: settings.base_broadcast_enabled,
    }
}

pub fn operator_token_is_authorized(
    expected: Option<&str>,
    direct_header: Option<&str>,
    authorization_header: Option<&str>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let provided = direct_header.and_then(non_empty).or_else(|| {
        authorization_header
            .and_then(|value| value.trim().strip_prefix("Bearer "))
            .and_then(non_empty)
    });
    provided.is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes()))
}

fn non_empty(value: &str) -> Option<&str> {
    match value.trim() {
        "" => None,
        value => Some(value),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

pub fn eval_run_from_suite(result: &EvalSuiteResult) -> EvalRun {
    EvalRun {
        id: Uuid::new_v4(),
        suite: result.suite.clone(),
        score: result.score,
        passed: result.passed,
        created_at: Utc::now(),
    }
}

pub fn eval_run_from_loop_suite(result: &LoopSuiteResult) -> EvalRun {
    let score = if result.loops.is_empty() {
        0.0
    } else {
        result
            .loops
            .iter()
            .map(|loop_result| {
                loop_result
                    .candidates
                    .iter()
                    .map(|candidate| candidate.score)
                    .fold(0.0_f32, f32::max)
            })
            .sum::<f32>()
            / result.loops.len() as f32
    };
    EvalRun {
        id: Uuid::new_v4(),
        suite: result.suite.clone(),
        score,
        passed: result.passed,
        created_at: Utc::now(),
    }
}

pub async fn record_eval_run(
    store: Option<&PostgresStore>,
    runs: &Mutex<Vec<EvalRun>>,
    run: EvalRun,
) -> anyhow::Result<()> {
    if let Some(store) = store {
        store.upsert_eval_run(&run).await?;
    }
    runs.lock().expect("state poisoned").insert(0, run);
    Ok(())
}

pub fn bounty_status_from_scope(scope: BountyStatusScope) -> AppResult<BountyStatusResponse> {
    let bounty_id = scope.bounty.id;
    BountyNetwork {
        bounties: [(bounty_id, scope.bounty)].into_iter().collect(),
        funding_intents: index_by_id(scope.funding_intents, |value| value.id),
        funding_contributions: index_by_id(scope.funding_contributions, |value| value.id),
        escrows: index_by_id(scope.escrows, |value| value.id),
        claims: index_by_id(scope.claims, |value| value.id),
        submissions: index_by_id(scope.submissions, |value| value.id),
        verifier_results: index_by_id(scope.verifier_results, |value| value.id),
        proofs: index_by_id(scope.proofs, |value| value.id),
        settlements: index_by_id(scope.settlements, |value| value.id),
        reputation_events: index_by_id(scope.reputation_events, |value| value.id),
        template_signals: index_by_id(scope.template_signals, |value| value.id),
        risk_events: index_by_id(scope.risk_events, |value| value.id),
        ..BountyNetwork::default()
    }
    .status(bounty_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BountyStatusLookupError {
    NotFound(String),
    Store(String),
}

impl BountyStatusLookupError {
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Store(_))
    }
}

impl fmt::Display for BountyStatusLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message) | Self::Store(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for BountyStatusLookupError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationError {
    Invalid(String),
    Store(String),
}

impl MutationError {
    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid(_))
    }
}

impl fmt::Display for MutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) | Self::Store(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for MutationError {}

fn invalid(error: impl ToString) -> MutationError {
    MutationError::Invalid(error.to_string())
}

fn store(error: impl ToString) -> MutationError {
    MutationError::Store(error.to_string())
}

async fn persist_risk_failure<T>(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    result: AppResult<T>,
) -> Result<T, MutationError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            persist_all_risk_events(store_ref, network)
                .await
                .map_err(store)?;
            Err(invalid(error))
        }
    }
}

pub async fn register_agent(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: RegisterAgentRequest,
) -> Result<Agent, MutationError> {
    let agent = network
        .lock()
        .expect("state poisoned")
        .register_agent(request);
    if let Some(store_ref) = store_ref {
        store_ref.upsert_agent(&agent).await.map_err(store)?;
    }
    Ok(agent)
}

pub async fn register_capability(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: RegisterCapabilityRequest,
) -> Result<Capability, MutationError> {
    let capability = network
        .lock()
        .expect("state poisoned")
        .register_capability(request)
        .map_err(invalid)?;
    if let Some(store_ref) = store_ref {
        store_ref
            .upsert_capability(&capability)
            .await
            .map_err(store)?;
    }
    Ok(capability)
}

pub async fn create_help_request(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: CreateHelpRequestRequest,
) -> Result<HelpRequest, MutationError> {
    let result = network
        .lock()
        .expect("state poisoned")
        .create_help_request(request);
    let help_request = persist_risk_failure(store_ref, network, result).await?;
    if let Some(store_ref) = store_ref {
        store_ref
            .upsert_help_request(&help_request)
            .await
            .map_err(store)?;
    }
    Ok(help_request)
}

async fn persist_quote_set(
    store_ref: Option<&PostgresStore>,
    quote_set: &QuoteSet,
    include_help_request: bool,
) -> Result<(), MutationError> {
    let Some(store_ref) = store_ref else {
        return Ok(());
    };
    if include_help_request {
        store_ref
            .upsert_help_request(&quote_set.help_request)
            .await
            .map_err(store)?;
    }
    for quote in &quote_set.quotes {
        store_ref.upsert_quote(quote).await.map_err(store)?;
    }
    Ok(())
}

pub async fn request_quotes(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: RequestQuotesRequest,
) -> Result<QuoteSet, MutationError> {
    let quote_set = network
        .lock()
        .expect("state poisoned")
        .request_quotes(request)
        .map_err(invalid)?;
    persist_quote_set(store_ref, &quote_set, false).await?;
    Ok(quote_set)
}

pub async fn create_help_and_request_quotes(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: CreateHelpRequestRequest,
) -> Result<QuoteSet, MutationError> {
    let result = {
        let mut network = network.lock().expect("state poisoned");
        network
            .create_help_request(request)
            .and_then(|help_request| {
                network.request_quotes(RequestQuotesRequest {
                    help_request_id: help_request.id,
                })
            })
    };
    let quote_set = persist_risk_failure(store_ref, network, result).await?;
    persist_quote_set(store_ref, &quote_set, true).await?;
    Ok(quote_set)
}

pub async fn fund_quote_as_bounty(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: FundQuoteRequest,
) -> Result<Bounty, MutationError> {
    let result = {
        let mut network = network.lock().expect("state poisoned");
        network
            .fund_quote_as_bounty(request)
            .map(|bounty| (bounty, network.ledger.entries().to_vec()))
    };
    let (bounty, entries) = persist_risk_failure(store_ref, network, result).await?;
    persist_bounty_and_ledger(store_ref, network, &bounty, &entries)
        .await
        .map_err(store)?;
    Ok(bounty)
}

pub async fn post_bounty(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: PostBountyRequest,
) -> Result<Bounty, MutationError> {
    let result = {
        let mut network = network.lock().expect("state poisoned");
        network
            .post_funded_bounty(request)
            .map(|bounty| (bounty, network.ledger.entries().to_vec()))
    };
    let (bounty, entries) = persist_risk_failure(store_ref, network, result).await?;
    persist_bounty_and_ledger(store_ref, network, &bounty, &entries)
        .await
        .map_err(store)?;
    Ok(bounty)
}

pub async fn open_pooled_bounty(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: OpenPooledBountyRequest,
) -> Result<Bounty, MutationError> {
    let result = network
        .lock()
        .expect("state poisoned")
        .open_pooled_bounty(request);
    let bounty = persist_risk_failure(store_ref, network, result).await?;
    persist_bounty_and_ledger(store_ref, network, &bounty, &[])
        .await
        .map_err(store)?;
    Ok(bounty)
}

pub async fn add_funding_contribution(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: AddFundingContributionRequest,
) -> Result<PooledFundingReport, MutationError> {
    let report = network
        .lock()
        .expect("state poisoned")
        .add_funding_contribution(request)
        .map_err(invalid)?;
    persist_pooled_funding_report(store_ref, &report)
        .await
        .map_err(store)?;
    Ok(report)
}

pub async fn create_funding_intent(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: CreateFundingIntentRequest,
    platform_base_url: String,
) -> Result<FundingIntentReport, MutationError> {
    let report = network
        .lock()
        .expect("state poisoned")
        .create_funding_intent(request, platform_base_url)
        .map_err(invalid)?;
    persist_funding_intent_report(store_ref, &report)
        .await
        .map_err(store)?;
    Ok(report)
}

pub async fn claim_bounty(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: ClaimBountyRequest,
) -> Result<Bounty, MutationError> {
    let (bounty, claim) = {
        let mut network = network.lock().expect("state poisoned");
        let bounty = network.claim_bounty(request).map_err(invalid)?;
        let claim = network
            .claims
            .values()
            .find(|claim| claim.bounty_id == bounty.id)
            .expect("claim exists after successful claim")
            .clone();
        (bounty, claim)
    };
    if let Some(store_ref) = store_ref {
        store_ref.upsert_bounty(&bounty).await.map_err(store)?;
        store_ref.upsert_claim(&claim).await.map_err(store)?;
    }
    Ok(bounty)
}

pub async fn submit_result(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: SubmitResultRequest,
) -> Result<Submission, MutationError> {
    let result = {
        let mut network = network.lock().expect("state poisoned");
        network.submit_result(request).map(|submission| {
            let bounty = network
                .bounties
                .get(&submission.bounty_id)
                .expect("submission bounty exists")
                .clone();
            (submission, bounty)
        })
    };
    let (submission, bounty) = persist_risk_failure(store_ref, network, result).await?;
    if let Some(store_ref) = store_ref {
        store_ref.upsert_bounty(&bounty).await.map_err(store)?;
        store_ref
            .upsert_submission(&submission)
            .await
            .map_err(store)?;
    }
    Ok(submission)
}

pub async fn approve_risk_bounty(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: ApproveRiskBountyRequest,
) -> Result<ReviewedBountyApproval, MutationError> {
    let (approval, entries) = {
        let mut network = network.lock().expect("state poisoned");
        network
            .approve_risk_bounty(request)
            .map(|approval| (approval, network.ledger.entries().to_vec()))
            .map_err(invalid)?
    };
    persist_reviewed_bounty_approval(store_ref, &approval, &entries)
        .await
        .map_err(store)?;
    Ok(approval)
}

pub async fn approve_risk_payout(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: ApproveRiskPayoutRequest,
) -> Result<RiskReviewRecord, MutationError> {
    let review = network
        .lock()
        .expect("state poisoned")
        .approve_risk_payout(request)
        .map_err(invalid)?;
    persist_risk_review(store_ref, &review)
        .await
        .map_err(store)?;
    Ok(review)
}

pub async fn reject_risk_event(
    store_ref: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    request: RejectRiskEventRequest,
) -> Result<RiskReviewRecord, MutationError> {
    let review = network
        .lock()
        .expect("state poisoned")
        .reject_risk_event(request)
        .map_err(invalid)?;
    persist_risk_review(store_ref, &review)
        .await
        .map_err(store)?;
    Ok(review)
}

pub fn bounty_status_from_network(
    network: &Mutex<BountyNetwork>,
    bounty_id: Uuid,
) -> Result<BountyStatusResponse, BountyStatusLookupError> {
    network
        .lock()
        .expect("state poisoned")
        .status(bounty_id)
        .map_err(|error| BountyStatusLookupError::NotFound(error.to_string()))
}

pub async fn bounty_status(
    store: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    bounty_id: Uuid,
) -> Result<BountyStatusResponse, BountyStatusLookupError> {
    if let Some(store) = store {
        let scope = store
            .load_bounty_status_scope(bounty_id)
            .await
            .map_err(|error| BountyStatusLookupError::Store(error.to_string()))?
            .ok_or_else(|| BountyStatusLookupError::NotFound("bounty not found".to_string()))?;
        return bounty_status_from_scope(scope)
            .map_err(|error| BountyStatusLookupError::NotFound(error.to_string()));
    }
    bounty_status_from_network(network, bounty_id)
}

pub struct VerificationOutcome {
    pub proof: ProofRecord,
    bounty: Bounty,
    verifier_result: VerifierResult,
    settlements: Vec<Settlement>,
    funding_contributions: Vec<FundingContribution>,
    reputation_events: Vec<ReputationEvent>,
    template_signals: Vec<TemplateSignal>,
    ledger_entries: Vec<ledger::LedgerEntry>,
}

pub async fn execute_verification(
    mut network: BountyNetwork,
    request: VerifySubmissionRequest,
) -> (BountyNetwork, AppResult<VerificationOutcome>) {
    let result = network.verify_submission(request).await.map(|proof| {
        let bounty = network
            .bounties
            .get(&proof.bounty_id)
            .expect("proof bounty exists")
            .clone();
        let verifier_result = network
            .verifier_results
            .get(&proof.verifier_result_id)
            .expect("proof verifier result exists")
            .clone();
        let for_bounty = |id| id == proof.bounty_id;
        VerificationOutcome {
            settlements: network
                .settlements
                .values()
                .filter(|value| for_bounty(value.bounty_id))
                .cloned()
                .collect(),
            funding_contributions: network
                .funding_contributions
                .values()
                .filter(|value| for_bounty(value.bounty_id))
                .cloned()
                .collect(),
            reputation_events: network
                .reputation_events
                .values()
                .filter(|value| for_bounty(value.bounty_id))
                .cloned()
                .collect(),
            template_signals: network
                .template_signals
                .values()
                .filter(|value| for_bounty(value.bounty_id))
                .cloned()
                .collect(),
            ledger_entries: network.ledger.entries().to_vec(),
            proof,
            bounty,
            verifier_result,
        }
    });
    (network, result)
}

pub async fn persist_verification(
    store: Option<&PostgresStore>,
    outcome: &VerificationOutcome,
) -> anyhow::Result<()> {
    let Some(store) = store else {
        return Ok(());
    };
    store.upsert_bounty(&outcome.bounty).await?;
    store
        .upsert_verifier_result(&outcome.verifier_result)
        .await?;
    store.upsert_proof_record(&outcome.proof).await?;
    for settlement in &outcome.settlements {
        store.upsert_settlement(settlement).await?;
    }
    for contribution in &outcome.funding_contributions {
        store.upsert_funding_contribution(contribution).await?;
    }
    for event in &outcome.reputation_events {
        store.upsert_reputation_event(event).await?;
    }
    for signal in &outcome.template_signals {
        store.upsert_template_signal(signal).await?;
    }
    persist_ledger_entries(store, &outcome.ledger_entries).await?;
    Ok(())
}

pub async fn persist_bounty_and_ledger(
    store: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
    bounty: &Bounty,
    ledger_entries: &[ledger::LedgerEntry],
) -> anyhow::Result<()> {
    let Some(store) = store else {
        return Ok(());
    };
    store.upsert_bounty(bounty).await?;
    let contributions = network
        .lock()
        .expect("state poisoned")
        .funding_contributions
        .values()
        .filter(|contribution| contribution.bounty_id == bounty.id)
        .cloned()
        .collect::<Vec<_>>();
    for contribution in &contributions {
        store.upsert_funding_contribution(contribution).await?;
    }
    persist_ledger_entries(store, ledger_entries).await
}

pub async fn persist_reviewed_bounty_approval(
    store: Option<&PostgresStore>,
    approval: &ReviewedBountyApproval,
    ledger_entries: &[ledger::LedgerEntry],
) -> anyhow::Result<()> {
    let Some(store) = store else {
        return Ok(());
    };
    store.upsert_bounty(&approval.bounty).await?;
    store.upsert_risk_review(&approval.review).await?;
    persist_ledger_entries(store, ledger_entries).await
}

pub async fn persist_pooled_funding_report(
    store: Option<&PostgresStore>,
    report: &PooledFundingReport,
) -> anyhow::Result<()> {
    let Some(store) = store else {
        return Ok(());
    };
    store.upsert_bounty(&report.bounty).await?;
    store
        .upsert_funding_contribution(&report.contribution)
        .await?;
    persist_ledger_entries(store, &report.ledger_entries).await
}

pub async fn persist_funding_intent_report(
    store: Option<&PostgresStore>,
    report: &FundingIntentReport,
) -> anyhow::Result<()> {
    let Some(store) = store else {
        return Ok(());
    };
    store.upsert_bounty(&report.bounty).await?;
    store.upsert_funding_intent(&report.intent).await?;
    Ok(())
}

pub async fn persist_risk_review(
    store: Option<&PostgresStore>,
    review: &RiskReviewRecord,
) -> anyhow::Result<()> {
    if let Some(store) = store {
        store.upsert_risk_review(review).await?;
    }
    Ok(())
}

pub async fn persist_ledger_entries(
    store: &PostgresStore,
    entries: &[ledger::LedgerEntry],
) -> anyhow::Result<()> {
    for entry in entries {
        store.insert_ledger_entry(entry).await?;
    }
    Ok(())
}

pub async fn persist_all_risk_events(
    store: Option<&PostgresStore>,
    network: &Mutex<BountyNetwork>,
) -> anyhow::Result<()> {
    let Some(store) = store else {
        return Ok(());
    };
    let events = network
        .lock()
        .expect("state poisoned")
        .risk_events
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for event in &events {
        store.upsert_risk_event(event).await?;
    }
    Ok(())
}

fn index_by_id<T>(values: Vec<T>, id: impl Fn(&T) -> Id) -> HashMap<Id, T> {
    values
        .into_iter()
        .map(|value| (id(&value), value))
        .collect()
}

pub async fn hydrate_bounty_network(store: &PostgresStore) -> anyhow::Result<BountyNetwork> {
    hydrate(store, None).await
}

pub async fn hydrate_bounty_network_with_ledger_context(
    store: &PostgresStore,
    context: &'static str,
) -> anyhow::Result<BountyNetwork> {
    hydrate(store, Some(context)).await
}

async fn hydrate(
    store: &PostgresStore,
    ledger_context: Option<&'static str>,
) -> anyhow::Result<BountyNetwork> {
    Ok(BountyNetwork {
        agents: index_by_id(store.list_agents().await?, |value| value.id),
        contributor_contacts: index_by_id(store.list_contributor_contacts().await?, |value| {
            value.id
        }),
        audience_members: index_by_id(store.list_audience_members().await?, |value| value.id),
        audience_interactions: index_by_id(store.list_audience_interactions().await?, |value| {
            value.id
        }),
        discovery_responses: index_by_id(store.list_discovery_responses().await?, |value| value.id),
        outreach_attempts: index_by_id(store.list_outreach_attempts().await?, |value| value.id),
        capabilities: index_by_id(store.list_capabilities().await?, |value| value.id),
        help_requests: index_by_id(store.list_help_requests().await?, |value| value.id),
        quotes: index_by_id(store.list_quotes().await?, |value| value.id),
        bounties: index_by_id(store.list_bounties().await?, |value| value.id),
        objectives: index_by_id(store.list_objectives().await?, |value| value.id),
        funding_intents: index_by_id(store.list_funding_intents().await?, |value| value.id),
        funding_contributions: index_by_id(store.list_funding_contributions().await?, |value| {
            value.id
        }),
        escrows: index_by_id(store.list_escrows().await?, |value| value.id),
        claims: index_by_id(store.list_claims().await?, |value| value.id),
        submissions: index_by_id(store.list_submissions().await?, |value| value.id),
        verifier_results: index_by_id(store.list_verifier_results().await?, |value| value.id),
        proofs: index_by_id(store.list_proof_records().await?, |value| value.id),
        settlements: index_by_id(store.list_settlements().await?, |value| value.id),
        reputation_events: index_by_id(store.list_reputation_events().await?, |value| value.id),
        template_signals: index_by_id(store.list_template_signals().await?, |value| value.id),
        risk_events: index_by_id(store.list_risk_events().await?, |value| value.id),
        risk_reviews: index_by_id(store.list_risk_reviews().await?, |value| value.id),
        payment_events: index_by_id(store.list_payment_events().await?, |value| value.id),
        ledger: match ledger_context {
            Some(context) => {
                Ledger::from_entries(store.list_ledger_entries().await?).context(context)?
            }
            None => Ledger::from_entries(store.list_ledger_entries().await?)?,
        },
        ..BountyNetwork::default()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        autonomous_planner_addresses, bounty_status_from_network, operator_token_is_authorized,
        BountyStatusLookupError, PlannerAddressError, CANONICAL_BASE_MAINNET_BOUNTY_FACTORY,
        CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION,
    };
    use app::{BountyNetwork, PostBountyRequest};
    use domain::{FundingMode, PrivacyLevel};
    use std::sync::Mutex;
    use uuid::Uuid;

    #[test]
    fn operator_auth_preserves_direct_and_bearer_rules() {
        let cases = [
            ("disabled", None, None, None, true),
            ("direct", Some("secret"), Some(" secret "), None, true),
            (
                "bearer",
                Some("secret"),
                None,
                Some(" Bearer secret "),
                true,
            ),
            (
                "wrong direct wins",
                Some("secret"),
                Some("wrong"),
                Some("Bearer secret"),
                false,
            ),
            (
                "case sensitive",
                Some("secret"),
                None,
                Some("bearer secret"),
                false,
            ),
        ];
        for (name, expected, direct, bearer, authorized) in cases {
            assert_eq!(
                operator_token_is_authorized(expected, direct, bearer),
                authorized,
                "{name}"
            );
        }
    }

    #[test]
    fn planner_addresses_preserve_canonical_mainnet_and_explicit_sepolia() {
        assert_eq!(
            autonomous_planner_addresses(8_453, None, None).unwrap(),
            (
                CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_string(),
                CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_string()
            )
        );
        assert_eq!(
            autonomous_planner_addresses(84_532, Some(" factory ".into()), Some("impl".into()))
                .unwrap(),
            ("factory".into(), "impl".into())
        );
        assert_eq!(
            autonomous_planner_addresses(8_453, Some("wrong".into()), None),
            Err(PlannerAddressError::NoncanonicalMainnet)
        );
    }

    #[test]
    fn bounty_status_lookup_preserves_success_and_retry_classification() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Parity fixture".to_string(),
                template_slug: "small-code-change".to_string(),
                amount_minor: 100,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let network = Mutex::new(network);

        assert_eq!(
            bounty_status_from_network(&network, bounty.id)
                .unwrap()
                .bounty
                .id,
            bounty.id
        );
        let missing = bounty_status_from_network(&network, Uuid::nil()).unwrap_err();
        assert!(matches!(missing, BountyStatusLookupError::NotFound(_)));
        assert!(!missing.retryable());
        assert!(BountyStatusLookupError::Store("temporarily unavailable".to_string()).retryable());
    }
}
