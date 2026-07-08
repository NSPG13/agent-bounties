use bounty_router::{template_for_class, BountyRouter};
use chain_base::{
    base_network_descriptor, BaseEscrowCreate, BaseEscrowEvent, BaseEscrowEventKind,
    BaseEscrowFundingPlan, BaseEscrowRelease, BaseEscrowReleaseCall, BaseEscrowTxPlanner,
    BaseNetworkDescriptor, EscrowRecipient, EvmTransactionIntent,
};
use chrono::{DateTime, Utc};
use domain::{
    Agent, Bounty, BountyStatus, Capability, CapabilityClass, Claim, Escrow, EscrowStatus,
    FundingContribution, FundingContributionStatus, FundingIntent, FundingIntentStatus,
    FundingMode, FundingPartitionTarget, HelpRequest, Id, Money, PaymentEvent, PaymentEventStatus,
    PaymentRail, PayoutIntent, PayoutStatus, PrivacyLevel, ProofRecord, Quote, ReputationEvent,
    RiskAction, RiskEvent, RiskReviewOutcome, RiskReviewRecord, RiskSurface, Settlement,
    Submission, TemplateSignal, VerificationDecision, VerifierKind, VerifierResult,
};
use ledger::{credit, debit, AccountCode, Ledger, LedgerEntry};
use payments_stripe::{
    evaluate_connect_payout, CheckoutTopUpRequest, ConnectAccountSnapshot, ConnectPayoutState,
    StripeFundingCredit, StripePlanner, StripeRequestIntent,
};
use risk::{
    BountyRiskInput, HelpRequestRiskInput, PayoutRiskInput, RiskAssessment, RiskPolicy,
    SubmissionRiskInput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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
    #[error("invalid base escrow event: {0}")]
    InvalidBaseEscrowEvent(String),
    #[error("invalid Base funding plan: {0}")]
    InvalidBaseFundingPlan(String),
    #[error("invalid Base release plan: {0}")]
    InvalidBaseReleasePlan(String),
    #[error("invalid Base escrow plan: {0}")]
    InvalidBaseEscrowPlan(String),
    #[error("invalid funding contribution: {0}")]
    InvalidFundingContribution(String),
    #[error("invalid funding intent: {0}")]
    InvalidFundingIntent(String),
    #[error("invalid Stripe payout reconciliation: {0}")]
    InvalidStripePayout(String),
    #[error(transparent)]
    Domain(#[from] domain::DomainError),
    #[error(transparent)]
    Ledger(#[from] ledger::LedgerError),
    #[error(transparent)]
    Verifier(#[from] verifier_sdk::VerifierError),
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterAgentRequest {
    pub handle: String,
    pub payout_wallet: Option<String>,
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
    pub base_escrow_contract: Option<String>,
    pub base_payer: Option<String>,
    pub base_token: Option<String>,
    #[serde(default)]
    pub base_network: Option<String>,
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
pub struct PlanBaseFundingRequest {
    pub bounty_id: Id,
    pub escrow_contract: String,
    pub payer: String,
    pub token: String,
    #[serde(default)]
    pub network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanBaseReleaseRequest {
    pub bounty_id: Id,
    pub escrow_contract: String,
    pub platform_fee_wallet: String,
    #[serde(default)]
    pub network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanBaseRefundRequest {
    pub bounty_id: Id,
    pub escrow_contract: String,
    pub reason_hash: String,
    #[serde(default)]
    pub network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanBaseDisputeRequest {
    pub bounty_id: Id,
    pub escrow_contract: String,
    pub dispute_hash: String,
    #[serde(default)]
    pub network: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BaseReleaseQueueRequest {
    pub escrow_contract: Option<String>,
    pub platform_fee_wallet: Option<String>,
    #[serde(default)]
    pub network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseFundingPlan {
    pub network: BaseNetworkDescriptor,
    pub bounty: Bounty,
    pub create: BaseEscrowCreate,
    pub funding: BaseEscrowFundingPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum FundingIntentNextAction {
    StripeCheckout { request: StripeRequestIntent },
    BaseEscrowFunding { plan: Box<BaseFundingPlan> },
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
pub struct BaseReleasePlan {
    pub network: BaseNetworkDescriptor,
    pub bounty: Bounty,
    pub escrow: Escrow,
    pub settlement: Settlement,
    pub release_call: BaseEscrowReleaseCall,
    pub transaction: EvmTransactionIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseRefundPlan {
    pub network: BaseNetworkDescriptor,
    pub bounty: Bounty,
    pub escrow: Escrow,
    pub onchain_escrow_id: u128,
    pub reason_hash: String,
    pub transaction: EvmTransactionIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseDisputePlan {
    pub network: BaseNetworkDescriptor,
    pub bounty: Bounty,
    pub escrow: Escrow,
    pub onchain_escrow_id: u128,
    pub dispute_hash: String,
    pub transaction: EvmTransactionIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseReleaseQueueItem {
    pub bounty: Bounty,
    pub settlement: Settlement,
    pub escrow: Option<Escrow>,
    pub proof: Option<ProofRecord>,
    pub pending_payout_count: usize,
    pub pending_amount: Money,
    pub onchain_escrow_id: Option<u128>,
    pub missing_recipient_agent_ids: Vec<Id>,
    pub ready: bool,
    pub readiness_error: Option<String>,
    pub release_plan: Option<BaseReleasePlan>,
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
pub struct BaseEscrowReconciliation {
    pub event: BaseEscrowEvent,
    pub escrow: Escrow,
    pub bounty: Bounty,
    pub funding_intents: Vec<FundingIntent>,
    pub settlements: Vec<Settlement>,
    pub ledger_entries: Vec<LedgerEntry>,
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
    pub capabilities: HashMap<Id, Capability>,
    pub help_requests: HashMap<Id, HelpRequest>,
    pub quotes: HashMap<Id, Quote>,
    pub bounties: HashMap<Id, Bounty>,
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
            capabilities: HashMap::new(),
            help_requests: HashMap::new(),
            quotes: HashMap::new(),
            bounties: HashMap::new(),
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

impl BountyNetwork {
    pub fn register_agent(&mut self, request: RegisterAgentRequest) -> Agent {
        let mut agent = Agent::new(request.handle);
        agent.payout_wallet = request.payout_wallet;
        self.agents.insert(agent.id, agent.clone());
        agent
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
        if funding_mode == FundingMode::BaseUsdcEscrow {
            bounty.terms_hash = Some(terms_hash);
            self.bounties.insert(bounty.id, bounty.clone());
            return Ok(bounty);
        }
        if funding_mode == FundingMode::MixedRails {
            return Err(AppError::InvalidFundingContribution(
                "mixed rail bounties must be opened as pooled bounties with explicit funding targets"
                    .to_string(),
            ));
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
        let amount = Money::new(request.target_amount_minor, request.currency)?;
        let funding_mode = request.funding_mode.clone();
        let funding_targets =
            funding_targets_from_request(&funding_mode, &amount, &request.funding_targets)?;
        let mut bounty = Bounty::new(
            request.title,
            request.template_slug,
            amount.clone(),
            funding_mode.clone(),
            request.privacy.clone(),
        )
        .with_funding_targets(funding_targets);
        let risk = self.risk_policy.evaluate_bounty(&BountyRiskInput {
            title: bounty.title.clone(),
            template_slug: bounty.template_slug.clone(),
            amount: amount.clone(),
            funding_mode,
            privacy: request.privacy,
        });
        self.enforce_risk(risk, bounty.id, None, Some(bounty.id))?;
        bounty.terms_hash = Some(hash_terms(&bounty.title, &bounty.template_slug, &amount));
        self.bounties.insert(bounty.id, bounty.clone());
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
        if request.rail == PaymentRail::Simulated {
            return Err(AppError::InvalidFundingIntent(
                "funding intents are only for real payment rails".to_string(),
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
        let mut intent = FundingIntent {
            id: intent_id,
            bounty_id: bounty.id,
            contributor_agent_id: request.contributor_agent_id,
            source_organization_id: request.source_organization_id,
            rail: request.rail.clone(),
            amount: amount.clone(),
            status: FundingIntentStatus::AwaitingEvidence,
            external_reference: Some(external_reference.clone()),
            created_at: Utc::now(),
        };

        let platform_base_url = platform_base_url.into();
        let (next_action, reconciliation_hint) = match request.rail {
            PaymentRail::StripeFiat => {
                let organization_id = request.source_organization_id.ok_or_else(|| {
                    AppError::InvalidFundingIntent(
                        "Stripe fiat funding intents require source_organization_id".to_string(),
                    )
                })?;
                let success_url = request.stripe_success_url.unwrap_or_else(|| {
                    format!("{}/stripe/success", platform_base_url.trim_end_matches('/'))
                });
                let cancel_url = request.stripe_cancel_url.unwrap_or_else(|| {
                    format!("{}/stripe/cancel", platform_base_url.trim_end_matches('/'))
                });
                let mut checkout = StripePlanner::new(platform_base_url.clone())
                    .checkout_top_up(&CheckoutTopUpRequest {
                        organization_id,
                        amount: amount.clone(),
                        success_url,
                        cancel_url,
                    })
                    .map_err(|error| AppError::InvalidFundingIntent(error.to_string()))?;
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
                        serde_json::json!(external_reference),
                    );
                    metadata.insert(
                        "purpose".to_string(),
                        serde_json::json!("bounty_funding_intent"),
                    );
                }
                (
                    FundingIntentNextAction::StripeCheckout { request: checkout },
                    "Stripe intent remains pending until a verified paid Checkout webhook credits the source organization and reserves the balance into the bounty.".to_string(),
                )
            }
            PaymentRail::BaseUsdc => {
                if request.source_organization_id.is_some() {
                    return Err(AppError::InvalidFundingIntent(
                        "source_organization_id is only valid for Stripe fiat intents".to_string(),
                    ));
                }
                if amount.amount != target.amount.amount || amount.amount != remaining_before {
                    return Err(AppError::InvalidFundingIntent(
                        "Base USDC funding intents must cover the full remaining Base partition for the current escrow contract".to_string(),
                    ));
                }
                let plan = self.plan_base_funding(PlanBaseFundingRequest {
                    bounty_id: bounty.id,
                    escrow_contract: required_base_field(
                        request.base_escrow_contract,
                        "base_escrow_contract",
                    )?,
                    payer: required_base_field(request.base_payer, "base_payer")?,
                    token: required_base_field(request.base_token, "base_token")?,
                    network: request.base_network,
                })?;
                (
                    FundingIntentNextAction::BaseEscrowFunding {
                        plan: Box::new(plan),
                    },
                    "Base intent remains pending until an indexed EscrowCreated log with matching bounty, amount, and terms hash is reconciled.".to_string(),
                )
            }
            PaymentRail::Simulated => unreachable!("simulated rail rejected above"),
        };

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

    pub fn claim_bounty(&mut self, request: ClaimBountyRequest) -> AppResult<Bounty> {
        if !self.agents.contains_key(&request.solver_agent_id) {
            return Err(AppError::AgentNotFound);
        }

        let bounty_snapshot = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        let requires_base_escrow = self
            .effective_funding_targets(&bounty_snapshot)?
            .iter()
            .any(|target| target.rail == PaymentRail::BaseUsdc);
        if requires_base_escrow && !self.has_funded_base_escrow(request.bounty_id) {
            return Err(AppError::InvalidBaseEscrowEvent(
                "Base USDC bounty cannot be claimed before funded escrow is indexed".to_string(),
            ));
        }
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

    pub fn apply_base_escrow_event(
        &mut self,
        event: BaseEscrowEvent,
    ) -> AppResult<BaseEscrowReconciliation> {
        if event.onchain_escrow_id == 0 {
            return Err(AppError::InvalidBaseEscrowEvent(
                "onchain escrow id must be non-zero".to_string(),
            ));
        }
        let bounty = self
            .bounties
            .get(&event.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if !matches!(
            bounty.funding_mode,
            FundingMode::BaseUsdcEscrow | FundingMode::MixedRails
        ) {
            return Err(AppError::InvalidBaseEscrowEvent(
                "event bounty is not funded through Base USDC escrow".to_string(),
            ));
        }

        let escrow_id = base_escrow_uuid(event.bounty_id, event.onchain_escrow_id);
        let mut ledger_entries = Vec::new();
        match event.kind {
            BaseEscrowEventKind::Created => {
                let token = event.token.clone().ok_or_else(|| {
                    AppError::InvalidBaseEscrowEvent("created event must include token".to_string())
                })?;
                let amount = event.amount.clone().ok_or_else(|| {
                    AppError::InvalidBaseEscrowEvent(
                        "created event must include amount".to_string(),
                    )
                })?;
                let terms_hash = event.terms_hash.clone().ok_or_else(|| {
                    AppError::InvalidBaseEscrowEvent(
                        "created event must include terms hash".to_string(),
                    )
                })?;
                let target = self.base_funding_target(&bounty)?;
                validate_created_base_escrow(&bounty, &target, &amount, &terms_hash)?;
                if self.escrows.values().any(|escrow| {
                    escrow.bounty_id == event.bounty_id
                        && escrow.rail == PaymentRail::BaseUsdc
                        && escrow.id != escrow_id
                        && escrow.status != EscrowStatus::Refunded
                }) {
                    return Err(AppError::InvalidBaseEscrowEvent(
                        "bounty already has a different indexed Base USDC escrow".to_string(),
                    ));
                }

                let already_confirmed = self.escrows.get(&escrow_id).is_some_and(|escrow| {
                    matches!(
                        escrow.status,
                        EscrowStatus::Funded | EscrowStatus::Disputed | EscrowStatus::Released
                    )
                });
                let status = self
                    .escrows
                    .get(&escrow_id)
                    .map(|escrow| escrow.status.clone())
                    .filter(|status| *status != EscrowStatus::Funded)
                    .unwrap_or(EscrowStatus::Funded);
                self.escrows.insert(
                    escrow_id,
                    Escrow {
                        id: escrow_id,
                        bounty_id: event.bounty_id,
                        rail: PaymentRail::BaseUsdc,
                        token,
                        amount: amount.clone(),
                        status,
                        external_reference: Some(base_escrow_reference(event.onchain_escrow_id)),
                    },
                );
                if !already_confirmed {
                    if let Some(entry) = self
                        .mark_base_escrow_funded(amount, format!("base-fund:{}", event.log_key))?
                    {
                        ledger_entries.push(entry);
                    }
                }
                self.mark_matching_base_funding_intent_applied(event.bounty_id)?;
                self.mark_bounty_claimable_if_fully_funded(event.bounty_id)?;
            }
            BaseEscrowEventKind::Released => {
                if let Some(entry) = self.mark_base_release_paid(
                    event.bounty_id,
                    format!("base-release:{}", event.log_key),
                )? {
                    ledger_entries.push(entry);
                }
                self.update_base_escrow_status(escrow_id, EscrowStatus::Released)?;
            }
            BaseEscrowEventKind::Refunded => {
                if let Some(entry) = self
                    .mark_base_refunded(event.bounty_id, format!("base-refund:{}", event.log_key))?
                {
                    ledger_entries.push(entry);
                }
                self.update_base_escrow_status(escrow_id, EscrowStatus::Refunded)?;
            }
            BaseEscrowEventKind::Disputed => {
                self.update_base_escrow_status(escrow_id, EscrowStatus::Disputed)?;
                self.mark_base_disputed(event.bounty_id)?;
            }
            BaseEscrowEventKind::Paused => {}
        }

        let escrow = self
            .escrows
            .get(&escrow_id)
            .ok_or_else(|| {
                AppError::InvalidBaseEscrowEvent("escrow event did not create escrow state".into())
            })?
            .clone();
        let bounty = self
            .bounties
            .get(&event.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        let funding_intents = self
            .funding_intents
            .values()
            .filter(|intent| intent.bounty_id == event.bounty_id)
            .cloned()
            .collect();
        let settlements = self
            .settlements
            .values()
            .filter(|settlement| settlement.bounty_id == event.bounty_id)
            .cloned()
            .collect();

        Ok(BaseEscrowReconciliation {
            event,
            escrow,
            bounty,
            funding_intents,
            settlements,
            ledger_entries,
        })
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

        let mut ledger_entries = Vec::new();
        let mut updated_settlement_ids = Vec::new();
        let mut updated_bounty_ids = Vec::new();

        for settlement_id in settlement_ids {
            if payout_state.eligible {
                let entries = self.mark_stripe_agent_payouts_paid(
                    settlement_id,
                    snapshot.agent_id,
                    snapshot
                        .connected_account_id
                        .as_deref()
                        .unwrap_or("unknown-account"),
                )?;
                ledger_entries.extend(entries);
                if let Some(entry) = self.finalize_stripe_settlement_if_complete(settlement_id)? {
                    updated_bounty_ids.push(
                        self.settlements
                            .get(&settlement_id)
                            .expect("settlement exists")
                            .bounty_id,
                    );
                    ledger_entries.push(entry);
                }
            } else {
                self.mark_stripe_agent_payouts_blocked(settlement_id, snapshot.agent_id)?;
            }
            updated_settlement_ids.push(settlement_id);
        }

        let settlements = updated_settlement_ids
            .into_iter()
            .filter_map(|id| self.settlements.get(&id).cloned())
            .collect();
        let bounties = updated_bounty_ids
            .into_iter()
            .filter_map(|id| self.bounties.get(&id).cloned())
            .collect();

        Ok(StripeConnectPayoutReconciliation {
            payout_state,
            settlements,
            bounties,
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
        if request.funding_mode == FundingMode::BaseUsdcEscrow {
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

    pub fn plan_base_funding(&self, request: PlanBaseFundingRequest) -> AppResult<BaseFundingPlan> {
        let bounty = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if !matches!(
            bounty.funding_mode,
            FundingMode::BaseUsdcEscrow | FundingMode::MixedRails
        ) {
            return Err(AppError::InvalidBaseFundingPlan(
                "bounty is not funded through Base USDC escrow".to_string(),
            ));
        }
        if bounty.status.is_terminal() {
            return Err(AppError::InvalidBaseFundingPlan(format!(
                "terminal bounty cannot be funded on-chain; current status is {:?}",
                bounty.status
            )));
        }
        if self.escrows.values().any(|escrow| {
            escrow.bounty_id == request.bounty_id && escrow.rail == PaymentRail::BaseUsdc
        }) {
            return Err(AppError::InvalidBaseFundingPlan(
                "bounty already has indexed Base USDC escrow state".to_string(),
            ));
        }
        let terms_hash = bounty.terms_hash.clone().ok_or_else(|| {
            AppError::InvalidBaseFundingPlan("bounty is missing a terms hash".to_string())
        })?;
        let base_target = self.base_funding_target(&bounty)?;
        let create = BaseEscrowCreate {
            bounty_id: bounty.id,
            payer: request.payer,
            token: request.token,
            amount: base_target.amount.clone(),
            terms_hash,
        };
        let funding = BaseEscrowTxPlanner::new(request.escrow_contract)
            .map_err(|error| AppError::InvalidBaseFundingPlan(error.to_string()))?
            .plan_funding_for_network(
                request.network.as_deref().unwrap_or("base-sepolia"),
                &create,
            )
            .map_err(|error| AppError::InvalidBaseFundingPlan(error.to_string()))?;

        Ok(BaseFundingPlan {
            network: funding.network.clone(),
            bounty,
            create,
            funding,
        })
    }

    pub fn plan_base_release(&self, request: PlanBaseReleaseRequest) -> AppResult<BaseReleasePlan> {
        let network = base_plan_network(request.network.as_deref(), "release")?;
        let bounty = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if bounty.status != BountyStatus::Payable {
            return Err(AppError::InvalidBaseReleasePlan(format!(
                "bounty must be Payable before release planning; current status is {:?}",
                bounty.status
            )));
        }

        let escrow = self
            .escrows
            .values()
            .find(|escrow| {
                escrow.bounty_id == request.bounty_id
                    && escrow.rail == PaymentRail::BaseUsdc
                    && is_releasable_base_escrow_status(&escrow.status)
            })
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidBaseReleasePlan(
                    "bounty has no funded or disputed Base USDC escrow".to_string(),
                )
            })?;
        let onchain_escrow_id = parse_base_escrow_reference(&escrow.external_reference)?;
        let settlement = self
            .settlements
            .values()
            .find(|settlement| {
                settlement.bounty_id == request.bounty_id
                    && settlement.rail == PaymentRail::BaseUsdc
                    && settlement
                        .payout_intents
                        .iter()
                        .any(|intent| intent.status == PayoutStatus::Pending)
            })
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidBaseReleasePlan(
                    "bounty has no pending Base settlement".to_string(),
                )
            })?;
        let proof = self
            .proofs
            .get(&settlement.proof_record_id)
            .ok_or_else(|| {
                AppError::InvalidBaseReleasePlan("settlement proof record is missing".to_string())
            })?;

        let mut recipients = settlement
            .payout_intents
            .iter()
            .filter(|intent| intent.rail == PaymentRail::BaseUsdc)
            .filter(|intent| intent.status == PayoutStatus::Pending)
            .map(|intent| {
                let agent = self.agents.get(&intent.recipient_agent_id).ok_or_else(|| {
                    AppError::InvalidBaseReleasePlan(format!(
                        "recipient agent {} is missing",
                        intent.recipient_agent_id
                    ))
                })?;
                let address = agent.payout_wallet.clone().ok_or_else(|| {
                    AppError::InvalidBaseReleasePlan(format!(
                        "recipient agent {} has no payout wallet",
                        agent.id
                    ))
                })?;
                Ok(EscrowRecipient {
                    address,
                    amount: intent.amount.clone(),
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        recipients.push(EscrowRecipient {
            address: request.platform_fee_wallet,
            amount: settlement.platform_fee.clone(),
        });
        BaseEscrowRelease {
            escrow_id: escrow.id,
            recipients: recipients.clone(),
            proof_hash: proof.proof_hash.clone(),
        }
        .validate_split(&settlement_total_amount(&settlement)?)
        .map_err(|error| AppError::InvalidBaseReleasePlan(error.to_string()))?;

        let release_call = BaseEscrowReleaseCall {
            onchain_escrow_id,
            recipients,
            proof_hash: proof.proof_hash.clone(),
        };
        let transaction = BaseEscrowTxPlanner::new(request.escrow_contract)
            .map_err(|error| AppError::InvalidBaseReleasePlan(error.to_string()))?
            .release(&release_call)
            .map_err(|error| AppError::InvalidBaseReleasePlan(error.to_string()))?;

        Ok(BaseReleasePlan {
            network,
            bounty,
            escrow,
            settlement,
            release_call,
            transaction,
        })
    }

    pub fn plan_base_refund(&self, request: PlanBaseRefundRequest) -> AppResult<BaseRefundPlan> {
        let network = base_plan_network(request.network.as_deref(), "refund")?;
        let bounty = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if !matches!(
            bounty.status,
            BountyStatus::Funded
                | BountyStatus::Claimable
                | BountyStatus::Claimed
                | BountyStatus::Submitted
                | BountyStatus::Disputed
                | BountyStatus::Refunding
        ) {
            return Err(AppError::InvalidBaseEscrowPlan(format!(
                "bounty must be funded, claimable, claimed, submitted, disputed, or refunding before refund planning; current status is {:?}",
                bounty.status
            )));
        }

        let escrow = self
            .escrows
            .values()
            .find(|escrow| {
                escrow.bounty_id == request.bounty_id
                    && escrow.rail == PaymentRail::BaseUsdc
                    && is_refundable_base_escrow_status(&escrow.status)
            })
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidBaseEscrowPlan(
                    "bounty has no funded or disputed Base USDC escrow".to_string(),
                )
            })?;
        let onchain_escrow_id = parse_base_escrow_reference(&escrow.external_reference)?;
        let transaction = BaseEscrowTxPlanner::new(request.escrow_contract)
            .map_err(|error| AppError::InvalidBaseEscrowPlan(error.to_string()))?
            .refund(onchain_escrow_id, &request.reason_hash)
            .map_err(|error| AppError::InvalidBaseEscrowPlan(error.to_string()))?;

        Ok(BaseRefundPlan {
            network,
            bounty,
            escrow,
            onchain_escrow_id,
            reason_hash: request.reason_hash,
            transaction,
        })
    }

    pub fn plan_base_dispute(&self, request: PlanBaseDisputeRequest) -> AppResult<BaseDisputePlan> {
        let network = base_plan_network(request.network.as_deref(), "dispute")?;
        let bounty = self
            .bounties
            .get(&request.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if !matches!(
            bounty.status,
            BountyStatus::Submitted | BountyStatus::Verifying
        ) {
            return Err(AppError::InvalidBaseEscrowPlan(format!(
                "bounty must be Submitted or Verifying before dispute planning; current status is {:?}",
                bounty.status
            )));
        }

        let escrow = self
            .escrows
            .values()
            .find(|escrow| {
                escrow.bounty_id == request.bounty_id
                    && escrow.rail == PaymentRail::BaseUsdc
                    && escrow.status == EscrowStatus::Funded
            })
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidBaseEscrowPlan(
                    "bounty has no funded Base USDC escrow to dispute".to_string(),
                )
            })?;
        let onchain_escrow_id = parse_base_escrow_reference(&escrow.external_reference)?;
        let transaction = BaseEscrowTxPlanner::new(request.escrow_contract)
            .map_err(|error| AppError::InvalidBaseEscrowPlan(error.to_string()))?
            .mark_disputed(onchain_escrow_id, &request.dispute_hash)
            .map_err(|error| AppError::InvalidBaseEscrowPlan(error.to_string()))?;

        Ok(BaseDisputePlan {
            network,
            bounty,
            escrow,
            onchain_escrow_id,
            dispute_hash: request.dispute_hash,
            transaction,
        })
    }

    pub fn list_base_release_queue(
        &self,
        request: BaseReleaseQueueRequest,
    ) -> Vec<BaseReleaseQueueItem> {
        let mut items = self
            .settlements
            .values()
            .filter(|settlement| settlement.rail == PaymentRail::BaseUsdc)
            .filter(|settlement| {
                settlement
                    .payout_intents
                    .iter()
                    .any(|intent| intent.status == PayoutStatus::Pending)
            })
            .filter_map(|settlement| self.base_release_queue_item(settlement, &request))
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .pending_amount
                .amount
                .cmp(&left.pending_amount.amount)
                .then_with(|| left.bounty.created_at.cmp(&right.bounty.created_at))
        });
        items
    }

    pub fn list_claimable_bounties(&self) -> Vec<Bounty> {
        self.bounties
            .values()
            .filter(|bounty| self.is_claimable_with_confirmed_funding(bounty))
            .cloned()
            .collect()
    }

    fn base_release_queue_item(
        &self,
        settlement: &Settlement,
        request: &BaseReleaseQueueRequest,
    ) -> Option<BaseReleaseQueueItem> {
        let bounty = self.bounties.get(&settlement.bounty_id)?.clone();
        if bounty.status != BountyStatus::Payable {
            return None;
        }

        let escrow = self
            .escrows
            .values()
            .find(|escrow| {
                escrow.bounty_id == settlement.bounty_id
                    && escrow.rail == PaymentRail::BaseUsdc
                    && is_releasable_base_escrow_status(&escrow.status)
            })
            .cloned();
        let proof = self.proofs.get(&settlement.proof_record_id).cloned();
        let pending_payouts = settlement
            .payout_intents
            .iter()
            .filter(|intent| intent.rail == PaymentRail::BaseUsdc)
            .filter(|intent| intent.status == PayoutStatus::Pending)
            .collect::<Vec<_>>();
        let pending_payout_count = pending_payouts.len();
        if pending_payout_count == 0 {
            return None;
        }
        let pending_total = pending_payouts
            .iter()
            .map(|intent| intent.amount.amount)
            .sum::<i64>();
        let pending_amount = Money::new(pending_total, bounty.amount.currency.clone())
            .expect("payout intents are created from valid bounty amount");
        let missing_recipient_agent_ids = pending_payouts
            .iter()
            .filter_map(|intent| {
                self.agents
                    .get(&intent.recipient_agent_id)
                    .filter(|agent| agent.payout_wallet.is_some())
                    .map(|_| None)
                    .unwrap_or(Some(intent.recipient_agent_id))
            })
            .collect::<Vec<_>>();
        let onchain_escrow_id = escrow
            .as_ref()
            .and_then(|escrow| parse_base_escrow_reference(&escrow.external_reference).ok());

        let mut readiness_error = structural_base_release_error(
            &escrow,
            &proof,
            onchain_escrow_id,
            &missing_recipient_agent_ids,
        );
        let mut release_plan = None;
        if readiness_error.is_none() {
            match (&request.escrow_contract, &request.platform_fee_wallet) {
                (Some(escrow_contract), Some(platform_fee_wallet)) => {
                    match self.plan_base_release(PlanBaseReleaseRequest {
                        bounty_id: bounty.id,
                        escrow_contract: escrow_contract.clone(),
                        platform_fee_wallet: platform_fee_wallet.clone(),
                        network: request.network.clone(),
                    }) {
                        Ok(plan) => release_plan = Some(plan),
                        Err(error) => readiness_error = Some(error.to_string()),
                    }
                }
                _ => {
                    readiness_error = Some(
                        "escrow_contract and platform_fee_wallet are required to build release transaction"
                            .to_string(),
                    );
                }
            }
        }

        Some(BaseReleaseQueueItem {
            bounty,
            settlement: settlement.clone(),
            escrow,
            proof,
            pending_payout_count,
            pending_amount,
            onchain_escrow_id,
            missing_recipient_agent_ids,
            ready: release_plan.is_some(),
            readiness_error,
            release_plan,
        })
    }

    fn settle_payable_bounty(
        &mut self,
        bounty_id: Id,
        proof: &ProofRecord,
        solver_agent_id: Id,
        verifier_agent_id: Option<Id>,
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
            let solver_amount = Money::new(amount.amount * 90 / 100, amount.currency.clone())?;
            let verifier_amount = verifier_agent_id
                .map(|_| Money::new(amount.amount * 5 / 100, amount.currency.clone()))
                .transpose()?;
            let platform_amount = Money::new(
                amount.amount
                    - solver_amount.amount
                    - verifier_amount
                        .as_ref()
                        .map(|amount| amount.amount)
                        .unwrap_or_default(),
                amount.currency.clone(),
            )?;

            let payout_status = match rail {
                PaymentRail::StripeFiat => PayoutStatus::Blocked,
                PaymentRail::BaseUsdc => PayoutStatus::Pending,
                PaymentRail::Simulated => PayoutStatus::Paid,
            };
            all_payouts_paid &= payout_status == PayoutStatus::Paid;

            let mut payout_intents = vec![PayoutIntent {
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
            if let (Some(verifier_agent_id), Some(verifier_amount)) =
                (verifier_agent_id, verifier_amount.clone())
            {
                payout_intents.push(PayoutIntent {
                    id: Uuid::new_v4(),
                    bounty_id,
                    recipient_agent_id: verifier_agent_id,
                    rail: rail.clone(),
                    amount: verifier_amount.clone(),
                    status: payout_status.clone(),
                });
                if payout_status == PayoutStatus::Paid {
                    postings.push(credit(
                        format!("agent_payable:{verifier_agent_id}"),
                        verifier_amount,
                    ));
                }
            }
            if payout_status == PayoutStatus::Paid {
                postings.push(credit("platform_fee", platform_amount.clone()));

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

    fn update_base_escrow_status(&mut self, escrow_id: Id, status: EscrowStatus) -> AppResult<()> {
        let escrow = self.escrows.get_mut(&escrow_id).ok_or_else(|| {
            AppError::InvalidBaseEscrowEvent(
                "terminal escrow event arrived before created event".to_string(),
            )
        })?;
        escrow.status = status;
        Ok(())
    }

    fn mark_base_escrow_funded(
        &mut self,
        amount: Money,
        external_event_id: String,
    ) -> AppResult<Option<LedgerEntry>> {
        if self.ledger.has_external_event(&external_event_id) {
            return Ok(None);
        }
        let entry = LedgerEntry::new(
            "base escrow funded",
            Some(external_event_id),
            vec![
                debit("escrow_asset", amount.clone()),
                credit("bounty_liability", amount),
            ],
        )?;
        self.ledger.append(entry.clone())?;
        Ok(Some(entry))
    }

    fn mark_base_release_paid(
        &mut self,
        bounty_id: Id,
        external_event_id: String,
    ) -> AppResult<Option<LedgerEntry>> {
        if self.ledger.has_external_event(&external_event_id) {
            return Ok(None);
        }

        let bounty = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if bounty.status == BountyStatus::Paid {
            return Ok(None);
        }
        if bounty.status != BountyStatus::Payable {
            return Err(domain::DomainError::InvalidTransition {
                from: format!("{:?}", bounty.status),
                to: "Paid".to_string(),
            }
            .into());
        }

        let settlement_id = self
            .settlements
            .values()
            .find(|settlement| {
                settlement.bounty_id == bounty_id && settlement.rail == PaymentRail::BaseUsdc
            })
            .map(|settlement| settlement.id)
            .ok_or_else(|| {
                AppError::InvalidBaseEscrowEvent(
                    "released event has no pending Base settlement".to_string(),
                )
            })?;
        let settlement = self
            .settlements
            .get(&settlement_id)
            .expect("settlement id selected from map")
            .clone();
        let settlement_amount = settlement_total_amount(&settlement)?;
        let mut postings = vec![debit("bounty_liability", settlement_amount)];
        for intent in &settlement.payout_intents {
            postings.push(credit(
                format!("agent_payable:{}", intent.recipient_agent_id),
                intent.amount.clone(),
            ));
        }
        postings.push(credit("platform_fee", settlement.platform_fee.clone()));

        let entry = LedgerEntry::new("base escrow released", Some(external_event_id), postings)?;
        self.ledger.append(entry.clone())?;

        let settlement = self
            .settlements
            .get_mut(&settlement_id)
            .expect("settlement id selected from map");
        for intent in &mut settlement.payout_intents {
            intent.status = PayoutStatus::Paid;
        }
        self.mark_bounty_paid_if_all_settlements_paid(bounty_id)?;

        Ok(Some(entry))
    }

    fn mark_base_refunded(
        &mut self,
        bounty_id: Id,
        external_event_id: String,
    ) -> AppResult<Option<LedgerEntry>> {
        if self.ledger.has_external_event(&external_event_id) {
            return Ok(None);
        }

        let escrow = self
            .escrows
            .values()
            .find(|escrow| {
                escrow.bounty_id == bounty_id
                    && escrow.rail == PaymentRail::BaseUsdc
                    && is_refundable_base_escrow_status(&escrow.status)
            })
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidBaseEscrowEvent(
                    "refunded event has no funded or disputed Base escrow".to_string(),
                )
            })?;
        let amount = escrow.amount.clone();
        let bounty_snapshot = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        if bounty_snapshot.status == BountyStatus::Refunded {
            return Ok(None);
        }

        {
            let bounty = self
                .bounties
                .get_mut(&bounty_id)
                .ok_or(AppError::BountyNotFound)?;
            if bounty.funding_mode == FundingMode::MixedRails {
                match bounty.status {
                    BountyStatus::Unfunded | BountyStatus::Funded | BountyStatus::Claimable => {
                        bounty.reopen_for_funding()?;
                    }
                    BountyStatus::Claimed
                    | BountyStatus::Submitted
                    | BountyStatus::Verifying
                    | BountyStatus::Disputed => {
                        bounty.mark_payment_disputed()?;
                    }
                    BountyStatus::Accepted
                    | BountyStatus::Payable
                    | BountyStatus::Paid
                    | BountyStatus::Refunding
                    | BountyStatus::Refunded
                    | BountyStatus::Expired => {
                        return Err(domain::DomainError::InvalidTransition {
                            from: format!("{:?}", bounty.status),
                            to: "Refunded".to_string(),
                        }
                        .into());
                    }
                }
            } else {
                if bounty.status != BountyStatus::Refunding {
                    bounty.refunding()?;
                }
                bounty.mark_refunded()?;
            }
        }

        let entry = LedgerEntry::new(
            "base escrow refunded",
            Some(external_event_id),
            vec![
                debit("bounty_liability", amount.clone()),
                credit("escrow_asset", amount),
            ],
        )?;
        self.ledger.append(entry.clone())?;

        Ok(Some(entry))
    }

    fn mark_base_disputed(&mut self, bounty_id: Id) -> AppResult<()> {
        let status = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .status
            .clone();
        if matches!(status, BountyStatus::Submitted | BountyStatus::Verifying) {
            self.bounties
                .get_mut(&bounty_id)
                .ok_or(AppError::BountyNotFound)?
                .dispute()?;
        }
        Ok(())
    }

    fn mark_stripe_agent_payouts_paid(
        &mut self,
        settlement_id: Id,
        agent_id: Id,
        connected_account_id: &str,
    ) -> AppResult<Vec<LedgerEntry>> {
        let settlement = self
            .settlements
            .get(&settlement_id)
            .ok_or_else(|| AppError::InvalidStripePayout("settlement not found".to_string()))?
            .clone();
        let mut entries = Vec::new();
        for intent in settlement
            .payout_intents
            .iter()
            .filter(|intent| intent.recipient_agent_id == agent_id)
            .filter(|intent| intent.status != PayoutStatus::Paid)
        {
            let external_event_id =
                format!("stripe-connect-payout:{connected_account_id}:{}", intent.id);
            if self.ledger.has_external_event(&external_event_id) {
                continue;
            }
            let entry = LedgerEntry::new(
                "stripe connect payout eligible",
                Some(external_event_id),
                vec![
                    debit("bounty_liability", intent.amount.clone()),
                    credit(format!("agent_payable:{agent_id}"), intent.amount.clone()),
                ],
            )?;
            self.ledger.append(entry.clone())?;
            entries.push(entry);
        }

        if let Some(settlement) = self.settlements.get_mut(&settlement_id) {
            for intent in settlement
                .payout_intents
                .iter_mut()
                .filter(|intent| intent.recipient_agent_id == agent_id)
            {
                intent.status = PayoutStatus::Paid;
            }
        }

        Ok(entries)
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
        let should_mark_paid = self
            .bounties
            .get(&settlement.bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .status
            == BountyStatus::Payable;
        if should_mark_paid {
            self.mark_bounty_paid_if_all_settlements_paid(settlement.bounty_id)?;
        }
        Ok(Some(entry))
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
        FundingMode::BaseUsdcEscrow => Ok(PaymentRail::BaseUsdc),
        FundingMode::StripeFiatLedger => Ok(PaymentRail::StripeFiat),
        FundingMode::MixedRails => Err(AppError::InvalidFundingContribution(
            "mixed rail bounty requires explicit funding targets".to_string(),
        )),
    }
}

fn funding_targets_from_request(
    funding_mode: &FundingMode,
    amount: &Money,
    requested_targets: &[FundingPartitionTargetRequest],
) -> AppResult<Vec<FundingPartitionTarget>> {
    if *funding_mode != FundingMode::MixedRails {
        if !requested_targets.is_empty() {
            return Err(AppError::InvalidFundingContribution(
                "funding_targets are only valid for MixedRails pooled bounties".to_string(),
            ));
        }
        return Ok(Vec::new());
    }

    if requested_targets.is_empty() {
        return Err(AppError::InvalidFundingContribution(
            "MixedRails pooled bounties require explicit funding_targets".to_string(),
        ));
    }

    let mut targets = Vec::new();
    for target in requested_targets {
        if target.rail == PaymentRail::Simulated {
            return Err(AppError::InvalidFundingContribution(
                "MixedRails funding targets must use real payment rails".to_string(),
            ));
        }
        let money = Money::new(target.amount_minor, target.currency.clone())?;
        if targets.iter().any(|existing: &FundingPartitionTarget| {
            existing.rail == target.rail && existing.amount.currency == money.currency
        }) {
            return Err(AppError::InvalidFundingContribution(format!(
                "duplicate {:?} {} funding target",
                target.rail, money.currency
            )));
        }
        targets.push(FundingPartitionTarget {
            rail: target.rail.clone(),
            amount: money,
        });
    }

    if targets
        .iter()
        .any(|target| target.amount.currency == amount.currency)
    {
        let display_currency_total = targets
            .iter()
            .filter(|target| target.amount.currency == amount.currency)
            .map(|target| target.amount.amount)
            .sum::<i64>();
        if display_currency_total != amount.amount {
            return Err(AppError::InvalidFundingContribution(format!(
                "MixedRails display target must equal confirmed targets in {}; expected {}, got {}",
                amount.currency, display_currency_total, amount.amount
            )));
        }
    }

    Ok(targets)
}

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
        "fix-ci-failure" => CapabilityClass::Ci,
        "run-browser-workflow" => CapabilityClass::BrowserWorkflow,
        _ => CapabilityClass::Coding,
    }
}

fn verifier_kind_for_template(template_slug: &str) -> VerifierKind {
    match template_slug {
        "fix-ci-failure" | "small-code-change" => VerifierKind::GitHubCi,
        "extract-data-to-schema" => VerifierKind::JsonSchema,
        "run-browser-workflow" => VerifierKind::DockerCommand,
        "write-docs-for-area" => VerifierKind::AiJudgeFilter,
        "independent-claim-verification" | "primary-source-research" => VerifierKind::Manual,
        _ => VerifierKind::Manual,
    }
}

fn validate_created_base_escrow(
    bounty: &Bounty,
    target: &FundingPartitionTarget,
    amount: &Money,
    terms_hash: &str,
) -> AppResult<()> {
    if target.rail != PaymentRail::BaseUsdc || &target.amount != amount {
        return Err(AppError::InvalidBaseEscrowEvent(
            "created event amount does not match Base USDC funding target".to_string(),
        ));
    }
    if let Some(expected_terms_hash) = &bounty.terms_hash {
        if normalize_hash(expected_terms_hash) != normalize_hash(terms_hash) {
            return Err(AppError::InvalidBaseEscrowEvent(
                "created event terms hash does not match bounty terms".to_string(),
            ));
        }
    }
    Ok(())
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

    fn base_funding_target(&self, bounty: &Bounty) -> AppResult<FundingPartitionTarget> {
        self.effective_funding_targets(bounty)?
            .into_iter()
            .find(|target| target.rail == PaymentRail::BaseUsdc)
            .ok_or_else(|| {
                AppError::InvalidBaseFundingPlan(
                    "bounty has no Base USDC funding target".to_string(),
                )
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

    fn mark_matching_base_funding_intent_applied(&mut self, bounty_id: Id) -> AppResult<()> {
        let bounty = self
            .bounties
            .get(&bounty_id)
            .ok_or(AppError::BountyNotFound)?
            .clone();
        let target = self.base_funding_target(&bounty)?;
        if self.confirmed_funding_for_target(&bounty, &target) < target.amount.amount {
            return Ok(());
        }
        for intent in self.funding_intents.values_mut().filter(|intent| {
            intent.bounty_id == bounty_id
                && intent.rail == PaymentRail::BaseUsdc
                && intent.amount == target.amount
                && intent.status == FundingIntentStatus::AwaitingEvidence
        }) {
            intent.status = FundingIntentStatus::Applied;
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

    fn has_funded_base_escrow(&self, bounty_id: Id) -> bool {
        self.escrows.values().any(|escrow| {
            escrow.bounty_id == bounty_id
                && escrow.rail == PaymentRail::BaseUsdc
                && escrow.status == EscrowStatus::Funded
        })
    }
}

fn base_escrow_uuid(bounty_id: Id, onchain_escrow_id: u128) -> Id {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("agent-bounties:base:{bounty_id}:{onchain_escrow_id}").as_bytes(),
    )
}

fn base_escrow_reference(onchain_escrow_id: u128) -> String {
    format!("base:{onchain_escrow_id}")
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

fn required_base_field(value: Option<String>, field: &str) -> AppResult<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AppError::InvalidFundingIntent(format!("{field} is required for Base USDC intents"))
        })
}

fn parse_base_escrow_reference(reference: &Option<String>) -> AppResult<u128> {
    let value = reference.as_deref().ok_or_else(|| {
        AppError::InvalidBaseEscrowPlan("Base escrow is missing external reference".to_string())
    })?;
    value
        .strip_prefix("base:")
        .ok_or_else(|| {
            AppError::InvalidBaseEscrowPlan(format!(
                "invalid Base escrow external reference: {value}"
            ))
        })?
        .parse()
        .map_err(|_| {
            AppError::InvalidBaseEscrowPlan(format!(
                "invalid Base escrow external reference: {value}"
            ))
        })
}

fn base_plan_network(network: Option<&str>, plan_kind: &str) -> AppResult<BaseNetworkDescriptor> {
    base_network_descriptor(network.unwrap_or("base-sepolia")).map_err(|error| {
        let message = error.to_string();
        match plan_kind {
            "release" => AppError::InvalidBaseReleasePlan(message),
            _ => AppError::InvalidBaseEscrowPlan(message),
        }
    })
}

fn is_releasable_base_escrow_status(status: &EscrowStatus) -> bool {
    matches!(status, EscrowStatus::Funded | EscrowStatus::Disputed)
}

fn is_refundable_base_escrow_status(status: &EscrowStatus) -> bool {
    matches!(status, EscrowStatus::Funded | EscrowStatus::Disputed)
}

fn structural_base_release_error(
    escrow: &Option<Escrow>,
    proof: &Option<ProofRecord>,
    onchain_escrow_id: Option<u128>,
    missing_recipient_agent_ids: &[Id],
) -> Option<String> {
    if escrow.is_none() {
        return Some("funded or disputed Base USDC escrow is missing".to_string());
    }
    if onchain_escrow_id.is_none() {
        return Some("funded Base USDC escrow has invalid external reference".to_string());
    }
    if proof.is_none() {
        return Some("settlement proof record is missing".to_string());
    }
    if !missing_recipient_agent_ids.is_empty() {
        return Some(format!(
            "recipient agents missing payout wallets: {}",
            missing_recipient_agent_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    None
}

fn normalize_hash(value: &str) -> String {
    value
        .strip_prefix("0x")
        .unwrap_or(value)
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fund_base_bounty(
        network: &mut BountyNetwork,
        bounty: &Bounty,
        onchain_escrow_id: u128,
    ) -> BaseEscrowReconciliation {
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                onchain_escrow_id,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap()
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

    #[tokio::test]
    async fn funding_intents_assign_stripe_after_webhook_and_base_after_escrow_log() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                title: "Fund mixed intent bounty".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::MixedRails,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::StripeFiat,
                        amount_minor: 500,
                        currency: "usd".to_string(),
                    },
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::BaseUsdc,
                        amount_minor: 1_000,
                        currency: "usdc".to_string(),
                    },
                ],
            })
            .unwrap();

        let stripe_intent = network
            .create_funding_intent(
                CreateFundingIntentRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: Some(organization_id),
                    amount_minor: 500,
                    currency: "usd".to_string(),
                    rail: PaymentRail::StripeFiat,
                    external_reference: Some("intent-stripe-500".to_string()),
                    stripe_success_url: None,
                    stripe_cancel_url: None,
                    base_escrow_contract: None,
                    base_payer: None,
                    base_token: None,
                    base_network: None,
                },
                "https://network.example",
            )
            .unwrap();
        assert_eq!(
            stripe_intent.intent.status,
            FundingIntentStatus::AwaitingEvidence
        );
        assert!(stripe_intent.requires_reconciliation);
        assert!(!stripe_intent.funding_summary.claimable);
        let checkout = match &stripe_intent.next_action {
            FundingIntentNextAction::StripeCheckout { request } => request,
            FundingIntentNextAction::BaseEscrowFunding { .. } => panic!("expected Stripe action"),
        };
        assert_eq!(
            checkout.body["metadata"]["funding_intent_id"],
            stripe_intent.intent.id.to_string()
        );
        assert_eq!(
            checkout.body["metadata"]["bounty_id"],
            bounty.id.to_string()
        );

        let stripe_event = payments_stripe::StripeWebhookEvent {
            id: "evt_intent_paid".to_string(),
            event_type: "checkout.session.completed".to_string(),
            payload: serde_json::json!({
                "id": "cs_intent_paid",
                "client_reference_id": organization_id.to_string(),
                "amount_total": 500,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_intent_paid",
                "metadata": {
                    "bounty_id": bounty.id.to_string(),
                    "funding_intent_id": stripe_intent.intent.id.to_string()
                }
            }),
        };
        let stripe_credit = payments_stripe::StripeEventDeduper::default()
            .apply_checkout_top_up(&stripe_event)
            .unwrap();
        let stripe_reconciliation = network.apply_stripe_funding_credit(stripe_credit).unwrap();
        assert!(!stripe_reconciliation.duplicate);
        assert!(stripe_reconciliation.funding_report.is_some());
        assert_eq!(
            stripe_reconciliation.funding_intent.unwrap().status,
            FundingIntentStatus::Applied
        );
        assert_eq!(stripe_reconciliation.ledger_entries.len(), 2);
        assert_eq!(
            network
                .apply_stripe_funding_credit(stripe_reconciliation.funding_credit.clone())
                .unwrap()
                .duplicate,
            true
        );

        let after_stripe = network.status(bounty.id).unwrap();
        assert_eq!(after_stripe.bounty.status, BountyStatus::Unfunded);
        assert!(!after_stripe.funding_summary.claimable);
        assert_eq!(after_stripe.funding_contributions.len(), 1);
        assert_eq!(
            after_stripe.funding_intents[0].status,
            FundingIntentStatus::Applied
        );

        let base_intent = network
            .create_funding_intent(
                CreateFundingIntentRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: None,
                    amount_minor: 1_000,
                    currency: "usdc".to_string(),
                    rail: PaymentRail::BaseUsdc,
                    external_reference: Some("intent-base-1000".to_string()),
                    stripe_success_url: None,
                    stripe_cancel_url: None,
                    base_escrow_contract: Some(
                        "0x1111111111111111111111111111111111111111".to_string(),
                    ),
                    base_payer: Some("0x2222222222222222222222222222222222222222".to_string()),
                    base_token: Some("0x3333333333333333333333333333333333333333".to_string()),
                    base_network: Some("base-sepolia".to_string()),
                },
                "https://network.example",
            )
            .unwrap();
        assert_eq!(
            base_intent.intent.status,
            FundingIntentStatus::AwaitingEvidence
        );
        let base_plan = match &base_intent.next_action {
            FundingIntentNextAction::BaseEscrowFunding { plan } => plan,
            FundingIntentNextAction::StripeCheckout { .. } => panic!("expected Base action"),
        };
        assert_eq!(base_plan.network.chain_id, 84_532);
        assert_eq!(
            network.status(bounty.id).unwrap().bounty.status,
            BountyStatus::Unfunded
        );

        let base_created = network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                77,
                "0x3333333333333333333333333333333333333333",
                Money::new(1_000, "usdc").unwrap(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        assert!(base_created
            .funding_intents
            .iter()
            .any(|intent| intent.id == base_intent.intent.id
                && intent.status == FundingIntentStatus::Applied));
        let claimable = network.status(bounty.id).unwrap();
        assert_eq!(claimable.bounty.status, BountyStatus::Claimable);
        assert!(claimable.funding_summary.claimable);
    }

    #[test]
    fn funding_intents_reject_duplicate_reference_and_partial_base_partition() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                title: "Reject bad funding intents".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::MixedRails,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::StripeFiat,
                        amount_minor: 500,
                        currency: "usd".to_string(),
                    },
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::BaseUsdc,
                        amount_minor: 1_000,
                        currency: "usdc".to_string(),
                    },
                ],
            })
            .unwrap();

        let stripe_request = CreateFundingIntentRequest {
            bounty_id: bounty.id,
            contributor_agent_id: None,
            source_organization_id: Some(organization_id),
            amount_minor: 500,
            currency: "usd".to_string(),
            rail: PaymentRail::StripeFiat,
            external_reference: Some("duplicate-reference".to_string()),
            stripe_success_url: None,
            stripe_cancel_url: None,
            base_escrow_contract: None,
            base_payer: None,
            base_token: None,
            base_network: None,
        };
        network
            .create_funding_intent(stripe_request.clone(), "https://network.example")
            .unwrap();
        assert!(matches!(
            network
                .create_funding_intent(stripe_request, "https://network.example")
                .unwrap_err(),
            AppError::InvalidFundingIntent(_)
        ));

        let err = network
            .create_funding_intent(
                CreateFundingIntentRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: None,
                    amount_minor: 500,
                    currency: "usdc".to_string(),
                    rail: PaymentRail::BaseUsdc,
                    external_reference: Some("partial-base".to_string()),
                    stripe_success_url: None,
                    stripe_cancel_url: None,
                    base_escrow_contract: Some(
                        "0x1111111111111111111111111111111111111111".to_string(),
                    ),
                    base_payer: Some("0x2222222222222222222222222222222222222222".to_string()),
                    base_token: Some("0x3333333333333333333333333333333333333333".to_string()),
                    base_network: Some("base-sepolia".to_string()),
                },
                "https://network.example",
            )
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidFundingIntent(_)));
    }

    #[tokio::test]
    async fn full_in_memory_paid_bounty_loop() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Extract data".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();

        assert_eq!(bounty.status, BountyStatus::Unfunded);
        assert!(bounty.terms_hash.is_some());
        assert!(network.list_claimable_bounties().is_empty());
        assert!(matches!(
            network
                .claim_bounty(ClaimBountyRequest {
                    bounty_id: bounty.id,
                    solver_agent_id: solver.id,
                })
                .unwrap_err(),
            AppError::InvalidBaseEscrowEvent(_)
        ));

        let funding_plan = network
            .plan_base_funding(PlanBaseFundingRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                payer: "0x2222222222222222222222222222222222222222".to_string(),
                token: "0x3333333333333333333333333333333333333333".to_string(),
                network: Some("base-mainnet".to_string()),
            })
            .unwrap();
        assert_eq!(funding_plan.network.chain_id, 8_453);
        assert_eq!(funding_plan.bounty.id, bounty.id);
        assert_eq!(funding_plan.create.bounty_id, bounty.id);
        assert_eq!(
            funding_plan.create.terms_hash,
            bounty.terms_hash.clone().unwrap()
        );
        assert_eq!(
            funding_plan.funding.create_escrow.function,
            "createEscrow(bytes32,address,uint256,bytes32)"
        );
        assert!(funding_plan
            .funding
            .create_escrow
            .data
            .contains(&bounty.id.simple().to_string()));

        let funding_reconciliation = fund_base_bounty(&mut network, &bounty, 1);
        assert_eq!(
            funding_reconciliation.bounty.status,
            BountyStatus::Claimable
        );
        assert_eq!(funding_reconciliation.ledger_entries.len(), 1);
        assert_eq!(network.ledger.entries().len(), 1);
        assert_eq!(network.list_claimable_bounties().len(), 1);
        assert!(matches!(
            network
                .plan_base_funding(PlanBaseFundingRequest {
                    bounty_id: bounty.id,
                    escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                    payer: "0x2222222222222222222222222222222222222222".to_string(),
                    token: "0x3333333333333333333333333333333333333333".to_string(),
                    network: None,
                })
                .unwrap_err(),
            AppError::InvalidBaseFundingPlan(_)
        ));

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
                artifact_uri: "s3://local/artifact.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        let proof = network
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
        assert_eq!(status.claims.len(), 1);
        assert_eq!(network.ledger.entries().len(), 1);
        assert_eq!(status.settlements.len(), 1);
        assert_eq!(status.reputation_events.len(), 1);
        assert_eq!(status.template_signals.len(), 1);
        assert_eq!(status.settlements[0].payout_intents.len(), 1);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Pending
        );
        assert_eq!(status.reputation_events[0].agent_id, solver.id);
        assert_eq!(
            status.reputation_events[0].template_slug,
            "extract-data-to-schema"
        );
        assert_eq!(
            status.template_signals[0].template_slug,
            "extract-data-to-schema"
        );
        assert_eq!(
            status.template_signals[0].capability_class,
            CapabilityClass::Extraction
        );
        assert_eq!(
            status.template_signals[0].verifier_kind,
            VerifierKind::JsonSchema
        );
        assert_eq!(status.template_signals[0].amount.amount, 1_000_000);
        assert!(status.template_signals[0].success);
        let queue = network.list_base_release_queue(BaseReleaseQueueRequest {
            escrow_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
            platform_fee_wallet: Some("0x4444444444444444444444444444444444444444".to_string()),
            network: Some("base-mainnet".to_string()),
        });
        assert_eq!(queue.len(), 1);
        assert!(queue[0].ready);
        assert!(queue[0].readiness_error.is_none());
        assert_eq!(queue[0].onchain_escrow_id, Some(1));
        assert_eq!(queue[0].pending_payout_count, 1);
        assert_eq!(queue[0].pending_amount.amount, 900_000);
        assert!(queue[0].release_plan.is_some());
        let pending_agent_payouts = network.agent_payout_status(solver.id).unwrap();
        assert_eq!(pending_agent_payouts.agent.id, solver.id);
        assert_eq!(pending_agent_payouts.payouts.len(), 1);
        assert_eq!(pending_agent_payouts.payouts[0].bounty_id, bounty.id);
        assert_eq!(
            pending_agent_payouts.payouts[0].status,
            PayoutStatus::Pending
        );
        assert_eq!(pending_agent_payouts.totals.len(), 1);
        assert_eq!(pending_agent_payouts.totals[0].currency, "usdc");
        assert_eq!(pending_agent_payouts.totals[0].pending_minor, 900_000);
        assert_eq!(pending_agent_payouts.totals[0].paid_minor, 0);
        assert_eq!(pending_agent_payouts.reputation_events.len(), 1);
        let release_plan = network
            .plan_base_release(PlanBaseReleaseRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                platform_fee_wallet: "0x4444444444444444444444444444444444444444".to_string(),
                network: Some("base-mainnet".to_string()),
            })
            .unwrap();
        assert_eq!(release_plan.network.name, "Base");
        assert_eq!(release_plan.network.chain_id, 8_453);
        assert_eq!(release_plan.release_call.onchain_escrow_id, 1);
        assert_eq!(release_plan.release_call.recipients.len(), 2);
        assert_eq!(
            release_plan.release_call.recipients[0].address,
            "0x2222222222222222222222222222222222222222"
        );
        assert_eq!(
            release_plan.release_call.recipients[0].amount.amount,
            900_000
        );
        assert_eq!(
            release_plan.release_call.recipients[1].amount.amount,
            100_000
        );
        assert!(release_plan.transaction.data.starts_with("0xbfc95334"));
        let released = chain_base::simulated_released_event(bounty.id, 1, proof.proof_hash);
        let reconciliation = network.apply_base_escrow_event(released.clone()).unwrap();
        assert_eq!(reconciliation.ledger_entries.len(), 1);

        let paid_status = network.status(bounty.id).unwrap();
        assert_eq!(paid_status.bounty.status, BountyStatus::Paid);
        assert_eq!(paid_status.escrows[0].status, EscrowStatus::Released);
        assert_eq!(
            paid_status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
        let paid_agent_payouts = network.agent_payout_status(solver.id).unwrap();
        assert_eq!(paid_agent_payouts.payouts[0].status, PayoutStatus::Paid);
        assert_eq!(paid_agent_payouts.totals[0].pending_minor, 0);
        assert_eq!(paid_agent_payouts.totals[0].paid_minor, 900_000);
        assert_eq!(paid_status.template_signals.len(), 1);
        assert_eq!(network.ledger.entries().len(), 2);

        let replay = network.apply_base_escrow_event(released).unwrap();
        assert!(replay.ledger_entries.is_empty());
        assert_eq!(network.ledger.entries().len(), 2);
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
    async fn base_release_queue_reports_missing_payout_wallet() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Extract data".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        fund_base_bounty(&mut network, &bounty, 1);
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
                artifact_uri: "s3://local/artifact.json".to_string(),
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
        let queue = network.list_base_release_queue(BaseReleaseQueueRequest {
            escrow_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
            platform_fee_wallet: Some("0x4444444444444444444444444444444444444444".to_string()),
            network: None,
        });

        assert_eq!(queue.len(), 1);
        assert!(!queue[0].ready);
        assert_eq!(queue[0].missing_recipient_agent_ids, vec![solver.id]);
        assert!(queue[0]
            .readiness_error
            .as_ref()
            .expect("readiness error")
            .contains("missing payout wallets"));
        assert!(queue[0].release_plan.is_none());
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
                funding_mode: None,
            })
            .unwrap();

        assert_eq!(bounty.status, BountyStatus::Unfunded);
        assert!(bounty.terms_hash.is_some());
        assert!(network.list_claimable_bounties().is_empty());
        let funded = fund_base_bounty(&mut network, &bounty, 1);
        assert_eq!(funded.bounty.status, BountyStatus::Claimable);
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
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        fund_base_bounty(&mut network, &bounty, 1);
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
        assert_eq!(status.bounty.status, BountyStatus::Payable);
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
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let second = network
            .post_funded_bounty(PostBountyRequest {
                title: "Extract second artifact".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        fund_base_bounty(&mut network, &first, 1);
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
            BountyStatus::Unfunded
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

        let paid = network
            .apply_stripe_connect_snapshot(ConnectAccountSnapshot {
                agent_id: solver.id,
                connected_account_id: Some("acct_test".to_string()),
                payouts_enabled: true,
                disabled_reason: None,
                currently_due: vec![],
            })
            .unwrap();
        assert!(paid.payout_state.eligible);
        assert_eq!(paid.ledger_entries.len(), 2);

        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
        assert_eq!(network.ledger.entries().len(), 4);

        let replay = network
            .apply_stripe_connect_snapshot(ConnectAccountSnapshot {
                agent_id: solver.id,
                connected_account_id: Some("acct_test".to_string()),
                payouts_enabled: true,
                disabled_reason: None,
                currently_due: vec![],
            })
            .unwrap();
        assert!(replay.ledger_entries.is_empty());
        assert_eq!(network.ledger.entries().len(), 4);
    }

    #[tokio::test]
    async fn mixed_stripe_and_base_partitions_settle_by_rail_after_one_proof() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "mixed-solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                title: "Implement mixed funding fixture".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::MixedRails,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::StripeFiat,
                        amount_minor: 500,
                        currency: "usd".to_string(),
                    },
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::BaseUsdc,
                        amount_minor: 1_000,
                        currency: "usdc".to_string(),
                    },
                ],
            })
            .unwrap();

        network
            .apply_stripe_funding_credit(stripe_funding_credit(
                organization_id,
                500,
                "usd",
                "evt_mixed_topup",
            ))
            .unwrap();
        let stripe = network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(organization_id),
                amount_minor: 500,
                currency: "usd".to_string(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some("mixed-stripe-500".to_string()),
            })
            .unwrap();
        assert_eq!(stripe.bounty.status, BountyStatus::Unfunded);
        assert!(!stripe.funding_summary.claimable);
        assert_eq!(stripe.funding_summary.partitions.len(), 2);
        assert_eq!(
            stripe
                .funding_summary
                .partitions
                .iter()
                .find(|partition| partition.rail == PaymentRail::StripeFiat)
                .unwrap()
                .remaining
                .amount,
            0
        );

        let base_created = network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                42,
                "0x3333333333333333333333333333333333333333",
                Money::new(1_000, "usdc").unwrap(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        assert_eq!(base_created.bounty.status, BountyStatus::Claimable);
        let status = network.status(bounty.id).unwrap();
        assert!(status.funding_summary.claimable);
        assert!(status
            .funding_summary
            .partitions
            .iter()
            .all(|partition| partition.remaining.amount == 0));

        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"mixed\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "memory://mixed-artifact".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        let proof = network
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
        assert_eq!(status.settlements.len(), 2);
        let stripe_settlement = status
            .settlements
            .iter()
            .find(|settlement| settlement.rail == PaymentRail::StripeFiat)
            .unwrap();
        let base_settlement = status
            .settlements
            .iter()
            .find(|settlement| settlement.rail == PaymentRail::BaseUsdc)
            .unwrap();
        assert_eq!(stripe_settlement.platform_fee.currency, "usd");
        assert_eq!(base_settlement.platform_fee.currency, "usdc");
        assert_eq!(
            status.funding_contributions[0].settlement_id,
            Some(stripe_settlement.id)
        );
        assert_eq!(
            base_settlement.payout_intents[0].status,
            PayoutStatus::Pending
        );
        assert_eq!(
            stripe_settlement.payout_intents[0].status,
            PayoutStatus::Blocked
        );

        let release_plan = network
            .plan_base_release(PlanBaseReleaseRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                platform_fee_wallet: "0x5555555555555555555555555555555555555555".to_string(),
                network: Some("base-sepolia".to_string()),
            })
            .unwrap();
        assert_eq!(release_plan.release_call.recipients.len(), 2);
        network
            .apply_base_escrow_event(chain_base::simulated_released_event(
                bounty.id,
                42,
                proof.proof_hash,
            ))
            .unwrap();
        let after_base = network.status(bounty.id).unwrap();
        assert_eq!(after_base.bounty.status, BountyStatus::Payable);
        assert_eq!(
            after_base
                .settlements
                .iter()
                .find(|settlement| settlement.rail == PaymentRail::BaseUsdc)
                .unwrap()
                .payout_intents[0]
                .status,
            PayoutStatus::Paid
        );

        network
            .apply_stripe_connect_snapshot(ConnectAccountSnapshot {
                agent_id: solver.id,
                connected_account_id: Some("acct_mixed".to_string()),
                payouts_enabled: true,
                disabled_reason: None,
                currently_due: vec![],
            })
            .unwrap();
        let paid = network.status(bounty.id).unwrap();
        assert_eq!(paid.bounty.status, BountyStatus::Paid);
        assert!(paid
            .settlements
            .iter()
            .flat_map(|settlement| &settlement.payout_intents)
            .all(|intent| intent.status == PayoutStatus::Paid));
    }

    #[test]
    fn mixed_base_refund_reopens_base_partition_without_refunding_stripe_partition() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "mixed-refund-solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                title: "Recover mixed funding after Base refund".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::MixedRails,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::StripeFiat,
                        amount_minor: 500,
                        currency: "usd".to_string(),
                    },
                    FundingPartitionTargetRequest {
                        rail: PaymentRail::BaseUsdc,
                        amount_minor: 1_000,
                        currency: "usdc".to_string(),
                    },
                ],
            })
            .unwrap();

        network
            .apply_stripe_funding_credit(stripe_funding_credit(
                organization_id,
                500,
                "usd",
                "evt_mixed_refund_topup",
            ))
            .unwrap();
        network
            .add_funding_contribution(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(organization_id),
                amount_minor: 500,
                currency: "usd".to_string(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some("mixed-refund-stripe-500".to_string()),
            })
            .unwrap();
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                42,
                "0x3333333333333333333333333333333333333333",
                Money::new(1_000, "usdc").unwrap(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        assert_eq!(
            network.status(bounty.id).unwrap().bounty.status,
            BountyStatus::Claimable
        );
        assert_eq!(network.list_claimable_bounties().len(), 1);

        let refunded = network
            .apply_base_escrow_event(chain_base::simulated_refunded_event(
                bounty.id,
                42,
                format!("0x{}", "ab".repeat(32)),
            ))
            .unwrap();
        assert_eq!(refunded.bounty.status, BountyStatus::Unfunded);
        assert_eq!(refunded.escrow.status, EscrowStatus::Refunded);
        assert_eq!(refunded.ledger_entries.len(), 1);
        assert!(network.list_claimable_bounties().is_empty());

        let status = network.status(bounty.id).unwrap();
        let stripe_partition = status
            .funding_summary
            .partitions
            .iter()
            .find(|partition| partition.rail == PaymentRail::StripeFiat)
            .unwrap();
        let base_partition = status
            .funding_summary
            .partitions
            .iter()
            .find(|partition| partition.rail == PaymentRail::BaseUsdc)
            .unwrap();
        assert_eq!(stripe_partition.remaining.amount, 0);
        assert_eq!(base_partition.remaining.amount, 1_000);
        assert!(!status.funding_summary.claimable);
        assert_eq!(
            status.funding_contributions[0].status,
            FundingContributionStatus::Applied
        );
        assert!(status
            .escrows
            .iter()
            .any(|escrow| escrow.status == EscrowStatus::Refunded));

        let err = network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidBaseEscrowEvent(_)));

        let refilled = network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                43,
                "0x3333333333333333333333333333333333333333",
                Money::new(1_000, "usdc").unwrap(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        assert_eq!(refilled.bounty.status, BountyStatus::Claimable);
        assert_eq!(network.list_claimable_bounties().len(), 1);
        assert_eq!(network.status(bounty.id).unwrap().escrows.len(), 2);
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
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();

        fund_base_bounty(&mut network, &bounty, 1);
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

    #[test]
    fn high_value_base_bounty_requires_review_before_claimable() {
        let mut network = BountyNetwork::default();
        let err = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap_err();

        assert!(matches!(err, AppError::RiskNeedsReview(_)));
        assert!(network.bounties.is_empty());
        assert_eq!(network.risk_events.len(), 1);

        let events = network.list_risk_events(RiskEventFilter {
            action: Some(RiskAction::NeedsReview),
            surface: Some(domain::RiskSurface::Bounty),
            limit: Some(1),
            ..RiskEventFilter::default()
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, RiskAction::NeedsReview);
        assert!(events[0].reasons[0].contains("low-value cap"));
    }

    #[test]
    fn operator_can_approve_reviewed_bounty_into_claimable_state() {
        let mut network = BountyNetwork::default();
        let err = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::RiskNeedsReview(_)));
        let risk_event = network
            .list_risk_events(RiskEventFilter {
                action: Some(RiskAction::NeedsReview),
                surface: Some(RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .pop()
            .unwrap();

        let approval = network
            .approve_risk_bounty(ApproveRiskBountyRequest {
                risk_event_id: risk_event.id,
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
                operator_id: "operator-1".to_string(),
                note: "Approved high-value testnet bounty after manual scope review".to_string(),
            })
            .unwrap();

        assert_eq!(approval.bounty.id, risk_event.subject_id);
        assert_eq!(approval.bounty.status, BountyStatus::Unfunded);
        assert!(approval.bounty.terms_hash.is_some());
        assert_eq!(approval.review.outcome, RiskReviewOutcome::Approved);
        assert_eq!(network.bounties.len(), 1);
        assert_eq!(network.risk_reviews.len(), 1);
        assert!(network.ledger.entries().is_empty());

        let funded = fund_base_bounty(&mut network, &approval.bounty, 99);
        assert_eq!(funded.bounty.status, BountyStatus::Claimable);
        assert!(network
            .ledger
            .has_external_event("base-fund:base:99:created"));
    }

    #[tokio::test]
    async fn operator_can_approve_high_value_payout_risk_before_verification() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let err = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::RiskNeedsReview(_)));
        let bounty_event = network
            .list_risk_events(RiskEventFilter {
                action: Some(RiskAction::NeedsReview),
                surface: Some(RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .pop()
            .unwrap();
        let approval = network
            .approve_risk_bounty(ApproveRiskBountyRequest {
                risk_event_id: bounty_event.id,
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
                operator_id: "operator-1".to_string(),
                note: "Approved high-value bounty scope".to_string(),
            })
            .unwrap();

        fund_base_bounty(&mut network, &approval.bounty, 99);
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: approval.bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: approval.bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "https://github.com/example/repo/pull/1".to_string(),
                artifact_body: "{\"check\":\"green\"}".to_string(),
            })
            .unwrap();

        let err = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: approval.bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: "not-used-by-github-ci".to_string(),
                verifier_kind: None,
                rubric: None,
                evidence: Some(github_ci_evidence()),
                approved_risk_event_id: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::RiskNeedsReview(_)));
        assert_eq!(
            network.status(approval.bounty.id).unwrap().bounty.status,
            BountyStatus::Submitted
        );

        let payout_event = network
            .list_risk_events(RiskEventFilter {
                action: Some(RiskAction::NeedsReview),
                surface: Some(RiskSurface::Payout),
                bounty_id: Some(approval.bounty.id),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .pop()
            .unwrap();
        let payout_review = network
            .approve_risk_payout(ApproveRiskPayoutRequest {
                risk_event_id: payout_event.id,
                operator_id: "operator-1".to_string(),
                note: "Approved high-value payout after verifier scope review".to_string(),
            })
            .unwrap();
        assert_eq!(payout_review.outcome, RiskReviewOutcome::Approved);
        assert_eq!(payout_review.surface, RiskSurface::Payout);
        assert_eq!(payout_review.bounty_id, Some(approval.bounty.id));

        let proof = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: approval.bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: "not-used-by-github-ci".to_string(),
                verifier_kind: None,
                rubric: None,
                evidence: Some(github_ci_evidence()),
                approved_risk_event_id: Some(payout_event.id),
            })
            .await
            .unwrap();

        let status = network.status(approval.bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Payable);
        assert_eq!(proof.bounty_id, approval.bounty.id);
        assert!(status
            .risk_events
            .iter()
            .any(|event| event.surface == RiskSurface::Payout));
        assert_eq!(network.risk_reviews.len(), 2);
        assert_eq!(status.settlements.len(), 1);
    }

    #[test]
    fn operator_can_reject_reviewed_bounty_without_creating_bounty() {
        let mut network = BountyNetwork::default();
        let err = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::RiskNeedsReview(_)));
        let risk_event = network
            .list_risk_events(RiskEventFilter {
                action: Some(RiskAction::NeedsReview),
                surface: Some(RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .pop()
            .unwrap();

        let review = network
            .reject_risk_event(RejectRiskEventRequest {
                risk_event_id: risk_event.id,
                operator_id: "operator-1".to_string(),
                note: "Rejected until payer completes manual onboarding".to_string(),
            })
            .unwrap();

        assert_eq!(review.outcome, RiskReviewOutcome::Rejected);
        assert!(network.bounties.is_empty());
        assert_eq!(network.risk_reviews.len(), 1);
        assert!(matches!(
            network
                .approve_risk_bounty(ApproveRiskBountyRequest {
                    risk_event_id: risk_event.id,
                    title: "Fix deterministic payout reconciliation failure".to_string(),
                    template_slug: "fix-ci-failure".to_string(),
                    amount_minor: 25_000_000,
                    currency: "usdc".to_string(),
                    funding_mode: FundingMode::BaseUsdcEscrow,
                    privacy: PrivacyLevel::Public,
                    operator_id: "operator-1".to_string(),
                    note: "Second review should not be accepted".to_string(),
                })
                .unwrap_err(),
            AppError::RiskAlreadyReviewed
        ));
    }

    #[test]
    fn base_refund_event_reverses_funded_bounty_once() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic refund path".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();

        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                7,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        let refunded =
            chain_base::simulated_refunded_event(bounty.id, 7, format!("0x{}", "aa".repeat(32)));
        let reconciliation = network.apply_base_escrow_event(refunded.clone()).unwrap();

        assert_eq!(reconciliation.bounty.status, BountyStatus::Refunded);
        assert_eq!(reconciliation.escrow.status, EscrowStatus::Refunded);
        assert_eq!(reconciliation.ledger_entries.len(), 1);
        assert_eq!(network.ledger.entries().len(), 2);

        let replay = network.apply_base_escrow_event(refunded).unwrap();
        assert!(replay.ledger_entries.is_empty());
        assert_eq!(network.ledger.entries().len(), 2);
    }

    #[test]
    fn second_base_created_escrow_for_bounty_is_rejected() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic double funding path".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        fund_base_bounty(&mut network, &bounty, 7);

        let err = network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                8,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap_err();

        assert!(matches!(err, AppError::InvalidBaseEscrowEvent(_)));
        assert_eq!(network.escrows.len(), 1);
        assert_eq!(network.ledger.entries().len(), 1);
    }

    #[test]
    fn base_refund_and_dispute_plans_build_operator_transactions() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic dispute path".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                7,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();

        let refund_plan = network
            .plan_base_refund(PlanBaseRefundRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                reason_hash: format!("0x{}", "aa".repeat(32)),
                network: None,
            })
            .unwrap();
        assert_eq!(refund_plan.network.chain_id, 84_532);
        assert_eq!(refund_plan.onchain_escrow_id, 7);
        assert_eq!(refund_plan.transaction.function, "refund(uint256,bytes32)");
        assert!(refund_plan.transaction.data.starts_with("0x71eedb88"));

        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://local/disputed.json".to_string(),
                artifact_body: "{\"ok\":false}".to_string(),
            })
            .unwrap();

        let dispute_plan = network
            .plan_base_dispute(PlanBaseDisputeRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                dispute_hash: format!("0x{}", "bb".repeat(32)),
                network: Some("base-mainnet".to_string()),
            })
            .unwrap();
        assert_eq!(dispute_plan.network.chain_id, 8_453);
        assert_eq!(dispute_plan.onchain_escrow_id, 7);
        assert_eq!(
            dispute_plan.transaction.function,
            "markDisputed(uint256,bytes32)"
        );
        assert!(dispute_plan.transaction.data.starts_with("0x4dcc33b8"));
    }

    #[test]
    fn disputed_base_escrow_can_be_refunded_from_chain_event() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Refund disputed escrow".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                7,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://local/disputed.json".to_string(),
                artifact_body: "{\"ok\":false}".to_string(),
            })
            .unwrap();

        let disputed =
            chain_base::simulated_disputed_event(bounty.id, 7, format!("0x{}", "bb".repeat(32)));
        let disputed_report = network.apply_base_escrow_event(disputed).unwrap();
        assert_eq!(disputed_report.bounty.status, BountyStatus::Disputed);
        assert_eq!(disputed_report.escrow.status, EscrowStatus::Disputed);

        let refund_plan = network
            .plan_base_refund(PlanBaseRefundRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                reason_hash: format!("0x{}", "cc".repeat(32)),
                network: None,
            })
            .unwrap();
        assert_eq!(refund_plan.escrow.status, EscrowStatus::Disputed);

        let refunded =
            chain_base::simulated_refunded_event(bounty.id, 7, format!("0x{}", "cc".repeat(32)));
        let refund_report = network.apply_base_escrow_event(refunded).unwrap();
        assert_eq!(refund_report.bounty.status, BountyStatus::Refunded);
        assert_eq!(refund_report.escrow.status, EscrowStatus::Refunded);
        assert_eq!(refund_report.ledger_entries.len(), 1);
    }

    fn github_ci_evidence() -> serde_json::Value {
        serde_json::json!({
            "repository": "example/repo",
            "pull_request_url": "https://github.com/example/repo/pull/1",
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
