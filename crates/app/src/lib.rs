use bounty_router::{template_for_class, BountyRouter};
use chain_base::{
    base_network_descriptor, AutonomousBountyEventKind, AutonomousBountyFeedItem, ChainBaseError,
};
use chrono::{DateTime, Utc};
use domain::{
    Agent, AudienceInteraction, AudienceInteractionKind, AudienceLifecycleStage, AudienceMember,
    AudienceMetric, AudienceProvider, AudienceReport, AudienceRole, Bounty, BountyStatus,
    CanonicalBountyBinding, CanonicalFundingEvidence, CanonicalSettlementEvidence, Capability,
    CapabilityClass, Claim, ContributorContact, DiscoveryResponse, Escrow, EscrowStatus,
    FundingContribution, FundingContributionStatus, FundingIntent, FundingIntentStatus,
    FundingMode, FundingPartitionTarget, HelpRequest, Id, Money, Objective,
    ObjectiveCanonicalEvidence, OutreachAttempt, OutreachChannel, OutreachStatus, PaymentEvent,
    PaymentEventStatus, PaymentRail, PayoutIntent, PayoutStatus, PrivacyLevel, ProofRecord, Quote,
    ReputationEvent, RiskAction, RiskEvent, RiskReviewOutcome, RiskReviewRecord, RiskSurface,
    Settlement, Submission, TemplateSignal, VerificationDecision, VerifierKind, VerifierResult,
};
use ledger::{credit, debit, AccountCode, Ledger, LedgerEntry};
use payments_stripe::{
    evaluate_connect_payout, CheckoutTopUpRequest, ConnectAccountSnapshot, ConnectPayoutState,
    ConnectTransferEvidence, ConnectTransferRequest, StripeFundingCredit, StripePlanner,
    StripeRequestIntent,
};
use risk::{
    BountyRiskInput, HelpRequestRiskInput, PayoutRiskInput, RiskAssessment, RiskPolicy,
    SubmissionRiskInput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;
use verifier_sdk::{verify_with_builtin, VerificationInput};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("agent not found")]
    AgentNotFound,
    #[error("help request not found")]
    HelpRequestNotFound,
    #[error("quote not found")]
    QuoteNotFound,
    #[error("bounty not found")]
    BountyNotFound,
    #[error("submission not found")]
    SubmissionNotFound,
    #[error("submission does not belong to bounty")]
    SubmissionBountyMismatch,
    #[error("verifier result not found")]
    VerifierResultNotFound,
    #[error("verification did not accept submission: {0}")]
    VerificationNotAccepted(String),
    #[error("risk policy blocked operation: {0}")]
    RiskBlocked(String),
    #[error("risk policy requires review: {0}")]
    RiskNeedsReview(String),
    #[error("risk event not found")]
    RiskEventNotFound,
    #[error("risk event has already been reviewed")]
    RiskAlreadyReviewed,
    #[error("invalid risk review: {0}")]
    InvalidRiskReview(String),
    #[error("invalid funding contribution: {0}")]
    InvalidFundingContribution(String),
    #[error("invalid funding intent: {0}")]
    InvalidFundingIntent(String),
    #[error("invalid Stripe payout reconciliation: {0}")]
    InvalidStripePayout(String),
    #[error("invalid contributor contact: {0}")]
    InvalidContributorContact(String),
    #[error("audience member not found")]
    AudienceMemberNotFound,
    #[error("invalid audience record: {0}")]
    InvalidAudienceRecord(String),
    #[error(transparent)]
    Domain(#[from] domain::DomainError),
    #[error(transparent)]
    Ledger(#[from] ledger::LedgerError),
    #[error(transparent)]
    Verifier(#[from] verifier_sdk::VerifierError),
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveMoneyReadinessConfig {
    pub network: String,
    pub escrow_contract: Option<String>,
    pub usdc_token: Option<String>,
    pub stripe_secret_key_mode: String,
    pub stripe_live_execution_enabled: bool,
    pub stripe_payment_method_configuration_configured: bool,
    pub stripe_webhook_secret_configured: bool,
    pub allow_unsigned_stripe_webhooks: bool,
    pub operator_auth_configured: bool,
    pub base_rpc_url_configured: bool,
    pub base_broadcast_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveMoneyReadinessCheck {
    pub name: String,
    pub configured: bool,
    pub required_for: String,
    pub env_vars: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveMoneyReadinessReport {
    pub local_rehearsal_ready: bool,
    pub stripe_test_mode_ready: bool,
    pub stripe_live_mode_ready: bool,
    pub stripe_webhook_ready: bool,
    pub base_testnet_ready: bool,
    pub base_mainnet_ready: bool,
    pub base_broadcast_ready: bool,
    pub operator_auth_configured: bool,
    pub live_money_ready: bool,
    pub network: String,
    pub network_chain_id: u64,
    pub network_rpc_url_env: String,
    pub network_native_usdc_token_address: String,
    pub stripe_secret_key_mode: String,
    pub stripe_payment_method_configuration_configured: bool,
    pub supplied_usdc_token_matches_native: Option<bool>,
    pub checks: Vec<LiveMoneyReadinessCheck>,
    pub evidence_boundaries: Vec<String>,
    pub commands: Vec<String>,
    pub warnings: Vec<String>,
}

struct ReadinessWarningInputs<'a> {
    stripe_test_mode_ready: bool,
    stripe_live_mode_ready: bool,
    stripe_webhook_ready: bool,
    base_testnet_ready: bool,
    base_mainnet_ready: bool,
    unsigned_stripe_webhooks: bool,
    operator_token: bool,
    token_mismatch: bool,
    native_usdc_token_address: &'a str,
}

pub fn build_live_money_readiness_report(
    config: LiveMoneyReadinessConfig,
) -> Result<LiveMoneyReadinessReport, ChainBaseError> {
    let network_descriptor = base_network_descriptor(&config.network)?;
    let rpc_env = network_descriptor.rpc_url_env.clone();
    let stripe_secret_key_mode = config.stripe_secret_key_mode.trim().to_ascii_lowercase();
    let stripe_webhook_secret = config.stripe_webhook_secret_configured;
    let unsigned_stripe_webhooks = config.allow_unsigned_stripe_webhooks;
    let operator_token = config.operator_auth_configured;
    let base_rpc = config.base_rpc_url_configured;
    let base_broadcast_enabled = config.base_broadcast_enabled;
    // This compatibility field now carries the canonical autonomous factory address.
    let factory_configured = config.escrow_contract.as_deref().is_some_and(nonempty);
    let token_configured = config.usdc_token.as_deref().is_some_and(nonempty);
    let supplied_usdc_token_matches_native = config
        .usdc_token
        .as_deref()
        .filter(|value| nonempty(value))
        .map(|value| {
            value
                .trim()
                .eq_ignore_ascii_case(network_descriptor.native_usdc_token_address.as_str())
        });
    let token_matches_native = supplied_usdc_token_matches_native.unwrap_or(false);

    let checks = vec![
        readiness_check(
            "local deterministic rehearsal",
            true,
            "autonomous bounty creation, pooled funding, claims, verification, and settlement",
            vec![],
            "Run Foundry and chain-base tests; no external credentials required.",
        ),
        readiness_check(
            "Stripe test-mode execution gate",
            stripe_secret_key_mode == "test" && config.stripe_live_execution_enabled,
            "creating Stripe test-mode Checkout Sessions and Connect transfers",
            vec![
                "STRIPE_SECRET_KEY".to_string(),
                "ENABLE_STRIPE_LIVE_EXECUTION".to_string(),
            ],
            if stripe_secret_key_mode == "test" && config.stripe_live_execution_enabled {
                "Stripe test-mode execution is operator-enabled."
            } else if stripe_secret_key_mode == "live" {
                "A live Stripe key is configured; use the live-money check rather than test-mode execution."
            } else {
                "Set STRIPE_SECRET_KEY=sk_test_... and ENABLE_STRIPE_LIVE_EXECUTION=true before executing Stripe request intents."
            },
        ),
        readiness_check(
            "Stripe live-money execution gate",
            stripe_secret_key_mode == "live"
                && config.stripe_live_execution_enabled
                && stripe_webhook_secret
                && !unsigned_stripe_webhooks
                && operator_token,
            "creating live Stripe Checkout Sessions, live Connect accounts, and live transfer requests",
            vec![
                "STRIPE_SECRET_KEY".to_string(),
                "ENABLE_STRIPE_LIVE_EXECUTION".to_string(),
                "STRIPE_WEBHOOK_SECRET".to_string(),
                "OPERATOR_API_TOKEN".to_string(),
            ],
            if stripe_secret_key_mode == "live"
                && config.stripe_live_execution_enabled
                && stripe_webhook_secret
                && !unsigned_stripe_webhooks
                && operator_token
            {
                "Live Stripe execution is operator-gated and signed webhook reconciliation is configured."
            } else {
                "Use sk_live_ only in a hosted environment with ENABLE_STRIPE_LIVE_EXECUTION=true, STRIPE_WEBHOOK_SECRET, OPERATOR_API_TOKEN, and ALLOW_UNSIGNED_STRIPE_WEBHOOKS=false."
            },
        ),
        readiness_check(
            "Stripe Checkout payment-method configuration",
            config.stripe_payment_method_configuration_configured,
            "optional Dashboard-managed Checkout payment-method sets such as PayPal-capable configurations",
            vec!["STRIPE_PAYMENT_METHOD_CONFIGURATION".to_string()],
            if config.stripe_payment_method_configuration_configured {
                "Optional Stripe Payment Method Configuration is set; readiness reports only this boolean and not the configuration id."
            } else {
                "Optional Stripe Payment Method Configuration is unset; Checkout remains Dashboard-managed by default."
            },
        ),
        readiness_check(
            "Stripe webhook evidence gate",
            stripe_webhook_secret || unsigned_stripe_webhooks,
            "crediting fiat balances and marking fiat transfer evidence in local/test environments",
            vec![
                "STRIPE_WEBHOOK_SECRET".to_string(),
                "ALLOW_UNSIGNED_STRIPE_WEBHOOKS".to_string(),
            ],
            if stripe_webhook_secret {
                "Signed Stripe webhooks are configured."
            } else if unsigned_stripe_webhooks {
                "Unsigned Stripe webhook simulation is enabled; use only for local or mock-provider rehearsal."
            } else {
                "Set STRIPE_WEBHOOK_SECRET for signed webhooks, or ALLOW_UNSIGNED_STRIPE_WEBHOOKS=true for local-only simulation."
            },
        ),
        readiness_check(
            "Autonomous Base event indexing",
            base_rpc,
            "indexing canonical factory and per-bounty contract events",
            vec![rpc_env.clone()],
            if base_rpc {
                "Base RPC URL is configured for the selected network."
            } else {
                "Set the selected network RPC URL before indexing autonomous bounty events."
            },
        ),
        readiness_check(
            "Base signed transaction broadcast gate",
            base_rpc && base_broadcast_enabled,
            "relaying already-signed autonomous protocol transactions through the service",
            vec![rpc_env.clone(), "ENABLE_BASE_TX_BROADCAST".to_string()],
            if base_rpc && base_broadcast_enabled {
                "Base transaction broadcast is enabled for already-signed raw transactions."
            } else {
                "Signed transaction broadcast remains disabled; agents can submit through their wallet or another relayer."
            },
        ),
        readiness_check(
            "Autonomous bounty factory",
            factory_configured && token_configured && token_matches_native,
            "planning canonical bounty creation, pooled funding, claims, and settlement",
            vec![
                "BASE_MAINNET_BOUNTY_FACTORY".to_string(),
                "BASE_MAINNET_BOUNTY_IMPLEMENTATION".to_string(),
            ],
            if factory_configured && token_configured && token_matches_native {
                "Canonical factory and the selected network's native USDC address are configured."
            } else if factory_configured && token_configured {
                "Factory and token are configured, but the token is not native USDC for the selected network."
            } else {
                "Configure the reviewed autonomous factory and implementation before enabling live bounty actions."
            },
        ),
        readiness_check(
            "operator mutation auth",
            operator_token,
            "protecting optional hosted broadcast and live Stripe on-ramp endpoints",
            vec!["OPERATOR_API_TOKEN".to_string()],
            if operator_token {
                "Hosted operator token is configured."
            } else {
                "OPERATOR_API_TOKEN is optional for the protocol and required only for protected hosted operations."
            },
        ),
    ];

    let stripe_test_mode_ready =
        stripe_secret_key_mode == "test" && config.stripe_live_execution_enabled;
    let stripe_live_mode_ready = stripe_secret_key_mode == "live"
        && config.stripe_live_execution_enabled
        && stripe_webhook_secret
        && !unsigned_stripe_webhooks
        && operator_token;
    let stripe_webhook_ready = stripe_webhook_secret || unsigned_stripe_webhooks;
    let base_testnet_ready = network_descriptor.name == "Base Sepolia"
        && base_rpc
        && factory_configured
        && token_configured
        && token_matches_native;
    let base_mainnet_ready = network_descriptor.chain_id == 8_453
        && base_rpc
        && factory_configured
        && token_configured
        && token_matches_native;
    let base_broadcast_ready = base_rpc && base_broadcast_enabled;
    let live_money_ready = base_mainnet_ready;
    let warnings = readiness_warnings(ReadinessWarningInputs {
        stripe_test_mode_ready,
        stripe_live_mode_ready,
        stripe_webhook_ready,
        base_testnet_ready,
        base_mainnet_ready,
        unsigned_stripe_webhooks,
        operator_token,
        token_mismatch: token_configured && !token_matches_native,
        native_usdc_token_address: network_descriptor.native_usdc_token_address.as_str(),
    });

    Ok(LiveMoneyReadinessReport {
        local_rehearsal_ready: true,
        stripe_test_mode_ready,
        stripe_live_mode_ready,
        stripe_webhook_ready,
        base_testnet_ready,
        base_mainnet_ready,
        base_broadcast_ready,
        operator_auth_configured: operator_token,
        live_money_ready,
        network: network_descriptor.name,
        network_chain_id: network_descriptor.chain_id,
        network_rpc_url_env: network_descriptor.rpc_url_env,
        network_native_usdc_token_address: network_descriptor.native_usdc_token_address,
        stripe_secret_key_mode,
        stripe_payment_method_configuration_configured: config
            .stripe_payment_method_configuration_configured,
        supplied_usdc_token_matches_native,
        checks,
        evidence_boundaries: vec![
            "Stripe Checkout Session creation is not funding; only a verified checkout.session.completed webhook credits balance.".to_string(),
            "Stripe Payment Method Configuration only changes eligible Checkout methods; it is not funding, payout, or settlement evidence.".to_string(),
            "A signature or transaction hash is not funding evidence; only confirmed canonical FundingAdded events count.".to_string(),
            "Verification output is not payout evidence; only a confirmed BountySettled event from the canonical bounty contract proves payment.".to_string(),
            "Stripe and PayPal are optional convenience on-ramps and must deliver native USDC into the exact canonical bounty contract before funding is recognized.".to_string(),
        ],
        commands: vec![
            "forge test --fuzz-runs 1000".to_string(),
            "cargo test -p chain-base -p worker".to_string(),
            "GET /v1/base/autonomous-bounties/feed?network=base-mainnet".to_string(),
        ],
        warnings,
    })
}

pub fn stripe_secret_key_mode_from_secret(secret: Option<&str>) -> String {
    secret
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                "unset"
            } else if value.starts_with("sk_test_") {
                "test"
            } else if value.starts_with("sk_live_") {
                "live"
            } else if value.starts_with("rk_") {
                "restricted"
            } else {
                "unknown"
            }
        })
        .unwrap_or("unset")
        .to_string()
}

fn readiness_check(
    name: impl Into<String>,
    configured: bool,
    required_for: impl Into<String>,
    env_vars: Vec<String>,
    detail: impl Into<String>,
) -> LiveMoneyReadinessCheck {
    LiveMoneyReadinessCheck {
        name: name.into(),
        configured,
        required_for: required_for.into(),
        env_vars,
        detail: detail.into(),
    }
}

fn readiness_warnings(input: ReadinessWarningInputs<'_>) -> Vec<String> {
    let mut warnings = Vec::new();
    if !input.stripe_test_mode_ready {
        warnings.push(
            "Stripe request execution is not ready; use plan-only mode or set test-mode credentials and ENABLE_STRIPE_LIVE_EXECUTION=true."
                .to_string(),
        );
    }
    if !input.stripe_live_mode_ready {
        warnings.push(
            "Stripe live-money execution is not ready; hosted live flows require sk_live_ credentials, signed webhooks, ENABLE_STRIPE_LIVE_EXECUTION=true, OPERATOR_API_TOKEN, and unsigned webhooks disabled."
                .to_string(),
        );
    }
    if !input.stripe_webhook_ready {
        warnings.push(
            "Stripe fiat ledger credits will be rejected until signed webhooks or local unsigned webhook simulation are configured."
                .to_string(),
        );
    }
    if !input.base_testnet_ready {
        warnings.push(
            "Base Sepolia autonomous indexing needs an RPC URL, reviewed factory, and native USDC configuration."
                .to_string(),
        );
    }
    if !input.base_mainnet_ready {
        warnings.push(
            "Base mainnet autonomous USDC is not live-ready; configure the RPC URL, reviewed bounty factory and implementation, and native USDC."
                .to_string(),
        );
    }
    if input.token_mismatch {
        warnings.push(format!(
            "The supplied USDC token does not match the selected network native USDC token: {}.",
            input.native_usdc_token_address
        ));
    }
    if input.unsigned_stripe_webhooks {
        warnings.push(
            "ALLOW_UNSIGNED_STRIPE_WEBHOOKS must not be used for hosted or production money flows."
                .to_string(),
        );
    }
    if !input.operator_token {
        warnings.push(
            "Set OPERATOR_API_TOKEN before exposing optional hosted broadcast or live Stripe on-ramp endpoints; autonomous contract settlement does not require it."
                .to_string(),
        );
    }
    warnings
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn normalized_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn dedup_nonempty(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if !value.is_empty() && seen.insert(value.to_lowercase()) {
            normalized.push(value);
        }
    }
    normalized
}

fn required_audience_string(value: String, field: &str) -> AppResult<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(AppError::InvalidAudienceRecord(format!(
            "{field} is required"
        )));
    }
    Ok(value)
}

fn normalized_public_url(value: Option<String>, field: &str) -> AppResult<Option<String>> {
    let Some(value) = normalized_optional_string(value) else {
        return Ok(None);
    };
    if !(value.starts_with("https://") || value.starts_with("http://")) {
        return Err(AppError::InvalidAudienceRecord(format!(
            "{field} must be an http(s) URL"
        )));
    }
    Ok(Some(value))
}

fn deterministic_audience_id(record_kind: &str, key: &str) -> Id {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("agent-bounties:{record_kind}:{key}").as_bytes(),
    )
}

fn dedup_audience_roles(values: impl IntoIterator<Item = AudienceRole>) -> Vec<AudienceRole> {
    let mut roles = Vec::new();
    for role in values {
        if !roles.contains(&role) {
            roles.push(role);
        }
    }
    if roles.is_empty() {
        roles.push(AudienceRole::Observer);
    }
    roles
}

fn role_for_interaction(kind: AudienceInteractionKind) -> AudienceRole {
    match kind {
        AudienceInteractionKind::IssueOpened
        | AudienceInteractionKind::PullRequestOpened
        | AudienceInteractionKind::IssueCommented
        | AudienceInteractionKind::PullRequestReviewed => AudienceRole::Contributor,
        AudienceInteractionKind::BountyPosted => AudienceRole::BountyPoster,
        AudienceInteractionKind::FundingSignaled => AudienceRole::ProspectiveFunder,
        AudienceInteractionKind::BountyFunded => AudienceRole::Funder,
        AudienceInteractionKind::ClaimSignaled => AudienceRole::ProspectiveSolver,
        AudienceInteractionKind::BountyClaimed => AudienceRole::Claimer,
        AudienceInteractionKind::SubmissionMade | AudienceInteractionKind::SubmissionAccepted => {
            AudienceRole::Solver
        }
        AudienceInteractionKind::VerificationSubmitted => AudienceRole::Verifier,
        AudienceInteractionKind::PayoutReceived => AudienceRole::Recipient,
        AudienceInteractionKind::RepoStarred
        | AudienceInteractionKind::BountyUpvoted
        | AudienceInteractionKind::ProofShared
        | AudienceInteractionKind::ReferralCreated => AudienceRole::Promoter,
    }
}

fn lifecycle_for_roles(roles: &[AudienceRole]) -> AudienceLifecycleStage {
    if roles.iter().any(|role| {
        matches!(
            role,
            AudienceRole::BountyPoster
                | AudienceRole::Funder
                | AudienceRole::Solver
                | AudienceRole::Verifier
                | AudienceRole::Recipient
        )
    }) {
        AudienceLifecycleStage::Converted
    } else if roles.iter().any(|role| *role != AudienceRole::Observer) {
        AudienceLifecycleStage::Engaged
    } else {
        AudienceLifecycleStage::Observed
    }
}

fn interaction_kind_key(kind: AudienceInteractionKind) -> &'static str {
    match kind {
        AudienceInteractionKind::IssueOpened => "issue_opened",
        AudienceInteractionKind::PullRequestOpened => "pull_request_opened",
        AudienceInteractionKind::IssueCommented => "issue_commented",
        AudienceInteractionKind::PullRequestReviewed => "pull_request_reviewed",
        AudienceInteractionKind::BountyPosted => "bounty_posted",
        AudienceInteractionKind::FundingSignaled => "funding_signaled",
        AudienceInteractionKind::BountyFunded => "bounty_funded",
        AudienceInteractionKind::ClaimSignaled => "claim_signaled",
        AudienceInteractionKind::BountyClaimed => "bounty_claimed",
        AudienceInteractionKind::SubmissionMade => "submission_made",
        AudienceInteractionKind::SubmissionAccepted => "submission_accepted",
        AudienceInteractionKind::VerificationSubmitted => "verification_submitted",
        AudienceInteractionKind::PayoutReceived => "payout_received",
        AudienceInteractionKind::RepoStarred => "repo_starred",
        AudienceInteractionKind::BountyUpvoted => "bounty_upvoted",
        AudienceInteractionKind::ProofShared => "proof_shared",
        AudienceInteractionKind::ReferralCreated => "referral_created",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterAgentRequest {
    pub handle: String,
    pub payout_wallet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertContributorContactRequest {
    pub github_login: String,
    pub email: Option<String>,
    pub payout_wallet: Option<String>,
    #[serde(default)]
    pub associated_prs: Vec<String>,
    #[serde(default)]
    pub contact_consent: bool,
    #[serde(default)]
    pub wallet_consent: bool,
    #[serde(default)]
    pub outreach_allowed: bool,
    #[serde(default)]
    pub source: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertAudienceMemberRequest {
    pub provider: AudienceProvider,
    pub external_id: String,
    pub handle: String,
    pub public_profile_url: Option<String>,
    #[serde(default)]
    pub roles: Vec<AudienceRole>,
    pub observed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordAudienceInteractionRequest {
    pub audience_member_id: Id,
    pub provider_event_id: String,
    pub kind: AudienceInteractionKind,
    pub public_url: Option<String>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub referrer_url: Option<String>,
    pub campaign: Option<String>,
    pub source_interaction_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordDiscoveryResponseRequest {
    pub audience_member_id: Id,
    pub interaction_id: Option<Id>,
    pub provider_response_id: String,
    pub public_source_url: Option<String>,
    pub found_via: String,
    pub motivation: String,
    pub improvement_suggestion: String,
    pub agent_or_tool: Option<String>,
    #[serde(default)]
    pub private_storage_consent: bool,
    pub captured_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordOutreachAttemptRequest {
    pub audience_member_id: Id,
    pub provider_event_id: String,
    pub channel: OutreachChannel,
    pub public_url: Option<String>,
    pub prompt_version: String,
    pub status: OutreachStatus,
    pub sent_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostBountyRequest {
    pub title: String,
    pub template_slug: String,
    pub amount_minor: i64,
    pub currency: String,
    pub funding_mode: FundingMode,
    pub privacy: PrivacyLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenPooledBountyRequest {
    #[serde(default)]
    pub bounty_id: Option<Id>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    pub title: String,
    pub template_slug: String,
    pub target_amount_minor: i64,
    pub currency: String,
    pub funding_mode: FundingMode,
    pub privacy: PrivacyLevel,
    #[serde(default)]
    pub funding_targets: Vec<FundingPartitionTargetRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddFundingContributionRequest {
    pub bounty_id: Id,
    pub contributor_agent_id: Option<Id>,
    pub source_organization_id: Option<Id>,
    pub amount_minor: i64,
    pub currency: String,
    pub rail: PaymentRail,
    pub external_reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingPartitionTargetRequest {
    pub rail: PaymentRail,
    pub amount_minor: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFundingIntentRequest {
    pub bounty_id: Id,
    pub contributor_agent_id: Option<Id>,
    pub source_organization_id: Option<Id>,
    pub amount_minor: i64,
    pub currency: String,
    pub rail: PaymentRail,
    pub external_reference: Option<String>,
    pub stripe_success_url: Option<String>,
    pub stripe_cancel_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterCapabilityRequest {
    pub agent_id: Id,
    pub class: CapabilityClass,
    pub template_slugs: Vec<String>,
    pub min_price_minor: i64,
    pub max_price_minor: i64,
    pub currency: String,
    pub latency_seconds: u64,
    pub supported_verifiers: Vec<VerifierKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateHelpRequestRequest {
    pub requester_agent_id: Id,
    pub goal: String,
    pub context: String,
    pub budget_minor: i64,
    pub currency: String,
    pub privacy: PrivacyLevel,
    pub required_confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestQuotesRequest {
    pub help_request_id: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteSet {
    pub help_request: HelpRequest,
    pub route: bounty_router::RouteDecision,
    pub quotes: Vec<Quote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundQuoteRequest {
    pub quote_id: Id,
    pub title: Option<String>,
    pub funding_mode: Option<FundingMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimBountyRequest {
    pub bounty_id: Id,
    pub solver_agent_id: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitResultRequest {
    pub bounty_id: Id,
    pub solver_agent_id: Id,
    pub artifact_uri: String,
    pub artifact_body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifySubmissionRequest {
    pub bounty_id: Id,
    pub submission_id: Id,
    pub expected_artifact_digest: String,
    pub verifier_kind: Option<VerifierKind>,
    pub rubric: Option<String>,
    pub evidence: Option<Value>,
    pub approved_risk_event_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum FundingIntentNextAction {
    StripeCheckout { request: StripeRequestIntent },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingIntentReport {
    pub bounty: Bounty,
    pub intent: FundingIntent,
    pub funding_summary: PooledFundingSummary,
    pub next_action: FundingIntentNextAction,
    pub requires_reconciliation: bool,
    pub reconciliation_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BountyStatusResponse {
    pub bounty: Bounty,
    pub funding_summary: PooledFundingSummary,
    pub funding_intents: Vec<FundingIntent>,
    pub funding_contributions: Vec<FundingContribution>,
    pub escrows: Vec<Escrow>,
    pub claims: Vec<Claim>,
    pub submissions: Vec<Submission>,
    pub verifier_results: Vec<VerifierResult>,
    pub proofs: Vec<ProofRecord>,
    pub settlements: Vec<Settlement>,
    pub reputation_events: Vec<ReputationEvent>,
    pub template_signals: Vec<TemplateSignal>,
    pub risk_events: Vec<RiskEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPayoutLine {
    pub settlement_id: Id,
    pub bounty_id: Id,
    pub proof_record_id: Id,
    pub rail: PaymentRail,
    pub amount: Money,
    pub status: PayoutStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentPayoutTotalsByCurrency {
    pub currency: String,
    pub pending_minor: i64,
    pub blocked_minor: i64,
    pub paying_minor: i64,
    pub paid_minor: i64,
    pub failed_minor: i64,
    pub total_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPayoutStatusResponse {
    pub agent: Agent,
    pub payouts: Vec<AgentPayoutLine>,
    pub totals: Vec<AgentPayoutTotalsByCurrency>,
    pub reputation_events: Vec<ReputationEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RiskEventFilter {
    pub action: Option<RiskAction>,
    pub surface: Option<RiskSurface>,
    pub bounty_id: Option<Id>,
    pub agent_id: Option<Id>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveRiskBountyRequest {
    pub risk_event_id: Id,
    pub title: String,
    pub template_slug: String,
    pub amount_minor: i64,
    pub currency: String,
    pub funding_mode: FundingMode,
    pub privacy: PrivacyLevel,
    pub operator_id: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveRiskPayoutRequest {
    pub risk_event_id: Id,
    pub operator_id: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectRiskEventRequest {
    pub risk_event_id: Id,
    pub operator_id: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewedBountyApproval {
    pub bounty: Bounty,
    pub review: RiskReviewRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PooledFundingSummary {
    pub bounty_id: Id,
    pub target: Money,
    pub applied: Money,
    pub remaining: Money,
    pub contribution_count: usize,
    pub partitions: Vec<FundingPartitionSummary>,
    pub claimable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingPartitionSummary {
    pub rail: PaymentRail,
    pub target: Money,
    pub confirmed: Money,
    pub remaining: Money,
    pub contribution_count: usize,
    pub escrow_count: usize,
    pub claimable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PooledFundingReport {
    pub bounty: Bounty,
    pub contribution: FundingContribution,
    pub funding_summary: PooledFundingSummary,
    pub ledger_entries: Vec<LedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeConnectPayoutReconciliation {
    pub payout_state: ConnectPayoutState,
    pub settlements: Vec<Settlement>,
    pub bounties: Vec<Bounty>,
    pub ledger_entries: Vec<LedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStripeTransferRequest {
    pub payout_intent_id: Id,
    pub connected_account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeTransferPlan {
    pub settlement: Settlement,
    pub payout_intent: PayoutIntent,
    pub request: StripeRequestIntent,
    pub requires_reconciliation: bool,
    pub reconciliation_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeTransferReconciliation {
    pub evidence: ConnectTransferEvidence,
    pub duplicate: bool,
    pub settlement: Option<Settlement>,
    pub bounty: Option<Bounty>,
    pub ledger_entries: Vec<LedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeFundingReconciliation {
    pub funding_credit: StripeFundingCredit,
    pub duplicate: bool,
    pub ledger_entries: Vec<LedgerEntry>,
    pub funding_intent: Option<FundingIntent>,
    pub funding_report: Option<PooledFundingReport>,
}

#[derive(Debug)]
pub struct BountyNetwork {
    pub agents: HashMap<Id, Agent>,
    pub contributor_contacts: HashMap<Id, ContributorContact>,
    pub audience_members: HashMap<Id, AudienceMember>,
    pub audience_interactions: HashMap<Id, AudienceInteraction>,
    pub discovery_responses: HashMap<Id, DiscoveryResponse>,
    pub outreach_attempts: HashMap<Id, OutreachAttempt>,
    pub capabilities: HashMap<Id, Capability>,
    pub help_requests: HashMap<Id, HelpRequest>,
    pub quotes: HashMap<Id, Quote>,
    pub bounties: HashMap<Id, Bounty>,
    pub objectives: HashMap<Id, Objective>,
    pub funding_intents: HashMap<Id, FundingIntent>,
    pub funding_contributions: HashMap<Id, FundingContribution>,
    pub escrows: HashMap<Id, Escrow>,
    pub claims: HashMap<Id, Claim>,
    pub submissions: HashMap<Id, Submission>,
    pub verifier_results: HashMap<Id, VerifierResult>,
    pub proofs: HashMap<Id, ProofRecord>,
    pub settlements: HashMap<Id, Settlement>,
    pub reputation_events: HashMap<Id, ReputationEvent>,
    pub template_signals: HashMap<Id, TemplateSignal>,
    pub risk_events: HashMap<Id, RiskEvent>,
    pub risk_reviews: HashMap<Id, RiskReviewRecord>,
    pub payment_events: HashMap<Id, PaymentEvent>,
    pub ledger: Ledger,
    pub risk_policy: RiskPolicy,
}

impl Default for BountyNetwork {
    fn default() -> Self {
        Self {
            agents: HashMap::new(),
            contributor_contacts: HashMap::new(),
            audience_members: HashMap::new(),
            audience_interactions: HashMap::new(),
            discovery_responses: HashMap::new(),
            outreach_attempts: HashMap::new(),
            capabilities: HashMap::new(),
            help_requests: HashMap::new(),
            quotes: HashMap::new(),
            bounties: HashMap::new(),
            objectives: HashMap::new(),
            funding_intents: HashMap::new(),
            funding_contributions: HashMap::new(),
            escrows: HashMap::new(),
            claims: HashMap::new(),
            submissions: HashMap::new(),
            verifier_results: HashMap::new(),
            proofs: HashMap::new(),
            settlements: HashMap::new(),
            reputation_events: HashMap::new(),
            template_signals: HashMap::new(),
            risk_events: HashMap::new(),
            risk_reviews: HashMap::new(),
            payment_events: HashMap::new(),
            ledger: Ledger::new(),
            risk_policy: RiskPolicy::default(),
        }
    }
}

pub fn build_objective_canonical_evidence(
    network: &str,
    feed: &[AutonomousBountyFeedItem],
) -> ObjectiveCanonicalEvidence {
    let mut evidence = ObjectiveCanonicalEvidence::default();
    for item in feed {
        let binding = CanonicalBountyBinding {
            network: network.to_string(),
            bounty_contract: item.bounty_contract.clone(),
            bounty_id: item.bounty_id.clone(),
            terms_hash: item.terms_hash.clone(),
        };
        if let (Ok(funded_atomic_amount), Ok(target_atomic_amount)) = (
            item.funded_amount.parse::<u64>(),
            item.target_amount.parse::<u64>(),
        ) {
            let confirming_event_id = item
                .events
                .iter()
                .rev()
                .find(|event| {
                    matches!(
                        event.kind,
                        AutonomousBountyEventKind::FundingAdded
                            | AutonomousBountyEventKind::BountyBecameClaimable
                    )
                })
                .map(|event| event.log_key.clone())
                .unwrap_or_default();
            evidence.funding.push(CanonicalFundingEvidence {
                binding: binding.clone(),
                funded_atomic_amount,
                target_atomic_amount,
                status: item.status.clone(),
                verification_ready: item.verification_ready,
                verification_readiness_reason: item.verification_readiness_reason.clone(),
                confirming_event_id,
            });
        }
        for event in item
            .events
            .iter()
            .filter(|event| event.kind == AutonomousBountyEventKind::BountySettled)
        {
            let Some(recipient_wallet) = event.data["solver"].as_str() else {
                continue;
            };
            let Some(solver_payout_atomic_amount) = event.data["solver_payout"].as_u64() else {
                continue;
            };
            let Some(submission_hash) = event.data["submission_hash"].as_str() else {
                continue;
            };
            let Some(evidence_hash) = event.data["evidence_hash"].as_str() else {
                continue;
            };
            evidence.settlements.push(CanonicalSettlementEvidence {
                binding: binding.clone(),
                event_id: event.log_key.clone(),
                tx_hash: event.tx_hash.clone(),
                block_number: event.block_number,
                log_index: event.log_index,
                recipient_wallet: recipient_wallet.to_string(),
                solver_payout_atomic_amount,
                submission_hash: submission_hash.to_string(),
                evidence_hash: evidence_hash.to_string(),
            });
        }
    }
    evidence
}

pub fn build_audience_report(
    members: &[AudienceMember],
    interactions: &[AudienceInteraction],
    discovery_responses: &[DiscoveryResponse],
    outreach_attempts: &[OutreachAttempt],
) -> AudienceReport {
    let asked_member_ids: HashSet<_> = outreach_attempts
        .iter()
        .map(|attempt| attempt.audience_member_id)
        .collect();
    let answered_member_ids: HashSet<_> = discovery_responses
        .iter()
        .map(|response| response.audience_member_id)
        .collect();
    let mut interaction_counts_by_member = HashMap::<Id, u64>::new();
    let mut posters = HashSet::new();
    let mut funders = HashSet::new();
    let mut solvers = HashSet::new();
    let mut paid = HashSet::new();
    let mut interactions_by_kind = BTreeMap::<String, u64>::new();
    let mut repo_stars = 0_u64;
    let mut shares = 0_u64;

    for interaction in interactions {
        *interaction_counts_by_member
            .entry(interaction.audience_member_id)
            .or_default() += 1;
        *interactions_by_kind
            .entry(interaction_kind_key(interaction.kind).to_string())
            .or_default() += 1;
        match interaction.kind {
            AudienceInteractionKind::BountyPosted => {
                posters.insert(interaction.audience_member_id);
            }
            AudienceInteractionKind::BountyFunded => {
                funders.insert(interaction.audience_member_id);
            }
            AudienceInteractionKind::SubmissionMade
            | AudienceInteractionKind::SubmissionAccepted => {
                solvers.insert(interaction.audience_member_id);
            }
            AudienceInteractionKind::PayoutReceived => {
                paid.insert(interaction.audience_member_id);
            }
            AudienceInteractionKind::RepoStarred => repo_stars += 1,
            AudienceInteractionKind::ProofShared | AudienceInteractionKind::ReferralCreated => {
                shares += 1;
            }
            _ => {}
        }
    }

    let mut not_asked_or_answered_handles = members
        .iter()
        .filter(|member| {
            !asked_member_ids.contains(&member.id) && !answered_member_ids.contains(&member.id)
        })
        .map(|member| member.handle.clone())
        .collect::<Vec<_>>();
    let mut asked_without_response_handles = members
        .iter()
        .filter(|member| {
            asked_member_ids.contains(&member.id) && !answered_member_ids.contains(&member.id)
        })
        .map(|member| member.handle.clone())
        .collect::<Vec<_>>();
    not_asked_or_answered_handles.sort_by_key(|handle| handle.to_lowercase());
    asked_without_response_handles.sort_by_key(|handle| handle.to_lowercase());

    AudienceReport {
        total_members: members.len() as u64,
        total_interactions: interactions.len() as u64,
        members_asked_for_discovery_feedback: asked_member_ids.len() as u64,
        members_with_discovery_responses: answered_member_ids.len() as u64,
        repeat_participants: interaction_counts_by_member
            .values()
            .filter(|count| **count >= 2)
            .count() as u64,
        external_bounty_posters: posters.len() as u64,
        external_funders: funders.len() as u64,
        external_solvers: solvers.len() as u64,
        paid_participants: paid.len() as u64,
        repo_stars_attributed: repo_stars,
        shares_attributed: shares,
        not_asked_or_answered_handles,
        asked_without_response_handles,
        interactions_by_kind: interactions_by_kind
            .into_iter()
            .map(|(key, count)| AudienceMetric { key, count })
            .collect(),
        generated_at: Utc::now(),
    }
}

impl BountyNetwork {
    pub fn register_agent(&mut self, request: RegisterAgentRequest) -> Agent {
        let mut agent = Agent::new(request.handle);
        agent.payout_wallet = request.payout_wallet;
        self.agents.insert(agent.id, agent.clone());
        agent
    }

    pub fn upsert_contributor_contact(
        &mut self,
        request: UpsertContributorContactRequest,
    ) -> AppResult<ContributorContact> {
        let github_login = request
            .github_login
            .trim()
            .trim_start_matches('@')
            .to_string();
        if github_login.is_empty() {
            return Err(AppError::InvalidContributorContact(
                "github_login is required".to_string(),
            ));
        }

        let email = normalized_optional_string(request.email);
        if email.is_some() && !request.contact_consent {
            return Err(AppError::InvalidContributorContact(
                "email requires contact_consent=true".to_string(),
            ));
        }
        let payout_wallet = normalized_optional_string(request.payout_wallet);
        if payout_wallet.is_some() && !request.wallet_consent {
            return Err(AppError::InvalidContributorContact(
                "payout_wallet requires wallet_consent=true".to_string(),
            ));
        }
        if request.outreach_allowed && !request.contact_consent {
            return Err(AppError::InvalidContributorContact(
                "outreach_allowed requires contact_consent=true".to_string(),
            ));
        }

        let source =
            normalized_optional_string(request.source).unwrap_or_else(|| "operator".into());
        let notes = normalized_optional_string(request.notes);
        let login_key = github_login.to_lowercase();
        let existing_id = self.contributor_contacts.iter().find_map(|(id, contact)| {
            (contact.github_login.to_lowercase() == login_key).then_some(*id)
        });
        let mut associated_prs = dedup_nonempty(request.associated_prs);
        let now = Utc::now();

        let contact = if let Some(existing_id) = existing_id {
            let mut contact = self
                .contributor_contacts
                .get(&existing_id)
                .cloned()
                .ok_or_else(|| {
                    AppError::InvalidContributorContact(
                        "existing contributor contact was not found".to_string(),
                    )
                })?;
            associated_prs.extend(contact.associated_prs.iter().cloned());
            contact.github_login = github_login;
            contact.email = email;
            contact.payout_wallet = payout_wallet;
            contact.associated_prs = dedup_nonempty(associated_prs);
            contact.contact_consent = request.contact_consent;
            contact.wallet_consent = request.wallet_consent;
            contact.outreach_allowed = request.outreach_allowed;
            contact.source = source;
            contact.notes = notes;
            contact.updated_at = now;
            contact
        } else {
            let mut contact = ContributorContact::new(github_login, source);
            contact.email = email;
            contact.payout_wallet = payout_wallet;
            contact.associated_prs = associated_prs;
            contact.contact_consent = request.contact_consent;
            contact.wallet_consent = request.wallet_consent;
            contact.outreach_allowed = request.outreach_allowed;
            contact.notes = notes;
            contact.updated_at = now;
            contact
        };
        self.contributor_contacts
            .insert(contact.id, contact.clone());
        Ok(contact)
    }

    pub fn list_contributor_contacts(&self) -> Vec<ContributorContact> {
        let mut contacts: Vec<_> = self.contributor_contacts.values().cloned().collect();
        contacts.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.github_login.cmp(&right.github_login))
        });
        contacts
    }

    pub fn upsert_audience_member(
        &mut self,
        request: UpsertAudienceMemberRequest,
    ) -> AppResult<AudienceMember> {
        let external_id = required_audience_string(request.external_id, "external_id")?;
        let handle = required_audience_string(request.handle, "handle")?;
        let public_profile_url =
            normalized_public_url(request.public_profile_url, "public_profile_url")?;
        let observed_at = request.observed_at.unwrap_or_else(Utc::now);
        let external_id_key = external_id.to_lowercase();
        let existing_id = self.audience_members.iter().find_map(|(id, member)| {
            (member.provider == request.provider
                && member.external_id.to_lowercase() == external_id_key)
                .then_some(*id)
        });

        let member = if let Some(existing_id) = existing_id {
            let mut member = self
                .audience_members
                .get(&existing_id)
                .cloned()
                .ok_or(AppError::AudienceMemberNotFound)?;
            let mut roles = member.roles.clone();
            roles.extend(request.roles);
            member.external_id = external_id;
            member.handle = handle;
            if public_profile_url.is_some() {
                member.public_profile_url = public_profile_url;
            }
            member.roles = dedup_audience_roles(roles);
            member.lifecycle_stage = member
                .lifecycle_stage
                .max(lifecycle_for_roles(&member.roles));
            member.first_seen_at = member.first_seen_at.min(observed_at);
            member.last_seen_at = member.last_seen_at.max(observed_at);
            member
        } else {
            let roles = dedup_audience_roles(request.roles);
            let provider_key = format!("{:?}", request.provider).to_lowercase();
            AudienceMember {
                id: deterministic_audience_id(
                    "member",
                    &format!("{provider_key}:{external_id_key}"),
                ),
                provider: request.provider,
                external_id,
                handle,
                public_profile_url,
                lifecycle_stage: lifecycle_for_roles(&roles),
                roles,
                first_seen_at: observed_at,
                last_seen_at: observed_at,
            }
        };
        self.audience_members.insert(member.id, member.clone());
        Ok(member)
    }

    pub fn list_audience_members(&self) -> Vec<AudienceMember> {
        let mut members: Vec<_> = self.audience_members.values().cloned().collect();
        members.sort_by(|left, right| {
            left.first_seen_at
                .cmp(&right.first_seen_at)
                .then_with(|| left.handle.cmp(&right.handle))
        });
        members
    }

    pub fn record_audience_interaction(
        &mut self,
        request: RecordAudienceInteractionRequest,
    ) -> AppResult<AudienceInteraction> {
        if !self
            .audience_members
            .contains_key(&request.audience_member_id)
        {
            return Err(AppError::AudienceMemberNotFound);
        }
        let provider_event_id =
            required_audience_string(request.provider_event_id, "provider_event_id")?;
        if let Some(existing) = self.audience_interactions.values().find(|interaction| {
            interaction.audience_member_id == request.audience_member_id
                && interaction.provider_event_id == provider_event_id
        }) {
            return Ok(existing.clone());
        }
        if let Some(source_interaction_id) = request.source_interaction_id {
            if !self
                .audience_interactions
                .contains_key(&source_interaction_id)
            {
                return Err(AppError::InvalidAudienceRecord(
                    "source_interaction_id was not found".to_string(),
                ));
            }
        }

        let public_url = normalized_public_url(request.public_url, "public_url")?;
        let referrer_url = normalized_public_url(request.referrer_url, "referrer_url")?;
        let occurred_at = request.occurred_at.unwrap_or_else(Utc::now);
        let interaction = AudienceInteraction {
            id: deterministic_audience_id(
                "interaction",
                &format!("{}:{provider_event_id}", request.audience_member_id),
            ),
            audience_member_id: request.audience_member_id,
            provider_event_id,
            kind: request.kind,
            public_url,
            occurred_at,
            referrer_url,
            campaign: normalized_optional_string(request.campaign),
            source_interaction_id: request.source_interaction_id,
        };
        self.audience_interactions
            .insert(interaction.id, interaction.clone());

        let interaction_count = self
            .audience_interactions
            .values()
            .filter(|candidate| candidate.audience_member_id == request.audience_member_id)
            .count();
        if let Some(member) = self.audience_members.get_mut(&request.audience_member_id) {
            let mut roles = member.roles.clone();
            roles.push(role_for_interaction(request.kind));
            member.roles = dedup_audience_roles(roles);
            member.last_seen_at = member.last_seen_at.max(occurred_at);
            member.lifecycle_stage = if interaction_count >= 2 {
                AudienceLifecycleStage::Retained
            } else {
                member
                    .lifecycle_stage
                    .max(lifecycle_for_roles(&member.roles))
            };
        }
        Ok(interaction)
    }

    pub fn list_audience_interactions(&self) -> Vec<AudienceInteraction> {
        let mut interactions: Vec<_> = self.audience_interactions.values().cloned().collect();
        interactions.sort_by(|left, right| {
            left.occurred_at
                .cmp(&right.occurred_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        interactions
    }

    pub fn record_discovery_response(
        &mut self,
        request: RecordDiscoveryResponseRequest,
    ) -> AppResult<DiscoveryResponse> {
        if !self
            .audience_members
            .contains_key(&request.audience_member_id)
        {
            return Err(AppError::AudienceMemberNotFound);
        }
        if let Some(interaction_id) = request.interaction_id {
            let interaction = self
                .audience_interactions
                .get(&interaction_id)
                .ok_or_else(|| {
                    AppError::InvalidAudienceRecord("interaction_id was not found".to_string())
                })?;
            if interaction.audience_member_id != request.audience_member_id {
                return Err(AppError::InvalidAudienceRecord(
                    "interaction_id belongs to another audience member".to_string(),
                ));
            }
        }
        let provider_response_id =
            required_audience_string(request.provider_response_id, "provider_response_id")?;
        if let Some(existing) = self.discovery_responses.values().find(|response| {
            response.audience_member_id == request.audience_member_id
                && response.provider_response_id == provider_response_id
        }) {
            return Ok(existing.clone());
        }
        let public_source_url =
            normalized_public_url(request.public_source_url, "public_source_url")?;
        if public_source_url.is_none() && !request.private_storage_consent {
            return Err(AppError::InvalidAudienceRecord(
                "a discovery response requires a public source URL or private_storage_consent=true"
                    .to_string(),
            ));
        }

        let response = DiscoveryResponse {
            id: deterministic_audience_id(
                "discovery-response",
                &format!("{}:{provider_response_id}", request.audience_member_id),
            ),
            audience_member_id: request.audience_member_id,
            interaction_id: request.interaction_id,
            provider_response_id,
            public_source_url,
            found_via: required_audience_string(request.found_via, "found_via")?,
            motivation: required_audience_string(request.motivation, "motivation")?,
            improvement_suggestion: required_audience_string(
                request.improvement_suggestion,
                "improvement_suggestion",
            )?,
            agent_or_tool: normalized_optional_string(request.agent_or_tool),
            private_storage_consent: request.private_storage_consent,
            captured_at: request.captured_at.unwrap_or_else(Utc::now),
        };
        self.discovery_responses
            .insert(response.id, response.clone());
        Ok(response)
    }

    pub fn list_discovery_responses(&self) -> Vec<DiscoveryResponse> {
        let mut responses: Vec<_> = self.discovery_responses.values().cloned().collect();
        responses.sort_by(|left, right| {
            left.captured_at
                .cmp(&right.captured_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        responses
    }

    pub fn record_outreach_attempt(
        &mut self,
        request: RecordOutreachAttemptRequest,
    ) -> AppResult<OutreachAttempt> {
        let member = self
            .audience_members
            .get(&request.audience_member_id)
            .ok_or(AppError::AudienceMemberNotFound)?;
        let provider_event_id =
            required_audience_string(request.provider_event_id, "provider_event_id")?;
        if let Some(existing) = self.outreach_attempts.values().find(|attempt| {
            attempt.audience_member_id == request.audience_member_id
                && attempt.provider_event_id == provider_event_id
        }) {
            return Ok(existing.clone());
        }

        let public_url = normalized_public_url(request.public_url, "public_url")?;
        let consent_contact_id = if request.channel == OutreachChannel::EmailPrivate {
            let contact = self
                .contributor_contacts
                .values()
                .find(|contact| {
                    contact.github_login.eq_ignore_ascii_case(&member.handle)
                        && contact.contact_consent
                        && contact.outreach_allowed
                })
                .ok_or_else(|| {
                    AppError::InvalidAudienceRecord(
                        "private outreach requires an opted-in contributor contact".to_string(),
                    )
                })?;
            Some(contact.id)
        } else {
            if public_url.is_none() {
                return Err(AppError::InvalidAudienceRecord(
                    "public outreach requires public_url".to_string(),
                ));
            }
            None
        };

        let attempt = OutreachAttempt {
            id: deterministic_audience_id(
                "outreach",
                &format!("{}:{provider_event_id}", request.audience_member_id),
            ),
            audience_member_id: request.audience_member_id,
            provider_event_id,
            channel: request.channel,
            public_url,
            prompt_version: required_audience_string(request.prompt_version, "prompt_version")?,
            status: request.status,
            consent_contact_id,
            sent_at: request.sent_at.unwrap_or_else(Utc::now),
        };
        self.outreach_attempts.insert(attempt.id, attempt.clone());
        Ok(attempt)
    }

    pub fn list_outreach_attempts(&self) -> Vec<OutreachAttempt> {
        let mut attempts: Vec<_> = self.outreach_attempts.values().cloned().collect();
        attempts.sort_by(|left, right| {
            left.sent_at
                .cmp(&right.sent_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        attempts
    }

    pub fn audience_report(&self) -> AudienceReport {
        build_audience_report(
            &self.list_audience_members(),
            &self.list_audience_interactions(),
            &self.list_discovery_responses(),
            &self.list_outreach_attempts(),
        )
    }

    pub fn register_capability(
        &mut self,
        request: RegisterCapabilityRequest,
    ) -> AppResult<Capability> {
        if !self.agents.contains_key(&request.agent_id) {
            return Err(AppError::AgentNotFound);
        }

        let capability = Capability {
            id: Uuid::new_v4(),
            agent_id: request.agent_id,
            class: request.class,
            template_slugs: request.template_slugs,
            min_price: Money::new(request.min_price_minor, request.currency.clone())?,
            max_price: Money::new(request.max_price_minor, request.currency)?,
            latency_seconds: request.latency_seconds,
            supported_verifiers: request.supported_verifiers,
        };
        self.capabilities.insert(capability.id, capability.clone());
        Ok(capability)
    }

    pub fn create_help_request(
        &mut self,
        request: CreateHelpRequestRequest,
    ) -> AppResult<HelpRequest> {
        if !self.agents.contains_key(&request.requester_agent_id) {
            return Err(AppError::AgentNotFound);
        }

        let mut help_request = HelpRequest::new(
            request.requester_agent_id,
            request.goal,
            request.context,
            Money::new(request.budget_minor, request.currency)?,
            request.privacy,
        );
        if let Some(confidence) = request.required_confidence {
            help_request.required_confidence = confidence.clamp(0.0, 1.0);
        }
        let risk = self
            .risk_policy
            .evaluate_help_request(&HelpRequestRiskInput {
                goal: help_request.goal.clone(),
                context: help_request.context.clone(),
                budget: help_request.budget.clone(),
                privacy: help_request.privacy.clone(),
            });
        self.enforce_risk(
            risk,
            help_request.id,
            Some(help_request.requester_agent_id),
            None,
        )?;
        self.help_requests
            .insert(help_request.id, help_request.clone());
        Ok(help_request)
    }

    pub fn request_quotes(&mut self, request: RequestQuotesRequest) -> AppResult<QuoteSet> {
        let help_request = self
            .help_requests
            .get(&request.help_request_id)
            .ok_or(AppError::HelpRequestNotFound)?
            .clone();
        let capabilities = self.capabilities.values().cloned().collect::<Vec<_>>();
        let route = BountyRouter::default().route_blocked_goal(&help_request, &capabilities);
        let quotes = capabilities
            .iter()
            .filter(|capability| capability.class == route.capability_class)
            .filter(|capability| capability.min_price.currency == help_request.budget.currency)
            .filter(|capability| capability.min_price.amount <= help_request.budget.amount)
            .map(|capability| {
                let quoted_amount = capability
                    .max_price
                    .amount
                    .min(help_request.budget.amount)
                    .max(capability.min_price.amount);
                Quote {
                    id: Uuid::new_v4(),
                    help_request_id: help_request.id,
                    solver_agent_id: capability.agent_id,
                    price: Money::new(quoted_amount, capability.min_price.currency.clone())
                        .expect("capability prices are valid"),
                    estimated_seconds: capability.latency_seconds,
                    verifier_kind: capability
                        .supported_verifiers
                        .first()
                        .cloned()
                        .unwrap_or(route.verifier_kind.clone()),
                    confidence: route.confidence,
                }
            })
            .collect::<Vec<_>>();

        for quote in &quotes {
            self.quotes.insert(quote.id, quote.clone());
        }

        Ok(QuoteSet {
            help_request,
            route,
            quotes,
        })
    }

    pub fn fund_quote_as_bounty(&mut self, request: FundQuoteRequest) -> AppResult<Bounty> {
        let quote = self
            .quotes
            .get(&request.quote_id)
            .ok_or(AppError::QuoteNotFound)?
            .clone();
        let help_request = self
            .help_requests
            .get(&quote.help_request_id)
            .ok_or(AppError::HelpRequestNotFound)?
            .clone();
        let capabilities = self.capabilities.values().cloned().collect::<Vec<_>>();
        let route = BountyRouter::default().route_blocked_goal(&help_request, &capabilities);
        let template = route
            .template_slug
            .unwrap_or_else(|| template_for_class(&route.capability_class).to_string());

        let mut bounty = self.post_funded_bounty(PostBountyRequest {
            title: request.title.unwrap_or(help_request.goal),
            template_slug: template,
            amount_minor: quote.price.amount,
            currency: quote.price.currency,
            funding_mode: request.funding_mode.unwrap_or(route.funding_mode),
            privacy: help_request.privacy,
        })?;
        bounty.help_request_id = Some(help_request.id);
        self.bounties.insert(bounty.id, bounty.clone());
        Ok(bounty)
    }

    pub fn post_funded_bounty(&mut self, request: PostBountyRequest) -> AppResult<Bounty> {
        let amount = Money::new(request.amount_minor, request.currency)?;
        let funding_mode = request.funding_mode.clone();
        let mut bounty = Bounty::new(
            request.title,
            request.template_slug,
            amount.clone(),
            funding_mode.clone(),
            request.privacy.clone(),
        );
        let risk = self.risk_policy.evaluate_bounty(&BountyRiskInput {
            title: bounty.title.clone(),
            template_slug: bounty.template_slug.clone(),
            amount: amount.clone(),
            funding_mode: funding_mode.clone(),
            privacy: request.privacy,
        });
        self.enforce_risk(risk, bounty.id, None, Some(bounty.id))?;
        let terms_hash = hash_terms(&bounty.title, &bounty.template_slug, &amount);
        if matches!(
            funding_mode,
            FundingMode::BaseUsdcEscrow | FundingMode::MixedRails
        ) {
            return Err(AppError::InvalidFundingContribution(
                "retired Base and mixed-rail modes cannot create new bounties; use autonomous-v1"
                    .to_string(),
            ));
        }
        if funding_mode == FundingMode::StripeFiatLedger {
            bounty.terms_hash = Some(terms_hash);
            self.bounties.insert(bounty.id, bounty.clone());
            return Ok(bounty);
        }

        bounty.mark_funded(terms_hash)?;
        bounty.make_claimable()?;
        let entry = LedgerEntry::new(
            "fund bounty",
            Some(format!("fund:{}", bounty.id)),
            vec![
                debit("escrow_asset", amount.clone()),
                credit("bounty_liability", amount.clone()),
            ],
        )?;
        self.ledger.append(entry.clone())?;

        let contribution = FundingContribution {
            id: Uuid::new_v4(),
            bounty_id: bounty.id,
            contributor_agent_id: None,
            source_organization_id: None,
            rail: payment_rail_for_funding_mode(&funding_mode)?,
            amount,
            status: FundingContributionStatus::Applied,
            funding_ledger_entry_id: Some(entry.id),
            refund_ledger_entry_id: None,
            settlement_id: None,
            external_reference: Some(format!("initial:{}", bounty.id)),
            created_at: Utc::now(),
        };

        self.funding_contributions
            .insert(contribution.id, contribution);
        self.bounties.insert(bounty.id, bounty.clone());
        Ok(bounty)
    }

    pub fn open_pooled_bounty(&mut self, request: OpenPooledBountyRequest) -> AppResult<Bounty> {
        if request.bounty_id.is_some() || request.idempotency_key.is_some() {
            return Err(AppError::InvalidFundingContribution(
                "public pooled bounty creation cannot set bounty_id or idempotency_key; use an operator-gated sync endpoint"
                    .to_string(),
            ));
        }
        self.open_pooled_bounty_internal(request)
    }

    pub fn upsert_github_issue_pooled_bounty(
        &mut self,
        mut request: OpenPooledBountyRequest,
        bounty_id: Id,
        idempotency_key: String,
    ) -> AppResult<Bounty> {
        if !idempotency_key.starts_with("github-issue-sync:") {
            return Err(AppError::InvalidFundingContribution(
                "GitHub issue sync idempotency key must use the github-issue-sync prefix"
                    .to_string(),
            ));
        }
        request.bounty_id = Some(bounty_id);
        request.idempotency_key = Some(idempotency_key);
        self.open_pooled_bounty_internal(request)
    }

    pub fn build_github_issue_pooled_bounty(
        &mut self,
        mut request: OpenPooledBountyRequest,
        bounty_id: Id,
        idempotency_key: String,
    ) -> AppResult<Bounty> {
        if !idempotency_key.starts_with("github-issue-sync:") {
            return Err(AppError::InvalidFundingContribution(
                "GitHub issue sync idempotency key must use the github-issue-sync prefix"
                    .to_string(),
            ));
        }
        request.bounty_id = Some(bounty_id);
        request.idempotency_key = Some(idempotency_key);
        self.build_pooled_bounty(request, None)
    }

    fn open_pooled_bounty_internal(
        &mut self,
        request: OpenPooledBountyRequest,
    ) -> AppResult<Bounty> {
        let requested_bounty_id = request.bounty_id;
        let existing_created_at = if let Some(bounty_id) = requested_bounty_id {
            if let Some(existing) = self.bounties.get(&bounty_id) {
                if self.has_pooled_bounty_activity(bounty_id) {
                    return Err(AppError::InvalidFundingContribution(
                        "pooled bounty idempotent update is only allowed before funding, claim, or submission activity"
                            .to_string(),
                    ));
                }
                Some(existing.created_at)
            } else {
                None
            }
        } else {
            None
        };
        let bounty = self.build_pooled_bounty(request, existing_created_at)?;
        self.bounties.insert(bounty.id, bounty.clone());
        Ok(bounty)
    }

    fn has_pooled_bounty_activity(&self, bounty_id: Id) -> bool {
        self.bounties
            .get(&bounty_id)
            .map(|existing| existing.status != BountyStatus::Unfunded)
            .unwrap_or(false)
            || self
                .funding_intents
                .values()
                .any(|intent| intent.bounty_id == bounty_id)
            || self
                .funding_contributions
                .values()
                .any(|contribution| contribution.bounty_id == bounty_id)
            || self
                .claims
                .values()
                .any(|claim| claim.bounty_id == bounty_id)
            || self
                .submissions
                .values()
                .any(|submission| submission.bounty_id == bounty_id)
    }

    fn build_pooled_bounty(
        &mut self,
        request: OpenPooledBountyRequest,
        existing_created_at: Option<chrono::DateTime<Utc>>,
    ) -> AppResult<Bounty> {
        let amount = Money::new(request.target_amount_minor, request.currency)?;
        let funding_mode = request.funding_mode.clone();
        let funding_targets =
            funding_targets_from_request(&funding_mode, &amount, &request.funding_targets)?;
        let requested_bounty_id = request.bounty_id;
        let mut bounty = Bounty::new(
            request.title,
            request.template_slug,
            amount.clone(),
            funding_mode.clone(),
            request.privacy.clone(),
        )
        .with_funding_targets(funding_targets);
        if let Some(bounty_id) = requested_bounty_id {
            bounty.id = bounty_id;
        }
        if let Some(created_at) = existing_created_at {
            bounty.created_at = created_at;
        }
        let risk = self.risk_policy.evaluate_bounty(&BountyRiskInput {
            title: bounty.title.clone(),
            template_slug: bounty.template_slug.clone(),
            amount: amount.clone(),
            funding_mode,
            privacy: request.privacy,
        });
        self.enforce_risk(risk, bounty.id, None, Some(bounty.id))?;
        bounty.terms_hash = Some(hash_terms(&bounty.title, &bounty.template_slug, &amount));
        Ok(bounty)
    }

    pub fn add_funding_contribution(
        &mut self,
        request: AddFundingContributionRequest,
    ) -> AppResult<PooledFundingReport> {
        let bounty = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if let Some(agent_id) = request.contributor_agent_id {
            if !self.agents.contains_key(&agent_id) {
                return Err(AppError::AgentNotFound);
            }
        }
        if bounty.status != BountyStatus::Unfunded && bounty.status != BountyStatus::Funded {
            return Err(AppError::InvalidFundingContribution(format!(
                "funding contributions are closed after {:?}",
                bounty.status
            )));
        }
        let target =
            self.funding_target_for_contribution(&bounty, &request.rail, &request.currency)?;
        if request.rail == PaymentRail::BaseUsdc {
            return Err(AppError::InvalidFundingContribution(
                "Base USDC funding must be indexed from escrow events, not applied as an off-chain contribution".to_string(),
            ));
        }
        match (request.rail.clone(), request.source_organization_id) {
            (PaymentRail::StripeFiat, None) => {
                return Err(AppError::InvalidFundingContribution(
                    "Stripe fiat bounty funding must name a source organization with verified platform balance".to_string(),
                ));
            }
            (PaymentRail::StripeFiat, Some(_)) => {}
            (_, Some(_)) => {
                return Err(AppError::InvalidFundingContribution(
                    "source organization balance is only valid for Stripe fiat contributions"
                        .to_string(),
                ));
            }
            _ => {}
        }
        let amount = Money::new(request.amount_minor, request.currency)?;
        if amount.currency != target.amount.currency {
            return Err(AppError::InvalidFundingContribution(format!(
                "contribution currency {} does not match {:?} target currency {}",
                amount.currency, target.rail, target.amount.currency
            )));
        }
        if let Some(reference) = request.external_reference.as_deref() {
            if self.funding_contributions.values().any(|contribution| {
                contribution.bounty_id == bounty.id
                    && contribution.external_reference.as_deref() == Some(reference)
                    && contribution.status == FundingContributionStatus::Applied
            }) {
                return Err(AppError::InvalidFundingContribution(format!(
                    "duplicate funding contribution reference: {reference}"
                )));
            }
        }

        let funded_before = self.confirmed_funding_for_target(&bounty, &target);
        let remaining_before = target.amount.amount.saturating_sub(funded_before);
        if remaining_before == 0 {
            return Err(AppError::InvalidFundingContribution(format!(
                "{:?} {} partition is already fully funded",
                target.rail, target.amount.currency
            )));
        }
        if amount.amount > remaining_before {
            return Err(AppError::InvalidFundingContribution(format!(
                "contribution would overfund {:?} {} partition by {} {}",
                target.rail,
                target.amount.currency,
                amount.amount - remaining_before,
                amount.currency
            )));
        }

        let contribution_id = Uuid::new_v4();
        let contributor_agent_id = request.contributor_agent_id;
        let rail = request.rail;
        let external_reference = request.external_reference;
        let source_organization_id = request.source_organization_id;
        let contributor_account = if let Some(organization_id) = source_organization_id {
            let available =
                self.stripe_platform_balance_available_minor(organization_id, &amount.currency);
            if available < amount.amount {
                return Err(AppError::InvalidFundingContribution(format!(
                    "insufficient Stripe platform balance for organization {organization_id}: available {} {}, requested {} {}",
                    available, amount.currency, amount.amount, amount.currency
                )));
            }
            stripe_platform_balance_account(organization_id)
        } else {
            contributor_agent_id
                .map(|id| format!("contributor_funds:{id}"))
                .unwrap_or_else(|| "external_contributor_funds".to_string())
        };
        let entry = LedgerEntry::new(
            "pooled bounty funding contribution",
            Some(format!("fund-contribution:{contribution_id}")),
            vec![
                debit(contributor_account, amount.clone()),
                credit(format!("bounty_liability:{}", bounty.id), amount.clone()),
            ],
        )?;
        self.ledger.append(entry.clone())?;
        let contribution = FundingContribution {
            id: contribution_id,
            bounty_id: bounty.id,
            contributor_agent_id,
            source_organization_id,
            rail,
            amount: amount.clone(),
            status: FundingContributionStatus::Applied,
            funding_ledger_entry_id: Some(entry.id),
            refund_ledger_entry_id: None,
            settlement_id: None,
            external_reference,
            created_at: Utc::now(),
        };
        self.funding_contributions
            .insert(contribution.id, contribution.clone());

        self.mark_bounty_claimable_if_fully_funded(bounty.id)?;

        let bounty = self
            .bounties
            .get(&contribution.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        let funding_summary = self.funding_summary_for_bounty(&bounty);
        Ok(PooledFundingReport {
            bounty,
            contribution,
            funding_summary,
            ledger_entries: vec![entry],
        })
    }

    pub fn create_funding_intent(
        &mut self,
        request: CreateFundingIntentRequest,
        platform_base_url: impl Into<String>,
    ) -> AppResult<FundingIntentReport> {
        let bounty = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if let Some(agent_id) = request.contributor_agent_id {
            if !self.agents.contains_key(&agent_id) {
                return Err(AppError::AgentNotFound);
            }
        }
        if bounty.status != BountyStatus::Unfunded && bounty.status != BountyStatus::Funded {
            return Err(AppError::InvalidFundingIntent(format!(
                "funding intents are closed after {:?}",
                bounty.status
            )));
        }
        if request.rail != PaymentRail::StripeFiat {
            return Err(AppError::InvalidFundingIntent(
                "generic funding intents support Stripe fiat only; use autonomous-v1 for Base USDC"
                    .to_string(),
            ));
        }

        let amount = Money::new(request.amount_minor, request.currency.clone())?;
        let target =
            self.funding_target_for_contribution(&bounty, &request.rail, &amount.currency)?;
        let confirmed_before = self.confirmed_funding_for_target(&bounty, &target);
        let remaining_before = target.amount.amount.saturating_sub(confirmed_before);
        if remaining_before == 0 {
            return Err(AppError::InvalidFundingIntent(format!(
                "{:?} {} partition is already fully funded",
                target.rail, target.amount.currency
            )));
        }
        if amount.amount > remaining_before {
            return Err(AppError::InvalidFundingIntent(format!(
                "funding intent would overfund {:?} {} partition by {} {}",
                target.rail,
                target.amount.currency,
                amount.amount - remaining_before,
                amount.currency
            )));
        }

        let external_reference = request.external_reference.clone().unwrap_or_else(|| {
            funding_intent_reference(
                request.bounty_id,
                &request.rail,
                request.source_organization_id,
                request.contributor_agent_id,
                &amount,
            )
        });
        if self.funding_intents.values().any(|intent| {
            intent.bounty_id == bounty.id
                && intent.external_reference.as_deref() == Some(external_reference.as_str())
                && intent.status != FundingIntentStatus::Rejected
        }) {
            return Err(AppError::InvalidFundingIntent(format!(
                "duplicate funding intent reference: {external_reference}"
            )));
        }

        let intent_id = funding_intent_uuid(bounty.id, &external_reference);
        let stripe_success_url = request.stripe_success_url.clone();
        let stripe_cancel_url = request.stripe_cancel_url.clone();
        let mut intent = FundingIntent {
            id: intent_id,
            bounty_id: bounty.id,
            contributor_agent_id: request.contributor_agent_id,
            source_organization_id: request.source_organization_id,
            rail: request.rail.clone(),
            amount: amount.clone(),
            status: FundingIntentStatus::AwaitingEvidence,
            external_reference: Some(external_reference.clone()),
            stripe_success_url,
            stripe_cancel_url,
            created_at: Utc::now(),
        };

        let platform_base_url = platform_base_url.into();
        let checkout = stripe_checkout_for_funding_intent(
            &bounty,
            &intent,
            &platform_base_url,
            intent.stripe_success_url.clone(),
            intent.stripe_cancel_url.clone(),
        )?;
        let next_action = FundingIntentNextAction::StripeCheckout { request: checkout };
        let reconciliation_hint = "Stripe intent remains pending until a verified paid Checkout webhook credits the source organization and reserves the balance into the bounty.".to_string();

        self.funding_intents.insert(intent.id, intent.clone());
        let funding_summary = self.funding_summary_for_bounty(&bounty);
        Ok(FundingIntentReport {
            bounty,
            intent: {
                intent.status = FundingIntentStatus::AwaitingEvidence;
                intent
            },
            funding_summary,
            next_action,
            requires_reconciliation: true,
            reconciliation_hint,
        })
    }

    pub fn stripe_checkout_for_funding_intent(
        &self,
        funding_intent_id: Id,
        platform_base_url: impl Into<String>,
    ) -> AppResult<StripeRequestIntent> {
        let intent = self
            .funding_intents
            .get(&funding_intent_id)
            .ok_or_else(|| {
                AppError::InvalidFundingIntent(format!(
                    "funding intent not found: {funding_intent_id}"
                ))
            })?;
        let bounty = self
            .bounties
            .get(&intent.bounty_id)
            .ok_or(AppError::BountyNotFound)?;
        stripe_checkout_for_funding_intent(
            bounty,
            intent,
            &platform_base_url.into(),
            intent.stripe_success_url.clone(),
            intent.stripe_cancel_url.clone(),
        )
    }

    pub fn claim_bounty(&mut self, request: ClaimBountyRequest) -> AppResult<Bounty> {
        if !self.agents.contains_key(&request.solver_agent_id) {
            return Err(AppError::AgentNotFound);
        }

        let bounty_snapshot = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if !self.funding_targets_claimable(&bounty_snapshot)? {
            return Err(domain::DomainError::UnfundedBounty.into());
        }

        let bounty = self
            .bounties
            .get_mut(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?;
        bounty.claim()?;
        let claim = Claim {
            id: Uuid::new_v4(),
            bounty_id: request.bounty_id,
            solver_agent_id: request.solver_agent_id,
            claimed_at: Utc::now(),
        };
        self.claims.insert(claim.id, claim);
        Ok(bounty.clone())
    }

    pub fn submit_result(&mut self, request: SubmitResultRequest) -> AppResult<Submission> {
        if !self.agents.contains_key(&request.solver_agent_id) {
            return Err(AppError::AgentNotFound);
        }

        let claimed_solver_agent_id = self.claimed_solver_agent_id(request.bounty_id);
        let risk = self.risk_policy.evaluate_submission(&SubmissionRiskInput {
            bounty_id: request.bounty_id,
            solver_agent_id: request.solver_agent_id,
            claimed_solver_agent_id,
            artifact_uri: request.artifact_uri.clone(),
            artifact_body: request.artifact_body.clone(),
        });
        self.enforce_risk(
            risk,
            request.bounty_id,
            Some(request.solver_agent_id),
            Some(request.bounty_id),
        )?;

        let bounty = self
            .bounties
            .get_mut(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?;
        bounty.submit()?;

        let submission = Submission {
            id: Uuid::new_v4(),
            bounty_id: request.bounty_id,
            solver_agent_id: request.solver_agent_id,
            artifact_digest: hash_artifact(&request.artifact_body),
            artifact_uri: request.artifact_uri,
            submitted_at: Utc::now(),
        };

        self.submissions.insert(submission.id, submission.clone());
        Ok(submission)
    }

    pub async fn verify_submission(
        &mut self,
        request: VerifySubmissionRequest,
    ) -> AppResult<ProofRecord> {
        let submission = self
            .submissions
            .get(&request.submission_id)
            .ok_or(AppError::SubmissionNotFound)?
            .clone();
        if submission.bounty_id != request.bounty_id {
            return Err(AppError::SubmissionBountyMismatch);
        }
        let bounty_snapshot = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        let verifier_kind = request
            .verifier_kind
            .clone()
            .unwrap_or_else(|| verifier_kind_for_template(&bounty_snapshot.template_slug));
        for payout_risk_input in self.payout_risk_inputs_for_bounty(&bounty_snapshot)? {
            let payout_risk = self.risk_policy.evaluate_payout(&payout_risk_input);
            self.enforce_risk_with_optional_approval(
                payout_risk,
                request.bounty_id,
                None,
                Some(request.bounty_id),
                request.approved_risk_event_id,
            )?;
        }

        {
            let bounty = self
                .bounties
                .get_mut(&request.bounty_id)
                .ok_or(AppError::BountyNotFound)?;
            bounty.start_verification()?;
        }

        let result = verify_with_builtin(
            verifier_kind,
            VerificationInput {
                bounty_id: request.bounty_id,
                submission: submission.clone(),
                expected_artifact_digest: Some(request.expected_artifact_digest),
                rubric: request.rubric,
                evidence: request.evidence,
            },
            None,
        )
        .await?;
        if result.decision != VerificationDecision::Accepted {
            let summary = result.summary.clone();
            self.verifier_results.insert(result.id, result);
            return Err(AppError::VerificationNotAccepted(summary));
        }

        {
            let bounty = self
                .bounties
                .get_mut(&request.bounty_id)
                .ok_or(AppError::BountyNotFound)?;
            bounty.accept()?;
        }

        let proof = ProofRecord {
            id: Uuid::new_v4(),
            bounty_id: request.bounty_id,
            submission_id: submission.id,
            verifier_result_id: result.id,
            proof_hash: hash_proof(&submission.artifact_digest, &result.signed_payload_hash),
            public_summary: result.summary.clone(),
            privacy: PrivacyLevel::Public,
            created_at: Utc::now(),
        };

        {
            let bounty = self
                .bounties
                .get_mut(&request.bounty_id)
                .ok_or(AppError::BountyNotFound)?;
            bounty.make_payable(&proof)?;
        }
        let settlements = self.settle_payable_bounty(
            request.bounty_id,
            &proof,
            submission.solver_agent_id,
            result.verifier_agent_id,
        )?;
        self.link_funding_contributions_to_settlements(request.bounty_id, &settlements);
        let reputation_reason = if settlements
            .iter()
            .flat_map(|settlement| &settlement.payout_intents)
            .all(|intent| intent.status == PayoutStatus::Paid)
        {
            "accepted submission settled for payment"
        } else {
            "accepted submission; payout pending eligibility"
        };
        let capability_class = capability_class_for_template(&bounty_snapshot.template_slug);
        let template_slug = bounty_snapshot.template_slug.clone();
        let verifier_kind = result.kind.clone();
        let reputation_event = ReputationEvent {
            id: Uuid::new_v4(),
            agent_id: submission.solver_agent_id,
            bounty_id: request.bounty_id,
            capability_class: capability_class.clone(),
            template_slug: template_slug.clone(),
            delta: 10,
            reason: reputation_reason.to_string(),
            created_at: Utc::now(),
        };
        let template_signal = TemplateSignal {
            id: Uuid::new_v4(),
            bounty_id: request.bounty_id,
            proof_record_id: proof.id,
            template_slug,
            capability_class,
            verifier_kind,
            amount: bounty_snapshot.amount,
            success: true,
            created_at: Utc::now(),
        };

        self.verifier_results.insert(result.id, result);
        self.proofs.insert(proof.id, proof.clone());
        for settlement in settlements {
            self.settlements.insert(settlement.id, settlement);
        }
        self.reputation_events
            .insert(reputation_event.id, reputation_event);
        self.template_signals
            .insert(template_signal.id, template_signal);
        Ok(proof)
    }

    pub fn apply_stripe_connect_snapshot(
        &mut self,
        snapshot: ConnectAccountSnapshot,
    ) -> AppResult<StripeConnectPayoutReconciliation> {
        let payout_state = evaluate_connect_payout(&snapshot);
        let settlement_ids = self
            .settlements
            .values()
            .filter(|settlement| settlement.rail == PaymentRail::StripeFiat)
            .filter(|settlement| {
                settlement.payout_intents.iter().any(|intent| {
                    intent.recipient_agent_id == snapshot.agent_id
                        && intent.status != PayoutStatus::Paid
                })
            })
            .map(|settlement| settlement.id)
            .collect::<Vec<_>>();

        let mut updated_settlement_ids = Vec::new();

        for settlement_id in settlement_ids {
            if payout_state.eligible {
                self.mark_stripe_agent_payouts_pending(settlement_id, snapshot.agent_id)?;
            } else {
                self.mark_stripe_agent_payouts_blocked(settlement_id, snapshot.agent_id)?;
            }
            updated_settlement_ids.push(settlement_id);
        }

        let settlements = updated_settlement_ids
            .into_iter()
            .filter_map(|id| self.settlements.get(&id).cloned())
            .collect();

        Ok(StripeConnectPayoutReconciliation {
            payout_state,
            settlements,
            bounties: Vec::new(),
            ledger_entries: Vec::new(),
        })
    }

    pub fn plan_stripe_transfer(
        &self,
        request: PlanStripeTransferRequest,
        platform_base_url: impl Into<String>,
    ) -> AppResult<StripeTransferPlan> {
        let (settlement, payout_intent) = self
            .settlement_and_payout_intent(request.payout_intent_id)
            .ok_or_else(|| AppError::InvalidStripePayout("payout intent not found".to_string()))?;
        if settlement.rail != PaymentRail::StripeFiat
            || payout_intent.rail != PaymentRail::StripeFiat
        {
            return Err(AppError::InvalidStripePayout(
                "only Stripe fiat payout intents can be transferred through Stripe".to_string(),
            ));
        }
        if payout_intent.status == PayoutStatus::Paid {
            return Err(AppError::InvalidStripePayout(
                "payout intent is already paid".to_string(),
            ));
        }
        if payout_intent.status != PayoutStatus::Pending {
            return Err(AppError::InvalidStripePayout(
                "payout intent must be pending after Connect eligibility reconciliation before transfer planning"
                    .to_string(),
            ));
        }
        let stripe_request = StripePlanner::new(platform_base_url)
            .connect_transfer(&ConnectTransferRequest {
                bounty_id: settlement.bounty_id,
                proof_record_id: settlement.proof_record_id,
                settlement_id: settlement.id,
                payout_intent_id: payout_intent.id,
                agent_id: payout_intent.recipient_agent_id,
                connected_account_id: request.connected_account_id,
                amount: payout_intent.amount.clone(),
            })
            .map_err(|error| AppError::InvalidStripePayout(error.to_string()))?;

        Ok(StripeTransferPlan {
            settlement,
            payout_intent,
            request: stripe_request,
            requires_reconciliation: true,
            reconciliation_hint:
                "Execute the Stripe transfer request in test mode, then reconcile the transfer.created event with matching payout metadata."
                    .to_string(),
        })
    }

    pub fn apply_stripe_transfer_evidence(
        &mut self,
        mut evidence: ConnectTransferEvidence,
    ) -> AppResult<StripeTransferReconciliation> {
        let external_event_id = format!(
            "stripe-connect-transfer:{}:{}",
            evidence.transfer_id, evidence.payout_intent_id
        );
        let duplicate = self.payment_events.values().any(|event| {
            event.external_id == evidence.payment_event.external_id
                && event.status == PaymentEventStatus::Applied
        }) || self.ledger.has_external_event(&external_event_id);
        if duplicate {
            evidence.payment_event.status = PaymentEventStatus::IgnoredDuplicate;
            return Ok(StripeTransferReconciliation {
                evidence,
                duplicate: true,
                settlement: None,
                bounty: None,
                ledger_entries: vec![],
            });
        }

        let settlement = self
            .settlements
            .get(&evidence.settlement_id)
            .ok_or_else(|| AppError::InvalidStripePayout("settlement not found".to_string()))?
            .clone();
        if settlement.bounty_id != evidence.bounty_id
            || settlement.proof_record_id != evidence.proof_record_id
            || settlement.rail != PaymentRail::StripeFiat
        {
            return Err(AppError::InvalidStripePayout(
                "Stripe transfer evidence does not match settlement".to_string(),
            ));
        }
        let payout_intent = settlement
            .payout_intents
            .iter()
            .find(|intent| intent.id == evidence.payout_intent_id)
            .cloned()
            .ok_or_else(|| AppError::InvalidStripePayout("payout intent not found".to_string()))?;
        if payout_intent.recipient_agent_id != evidence.agent_id
            || payout_intent.rail != PaymentRail::StripeFiat
            || payout_intent.amount != evidence.amount
        {
            return Err(AppError::InvalidStripePayout(
                "Stripe transfer evidence does not match payout intent".to_string(),
            ));
        }

        let mut ledger_entries = Vec::new();
        if payout_intent.status != PayoutStatus::Paid {
            let entry = LedgerEntry::new(
                "stripe connect transfer paid",
                Some(external_event_id),
                vec![
                    debit("bounty_liability", payout_intent.amount.clone()),
                    credit(
                        format!("agent_payable:{}", payout_intent.recipient_agent_id),
                        payout_intent.amount.clone(),
                    ),
                ],
            )?;
            self.ledger.append(entry.clone())?;
            ledger_entries.push(entry);
            if let Some(settlement) = self.settlements.get_mut(&evidence.settlement_id) {
                for intent in settlement
                    .payout_intents
                    .iter_mut()
                    .filter(|intent| intent.id == evidence.payout_intent_id)
                {
                    intent.status = PayoutStatus::Paid;
                }
            }
        }

        if let Some(entry) = self.finalize_stripe_settlement_if_complete(evidence.settlement_id)? {
            ledger_entries.push(entry);
        }
        self.payment_events
            .insert(evidence.payment_event.id, evidence.payment_event.clone());
        let settlement = self.settlements.get(&evidence.settlement_id).cloned();
        let bounty = self.bounties.get(&evidence.bounty_id).cloned();

        Ok(StripeTransferReconciliation {
            evidence,
            duplicate: false,
            settlement,
            bounty,
            ledger_entries,
        })
    }

    pub fn apply_stripe_funding_credit(
        &mut self,
        mut funding_credit: StripeFundingCredit,
    ) -> AppResult<StripeFundingReconciliation> {
        let external_event_id = format!(
            "stripe-checkout-top-up:{}",
            funding_credit.payment_event.external_id
        );
        let duplicate = self.payment_events.values().any(|event| {
            event.external_id == funding_credit.payment_event.external_id
                && event.status == PaymentEventStatus::Applied
        }) || self.ledger.has_external_event(&external_event_id);
        if duplicate {
            funding_credit.payment_event.status = PaymentEventStatus::IgnoredDuplicate;
            return Ok(StripeFundingReconciliation {
                funding_credit,
                duplicate: true,
                ledger_entries: vec![],
                funding_intent: None,
                funding_report: None,
            });
        }

        let entry = LedgerEntry::new(
            "stripe checkout top-up",
            Some(external_event_id),
            vec![
                debit(
                    format!("stripe_cash:{}", funding_credit.organization_id),
                    funding_credit.amount.clone(),
                ),
                credit(
                    format!("platform_balance:{}", funding_credit.organization_id),
                    funding_credit.amount.clone(),
                ),
            ],
        )?;
        self.ledger.append(entry.clone())?;
        self.payment_events.insert(
            funding_credit.payment_event.id,
            funding_credit.payment_event.clone(),
        );
        let mut ledger_entries = vec![entry];
        let mut funding_intent = None;
        let funding_report = if let (Some(intent_id), Some(bounty_id)) =
            (funding_credit.funding_intent_id, funding_credit.bounty_id)
        {
            let intent = self
                .funding_intents
                .get(&intent_id)
                .ok_or_else(|| {
                    AppError::InvalidFundingIntent(format!(
                        "Stripe webhook references unknown funding intent {intent_id}"
                    ))
                })?
                .clone();
            if intent.status != FundingIntentStatus::AwaitingEvidence {
                return Err(AppError::InvalidFundingIntent(format!(
                    "funding intent {intent_id} is not awaiting evidence"
                )));
            }
            if intent.bounty_id != bounty_id
                || intent.rail != PaymentRail::StripeFiat
                || intent.amount != funding_credit.amount
                || intent.source_organization_id != Some(funding_credit.organization_id)
            {
                return Err(AppError::InvalidFundingIntent(
                    "Stripe webhook funding intent metadata does not match intent state"
                        .to_string(),
                ));
            }
            let report = self.add_funding_contribution(AddFundingContributionRequest {
                bounty_id,
                contributor_agent_id: intent.contributor_agent_id,
                source_organization_id: Some(funding_credit.organization_id),
                amount_minor: funding_credit.amount.amount,
                currency: funding_credit.amount.currency.clone(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some(format!(
                    "stripe-funding-intent:{intent_id}:{}",
                    funding_credit.checkout_session_id
                )),
            })?;
            ledger_entries.extend(report.ledger_entries.clone());
            if let Some(intent) = self.funding_intents.get_mut(&intent_id) {
                intent.status = FundingIntentStatus::Applied;
                funding_intent = Some(intent.clone());
            }
            Some(report)
        } else {
            None
        };

        Ok(StripeFundingReconciliation {
            funding_credit,
            duplicate: false,
            ledger_entries,
            funding_intent,
            funding_report,
        })
    }

    pub fn status(&self, bounty_id: Id) -> AppResult<BountyStatusResponse> {
        let bounty = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        let funding_summary = self.funding_summary_for_bounty(&bounty);
        let funding_intents = self
            .funding_intents
            .values()
            .filter(|intent| intent.bounty_id == bounty_id)
            .cloned()
            .collect();
        let funding_contributions = self
            .funding_contributions
            .values()
            .filter(|contribution| contribution.bounty_id == bounty_id)
            .cloned()
            .collect();
        let escrows = self
            .escrows
            .values()
            .filter(|escrow| escrow.bounty_id == bounty_id)
            .cloned()
            .collect();
        let claims = self
            .claims
            .values()
            .filter(|claim| claim.bounty_id == bounty_id)
            .cloned()
            .collect();
        let submissions = self
            .submissions
            .values()
            .filter(|submission| submission.bounty_id == bounty_id)
            .cloned()
            .collect();
        let verifier_results = self
            .verifier_results
            .values()
            .filter(|result| result.bounty_id == bounty_id)
            .cloned()
            .collect();
        let proofs = self
            .proofs
            .values()
            .filter(|proof| proof.bounty_id == bounty_id)
            .cloned()
            .collect();
        let settlements = self
            .settlements
            .values()
            .filter(|settlement| settlement.bounty_id == bounty_id)
            .cloned()
            .collect();
        let reputation_events = self
            .reputation_events
            .values()
            .filter(|event| event.bounty_id == bounty_id)
            .cloned()
            .collect();
        let template_signals = self
            .template_signals
            .values()
            .filter(|signal| signal.bounty_id == bounty_id)
            .cloned()
            .collect();
        let risk_events = self
            .risk_events
            .values()
            .filter(|event| event.bounty_id == Some(bounty_id))
            .cloned()
            .collect();

        Ok(BountyStatusResponse {
            bounty,
            funding_summary,
            funding_intents,
            funding_contributions,
            escrows,
            claims,
            submissions,
            verifier_results,
            proofs,
            settlements,
            reputation_events,
            template_signals,
            risk_events,
        })
    }

    pub fn agent_payout_status(&self, agent_id: Id) -> AppResult<AgentPayoutStatusResponse> {
        let agent = self
            .agents
            .get(&agent_id)
            .ok_or(AppError::AgentNotFound)?
            .clone();
        let mut payouts = self
            .settlements
            .values()
            .flat_map(|settlement| {
                settlement
                    .payout_intents
                    .iter()
                    .filter(move |intent| intent.recipient_agent_id == agent_id)
                    .map(move |intent| AgentPayoutLine {
                        settlement_id: settlement.id,
                        bounty_id: settlement.bounty_id,
                        proof_record_id: settlement.proof_record_id,
                        rail: intent.rail.clone(),
                        amount: intent.amount.clone(),
                        status: intent.status.clone(),
                        created_at: settlement.created_at,
                    })
            })
            .collect::<Vec<_>>();
        payouts.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.bounty_id.cmp(&right.bounty_id))
        });

        let mut totals_by_currency: HashMap<String, AgentPayoutTotalsByCurrency> = HashMap::new();
        for payout in &payouts {
            let totals = totals_by_currency
                .entry(payout.amount.currency.clone())
                .or_insert_with(|| AgentPayoutTotalsByCurrency {
                    currency: payout.amount.currency.clone(),
                    ..AgentPayoutTotalsByCurrency::default()
                });
            totals.total_minor += payout.amount.amount;
            match payout.status {
                PayoutStatus::Pending => totals.pending_minor += payout.amount.amount,
                PayoutStatus::Blocked => totals.blocked_minor += payout.amount.amount,
                PayoutStatus::Paying => totals.paying_minor += payout.amount.amount,
                PayoutStatus::Paid => totals.paid_minor += payout.amount.amount,
                PayoutStatus::Failed => totals.failed_minor += payout.amount.amount,
            }
        }
        let mut totals = totals_by_currency.into_values().collect::<Vec<_>>();
        totals.sort_by(|left, right| left.currency.cmp(&right.currency));

        let mut reputation_events = self
            .reputation_events
            .values()
            .filter(|event| event.agent_id == agent_id)
            .cloned()
            .collect::<Vec<_>>();
        reputation_events.sort_by_key(|event| std::cmp::Reverse(event.created_at));

        Ok(AgentPayoutStatusResponse {
            agent,
            payouts,
            totals,
            reputation_events,
        })
    }

    pub fn list_risk_events(&self, filter: RiskEventFilter) -> Vec<RiskEvent> {
        let mut events = self
            .risk_events
            .values()
            .filter(|event| {
                filter
                    .action
                    .map(|action| event.action == action)
                    .unwrap_or(true)
            })
            .filter(|event| {
                filter
                    .surface
                    .map(|surface| event.surface == surface)
                    .unwrap_or(true)
            })
            .filter(|event| {
                filter
                    .bounty_id
                    .map(|bounty_id| event.bounty_id == Some(bounty_id))
                    .unwrap_or(true)
            })
            .filter(|event| {
                filter
                    .agent_id
                    .map(|agent_id| event.agent_id == Some(agent_id))
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|event| std::cmp::Reverse(event.created_at));
        events.truncate(filter.limit.unwrap_or(100).min(500));
        events
    }

    pub fn list_risk_reviews(&self) -> Vec<RiskReviewRecord> {
        let mut reviews = self.risk_reviews.values().cloned().collect::<Vec<_>>();
        reviews.sort_by_key(|review| std::cmp::Reverse(review.created_at));
        reviews
    }

    pub fn approve_risk_bounty(
        &mut self,
        request: ApproveRiskBountyRequest,
    ) -> AppResult<ReviewedBountyApproval> {
        validate_operator_review(&request.operator_id, &request.note)?;
        let event = self
            .risk_events
            .get(&request.risk_event_id)
            .ok_or(AppError::RiskEventNotFound)?
            .clone();
        if self
            .risk_reviews
            .values()
            .any(|review| review.risk_event_id == request.risk_event_id)
        {
            return Err(AppError::RiskAlreadyReviewed);
        }
        if event.action != RiskAction::NeedsReview || event.surface != RiskSurface::Bounty {
            return Err(AppError::InvalidRiskReview(
                "only NeedsReview bounty events can be approved into claimable bounties"
                    .to_string(),
            ));
        }
        if self.bounties.contains_key(&event.subject_id) {
            return Err(AppError::InvalidRiskReview(
                "review subject already exists as a bounty".to_string(),
            ));
        }

        let amount = Money::new(request.amount_minor, request.currency)?;
        let risk = self.risk_policy.evaluate_bounty(&BountyRiskInput {
            title: request.title.clone(),
            template_slug: request.template_slug.clone(),
            amount: amount.clone(),
            funding_mode: request.funding_mode.clone(),
            privacy: request.privacy.clone(),
        });
        if risk.action == RiskAction::Block {
            return Err(AppError::InvalidRiskReview(format!(
                "operator approval cannot bypass blocked bounty policy: {}",
                risk.reasons.join("; ")
            )));
        }

        let mut bounty = Bounty::new(
            request.title,
            request.template_slug,
            amount.clone(),
            request.funding_mode.clone(),
            request.privacy,
        );
        bounty.id = event.subject_id;
        let terms_hash = hash_terms(&bounty.title, &bounty.template_slug, &amount);
        if matches!(
            request.funding_mode,
            FundingMode::BaseUsdcEscrow | FundingMode::MixedRails
        ) {
            return Err(AppError::InvalidRiskReview(
                "retired Base and mixed-rail modes cannot be approved; use autonomous-v1"
                    .to_string(),
            ));
        }
        if request.funding_mode == FundingMode::StripeFiatLedger {
            bounty.terms_hash = Some(terms_hash);
            let review = RiskReviewRecord {
                id: Uuid::new_v4(),
                risk_event_id: event.id,
                subject_id: event.subject_id,
                bounty_id: Some(bounty.id),
                surface: event.surface,
                outcome: RiskReviewOutcome::Approved,
                operator_id: request.operator_id,
                note: request.note,
                created_at: Utc::now(),
            };
            self.bounties.insert(bounty.id, bounty.clone());
            self.risk_reviews.insert(review.id, review.clone());

            return Ok(ReviewedBountyApproval { bounty, review });
        }

        bounty.mark_funded(terms_hash)?;
        bounty.make_claimable()?;

        self.ledger.append(LedgerEntry::new(
            "fund reviewed bounty",
            Some(format!("fund:{}", bounty.id)),
            vec![
                debit("escrow_asset", amount.clone()),
                credit("bounty_liability", amount),
            ],
        )?)?;

        let review = RiskReviewRecord {
            id: Uuid::new_v4(),
            risk_event_id: event.id,
            subject_id: event.subject_id,
            bounty_id: Some(bounty.id),
            surface: event.surface,
            outcome: RiskReviewOutcome::Approved,
            operator_id: request.operator_id,
            note: request.note,
            created_at: Utc::now(),
        };
        self.bounties.insert(bounty.id, bounty.clone());
        self.risk_reviews.insert(review.id, review.clone());

        Ok(ReviewedBountyApproval { bounty, review })
    }

    pub fn approve_risk_payout(
        &mut self,
        request: ApproveRiskPayoutRequest,
    ) -> AppResult<RiskReviewRecord> {
        validate_operator_review(&request.operator_id, &request.note)?;
        let event = self
            .risk_events
            .get(&request.risk_event_id)
            .ok_or(AppError::RiskEventNotFound)?
            .clone();
        if self
            .risk_reviews
            .values()
            .any(|review| review.risk_event_id == request.risk_event_id)
        {
            return Err(AppError::RiskAlreadyReviewed);
        }
        if event.action != RiskAction::NeedsReview || event.surface != RiskSurface::Payout {
            return Err(AppError::InvalidRiskReview(
                "only NeedsReview payout events can approve verification payout risk".to_string(),
            ));
        }
        let bounty_id = event.bounty_id.ok_or_else(|| {
            AppError::InvalidRiskReview("payout review event must reference a bounty".to_string())
        })?;
        if !self.bounties.contains_key(&bounty_id) {
            return Err(AppError::InvalidRiskReview(
                "payout review bounty does not exist".to_string(),
            ));
        }

        let review = RiskReviewRecord {
            id: Uuid::new_v4(),
            risk_event_id: event.id,
            subject_id: event.subject_id,
            bounty_id: Some(bounty_id),
            surface: event.surface,
            outcome: RiskReviewOutcome::Approved,
            operator_id: request.operator_id,
            note: request.note,
            created_at: Utc::now(),
        };
        self.risk_reviews.insert(review.id, review.clone());
        Ok(review)
    }

    pub fn reject_risk_event(
        &mut self,
        request: RejectRiskEventRequest,
    ) -> AppResult<RiskReviewRecord> {
        validate_operator_review(&request.operator_id, &request.note)?;
        let event = self
            .risk_events
            .get(&request.risk_event_id)
            .ok_or(AppError::RiskEventNotFound)?
            .clone();
        if self
            .risk_reviews
            .values()
            .any(|review| review.risk_event_id == request.risk_event_id)
        {
            return Err(AppError::RiskAlreadyReviewed);
        }
        if event.action != RiskAction::NeedsReview {
            return Err(AppError::InvalidRiskReview(
                "only NeedsReview events can be rejected from the review queue".to_string(),
            ));
        }
        let review = RiskReviewRecord {
            id: Uuid::new_v4(),
            risk_event_id: event.id,
            subject_id: event.subject_id,
            bounty_id: event.bounty_id,
            surface: event.surface,
            outcome: RiskReviewOutcome::Rejected,
            operator_id: request.operator_id,
            note: request.note,
            created_at: Utc::now(),
        };
        self.risk_reviews.insert(review.id, review.clone());
        Ok(review)
    }

    pub fn list_claimable_bounties(&self) -> Vec<Bounty> {
        self.bounties
            .values()
            .filter(|bounty| self.is_claimable_with_confirmed_funding(bounty))
            .cloned()
            .collect()
    }

    fn settle_payable_bounty(
        &mut self,
        bounty_id: Id,
        proof: &ProofRecord,
        solver_agent_id: Id,
        _verifier_agent_id: Option<Id>,
    ) -> AppResult<Vec<Settlement>> {
        let bounty = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?;
        if bounty.status != BountyStatus::Payable {
            return Err(domain::DomainError::InvalidTransition {
                from: format!("{:?}", bounty.status),
                to: "Paid".to_string(),
            }
            .into());
        }
        let targets = self.settlement_targets_for_bounty(bounty)?;

        let mut settlements = Vec::new();
        let mut all_payouts_paid = true;
        for target in targets {
            let amount = target.amount.clone();
            let rail = target.rail.clone();
            // Until a split is disclosed and terms-hashed before funding, the
            // advertised bounty amount is the solver's net payout.
            let solver_amount = amount.clone();
            let platform_amount = Money::zero(amount.currency.clone());

            let payout_status = match rail {
                PaymentRail::StripeFiat => PayoutStatus::Blocked,
                PaymentRail::BaseUsdc => PayoutStatus::Pending,
                PaymentRail::Simulated => PayoutStatus::Paid,
            };
            all_payouts_paid &= payout_status == PayoutStatus::Paid;

            let payout_intents = vec![PayoutIntent {
                id: Uuid::new_v4(),
                bounty_id,
                recipient_agent_id: solver_agent_id,
                rail: rail.clone(),
                amount: solver_amount.clone(),
                status: payout_status.clone(),
            }];
            let mut postings = Vec::new();
            if payout_status == PayoutStatus::Paid {
                postings.push(debit("bounty_liability", amount.clone()));
                postings.push(credit(
                    format!("agent_payable:{solver_agent_id}"),
                    solver_amount,
                ));
            }
            if payout_status == PayoutStatus::Paid && platform_amount.amount > 0 {
                postings.push(credit("platform_fee", platform_amount.clone()));
            }
            if payout_status == PayoutStatus::Paid {
                let external_event = if settlements.is_empty()
                    && self
                        .bounties
                        .get(&bounty_id)
                        .map(|bounty| bounty.funding_mode != FundingMode::MixedRails)
                        .unwrap_or(false)
                {
                    format!("settle:{bounty_id}")
                } else {
                    format!("settle:{bounty_id}:{:?}:{}", rail, amount.currency)
                };
                self.ledger.append(LedgerEntry::new(
                    "settle bounty",
                    Some(external_event),
                    postings,
                )?)?;
            }

            settlements.push(Settlement {
                id: Uuid::new_v4(),
                bounty_id,
                proof_record_id: proof.id,
                rail,
                payout_intents,
                platform_fee: platform_amount,
                created_at: Utc::now(),
            });
        }

        if all_payouts_paid {
            self.bounties
                .get_mut(&bounty_id)
                .ok_or(AppError::BountyNotFound)?
                .mark_paid()?;
        }
        Ok(settlements)
    }

    fn link_funding_contributions_to_settlements(
        &mut self,
        bounty_id: Id,
        settlements: &[Settlement],
    ) {
        for contribution in self
            .funding_contributions
            .values_mut()
            .filter(|contribution| contribution.bounty_id == bounty_id)
        {
            if contribution.status == FundingContributionStatus::Applied {
                contribution.settlement_id = settlements
                    .iter()
                    .find(|settlement| {
                        settlement.rail == contribution.rail
                            && settlement.platform_fee.currency == contribution.amount.currency
                    })
                    .map(|settlement| settlement.id);
            }
        }
    }

    fn mark_stripe_agent_payouts_pending(
        &mut self,
        settlement_id: Id,
        agent_id: Id,
    ) -> AppResult<()> {
        let settlement = self
            .settlements
            .get_mut(&settlement_id)
            .ok_or_else(|| AppError::InvalidStripePayout("settlement not found".to_string()))?;
        for intent in settlement
            .payout_intents
            .iter_mut()
            .filter(|intent| intent.recipient_agent_id == agent_id)
            .filter(|intent| intent.status != PayoutStatus::Paid)
        {
            intent.status = PayoutStatus::Pending;
        }
        Ok(())
    }

    fn mark_stripe_agent_payouts_blocked(
        &mut self,
        settlement_id: Id,
        agent_id: Id,
    ) -> AppResult<()> {
        let settlement = self
            .settlements
            .get_mut(&settlement_id)
            .ok_or_else(|| AppError::InvalidStripePayout("settlement not found".to_string()))?;
        for intent in settlement
            .payout_intents
            .iter_mut()
            .filter(|intent| intent.recipient_agent_id == agent_id)
            .filter(|intent| intent.status != PayoutStatus::Paid)
        {
            intent.status = PayoutStatus::Blocked;
        }
        Ok(())
    }

    fn finalize_stripe_settlement_if_complete(
        &mut self,
        settlement_id: Id,
    ) -> AppResult<Option<LedgerEntry>> {
        let settlement = self
            .settlements
            .get(&settlement_id)
            .ok_or_else(|| AppError::InvalidStripePayout("settlement not found".to_string()))?
            .clone();
        if settlement.rail != PaymentRail::StripeFiat
            || settlement
                .payout_intents
                .iter()
                .any(|intent| intent.status != PayoutStatus::Paid)
        {
            return Ok(None);
        }
        let mut fee_entry = None;
        if settlement.platform_fee.amount > 0 {
            let external_event_id = format!("stripe-platform-fee:{settlement_id}");
            if self.ledger.has_external_event(&external_event_id) {
                return Ok(None);
            }

            let entry = LedgerEntry::new(
                "stripe platform fee recognized",
                Some(external_event_id),
                vec![
                    debit("bounty_liability", settlement.platform_fee.clone()),
                    credit("platform_fee", settlement.platform_fee.clone()),
                ],
            )?;
            self.ledger.append(entry.clone())?;
            fee_entry = Some(entry);
        }
        let should_mark_paid = self
            .bounties
            .get(&settlement.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .status
            == BountyStatus::Payable;
        if should_mark_paid {
            self.mark_bounty_paid_if_all_settlements_paid(settlement.bounty_id)?;
        }
        Ok(fee_entry)
    }

    fn settlement_and_payout_intent(
        &self,
        payout_intent_id: Id,
    ) -> Option<(Settlement, PayoutIntent)> {
        self.settlements.values().find_map(|settlement| {
            settlement
                .payout_intents
                .iter()
                .find(|intent| intent.id == payout_intent_id)
                .cloned()
                .map(|intent| (settlement.clone(), intent))
        })
    }

    fn claimed_solver_agent_id(&self, bounty_id: Id) -> Option<Id> {
        self.claims
            .values()
            .find(|claim| claim.bounty_id == bounty_id)
            .map(|claim| claim.solver_agent_id)
    }

    fn enforce_risk(
        &mut self,
        assessment: RiskAssessment,
        subject_id: Id,
        agent_id: Option<Id>,
        bounty_id: Option<Id>,
    ) -> AppResult<()> {
        if assessment.is_allowed() {
            return Ok(());
        }

        let reasons = assessment.reasons.join("; ");
        let event = RiskEvent {
            id: Uuid::new_v4(),
            subject_id,
            agent_id,
            bounty_id,
            surface: assessment.surface,
            action: assessment.action,
            score: assessment.score,
            reasons: assessment.reasons,
            created_at: Utc::now(),
        };
        self.risk_events.insert(event.id, event);

        match assessment.action {
            RiskAction::Allow => Ok(()),
            RiskAction::NeedsReview => Err(AppError::RiskNeedsReview(reasons)),
            RiskAction::Block => Err(AppError::RiskBlocked(reasons)),
        }
    }

    fn enforce_risk_with_optional_approval(
        &mut self,
        assessment: RiskAssessment,
        subject_id: Id,
        agent_id: Option<Id>,
        bounty_id: Option<Id>,
        approved_risk_event_id: Option<Id>,
    ) -> AppResult<()> {
        if assessment.is_allowed() {
            return Ok(());
        }
        if let Some(risk_event_id) = approved_risk_event_id {
            return self.accept_approved_risk_event(
                &assessment,
                subject_id,
                agent_id,
                bounty_id,
                risk_event_id,
            );
        }
        self.enforce_risk(assessment, subject_id, agent_id, bounty_id)
    }

    fn accept_approved_risk_event(
        &self,
        assessment: &RiskAssessment,
        subject_id: Id,
        agent_id: Option<Id>,
        bounty_id: Option<Id>,
        risk_event_id: Id,
    ) -> AppResult<()> {
        if assessment.action == RiskAction::Block {
            return Err(AppError::InvalidRiskReview(
                "operator approval cannot bypass blocked risk policy".to_string(),
            ));
        }
        let event = self
            .risk_events
            .get(&risk_event_id)
            .ok_or(AppError::RiskEventNotFound)?;
        if event.action != RiskAction::NeedsReview
            || event.surface != assessment.surface
            || event.subject_id != subject_id
            || event.agent_id != agent_id
            || event.bounty_id != bounty_id
        {
            return Err(AppError::InvalidRiskReview(
                "approved risk event does not match the current risk assessment".to_string(),
            ));
        }
        let review = self
            .risk_reviews
            .values()
            .find(|review| review.risk_event_id == risk_event_id)
            .ok_or_else(|| {
                AppError::InvalidRiskReview("risk event has not been reviewed".to_string())
            })?;
        if review.outcome != RiskReviewOutcome::Approved
            || review.surface != event.surface
            || review.subject_id != event.subject_id
            || review.bounty_id != event.bounty_id
        {
            return Err(AppError::InvalidRiskReview(
                "risk event review is not an approval for this subject".to_string(),
            ));
        }
        Ok(())
    }
}

fn validate_operator_review(operator_id: &str, note: &str) -> AppResult<()> {
    if operator_id.trim().is_empty() {
        return Err(AppError::InvalidRiskReview(
            "operator_id is required".to_string(),
        ));
    }
    if note.trim().is_empty() {
        return Err(AppError::InvalidRiskReview(
            "review note is required".to_string(),
        ));
    }
    Ok(())
}

pub fn hash_artifact(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hex::encode(hasher.finalize())
}

fn hash_terms(title: &str, template_slug: &str, amount: &Money) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{}:{}:{}:{}",
        title, template_slug, amount.amount, amount.currency
    ));
    hex::encode(hasher.finalize())
}

fn hash_proof(artifact_digest: &str, verifier_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{artifact_digest}:{verifier_hash}"));
    hex::encode(hasher.finalize())
}

fn payment_rail_for_funding_mode(funding_mode: &FundingMode) -> AppResult<PaymentRail> {
    match funding_mode {
        FundingMode::Simulated => Ok(PaymentRail::Simulated),
        FundingMode::StripeFiatLedger => Ok(PaymentRail::StripeFiat),
        FundingMode::BaseUsdcEscrow | FundingMode::MixedRails => {
            Err(AppError::InvalidFundingContribution(
                "retired Base and mixed-rail modes cannot create funding records; use autonomous-v1"
                    .to_string(),
            ))
        }
    }
}

fn funding_targets_from_request(
    funding_mode: &FundingMode,
    _amount: &Money,
    requested_targets: &[FundingPartitionTargetRequest],
) -> AppResult<Vec<FundingPartitionTarget>> {
    if matches!(
        funding_mode,
        FundingMode::BaseUsdcEscrow | FundingMode::MixedRails
    ) {
        return Err(AppError::InvalidFundingContribution(
            "retired Base and mixed-rail pooled bounties cannot be opened; use autonomous-v1"
                .to_string(),
        ));
    }
    if !requested_targets.is_empty() {
        return Err(AppError::InvalidFundingContribution(
            "funding_targets are retired; use autonomous-v1 pooled funding".to_string(),
        ));
    }
    Ok(Vec::new())
}

#[cfg(test)]
fn settlement_total_amount(settlement: &Settlement) -> AppResult<Money> {
    let currency = settlement.platform_fee.currency.clone();
    let payout_total = settlement
        .payout_intents
        .iter()
        .try_fold(0_i64, |total, intent| {
            if intent.amount.currency != currency {
                return Err(AppError::InvalidFundingContribution(
                    "settlement payout currencies do not match platform fee currency".to_string(),
                ));
            }
            Ok(total + intent.amount.amount)
        })?;
    Money::new(payout_total + settlement.platform_fee.amount, currency).map_err(AppError::from)
}

fn capability_class_for_template(template_slug: &str) -> CapabilityClass {
    match template_slug {
        "extract-data-to-schema" => CapabilityClass::Extraction,
        "independent-claim-verification" => CapabilityClass::Verification,
        "primary-source-research" => CapabilityClass::Research,
        "write-docs-for-area" => CapabilityClass::Documentation,
        "docs-and-cli-report" => CapabilityClass::Documentation,
        "fix-ci-failure" => CapabilityClass::Ci,
        "run-browser-workflow" => CapabilityClass::BrowserWorkflow,
        _ => CapabilityClass::Coding,
    }
}

fn verifier_kind_for_template(template_slug: &str) -> VerifierKind {
    match template_slug {
        "fix-ci-failure"
        | "small-code-change"
        | "payment-state-machine"
        | "small-web-public-change"
        | "docs-and-cli-report" => VerifierKind::GitHubCi,
        "extract-data-to-schema" => VerifierKind::JsonSchema,
        "run-browser-workflow" => VerifierKind::DockerCommand,
        "write-docs-for-area" => VerifierKind::AiJudgeFilter,
        "independent-claim-verification" | "primary-source-research" => VerifierKind::Manual,
        _ => VerifierKind::Manual,
    }
}

impl BountyNetwork {
    pub fn funding_summary(&self, bounty_id: Id) -> AppResult<PooledFundingSummary> {
        let bounty = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?;
        Ok(self.funding_summary_for_bounty(bounty))
    }

    fn effective_funding_targets(&self, bounty: &Bounty) -> AppResult<Vec<FundingPartitionTarget>> {
        if !bounty.funding_targets.is_empty() {
            return Ok(bounty.funding_targets.clone());
        }
        if bounty.funding_mode == FundingMode::MixedRails {
            return Err(AppError::InvalidFundingContribution(
                "mixed rail bounty is missing explicit funding targets".to_string(),
            ));
        }
        Ok(vec![FundingPartitionTarget {
            rail: payment_rail_for_funding_mode(&bounty.funding_mode)?,
            amount: bounty.amount.clone(),
        }])
    }

    fn funding_target_for_contribution(
        &self,
        bounty: &Bounty,
        rail: &PaymentRail,
        currency: &str,
    ) -> AppResult<FundingPartitionTarget> {
        self.effective_funding_targets(bounty)?
            .into_iter()
            .find(|target| {
                target.rail == *rail && target.amount.currency == currency.to_lowercase()
            })
            .ok_or_else(|| {
                AppError::InvalidFundingContribution(format!(
                    "bounty has no {:?} {} funding target",
                    rail,
                    currency.to_lowercase()
                ))
            })
    }

    fn settlement_targets_for_bounty(
        &self,
        bounty: &Bounty,
    ) -> AppResult<Vec<FundingPartitionTarget>> {
        let targets = self.effective_funding_targets(bounty)?;
        let funded_targets = targets
            .into_iter()
            .filter(|target| {
                self.confirmed_funding_for_target(bounty, target) >= target.amount.amount
            })
            .collect::<Vec<_>>();
        if funded_targets.is_empty() {
            return Err(AppError::InvalidFundingContribution(
                "bounty has no confirmed funding partitions to settle".to_string(),
            ));
        }
        Ok(funded_targets)
    }

    fn payout_risk_inputs_for_bounty(&self, bounty: &Bounty) -> AppResult<Vec<PayoutRiskInput>> {
        Ok(self
            .settlement_targets_for_bounty(bounty)?
            .into_iter()
            .map(|target| PayoutRiskInput {
                bounty_id: bounty.id,
                rail: target.rail,
                amount: target.amount,
            })
            .collect())
    }

    fn is_claimable_with_confirmed_funding(&self, bounty: &Bounty) -> bool {
        if bounty.status != BountyStatus::Claimable {
            return false;
        }
        self.funding_targets_claimable(bounty).unwrap_or(false)
    }

    fn funding_summary_for_bounty(&self, bounty: &Bounty) -> PooledFundingSummary {
        let applied_amount = self.applied_funding_amount(bounty);
        let remaining_amount = bounty.amount.amount.saturating_sub(applied_amount);
        let partitions = self.funding_partition_summaries(bounty);
        PooledFundingSummary {
            bounty_id: bounty.id,
            target: bounty.amount.clone(),
            applied: Money {
                amount: applied_amount.max(0),
                currency: bounty.amount.currency.clone(),
            },
            remaining: Money {
                amount: remaining_amount,
                currency: bounty.amount.currency.clone(),
            },
            contribution_count: self
                .funding_contributions
                .values()
                .filter(|contribution| contribution.bounty_id == bounty.id)
                .filter(|contribution| contribution.status == FundingContributionStatus::Applied)
                .count(),
            partitions,
            claimable: self.is_claimable_with_confirmed_funding(bounty),
        }
    }

    fn applied_funding_amount(&self, bounty: &Bounty) -> i64 {
        self.confirmed_funding_in_currency(bounty, &bounty.amount.currency)
    }

    fn funding_targets_claimable(&self, bounty: &Bounty) -> AppResult<bool> {
        Ok(self
            .effective_funding_targets(bounty)?
            .iter()
            .all(|target| {
                self.confirmed_funding_for_target(bounty, target) >= target.amount.amount
            }))
    }

    fn mark_bounty_claimable_if_fully_funded(&mut self, bounty_id: Id) -> AppResult<()> {
        let bounty_snapshot = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if !self.funding_targets_claimable(&bounty_snapshot)? {
            return Ok(());
        }
        let bounty = self
            .bounties
            .get_mut(&bounty_id)
            .ok_or(AppError::BountyNotFound)?;
        match bounty.status {
            BountyStatus::Unfunded => {
                let terms_hash = bounty.terms_hash.clone().unwrap_or_else(|| {
                    hash_terms(&bounty.title, &bounty.template_slug, &bounty.amount)
                });
                bounty.mark_funded(terms_hash)?;
                bounty.make_claimable()?;
            }
            BountyStatus::Funded => bounty.make_claimable()?,
            BountyStatus::Claimable
            | BountyStatus::Claimed
            | BountyStatus::Submitted
            | BountyStatus::Verifying
            | BountyStatus::Accepted
            | BountyStatus::Payable => {}
            BountyStatus::Paid
            | BountyStatus::Refunding
            | BountyStatus::Refunded
            | BountyStatus::Disputed
            | BountyStatus::Expired => {
                return Err(domain::DomainError::InvalidTransition {
                    from: format!("{:?}", bounty.status),
                    to: "Claimable".to_string(),
                }
                .into());
            }
        }
        Ok(())
    }

    fn mark_bounty_paid_if_all_settlements_paid(&mut self, bounty_id: Id) -> AppResult<()> {
        let all_paid = self
            .settlements
            .values()
            .filter(|settlement| settlement.bounty_id == bounty_id)
            .flat_map(|settlement| &settlement.payout_intents)
            .all(|intent| intent.status == PayoutStatus::Paid);
        if all_paid {
            let bounty = self
                .bounties
                .get_mut(&bounty_id)
                .ok_or(AppError::BountyNotFound)?;
            if bounty.status == BountyStatus::Payable {
                bounty.mark_paid()?;
            }
        }
        Ok(())
    }

    fn funding_partition_summaries(&self, bounty: &Bounty) -> Vec<FundingPartitionSummary> {
        self.effective_funding_targets(bounty)
            .unwrap_or_default()
            .into_iter()
            .map(|target| {
                let confirmed = self.confirmed_funding_for_target(bounty, &target);
                let remaining = target.amount.amount.saturating_sub(confirmed);
                FundingPartitionSummary {
                    rail: target.rail.clone(),
                    target: target.amount.clone(),
                    confirmed: Money {
                        amount: confirmed,
                        currency: target.amount.currency.clone(),
                    },
                    remaining: Money {
                        amount: remaining,
                        currency: target.amount.currency.clone(),
                    },
                    contribution_count: self
                        .funding_contributions
                        .values()
                        .filter(|contribution| contribution.bounty_id == bounty.id)
                        .filter(|contribution| {
                            contribution.status == FundingContributionStatus::Applied
                        })
                        .filter(|contribution| contribution.rail == target.rail)
                        .filter(|contribution| {
                            contribution.amount.currency == target.amount.currency
                        })
                        .count(),
                    escrow_count: self
                        .escrows
                        .values()
                        .filter(|escrow| escrow.bounty_id == bounty.id)
                        .filter(|escrow| escrow.rail == target.rail)
                        .filter(|escrow| escrow.amount.currency == target.amount.currency)
                        .filter(|escrow| {
                            matches!(escrow.status, EscrowStatus::Funded | EscrowStatus::Released)
                        })
                        .count(),
                    claimable: confirmed >= target.amount.amount,
                }
            })
            .collect()
    }

    fn confirmed_funding_for_target(
        &self,
        bounty: &Bounty,
        target: &FundingPartitionTarget,
    ) -> i64 {
        self.confirmed_funding_for_rail_currency(bounty.id, &target.rail, &target.amount.currency)
    }

    fn confirmed_funding_in_currency(&self, bounty: &Bounty, currency: &str) -> i64 {
        self.funding_partition_summaries(bounty)
            .into_iter()
            .filter(|partition| partition.target.currency == currency)
            .map(|partition| partition.confirmed.amount)
            .sum()
    }

    fn confirmed_funding_for_rail_currency(
        &self,
        bounty_id: Id,
        rail: &PaymentRail,
        currency: &str,
    ) -> i64 {
        let contribution_total = self
            .funding_contributions
            .values()
            .filter(|contribution| contribution.bounty_id == bounty_id)
            .filter(|contribution| contribution.status == FundingContributionStatus::Applied)
            .filter(|contribution| contribution.rail == *rail)
            .filter(|contribution| contribution.amount.currency == currency)
            .map(|contribution| contribution.amount.amount)
            .sum::<i64>();
        let escrow_total = self
            .escrows
            .values()
            .filter(|escrow| escrow.bounty_id == bounty_id)
            .filter(|escrow| escrow.rail == *rail)
            .filter(|escrow| escrow.amount.currency == currency)
            .filter(|escrow| matches!(escrow.status, EscrowStatus::Funded | EscrowStatus::Released))
            .map(|escrow| escrow.amount.amount)
            .sum::<i64>();
        contribution_total + escrow_total
    }

    fn stripe_platform_balance_available_minor(&self, organization_id: Id, currency: &str) -> i64 {
        let balance = self.ledger.balance(
            &AccountCode::new(stripe_platform_balance_account(organization_id)),
            currency,
        );
        (-balance).max(0)
    }
}

fn stripe_platform_balance_account(organization_id: Id) -> String {
    format!("platform_balance:{organization_id}")
}

fn funding_intent_uuid(bounty_id: Id, external_reference: &str) -> Id {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("agent-bounties:funding-intent:{bounty_id}:{external_reference}").as_bytes(),
    )
}

fn funding_intent_reference(
    bounty_id: Id,
    rail: &PaymentRail,
    source_organization_id: Option<Id>,
    contributor_agent_id: Option<Id>,
    amount: &Money,
) -> String {
    let contributor = source_organization_id
        .map(|id| format!("org:{id}"))
        .or_else(|| contributor_agent_id.map(|id| format!("agent:{id}")))
        .unwrap_or_else(|| "anonymous".to_string());
    format!(
        "funding-intent:{bounty_id}:{rail:?}:{contributor}:{}:{}",
        amount.currency, amount.amount
    )
}

fn stripe_checkout_for_funding_intent(
    bounty: &Bounty,
    intent: &FundingIntent,
    platform_base_url: &str,
    success_url: Option<String>,
    cancel_url: Option<String>,
) -> AppResult<StripeRequestIntent> {
    if intent.bounty_id != bounty.id {
        return Err(AppError::InvalidFundingIntent(
            "funding intent does not belong to bounty".to_string(),
        ));
    }
    if intent.rail != PaymentRail::StripeFiat {
        return Err(AppError::InvalidFundingIntent(
            "only Stripe fiat funding intents can create Checkout Sessions".to_string(),
        ));
    }
    if intent.status != FundingIntentStatus::AwaitingEvidence {
        return Err(AppError::InvalidFundingIntent(format!(
            "funding intent is not awaiting evidence: {:?}",
            intent.status
        )));
    }
    let organization_id = intent.source_organization_id.ok_or_else(|| {
        AppError::InvalidFundingIntent(
            "Stripe fiat funding intents require source_organization_id".to_string(),
        )
    })?;
    let platform_base_url = platform_base_url.trim_end_matches('/');
    let mut checkout = StripePlanner::new(platform_base_url)
        .checkout_top_up(&CheckoutTopUpRequest {
            organization_id,
            amount: intent.amount.clone(),
            success_url: success_url
                .unwrap_or_else(|| format!("{platform_base_url}/stripe/success")),
            cancel_url: cancel_url.unwrap_or_else(|| format!("{platform_base_url}/stripe/cancel")),
        })
        .map_err(|error| AppError::InvalidFundingIntent(error.to_string()))?;
    checkout.idempotency_key = format!("bounty_funding_intent:{}", intent.id);
    if let Some(metadata) = checkout
        .body
        .get_mut("metadata")
        .and_then(serde_json::Value::as_object_mut)
    {
        metadata.insert("bounty_id".to_string(), serde_json::json!(bounty.id));
        metadata.insert(
            "funding_intent_id".to_string(),
            serde_json::json!(intent.id),
        );
        metadata.insert(
            "funding_intent_reference".to_string(),
            serde_json::json!(intent.external_reference.clone()),
        );
        metadata.insert(
            "purpose".to_string(),
            serde_json::json!("bounty_funding_intent"),
        );
    }
    Ok(checkout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github_sync_pooled_request(title: &str, amount_minor: i64) -> OpenPooledBountyRequest {
        OpenPooledBountyRequest {
            bounty_id: None,
            idempotency_key: None,
            title: title.to_string(),
            template_slug: "write-docs-for-area".to_string(),
            target_amount_minor: amount_minor,
            currency: "usdc".to_string(),
            funding_mode: FundingMode::Simulated,
            privacy: PrivacyLevel::Public,
            funding_targets: vec![],
        }
    }

    fn github_sync_idempotency_key(bounty_id: Id) -> String {
        format!("github-issue-sync:agent-bounties/example:{bounty_id}")
    }

    #[test]
    fn contributor_contact_requires_consent_for_private_fields() {
        let mut network = BountyNetwork::default();

        let email_error =
            network
                .upsert_contributor_contact(UpsertContributorContactRequest {
                    github_login: "qilu13".to_string(),
                    email: Some("qilu13@example.com".to_string()),
                    payout_wallet: None,
                    associated_prs: vec![
                        "https://github.com/NSPG13/agent-bounties/pull/24".to_string()
                    ],
                    contact_consent: false,
                    wallet_consent: false,
                    outreach_allowed: false,
                    source: None,
                    notes: None,
                })
                .unwrap_err();
        assert!(matches!(
            email_error,
            AppError::InvalidContributorContact(message)
                if message.contains("contact_consent")
        ));

        let wallet_error = network
            .upsert_contributor_contact(UpsertContributorContactRequest {
                github_login: "qilu13".to_string(),
                email: None,
                payout_wallet: Some("0x1111111111111111111111111111111111111111".to_string()),
                associated_prs: vec![],
                contact_consent: false,
                wallet_consent: false,
                outreach_allowed: false,
                source: None,
                notes: None,
            })
            .unwrap_err();
        assert!(matches!(
            wallet_error,
            AppError::InvalidContributorContact(message)
                if message.contains("wallet_consent")
        ));
    }

    #[test]
    fn contributor_contact_upsert_merges_prs_by_github_login() {
        let mut network = BountyNetwork::default();
        let first = network
            .upsert_contributor_contact(UpsertContributorContactRequest {
                github_login: "@Qilu13".to_string(),
                email: None,
                payout_wallet: None,
                associated_prs: vec!["#24".to_string()],
                contact_consent: false,
                wallet_consent: false,
                outreach_allowed: false,
                source: Some("github-pr-history".to_string()),
                notes: None,
            })
            .unwrap();
        let second = network
            .upsert_contributor_contact(UpsertContributorContactRequest {
                github_login: "qilu13".to_string(),
                email: None,
                payout_wallet: Some("0x1111111111111111111111111111111111111111".to_string()),
                associated_prs: vec!["#59".to_string(), "#24".to_string()],
                contact_consent: false,
                wallet_consent: true,
                outreach_allowed: false,
                source: Some("github-comment-opt-in".to_string()),
                notes: Some("Base-compatible wallet supplied publicly.".to_string()),
            })
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.github_login, "qilu13");
        assert_eq!(
            second.associated_prs,
            vec!["#59".to_string(), "#24".to_string()]
        );
        assert!(second.wallet_consent);
        assert_eq!(network.list_contributor_contacts().len(), 1);
    }

    #[test]
    fn audience_registry_is_idempotent_and_reports_conversion_gaps() {
        let mut network = BountyNetwork::default();
        let member = network
            .upsert_audience_member(UpsertAudienceMemberRequest {
                provider: AudienceProvider::Github,
                external_id: "U_123".to_string(),
                handle: "nexicturbo".to_string(),
                public_profile_url: Some("https://github.com/nexicturbo".to_string()),
                roles: vec![],
                observed_at: None,
            })
            .unwrap();
        let same_member = network
            .upsert_audience_member(UpsertAudienceMemberRequest {
                provider: AudienceProvider::Github,
                external_id: "u_123".to_string(),
                handle: "NexicTurbo".to_string(),
                public_profile_url: None,
                roles: vec![AudienceRole::Contributor],
                observed_at: None,
            })
            .unwrap();
        assert_eq!(member.id, same_member.id);

        let interaction_request = RecordAudienceInteractionRequest {
            audience_member_id: member.id,
            provider_event_id: "pull-request:128".to_string(),
            kind: AudienceInteractionKind::PullRequestOpened,
            public_url: Some("https://github.com/NSPG13/agent-bounties/pull/128".to_string()),
            occurred_at: None,
            referrer_url: None,
            campaign: Some("github-bounty-label".to_string()),
            source_interaction_id: None,
        };
        let first = network
            .record_audience_interaction(interaction_request.clone())
            .unwrap();
        let duplicate = network
            .record_audience_interaction(interaction_request)
            .unwrap();
        assert_eq!(first.id, duplicate.id);
        assert_eq!(network.list_audience_interactions().len(), 1);

        network
            .record_audience_interaction(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: "payout:escrow:1".to_string(),
                kind: AudienceInteractionKind::PayoutReceived,
                public_url: Some("https://basescan.org/tx/0xabc".to_string()),
                occurred_at: None,
                referrer_url: None,
                campaign: None,
                source_interaction_id: Some(first.id),
            })
            .unwrap();
        network
            .record_outreach_attempt(RecordOutreachAttemptRequest {
                audience_member_id: member.id,
                provider_event_id: "issue-comment:feedback:128".to_string(),
                channel: OutreachChannel::GithubPublic,
                public_url: Some(
                    "https://github.com/NSPG13/agent-bounties/issues/127#issuecomment-1"
                        .to_string(),
                ),
                prompt_version: "distribution-v1".to_string(),
                status: OutreachStatus::Responded,
                sent_at: None,
            })
            .unwrap();
        network
            .record_discovery_response(RecordDiscoveryResponseRequest {
                audience_member_id: member.id,
                interaction_id: Some(first.id),
                provider_response_id: "issue-comment:answer:128".to_string(),
                public_source_url: Some(
                    "https://github.com/NSPG13/agent-bounties/pull/138".to_string(),
                ),
                found_via: "GitHub bounty issue search".to_string(),
                motivation: "Small scope and deterministic checks".to_string(),
                improvement_suggestion: "Show exact settlement status".to_string(),
                agent_or_tool: Some("autonomous coding agent".to_string()),
                private_storage_consent: false,
                captured_at: None,
            })
            .unwrap();

        let report = network.audience_report();
        assert_eq!(report.total_members, 1);
        assert_eq!(report.total_interactions, 2);
        assert_eq!(report.repeat_participants, 1);
        assert_eq!(report.paid_participants, 1);
        assert_eq!(report.members_asked_for_discovery_feedback, 1);
        assert_eq!(report.members_with_discovery_responses, 1);
        assert!(report.not_asked_or_answered_handles.is_empty());
        assert!(report.asked_without_response_handles.is_empty());
        assert_eq!(
            network.audience_members[&member.id].lifecycle_stage,
            AudienceLifecycleStage::Retained
        );
    }

    #[test]
    fn private_audience_data_and_outreach_require_explicit_consent() {
        let mut network = BountyNetwork::default();
        let member = network
            .upsert_audience_member(UpsertAudienceMemberRequest {
                provider: AudienceProvider::Github,
                external_id: "U_456".to_string(),
                handle: "private-user".to_string(),
                public_profile_url: Some("https://github.com/private-user".to_string()),
                roles: vec![],
                observed_at: None,
            })
            .unwrap();

        let response_error = network
            .record_discovery_response(RecordDiscoveryResponseRequest {
                audience_member_id: member.id,
                interaction_id: None,
                provider_response_id: "private-form:1".to_string(),
                public_source_url: None,
                found_via: "private referral".to_string(),
                motivation: "payment trust".to_string(),
                improvement_suggestion: "simpler funding".to_string(),
                agent_or_tool: None,
                private_storage_consent: false,
                captured_at: None,
            })
            .unwrap_err();
        assert!(matches!(
            response_error,
            AppError::InvalidAudienceRecord(message)
                if message.contains("private_storage_consent")
        ));

        let outreach_request = RecordOutreachAttemptRequest {
            audience_member_id: member.id,
            provider_event_id: "email:distribution-v1".to_string(),
            channel: OutreachChannel::EmailPrivate,
            public_url: None,
            prompt_version: "distribution-v1".to_string(),
            status: OutreachStatus::Pending,
            sent_at: None,
        };
        let outreach_error = network
            .record_outreach_attempt(outreach_request.clone())
            .unwrap_err();
        assert!(matches!(
            outreach_error,
            AppError::InvalidAudienceRecord(message) if message.contains("opted-in")
        ));

        let contact = network
            .upsert_contributor_contact(UpsertContributorContactRequest {
                github_login: "private-user".to_string(),
                email: Some("private@example.com".to_string()),
                payout_wallet: None,
                associated_prs: vec![],
                contact_consent: true,
                wallet_consent: false,
                outreach_allowed: true,
                source: Some("private-opt-in".to_string()),
                notes: None,
            })
            .unwrap();
        let attempt = network.record_outreach_attempt(outreach_request).unwrap();
        assert_eq!(attempt.consent_contact_id, Some(contact.id));
    }

    #[test]
    fn live_money_readiness_uses_autonomous_base_without_stripe_or_operator() {
        let report = build_live_money_readiness_report(LiveMoneyReadinessConfig {
            network: "base-mainnet".to_string(),
            escrow_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
            usdc_token: Some(chain_base::BASE_MAINNET_USDC_TOKEN_ADDRESS.to_string()),
            stripe_secret_key_mode: "unset".to_string(),
            stripe_live_execution_enabled: false,
            stripe_payment_method_configuration_configured: true,
            stripe_webhook_secret_configured: false,
            allow_unsigned_stripe_webhooks: false,
            operator_auth_configured: false,
            base_rpc_url_configured: true,
            base_broadcast_enabled: false,
        })
        .unwrap();

        assert!(!report.stripe_live_mode_ready);
        assert!(report.base_mainnet_ready);
        assert!(report.live_money_ready);
        assert_eq!(report.network_chain_id, 8_453);
        assert!(report.stripe_payment_method_configuration_configured);
        assert_eq!(
            report.network_native_usdc_token_address,
            chain_base::BASE_MAINNET_USDC_TOKEN_ADDRESS
        );
        assert_eq!(report.supplied_usdc_token_matches_native, Some(true));
        assert!(report
            .evidence_boundaries
            .iter()
            .any(|boundary| boundary.contains("checkout.session.completed")));
        assert!(report.evidence_boundaries.iter().any(|boundary| {
            boundary.contains("Payment Method Configuration")
                && boundary.contains("not funding, payout, or settlement evidence")
        }));
        assert!(report.checks.iter().any(|check| {
            check.name == "Stripe Checkout payment-method configuration"
                && check.configured
                && check.env_vars == vec!["STRIPE_PAYMENT_METHOD_CONFIGURATION".to_string()]
        }));
    }

    #[test]
    fn live_money_readiness_warns_on_wrong_usdc_token() {
        let report = build_live_money_readiness_report(LiveMoneyReadinessConfig {
            network: "base-sepolia".to_string(),
            escrow_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
            usdc_token: Some("0x3333333333333333333333333333333333333333".to_string()),
            stripe_secret_key_mode: "test".to_string(),
            stripe_live_execution_enabled: true,
            stripe_payment_method_configuration_configured: false,
            stripe_webhook_secret_configured: true,
            allow_unsigned_stripe_webhooks: false,
            operator_auth_configured: true,
            base_rpc_url_configured: true,
            base_broadcast_enabled: false,
        })
        .unwrap();

        assert!(!report.base_testnet_ready);
        assert!(!report.stripe_payment_method_configuration_configured);
        assert_eq!(report.supplied_usdc_token_matches_native, Some(false));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains(chain_base::BASE_SEPOLIA_USDC_TOKEN_ADDRESS)));
    }

    fn stripe_funding_credit(
        organization_id: Id,
        amount_minor: i64,
        currency: &str,
        event_id: &str,
    ) -> StripeFundingCredit {
        StripeFundingCredit {
            organization_id,
            amount: Money::new(amount_minor, currency).unwrap(),
            checkout_session_id: format!("cs_{event_id}"),
            payment_intent_id: Some(format!("pi_{event_id}")),
            bounty_id: None,
            funding_intent_id: None,
            payment_event: PaymentEvent {
                id: Uuid::new_v4(),
                rail: PaymentRail::StripeFiat,
                external_id: event_id.to_string(),
                status: PaymentEventStatus::Applied,
                payload_hash: hash_artifact(event_id),
                received_at: Utc::now(),
            },
        }
    }

    fn stripe_transfer_evidence(
        settlement: &Settlement,
        payout_intent: &PayoutIntent,
        connected_account_id: &str,
        transfer_id: &str,
    ) -> ConnectTransferEvidence {
        ConnectTransferEvidence {
            transfer_id: transfer_id.to_string(),
            connected_account_id: connected_account_id.to_string(),
            bounty_id: settlement.bounty_id,
            proof_record_id: settlement.proof_record_id,
            settlement_id: settlement.id,
            payout_intent_id: payout_intent.id,
            agent_id: payout_intent.recipient_agent_id,
            amount: payout_intent.amount.clone(),
            payment_event: PaymentEvent {
                id: Uuid::new_v4(),
                rail: PaymentRail::StripeFiat,
                external_id: format!("evt_{transfer_id}"),
                status: PaymentEventStatus::Applied,
                payload_hash: hash_artifact(transfer_id),
                received_at: Utc::now(),
            },
        }
    }

    #[tokio::test]
    async fn open_beta_pays_even_one_minor_unit_in_full_to_solver() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "one-unit-solver".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Verify one unit payout".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1,
                currency: "usd".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"ok\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "memory://one-unit.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: hash_artifact(artifact),
                verifier_kind: Some(VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(status.settlements.len(), 1);
        assert_eq!(status.settlements[0].payout_intents.len(), 1);
        assert_eq!(status.settlements[0].payout_intents[0].amount.amount, 1);
        assert_eq!(status.settlements[0].platform_fee, Money::zero("usd"));
        assert_eq!(
            settlement_total_amount(&status.settlements[0])
                .unwrap()
                .amount,
            1
        );
    }

    #[test]
    fn initial_non_base_funding_contribution_links_to_ledger_entry() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Write onboarding docs".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();

        let status = network.status(bounty.id).unwrap();
        let contribution = status.funding_contributions.first().unwrap();

        assert_eq!(network.ledger.entries().len(), 1);
        assert_eq!(
            contribution.funding_ledger_entry_id,
            Some(network.ledger.entries()[0].id)
        );
        assert_eq!(contribution.settlement_id, None);
        assert_eq!(contribution.refund_ledger_entry_id, None);
    }

    #[test]
    fn open_pooled_bounty_rejects_public_caller_supplied_identity() {
        let mut network = BountyNetwork::default();
        let existing = network
            .open_pooled_bounty(github_sync_pooled_request("Public bounty", 1_000))
            .unwrap();
        let mut overwrite = github_sync_pooled_request("Overwrite public bounty", 2_000);
        overwrite.bounty_id = Some(existing.id);
        overwrite.idempotency_key = Some(github_sync_idempotency_key(existing.id));

        let err = network.open_pooled_bounty(overwrite).unwrap_err();

        assert!(matches!(
            err,
            AppError::InvalidFundingContribution(message)
                if message.contains("public pooled bounty creation cannot set")
        ));
        assert_eq!(
            network.bounties.get(&existing.id).unwrap().title,
            "Public bounty"
        );
    }

    #[test]
    fn github_issue_sync_replays_stable_unfunded_id_without_duplicate() {
        let mut network = BountyNetwork::default();
        let bounty_id = Uuid::new_v4();
        let first = network
            .upsert_github_issue_pooled_bounty(
                github_sync_pooled_request("Sync GitHub issue into API", 1_000),
                bounty_id,
                github_sync_idempotency_key(bounty_id),
            )
            .unwrap();

        let second = network
            .upsert_github_issue_pooled_bounty(
                github_sync_pooled_request("Sync GitHub issue into hosted API", 2_000),
                bounty_id,
                github_sync_idempotency_key(bounty_id),
            )
            .unwrap();

        assert_eq!(second.id, bounty_id);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(network.bounties.len(), 1);
        assert_eq!(
            network.bounties.get(&bounty_id).unwrap().title,
            "Sync GitHub issue into hosted API"
        );
    }

    #[test]
    fn github_issue_sync_rejects_stable_id_replay_after_funding_activity() {
        let mut network = BountyNetwork::default();
        let bounty_id = Uuid::new_v4();
        let sponsor = network.register_agent(RegisterAgentRequest {
            handle: "github-sync-sponsor".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .upsert_github_issue_pooled_bounty(
                github_sync_pooled_request("Sync GitHub issue into API", 1_000),
                bounty_id,
                github_sync_idempotency_key(bounty_id),
            )
            .unwrap();
        network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: Some(sponsor.id),
                source_organization_id: None,
                amount_minor: 100,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("github-sync-partial".to_string()),
            })
            .unwrap();

        let err = network
            .upsert_github_issue_pooled_bounty(
                github_sync_pooled_request("Attempt duplicate GitHub issue sync", 1_000),
                bounty_id,
                github_sync_idempotency_key(bounty_id),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            AppError::InvalidFundingContribution(message)
                if message.contains("idempotent update")
        ));
    }

    #[test]
    fn pooled_simulated_funding_becomes_claimable_only_at_target() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: None,
        });
        let sponsor_a = network.register_agent(RegisterAgentRequest {
            handle: "sponsor-a".to_string(),
            payout_wallet: None,
        });
        let sponsor_b = network.register_agent(RegisterAgentRequest {
            handle: "sponsor-b".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Write agent onboarding docs".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            })
            .unwrap();

        assert_eq!(bounty.status, BountyStatus::Unfunded);
        assert!(network.list_claimable_bounties().is_empty());
        assert!(matches!(
            network
                .claim_bounty(ClaimBountyRequest {
                    bounty_id: bounty.id,
                    solver_agent_id: solver.id,
                })
                .unwrap_err(),
            AppError::Domain(domain::DomainError::UnfundedBounty)
        ));

        let partial = network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: Some(sponsor_a.id),
                source_organization_id: None,
                amount_minor: 400,
                currency: "USDC".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("sponsor-a-400".to_string()),
            })
            .unwrap();
        assert_eq!(partial.bounty.status, BountyStatus::Unfunded);
        assert_eq!(partial.funding_summary.applied.amount, 400);
        assert_eq!(partial.funding_summary.remaining.amount, 600);
        assert!(!partial.funding_summary.claimable);
        assert_eq!(
            partial.contribution.funding_ledger_entry_id,
            Some(partial.ledger_entries[0].id)
        );
        assert_eq!(partial.contribution.settlement_id, None);
        assert_eq!(partial.contribution.refund_ledger_entry_id, None);
        assert!(network.list_claimable_bounties().is_empty());

        let funded = network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: Some(sponsor_b.id),
                source_organization_id: None,
                amount_minor: 600,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("sponsor-b-600".to_string()),
            })
            .unwrap();
        assert_eq!(funded.bounty.status, BountyStatus::Claimable);
        assert_eq!(funded.funding_summary.applied.amount, 1_000);
        assert_eq!(funded.funding_summary.remaining.amount, 0);
        assert_eq!(funded.funding_summary.contribution_count, 2);
        assert!(funded.funding_summary.claimable);
        assert_eq!(
            funded.contribution.funding_ledger_entry_id,
            Some(funded.ledger_entries[0].id)
        );
        assert_eq!(network.list_claimable_bounties().len(), 1);
        assert_eq!(network.ledger.entries().len(), 2);

        let claimed = network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        assert_eq!(claimed.status, BountyStatus::Claimed);
    }

    #[tokio::test]
    async fn pooled_contributions_link_to_settlement_after_verification() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: None,
        });
        let sponsor = network.register_agent(RegisterAgentRequest {
            handle: "sponsor".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Extract public data".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            })
            .unwrap();
        network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: Some(sponsor.id),
                source_organization_id: None,
                amount_minor: 1_000,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("sponsor-full".to_string()),
            })
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"ok\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "memory://pooled-artifact".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();

        network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: hash_artifact(artifact),
                verifier_kind: Some(VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();

        let status = network.status(bounty.id).unwrap();
        let settlement = status.settlements.first().unwrap();

        assert_eq!(status.funding_contributions.len(), 1);
        assert_eq!(
            status.funding_contributions[0].settlement_id,
            Some(settlement.id)
        );
        assert_eq!(status.funding_contributions[0].refund_ledger_entry_id, None);
        assert_eq!(status.bounty.status, BountyStatus::Paid);
    }

    #[test]
    fn pooled_funding_rejects_overfunding_and_duplicate_references() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fix docs typo".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            })
            .unwrap();

        network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 700,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("shared-ref".to_string()),
            })
            .unwrap();
        assert!(matches!(
            network
                .add_funding_contribution(AddFundingContributionRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: None,
                    amount_minor: 100,
                    currency: "usdc".to_string(),
                    rail: PaymentRail::Simulated,
                    external_reference: Some("shared-ref".to_string()),
                })
                .unwrap_err(),
            AppError::InvalidFundingContribution(_)
        ));
        assert!(matches!(
            network
                .add_funding_contribution(AddFundingContributionRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: None,
                    amount_minor: 400,
                    currency: "usdc".to_string(),
                    rail: PaymentRail::Simulated,
                    external_reference: Some("too-much".to_string()),
                })
                .unwrap_err(),
            AppError::InvalidFundingContribution(_)
        ));

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.funding_contributions.len(), 1);
        assert_eq!(status.funding_summary.applied.amount, 700);
        assert_eq!(status.funding_summary.remaining.amount, 300);
        assert!(!status.funding_summary.claimable);
    }

    #[test]
    fn stripe_pooled_funding_requires_verified_platform_balance() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund fiat-backed docs work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            })
            .unwrap();

        assert!(matches!(
            network
                .add_funding_contribution(AddFundingContributionRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: None,
                    amount_minor: 500,
                    currency: "usd".to_string(),
                    rail: PaymentRail::StripeFiat,
                    external_reference: Some("unbacked".to_string()),
                })
                .unwrap_err(),
            AppError::InvalidFundingContribution(_)
        ));
        assert!(matches!(
            network
                .add_funding_contribution(AddFundingContributionRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: Some(organization_id),
                    amount_minor: 500,
                    currency: "usd".to_string(),
                    rail: PaymentRail::StripeFiat,
                    external_reference: Some("insufficient".to_string()),
                })
                .unwrap_err(),
            AppError::InvalidFundingContribution(_)
        ));

        network
            .apply_stripe_funding_credit(stripe_funding_credit(
                organization_id,
                700,
                "usd",
                "evt_topup_700",
            ))
            .unwrap();
        assert_eq!(
            network.stripe_platform_balance_available_minor(organization_id, "usd"),
            700
        );

        let partial = network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(organization_id),
                amount_minor: 700,
                currency: "usd".to_string(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some("stripe-700".to_string()),
            })
            .unwrap();
        assert_eq!(partial.bounty.status, BountyStatus::Unfunded);
        assert_eq!(partial.funding_summary.remaining.amount, 300);
        assert_eq!(
            partial.contribution.source_organization_id,
            Some(organization_id)
        );
        assert_eq!(
            network.stripe_platform_balance_available_minor(organization_id, "usd"),
            0
        );

        assert!(matches!(
            network
                .add_funding_contribution(AddFundingContributionRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: Some(organization_id),
                    amount_minor: 300,
                    currency: "usd".to_string(),
                    rail: PaymentRail::StripeFiat,
                    external_reference: Some("stripe-300-before-topup".to_string()),
                })
                .unwrap_err(),
            AppError::InvalidFundingContribution(_)
        ));

        network
            .apply_stripe_funding_credit(stripe_funding_credit(
                organization_id,
                300,
                "usd",
                "evt_topup_300",
            ))
            .unwrap();
        let funded = network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(organization_id),
                amount_minor: 300,
                currency: "usd".to_string(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some("stripe-300".to_string()),
            })
            .unwrap();

        assert_eq!(funded.bounty.status, BountyStatus::Claimable);
        assert!(funded.funding_summary.claimable);
        assert_eq!(
            network.stripe_platform_balance_available_minor(organization_id, "usd"),
            0
        );
    }

    #[tokio::test]
    async fn capability_help_quote_to_bounty_loop() {
        let mut network = BountyNetwork::default();
        let requester = network.register_agent(RegisterAgentRequest {
            handle: "requester".to_string(),
            payout_wallet: None,
        });
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0xsolver".to_string()),
        });

        network
            .register_capability(RegisterCapabilityRequest {
                agent_id: solver.id,
                class: CapabilityClass::Ci,
                template_slugs: vec!["fix-ci-failure".to_string()],
                min_price_minor: 100,
                max_price_minor: 2_000_000,
                currency: "usdc".to_string(),
                latency_seconds: 600,
                supported_verifiers: vec![VerifierKind::GitHubCi],
            })
            .unwrap();

        let help_request = network
            .create_help_request(CreateHelpRequestRequest {
                requester_agent_id: requester.id,
                goal: "Fix CI failure".to_string(),
                context: "GitHub check failed".to_string(),
                budget_minor: 1_000_000,
                currency: "usdc".to_string(),
                privacy: PrivacyLevel::Public,
                required_confidence: None,
            })
            .unwrap();

        let quote_set = network
            .request_quotes(RequestQuotesRequest {
                help_request_id: help_request.id,
            })
            .unwrap();
        assert_eq!(quote_set.quotes.len(), 1);

        let bounty = network
            .fund_quote_as_bounty(FundQuoteRequest {
                quote_id: quote_set.quotes[0].id,
                title: None,
                funding_mode: Some(FundingMode::Simulated),
            })
            .unwrap();

        assert_eq!(bounty.status, BountyStatus::Claimable);
        assert!(bounty.terms_hash.is_some());
        assert_eq!(network.list_claimable_bounties().len(), 1);
        assert_eq!(bounty.template_slug, "fix-ci-failure");
        assert_eq!(bounty.help_request_id, Some(help_request.id));
    }

    #[tokio::test]
    async fn ci_bounty_uses_github_ci_verifier_by_default() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0xsolver".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix CI failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "https://github.com/example/repo/pull/1".to_string(),
                artifact_body: "{\"check\":\"green\"}".to_string(),
            })
            .unwrap();

        network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: "not-used-by-github-ci".to_string(),
                verifier_kind: None,
                rubric: None,
                evidence: Some(github_ci_evidence()),
                approved_risk_event_id: None,
            })
            .await
            .unwrap();

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(status.verifier_results[0].kind, VerifierKind::GitHubCi);
        assert_eq!(
            status.verifier_results[0].decision,
            VerificationDecision::Accepted
        );
    }

    #[tokio::test]
    async fn cannot_verify_submission_against_another_bounty() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0xsolver".to_string()),
        });
        let first = network
            .post_funded_bounty(PostBountyRequest {
                title: "Extract first artifact".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let second = network
            .post_funded_bounty(PostBountyRequest {
                title: "Extract second artifact".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: first.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"ok\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: first.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://local/artifact.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();

        let err = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: second.id,
                submission_id: submission.id,
                expected_artifact_digest: hash_artifact(artifact),
                verifier_kind: Some(VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::SubmissionBountyMismatch));
        assert_eq!(
            network.status(second.id).unwrap().bounty.status,
            BountyStatus::Claimable
        );
    }

    #[tokio::test]
    async fn stripe_fiat_settlement_blocks_payout_until_connect_eligible() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Summarize private notes".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 5_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Private,
                funding_targets: vec![],
            })
            .unwrap();
        network
            .apply_stripe_funding_credit(stripe_funding_credit(
                organization_id,
                5_000,
                "usd",
                "evt_private_notes_topup",
            ))
            .unwrap();
        network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(organization_id),
                amount_minor: 5_000,
                currency: "usd".to_string(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some("stripe-private-notes".to_string()),
            })
            .unwrap();

        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"summary\":\"ok\"}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://private/summary.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: hash_artifact(artifact),
                verifier_kind: Some(VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Payable);
        assert_eq!(status.settlements.len(), 1);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Blocked
        );
        assert_eq!(status.funding_contributions.len(), 1);
        assert_eq!(
            status.funding_contributions[0].source_organization_id,
            Some(organization_id)
        );
        assert_eq!(network.ledger.entries().len(), 2);

        let premature_plan = network
            .plan_stripe_transfer(
                PlanStripeTransferRequest {
                    payout_intent_id: status.settlements[0].payout_intents[0].id,
                    connected_account_id: "acct_test".to_string(),
                },
                "https://agentbounties.test",
            )
            .unwrap_err();
        assert!(matches!(premature_plan, AppError::InvalidStripePayout(_)));

        let blocked = network
            .apply_stripe_connect_snapshot(ConnectAccountSnapshot {
                agent_id: solver.id,
                connected_account_id: Some("acct_test".to_string()),
                payouts_enabled: false,
                disabled_reason: None,
                currently_due: vec!["external_account".to_string()],
            })
            .unwrap();
        assert!(!blocked.payout_state.eligible);
        assert!(blocked.ledger_entries.is_empty());
        assert_eq!(
            blocked.settlements[0].payout_intents[0].status,
            PayoutStatus::Blocked
        );

        let eligible = network
            .apply_stripe_connect_snapshot(ConnectAccountSnapshot {
                agent_id: solver.id,
                connected_account_id: Some("acct_test".to_string()),
                payouts_enabled: true,
                disabled_reason: None,
                currently_due: vec![],
            })
            .unwrap();
        assert!(eligible.payout_state.eligible);
        assert!(eligible.ledger_entries.is_empty());

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Payable);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Pending
        );
        assert_eq!(network.ledger.entries().len(), 2);

        let transfer_plan = network
            .plan_stripe_transfer(
                PlanStripeTransferRequest {
                    payout_intent_id: status.settlements[0].payout_intents[0].id,
                    connected_account_id: "acct_test".to_string(),
                },
                "https://agentbounties.test",
            )
            .unwrap();
        assert_eq!(
            transfer_plan.request.body["metadata"]["payout_intent_id"],
            status.settlements[0].payout_intents[0].id.to_string()
        );
        assert!(transfer_plan.requires_reconciliation);

        let transfer = network
            .apply_stripe_transfer_evidence(stripe_transfer_evidence(
                &status.settlements[0],
                &status.settlements[0].payout_intents[0],
                "acct_test",
                "tr_private_notes_solver",
            ))
            .unwrap();
        assert!(!transfer.duplicate);
        assert_eq!(transfer.ledger_entries.len(), 1);

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
        assert_eq!(network.ledger.entries().len(), 3);

        let replay = network
            .apply_stripe_transfer_evidence(transfer.evidence.clone())
            .unwrap();
        assert!(replay.duplicate);
        assert!(replay.ledger_entries.is_empty());
        assert_eq!(network.ledger.entries().len(), 3);
    }

    #[test]
    fn non_claim_owner_cannot_submit() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0xsolver".to_string()),
        });
        let other = network.register_agent(RegisterAgentRequest {
            handle: "other".to_string(),
            payout_wallet: Some("0xother".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic test failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();

        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();

        let err = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: other.id,
                artifact_uri: "s3://local/artifact.json".to_string(),
                artifact_body: "{}".to_string(),
            })
            .unwrap_err();

        assert!(matches!(err, AppError::RiskBlocked(_)));
        assert_eq!(network.risk_events.len(), 1);
    }

    fn github_ci_evidence() -> serde_json::Value {
        serde_json::json!({
            "repository": "example/repo",
            "pull_request_url": "https://github.com/example/repo/pull/1",
            "pull_request": {
                "author_login": "solver-agent",
                "merged": true,
                "merged_by_login": "maintainer",
                "reviews": [
                    {
                        "author_login": "maintainer",
                        "state": "APPROVED"
                    }
                ]
            },
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "check_run": {
                "id": 123456789_u64,
                "name": "full-check",
                "status": "completed",
                "conclusion": "success",
                "head_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "html_url": "https://github.com/example/repo/actions/runs/123456789",
                "repository": {
                    "full_name": "example/repo"
                }
            }
        })
    }
}
