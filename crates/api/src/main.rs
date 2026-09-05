mod a2a;
mod discoverability;
mod distribution;
mod github_discovery;
mod open_competition_v2_api;
mod opportunities;
mod site_auth;

use app::{
    build_audience_report, build_live_money_readiness_report, build_objective_canonical_evidence,
    hash_artifact, AddFundingContributionRequest, ApproveRiskBountyRequest,
    ApproveRiskPayoutRequest, BountyNetwork, BountyStatusResponse, ClaimBountyRequest,
    CreateFundingIntentRequest, CreateHelpRequestRequest, FundQuoteRequest, FundingIntentReport,
    LiveMoneyReadinessConfig, LiveMoneyReadinessReport, OpenPooledBountyRequest,
    PlanStripeTransferRequest as AppPlanStripeTransferRequest, PooledFundingReport,
    PostBountyRequest, QuoteSet, RecordAudienceInteractionRequest, RecordDiscoveryResponseRequest,
    RecordOutreachAttemptRequest, RegisterAgentRequest, RegisterCapabilityRequest,
    RejectRiskEventRequest, RequestQuotesRequest, ReviewedBountyApproval, RiskEventFilter,
    StripeTransferPlan, StripeTransferReconciliation, SubmitResultRequest,
    UpsertAudienceMemberRequest, UpsertContributorContactRequest, VerifySubmissionRequest,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode, Uri},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Redirect, Response,
    },
    routing::{get, post},
    Json, Router,
};
use bounty_router::{BountyRouter, RouteDecision};
use chain_base::{
    attach_open_competition_commit_calls, attach_open_competition_entrant_relay_signature,
    attach_open_competition_reveal_call, attach_open_competition_withdrawal_call,
    autonomous_bounty_is_earning_ready, base_network_descriptor, broadcast_signed_transaction,
    build_autonomous_bounty_feed, build_autonomous_bounty_terms_record,
    build_autonomous_submission_evidence_record, build_autonomous_submission_preparation,
    build_autonomous_verification_jobs, built_in_open_competition_verifier_catalog,
    decode_autonomous_bounty_logs, encode_open_competition_entrant_commit_payload,
    encode_open_competition_entrant_reveal_payload,
    encode_open_competition_entrant_withdraw_payload, eth_get_transaction_receipt_request,
    eth_send_raw_transaction_request, event_topic, fetch_block_number, fetch_exact_block_identity,
    fetch_safe_block_identity, fetch_transaction_receipt, normalize_evm_address,
    observe_erc20_balance_safe, observe_open_competition_entrant_wallet_safe_state,
    observe_open_competition_safe_state, observe_solver_leaderboard_paid_winner_safe,
    open_competition_entrant_payload_bounty, open_competition_readiness_from_state,
    plan_canonical_child_bounty_terms as build_canonical_child_bounty_terms_plan,
    plan_open_competition_action, plan_open_competition_creation,
    plan_open_competition_entrant_action, plan_standing_meta_v4_action,
    prepare_agent_to_earn as inspect_agent_wallet_readiness, solver_leaderboard_award_id,
    standing_meta_v2_parent_context, standing_meta_v4_readiness,
    validate_attestation_request_against_feed, validate_autonomous_cancel_authority,
    validate_autonomous_creation_for_public_earning, validate_open_competition_commitment_envelope,
    AgentWalletReadinessReport, AtomicClaimSponsorGrant, AutonomousBountyAuthorizationSignature,
    AutonomousBountyAuthorizedClaimPlan, AutonomousBountyAuthorizedContributionPlan,
    AutonomousBountyAuthorizedCreationPlan, AutonomousBountyClaimPlan,
    AutonomousBountyContribution, AutonomousBountyContributionPlan, AutonomousBountyCreate,
    AutonomousBountyCreationPlan, AutonomousBountyEvent, AutonomousBountyEventKind,
    AutonomousBountyFeedItem, AutonomousBountyRecoveryReservations,
    AutonomousBountySubmissionAuthorizationRequest,
    AutonomousBountySubmissionAuthorizationTypedData, AutonomousBountySubmissionPreparation,
    AutonomousBountyTxPlanner, AutonomousSignedAttestation,
    AutonomousVerificationAttestationRequest, AutonomousVerificationAttestationTypedData,
    AutonomousVerificationJob, BaseNetworkDescriptor, BaseRelayedTransaction, BaseRpcUrlConfig,
    BaseTransactionRelayer, CanonicalChildBountyTermsPlan, CanonicalChildBountyTermsRequest,
    ChainBaseError, Eip3009AuthorizationTypedData, EthGetTransactionReceiptRequest,
    EthSendRawTransactionRequest, EvmLog, EvmTransactionIntent, OpenCompetitionActionPlan,
    OpenCompetitionAuthorizationSignature, OpenCompetitionCommitmentEnvelope,
    OpenCompetitionCreateParams, OpenCompetitionCreationPlan, OpenCompetitionCreationRequest,
    OpenCompetitionDeploymentState, OpenCompetitionEntrantAction, OpenCompetitionEntrantActionPlan,
    OpenCompetitionEntrantWalletReleaseManifest, OpenCompetitionEntrantWalletSafeState,
    OpenCompetitionEvent, OpenCompetitionFundingAuthorization, OpenCompetitionOffchainGates,
    OpenCompetitionOperation, OpenCompetitionReadinessReport, OpenCompetitionReleaseManifest,
    OpenCompetitionSafeState, OpenCompetitionStateQuery, OpenCompetitionVerifierCatalog,
    OpenCompetitionVerifierProfile, PrepareAgentToEarnInput, RpcTransactionReceipt,
    SolverLeaderboardAwardSafeObservation, StandingMetaV2ChildPreparationPlan,
    StandingMetaV2ChildPreparationRequest, StandingMetaV4ActionPlan,
    StandingMetaV4EconomicsEvidence, StandingMetaV4Operation, StandingMetaV4ReadinessEvidence,
    StandingMetaV4ReadinessReport, AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION,
    AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use cloud_agent::{
    CloudAgentError, CloudAgentReadiness, CloudAgentService, CloudBountyAnalysis,
    CloudBountyAnalysisRequest, CloudBountyDraft, CloudBountyDraftRequest, CloudDemoSolution,
    CloudObjectiveExecutionPolicy, CloudObjectivePlan, CloudObjectivePlanRequest,
    CloudObjectiveSettlementPolicy, CloudObjectiveTask, CloudObjectiveVerificationPolicy,
    CloudObjectiveVerifierDraft, CloudUnfundedBountyRequest,
};
use db::{
    AttributionReliability, BaseIndexerHeartbeat, ChatgptActionIntent as DbChatgptActionIntent,
    ChatgptActionObservation, ClaimCandidateReservation, ClaimFunnelStats, DbError,
    DiscoveryInterface, DiscoveryRouteFamily, GitHubIssueSyncBountyUpsert, NewBondSponsorship,
    NewChatgptActionIntent, NewClaimCandidate, NewDiscoveryWebhookSubscription, NewLegalAcceptance,
    NewOpenCompetitionEntrantRelay, NewOpportunityComment, NewSiteAnalyticsEvent,
    NewSocialMentionIngestion, NewTrialBounty, NewUnfundedBountySolution, NewX402RelayAttempt,
    ObservedInterface, ObservedProtocolEra, OpenCompetitionEntrantRelay,
    OpenCompetitionEntrantRelayStatus, OpportunityComment as DbOpportunityComment,
    OpportunityLifecycleStats, PlatformDemandGrowthStats, PlatformMetricsStats, PostgresStore,
    SiteAnalyticsStats, SocialMentionIngestion, TrialBounty, UnfundedBountySolution,
    WebhookSubscription, X402RelayAttempt, X402RelayStatus,
};
use domain::{
    leaderboard_period, rank_solver_completions, Agent, AgentEligibilityDecision,
    AgentEligibilityEvidence, AgentEligibilityPolicy, AgentStatus, AgentWebhookEventType,
    AudienceInteraction, AudienceMember, AudienceReport, AutonomousBountyTermsDocument,
    AutonomousBountyTermsRecord, AutonomousSubmissionEvidenceRecord, BondSponsorship,
    BondSponsorshipStatus, BountyStatus, Capability, CapabilityClass, ClaimCandidate,
    ClaimCandidateStatus, ContributorContact, DiscoveryResponse, DiscoverySubscriptionFilters,
    EvalRun, HelpRequest, Id, LeaderboardPeriodKind, Money, Objective, ObjectiveAction,
    ObjectiveActionPlan, ObjectiveCanonicalEvidence, ObjectiveCreationDraft, ObjectiveCreationPlan,
    ObjectiveError, ObjectiveView, OutreachAttempt, PaymentRail, PayoutStatus, PrivacyLevel,
    RiskEvent, RiskReviewRecord, SignedObjectiveAction, SignedObjectiveCreation,
    SolverLeaderboardRanking, VerificationDecision, VerifierKind,
};
use eval_harness::{
    bundled_abuse_fixtures, bundled_fixtures, bundled_judge_fixtures, run_eval_loops, AbuseBench,
    BountyBench, EvalSuiteResult, JudgeBench, LoopSuiteResult,
};
use github_app::{
    bounty_check_output, claim_comment_plan, create_comment_plan, funding_comment_plan,
    issue_api_sync_plan, parse_issue_form_bounty, proof_comment_plan, social_mention_draft_plan,
    GitHubCanonicalConversionEvidence, GitHubCheckRunOutput, GitHubClaimCommentInput,
    GitHubClaimCommentPlan, GitHubCreateCommentInput, GitHubCreateCommentPlan,
    GitHubFundingCommentInput, GitHubFundingCommentPlan, GitHubIssueApiSyncInput,
    GitHubIssueApiSyncPlan, GitHubIssueFormBounty, GitHubProofComment, GitHubProofCommentPlan,
    SocialMentionDraftInput, SocialMentionDraftPlan,
};
use github_discovery::{
    assemble_projection as assemble_github_discovery_projection, autonomous_discovery_items,
    open_competition_discovery_items, GitHubDiscoveryProjectionResponse, GitHubDiscoverySafeBlock,
    GitHubDiscoverySourceStatus, AUTONOMOUS_PROTOCOL_VERSION, OPEN_COMPETITION_PROTOCOL_VERSION,
};
use hmac::{Hmac, Mac};
use opportunities::{
    apply_query as apply_opportunity_query, canonical_opportunity, legacy_opportunity,
    open_competition_opportunities, open_competition_v2_opportunities, render_opportunity_feeds,
    unfunded_opportunity, OpportunityItem, OpportunityProjectionResponse, OpportunityQuery,
    OpportunitySourceStatus, OpportunityView, OPPORTUNITY_PROJECTION_SCHEMA,
};
use payments_stripe::{
    apply_checkout_payment_method_configuration, execute_stripe_request, verify_webhook_signature,
    CheckoutTopUpRequest, ConnectAccountSnapshot, StripeEventDeduper, StripeExecutionReport,
    StripePlanner, StripeRequestIntent, StripeWebhookEvent, STRIPE_API_BASE_URL,
};
use payments_x402::{
    base_usdc_funding_challenge, decode_payment_signature_header, encode_payment_required_header,
    encode_payment_response_header, validate_funding_payload, Eip3009Authorization, Eip3009Payload,
    PaymentPayload, PaymentRequired, SettlementResponse, AGENT_BOUNTY_FUND_SCHEME,
    PAYMENT_REQUIRED_HEADER, PAYMENT_RESPONSE_HEADER, PAYMENT_SIGNATURE_HEADER, X402_VERSION,
};
use risk::{RiskPolicy, RiskPolicyDescriptor};
use serde::{Deserialize, Serialize};
use service_runtime::{
    autonomous_factory_for_chain, eval_run_from_loop_suite, eval_run_from_suite,
    LiveMoneyRuntimeSettings, PlannerAddressError,
};
#[cfg(test)]
use service_runtime::{
    canonical_mainnet_factory, CANONICAL_BASE_MAINNET_BOUNTY_FACTORY,
    CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION,
};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::env;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration, Instant};
use tokio_stream::{wrappers::IntervalStream, StreamExt};
use tower_http::cors::CorsLayer;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use utoipa::openapi::Components;
use utoipa::{Modify, OpenApi, ToSchema};
use uuid::Uuid;
use worker::{
    derive_discovery_webhook_secret, enqueue_discovery_event, validate_public_https_endpoint,
    DiscoveryWebhookConfig,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        a2a::agent_card,
        a2a::send_message,
        a2a::get_task,
        a2a::list_tasks,
        a2a::cancel_task,
        llms_txt,
        legal_policy,
        record_legal_acceptance,
        discovery_manifest_schema,
        agent_bounties_discovery,
        x402_discovery,
        risk_policy,
        live_money_readiness,
        cloud_agent_readiness,
        compile_objective_with_cloud_agent,
        draft_bounty_with_cloud_agent,
        analyze_bounty_fit,
        github_bounty_discovery,
        list_opportunities,
        stream_opportunities,
        list_opportunity_comments,
        create_opportunity_comment,
        create_chatgpt_action_intent,
        get_chatgpt_action_intent,
        observe_chatgpt_action_transaction,
        opportunity_feed_rss,
        opportunity_feed_atom,
        opportunity_feed_json,
        opportunity_embed_page,
        opportunity_embed_svg,
        opportunity_embed_markdown,
        opportunity_conversion_funnel,
        record_site_analytics_event,
        site_analytics,
        platform_metrics,
        discoverability::ingest_snapshots,
        discoverability::operator_report,
        discoverability::public_summary,
        distribution::operator_report,
        distribution::public_summary,
        distribution::mark_wallet_reviewed,
        distribution::upsert_wallet_exclusion,
        create_discovery_subscription,
        get_discovery_subscription,
        delete_discovery_subscription,
        publish_unfunded_bounty,
        list_unfunded_bounties,
        get_unfunded_bounty,
        submit_unfunded_bounty_solution,
        prepare_agent_wallet_to_earn,
        list_open_competition_verifiers,
        list_open_competition_events,
        prepare_open_competition_creation,
        prepare_open_competition_authorized_creation,
        get_open_competition_state,
        get_open_competition_readiness,
        prepare_open_competition_commit,
        prepare_open_competition_reveal,
        prepare_open_competition_entrant_action,
        relay_open_competition_entrant_action,
        get_open_competition_entrant_relay,
        get_open_competition_status,
        withdraw_open_competition_bond,
        open_competition_v2_api::release,
        open_competition_v2_api::profiles,
        open_competition_v2_api::prepare_structured_artifact_profile,
        open_competition_v2_api::validate_creation,
        open_competition_v2_api::prepare_creation,
        open_competition_v2_api::prepare_funding,
        open_competition_v2_api::inventory,
        open_competition_v2_api::events,
        open_competition_v2_api::create_proof_quote,
        open_competition_v2_api::prepare_proof,
        open_competition_v2_api::prepare_action,
        open_competition_v2_api::get_proof_job,
        open_competition_v2_api::proof_attribution,
        open_competition_v2_api::pay_proof_job,
        open_competition_v2_api::authorize_proof_job_relay,
        get_standing_meta_v4_readiness,
        prepare_standing_meta_v4_claim,
        prepare_anonymous_stake_registration,
        set_anonymous_stake_availability,
        list_verification_assignments,
        submit_primary_verdict,
        waive_verification_appeal,
        open_verification_appeal,
        submit_appeal_vote,
        finalize_verification_case,
        list_risk_events,
        list_risk_reviews,
        approve_risk_bounty,
        approve_risk_payout,
        reject_risk_event,
        route_blocked_goal,
        run_bountybench,
        run_abusebench,
        run_judgebench,
        run_eval_loop_suite,
        list_eval_runs,
        register_agent,
        agent_paid_status,
        upsert_contributor_contact,
        list_contributor_contacts,
        upsert_audience_member,
        list_audience_members,
        record_audience_interaction,
        list_audience_interactions,
        record_discovery_response,
        list_discovery_responses,
        record_outreach_attempt,
        list_outreach_attempts,
        audience_report,
        plan_objective_creation,
        create_objective,
        list_objectives,
        get_objective,
        plan_objective_action,
        apply_objective_action,
        reconcile_objective,
        register_capability,
        search_capabilities,
        create_help_request,
        request_quotes,
        fund_quote,
        list_claimable_bounties,
        public_bounty_feed,
        public_funding_feed,
        public_capability_feed,
        x402_base_bounty_funding,
        get_x402_relay,
        broadcast_base_signed_transaction,
        get_base_transaction_receipt,
        plan_autonomous_canonical_child_terms,
        prepare_standing_meta_v2_child,
        plan_autonomous_bounty_creation,
        plan_autonomous_bounty_authorized_creation,
        plan_autonomous_bounty_contribution,
        plan_autonomous_bounty_authorized_contribution,
        plan_autonomous_bounty_claim,
        plan_autonomous_bounty_authorized_claim,
        agent_native_claim,
        claim_funnel,
        plan_autonomous_bounty_submission,
        prepare_autonomous_bounty_submission,
        plan_autonomous_bounty_submission_authorization,
        plan_autonomous_verification_attestation,
        plan_autonomous_module_settlement,
        plan_autonomous_attestation_settlement,
        plan_autonomous_expire_claim,
        plan_autonomous_expire_submission,
        relay_autonomous_timeout,
        plan_autonomous_cancel,
        plan_autonomous_refund_withdrawal,
        plan_bounded_wallet_cancel_refund,
        decode_autonomous_bounty_events,
        list_autonomous_bounty_events,
        publish_autonomous_bounty_terms,
        get_autonomous_bounty_terms,
        publish_autonomous_submission_evidence,
        get_autonomous_submission_evidence,
        autonomous_bounty_feed,
        solver_leaderboard,
        autonomous_bounty_inventory_summary,
        autonomous_bounty_inventory_badge,
        autonomous_verification_jobs,
        plan_stripe_checkout_top_up,
        plan_stripe_connect_account,
        plan_stripe_connect_transfer,
        execute_stripe_funding_intent_checkout,
        execute_stripe_checkout_top_up,
        execute_stripe_connect_account,
        execute_stripe_connect_transfer,
        reconcile_stripe_connect_snapshot,
        reconcile_stripe_transfer_event,
        reconcile_stripe_checkout_webhook,
        plan_github_issue_bounty,
        plan_github_issue_api_sync,
        sync_github_issue_api_bounty,
        plan_github_create_comment,
        plan_github_funding_comment,
        plan_github_claim_comment,
        plan_social_mention_draft,
        social_mention_ingestion_readiness,
        ingest_neynar_social_mention,
        get_social_mention_draft,
        plan_github_proof_comment,
        plan_github_proof_comment_from_proof,
        post_bounty,
        open_pooled_bounty,
        create_funding_intent,
        add_funding_contribution,
        claim_bounty,
        submit_result,
        verify_submission,
        bounty_status
    ),
    components(schemas(
        RouteRequest,
        RouteDecision,
        EvalSuiteResult,
        LoopSuiteResult,
        EvalRun,
        RiskEvent,
        RiskReviewRecord,
        RiskPolicyDescriptor,
        PlanStripeCheckoutTopUpRequest,
        PlanStripeConnectAccountRequest,
        PlanStripeConnectTransferRequest,
        PlanGitHubIssueBountyRequest,
        PlanGitHubIssueApiSyncRequest,
        PlanGitHubCreateCommentRequest,
        PlanGitHubFundingCommentRequest,
        PlanGitHubClaimCommentRequest,
        PlanSocialMentionDraftRequest,
        SocialMentionIngestionReadiness,
        SocialMentionWebhookResponse,
        SocialMentionDraftResponse,
        PlanGitHubProofCommentRequest,
        PlanGitHubProofCommentFromProofRequest,
        BroadcastBaseSignedTransactionRequest,
        GetBaseTransactionReceiptRequest,
        SearchCapabilitiesRequest,
        ContributorContact,
        AudienceMember,
        AudienceInteraction,
        DiscoveryResponse,
        OutreachAttempt,
        AudienceReport
        ,ObjectiveCreationDraft
        ,ObjectiveCreationPlan
        ,SignedObjectiveCreation
        ,ObjectiveAction
        ,ObjectiveActionPlan
        ,SignedObjectiveAction
        ,ObjectiveView
        ,CloudAgentReadiness
        ,CloudBountyDraftRequest
        ,CloudBountyDraft
        ,CloudBountyAnalysis
        ,CloudBountyAnalysisRequest
        ,cloud_agent::CloudBountyAnalysisReference
        ,CloudObjectivePlanRequest
        ,CloudObjectivePlan
        ,CloudObjectiveTask
        ,CloudObjectiveVerifierDraft
        ,CloudObjectiveExecutionPolicy
        ,CloudObjectiveVerificationPolicy
        ,CloudObjectiveSettlementPolicy
        ,CloudUnfundedBountyRequest
        ,CloudDemoSolution
        ,OpportunityProjectionResponse
        ,OpportunityItem
        ,opportunities::OpportunityAmount
        ,opportunities::OpportunityNextAction
        ,opportunities::OpportunityEmbedLinks
        ,opportunities::OpportunityStandingMetaV4Economics
        ,opportunities::OpportunityAnonymousSeparation
        ,opportunities::OpportunityVerifierGovernance
        ,opportunities::OpportunityAppealPolicy
        ,opportunities::OpportunityStandingMetaV4Coordination
        ,opportunities::OpportunityStandingMetaV4
        ,OpportunitySourceStatus
        ,OpportunityCommentsResponse
        ,OpportunityCommentResponse
        ,CreateOpportunityCommentRequest
        ,OpportunityFeedbackRequest
        ,OpportunityFeedbackResponse
        ,ChatgptActionKind
        ,CreateChatgptActionIntentRequest
        ,ObserveChatgptActionTransactionRequest
        ,ChatgptActionIntentResponse
        ,DiscoverySubscriptionFilters
        ,domain::DiscoveryRewardFilter
        ,CreateDiscoverySubscriptionRequest
        ,CreateDiscoverySubscriptionResponse
        ,DiscoverySubscriptionResponse
        ,OpportunityConversionFunnelResponse
        ,OpportunityConversionStage
        ,OpportunityConversionRate
        ,OpportunityActorMetrics
        ,SiteAnalyticsEventRequest
        ,SiteAnalyticsReceipt
        ,SiteAnalyticsOverviewResponse
        ,SiteAnalyticsEventCountResponse
        ,SiteAnalyticsDailyResponse
        ,SiteAnalyticsChannelResponse
        ,InterfaceUsageResponse
        ,SiteAnalyticsRateResponse
        ,SiteAnalyticsResponse
        ,PlatformMetricsResponse
        ,PlatformMetricsWindowResponse
        ,PlatformIdentityMetricsResponse
        ,PlatformIdentityRoleResponse
        ,PlatformIdentityNamespaceResponse
        ,PlatformAmountResponse
        ,PlatformPayoutMetricsResponse
        ,PlatformClaimCohortResponse
        ,PlatformInventoryResponse
        ,PlatformDemandGrowthResponse
        ,PlatformDailyResponse
        ,PlatformCoverageResponse
        ,discoverability::DiscoverabilitySnapshotRequest
        ,discoverability::DiscoverabilityIngestionRequest
        ,discoverability::DiscoverabilityIngestionResponse
        ,discoverability::DiscoverabilitySourceStatus
        ,discoverability::HumanReachSummary
        ,discoverability::AutomationReachSummary
        ,discoverability::DiscoverabilityPublicSummary
        ,distribution::DistributionRailMetrics
        ,distribution::DistributionOperatorReport
        ,distribution::PublicDistributionRailMetrics
        ,distribution::DistributionPublicSummary
        ,distribution::DistributionWalletExclusionRequest
        ,distribution::DistributionWalletExclusionResponse
        ,distribution::DistributionWalletReviewResponse
        ,UnfundedBountyResponse
        ,UnfundedBountyAgentSolution
        ,SubmitUnfundedBountySolutionRequest
        ,AutonomousBountyInventorySummary
        ,AutonomousBountyInventoryItem
        ,SolverLeaderboardResponse
        ,SolverLeaderboardPeriodResponse
        ,SolverLeaderboardRanking
        ,LegalPolicyResponse
        ,RecordLegalAcceptanceRequest
        ,LegalAcceptanceResponse
        ,GitHubDiscoveryProjectionResponse
        ,github_discovery::GitHubDiscoveryItem
        ,github_discovery::GitHubDiscoveryAction
        ,github_discovery::GitHubDiscoveryVerifier
        ,github_discovery::GitHubDiscoverySafeBlock
        ,github_discovery::GitHubDiscoverySourceStatus
        ,github_discovery::GitHubSettlementEvidence
        ,a2a::A2aAgentCard
        ,a2a::A2aSendMessageRequest
        ,a2a::A2aSendMessageResponse
        ,a2a::A2aMessage
        ,a2a::A2aPart
        ,a2a::A2aTaskState
        ,a2a::A2aTaskStatus
        ,a2a::A2aArtifact
        ,a2a::A2aTask
        ,a2a::A2aListTasksResponse
        ,a2a::A2aProblem
        ,a2a::A2aHttpErrorEnvelope
        ,a2a::A2aHttpError
        ,a2a::A2aErrorInfo
    )),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Components::new);
        components.add_security_scheme(
            "operator_api_token",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                OPERATOR_TOKEN_HEADER,
                "Operator API token required for hosted mutation surfaces when OPERATOR_API_TOKEN is configured.",
            ))),
        );
        let mut bearer = Http::new(HttpAuthScheme::Bearer);
        bearer.bearer_format = Some("operator-api-token".to_string());
        bearer.description =
            Some("Bearer form of the operator API token for hosted mutation surfaces.".to_string());
        components.add_security_scheme("operator_bearer", SecurityScheme::Http(bearer));
        components.add_security_scheme(
            "discoverability_ingest_token",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "x-agent-bounties-discoverability-ingest",
                "Ingestion-only token for discoverability snapshots. It grants no wallet, payment, or general operator authority.",
            ))),
        );
        let mut ingest_bearer = Http::new(HttpAuthScheme::Bearer);
        ingest_bearer.bearer_format = Some("discoverability-ingest-token".to_string());
        ingest_bearer.description =
            Some("Bearer form of the ingestion-only discoverability token.".to_string());
        components.add_security_scheme(
            "discoverability_ingest_bearer",
            SecurityScheme::Http(ingest_bearer),
        );
    }
}

#[derive(Clone)]
struct AppState {
    network: Arc<Mutex<BountyNetwork>>,
    eval_runs: Arc<Mutex<Vec<EvalRun>>>,
    stripe_webhook_secret: Option<Vec<u8>>,
    allow_unsigned_stripe_webhooks: bool,
    stripe_secret_key: Option<String>,
    stripe_live_execution_enabled: bool,
    stripe_public_checkout_enabled: bool,
    stripe_api_base_url: String,
    stripe_payment_method_configuration: Option<String>,
    store: Option<PostgresStore>,
    base_rpc_urls: BaseRpcUrlConfig,
    base_broadcast_enabled: bool,
    operator_api_token: Option<String>,
    discoverability_ingest_token: Option<String>,
    analytics_exclusion_token: Option<String>,
    distribution_attribution_signing_secret: Option<String>,
    distribution_excluded_wallet_classes: Vec<String>,
    public_base_url: String,
    mcp_base_url: String,
    x402_relayer: X402HostedRelayerConfig,
    bond_sponsor: BondSponsorConfig,
    recovery_reservations: AutonomousBountyRecoveryReservations,
    cloud_agent: Arc<CloudAgentService>,
    discovery_webhooks: Option<Arc<DiscoveryWebhookConfig>>,
    neynar_social: Option<Arc<NeynarSocialIngestionConfig>>,
}

#[derive(Clone)]
struct NeynarSocialIngestionConfig {
    webhook_secret: Vec<u8>,
    bot_fid: i64,
    bot_username: String,
    api_key: Option<String>,
    signer_uuid: Option<String>,
    api_base_url: String,
    website_base_url: String,
    client: reqwest::Client,
}

impl NeynarSocialIngestionConfig {
    fn from_env() -> anyhow::Result<Option<Self>> {
        let webhook_secret = env::var("NEYNAR_WEBHOOK_SECRET")
            .ok()
            .and_then(non_empty_secret);
        let bot_fid = env::var("NEYNAR_BOT_FID").ok().and_then(non_empty_secret);
        let bot_username = env::var("NEYNAR_BOT_USERNAME")
            .ok()
            .and_then(non_empty_secret);
        let api_key = env::var("NEYNAR_API_KEY").ok().and_then(non_empty_secret);
        let signer_uuid = env::var("NEYNAR_SIGNER_UUID")
            .ok()
            .and_then(non_empty_secret);
        let configured_count = [
            webhook_secret.is_some(),
            bot_fid.is_some(),
            bot_username.is_some(),
            api_key.is_some(),
            signer_uuid.is_some(),
        ]
        .into_iter()
        .filter(|configured| *configured)
        .count();
        if configured_count == 0 {
            return Ok(None);
        }
        if configured_count != 5 {
            anyhow::bail!(
                "Neynar ingestion requires NEYNAR_API_KEY, NEYNAR_WEBHOOK_SECRET, NEYNAR_SIGNER_UUID, NEYNAR_BOT_FID, and NEYNAR_BOT_USERNAME together"
            );
        }
        let bot_fid = bot_fid
            .expect("checked")
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("NEYNAR_BOT_FID must be a positive integer"))?;
        if bot_fid <= 0 {
            anyhow::bail!("NEYNAR_BOT_FID must be a positive integer");
        }
        let bot_username = bot_username
            .expect("checked")
            .trim_start_matches('@')
            .to_string();
        if bot_username.is_empty()
            || bot_username.len() > 64
            || !bot_username.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })
        {
            anyhow::bail!("NEYNAR_BOT_USERNAME is invalid");
        }
        let api_base_url = env::var("NEYNAR_API_BASE_URL")
            .ok()
            .and_then(non_empty_secret)
            .unwrap_or_else(|| "https://api.neynar.com".to_string())
            .trim_end_matches('/')
            .to_string();
        if !api_base_url.starts_with("https://") && !api_base_url.starts_with("http://127.0.0.1:") {
            anyhow::bail!("NEYNAR_API_BASE_URL must use HTTPS");
        }
        let website_base_url = env::var("WEBSITE_BASE_URL")
            .ok()
            .and_then(non_empty_secret)
            .unwrap_or_else(|| "https://agentbounties.app".to_string())
            .trim_end_matches('/')
            .to_string();
        if !website_base_url.starts_with("https://")
            && !website_base_url.starts_with("http://127.0.0.1:")
        {
            anyhow::bail!("WEBSITE_BASE_URL must use HTTPS");
        }
        Ok(Some(Self {
            webhook_secret: webhook_secret.expect("checked").into_bytes(),
            bot_fid,
            bot_username,
            api_key,
            signer_uuid,
            api_base_url,
            website_base_url,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?,
        }))
    }

    fn reply_configured(&self) -> bool {
        self.api_key.is_some() && self.signer_uuid.is_some()
    }

    fn draft_handoff_url(&self, id: Uuid) -> String {
        format!(
            "{}/?from=social-mention&socialDraft={id}#post-a-bounty",
            self.website_base_url
        )
    }
}

#[derive(Clone)]
struct X402HostedRelayerConfig {
    enabled: bool,
    relayer: Option<Arc<BaseTransactionRelayer>>,
    min_amount: u64,
    max_amount: u64,
    max_gas: u64,
    max_fee_per_gas_wei: u128,
    max_daily_attempts: u32,
    max_daily_attempts_per_contributor: u32,
    confirmations: u64,
    wait_seconds: u64,
    rpc_timeout_seconds: u64,
    lease_seconds: u64,
}

impl Default for X402HostedRelayerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            relayer: None,
            min_amount: 100_000,
            max_amount: 5_000_000,
            max_gas: 300_000,
            max_fee_per_gas_wei: 10_000_000_000,
            max_daily_attempts: 100,
            max_daily_attempts_per_contributor: 10,
            confirmations: 2,
            wait_seconds: 20,
            rpc_timeout_seconds: 15,
            lease_seconds: 45,
        }
    }
}

impl X402HostedRelayerConfig {
    fn from_env() -> anyhow::Result<Self> {
        let enabled = env_flag("ENABLE_X402_HOSTED_RELAY");
        let relayer = env::var("X402_RELAYER_PRIVATE_KEY")
            .ok()
            .and_then(non_empty_secret)
            .map(|private_key| BaseTransactionRelayer::from_private_key(&private_key))
            .transpose()
            .map_err(|_| anyhow::anyhow!("X402_RELAYER_PRIVATE_KEY is invalid"))?
            .map(Arc::new);
        if enabled && relayer.is_none() {
            anyhow::bail!("ENABLE_X402_HOSTED_RELAY requires X402_RELAYER_PRIVATE_KEY");
        }
        let min_amount = env_u64("X402_RELAYER_MIN_USDC_BASE_UNITS", 100_000)?;
        let max_amount = env_u64("X402_RELAYER_MAX_USDC_BASE_UNITS", 5_000_000)?;
        let max_gas = env_u64("X402_RELAYER_MAX_GAS", 700_000)?;
        let max_fee_per_gas_wei = env_u128("X402_RELAYER_MAX_FEE_PER_GAS_WEI", 10_000_000_000)?;
        if min_amount == 0 || max_amount < min_amount || max_gas == 0 || max_fee_per_gas_wei == 0 {
            anyhow::bail!("x402 relayer amount, gas, and fee caps must be positive");
        }
        let max_daily_attempts = u32::try_from(env_u64("X402_RELAYER_MAX_DAILY_ATTEMPTS", 100)?)
            .map_err(|_| anyhow::anyhow!("X402_RELAYER_MAX_DAILY_ATTEMPTS exceeds u32"))?;
        let max_daily_attempts_per_contributor = u32::try_from(env_u64(
            "X402_RELAYER_MAX_DAILY_ATTEMPTS_PER_CONTRIBUTOR",
            10,
        )?)
        .map_err(|_| {
            anyhow::anyhow!("X402_RELAYER_MAX_DAILY_ATTEMPTS_PER_CONTRIBUTOR exceeds u32")
        })?;
        if max_daily_attempts == 0
            || max_daily_attempts_per_contributor == 0
            || max_daily_attempts_per_contributor > max_daily_attempts
        {
            anyhow::bail!("x402 relayer rolling-24-hour quotas are invalid");
        }
        let rpc_timeout_seconds = env_u64("X402_RELAYER_RPC_TIMEOUT_SECONDS", 15)?.clamp(1, 30);
        let lease_seconds = env_u64("X402_RELAYER_LEASE_SECONDS", 45)?.max(15);
        if lease_seconds <= rpc_timeout_seconds {
            anyhow::bail!("X402_RELAYER_LEASE_SECONDS must exceed the RPC timeout");
        }
        Ok(Self {
            enabled,
            relayer,
            min_amount,
            max_amount,
            max_gas,
            max_fee_per_gas_wei,
            max_daily_attempts,
            max_daily_attempts_per_contributor,
            confirmations: env_u64("X402_RELAYER_CONFIRMATIONS", 2)?.max(1),
            wait_seconds: env_u64("X402_RELAYER_WAIT_SECONDS", 20)?.min(60),
            rpc_timeout_seconds,
            lease_seconds,
        })
    }

    fn address(&self) -> Option<String> {
        self.relayer.as_ref().map(|relayer| relayer.address())
    }
}

#[derive(Clone)]
struct BondSponsorConfig {
    enabled: bool,
    grant_signer: Option<Arc<BaseTransactionRelayer>>,
    base_mainnet_contract: Option<String>,
    base_sepolia_contract: Option<String>,
    max_bond: u64,
    max_network_amount_24h: u64,
    max_solver_amount_24h: u64,
    max_gas: u64,
    max_fee_per_gas_wei: u128,
    rpc_timeout_seconds: u64,
}

impl Default for BondSponsorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            grant_signer: None,
            base_mainnet_contract: None,
            base_sepolia_contract: None,
            max_bond: 100_000,
            max_network_amount_24h: 1_000_000,
            max_solver_amount_24h: 100_000,
            max_gas: 500_000,
            max_fee_per_gas_wei: 10_000_000_000,
            rpc_timeout_seconds: 15,
        }
    }
}

impl BondSponsorConfig {
    fn from_env() -> anyhow::Result<Self> {
        let enabled = env_flag("ENABLE_BOND_SPONSORSHIP");
        let grant_signer = env::var("BOND_SPONSOR_GRANT_SIGNER_PRIVATE_KEY")
            .ok()
            .and_then(non_empty_secret)
            .map(|private_key| BaseTransactionRelayer::from_private_key(&private_key))
            .transpose()
            .map_err(|_| anyhow::anyhow!("BOND_SPONSOR_GRANT_SIGNER_PRIVATE_KEY is invalid"))?
            .map(Arc::new);
        let base_mainnet_contract = optional_evm_address("BOND_SPONSOR_BASE_MAINNET_CONTRACT")?;
        let base_sepolia_contract = optional_evm_address("BOND_SPONSOR_BASE_SEPOLIA_CONTRACT")?;
        if enabled
            && (grant_signer.is_none()
                || (base_mainnet_contract.is_none() && base_sepolia_contract.is_none()))
        {
            anyhow::bail!(
                "ENABLE_BOND_SPONSORSHIP requires BOND_SPONSOR_GRANT_SIGNER_PRIVATE_KEY and at least one network sponsor contract"
            );
        }
        let max_bond = env_u64("BOND_SPONSOR_MAX_BOND_BASE_UNITS", 100_000)?;
        let max_network_amount_24h = env_u64("BOND_SPONSOR_MAX_NETWORK_24H_BASE_UNITS", 1_000_000)?;
        let max_solver_amount_24h = env_u64("BOND_SPONSOR_MAX_SOLVER_24H_BASE_UNITS", 100_000)?;
        let max_gas = env_u64("BOND_SPONSOR_MAX_GAS", 500_000)?;
        let max_fee_per_gas_wei = env_u128("BOND_SPONSOR_MAX_FEE_PER_GAS_WEI", 10_000_000_000)?;
        if max_bond == 0
            || max_solver_amount_24h < max_bond
            || max_network_amount_24h < max_solver_amount_24h
            || max_gas == 0
            || max_fee_per_gas_wei == 0
        {
            anyhow::bail!("bond sponsor amount, gas, and fee caps are invalid");
        }
        Ok(Self {
            enabled,
            grant_signer,
            base_mainnet_contract,
            base_sepolia_contract,
            max_bond,
            max_network_amount_24h,
            max_solver_amount_24h,
            max_gas,
            max_fee_per_gas_wei,
            rpc_timeout_seconds: env_u64("BOND_SPONSOR_RPC_TIMEOUT_SECONDS", 15)?.clamp(1, 30),
        })
    }

    fn contract_for(&self, network: &str) -> Option<&str> {
        if !self.enabled {
            return None;
        }
        match network {
            "base-mainnet" => self.base_mainnet_contract.as_deref(),
            "base-sepolia" => self.base_sepolia_contract.as_deref(),
            _ => None,
        }
    }

    fn grant_signer(&self) -> Option<&BaseTransactionRelayer> {
        self.grant_signer.as_deref().filter(|_| self.enabled)
    }
}

type SharedState = Arc<AppState>;
const OPERATOR_TOKEN_HEADER: &str = "x-operator-token";
const INTERFACE_ATTRIBUTION_HEADER: &str = "x-agent-bounties-interface";
const ANALYTICS_EXCLUSION_HEADER: &str = "x-agent-bounties-analytics-exclusion";
const ANALYTICS_EXCLUDED_HEADER: &str = "x-agent-bounties-analytics-excluded";
const PLATFORM_LAUNCH_AT: &str = "2026-07-08T20:22:19Z";
const PLATFORM_FIRST_MONTH_ENDED_AT: &str = "2026-08-08T20:22:19Z";
const PUBLIC_METRICS_POLICY_JSON: &str = include_str!("../fixtures/public-metrics-policy.json");
const LEGAL_TERMS_VERSION: &str = "2026-07-18";
const LEGAL_PRIVACY_VERSION: &str = "2026-07-18";
const LEGAL_ACCEPTANCE_STATEMENT: &str = "I meet the age requirement in the Terms and am authorized to use this wallet and perform this action. I understand that public and blockchain records may be permanent. I accept the posted task, verification, and settlement rules. I am responsible for legal compliance, taxes, content rights, agent authority, and wallet security. I agree to the Terms of Use and Privacy Policy.";
const LEGAL_ACTIONS: &[&str] = &[
    "post_bounty",
    "fund_bounty",
    "claim_bounty",
    "submit_result",
    "verify_submission",
    "cancel_bounty",
    "recover_funds",
    "activate_agent_budget",
    "update_agent_policy",
    "revoke_agent_policy",
];

fn non_empty_secret(secret: String) -> Option<String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_distribution_excluded_wallet_classes(
    configured: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let defaults = db::DISTRIBUTION_EXCLUSION_CLASSES.join(",");
    let candidates = configured
        .unwrap_or(&defaults)
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut classes = BTreeSet::new();
    for candidate in candidates {
        let class = db::normalize_distribution_exclusion_class(candidate).ok_or_else(|| {
            anyhow::anyhow!(
                "DISTRIBUTION_EXCLUDED_WALLET_CLASSES contains unsupported class {candidate}"
            )
        })?;
        classes.insert(class.to_string());
    }
    let required = db::DISTRIBUTION_EXCLUSION_CLASSES
        .iter()
        .map(|value| (*value).to_string())
        .collect::<BTreeSet<_>>();
    if classes != required {
        anyhow::bail!("DISTRIBUTION_EXCLUDED_WALLET_CLASSES must contain every required class");
    }
    Ok(classes.into_iter().collect())
}

fn distribution_excluded_wallet_classes_from_env() -> anyhow::Result<Vec<String>> {
    let configured = env::var("DISTRIBUTION_EXCLUDED_WALLET_CLASSES").ok();
    parse_distribution_excluded_wallet_classes(configured.as_deref())
}

fn optional_evm_address(key: &str) -> anyhow::Result<Option<String>> {
    env::var(key)
        .ok()
        .and_then(non_empty_secret)
        .map(|value| normalize_evm_address(&value).map_err(|_| anyhow::anyhow!("{key} is invalid")))
        .transpose()
}

fn env_flag(key: &str) -> bool {
    env::var(key)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn env_u64(key: &str, default: u64) -> anyhow::Result<u64> {
    env::var(key)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("{key} must be a positive integer"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn env_u128(key: &str, default: u128) -> anyhow::Result<u128> {
    env::var(key)
        .ok()
        .map(|value| {
            value
                .parse::<u128>()
                .map_err(|_| anyhow::anyhow!("{key} must be a positive integer"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn require_operator(state: &SharedState, headers: &HeaderMap) -> Result<(), StatusCode> {
    if service_runtime::operator_token_is_authorized(
        state.operator_api_token.as_deref(),
        headers
            .get(OPERATOR_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok()),
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
    ) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct RouteRequest {
    goal: String,
    context: String,
    budget_minor: i64,
    currency: String,
    privacy: PrivacyLevel,
}

#[derive(Debug, Deserialize)]
struct LiveMoneyReadinessQuery {
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanStripeCheckoutTopUpRequest {
    organization_id: Uuid,
    amount_minor: i64,
    currency: String,
    success_url: Option<String>,
    cancel_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanStripeConnectAccountRequest {
    agent_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanStripeConnectTransferRequest {
    payout_intent_id: Uuid,
    connected_account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubIssueBountyRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubIssueApiSyncRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
    api_base_url: Option<String>,
    #[serde(default)]
    existing_bounty_ids: Vec<Uuid>,
    hosted_api_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubFundingCommentRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
    comment_body: String,
    contributor_login: Option<String>,
    comment_id: Option<String>,
    funding_api_base_url: Option<String>,
    #[serde(default)]
    existing_idempotency_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubCreateCommentRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
    comment_body: String,
    contributor_login: Option<String>,
    comment_id: Option<String>,
    #[serde(default)]
    existing_idempotency_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanSocialMentionDraftRequest {
    source_network: String,
    mention_url: String,
    mention_id: String,
    mention_text: String,
    author_handle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct SocialMentionIngestionReadiness {
    schema_version: String,
    provider: String,
    source_network: String,
    enabled: bool,
    operator_enabled: bool,
    database_configured: bool,
    webhook_configured: bool,
    reply_configured: bool,
    bot_fid: Option<i64>,
    bot_username: Option<String>,
    webhook_path: String,
    gate_passed: bool,
    github_originated_canonical_funded: u32,
    github_originated_canonical_settled: u32,
    reason: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct SocialMentionWebhookResponse {
    schema_version: String,
    accepted: bool,
    duplicate: bool,
    status: String,
    ingestion_id: Option<Uuid>,
    draft_handoff_url: Option<String>,
    reply_cast_hash: Option<String>,
    message: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct SocialMentionDraftResponse {
    schema_version: String,
    ingestion_id: Uuid,
    status: String,
    source_network: String,
    mention_url: String,
    author_handle: Option<String>,
    draft: serde_json::Value,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Deserialize)]
struct NeynarWebhookEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: NeynarCast,
}

#[derive(Debug, Clone, Deserialize)]
struct NeynarCast {
    object: String,
    hash: String,
    author: NeynarAuthor,
    text: String,
    #[serde(default)]
    mentioned_profiles: Vec<NeynarMentionProfile>,
}

#[derive(Debug, Clone, Deserialize)]
struct NeynarAuthor {
    fid: i64,
    username: String,
}

#[derive(Debug, Clone, Deserialize)]
struct NeynarMentionProfile {
    fid: i64,
}

#[derive(Debug, Serialize)]
struct NeynarPublishCastRequest<'a> {
    signer_uuid: &'a str,
    text: &'a str,
    parent: &'a str,
    parent_author_fid: i64,
    idem: &'a str,
}

#[derive(Debug, Deserialize)]
struct NeynarPublishCastResponse {
    cast: NeynarPublishedCast,
}

#[derive(Debug, Deserialize)]
struct NeynarPublishedCast {
    hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubClaimCommentRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
    comment_body: String,
    contributor_login: Option<String>,
    comment_id: Option<String>,
    claim_age_minutes: Option<u64>,
    #[serde(default)]
    progress_signal_count: u32,
    active_claim_login: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubProofCommentRequest {
    bounty_id: Uuid,
    proof_url: String,
    verifier_summary: String,
    settlement_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubProofCommentFromProofRequest {
    proof_id: Uuid,
    settlement_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubIssueBountyPlan {
    ready: bool,
    parsed: Option<GitHubIssueFormBounty>,
    error: Option<String>,
    check: GitHubCheckRunOutput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
struct SearchCapabilitiesRequest {
    class: Option<CapabilityClass>,
    template_slug: Option<String>,
    currency: Option<String>,
    max_price_minor: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyCreationRequest {
    network: Option<String>,
    create: AutonomousBountyCreate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyContributionRequest {
    network: Option<String>,
    contribution: AutonomousBountyContribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedCreationRequest {
    network: Option<String>,
    create: AutonomousBountyCreate,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedContributionRequest {
    network: Option<String>,
    contribution: AutonomousBountyContribution,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyClaimRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    authorization_nonce: Option<String>,
    authorization_valid_before: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedClaimRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    authorization_nonce: String,
    authorization_valid_before: u64,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentNativeClaimRequest {
    idempotency_key: String,
    network: Option<String>,
    bounty_contract: String,
    solver_wallet: String,
    agent_id: Option<Uuid>,
    #[serde(default)]
    request_bond_sponsorship: bool,
    signature: Option<AutonomousBountyAuthorizationSignature>,
    wallet_signature: Option<String>,
    source: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct StandingMetaV4ReadinessQuery {
    network: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct StandingMetaV4ActionRequest {
    network: Option<String>,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionReadinessQuery {
    network: Option<String>,
    bounty_contract: Option<String>,
    solver: Option<String>,
    verifier_profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionActionRequest {
    network: Option<String>,
    bounty_contract: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionCommitRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    commitment: String,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionVerifierQuery {
    network: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionEventsQuery {
    network: Option<String>,
    bounty_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionCreateParamsRequest {
    solver_reward: u64,
    verifier_reward: u64,
    terms_hash: String,
    policy_hash: String,
    acceptance_criteria_hash: String,
    benchmark_hash: String,
    evidence_schema_hash: String,
    funding_deadline: u64,
    competition_window_seconds: u64,
    reveal_window_seconds: u64,
    max_entries: u8,
    verifier_reward_recipient: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionAuthorizationSignatureRequest {
    v: u8,
    r: String,
    s: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionFundingAuthorizationRequest {
    valid_after: u64,
    valid_before: u64,
    nonce: String,
    signature: Option<OpenCompetitionAuthorizationSignatureRequest>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionCreationRequestBody {
    network: Option<String>,
    creator: String,
    creation_nonce: String,
    initial_funding: u64,
    verifier_profile_id: String,
    params: OpenCompetitionCreateParamsRequest,
    funding_authorization: Option<OpenCompetitionFundingAuthorizationRequest>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionRevealRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    commitment_envelope: serde_json::Value,
    proof: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum OpenCompetitionEntrantRelayActionRequest {
    Commit,
    Reveal,
    WithdrawBond,
}

impl From<OpenCompetitionEntrantRelayActionRequest> for OpenCompetitionEntrantAction {
    fn from(value: OpenCompetitionEntrantRelayActionRequest) -> Self {
        match value {
            OpenCompetitionEntrantRelayActionRequest::Commit => Self::Commit,
            OpenCompetitionEntrantRelayActionRequest::Reveal => Self::Reveal,
            OpenCompetitionEntrantRelayActionRequest::WithdrawBond => Self::WithdrawBond,
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionEntrantActionPreparationRequest {
    network: Option<String>,
    wallet: String,
    bounty_contract: String,
    action: OpenCompetitionEntrantRelayActionRequest,
    commitment: Option<String>,
    commitment_envelope: Option<serde_json::Value>,
    proof: Option<String>,
    deadline_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct OpenCompetitionEntrantRelayRequest {
    idempotency_key: String,
    plan: serde_json::Value,
    signature: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct OpenCompetitionEntrantRelayResponse {
    schema_version: String,
    id: Uuid,
    network: String,
    wallet: String,
    bounty_contract: String,
    action: u8,
    wallet_nonce: u64,
    status: String,
    retryable: bool,
    transaction_hash: Option<String>,
    receipt_block: Option<u64>,
    receipt_block_hash: Option<String>,
    canonical_safe_block: Option<u64>,
    canonical_safe_block_hash: Option<String>,
    canonical_event: Option<String>,
    payment_proven: bool,
    next_action: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaimFunnelQuery {
    window_hours: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpportunityConversionQuery {
    window_hours: Option<u32>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct OpportunityConversionStage {
    stage: String,
    count: u64,
    evidence_source: String,
    coverage_note: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct OpportunityConversionRate {
    metric: String,
    numerator: u64,
    denominator: u64,
    value: Option<f64>,
    cohort: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct OpportunityActorMetrics {
    unique_canonical_poster_wallets: u64,
    repeat_canonical_poster_wallets: u64,
    unique_paid_solver_wallets: u64,
    repeat_paid_solver_wallets: u64,
    independent_active_agents: Option<u64>,
    independence_measurement_available: bool,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct OpportunityConversionFunnelResponse {
    schema_version: String,
    window_hours: u32,
    window_started_at: String,
    generated_at: String,
    stages: Vec<OpportunityConversionStage>,
    rates: Vec<OpportunityConversionRate>,
    average_seconds_to_first_solution: Option<f64>,
    median_seconds_to_first_solution: Option<f64>,
    average_seconds_creation_to_settlement: Option<f64>,
    actors: OpportunityActorMetrics,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct SiteAnalyticsEventRequest {
    event_id: Uuid,
    visitor_id: Uuid,
    session_id: Uuid,
    event_name: String,
    page_path: String,
    source: Option<String>,
    campaign: Option<String>,
    referrer_host: Option<String>,
    opportunity_id: Option<String>,
    bounty_contract: Option<String>,
    occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SiteAnalyticsReceipt {
    schema_version: String,
    accepted: bool,
    duplicate: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct SiteAnalyticsQuery {
    window_hours: Option<u32>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SiteAnalyticsOverviewResponse {
    unique_visitors: u64,
    returning_visitors: u64,
    sessions: u64,
    page_views: u64,
    first_event_at: Option<String>,
    last_event_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SiteAnalyticsEventCountResponse {
    event_name: String,
    events: u64,
    sessions: u64,
    visitors: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SiteAnalyticsDailyResponse {
    day: String,
    visitors: u64,
    sessions: u64,
    page_views: u64,
    market_views: u64,
    funded_bounty_clicks: u64,
    canonical_posts_confirmed: u64,
    funding_starts: u64,
    claims_confirmed: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SiteAnalyticsChannelResponse {
    source: String,
    campaign: Option<String>,
    visitors: u64,
    sessions: u64,
    page_views: u64,
    funded_bounty_clicks: u64,
    canonical_posts_confirmed: u64,
    funding_starts: u64,
    claims_confirmed: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct InterfaceUsageResponse {
    interface: String,
    protocol_era: String,
    request_count: u64,
    successful_request_count: u64,
    first_observed_at: String,
    last_observed_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SiteAnalyticsRateResponse {
    metric: String,
    numerator_sessions: u64,
    denominator_sessions: u64,
    value: Option<f64>,
    cohort: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SiteAnalyticsResponse {
    schema_version: String,
    window_hours: u32,
    window_started_at: String,
    generated_at: String,
    overview: SiteAnalyticsOverviewResponse,
    event_counts: Vec<SiteAnalyticsEventCountResponse>,
    daily: Vec<SiteAnalyticsDailyResponse>,
    channels: Vec<SiteAnalyticsChannelResponse>,
    interfaces: Vec<InterfaceUsageResponse>,
    rates: Vec<SiteAnalyticsRateResponse>,
    definitions: Vec<String>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PlatformMetricsQuery {
    period: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicMetricsPolicy {
    schema_version: String,
    maintainer_github_logins: Vec<String>,
    maintainer_comment_authors: Vec<String>,
    maintainer_wallets: Vec<String>,
    excluded_bounty_contracts: Vec<String>,
    wallet_ownership_boundary: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformMetricsWindowResponse {
    period: String,
    started_at: String,
    ended_at: String,
    previous_started_at: String,
    previous_ended_at: String,
    launch_at: String,
    first_month_started_at: String,
    first_month_ended_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformIdentityRoleResponse {
    role: String,
    active_identities: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformIdentityNamespaceResponse {
    namespace: String,
    active_identities: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformIdentityMetricsResponse {
    selected: u64,
    previous: u64,
    latest_week: u64,
    previous_week: u64,
    first_month: u64,
    lifetime: u64,
    roles: Vec<PlatformIdentityRoleResponse>,
    namespaces: Vec<PlatformIdentityNamespaceResponse>,
    definition: String,
    cross_namespace_deduplication: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformAmountResponse {
    usdc_base_units: String,
    usdc: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformPayoutMetricsResponse {
    selected: PlatformAmountResponse,
    previous: PlatformAmountResponse,
    first_month: PlatformAmountResponse,
    lifetime: PlatformAmountResponse,
    selected_solver_pay: PlatformAmountResponse,
    selected_verifier_pay: PlatformAmountResponse,
    selected_keeper_pay: PlatformAmountResponse,
    selected_completion_bonus: PlatformAmountResponse,
    selected_settled_rounds: u64,
    previous_settled_rounds: u64,
    first_month_settled_rounds: u64,
    lifetime_settled_rounds: u64,
    definition: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformClaimCohortResponse {
    settled_rounds: u64,
    mature_claimed_rounds: u64,
    immature_claimed_rounds: u64,
    settlement_rate: Option<f64>,
    definition: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformInventoryResponse {
    status: String,
    active_funded_opportunities: Option<usize>,
    available_funding_usdc: Option<String>,
    available_solver_rewards_usdc: Option<String>,
    available_verifier_rewards_usdc: Option<String>,
    generated_at: Option<String>,
    definition: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformDemandGrowthResponse {
    gmv_usdc_7d: PlatformAmountResponse,
    gmv_usdc_28d: PlatformAmountResponse,
    lifetime_canonical_gmv_usdc: PlatformAmountResponse,
    lifetime_canonical_payouts_usdc: PlatformAmountResponse,
    new_poster_funder_wallets_28d: u64,
    repeat_poster_funder_rate_28d: Option<f64>,
    non_operator_funded_gmv_share_28d: Option<f64>,
    funding_attribution_complete_28d: bool,
    definition: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformDailyResponse {
    day: String,
    active_identities: u64,
    payout: PlatformAmountResponse,
    settled_rounds: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformCoverageResponse {
    status: String,
    marketplace_indexers_fresh: bool,
    verified_canonical_events: u64,
    awaiting_block_time_events: u64,
    opportunity_comments: u64,
    latest_verified_event_at: Option<String>,
    latest_comment_at: Option<String>,
    github_included: bool,
    github_snapshot_path: String,
    maintainer_exclusion_policy: String,
    identity_limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct PlatformMetricsResponse {
    schema_version: String,
    network: String,
    generated_at: String,
    window: PlatformMetricsWindowResponse,
    platform_active_identities: PlatformIdentityMetricsResponse,
    marketplace_payout_volume: PlatformPayoutMetricsResponse,
    mature_claim_to_settlement: PlatformClaimCohortResponse,
    current_inventory: PlatformInventoryResponse,
    demand_growth: PlatformDemandGrowthResponse,
    daily: Vec<PlatformDailyResponse>,
    platform_revenue: PlatformAmountResponse,
    monetization_status: String,
    coverage: PlatformCoverageResponse,
    definitions: BTreeMap<String, String>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize)]
struct AgentNativeClaimResponse {
    schema_version: String,
    candidate: ClaimCandidate,
    waitlist_position: Option<u32>,
    claim_bond: String,
    sponsorship_requested: bool,
    sponsorship_available: bool,
    sponsorship_protocol: Option<String>,
    sponsor_contract: Option<String>,
    sponsorship: Option<BondSponsorship>,
    signing_payload: Option<Eip3009AuthorizationTypedData>,
    wallet_request: Option<serde_json::Value>,
    claim_transaction_hash: Option<String>,
    canonical_event_id: Option<Uuid>,
    next_action: String,
    next_request: Option<serde_json::Value>,
    browser_fallback_url: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountySubmissionRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    submission_hash: String,
    evidence_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountySubmissionAuthorizationRequest {
    network: Option<String>,
    submission: AutonomousBountySubmissionAuthorizationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PrepareAutonomousBountySubmissionRequest {
    network: Option<String>,
    bounty_contract: String,
    solver_wallet: String,
    artifact_reference: String,
    evidence: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousVerificationAttestationRequest {
    network: Option<String>,
    attestation: AutonomousVerificationAttestationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousModuleSettlementRequest {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
    proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousAttestationSettlementRequest {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
    attestations: Vec<AutonomousSignedAttestation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousLifecycleRequest {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanBoundedWalletCancelRefundRequest {
    network: Option<String>,
    bounty_contract: String,
    bounded_wallet: String,
    caller: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum AutonomousTimeoutAction {
    ExpireClaim,
    ExpireSubmission,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct RelayAutonomousTimeoutRequest {
    network: Option<String>,
    bounty_contract: String,
    action: AutonomousTimeoutAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct RelayAutonomousTimeoutResponse {
    network: String,
    bounty_contract: String,
    action: AutonomousTimeoutAction,
    previous_bounty_state: String,
    expected_bounty_state: String,
    expected_canonical_event: String,
    transaction_hash: String,
    relayer: String,
    confirmed: bool,
    confirmed_block: Option<u64>,
    canonical_event_id: Option<String>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecodeAutonomousBountyEventsRequest {
    logs: Vec<EvmLog>,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousBountyEventsQuery {
    network: Option<String>,
    bounty_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct X402FundingQuery {
    network: Option<String>,
    amount: Option<u64>,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublishAutonomousBountyTermsRequest {
    creator_wallet: String,
    document: AutonomousBountyTermsDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
struct AgentActionError {
    schema_version: String,
    error_code: String,
    message: String,
    retryable: bool,
    next_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    competition_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    correct_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    competition_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_action_url: Option<String>,
}

type AgentActionApiError = (StatusCode, Json<Box<AgentActionError>>);

fn agent_action_error(
    status: StatusCode,
    error_code: &str,
    message: impl Into<String>,
    retryable: bool,
    next_action: &str,
) -> AgentActionApiError {
    (
        status,
        Json(Box::new(AgentActionError {
            schema_version: "agent-bounties/action-error-v1".to_string(),
            error_code: error_code.to_string(),
            message: message.into(),
            retryable,
            next_action: next_action.to_string(),
            competition_mode: None,
            correct_action: None,
            competition_url: None,
            next_action_url: None,
        })),
    )
}

fn wrong_competition_mode_error(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> AgentActionApiError {
    let website = legal_website_base_url(env::var("WEBSITE_BASE_URL").ok(), &state.public_base_url);
    let contract = bounty_contract.to_ascii_lowercase();
    (
        StatusCode::CONFLICT,
        Json(Box::new(AgentActionError {
            schema_version: "agent-bounties/action-error-v1".to_string(),
            error_code: "wrong_competition_mode".to_string(),
            message: "This bounty uses Open Competition and cannot be exclusively claimed."
                .to_string(),
            retryable: false,
            next_action: "Enter competition by preparing a commitment, saving the local recovery envelope, and revealing from the same wallet in a later block.".to_string(),
            competition_mode: Some("first_valid_submission".to_string()),
            correct_action: Some("enter_competition".to_string()),
            competition_url: Some(format!(
                "{}/competition.html?network={network}&bountyContract={contract}",
                website.trim_end_matches('/')
            )),
            next_action_url: Some(format!(
                "{}/v1/base/open-competition-v1/commit-preparation",
                state.public_base_url.trim_end_matches('/')
            )),
        })),
    )
}

fn status_agent_action_error(status: StatusCode, action: &str) -> AgentActionApiError {
    agent_action_error(
        status,
        "action_preparation_failed",
        "The canonical exclusive-claim action could not be prepared.",
        status.is_server_error(),
        action,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublishAutonomousSubmissionEvidenceRequest {
    network: Option<String>,
    bounty_contract: String,
    bounty_id: String,
    round: u64,
    solver_wallet: String,
    artifact_reference: String,
    evidence: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousSubmissionEvidenceQuery {
    network: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousBountyFeedQuery {
    network: Option<String>,
    claimable_only: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct SolverLeaderboardQuery {
    network: Option<String>,
    at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct SolverLeaderboardPeriodResponse {
    period_status: String,
    reward_usdc: String,
    reward_funding_status: String,
    reward_payout_status: String,
    reward_contract: Option<String>,
    reward_paid_wallet: Option<String>,
    reward_payout_observed_safe_block: Option<u64>,
    reward_payout_observed_safe_block_hash: Option<String>,
    ranking: SolverLeaderboardRanking,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct SolverLeaderboardRewardPoolResponse {
    contract: Option<String>,
    settlement_token: String,
    funding_status: String,
    balance_usdc_base_units: Option<String>,
    balance_usdc: Option<String>,
    current_daily_and_weekly_required_usdc: String,
    maximum_full_weeks_at_current_balance: Option<u64>,
    observed_safe_block: Option<u64>,
    observed_safe_block_hash: Option<String>,
    observation_error: Option<String>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct SolverLeaderboardResponse {
    schema_version: String,
    network: String,
    generated_at: DateTime<Utc>,
    reference_at: DateTime<Utc>,
    reward_pool: SolverLeaderboardRewardPoolResponse,
    daily: SolverLeaderboardPeriodResponse,
    weekly: SolverLeaderboardPeriodResponse,
    next_action: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CloudBountyAnalysisQuery {
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct AutonomousBountyInventoryItem {
    bounty_id: String,
    bounty_contract: String,
    title: Option<String>,
    status: String,
    funded_usdc_base_units: String,
    solver_reward_usdc_base_units: String,
    verifier_reward_usdc_base_units: String,
    verification_ready: bool,
    standing_meta_bounty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct AutonomousBountyInventorySummary {
    schema_version: String,
    network: String,
    generated_at: String,
    canonical_source: String,
    claimable_bounty_count: usize,
    verification_ready_bounty_count: usize,
    standing_meta_bounty_count: usize,
    funded_usdc_base_units: String,
    funded_usdc: String,
    solver_reward_usdc_base_units: String,
    solver_reward_usdc: String,
    verifier_reward_usdc_base_units: String,
    verifier_reward_usdc: String,
    items: Vec<AutonomousBountyInventoryItem>,
    evidence_boundary: String,
}

#[derive(Debug, Clone)]
struct OpenCompetitionInventorySummary {
    generated_at: String,
    ready_to_earn_count: usize,
    funded_usdc_base_units: String,
    solver_reward_usdc_base_units: String,
    verifier_reward_usdc_base_units: String,
}

#[derive(Debug, Clone, Copy)]
struct PlatformCanonicalSourceFreshness {
    autonomous: bool,
    open_competition: bool,
    open_competition_v2: bool,
}

impl PlatformCanonicalSourceFreshness {
    fn complete(self) -> bool {
        self.autonomous && self.open_competition && self.open_competition_v2
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousVerificationJobsQuery {
    network: Option<String>,
    verifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct BroadcastBaseSignedTransactionRequest {
    signed_transaction: String,
    request_id: Option<u64>,
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct GetBaseTransactionReceiptRequest {
    tx_hash: String,
    request_id: Option<u64>,
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaseSignedTransactionBroadcastReport {
    network: BaseNetworkDescriptor,
    request: EthSendRawTransactionRequest,
    tx_hash: String,
    next_step: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaseTransactionReceiptReport {
    network: BaseNetworkDescriptor,
    request: EthGetTransactionReceiptRequest,
    receipt_found: bool,
    tx_hash: String,
    block_number: Option<u64>,
    succeeded: Option<bool>,
    log_count: usize,
    receipt: Option<RpcTransactionReceipt>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = match env::var("DATABASE_URL") {
        Ok(database_url) => {
            let store = PostgresStore::connect(&database_url).await?;
            store.migrate().await?;
            Some(store)
        }
        Err(_) => None,
    };
    let network = if let Some(store) = &store {
        hydrate_network(store).await?
    } else {
        BountyNetwork::default()
    };
    let eval_runs = if let Some(store) = &store {
        store.list_eval_runs().await?
    } else {
        Vec::new()
    };
    let site_auth = site_auth::SiteAuthService::from_env(store.clone())?;
    let x402_relayer = X402HostedRelayerConfig::from_env()?;
    let bond_sponsor = BondSponsorConfig::from_env()?;
    if x402_relayer.enabled && store.is_none() {
        anyhow::bail!("ENABLE_X402_HOSTED_RELAY requires DATABASE_URL");
    }
    if bond_sponsor.enabled && (store.is_none() || !x402_relayer.enabled) {
        anyhow::bail!("ENABLE_BOND_SPONSORSHIP requires DATABASE_URL and ENABLE_X402_HOSTED_RELAY");
    }
    let recovery_reservations_raw = env::var("BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS").ok();
    let recovery_reservations =
        AutonomousBountyRecoveryReservations::parse_csv(recovery_reservations_raw.as_deref())
            .map_err(|error| {
                anyhow::anyhow!("BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS is invalid: {error}")
            })?;
    let cloud_agent = Arc::new(
        CloudAgentService::from_env()
            .map_err(|error| anyhow::anyhow!("cloud-agent configuration is invalid: {error}"))?,
    );
    let discovery_webhooks = DiscoveryWebhookConfig::from_env()?.map(Arc::new);
    let neynar_social = NeynarSocialIngestionConfig::from_env()?.map(Arc::new);
    let distribution_excluded_wallet_classes = distribution_excluded_wallet_classes_from_env()?;
    let state: SharedState = Arc::new(AppState {
        network: Arc::new(Mutex::new(network)),
        eval_runs: Arc::new(Mutex::new(eval_runs)),
        stripe_webhook_secret: env::var("STRIPE_WEBHOOK_SECRET")
            .ok()
            .map(|secret| secret.into_bytes()),
        allow_unsigned_stripe_webhooks: env::var("ALLOW_UNSIGNED_STRIPE_WEBHOOKS")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_secret_key: env::var("STRIPE_SECRET_KEY").ok(),
        stripe_live_execution_enabled: env::var("ENABLE_STRIPE_LIVE_EXECUTION")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_public_checkout_enabled: env::var("ENABLE_STRIPE_PUBLIC_CHECKOUT")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_api_base_url: env::var("STRIPE_API_BASE_URL")
            .unwrap_or_else(|_| STRIPE_API_BASE_URL.to_string()),
        stripe_payment_method_configuration: env::var("STRIPE_PAYMENT_METHOD_CONFIGURATION")
            .ok()
            .and_then(non_empty_secret),
        store,
        base_rpc_urls: BaseRpcUrlConfig::from_env(),
        base_broadcast_enabled: env::var("ENABLE_BASE_TX_BROADCAST")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        operator_api_token: env::var("OPERATOR_API_TOKEN")
            .ok()
            .and_then(non_empty_secret),
        discoverability_ingest_token: env::var("DISCOVERABILITY_INGEST_TOKEN")
            .ok()
            .and_then(non_empty_secret),
        analytics_exclusion_token: env::var("ANALYTICS_EXCLUSION_TOKEN")
            .ok()
            .and_then(non_empty_secret),
        distribution_attribution_signing_secret: env::var(
            "DISTRIBUTION_ATTRIBUTION_SIGNING_SECRET",
        )
        .ok()
        .and_then(non_empty_secret),
        distribution_excluded_wallet_classes,
        public_base_url: env::var("PUBLIC_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string()),
        mcp_base_url: env::var("MCP_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8090".to_string()),
        x402_relayer,
        bond_sponsor,
        recovery_reservations,
        cloud_agent,
        discovery_webhooks,
        neynar_social,
    });
    let public_app = Router::new()
        .route("/health", get(health))
        .merge(a2a::router())
        .merge(discoverability::router())
        .merge(distribution::router())
        .route("/llms.txt", get(llms_txt))
        .route("/v1/legal/policy", get(legal_policy))
        .route("/v1/legal/acceptances", post(record_legal_acceptance))
        .route(
            "/schemas/discovery-manifest.v2.json",
            get(discovery_manifest_schema),
        )
        .route(
            "/.well-known/agent-bounties.json",
            get(agent_bounties_discovery),
        )
        .route("/.well-known/x402.json", get(x402_discovery))
        .route("/v1/discovery", get(agent_bounties_discovery))
        .route("/v1/risk/policy", get(risk_policy))
        .route("/v1/readiness/live-money", get(live_money_readiness))
        .route("/v1/cloud-agent/readiness", get(cloud_agent_readiness))
        .route(
            "/v1/cloud-agent/objective-plans",
            post(compile_objective_with_cloud_agent),
        )
        .route(
            "/v1/cloud-agent/bounty-drafts",
            post(draft_bounty_with_cloud_agent),
        )
        .route(
            "/v1/base/autonomous-bounties/:bounty_contract/analysis",
            get(analyze_bounty_fit),
        )
        .route(
            "/v1/github/bounty-discovery-v1",
            get(github_bounty_discovery),
        )
        .route("/v1/opportunities", get(list_opportunities))
        .route("/v1/opportunities/stream", get(stream_opportunities))
        .route(
            "/v1/opportunities/:opportunity_id/comments",
            get(list_opportunity_comments).post(create_opportunity_comment),
        )
        .route(
            "/v1/chatgpt/action-intents",
            post(create_chatgpt_action_intent),
        )
        .route(
            "/v1/chatgpt/action-intents/:intent_id",
            get(get_chatgpt_action_intent),
        )
        .route(
            "/v1/chatgpt/action-intents/:intent_id/observations",
            post(observe_chatgpt_action_transaction),
        )
        .route("/v1/opportunities/feed.rss", get(opportunity_feed_rss))
        .route("/v1/opportunities/feed.atom", get(opportunity_feed_atom))
        .route("/v1/opportunities/feed.json", get(opportunity_feed_json))
        .route(
            "/v1/opportunities/conversion-funnel",
            get(opportunity_conversion_funnel),
        )
        .route("/v1/analytics/events", post(record_site_analytics_event))
        .route("/v1/analytics/site", get(site_analytics))
        .route("/v1/metrics/platform", get(platform_metrics))
        .route(
            "/public/opportunities/:opportunity_id/embed",
            get(opportunity_embed_page),
        )
        .route(
            "/public/opportunities/:opportunity_id/embed.svg",
            get(opportunity_embed_svg),
        )
        .route(
            "/public/opportunities/:opportunity_id/embed.md",
            get(opportunity_embed_markdown),
        )
        .route(
            "/v1/discovery/subscriptions",
            post(create_discovery_subscription),
        )
        .route(
            "/v1/discovery/subscriptions/:id",
            get(get_discovery_subscription).delete(delete_discovery_subscription),
        )
        .route(
            "/v1/unfunded-bounties",
            get(list_unfunded_bounties).post(publish_unfunded_bounty),
        )
        .route("/v1/unfunded-bounties/:id", get(get_unfunded_bounty))
        .route(
            "/v1/unfunded-bounties/:id/solutions",
            post(submit_unfunded_bounty_solution),
        )
        .route(
            "/v1/base/agent-wallet/readiness",
            post(prepare_agent_wallet_to_earn),
        )
        .route(
            "/v1/base/open-competition-v1/verifiers",
            get(list_open_competition_verifiers),
        )
        .route(
            "/v1/base/open-competition-v1/events",
            get(list_open_competition_events),
        )
        .route(
            "/v1/base/open-competition-v1/creation-preparation",
            post(prepare_open_competition_creation),
        )
        .route(
            "/v1/base/open-competition-v1/authorized-creation-preparation",
            post(prepare_open_competition_authorized_creation),
        )
        .route(
            "/v1/base/open-competition-v1/state",
            get(get_open_competition_state),
        )
        .route(
            "/v1/base/open-competition-v1/readiness",
            get(get_open_competition_readiness),
        )
        .route(
            "/v1/base/open-competition-v1/commit-preparation",
            post(prepare_open_competition_commit),
        )
        .route(
            "/v1/base/open-competition-v1/reveal-preparation",
            post(prepare_open_competition_reveal),
        )
        .route(
            "/v1/base/open-competition-v1/entrant-action-preparation",
            post(prepare_open_competition_entrant_action),
        )
        .route(
            "/v1/base/open-competition-v1/entrant-action-relays",
            post(relay_open_competition_entrant_action),
        )
        .route(
            "/v1/base/open-competition-v1/entrant-action-relays/:relay_id",
            get(get_open_competition_entrant_relay),
        )
        .route(
            "/v1/base/open-competition-v1/status",
            post(get_open_competition_status),
        )
        .route(
            "/v1/base/open-competition-v1/bond-withdrawal-preparation",
            post(withdraw_open_competition_bond),
        )
        .merge(open_competition_v2_api::router())
        .route(
            "/v1/base/standing-meta-v4/readiness",
            get(get_standing_meta_v4_readiness),
        )
        .route(
            "/v1/base/standing-meta-v4/claim-preparation",
            post(prepare_standing_meta_v4_claim),
        )
        .route(
            "/v1/base/standing-meta-v4/stake-registration-preparation",
            post(prepare_anonymous_stake_registration),
        )
        .route(
            "/v1/base/standing-meta-v4/stake-availability-preparation",
            post(set_anonymous_stake_availability),
        )
        .route(
            "/v1/base/standing-meta-v4/verification-assignments",
            post(list_verification_assignments),
        )
        .route(
            "/v1/base/standing-meta-v4/primary-verdict-preparation",
            post(submit_primary_verdict),
        )
        .route(
            "/v1/base/standing-meta-v4/appeal-waiver-preparation",
            post(waive_verification_appeal),
        )
        .route(
            "/v1/base/standing-meta-v4/appeal-opening-preparation",
            post(open_verification_appeal),
        )
        .route(
            "/v1/base/standing-meta-v4/appeal-vote-preparation",
            post(submit_appeal_vote),
        )
        .route(
            "/v1/base/standing-meta-v4/finalization-preparation",
            post(finalize_verification_case),
        )
        .route("/v1/risk/events", get(list_risk_events))
        .route("/v1/risk/reviews", get(list_risk_reviews))
        .route("/v1/risk/bounty-approvals", post(approve_risk_bounty))
        .route("/v1/risk/payout-approvals", post(approve_risk_payout))
        .route("/v1/risk/events/:id/reject", post(reject_risk_event))
        .route("/v1/route-blocked-goal", post(route_blocked_goal))
        .route("/v1/evals/bountybench", get(run_bountybench))
        .route("/v1/evals/abusebench", get(run_abusebench))
        .route("/v1/evals/judgebench", get(run_judgebench))
        .route("/v1/evals/loops", get(run_eval_loop_suite))
        .route("/v1/evals/runs", get(list_eval_runs))
        .route("/v1/agents", post(register_agent))
        .route("/v1/agents/:id/paid-status", get(agent_paid_status))
        .route(
            "/v1/contributor-contacts",
            post(upsert_contributor_contact).get(list_contributor_contacts),
        )
        .route(
            "/v1/audience/members",
            post(upsert_audience_member).get(list_audience_members),
        )
        .route(
            "/v1/audience/interactions",
            post(record_audience_interaction).get(list_audience_interactions),
        )
        .route(
            "/v1/audience/discovery-responses",
            post(record_discovery_response).get(list_discovery_responses),
        )
        .route(
            "/v1/audience/outreach-attempts",
            post(record_outreach_attempt).get(list_outreach_attempts),
        )
        .route("/v1/audience/report", get(audience_report))
        .route(
            "/v1/objectives/creation-plans",
            post(plan_objective_creation),
        )
        .route(
            "/v1/objectives",
            post(create_objective).get(list_objectives),
        )
        .route("/v1/objectives/:id", get(get_objective))
        .route(
            "/v1/objectives/:id/action-plans",
            post(plan_objective_action),
        )
        .route("/v1/objectives/:id/actions", post(apply_objective_action))
        .route("/v1/objectives/:id/reconcile", post(reconcile_objective))
        .route("/v1/capabilities", post(register_capability))
        .route("/v1/capabilities/feed", get(public_capability_feed))
        .route("/v1/capabilities/search", post(search_capabilities))
        .route("/v1/help-requests", post(create_help_request))
        .route("/v1/help-requests/:id/quotes", post(request_quotes))
        .route("/v1/quotes/:id/fund-bounty", post(fund_quote))
        .route("/v1/bounties", post(post_bounty))
        .route("/v1/bounties/pooled", post(open_pooled_bounty))
        .route("/v1/bounties/claimable", get(list_claimable_bounties))
        .route("/v1/bounties/feed", get(public_bounty_feed))
        .route("/v1/bounties/funding-feed", get(public_funding_feed))
        .route(
            "/v1/x402/base/bounties/:bounty_contract/funding",
            get(x402_base_bounty_funding),
        )
        .route("/v1/x402/base/relays/:relay_id", get(get_x402_relay))
        .route(
            "/v1/bounties/:id/funding-intents",
            post(create_funding_intent),
        )
        .route(
            "/v1/bounties/:id/funding-contributions",
            post(add_funding_contribution),
        )
        .route("/v1/bounties/:id/claim", post(claim_bounty))
        .route("/v1/bounties/:id/submit", post(submit_result))
        .route("/v1/bounties/:id/verify", post(verify_submission))
        .route("/v1/bounties/:id", get(bounty_status))
        .route(
            "/v1/base/broadcast-signed-transaction",
            post(broadcast_base_signed_transaction),
        )
        .route(
            "/v1/base/transaction-receipt",
            post(get_base_transaction_receipt),
        )
        .route(
            "/v1/base/autonomous-bounties/canonical-child-terms-plan",
            post(plan_autonomous_canonical_child_terms),
        )
        .route(
            "/v1/base/autonomous-bounties/standing-meta-v2-child-preparation",
            post(prepare_standing_meta_v2_child),
        )
        .route(
            "/v1/base/autonomous-bounties/creation-plan",
            post(plan_autonomous_bounty_creation),
        )
        .route(
            "/v1/base/autonomous-bounties/authorized-creation-plan",
            post(plan_autonomous_bounty_authorized_creation),
        )
        .route(
            "/v1/base/autonomous-bounties/contribution-plan",
            post(plan_autonomous_bounty_contribution),
        )
        .route(
            "/v1/base/autonomous-bounties/authorized-contribution-plan",
            post(plan_autonomous_bounty_authorized_contribution),
        )
        .route(
            "/v1/base/autonomous-bounties/claim-plan",
            post(plan_autonomous_bounty_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/authorized-claim-plan",
            post(plan_autonomous_bounty_authorized_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/claims",
            post(agent_native_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/claim-funnel",
            get(claim_funnel),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-plan",
            post(plan_autonomous_bounty_submission),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-preparation",
            post(prepare_autonomous_bounty_submission),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-authorization-plan",
            post(plan_autonomous_bounty_submission_authorization),
        )
        .route(
            "/v1/base/autonomous-bounties/verification-attestation-plan",
            post(plan_autonomous_verification_attestation),
        )
        .route(
            "/v1/base/autonomous-bounties/module-settlement-plan",
            post(plan_autonomous_module_settlement),
        )
        .route(
            "/v1/base/autonomous-bounties/attestation-settlement-plan",
            post(plan_autonomous_attestation_settlement),
        )
        .route(
            "/v1/base/autonomous-bounties/expire-claim-plan",
            post(plan_autonomous_expire_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/expire-submission-plan",
            post(plan_autonomous_expire_submission),
        )
        .route(
            "/v1/base/autonomous-bounties/timeout-relay",
            post(relay_autonomous_timeout),
        )
        .route(
            "/v1/base/autonomous-bounties/cancel-plan",
            post(plan_autonomous_cancel),
        )
        .route(
            "/v1/base/autonomous-bounties/refund-withdrawal-plan",
            post(plan_autonomous_refund_withdrawal),
        )
        .route(
            "/v1/base/autonomous-bounties/bounded-wallet-cancel-refund-plan",
            post(plan_bounded_wallet_cancel_refund),
        )
        .route(
            "/v1/base/autonomous-bounties/decode-events",
            post(decode_autonomous_bounty_events),
        )
        .route(
            "/v1/base/autonomous-bounties/events",
            get(list_autonomous_bounty_events),
        )
        .route(
            "/v1/base/autonomous-bounties/terms",
            post(publish_autonomous_bounty_terms),
        )
        .route(
            "/v1/base/autonomous-bounties/terms/:terms_hash",
            get(get_autonomous_bounty_terms),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-evidence",
            post(publish_autonomous_submission_evidence),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-evidence/:bounty_contract/:round",
            get(get_autonomous_submission_evidence),
        )
        .route(
            "/v1/base/autonomous-bounties/feed",
            get(autonomous_bounty_feed),
        )
        .route(
            "/v1/base/autonomous-bounties/leaderboard",
            get(solver_leaderboard),
        )
        .route(
            "/v1/base/autonomous-bounties/inventory-summary",
            get(autonomous_bounty_inventory_summary),
        )
        .route(
            "/v1/base/autonomous-bounties/inventory-badge.svg",
            get(autonomous_bounty_inventory_badge),
        )
        .route(
            "/v1/base/autonomous-bounties/verification-jobs",
            get(autonomous_verification_jobs),
        )
        .route(
            "/v1/stripe/checkout-top-ups",
            post(plan_stripe_checkout_top_up),
        )
        .route(
            "/v1/stripe/connect-accounts",
            post(plan_stripe_connect_account),
        )
        .route(
            "/v1/stripe/connect-transfers",
            post(plan_stripe_connect_transfer),
        )
        .route(
            "/v1/stripe/live/checkout-top-ups",
            post(execute_stripe_checkout_top_up),
        )
        .route(
            "/v1/stripe/live/funding-intents/:id/checkout-session",
            post(execute_stripe_funding_intent_checkout),
        )
        .route(
            "/v1/stripe/live/connect-accounts",
            post(execute_stripe_connect_account),
        )
        .route(
            "/v1/stripe/live/connect-transfers",
            post(execute_stripe_connect_transfer),
        )
        .route(
            "/v1/stripe/connect-snapshots",
            post(reconcile_stripe_connect_snapshot),
        )
        .route(
            "/v1/stripe/transfer-events",
            post(reconcile_stripe_transfer_event),
        )
        .route(
            "/v1/stripe/checkout-webhooks",
            post(reconcile_stripe_checkout_webhook),
        )
        .route(
            "/v1/github/issue-bounty-plan",
            post(plan_github_issue_bounty),
        )
        .route(
            "/v1/github/issue-api-sync-plan",
            post(plan_github_issue_api_sync),
        )
        .route(
            "/v1/github/issue-api-sync",
            post(sync_github_issue_api_bounty),
        )
        .route(
            "/v1/github/create-comment-plan",
            post(plan_github_create_comment),
        )
        .route(
            "/v1/github/funding-comment-plan",
            post(plan_github_funding_comment),
        )
        .route(
            "/v1/github/claim-comment-plan",
            post(plan_github_claim_comment),
        )
        .route(
            "/v1/social/mention-draft-plan",
            post(plan_social_mention_draft),
        )
        .route(
            "/v1/social/mention-ingestion/readiness",
            get(social_mention_ingestion_readiness),
        )
        .route(
            "/v1/social/webhooks/neynar",
            post(ingest_neynar_social_mention),
        )
        .route(
            "/v1/social/mention-drafts/:id",
            get(get_social_mention_draft),
        )
        .route(
            "/v1/github/proof-comment-plan",
            post(plan_github_proof_comment),
        )
        .route(
            "/v1/github/proof-comment-plan-from-proof",
            post(plan_github_proof_comment_from_proof),
        )
        .route("/public/proofs/:id", get(public_proof_page))
        .route("/public/agents/:id", get(public_agent_profile))
        .route("/public/capabilities", get(public_capability_feed_page))
        .route("/public/verifiers/:kind", get(public_verifier_profile))
        .route("/public/bounties", get(public_bounty_feed_page))
        .route("/public/funding", get(public_funding_feed_page))
        .route("/public/bounties/:id", get(public_bounty_page))
        .route("/public/templates", get(public_template_index))
        .route("/public/templates/:slug", get(public_template_page))
        .route("/api-docs/openapi.json", get(openapi_json))
        .route("/docs", get(api_docs))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            observe_interface_usage,
        ))
        .layer(CorsLayer::permissive())
        .layer(middleware::from_fn(redirect_marketing_domain))
        .with_state(state);
    let app = public_app.merge(site_auth::router(site_auth));

    let bind_addr = service_bind_addr(
        env::var("API_BIND_ADDR").ok().as_deref(),
        env::var("PORT").ok().as_deref(),
        "127.0.0.1:8080",
    );
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn service_bind_addr(configured: Option<&str>, port: Option<&str>, default_addr: &str) -> String {
    configured
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            port.filter(|value| !value.trim().is_empty())
                .map(|value| format!("0.0.0.0:{}", value.trim()))
        })
        .unwrap_or_else(|| default_addr.to_string())
}

fn attributed_api_interface(headers: &HeaderMap) -> Option<ObservedInterface> {
    let value = headers.get(INTERFACE_ATTRIBUTION_HEADER)?.to_str().ok()?;
    if value.eq_ignore_ascii_case("api") {
        Some(ObservedInterface::Api)
    } else if value.eq_ignore_ascii_case("cli") {
        Some(ObservedInterface::Cli)
    } else {
        None
    }
}

fn discovery_route_attribution(
    path: &str,
    headers: &HeaderMap,
) -> Option<(
    DiscoveryInterface,
    DiscoveryRouteFamily,
    AttributionReliability,
)> {
    if path == "/.well-known/agent-card.json" {
        return Some((
            DiscoveryInterface::A2a,
            DiscoveryRouteFamily::AgentCard,
            AttributionReliability::Observed,
        ));
    }
    if path.starts_with("/a2a/v1/") && path != "/a2a/v1/message:send" {
        return Some((
            DiscoveryInterface::A2a,
            DiscoveryRouteFamily::ProtocolOrientation,
            AttributionReliability::Observed,
        ));
    }
    if matches!(
        path,
        "/v1/opportunities/feed.rss"
            | "/v1/opportunities/feed.atom"
            | "/v1/opportunities/feed.json"
    ) {
        return Some((
            DiscoveryInterface::Feed,
            DiscoveryRouteFamily::OpportunityList,
            AttributionReliability::Observed,
        ));
    }
    let route_family = if path == "/v1/opportunities" {
        Some(DiscoveryRouteFamily::OpportunityList)
    } else if path.starts_with("/public/opportunities/") {
        Some(DiscoveryRouteFamily::OpportunityDetail)
    } else if path.starts_with("/v1/discovery/subscriptions") {
        Some(DiscoveryRouteFamily::Alerts)
    } else if matches!(
        path,
        "/.well-known/agent-bounties.json"
            | "/v1/discovery"
            | "/llms.txt"
            | "/api-docs/openapi.json"
            | "/docs"
    ) {
        Some(DiscoveryRouteFamily::ProtocolOrientation)
    } else {
        None
    }?;
    let interface = match attributed_api_interface(headers)? {
        ObservedInterface::Api => DiscoveryInterface::Api,
        ObservedInterface::Cli => DiscoveryInterface::Cli,
        ObservedInterface::Mcp => return None,
    };
    Some((interface, route_family, AttributionReliability::Declared))
}

fn analytics_exclusion_is_authorized(
    analytics_exclusion_token: Option<&str>,
    operator_api_token: Option<&str>,
    headers: &HeaderMap,
) -> bool {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    analytics_exclusion_token.is_some()
        && service_runtime::operator_token_is_authorized(
            analytics_exclusion_token,
            headers
                .get(ANALYTICS_EXCLUSION_HEADER)
                .and_then(|value| value.to_str().ok()),
            None,
        )
        || (operator_api_token.is_some()
            && service_runtime::operator_token_is_authorized(
                operator_api_token,
                headers
                    .get(OPERATOR_TOKEN_HEADER)
                    .and_then(|value| value.to_str().ok()),
                authorization,
            ))
}

async fn observe_interface_usage(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    let interface = attributed_api_interface(request.headers());
    let discovery_route = discovery_route_attribution(request.uri().path(), request.headers());
    let excluded = analytics_exclusion_is_authorized(
        state.analytics_exclusion_token.as_deref(),
        state.operator_api_token.as_deref(),
        request.headers(),
    );
    let mut response = next.run(request).await;
    if excluded {
        response
            .headers_mut()
            .insert(ANALYTICS_EXCLUDED_HEADER, HeaderValue::from_static("true"));
    }
    if !excluded {
        if let (Some(store), Some(interface)) = (state.store.clone(), interface) {
            let succeeded = response.status().is_success();
            tokio::spawn(async move {
                let _ = store
                    .record_interface_usage(
                        interface,
                        ObservedProtocolEra::NotApplicable,
                        succeeded,
                        Utc::now(),
                    )
                    .await;
            });
        }
        if let (Some(store), Some((interface, route_family, reliability))) =
            (state.store.clone(), discovery_route)
        {
            let succeeded = response.status().is_success();
            tokio::spawn(async move {
                let _ = store
                    .record_discovery_route_usage(
                        interface,
                        route_family,
                        reliability,
                        succeeded,
                        Utc::now(),
                    )
                    .await;
            });
        }
    }
    response
}

#[utoipa::path(get, path = "/health", responses((status = 200, body = String)))]
async fn health(State(state): State<SharedState>) -> Response {
    let mut response = health_response(&deployment_revision()).into_response();
    response.headers_mut().insert(
        "x-agent-bounties-x402-relay",
        HeaderValue::from_static(if state.x402_relayer.enabled {
            "enabled"
        } else {
            "disabled"
        }),
    );
    if let Some(address) = state.x402_relayer.address() {
        if let Ok(value) = HeaderValue::from_str(&address) {
            response
                .headers_mut()
                .insert("x-agent-bounties-x402-relayer", value);
        }
    }
    response
}

fn deployment_revision() -> String {
    env::var("RENDER_GIT_COMMIT")
        .ok()
        .filter(|value| {
            value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| "local".to_string())
}

fn health_response(revision: &str) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-agent-bounties-revision",
        HeaderValue::from_str(revision).unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    headers.insert(
        "x-agent-bounties-protocol",
        HeaderValue::from_static("agent-bounties/autonomous-v1"),
    );
    (headers, "ok")
}

#[utoipa::path(get, path = "/llms.txt", responses((status = 200, body = String)))]
async fn llms_txt(State(state): State<SharedState>) -> String {
    web_public::render_llms_txt(&state.public_base_url, &state.mcp_base_url)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct LegalPolicyResponse {
    schema_version: String,
    terms_version: String,
    privacy_version: String,
    statement: String,
    statement_hash: String,
    terms_url: String,
    privacy_url: String,
    supported_actions: Vec<String>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct RecordLegalAcceptanceRequest {
    terms_version: String,
    privacy_version: String,
    action: String,
    wallet_address: String,
    statement_hash: String,
    acceptance_method: String,
    accepted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct LegalAcceptanceResponse {
    schema_version: String,
    acceptance_id: Uuid,
    terms_version: String,
    privacy_version: String,
    action: String,
    wallet_address: String,
    statement_hash: String,
    acceptance_method: String,
    accepted_at: DateTime<Utc>,
    recorded_at: DateTime<Utc>,
    evidence_boundary: String,
}

fn legal_statement_hash() -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(LEGAL_ACCEPTANCE_STATEMENT.as_bytes()))
    )
}

fn build_legal_policy(public_base_url: &str) -> LegalPolicyResponse {
    let base = public_base_url.trim_end_matches('/');
    LegalPolicyResponse {
        schema_version: "agent-bounties/legal-policy-v1".to_string(),
        terms_version: LEGAL_TERMS_VERSION.to_string(),
        privacy_version: LEGAL_PRIVACY_VERSION.to_string(),
        statement: LEGAL_ACCEPTANCE_STATEMENT.to_string(),
        statement_hash: legal_statement_hash(),
        terms_url: format!("{base}/terms.html"),
        privacy_url: format!("{base}/privacy.html"),
        supported_actions: LEGAL_ACTIONS.iter().map(|action| (*action).to_string()).collect(),
        evidence_boundary: "This policy and an acceptance receipt record explicit assent on the hosted interface. Neither is a wallet signature, funding event, verifier verdict, settlement, legal advice, identity proof, or proof that the wallet controller had authority.".to_string(),
    }
}

fn legal_website_base_url(configured: Option<String>, public_base_url: &str) -> String {
    configured.and_then(non_empty_secret).unwrap_or_else(|| {
        match public_base_url.trim_end_matches('/') {
            "https://api.agentbounties.app" => "https://agentbounties.app".to_string(),
            value => value.to_string(),
        }
    })
}

#[utoipa::path(
    get,
    path = "/v1/legal/policy",
    responses((status = 200, body = LegalPolicyResponse))
)]
async fn legal_policy(State(state): State<SharedState>) -> Json<LegalPolicyResponse> {
    let website_base_url =
        legal_website_base_url(env::var("WEBSITE_BASE_URL").ok(), &state.public_base_url);
    Json(build_legal_policy(&website_base_url))
}

#[utoipa::path(
    post,
    path = "/v1/legal/acceptances",
    request_body = RecordLegalAcceptanceRequest,
    responses(
        (status = 201, body = LegalAcceptanceResponse),
        (status = 400, description = "Unsupported, stale, or malformed acceptance"),
        (status = 503, description = "Durable acceptance store unavailable")
    )
)]
async fn record_legal_acceptance(
    State(state): State<SharedState>,
    Json(request): Json<RecordLegalAcceptanceRequest>,
) -> Result<(StatusCode, Json<LegalAcceptanceResponse>), StatusCode> {
    let wallet_address = validate_legal_acceptance_request(&request, Utc::now())?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let acceptance = store
        .record_legal_acceptance(&NewLegalAcceptance {
            id: Uuid::new_v4(),
            terms_version: request.terms_version,
            privacy_version: request.privacy_version,
            action: request.action,
            wallet_address,
            statement_hash: request.statement_hash,
            acceptance_method: request.acceptance_method,
            accepted_at: request.accepted_at,
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((
        StatusCode::CREATED,
        Json(LegalAcceptanceResponse {
            schema_version: "agent-bounties/legal-acceptance-v1".to_string(),
            acceptance_id: acceptance.id,
            terms_version: acceptance.terms_version,
            privacy_version: acceptance.privacy_version,
            action: acceptance.action,
            wallet_address: acceptance.wallet_address,
            statement_hash: acceptance.statement_hash,
            acceptance_method: acceptance.acceptance_method,
            accepted_at: acceptance.accepted_at,
            recorded_at: acceptance.recorded_at,
            evidence_boundary: "This receipt records explicit hosted-interface assent. It does not prove identity, authority, funding, task completion, verification, or payment.".to_string(),
        }),
    ))
}

fn validate_legal_acceptance_request(
    request: &RecordLegalAcceptanceRequest,
    now: DateTime<Utc>,
) -> Result<String, StatusCode> {
    if request.terms_version != LEGAL_TERMS_VERSION
        || request.privacy_version != LEGAL_PRIVACY_VERSION
        || request.statement_hash != legal_statement_hash()
        || !LEGAL_ACTIONS.contains(&request.action.as_str())
        || !matches!(
            request.acceptance_method.as_str(),
            "web_clickwrap" | "api_explicit"
        )
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if request.accepted_at < now - ChronoDuration::minutes(15)
        || request.accepted_at > now + ChronoDuration::minutes(5)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let wallet =
        normalize_evm_address(&request.wallet_address).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(wallet.to_ascii_lowercase())
}

#[utoipa::path(
    get,
    path = "/v1/cloud-agent/readiness",
    responses((status = 200, body = CloudAgentReadiness))
)]
async fn cloud_agent_readiness(State(state): State<SharedState>) -> Json<CloudAgentReadiness> {
    Json(state.cloud_agent.readiness())
}

#[utoipa::path(
    post,
    path = "/v1/cloud-agent/objective-plans",
    request_body = CloudObjectivePlanRequest,
    responses(
        (status = 200, body = CloudObjectivePlan),
        (status = 400, body = AgentActionError, description = "Invalid objective or budget input"),
        (status = 401, description = "Public cloud planning is disabled and operator authorization is absent"),
        (status = 429, body = AgentActionError, description = "Bounded daily cloud-model quota exhausted"),
        (status = 502, body = AgentActionError, description = "GPT-5.6 returned a plan that still failed deterministic validation after one repair attempt"),
        (status = 503, body = AgentActionError, description = "GPT-5.6 cloud planning is not configured or unavailable")
    )
)]
async fn compile_objective_with_cloud_agent(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<CloudObjectivePlanRequest>,
) -> Result<Json<CloudObjectivePlan>, AgentActionApiError> {
    if !state.cloud_agent.public_drafts() {
        require_operator(&state, &headers).map_err(cloud_agent_access_error)?;
    }
    state
        .cloud_agent
        .compile_objective(request)
        .await
        .map(Json)
        .map_err(cloud_agent_api_error)
}

#[utoipa::path(
    post,
    path = "/v1/cloud-agent/bounty-drafts",
    request_body = CloudBountyDraftRequest,
    responses(
        (status = 200, body = CloudBountyDraft),
        (status = 400, body = AgentActionError, description = "Invalid or unverifiable drafting input"),
        (status = 401, description = "Public drafts are disabled and operator authorization is absent"),
        (status = 429, body = AgentActionError, description = "Bounded daily cloud-model quota exhausted"),
        (status = 502, body = AgentActionError, description = "Cloud model returned invalid structured output"),
        (status = 503, body = AgentActionError, description = "Cloud model is not configured or unavailable")
    )
)]
async fn draft_bounty_with_cloud_agent(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<CloudBountyDraftRequest>,
) -> Result<Json<CloudBountyDraft>, AgentActionApiError> {
    if !state.cloud_agent.public_drafts() {
        require_operator(&state, &headers).map_err(cloud_agent_access_error)?;
    }
    state
        .cloud_agent
        .draft(request)
        .await
        .map(Json)
        .map_err(cloud_agent_api_error)
}

#[utoipa::path(
    get,
    path = "/v1/base/autonomous-bounties/{bounty_contract}/analysis",
    params(
        ("bounty_contract" = String, Path, description = "Indexed canonical autonomous-v1 bounty contract"),
        ("network" = Option<String>, Query, description = "base-mainnet or base-sepolia; defaults to base-mainnet")
    ),
    responses(
        (status = 200, body = CloudBountyAnalysis),
        (status = 400, description = "Invalid network or bounded analysis input"),
        (status = 401, description = "Public cloud analysis is disabled and operator authorization is absent"),
        (status = 404, description = "Canonical bounty is not indexed"),
        (status = 409, description = "Published terms are missing, invalid, or inconsistent with canonical creation"),
        (status = 429, description = "Bounded daily cloud-model quota exhausted"),
        (status = 503, description = "Cloud model or canonical read model is unavailable")
    )
)]
async fn analyze_bounty_fit(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(bounty_contract): Path<String>,
    Query(query): Query<CloudBountyAnalysisQuery>,
) -> Result<Json<CloudBountyAnalysis>, StatusCode> {
    if !state.cloud_agent.public_drafts() {
        require_operator(&state, &headers)?;
    }
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &bounty_contract).await?;
    if !item.terms_valid || !item.validation_errors.is_empty() {
        return Err(StatusCode::CONFLICT);
    }
    let terms = item.terms.as_ref().ok_or(StatusCode::CONFLICT)?;
    let projected = canonical_opportunity(&item, network, &state.public_base_url)
        .ok_or(StatusCode::CONFLICT)?;
    let request = CloudBountyAnalysisRequest {
        terms_hash: item.terms_hash.clone(),
        title: terms.document.title.clone(),
        goal: terms.document.goal.clone(),
        acceptance_criteria: terms.document.acceptance_criteria.clone(),
        benchmark: terms.document.benchmark.clone(),
        evidence_schema: terms.document.evidence_schema.clone(),
        verification_policy: terms.document.verification_policy.clone(),
        reward: serde_json::to_value(&projected.reward)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        bond: serde_json::to_value(&projected.bond)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        deadline: projected.deadline,
        payment_status: serde_json::json!({
            "work_state": projected.work_state,
            "payment_state": projected.payment_state,
            "payment_committed": projected.payment_committed,
            "funded_amount": projected.funded_amount,
            "funding_target": projected.funding_target,
            "source_status": projected.source_status,
        }),
    };
    state
        .cloud_agent
        .analyze_bounty_fit(request)
        .await
        .map(Json)
        .map_err(cloud_agent_status)
}

#[utoipa::path(
    get,
    path = "/v1/opportunities",
    params(
        ("network" = Option<String>, Query, description = "Canonical network key; defaults to base-mainnet"),
        ("view" = Option<String>, Query, description = "Deterministic view: recent, engineering, creative, urgent, seeking_funding, or ready_to_earn"),
        ("source_type" = Option<String>, Query, description = "Filter by unfunded_offchain, legacy_bounty, or canonical_base"),
        ("work_state" = Option<String>, Query, description = "Filter by open, claimable, in_progress, submitted, or completed"),
        ("payment_state" = Option<String>, Query, description = "Filter by none, seeking_funding, escrowed, or paid"),
        ("limit" = Option<u32>, Query, description = "Maximum combined results; clamped to 1..300")
    ),
    responses(
        (status = 200, body = OpportunityProjectionResponse),
        (status = 400, description = "Unknown network, view, work state, payment state, or source type")
    )
)]
async fn list_opportunities(
    State(state): State<SharedState>,
    Query(query): Query<OpportunityQuery>,
) -> Result<Json<OpportunityProjectionResponse>, StatusCode> {
    build_opportunity_projection(&state, query).await.map(Json)
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubDiscoveryQuery {
    network: Option<String>,
}

#[utoipa::path(
    get,
    path = "/v1/github/bounty-discovery-v1",
    params(("network" = Option<String>, Query, description = "Canonical Base network; defaults to base-mainnet")),
    responses(
        (status = 200, body = GitHubDiscoveryProjectionResponse),
        (status = 400, description = "Unknown Base network"),
        (status = 503, description = "Projection identity conflict or malformed canonical record")
    )
)]
async fn github_bounty_discovery(
    State(state): State<SharedState>,
    Query(query): Query<GitHubDiscoveryQuery>,
) -> Result<Json<GitHubDiscoveryProjectionResponse>, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let (descriptor, rpc_url) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let generated_at = Utc::now();
    let safe_block = match tokio::time::timeout(
        Duration::from_secs(12),
        fetch_safe_block_identity(&rpc_url, 92),
    )
    .await
    {
        Ok(Ok(block)) => {
            let age_seconds = generated_at.timestamp() - block.timestamp as i64;
            Some(GitHubDiscoverySafeBlock {
                number: block.number,
                hash: block.hash,
                timestamp: block.timestamp,
                age_seconds,
                fresh: (0..=300).contains(&age_seconds),
            })
        }
        _ => None,
    };

    let website_base_url =
        legal_website_base_url(env::var("WEBSITE_BASE_URL").ok(), &state.public_base_url);
    let safe_block_number = safe_block.as_ref().map(|block| block.number);
    let mut items = Vec::new();
    let mut source_statuses = Vec::new();

    let autonomous_factory = autonomous_factory_for_chain(descriptor.chain_id);
    let autonomous_result = async {
        match (
            state.store.as_ref(),
            autonomous_factory.as_deref(),
            safe_block_number,
        ) {
            (Some(store), Some(factory), Some(safe_number)) => {
                let heartbeat = store
                    .get_base_indexer_heartbeat(network, factory)
                    .await
                    .map_err(|_| "autonomous_heartbeat_unavailable".to_string())?
                    .ok_or_else(|| "autonomous_heartbeat_missing".to_string());
                match heartbeat {
                    Ok(heartbeat)
                        if open_competition_monitoring_is_fresh(
                            &heartbeat,
                            safe_number,
                            generated_at,
                        ) =>
                    {
                        let feed = load_autonomous_bounty_feed(&state, network, false)
                            .await
                            .map_err(|_| "autonomous_read_model_unavailable".to_string())?;
                        let projected = autonomous_discovery_items(
                            &feed,
                            network,
                            descriptor.chain_id,
                            &state.public_base_url,
                            &website_base_url,
                        )?;
                        Ok((projected, heartbeat.persisted_cursor_block))
                    }
                    Ok(heartbeat) => Err(format!(
                        "autonomous_indexer_stale:{}",
                        heartbeat.persisted_cursor_block.unwrap_or_default()
                    )),
                    Err(error) => Err(error),
                }
            }
            _ => Err("autonomous_source_not_configured".to_string()),
        }
    }
    .await;
    match autonomous_result {
        Ok((mut projected, cursor)) => {
            source_statuses.push(GitHubDiscoverySourceStatus {
                source_type: "canonical_autonomous".to_string(),
                protocol_version: AUTONOMOUS_PROTOCOL_VERSION.to_string(),
                factory_contract: autonomous_factory.clone(),
                available: true,
                fresh: true,
                item_count: projected.len(),
                persisted_cursor_block: cursor,
                error: None,
            });
            items.append(&mut projected);
        }
        Err(error) => source_statuses.push(GitHubDiscoverySourceStatus {
            source_type: "canonical_autonomous".to_string(),
            protocol_version: AUTONOMOUS_PROTOCOL_VERSION.to_string(),
            factory_contract: autonomous_factory.clone(),
            available: false,
            fresh: false,
            item_count: 0,
            persisted_cursor_block: None,
            error: Some(error),
        }),
    }

    let open_competition_result = async {
        let release = open_competition_release_from_environment(network)
            .map_err(|_| "open_competition_release_unavailable".to_string())?;
        if release.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn {
            return Err("open_competition_release_not_active".to_string());
        }
        let prefix = open_competition_environment_prefix(network)
            .map_err(|_| "open_competition_network_unknown".to_string())?;
        let public_activation_block = env::var(format!("{prefix}_PUBLIC_ACTIVATION_BLOCK"))
            .map_err(|_| "open_competition_activation_block_missing".to_string())?
            .parse::<u64>()
            .map_err(|_| "open_competition_activation_block_invalid".to_string())?;
        let safe_number = safe_block_number.ok_or_else(|| "safe_block_unavailable".to_string())?;
        if safe_number < public_activation_block {
            return Err("open_competition_activation_not_safe".to_string());
        }
        let catalog = open_competition_verifier_catalog_from_environment(network)
            .map_err(|_| "open_competition_verifier_catalog_unavailable".to_string())?;
        let [profile] = catalog.profiles.as_slice() else {
            return Err("open_competition_verifier_catalog_ambiguous".to_string());
        };
        if !profile.public_inventory_eligible
            || profile.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn
        {
            return Err("open_competition_verifier_unapproved".to_string());
        }
        let store = state
            .store
            .as_ref()
            .ok_or_else(|| "open_competition_store_unavailable".to_string())?;
        let heartbeat = store
            .get_base_indexer_heartbeat(network, &release.factory_contract)
            .await
            .map_err(|_| "open_competition_heartbeat_unavailable".to_string())?
            .ok_or_else(|| "open_competition_heartbeat_missing".to_string())?;
        if !open_competition_monitoring_is_fresh(&heartbeat, safe_number, generated_at) {
            return Err("open_competition_indexer_stale".to_string());
        }
        let events = store
            .list_open_competition_events(network, &release.factory_contract)
            .await
            .map_err(|_| "open_competition_read_model_unavailable".to_string())?;
        let projected = open_competition_discovery_items(
            &events,
            profile,
            network,
            descriptor.chain_id,
            &state.public_base_url,
            &website_base_url,
            public_activation_block,
            generated_at,
        )?;
        Ok::<_, String>((
            release.factory_contract,
            projected,
            heartbeat.persisted_cursor_block,
        ))
    }
    .await;
    match open_competition_result {
        Ok((factory, mut projected, cursor)) => {
            source_statuses.push(GitHubDiscoverySourceStatus {
                source_type: "open_competition".to_string(),
                protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
                factory_contract: Some(factory),
                available: true,
                fresh: true,
                item_count: projected.len(),
                persisted_cursor_block: cursor,
                error: None,
            });
            items.append(&mut projected);
        }
        Err(error) => source_statuses.push(GitHubDiscoverySourceStatus {
            source_type: "open_competition".to_string(),
            protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
            factory_contract: open_competition_release_from_environment(network)
                .ok()
                .map(|release| release.factory_contract),
            available: false,
            fresh: false,
            item_count: 0,
            persisted_cursor_block: None,
            error: Some(error),
        }),
    }

    assemble_github_discovery_projection(
        network,
        descriptor.chain_id,
        generated_at,
        safe_block,
        source_statuses,
        items,
    )
    .map(Json)
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

#[utoipa::path(
    get,
    path = "/v1/opportunities/stream",
    params(
        ("network" = Option<String>, Query, description = "Canonical network key; defaults to base-mainnet"),
        ("view" = Option<String>, Query, description = "Deterministic view; use ready_to_earn for earning inventory"),
        ("source_type" = Option<String>, Query, description = "Filter by canonical_base for earning inventory"),
        ("work_state" = Option<String>, Query, description = "Optional work-state filter"),
        ("payment_state" = Option<String>, Query, description = "Optional payment-state filter"),
        ("limit" = Option<u32>, Query, description = "Maximum results; clamped to 1..300")
    ),
    responses(
        (status = 200, description = "Server-sent inventory snapshots; inventory events contain OpportunityProjectionResponse"),
        (status = 400, description = "Unknown network, view, state, or source type")
    )
)]
async fn stream_opportunities(
    State(state): State<SharedState>,
    Query(query): Query<OpportunityQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    build_opportunity_projection(&state, query.clone()).await?;
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let stream_state = state.clone();
    let stream_query = query.clone();
    let stream = IntervalStream::new(interval).then(move |_| {
        let state = stream_state.clone();
        let query = stream_query.clone();
        async move {
            let event = match build_opportunity_projection(&state, query).await {
                Ok(projection) => Event::default()
                    .event("inventory")
                    .json_data(projection)
                    .unwrap_or_else(|_| {
                        Event::default()
                            .event("projection_error")
                            .data("inventory serialization failed")
                    }),
                Err(status) => Event::default()
                    .event("projection_error")
                    .data(format!("inventory unavailable: {}", status.as_u16())),
            };
            Ok::<Event, Infallible>(event)
        }
    });
    Ok((
        [
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, no-store, must-revalidate"),
            ),
            (
                HeaderName::from_static("x-accel-buffering"),
                HeaderValue::from_static("no"),
            ),
        ],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("canonical inventory stream"),
        ),
    ))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct OpportunityCommentResponse {
    id: Uuid,
    opportunity_id: String,
    author: String,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    feedback: Option<OpportunityFeedbackResponse>,
    created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
struct OpportunityFeedbackRequest {
    stage: String,
    #[serde(default)]
    discovery_source: Option<String>,
    #[serde(default)]
    participation_reason: Option<String>,
    #[serde(default)]
    friction: Option<String>,
    #[serde(default)]
    recommendation: Option<String>,
    #[serde(default)]
    evidence_reference: Option<String>,
    #[serde(default)]
    wallet: Option<String>,
    #[serde(default)]
    wallet_signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
struct OpportunityFeedbackResponse {
    stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    discovery_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    participation_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    friction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recommendation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence_reference: Option<String>,
    evidence_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    wallet_evidence_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct OpportunityCommentsResponse {
    schema_version: String,
    opportunity_id: String,
    comments: Vec<OpportunityCommentResponse>,
    comment_count: usize,
    evidence_boundary: String,
    share_after: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
struct CreateOpportunityCommentRequest {
    id: Uuid,
    author: String,
    body: String,
    #[serde(default)]
    feedback: Option<OpportunityFeedbackRequest>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpportunityCommentsQuery {
    limit: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/v1/opportunities/{opportunity_id}/comments",
    params(
        ("opportunity_id" = String, Path, description = "Public opportunity identifier"),
        ("limit" = Option<u32>, Query, description = "Maximum comments returned; clamped to 1..100")
    ),
    responses(
        (status = 200, body = OpportunityCommentsResponse),
        (status = 400, description = "Invalid opportunity identifier"),
        (status = 503, description = "Durable comment store unavailable")
    )
)]
async fn list_opportunity_comments(
    State(state): State<SharedState>,
    Path(opportunity_id): Path<String>,
    Query(query): Query<OpportunityCommentsQuery>,
) -> Result<Json<OpportunityCommentsResponse>, StatusCode> {
    let opportunity_id = normalize_opportunity_comment_id(&opportunity_id)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let comments = store
        .list_opportunity_comments(&opportunity_id, query.limit.unwrap_or(100))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(opportunity_comments_response(
        opportunity_id,
        comments,
    )))
}

#[utoipa::path(
    post,
    path = "/v1/opportunities/{opportunity_id}/comments",
    params(
        ("opportunity_id" = String, Path, description = "Public opportunity identifier")
    ),
    request_body = CreateOpportunityCommentRequest,
    responses(
        (status = 200, body = OpportunityCommentsResponse),
        (status = 400, description = "Invalid bounded comment payload"),
        (status = 409, description = "Comment UUID was reused for different content"),
        (status = 503, description = "Durable comment store unavailable")
    )
)]
async fn create_opportunity_comment(
    State(state): State<SharedState>,
    Path(opportunity_id): Path<String>,
    Json(request): Json<CreateOpportunityCommentRequest>,
) -> Result<Json<OpportunityCommentsResponse>, StatusCode> {
    let opportunity_id = normalize_opportunity_comment_id(&opportunity_id)?;
    let author = bounded_public_text(&request.author, 60)?;
    let body = bounded_public_text(&request.body, 500)?;
    let feedback = request
        .feedback
        .as_ref()
        .map(normalize_opportunity_feedback)
        .transpose()?;
    let feedback = feedback
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .create_or_get_opportunity_comment(&NewOpportunityComment {
            id: request.id,
            opportunity_id: opportunity_id.clone(),
            author,
            body,
            feedback,
        })
        .await
        .map_err(|error| match error {
            DbError::OpportunityCommentConflict => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    let comments = store
        .list_opportunity_comments(&opportunity_id, 100)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(opportunity_comments_response(
        opportunity_id,
        comments,
    )))
}

fn normalize_opportunity_comment_id(value: &str) -> Result<String, StatusCode> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > 200
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value.to_string())
}

fn optional_bounded_public_text(
    value: &Option<String>,
    max_chars: usize,
) -> Result<Option<String>, StatusCode> {
    value
        .as_deref()
        .map(|value| bounded_public_text(value, max_chars))
        .transpose()
}

fn normalize_opportunity_feedback(
    feedback: &OpportunityFeedbackRequest,
) -> Result<OpportunityFeedbackRequest, StatusCode> {
    const STAGES: &[&str] = &[
        "discovery",
        "posting",
        "funding",
        "activation",
        "participation",
        "wrong_mode",
        "quote",
        "payment_pending",
        "proof_submission",
        "settlement",
        "cancellation",
        "refund",
    ];
    let stage = feedback.stage.trim().to_ascii_lowercase();
    if !STAGES.contains(&stage.as_str()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let discovery_source = optional_bounded_public_text(&feedback.discovery_source, 120)?;
    let participation_reason = optional_bounded_public_text(&feedback.participation_reason, 500)?;
    let friction = optional_bounded_public_text(&feedback.friction, 500)?;
    let recommendation = optional_bounded_public_text(&feedback.recommendation, 500)?;
    let evidence_reference = optional_bounded_public_text(&feedback.evidence_reference, 500)?;
    if discovery_source.is_none()
        && participation_reason.is_none()
        && friction.is_none()
        && recommendation.is_none()
        && evidence_reference.is_none()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let wallet_pair = match (&feedback.wallet, &feedback.wallet_signature) {
        (None, None) => (None, None),
        (Some(wallet), Some(signature)) => {
            let wallet = normalize_evm_address(wallet)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .to_ascii_lowercase();
            let encoded = signature
                .strip_prefix("0x")
                .or_else(|| signature.strip_prefix("0X"))
                .ok_or(StatusCode::BAD_REQUEST)?;
            if encoded.len() != 130 || !encoded.bytes().all(|value| value.is_ascii_hexdigit()) {
                return Err(StatusCode::BAD_REQUEST);
            }
            (
                Some(wallet),
                Some(format!("0x{}", encoded.to_ascii_lowercase())),
            )
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    Ok(OpportunityFeedbackRequest {
        stage,
        discovery_source,
        participation_reason,
        friction,
        recommendation,
        evidence_reference,
        wallet: wallet_pair.0,
        wallet_signature: wallet_pair.1,
    })
}

fn public_opportunity_feedback(
    value: Option<serde_json::Value>,
) -> Option<OpportunityFeedbackResponse> {
    let feedback: OpportunityFeedbackRequest = serde_json::from_value(value?).ok()?;
    Some(OpportunityFeedbackResponse {
        stage: feedback.stage,
        discovery_source: feedback.discovery_source,
        participation_reason: feedback.participation_reason,
        friction: feedback.friction,
        recommendation: feedback.recommendation,
        evidence_reference: feedback.evidence_reference,
        evidence_status: "self_reported".to_string(),
        wallet_evidence_status: feedback
            .wallet
            .zip(feedback.wallet_signature)
            .map(|_| "supplied_unverified".to_string()),
    })
}

fn opportunity_comments_response(
    opportunity_id: String,
    comments: Vec<DbOpportunityComment>,
) -> OpportunityCommentsResponse {
    let mut comments = comments;
    comments.reverse();
    let comments = comments
        .into_iter()
        .map(|comment| OpportunityCommentResponse {
            id: comment.id,
            opportunity_id: comment.opportunity_id,
            author: comment.author,
            body: comment.body,
            feedback: public_opportunity_feedback(comment.feedback),
            created_at: comment.created_at.to_rfc3339(),
        })
        .collect::<Vec<_>>();
    OpportunityCommentsResponse {
        schema_version: "agent-bounties/opportunity-comments-v2".to_string(),
        opportunity_id,
        comment_count: comments.len(),
        comments,
        evidence_boundary: "Comments and structured feedback are self-reported public context. Wallet material is redacted and remains unverified unless separately correlated with canonical participation. Neither a wallet nor a browser identifier is a unique person, and feedback does not prove funding, claimability, verification, settlement, or payment.".to_string(),
        share_after: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum ChatgptActionKind {
    Post,
    Fund,
    #[serde(alias = "compete")]
    Solve,
    Complete,
    Verify,
}

impl ChatgptActionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Post => "post",
            Self::Fund => "fund",
            Self::Solve => "solve",
            Self::Complete => "complete",
            Self::Verify => "verify",
        }
    }

    fn expected_canonical_events(self) -> Vec<String> {
        match self {
            Self::Post => vec!["canonical_bounty_created".to_string()],
            Self::Fund => vec!["funding_added".to_string()],
            Self::Solve => vec!["bounty_claimed".to_string()],
            Self::Complete => vec!["submission_added".to_string()],
            Self::Verify => vec![
                "bounty_settled".to_string(),
                "submission_rejected".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct CreateChatgptActionIntentRequest {
    idempotency_key: String,
    action: ChatgptActionKind,
    network: Option<String>,
    opportunity_id: Option<String>,
    bounty_contract: Option<String>,
    bounty_id: Option<String>,
    actor_wallet: Option<String>,
    amount_base_units: Option<u64>,
    #[serde(default = "empty_json_object")]
    details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct ObserveChatgptActionTransactionRequest {
    transaction_hash: String,
    bounty_contract: Option<String>,
    bounty_id: Option<String>,
    actor_wallet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct ChatgptActionIntentResponse {
    schema_version: String,
    intent_id: Uuid,
    action: ChatgptActionKind,
    status: String,
    network: String,
    opportunity_id: Option<String>,
    bounty_contract: Option<String>,
    bounty_id: Option<String>,
    actor_wallet: Option<String>,
    amount_base_units: Option<u64>,
    details: serde_json::Value,
    authorization_url: String,
    expected_canonical_events: Vec<String>,
    transaction_hash: Option<String>,
    canonical_event_id: Option<Uuid>,
    canonical_event_kind: Option<String>,
    confirmed_block: Option<u64>,
    paid: bool,
    expires_at: String,
    share_after: bool,
    next_action: String,
    evidence_boundary: String,
}

fn empty_json_object() -> serde_json::Value {
    serde_json::json!({})
}

#[utoipa::path(
    post,
    path = "/v1/chatgpt/action-intents",
    request_body = CreateChatgptActionIntentRequest,
    responses(
        (status = 201, body = ChatgptActionIntentResponse),
        (status = 400, description = "Invalid or incomplete action request"),
        (status = 409, description = "Idempotency key already belongs to another action"),
        (status = 503, description = "Durable action coordination is unavailable")
    )
)]
async fn create_chatgpt_action_intent(
    State(state): State<SharedState>,
    Json(request): Json<CreateChatgptActionIntentRequest>,
) -> Result<(StatusCode, Json<ChatgptActionIntentResponse>), StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .delete_expired_chatgpt_action_intents_before(Utc::now() - ChronoDuration::hours(24))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let normalized = normalize_chatgpt_action_request(request)?;
    let fingerprint_value = serde_json::json!({
        "action": normalized.action,
        "network": normalized.network,
        "opportunity_id": normalized.opportunity_id,
        "bounty_contract": normalized.bounty_contract,
        "bounty_id": normalized.bounty_id,
        "actor_wallet": normalized.actor_wallet,
        "amount_base_units": normalized.amount_base_units,
        "details": normalized.details,
    });
    let request_fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&fingerprint_value).map_err(|_| StatusCode::BAD_REQUEST)?,
    ));
    let intent = store
        .reserve_chatgpt_action_intent(&NewChatgptActionIntent {
            id: Uuid::new_v4(),
            idempotency_key: normalized.idempotency_key,
            action: normalized.action.as_str().to_string(),
            network: normalized.network.ok_or(StatusCode::BAD_REQUEST)?,
            opportunity_id: normalized.opportunity_id,
            bounty_contract: normalized.bounty_contract,
            bounty_id: normalized.bounty_id,
            actor_wallet: normalized.actor_wallet,
            amount_base_units: normalized.amount_base_units,
            details: normalized.details,
            request_fingerprint,
            expires_at: Utc::now() + ChronoDuration::hours(1),
        })
        .await
        .map_err(map_chatgpt_action_db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(chatgpt_action_intent_response(&state, intent)?),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/chatgpt/action-intents/{intent_id}",
    params(("intent_id" = Uuid, Path, description = "Opaque hosted action intent identifier")),
    responses(
        (status = 200, body = ChatgptActionIntentResponse),
        (status = 404, description = "Unknown action intent"),
        (status = 503, description = "Canonical status is unavailable")
    )
)]
async fn get_chatgpt_action_intent(
    State(state): State<SharedState>,
    Path(intent_id): Path<Uuid>,
) -> Result<Json<ChatgptActionIntentResponse>, StatusCode> {
    let intent = reconcile_chatgpt_action_intent(&state, intent_id).await?;
    Ok(Json(chatgpt_action_intent_response(&state, intent)?))
}

#[utoipa::path(
    post,
    path = "/v1/chatgpt/action-intents/{intent_id}/observations",
    params(("intent_id" = Uuid, Path, description = "Opaque hosted action intent identifier")),
    request_body = ObserveChatgptActionTransactionRequest,
    responses(
        (status = 200, body = ChatgptActionIntentResponse),
        (status = 400, description = "Malformed transaction observation"),
        (status = 404, description = "Unknown or expired action intent"),
        (status = 409, description = "Observation conflicts with the prepared action"),
        (status = 503, description = "Canonical status is unavailable")
    )
)]
async fn observe_chatgpt_action_transaction(
    State(state): State<SharedState>,
    Path(intent_id): Path<Uuid>,
    Json(request): Json<ObserveChatgptActionTransactionRequest>,
) -> Result<Json<ChatgptActionIntentResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let observation = ChatgptActionObservation {
        transaction_hash: normalize_fixed_hex(&request.transaction_hash, 32)?,
        bounty_contract: request
            .bounty_contract
            .map(|value| normalize_evm_address(&value).map(|value| value.to_ascii_lowercase()))
            .transpose()
            .map_err(|_| StatusCode::BAD_REQUEST)?,
        bounty_id: request
            .bounty_id
            .map(|value| normalize_fixed_hex(&value, 32))
            .transpose()?,
        actor_wallet: request
            .actor_wallet
            .map(|value| normalize_evm_address(&value).map(|value| value.to_ascii_lowercase()))
            .transpose()
            .map_err(|_| StatusCode::BAD_REQUEST)?,
    };
    store
        .observe_chatgpt_action_transaction(intent_id, &observation)
        .await
        .map_err(map_chatgpt_action_db_error)?;
    let intent = reconcile_chatgpt_action_intent(&state, intent_id).await?;
    Ok(Json(chatgpt_action_intent_response(&state, intent)?))
}

fn normalize_chatgpt_action_request(
    mut request: CreateChatgptActionIntentRequest,
) -> Result<CreateChatgptActionIntentRequest, StatusCode> {
    request.idempotency_key = normalize_public_identifier(&request.idempotency_key, 8, 200)?;
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let descriptor = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    request.network = Some(
        canonical_base_network_key(descriptor.chain_id)
            .ok_or(StatusCode::BAD_REQUEST)?
            .to_string(),
    );
    request.opportunity_id = request
        .opportunity_id
        .map(|value| normalize_opportunity_comment_id(&value))
        .transpose()?;
    request.bounty_contract = request
        .bounty_contract
        .map(|value| normalize_evm_address(&value).map(|value| value.to_ascii_lowercase()))
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    request.bounty_id = request
        .bounty_id
        .map(|value| normalize_fixed_hex(&value, 32))
        .transpose()?;
    request.actor_wallet = request
        .actor_wallet
        .map(|value| normalize_evm_address(&value).map(|value| value.to_ascii_lowercase()))
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_chatgpt_action_details(request.action, &request.details)?;
    match request.action {
        ChatgptActionKind::Post => {}
        ChatgptActionKind::Fund => {
            if request.bounty_contract.is_none()
                || request.amount_base_units.is_none_or(|amount| amount == 0)
            {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        ChatgptActionKind::Solve | ChatgptActionKind::Complete | ChatgptActionKind::Verify => {
            if request.bounty_contract.is_none() || request.amount_base_units.is_some() {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }
    Ok(CreateChatgptActionIntentRequest {
        network: request.network,
        ..request
    })
}

fn validate_chatgpt_action_details(
    action: ChatgptActionKind,
    details: &serde_json::Value,
) -> Result<(), StatusCode> {
    let object = details.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    if serde_json::to_vec(details)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .len()
        > 16_000
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let allowed = match action {
        ChatgptActionKind::Post => &["draft"][..],
        ChatgptActionKind::Fund => &["title", "public_url"][..],
        ChatgptActionKind::Solve => &["title", "claim_bond_base_units", "verification_ready"][..],
        ChatgptActionKind::Complete => &["artifact_reference", "evidence"][..],
        ChatgptActionKind::Verify => &["title", "verification_method"][..],
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if action == ChatgptActionKind::Post {
        if let Some(draft) = object.get("draft") {
            let draft = draft.as_object().ok_or(StatusCode::BAD_REQUEST)?;
            let allowed_draft = [
                "title",
                "goal",
                "acceptance_criteria",
                "solver_reward_usdc",
                "verifier_reward_usdc",
                "source_url",
                "crowdfund",
                "discovery_source",
            ];
            if draft
                .keys()
                .any(|key| !allowed_draft.contains(&key.as_str()))
            {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }
    let mut key_count = 0usize;
    validate_chatgpt_detail_value(details, 0, &mut key_count)
}

fn validate_chatgpt_detail_value(
    value: &serde_json::Value,
    depth: usize,
    key_count: &mut usize,
) -> Result<(), StatusCode> {
    if depth > 6 {
        return Err(StatusCode::BAD_REQUEST);
    }
    match value {
        serde_json::Value::Object(object) => {
            *key_count = key_count
                .checked_add(object.len())
                .ok_or(StatusCode::BAD_REQUEST)?;
            if *key_count > 64 {
                return Err(StatusCode::BAD_REQUEST);
            }
            for (key, child) in object {
                if key.is_empty()
                    || key.len() > 80
                    || key.chars().any(char::is_control)
                    || chatgpt_detail_key_is_sensitive(key)
                {
                    return Err(StatusCode::BAD_REQUEST);
                }
                validate_chatgpt_detail_value(child, depth + 1, key_count)?;
            }
        }
        serde_json::Value::Array(items) => {
            if items.len() > 50 {
                return Err(StatusCode::BAD_REQUEST);
            }
            for item in items {
                validate_chatgpt_detail_value(item, depth + 1, key_count)?;
            }
        }
        serde_json::Value::String(text)
            if text.len() > 12_000 || text.chars().any(|character| character == '\0') =>
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        _ => {}
    }
    Ok(())
}

fn chatgpt_detail_key_is_sensitive(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    matches!(
        normalized.as_str(),
        "password"
            | "passphrase"
            | "secret"
            | "token"
            | "api_key"
            | "access_token"
            | "refresh_token"
            | "bearer_token"
            | "auth_token"
            | "private_key"
            | "seed"
            | "seed_phrase"
            | "mnemonic"
            | "otp"
            | "one_time_password"
            | "mfa"
            | "mfa_code"
            | "cvv"
            | "cvc"
            | "card_number"
            | "pan"
            | "payment_authorization"
            | "wallet_signature"
            | "verifier_signature"
    )
}

fn normalize_public_identifier(
    value: &str,
    minimum: usize,
    maximum: usize,
) -> Result<String, StatusCode> {
    let value = value.trim();
    if value.chars().count() < minimum
        || value.chars().count() > maximum
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value.to_string())
}

fn normalize_fixed_hex(value: &str, bytes: usize) -> Result<String, StatusCode> {
    let value = value.trim();
    if value.len() != 2 + bytes * 2
        || !value.starts_with("0x")
        || !value[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value.to_ascii_lowercase())
}

fn map_chatgpt_action_db_error(error: DbError) -> StatusCode {
    match error {
        DbError::ChatgptActionIntentConflict(_) => StatusCode::CONFLICT,
        DbError::ChatgptActionIntentUnavailable => StatusCode::NOT_FOUND,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn reconcile_chatgpt_action_intent(
    state: &SharedState,
    intent_id: Uuid,
) -> Result<DbChatgptActionIntent, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .delete_expired_chatgpt_action_intents_before(Utc::now() - ChronoDuration::hours(24))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut intent = store
        .get_chatgpt_action_intent(intent_id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if intent.status == "confirmed" {
        return Ok(intent);
    }
    if let Some(transaction_hash) = intent.transaction_hash.as_deref() {
        let events = store
            .list_autonomous_bounty_events_by_transaction(&intent.network, transaction_hash)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if let Some(event) = events
            .iter()
            .find(|event| canonical_event_matches_chatgpt_intent(&intent, event))
        {
            intent = store
                .confirm_chatgpt_action_intent(intent.id, event)
                .await
                .map_err(map_chatgpt_action_db_error)?;
            return Ok(intent);
        }
    }
    if intent.expires_at <= Utc::now() {
        if let Some(expired) = store
            .expire_chatgpt_action_intent(intent.id)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        {
            intent = expired;
        }
    }
    Ok(intent)
}

fn canonical_event_matches_chatgpt_intent(
    intent: &DbChatgptActionIntent,
    event: &AutonomousBountyEvent,
) -> bool {
    if intent
        .transaction_hash
        .as_deref()
        .is_none_or(|hash| !hash.eq_ignore_ascii_case(&event.tx_hash))
        || event.occurred_at < intent.created_at
    {
        return false;
    }
    let kind_matches = match intent.action.as_str() {
        "post" => event.kind == AutonomousBountyEventKind::CanonicalBountyCreated,
        "fund" => event.kind == AutonomousBountyEventKind::FundingAdded,
        "solve" | "compete" => event.kind == AutonomousBountyEventKind::BountyClaimed,
        "complete" => event.kind == AutonomousBountyEventKind::SubmissionAdded,
        "verify" => matches!(
            event.kind,
            AutonomousBountyEventKind::BountySettled
                | AutonomousBountyEventKind::SubmissionRejected
        ),
        _ => false,
    };
    if !kind_matches {
        return false;
    }
    if intent
        .bounty_id
        .as_deref()
        .is_some_and(|bounty_id| !bounty_id.eq_ignore_ascii_case(&event.bounty_id))
    {
        return false;
    }
    let event_bounty_contract = if intent.action == "post" {
        event.data["bounty_contract"].as_str()
    } else {
        Some(event.contract_address.as_str())
    };
    if intent.bounty_contract.as_deref().is_some_and(|contract| {
        event_bounty_contract
            .is_none_or(|event_contract| !contract.eq_ignore_ascii_case(event_contract))
    }) {
        return false;
    }
    if let Some(actor_wallet) = intent.actor_wallet.as_deref() {
        let actor_field = match intent.action.as_str() {
            "post" => Some("creator"),
            "fund" => Some("contributor"),
            "solve" | "compete" | "complete" => Some("solver"),
            "verify" => None,
            _ => return false,
        };
        if actor_field.is_some_and(|field| {
            event.data[field]
                .as_str()
                .is_none_or(|actor| !actor.eq_ignore_ascii_case(actor_wallet))
        }) {
            return false;
        }
    }
    if intent.action == "fund"
        && intent
            .amount_base_units
            .is_some_and(|amount| json_u128(&event.data["amount"]) != Some(u128::from(amount)))
    {
        return false;
    }
    true
}

fn chatgpt_action_intent_response(
    state: &SharedState,
    intent: DbChatgptActionIntent,
) -> Result<ChatgptActionIntentResponse, StatusCode> {
    let action = match intent.action.as_str() {
        "post" => ChatgptActionKind::Post,
        "fund" => ChatgptActionKind::Fund,
        "solve" | "compete" => ChatgptActionKind::Solve,
        "complete" => ChatgptActionKind::Complete,
        "verify" => ChatgptActionKind::Verify,
        _ => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let website_base_url =
        legal_website_base_url(env::var("WEBSITE_BASE_URL").ok(), &state.public_base_url);
    let next_action = match intent.status.as_str() {
        "review_required" => {
            "Open the authorization URL, review the exact action, and approve it in your wallet."
        }
        "pending_confirmation" => {
            "Wait for the matching canonical event; a transaction hash is not completion evidence."
        }
        "confirmed" => "Share the canonical result or continue to the next bounty step.",
        "expired" => "Prepare a new action intent before signing anything.",
        "failed" => "Review the hosted error and prepare a new action only if it is safe to retry.",
        _ => "Refresh canonical status before taking another action.",
    }
    .to_string();
    Ok(ChatgptActionIntentResponse {
        schema_version: "agent-bounties/chatgpt-action-intent-v1".to_string(),
        intent_id: intent.id,
        action,
        status: intent.status,
        network: intent.network,
        opportunity_id: intent.opportunity_id,
        bounty_contract: intent.bounty_contract,
        bounty_id: intent.bounty_id,
        actor_wallet: intent.actor_wallet,
        amount_base_units: intent.amount_base_units,
        details: intent.details,
        authorization_url: format!(
            "{}/authorize.html?intent={}",
            website_base_url.trim_end_matches('/'),
            intent.id
        ),
        expected_canonical_events: action.expected_canonical_events(),
        transaction_hash: intent.transaction_hash,
        canonical_event_id: intent.canonical_event_id,
        paid: intent.canonical_event_kind.as_deref() == Some("bounty_settled"),
        canonical_event_kind: intent.canonical_event_kind,
        confirmed_block: intent.confirmed_block,
        expires_at: intent.expires_at.to_rfc3339(),
        share_after: true,
        next_action,
        evidence_boundary: "This hosted intent coordinates one bounded wallet action. Preparing it, opening the authorization page, signing, broadcasting, or observing a transaction hash does not prove funding, claim, completion, verification, settlement, or payment. Confirmed status requires the exact indexed canonical event; only BountySettled proves solver payment.".to_string(),
    })
}

#[utoipa::path(
    get,
    path = "/v1/opportunities/feed.rss",
    responses(
        (status = 200, description = "Live RSS 2.0 representation of the unified opportunity projection", content_type = "application/rss+xml"),
        (status = 503, description = "Opportunity projection unavailable")
    )
)]
async fn opportunity_feed_rss(State(state): State<SharedState>) -> Result<Response, StatusCode> {
    opportunity_feed_response(&state, OpportunityFeedFormat::Rss).await
}

#[utoipa::path(
    get,
    path = "/v1/opportunities/feed.atom",
    responses(
        (status = 200, description = "Live Atom 1.0 representation of the unified opportunity projection", content_type = "application/atom+xml"),
        (status = 503, description = "Opportunity projection unavailable")
    )
)]
async fn opportunity_feed_atom(State(state): State<SharedState>) -> Result<Response, StatusCode> {
    opportunity_feed_response(&state, OpportunityFeedFormat::Atom).await
}

#[utoipa::path(
    get,
    path = "/v1/opportunities/feed.json",
    responses(
        (status = 200, description = "Live JSON Feed 1.1 representation of the unified opportunity projection", content_type = "application/feed+json"),
        (status = 503, description = "Opportunity projection unavailable")
    )
)]
async fn opportunity_feed_json(State(state): State<SharedState>) -> Result<Response, StatusCode> {
    opportunity_feed_response(&state, OpportunityFeedFormat::Json).await
}

#[derive(Debug, Clone, Copy)]
enum OpportunityFeedFormat {
    Rss,
    Atom,
    Json,
}

async fn opportunity_feed_response(
    state: &SharedState,
    format: OpportunityFeedFormat,
) -> Result<Response, StatusCode> {
    let projection = build_opportunity_projection(
        state,
        OpportunityQuery {
            view: Some("ready_to_earn".to_string()),
            source_type: Some("canonical_base".to_string()),
            limit: Some(300),
            ..OpportunityQuery::default()
        },
    )
    .await?;
    let feeds = render_opportunity_feeds(&projection, &state.public_base_url);
    let (content_type, body) = match format {
        OpportunityFeedFormat::Rss => ("application/rss+xml; charset=utf-8", feeds.rss),
        OpportunityFeedFormat::Atom => ("application/atom+xml; charset=utf-8", feeds.atom),
        OpportunityFeedFormat::Json => ("application/feed+json; charset=utf-8", feeds.json),
    };
    let etag = format!("\"{}\"", hex::encode(Sha256::digest(body.as_bytes())));
    let mut response = Response::new(body.into());
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=15, must-revalidate"),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    response.headers_mut().insert(
        header::LAST_MODIFIED,
        HeaderValue::from_str(&feeds.updated_at).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}

async fn build_opportunity_projection(
    state: &SharedState,
    query: OpportunityQuery,
) -> Result<OpportunityProjectionResponse, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let view =
        OpportunityView::parse(query.view.as_deref()).map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_opportunity_filter(
        query.source_type.as_deref(),
        &["unfunded_offchain", "legacy_bounty", "canonical_base"],
    )?;
    validate_opportunity_filter(
        query.work_state.as_deref(),
        &["open", "claimable", "in_progress", "submitted", "completed"],
    )?;
    validate_opportunity_filter(
        query.payment_state.as_deref(),
        &["none", "seeking_funding", "escrowed", "paid"],
    )?;

    let api = state.public_base_url.trim_end_matches('/');
    let now = Utc::now();
    let mut items = Vec::<OpportunityItem>::new();
    let mut source_statuses = Vec::<OpportunitySourceStatus>::new();

    let (unfunded_items, unfunded_error) = match state.store.as_ref() {
        Some(store) => match store.list_trial_bounties(100).await {
            Ok(trials) => {
                let mut projected = Vec::with_capacity(trials.len());
                let mut error = None;
                for trial in trials {
                    match store.list_unfunded_bounty_solutions(trial.id).await {
                        Ok(solutions) => {
                            projected.push(unfunded_opportunity(&trial, &solutions, api));
                        }
                        Err(_) => {
                            error = Some("unfunded_solution_store_unavailable".to_string());
                            projected.clear();
                            break;
                        }
                    }
                }
                (projected, error)
            }
            Err(_) => (
                Vec::new(),
                Some("unfunded_bounty_store_unavailable".to_string()),
            ),
        },
        None => (Vec::new(), Some("durable_store_not_configured".to_string())),
    };
    let unfunded_available = unfunded_error.is_none();
    source_statuses.push(OpportunitySourceStatus {
        source_type: "unfunded_offchain".to_string(),
        available: unfunded_available,
        authoritative_urls: vec![format!("{api}/v1/unfunded-bounties")],
        item_count: unfunded_items.len(),
        error: unfunded_error,
    });
    items.extend(unfunded_items);

    let legacy_statuses = {
        let network_state = state
            .network
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        network_state
            .bounties
            .values()
            .filter_map(|bounty| network_state.status(bounty.id).ok())
            .collect::<Vec<_>>()
    };
    let legacy_items = legacy_statuses
        .iter()
        .filter_map(|status| legacy_opportunity(status, api))
        .collect::<Vec<_>>();
    source_statuses.push(OpportunitySourceStatus {
        source_type: "legacy_bounty".to_string(),
        available: true,
        authoritative_urls: vec![
            format!("{api}/v1/bounties/feed"),
            format!("{api}/v1/bounties/funding-feed"),
        ],
        item_count: legacy_items.len(),
        error: None,
    });
    items.extend(legacy_items);

    let (mut canonical_items, autonomous_error) =
        match load_autonomous_bounty_feed(state, network, false).await {
            Ok(feed) => (
                feed.iter()
                    .filter_map(|item| canonical_opportunity(item, network, api))
                    .collect::<Vec<_>>(),
                None,
            ),
            Err(_) => (
                Vec::new(),
                Some("canonical_read_model_unavailable".to_string()),
            ),
        };
    let (open_competition_items, open_competition_error) =
        match load_public_open_competition_opportunities(state, network, api, now).await {
            Ok(items) => (items, None),
            Err(_) => (
                Vec::new(),
                Some("open_competition_read_model_unavailable".to_string()),
            ),
        };
    canonical_items.extend(open_competition_items);
    let (open_competition_v2_items, open_competition_v2_error) =
        match load_public_open_competition_v2_opportunities(state, network, api, now).await {
            Ok(items) => (items, None),
            Err(_) => (
                Vec::new(),
                Some("open_competition_v2_read_model_unavailable".to_string()),
            ),
        };
    canonical_items.extend(open_competition_v2_items);
    let canonical_errors = [
        autonomous_error,
        open_competition_error,
        open_competition_v2_error,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let canonical_error = (!canonical_errors.is_empty()).then(|| canonical_errors.join("+"));
    source_statuses.push(OpportunitySourceStatus {
        source_type: "canonical_base".to_string(),
        available: canonical_error.is_none(),
        authoritative_urls: vec![
            format!(
                "{api}/v1/base/autonomous-bounties/feed?network={network}&claimable_only=false"
            ),
            format!("{api}/v1/base/open-competition-v1/events?network={network}"),
            format!("{api}/v1/base/open-competition-v2-beta3/inventory?network={network}"),
        ],
        item_count: canonical_items.len(),
        error: canonical_error,
    });
    items.extend(canonical_items);

    let items = apply_opportunity_query(items, &query, view, now);
    Ok(OpportunityProjectionResponse {
        schema_version: OPPORTUNITY_PROJECTION_SCHEMA.to_string(),
        generated_at: now.to_rfc3339(),
        network: network.to_string(),
        applied_view: view.map(|view| view.as_str().to_string()),
        degraded: source_statuses.iter().any(|source| !source.available),
        source_statuses,
        items,
        evidence_boundary: "This endpoint is a read-only projection. Each listed source remains authoritative for its own records; the projection cannot create funding, claims, verification, settlement, or payment evidence. Only confirmed canonical BountySettled proves autonomous-v1 solver payment, and only confirmed canonical CompetitionSettledV2 proves Open Competition V2 solver payment.".to_string(),
    })
}

async fn load_public_open_competition_v2_opportunities(
    state: &SharedState,
    network: &str,
    api_base_url: &str,
    now: DateTime<Utc>,
) -> Result<Vec<OpportunityItem>, StatusCode> {
    let release = match open_competition_v2_api::release_from_environment(network) {
        Ok(release) => release,
        Err(_) => return Ok(Vec::new()),
    };
    if !release.public_creation_enabled {
        return Ok(Vec::new());
    }
    let agreement = open_competition_v2_api::current_indexer_agreement(state, network, &release)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut records = store
        .list_open_competition_v2_projections(network, &release.factory_contract)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    records.retain(|record| record.projection.last_block <= agreement.common_safe_block);
    let mut events = store
        .list_open_competition_v2_events(network, &release.factory_contract)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    events.retain(|event| event.block_number <= agreement.common_safe_block);
    let proof_fee_name = |proof_system: &str| {
        format!(
            "OPEN_COMPETITION_V2_{}_PROOF_FEE_BASE_UNITS",
            proof_system.to_ascii_uppercase()
        )
    };
    let proof_fees = records
        .iter()
        .filter_map(|record| record.projection.proof_system.as_deref())
        .map(proof_fee_name)
        .map(|name| {
            env::var(name)
                .ok()
                .and_then(|value| value.parse::<u128>().ok())
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let proof_fee = proof_fees.into_iter().max().unwrap_or_default();
    let relay_fee = env::var("OPEN_COMPETITION_V2_RELAY_FEE_BASE_UNITS")
        .ok()
        .and_then(|value| value.parse::<u128>().ok())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    open_competition_v2_opportunities(
        &records,
        &events,
        &release,
        network,
        api_base_url,
        &legal_website_base_url(env::var("WEBSITE_BASE_URL").ok(), &state.public_base_url),
        opportunities::OpenCompetitionV2HostedCosts {
            proof_fee,
            relay_fee,
        },
        now,
    )
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

async fn load_public_open_competition_opportunities(
    state: &SharedState,
    network: &str,
    api_base_url: &str,
    now: DateTime<Utc>,
) -> Result<Vec<OpportunityItem>, StatusCode> {
    let release = match open_competition_release_from_environment(network) {
        Ok(release) => release,
        Err(_) => return Ok(Vec::new()),
    };
    if release.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn {
        return Ok(Vec::new());
    }
    let prefix = open_competition_environment_prefix(network)?;
    let public_activation_block = env::var(format!("{prefix}_PUBLIC_ACTIVATION_BLOCK"))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .parse::<u64>()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if public_activation_block < release.deployment_block {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    for gate in [
        "CREATION_ENABLED",
        "COMMITMENTS_ENABLED",
        "GAS_SPONSORSHIP_AVAILABLE",
        "RELAY_SUPPORT_AVAILABLE",
        "R4_EVIDENCE_COMPLETE",
        "MONITORING_ACTIVE",
    ] {
        if !env_flag(&format!("{prefix}_{gate}")) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    }
    if state.store.is_none()
        || !state.x402_relayer.enabled
        || state.x402_relayer.relayer.is_none()
        || open_competition_entrant_release_from_environment(network).is_err()
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let catalog = open_competition_verifier_catalog_from_environment(network)?;
    let [profile] = catalog.profiles.as_slice() else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !profile.public_inventory_eligible
        || profile.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let safe_block = tokio::time::timeout(
        Duration::from_secs(12),
        fetch_safe_block_identity(&rpc_url, 92),
    )
    .await
    .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if safe_block.number < public_activation_block {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let heartbeat = store
        .get_base_indexer_heartbeat(network, &release.factory_contract)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !open_competition_monitoring_is_fresh(&heartbeat, safe_block.number, now) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let events: Vec<OpenCompetitionEvent> = store
        .list_open_competition_events(network, &release.factory_contract)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let website_base_url =
        legal_website_base_url(env::var("WEBSITE_BASE_URL").ok(), &state.public_base_url);
    open_competition_opportunities(
        &events,
        profile,
        network,
        api_base_url,
        &website_base_url,
        public_activation_block,
        now,
    )
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

#[utoipa::path(
    get,
    path = "/v1/opportunities/conversion-funnel",
    params(("window_hours" = Option<u32>, Query, description = "Cohort lookback from 1 to 8760 hours; defaults to 720")),
    responses(
        (status = 200, body = OpportunityConversionFunnelResponse),
        (status = 400, description = "Invalid window"),
        (status = 503, description = "Durable analytics store unavailable")
    )
)]
async fn opportunity_conversion_funnel(
    State(state): State<SharedState>,
    Query(query): Query<OpportunityConversionQuery>,
) -> Result<Json<OpportunityConversionFunnelResponse>, StatusCode> {
    let window_hours = query.window_hours.unwrap_or(720);
    if !(1..=8_760).contains(&window_hours) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let generated_at = Utc::now();
    let window_started_at = generated_at - ChronoDuration::hours(i64::from(window_hours));
    let stats = store
        .opportunity_lifecycle_stats(window_started_at, &state.recovery_reservations.contracts())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(opportunity_conversion_response(
        stats,
        window_hours,
        window_started_at,
        generated_at,
    )))
}

fn opportunity_conversion_response(
    stats: OpportunityLifecycleStats,
    window_hours: u32,
    window_started_at: chrono::DateTime<Utc>,
    generated_at: chrono::DateTime<Utc>,
) -> OpportunityConversionFunnelResponse {
    let stage = |name: &str, count: u64, source: &str, note: &str| OpportunityConversionStage {
        stage: name.to_string(),
        count,
        evidence_source: source.to_string(),
        coverage_note: note.to_string(),
    };
    let rate =
        |metric: &str, numerator: u64, denominator: u64, cohort: &str| OpportunityConversionRate {
            metric: metric.to_string(),
            numerator,
            denominator,
            value: (denominator > 0).then(|| numerator as f64 / denominator as f64),
            cohort: cohort.to_string(),
        };
    OpportunityConversionFunnelResponse {
        schema_version: "agent-bounties/opportunity-conversion-funnel-v1".to_string(),
        window_hours,
        window_started_at: window_started_at.to_rfc3339(),
        generated_at: generated_at.to_rfc3339(),
        stages: vec![
            stage(
                "unfunded_published",
                stats.published,
                "trial_bounties.created_at",
                "Public off-chain publications in the selected cohort.",
            ),
            stage(
                "solution_received",
                stats.solution_received,
                "unfunded_bounty_solutions.created_at",
                "Distinct cohort publications with at least one registered-agent solution; agent identity is self-reported registration, not independence proof.",
            ),
            stage(
                "funding_prepared",
                stats.funding_prepared,
                "opportunity_creation_progress.funding_prepared_at",
                "A valid hosted creation plan was returned for immutable terms linked by source URL to the unfunded publication. A plan is not funding.",
            ),
            stage(
                "wallet_signed",
                stats.wallet_signed_observed,
                "opportunity_creation_progress.wallet_signed_at",
                "Observed only when a valid EIP-3009 signature is supplied to the authorized creation-plan endpoint. Direct wallet or wallet_sendCalls signatures remain client-side and are not counted.",
            ),
            stage(
                "canonical_created",
                stats.canonical_created,
                "confirmed CanonicalBountyCreated joined by immutable terms_hash",
                "Distinct unfunded cohort publications with confirmed canonical creation.",
            ),
            stage(
                "funded",
                stats.funded,
                "confirmed BountyBecameClaimable",
                "Funding is counted only when the canonical contract became fully funded and claimable.",
            ),
            stage(
                "claimed",
                stats.claimed,
                "confirmed BountyClaimed",
                "At least one confirmed canonical claim for the correlated bounty.",
            ),
            stage(
                "submitted",
                stats.submitted,
                "confirmed SubmissionAdded",
                "At least one confirmed canonical submission for the correlated bounty.",
            ),
            stage(
                "settled",
                stats.settled,
                "confirmed BountySettled",
                "At least one confirmed canonical settlement; this is the only stage that proves solver payment.",
            ),
        ],
        rates: vec![
            rate(
                "time_bounded_solution_rate",
                stats.solution_received,
                stats.published,
                "unfunded publications created within the selected window",
            ),
            rate(
                "unfunded_to_funded_conversion",
                stats.funded,
                stats.published,
                "unfunded publications created within the selected window and correlated by immutable terms hash",
            ),
            rate(
                "claim_rate_after_funding",
                stats.claimed,
                stats.funded,
                "correlated unfunded cohort that reached confirmed BountyBecameClaimable",
            ),
            rate(
                "completion_rate_after_claim",
                stats.settled,
                stats.claimed,
                "correlated unfunded cohort that reached confirmed BountyClaimed",
            ),
            rate(
                "canonical_created_to_settled",
                stats.settled,
                stats.canonical_created,
                "correlated unfunded cohort with confirmed CanonicalBountyCreated",
            ),
        ],
        average_seconds_to_first_solution: stats.average_seconds_to_first_solution,
        median_seconds_to_first_solution: stats.median_seconds_to_first_solution,
        average_seconds_creation_to_settlement: stats
            .average_seconds_creation_to_settlement,
        actors: OpportunityActorMetrics {
            unique_canonical_poster_wallets: stats.unique_canonical_poster_wallets,
            repeat_canonical_poster_wallets: stats.repeat_canonical_poster_wallets,
            unique_paid_solver_wallets: stats.unique_paid_solver_wallets,
            repeat_paid_solver_wallets: stats.repeat_paid_solver_wallets,
            independent_active_agents: None,
            independence_measurement_available: false,
            evidence_boundary: "Poster and paid-solver counts use confirmed canonical wallet addresses. A wallet is not proof of a distinct human or independent agent, and activity outside canonical events is not inferred. Therefore independent_active_agents is intentionally null."
                .to_string(),
        },
        evidence_boundary: format!(
            "The first nine stages are a cohort funnel rooted in unfunded publications. Canonical event counts outside that linked cohort are used only for settlement timing and wallet-repeat metrics: {} canonical creations, {} claims, and {} settlements occurred in the selected event window. Plans, signatures, transaction hashes, AI outputs, and webhook notifications are not settlement evidence.",
            stats.canonical_created_in_window,
            stats.canonical_claimed_in_window,
            stats.canonical_settled_in_window,
        ),
    }
}

fn site_analytics_origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    if matches!(
        origin,
        "https://agentbounties.app"
            | "https://www.agentbounties.app"
            | "https://bountyboard.global"
            | "https://www.bountyboard.global"
    ) {
        return true;
    }
    for prefix in ["http://localhost:", "http://127.0.0.1:"] {
        if let Some(port) = origin.strip_prefix(prefix) {
            return !port.is_empty() && port.chars().all(|character| character.is_ascii_digit());
        }
    }
    false
}

fn marketing_domain_destination(host: &str, uri: &Uri) -> Option<String> {
    let normalized = host
        .split_once(':')
        .map_or(host, |(hostname, _)| hostname)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let domain = normalized.strip_prefix("www.").unwrap_or(&normalized);
    let (base, home) = match domain {
        "status.agentbounties.app" => ("https://api.agentbounties.app", "/health"),
        "bountyboard.global" => ("https://agentbounties.app", "/"),
        "agentbounties.io" => ("https://agentbounties.app", "/developers/"),
        "agentbounties.dev" => ("https://agentbounties.app", "/docs/"),
        "agentbounties.work" => ("https://agentbounties.app", "/tasks/"),
        "agentbounties.global" => ("https://agentbounties.app", "/global/"),
        "agentbounties.network" => ("https://agentbounties.app", "/agents/"),
        "agentbounties.bid" => ("https://agentbounties.app", "/post-a-task/"),
        "agentbounties.org" => ("https://agentbounties.app", "/community/"),
        "agentbounties.co" | "agentbounties.net" | "agentbounties.xyz" => {
            ("https://agentbounties.app", "/")
        }
        _ => return None,
    };
    let retired_path = matches!(
        uri.path(),
        "/earn.html"
            | "/post.html"
            | "/objective.html"
            | "/create-competition.html"
            | "/refunds.html"
            | "/x402.html"
            | "/prepare-agent.html"
            | "/agent-budget.html"
            | "/funding.html"
    );
    let target_path = if retired_path {
        "/"
    } else if uri.path() == "/" {
        home
    } else {
        uri.path()
    };
    let query = if retired_path {
        String::new()
    } else {
        uri.query()
            .map_or(String::new(), |value| format!("?{value}"))
    };
    Some(format!("{base}{target_path}{query}"))
}

async fn redirect_marketing_domain(request: Request, next: Next) -> Response {
    let destination = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|host| marketing_domain_destination(host, request.uri()));
    match destination {
        Some(destination) => Redirect::permanent(&destination).into_response(),
        None => next.run(request).await,
    }
}

fn normalize_site_analytics_token(value: Option<String>) -> Result<Option<String>, StatusCode> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 64
        || !value.chars().enumerate().all(|(index, character)| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || (index > 0 && matches!(character, '.' | '_' | '-'))
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(Some(value))
}

fn normalize_site_analytics_source(value: Option<String>) -> Result<Option<String>, StatusCode> {
    let Some(source) = normalize_site_analytics_token(value)? else {
        return Ok(None);
    };
    let host = source.strip_prefix("www.").unwrap_or(&source);
    let normalized = if matches!(
        host,
        "chatgpt" | "openai" | "chat.openai.com" | "chatgpt.com"
    ) || host.ends_with(".chatgpt.com")
        || host.ends_with(".openai.com")
    {
        "chatgpt"
    } else if matches!(host, "google" | "google.com") || host.starts_with("google.") {
        "google"
    } else if matches!(host, "bing" | "bing.com") || host.ends_with(".bing.com") {
        "bing"
    } else if matches!(host, "github" | "github.com") || host.ends_with(".github.com") {
        "github"
    } else if matches!(
        host,
        "medium"
            | "medium.com"
            | "substack"
            | "substack.com"
            | "devto"
            | "dev.to"
            | "hackernews"
            | "hn"
            | "news.ycombinator.com"
            | "reddit"
            | "reddit.com"
            | "x"
            | "x.com"
            | "twitter"
            | "twitter.com"
            | "t.co"
    ) || host.ends_with(".substack.com")
        || host.ends_with(".reddit.com")
    {
        "syndicated"
    } else {
        return Ok(Some(source));
    };
    Ok(Some(normalized.to_string()))
}

fn normalize_site_analytics_referrer(value: Option<String>) -> Result<Option<String>, StatusCode> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 253
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '-')
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(Some(value))
}

fn validated_site_analytics_event(
    request: SiteAnalyticsEventRequest,
    now: DateTime<Utc>,
) -> Result<NewSiteAnalyticsEvent, StatusCode> {
    if !matches!(
        request.event_name.as_str(),
        "page_view"
            | "market_view"
            | "funded_bounty_click"
            | "opportunity_feed_click"
            | "unfunded_post_started"
            | "unfunded_post_completed"
            | "funding_started"
            | "claim_started"
            | "claim_confirmed"
            | "competition_entry_started"
            | "competition_entry_confirmed"
            | "competition_reveal_started"
            | "competition_reveal_confirmed"
            | "competition_view"
            | "competition_instructions_copied"
            | "competition_template_copied"
            | "competition_child_post_started"
            | "competition_feedback_started"
            | "competition_feedback_submitted"
            | "canonical_post_started"
            | "canonical_post_confirmed"
            | "auth_completed"
            | "wallet_link_started"
            | "wallet_link_confirmed"
            | "wallet_missing_detected"
            | "wallet_connected"
            | "wallet_unfunded_detected"
            | "wallet_funded_observed"
            | "canonical_post_handoff_viewed"
            | "onramp_viewed"
            | "onramp_moonpay_started"
            | "onramp_metamask_started"
            | "onramp_coinbase_started"
            | "onramp_returned"
    ) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if request.page_path.is_empty()
        || request.page_path.len() > 160
        || !request.page_path.starts_with('/')
        || request.page_path.contains(['?', '#'])
        || request.page_path.chars().any(char::is_control)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if request.occurred_at < now - ChronoDuration::days(7)
        || request.occurred_at > now + ChronoDuration::minutes(5)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let opportunity_id = request
        .opportunity_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if opportunity_id.as_ref().is_some_and(|value| {
        value.len() > 200
            || !value.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, ':' | '/' | '.' | '_' | '-')
            })
    }) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let bounty_contract = request
        .bounty_contract
        .map(|value| normalize_evm_address(&value).map(|value| value.to_ascii_lowercase()))
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(NewSiteAnalyticsEvent {
        event_id: request.event_id,
        visitor_id: request.visitor_id,
        session_id: request.session_id,
        event_name: request.event_name,
        page_path: request.page_path,
        source: normalize_site_analytics_source(request.source)?,
        campaign: normalize_site_analytics_token(request.campaign)?,
        referrer_host: normalize_site_analytics_referrer(request.referrer_host)?,
        opportunity_id,
        bounty_contract,
        occurred_at: request.occurred_at,
    })
}

#[utoipa::path(
    post,
    path = "/v1/analytics/events",
    request_body = SiteAnalyticsEventRequest,
    responses(
        (status = 200, body = SiteAnalyticsReceipt),
        (status = 400, description = "Invalid privacy-minimized event"),
        (status = 403, description = "Origin is not the first-party site"),
        (status = 503, description = "Durable analytics store unavailable")
    )
)]
async fn record_site_analytics_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<SiteAnalyticsEventRequest>,
) -> Result<Json<SiteAnalyticsReceipt>, StatusCode> {
    if !site_analytics_origin_allowed(&headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    let event = validated_site_analytics_event(request, Utc::now())?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let inserted = store
        .record_site_analytics_event(&event)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(SiteAnalyticsReceipt {
        schema_version: "agent-bounties/site-analytics-receipt-v1".to_string(),
        accepted: true,
        duplicate: !inserted,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/analytics/site",
    params(("window_hours" = Option<u32>, Query, description = "Lookback from 1 to 8760 hours; defaults to 720")),
    responses(
        (status = 200, body = SiteAnalyticsResponse),
        (status = 400, description = "Invalid window"),
        (status = 503, description = "Durable analytics store unavailable")
    )
)]
async fn site_analytics(
    State(state): State<SharedState>,
    Query(query): Query<SiteAnalyticsQuery>,
) -> Result<Json<SiteAnalyticsResponse>, StatusCode> {
    let window_hours = query.window_hours.unwrap_or(720);
    if !(1..=8_760).contains(&window_hours) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let generated_at = Utc::now();
    let window_started_at = generated_at - ChronoDuration::hours(i64::from(window_hours));
    let stats = store
        .site_analytics_stats(window_started_at)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(site_analytics_response(
        stats,
        window_hours,
        window_started_at,
        generated_at,
    )))
}

fn site_analytics_response(
    stats: SiteAnalyticsStats,
    window_hours: u32,
    window_started_at: DateTime<Utc>,
    generated_at: DateTime<Utc>,
) -> SiteAnalyticsResponse {
    let cohort = |metric: &str| match metric {
        "market_to_funded_bounty_click" | "market_to_funding_start" => {
            "sessions that loaded live market inventory"
        }
        "canonical_post_completion" => "sessions that began the wallet-backed canonical post flow",
        "claim_confirmation" => "sessions that began a claim flow",
        "auth_to_post_handoff" => "sessions that completed first-party authentication",
        "wallet_link_completion" => "sessions that began linking a wallet to an account",
        "no_wallet_to_connected" => "sessions where the posting flow detected no wallet",
        "unfunded_wallet_to_funded" => {
            "sessions where the posting flow observed an unfunded wallet"
        }
        "onramp_return"
        | "onramp_moonpay_start"
        | "onramp_metamask_start"
        | "onramp_coinbase_start" => "sessions that viewed the first-party on-ramp",
        "funded_click_to_competition_view" => {
            "sessions that opened a funded competition from the unified market"
        }
        "competition_instruction_engagement" | "competition_child_post_start" => {
            "sessions that loaded a contract-specific competition workspace"
        }
        "competition_feedback_completion" => {
            "sessions that began the public competition feedback form"
        }
        "competition_entry_confirmation" => "sessions that began an Open Competition entry flow",
        _ => "sessions in the named funnel",
    };
    let rates = stats
        .funnel_counts
        .into_iter()
        .map(|count| SiteAnalyticsRateResponse {
            cohort: cohort(&count.metric).to_string(),
            value: (count.denominator_sessions > 0)
                .then(|| count.numerator_sessions as f64 / count.denominator_sessions as f64),
            metric: count.metric,
            numerator_sessions: count.numerator_sessions,
            denominator_sessions: count.denominator_sessions,
        })
        .collect();
    SiteAnalyticsResponse {
        schema_version: "agent-bounties/site-analytics-v2".to_string(),
        window_hours,
        window_started_at: window_started_at.to_rfc3339(),
        generated_at: generated_at.to_rfc3339(),
        overview: SiteAnalyticsOverviewResponse {
            unique_visitors: stats.overview.unique_visitors,
            returning_visitors: stats.overview.returning_visitors,
            sessions: stats.overview.sessions,
            page_views: stats.overview.page_views,
            first_event_at: stats.overview.first_event_at.map(|value| value.to_rfc3339()),
            last_event_at: stats.overview.last_event_at.map(|value| value.to_rfc3339()),
        },
        event_counts: stats
            .event_counts
            .into_iter()
            .map(|count| SiteAnalyticsEventCountResponse {
                event_name: count.event_name,
                events: count.events,
                sessions: count.sessions,
                visitors: count.visitors,
            })
            .collect(),
        daily: stats
            .daily
            .into_iter()
            .map(|day| SiteAnalyticsDailyResponse {
                day: day.day,
                visitors: day.visitors,
                sessions: day.sessions,
                page_views: day.page_views,
                market_views: day.market_views,
                funded_bounty_clicks: day.funded_bounty_clicks,
                canonical_posts_confirmed: day.canonical_posts_confirmed,
                funding_starts: day.funding_starts,
                claims_confirmed: day.claims_confirmed,
            })
            .collect(),
        channels: stats
            .channels
            .into_iter()
            .map(|channel| SiteAnalyticsChannelResponse {
                source: channel.source,
                campaign: channel.campaign,
                visitors: channel.visitors,
                sessions: channel.sessions,
                page_views: channel.page_views,
                funded_bounty_clicks: channel.funded_bounty_clicks,
                canonical_posts_confirmed: channel.canonical_posts_confirmed,
                funding_starts: channel.funding_starts,
                claims_confirmed: channel.claims_confirmed,
            })
            .collect(),
        interfaces: stats
            .interfaces
            .into_iter()
            .map(|usage| InterfaceUsageResponse {
                interface: usage.interface,
                protocol_era: usage.protocol_era,
                request_count: usage.request_count,
                successful_request_count: usage.successful_request_count,
                first_observed_at: usage.first_observed_at.to_rfc3339(),
                last_observed_at: usage.last_observed_at.to_rfc3339(),
            })
            .collect(),
        rates,
        definitions: vec![
            "A visitor is one random browser-local UUID with a 90-day lifetime, not a person or wallet.".to_string(),
            "A returning visitor is the same browser-local UUID observed on at least two UTC dates in the selected window.".to_string(),
            "A session is one random sessionStorage UUID and ends with that browser tab session.".to_string(),
            "Channel attribution uses the visitor's earliest recorded privacy-safe source and campaign; only the referrer hostname is retained.".to_string(),
            "External interface usage is an hourly aggregate of observed requests, not unique people, agents, clients, or sessions; partial boundary hours are included.".to_string(),
            "Requests bearing a server-verified analytics exclusion or operator credential are omitted before aggregation; no operator identifier is stored.".to_string(),
            "API and CLI attribution is self-declared through x-agent-bounties-interface; MCP protocol era is observed by the MCP service.".to_string(),
            "Funnel numerators count only sessions that recorded the denominator first and the numerator later in the selected window.".to_string(),
        ],
        evidence_boundary: "Collection begins only after each feature is deployed and has no historical backfill. External interface counting restarts at the operator-exclusion release; the earlier launch aggregate is retained outside this public response because it contains maintainer validation traffic that cannot be separated retrospectively. Cleared storage, private browsing, multiple devices, disabled analytics, Global Privacy Control, and Do Not Track affect browser coverage. Interface counters cannot deduplicate users or prove preference, and self-declared API or CLI attribution can be absent or spoofed. No IP address, user agent, full referrer URL, wallet, client identifier, request body, prompt, tool arguments, or operator identity is stored. Client conversion events describe observed interface actions; canonical lifecycle and payment claims remain authoritative only in confirmed canonical events, and only BountySettled proves solver payment.".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformMetricWindow {
    period: String,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    previous_started_at: DateTime<Utc>,
    launch_at: DateTime<Utc>,
    first_month_ended_at: DateTime<Utc>,
}

fn parse_public_metrics_timestamp(value: &str) -> Result<DateTime<Utc>, StatusCode> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn platform_metric_window(
    period: Option<&str>,
    ended_at: DateTime<Utc>,
) -> Result<PlatformMetricWindow, StatusCode> {
    let launch_at = parse_public_metrics_timestamp(PLATFORM_LAUNCH_AT)?;
    let first_month_ended_at = parse_public_metrics_timestamp(PLATFORM_FIRST_MONTH_ENDED_AT)?;
    let period = period.unwrap_or("7d");
    let requested_started_at = match period {
        "7d" => ended_at - ChronoDuration::days(7),
        "28d" => ended_at - ChronoDuration::days(28),
        "90d" => ended_at - ChronoDuration::days(90),
        "lifetime" => launch_at,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let started_at = requested_started_at.max(launch_at);
    let selected_duration = ended_at.signed_duration_since(started_at);
    let previous_started_at = if period == "lifetime" {
        launch_at
    } else {
        started_at - selected_duration
    };
    Ok(PlatformMetricWindow {
        period: period.to_string(),
        started_at,
        ended_at,
        previous_started_at,
        launch_at,
        first_month_ended_at,
    })
}

fn public_metrics_policy() -> Result<PublicMetricsPolicy, StatusCode> {
    let mut policy: PublicMetricsPolicy = serde_json::from_str(PUBLIC_METRICS_POLICY_JSON)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    policy.maintainer_github_logins = policy
        .maintainer_github_logins
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    policy.maintainer_comment_authors = policy
        .maintainer_comment_authors
        .into_iter()
        .map(|value| {
            value
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty())
        .collect();
    policy.maintainer_wallets = policy
        .maintainer_wallets
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    policy.excluded_bounty_contracts = policy
        .excluded_bounty_contracts
        .into_iter()
        .map(|value| normalize_evm_address(&value).map(|value| value.to_ascii_lowercase()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(policy)
}

fn historical_platform_metric_exclusions(policy: &PublicMetricsPolicy) -> Vec<String> {
    policy.excluded_bounty_contracts.clone()
}

fn exact_usdc(amount: u128) -> String {
    format!("{}.{:06}", amount / 1_000_000, amount % 1_000_000)
}

fn platform_amount(base_units: impl Into<String>) -> Result<PlatformAmountResponse, StatusCode> {
    let usdc_base_units = base_units.into();
    let amount = usdc_base_units
        .parse::<u128>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(PlatformAmountResponse {
        usdc_base_units,
        usdc: exact_usdc(amount),
    })
}

fn platform_inventory_response(
    autonomous: Option<AutonomousBountyInventorySummary>,
    competition: Option<OpenCompetitionInventorySummary>,
    competition_v2: Option<OpenCompetitionInventorySummary>,
) -> Result<PlatformInventoryResponse, StatusCode> {
    match (autonomous, competition, competition_v2) {
        (Some(autonomous), Some(competition), Some(competition_v2)) => {
            let competition_count = competition
                .ready_to_earn_count
                .checked_add(competition_v2.ready_to_earn_count)
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
            let competition_funding = add_platform_base_units(
                &competition.funded_usdc_base_units,
                &competition_v2.funded_usdc_base_units,
            )?;
            let competition_solver_rewards = add_platform_base_units(
                &competition.solver_reward_usdc_base_units,
                &competition_v2.solver_reward_usdc_base_units,
            )?;
            let competition_verifier_rewards = add_platform_base_units(
                &competition.verifier_reward_usdc_base_units,
                &competition_v2.verifier_reward_usdc_base_units,
            )?;
            Ok(PlatformInventoryResponse {
            status: "ready".to_string(),
            active_funded_opportunities: Some(
                autonomous
                    .claimable_bounty_count
                    .checked_add(competition_count)
                    .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?,
            ),
            available_funding_usdc: Some(exact_usdc(
                add_platform_base_units(
                    &autonomous.funded_usdc_base_units,
                    &competition_funding,
                )?
                .parse::<u128>()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            )),
            available_solver_rewards_usdc: Some(exact_usdc(
                add_platform_base_units(
                    &autonomous.solver_reward_usdc_base_units,
                    &competition_solver_rewards,
                )?
                .parse::<u128>()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            )),
            available_verifier_rewards_usdc: Some(exact_usdc(
                add_platform_base_units(
                    &autonomous.verifier_reward_usdc_base_units,
                    &competition_verifier_rewards,
                )?
                .parse::<u128>()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            )),
            generated_at: [
                autonomous.generated_at,
                competition.generated_at,
                competition_v2.generated_at,
            ]
            .into_iter()
            .min(),
            definition: "Current funded opportunities are reported as one marketplace total. The point-in-time projection is not added to historical payout or conversion cohorts.".to_string(),
        })
        }
        (autonomous, competition, competition_v2) => Ok(PlatformInventoryResponse {
            status: if autonomous.is_some() || competition.is_some() || competition_v2.is_some() {
                "partial"
            } else {
                "unavailable"
            }
            .to_string(),
            active_funded_opportunities: None,
            available_funding_usdc: None,
            available_solver_rewards_usdc: None,
            available_verifier_rewards_usdc: None,
            generated_at: None,
            definition: "One or more canonical inventory sources could not be read. Historical metrics remain available, and unified inventory is not silently replaced with a lower total or zero.".to_string(),
        }),
    }
}

fn add_platform_base_units(left: &str, right: &str) -> Result<String, StatusCode> {
    let left = left
        .parse::<u128>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let right = right
        .parse::<u128>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    left.checked_add(right)
        .map(|value| value.to_string())
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn platform_metrics_response(
    stats: PlatformMetricsStats,
    growth: PlatformDemandGrowthStats,
    window: PlatformMetricWindow,
    policy: PublicMetricsPolicy,
    autonomous_inventory: Option<AutonomousBountyInventorySummary>,
    competition_inventory: Option<OpenCompetitionInventorySummary>,
    competition_v2_inventory: Option<OpenCompetitionInventorySummary>,
    source_freshness: PlatformCanonicalSourceFreshness,
) -> Result<PlatformMetricsResponse, StatusCode> {
    let settlement_rate = (stats.claim_cohort.mature > 0)
        .then(|| stats.claim_cohort.settled as f64 / stats.claim_cohort.mature as f64);
    let inventory_complete = autonomous_inventory.is_some()
        && competition_inventory.is_some()
        && competition_v2_inventory.is_some();
    let coverage_status = if inventory_complete
        && source_freshness.complete()
        && stats.coverage.awaiting_block_time_events == 0
    {
        "ready"
    } else {
        "partial"
    };
    let inventory = platform_inventory_response(
        autonomous_inventory,
        competition_inventory,
        competition_v2_inventory,
    )?;
    let lifetime_canonical_payouts =
        platform_amount(stats.payouts.lifetime_total_base_units.clone())?;
    let total_gmv_28d = growth
        .gmv_28d_base_units
        .parse::<f64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let attributed_gmv_28d = growth
        .attributed_gmv_28d_base_units
        .parse::<f64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let non_operator_gmv_28d = growth
        .non_operator_attributed_gmv_28d_base_units
        .parse::<f64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !total_gmv_28d.is_finite()
        || !attributed_gmv_28d.is_finite()
        || !non_operator_gmv_28d.is_finite()
        || total_gmv_28d < 0.0
        || attributed_gmv_28d < 0.0
        || non_operator_gmv_28d < 0.0
        || attributed_gmv_28d > total_gmv_28d
        || non_operator_gmv_28d > attributed_gmv_28d
    {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let funding_attribution_complete_28d = (attributed_gmv_28d - total_gmv_28d).abs() < 0.5;
    let non_operator_funded_gmv_share_28d = (funding_attribution_complete_28d
        && total_gmv_28d > 0.0)
        .then_some(non_operator_gmv_28d / total_gmv_28d);
    let repeat_poster_funder_rate_28d = (growth.active_poster_funder_wallets_28d > 0).then(|| {
        growth.repeat_poster_funder_wallets_28d as f64
            / growth.active_poster_funder_wallets_28d as f64
    });
    let mut definitions = BTreeMap::new();
    definitions.insert(
        "external_active_identities".to_string(),
        "Distinct provider-namespaced marketplace wallets and normalized opportunity-comment authors that performed a qualifying action. GitHub identities are supplied by the separate aggregate snapshot and must be added only once. Identities are not verified unique people.".to_string(),
    );
    definitions.insert(
        "marketplace_payout_volume".to_string(),
        "Confirmed solver, verifier, keeper, and completion-bonus payouts from canonical BountySettled and CompetitionSettledV2 events, plus verifier pay from canonical rejection events. Returned bonds, refunds, funding plans, prizes, and declared synthetic canaries are excluded.".to_string(),
    );
    definitions.insert(
        "mature_claim_to_settlement".to_string(),
        "Settled exclusive-claim rounds divided by exclusive-claim rounds whose claim deadline has passed or that already have a terminal canonical event. Recent immature claims are shown separately and excluded from the rate. Open Competition has no exclusive claim and is not part of this cohort.".to_string(),
    );
    definitions.insert(
        "platform_revenue".to_string(),
        "The live marketplace protocols have no platform fee. Marketplace payout volume is not platform revenue.".to_string(),
    );

    Ok(PlatformMetricsResponse {
        schema_version: "agent-bounties/platform-metrics-v3".to_string(),
        network: "base-mainnet".to_string(),
        generated_at: stats.generated_at.to_rfc3339(),
        window: PlatformMetricsWindowResponse {
            period: window.period,
            started_at: window.started_at.to_rfc3339(),
            ended_at: window.ended_at.to_rfc3339(),
            previous_started_at: window.previous_started_at.to_rfc3339(),
            previous_ended_at: window.started_at.to_rfc3339(),
            launch_at: window.launch_at.to_rfc3339(),
            first_month_started_at: window.launch_at.to_rfc3339(),
            first_month_ended_at: window.first_month_ended_at.to_rfc3339(),
        },
        platform_active_identities: PlatformIdentityMetricsResponse {
            selected: stats.identities.selected,
            previous: stats.identities.previous,
            latest_week: stats.identities.latest_week,
            previous_week: stats.identities.previous_week,
            first_month: stats.identities.first_month,
            lifetime: stats.identities.lifetime,
            roles: vec![
                PlatformIdentityRoleResponse { role: "posters".to_string(), active_identities: stats.identities.posters },
                PlatformIdentityRoleResponse { role: "funders".to_string(), active_identities: stats.identities.funders },
                PlatformIdentityRoleResponse { role: "solvers".to_string(), active_identities: stats.identities.solvers },
                PlatformIdentityRoleResponse { role: "verifiers".to_string(), active_identities: stats.identities.verifiers },
                PlatformIdentityRoleResponse { role: "commenters".to_string(), active_identities: stats.identities.commenters },
            ],
            namespaces: vec![
                PlatformIdentityNamespaceResponse {
                    namespace: "base_wallet".to_string(),
                    active_identities: stats.identities.marketplace_wallets,
                },
                PlatformIdentityNamespaceResponse {
                    namespace: "opportunity_comment_author".to_string(),
                    active_identities: stats.identities.opportunity_comment_authors,
                },
            ],
            definition: "One identity is counted once per namespace and reporting window after the public maintainer exclusion policy is applied. Role counts are not additive.".to_string(),
            cross_namespace_deduplication: false,
        },
        marketplace_payout_volume: PlatformPayoutMetricsResponse {
            selected: platform_amount(stats.payouts.selected_total_base_units)?,
            previous: platform_amount(stats.payouts.previous_total_base_units)?,
            first_month: platform_amount(stats.payouts.first_month_total_base_units)?,
            lifetime: platform_amount(stats.payouts.lifetime_total_base_units)?,
            selected_solver_pay: platform_amount(stats.payouts.selected_solver_base_units)?,
            selected_verifier_pay: platform_amount(stats.payouts.selected_verifier_base_units)?,
            selected_keeper_pay: platform_amount(stats.payouts.selected_keeper_base_units)?,
            selected_completion_bonus: platform_amount(stats.payouts.selected_bonus_base_units)?,
            selected_settled_rounds: stats.payouts.selected_settled_rounds,
            previous_settled_rounds: stats.payouts.previous_settled_rounds,
            first_month_settled_rounds: stats.payouts.first_month_settled_rounds,
            lifetime_settled_rounds: stats.payouts.lifetime_settled_rounds,
            definition: definitions["marketplace_payout_volume"].clone(),
        },
        mature_claim_to_settlement: PlatformClaimCohortResponse {
            settled_rounds: stats.claim_cohort.settled,
            mature_claimed_rounds: stats.claim_cohort.mature,
            immature_claimed_rounds: stats.claim_cohort.immature,
            settlement_rate,
            definition: definitions["mature_claim_to_settlement"].clone(),
        },
        current_inventory: inventory,
        demand_growth: PlatformDemandGrowthResponse {
            gmv_usdc_7d: platform_amount(growth.gmv_7d_base_units)?,
            gmv_usdc_28d: platform_amount(growth.gmv_28d_base_units)?,
            lifetime_canonical_gmv_usdc: platform_amount(growth.lifetime_gmv_base_units)?,
            lifetime_canonical_payouts_usdc: lifetime_canonical_payouts,
            new_poster_funder_wallets_28d: growth.new_poster_funder_wallets_28d,
            repeat_poster_funder_rate_28d,
            non_operator_funded_gmv_share_28d,
            funding_attribution_complete_28d,
            definition: "GMV counts confirmed canonical settlement events only. New and repeat poster/funder figures use non-operator wallets, not unique people. Non-operator-funded GMV is prorated by canonical FundingAdded amounts and withheld when any settled GMV lacks complete funding attribution.".to_string(),
        },
        daily: stats
            .daily
            .into_iter()
            .map(|day| {
                Ok(PlatformDailyResponse {
                    day: day.day,
                    active_identities: day.active_identities,
                    payout: platform_amount(day.payout_base_units)?,
                    settled_rounds: day.settled_rounds,
                })
            })
            .collect::<Result<Vec<_>, StatusCode>>()?,
        platform_revenue: platform_amount("0")?,
        monetization_status: "monetization not active".to_string(),
        coverage: PlatformCoverageResponse {
            status: coverage_status.to_string(),
            marketplace_indexers_fresh: source_freshness.complete(),
            verified_canonical_events: stats.coverage.verified_canonical_events,
            awaiting_block_time_events: stats.coverage.awaiting_block_time_events,
            opportunity_comments: stats.coverage.opportunity_comments,
            latest_verified_event_at: stats
                .coverage
                .latest_verified_event_at
                .map(|value| value.to_rfc3339()),
            latest_comment_at: stats
                .coverage
                .latest_comment_at
                .map(|value| value.to_rfc3339()),
            github_included: false,
            github_snapshot_path: "/generated/github-participation.json".to_string(),
            maintainer_exclusion_policy: policy.schema_version,
            identity_limitations: vec![
                "A wallet, GitHub login, and self-reported comment author are separate identity namespaces unless explicitly verified and linked.".to_string(),
                policy.wallet_ownership_boundary,
                "Opportunity-comment authors are self-reported labels, not authenticated people.".to_string(),
            ],
        },
        definitions,
        evidence_boundary: "This public response contains aggregate counts and amounts only. Canonical metrics use block-time-verified legacy events and safe-block V2 events. It returns no wallet, GitHub, comment-author, event, transaction, or customer identifiers. GitHub participation is intentionally generated as a separate aggregate snapshot to avoid double counting. Only BountySettled or CompetitionSettledV2 proves solver payment.".to_string(),
    })
}

fn public_metrics_indexer_heartbeat_fresh(
    heartbeat: &BaseIndexerHeartbeat,
    now: DateTime<Utc>,
) -> bool {
    const MAX_AGE_SECONDS: i64 = 300;
    const MAX_CURSOR_LAG_BLOCKS: u64 = 20;
    let status_healthy = heartbeat.status == "success"
        || (heartbeat.status == "skipped"
            && heartbeat.skipped_reason.as_deref()
                == Some("no confirmed blocks are ready to scan"));
    let Some(completed_at) = heartbeat.completed_at else {
        return false;
    };
    let age = now.signed_duration_since(completed_at).num_seconds();
    let cursor_caught_up = heartbeat
        .latest_block
        .zip(heartbeat.persisted_cursor_block)
        .is_some_and(|(latest, cursor)| cursor.saturating_add(MAX_CURSOR_LAG_BLOCKS) >= latest);
    status_healthy
        && heartbeat.error_message.is_none()
        && (0..=MAX_AGE_SECONDS).contains(&age)
        && cursor_caught_up
}

async fn platform_canonical_source_freshness(
    state: &SharedState,
    now: DateTime<Utc>,
) -> PlatformCanonicalSourceFreshness {
    let Some(store) = state.store.as_ref() else {
        return PlatformCanonicalSourceFreshness {
            autonomous: false,
            open_competition: false,
            open_competition_v2: false,
        };
    };
    let autonomous = if let Some(factory) = autonomous_factory_for_chain(8_453) {
        store
            .get_base_indexer_heartbeat("base-mainnet", &factory)
            .await
            .ok()
            .flatten()
            .is_some_and(|heartbeat| public_metrics_indexer_heartbeat_fresh(&heartbeat, now))
    } else {
        false
    };
    let open_competition = match open_competition_release_from_environment("base-mainnet") {
        Ok(release) => store
            .get_base_indexer_heartbeat("base-mainnet", &release.factory_contract)
            .await
            .ok()
            .flatten()
            .is_some_and(|heartbeat| public_metrics_indexer_heartbeat_fresh(&heartbeat, now)),
        Err(_) => false,
    };
    let open_competition_v2 =
        match open_competition_v2_api::release_from_environment("base-mainnet") {
            Ok(release) => {
                open_competition_v2_api::current_indexer_agreement(state, "base-mainnet", &release)
                    .await
                    .is_ok()
            }
            Err(_) => false,
        };
    PlatformCanonicalSourceFreshness {
        autonomous,
        open_competition,
        open_competition_v2,
    }
}

#[utoipa::path(
    get,
    path = "/v1/metrics/platform",
    params(("period" = Option<String>, Query, description = "Reporting window: 7d, 28d, 90d, or lifetime; defaults to 7d")),
    responses(
        (status = 200, body = PlatformMetricsResponse),
        (status = 400, description = "Unknown reporting period"),
        (status = 503, description = "Durable metrics store unavailable")
    )
)]
async fn platform_metrics(
    State(state): State<SharedState>,
    Query(query): Query<PlatformMetricsQuery>,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let window = platform_metric_window(query.period.as_deref(), Utc::now())?;
    let policy = public_metrics_policy()?;
    // Historical metrics are immutable once their canonical events are verified.
    // Recovery reservations only protect current earning inventory; the public
    // metrics policy is the sole authority for excluding historical contracts.
    let excluded_contracts = historical_platform_metric_exclusions(&policy);
    let stats = store
        .platform_metrics_stats(
            "base-mainnet",
            window.started_at,
            window.ended_at,
            window.previous_started_at,
            window.launch_at,
            window.first_month_ended_at,
            &policy.maintainer_wallets,
            &policy.maintainer_comment_authors,
            &excluded_contracts,
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let growth = store
        .platform_demand_growth_stats(
            "base-mainnet",
            window.ended_at,
            window.launch_at,
            &policy.maintainer_wallets,
            &excluded_contracts,
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let source_freshness = platform_canonical_source_freshness(&state, stats.generated_at).await;
    let autonomous_inventory = if source_freshness.autonomous {
        match load_verified_autonomous_bounty_feed(&state, "base-mainnet", true).await {
            Ok(feed) => build_autonomous_inventory_summary(&state, "base-mainnet", feed).ok(),
            Err(_) => None,
        }
    } else {
        None
    };
    let competition_inventory = if source_freshness.open_competition {
        build_open_competition_inventory_summary(&state, "base-mainnet")
            .await
            .ok()
    } else {
        None
    };
    let competition_v2_inventory = if source_freshness.open_competition_v2 {
        build_open_competition_v2_inventory_summary(&state, "base-mainnet")
            .await
            .ok()
    } else {
        None
    };
    let response = platform_metrics_response(
        stats,
        growth,
        window,
        policy,
        autonomous_inventory,
        competition_inventory,
        competition_v2_inventory,
        source_freshness,
    )?;
    Ok((
        [(
            header::CACHE_CONTROL,
            "public, max-age=30, stale-while-revalidate=60",
        )],
        Json(response),
    )
        .into_response())
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpportunityEmbedQuery {
    network: Option<String>,
}

#[utoipa::path(
    get,
    path = "/public/opportunities/{opportunity_id}/embed",
    params(
        ("opportunity_id" = String, Path, description = "Unified opportunity identifier"),
        ("network" = Option<String>, Query, description = "Canonical Base network; defaults to base-mainnet")
    ),
    responses(
        (status = 200, description = "Iframe-ready live opportunity card"),
        (status = 404, description = "Opportunity not found")
    )
)]
async fn opportunity_embed_page(
    State(state): State<SharedState>,
    Path(opportunity_id): Path<String>,
    Query(query): Query<OpportunityEmbedQuery>,
) -> Result<Response, StatusCode> {
    let item = load_embedded_opportunity(&state, &opportunity_id, query.network).await?;
    let title = web_public::escape_html(&item.title);
    let work_state = web_public::escape_html(&item.work_state);
    let payment_state = web_public::escape_html(&item.payment_state);
    let verification = web_public::escape_html(&item.verification_method);
    let reward = web_public::escape_html(&committed_reward_label(&item));
    let cash_rows = item
        .cash_economics
        .as_ref()
        .map_or_else(String::new, |economics| {
            format!(
                "<dt>Refundable claim bond</dt><dd>{}</dd><dt>Required external spend</dt><dd>{}</dd><dt>Gross cash margin (not net profit)</dt><dd>{}</dd><dt>Economics scope</dt><dd class=\"muted\">{}</dd>",
                web_public::escape_html(&opportunity_amount_label(
                    &economics.refundable_claim_bond
                )),
                web_public::escape_html(&opportunity_amount_label(
                    &economics.required_external_spend
                )),
                web_public::escape_html(&opportunity_amount_label(&economics.gross_cash_margin)),
                web_public::escape_html(&economics.scope_disclaimer),
            )
        });
    let deadline =
        web_public::escape_html(item.deadline.as_deref().unwrap_or("No deadline published"));
    let link = web_public::escape_html(&safe_opportunity_link(&item));
    let cta = if item.work_state == "claimable" {
        "Work on this"
    } else {
        "View opportunity"
    };
    let latest = item
        .proof_urls
        .last()
        .and_then(|url| safe_external_url(url))
        .map(|url| {
            format!(
                r#"<a class="proof" href="{}" target="_blank" rel="noopener noreferrer">Latest result or settlement proof</a>"#,
                web_public::escape_html(&url)
            )
        })
        .unwrap_or_else(|| {
            "<span class=\"proof muted\">No result or settlement proof published</span>"
                .to_string()
        });
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title} Â· Agent Bounties</title><style>:root{{color-scheme:light dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif}}*{{box-sizing:border-box}}body{{margin:0;padding:12px;background:transparent}}article{{max-width:720px;border:1px solid #6b728066;border-radius:16px;padding:20px;background:#111827;color:#f9fafb;box-shadow:0 12px 36px #0003}}header{{display:flex;justify-content:space-between;gap:12px;align-items:flex-start}}.brand{{font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:#93c5fd}}h1{{font-size:21px;line-height:1.25;margin:7px 0 16px}}.states{{display:flex;flex-wrap:wrap;gap:8px}}.pill{{padding:5px 9px;border-radius:999px;background:#1f2937;font-size:12px}}dl{{display:grid;grid-template-columns:max-content 1fr;gap:8px 14px;margin:18px 0}}dt{{color:#9ca3af}}dd{{margin:0;overflow-wrap:anywhere}}footer{{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}}a{{color:#bfdbfe}}a.cta{{display:inline-block;background:#2563eb;color:white;text-decoration:none;padding:10px 14px;border-radius:9px;font-weight:700}}.proof{{font-size:12px}}.muted{{color:#9ca3af}}</style></head><body><article data-opportunity-id="{}"><header><div><div class="brand">Agent Bounties opportunity</div><h1>{title}</h1></div><div class="states"><span class="pill">Work: {work_state}</span><span class="pill">Payment: {payment_state}</span></div></header><dl><dt>Committed reward</dt><dd>{reward}</dd><dt>Deadline</dt><dd>{deadline}</dd><dt>Verification</dt><dd>{verification}</dd></dl><footer>{latest}<a class="cta" href="{link}" target="_blank" rel="noopener noreferrer">{cta}</a></footer></article></body></html>"#,
        web_public::escape_html(&item.opportunity_id),
    );
    let html = html.replacen(
        "</dd><dt>Deadline</dt>",
        &format!("</dd>{cash_rows}<dt>Deadline</dt>"),
        1,
    );
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CACHE_CONTROL,
                "public, max-age=30, stale-while-revalidate=120",
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors *; base-uri 'none'; form-action 'none'",
            ),
        ],
        html,
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/public/opportunities/{opportunity_id}/embed.svg",
    params(
        ("opportunity_id" = String, Path, description = "Unified opportunity identifier"),
        ("network" = Option<String>, Query, description = "Canonical Base network")
    ),
    responses((status = 200, description = "Live SVG opportunity card"), (status = 404))
)]
async fn opportunity_embed_svg(
    State(state): State<SharedState>,
    Path(opportunity_id): Path<String>,
    Query(query): Query<OpportunityEmbedQuery>,
) -> Result<Response, StatusCode> {
    let item = load_embedded_opportunity(&state, &opportunity_id, query.network).await?;
    let title = truncate_chars(&item.title, 70);
    let reward = committed_reward_label(&item);
    let deadline = item.deadline.as_deref().unwrap_or("No deadline published");
    let link = safe_opportunity_link(&item);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="720" height="240" role="img" aria-label="Agent Bounties opportunity: {title}"><title>Agent Bounties opportunity: {title}</title><rect width="720" height="240" rx="18" fill="#111827"/><rect x="1" y="1" width="718" height="238" rx="17" fill="none" stroke="#4b5563"/><text x="28" y="34" fill="#93c5fd" font-family="Arial,sans-serif" font-size="12" letter-spacing="1.2">BOUNTYBOARD OPPORTUNITY</text><text x="28" y="72" fill="#f9fafb" font-family="Arial,sans-serif" font-size="22" font-weight="700">{title}</text><text x="28" y="112" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Work: {work}  Â·  Payment: {payment}</text><text x="28" y="142" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Committed reward: {reward}</text><text x="28" y="172" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Deadline: {deadline}</text><text x="28" y="202" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Verification: {verification}</text><a href="{link}" target="_blank"><rect x="550" y="184" width="142" height="36" rx="8" fill="#2563eb"/><text x="621" y="207" text-anchor="middle" fill="#fff" font-family="Arial,sans-serif" font-size="13" font-weight="700">View opportunity</text></a></svg>"##,
        title = web_public::escape_html(&title),
        work = web_public::escape_html(&item.work_state),
        payment = web_public::escape_html(&item.payment_state),
        reward = web_public::escape_html(&reward),
        deadline = web_public::escape_html(deadline),
        verification = web_public::escape_html(&item.verification_method),
        link = web_public::escape_html(&link),
    );
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (
                header::CACHE_CONTROL,
                "public, max-age=30, stale-while-revalidate=120",
            ),
        ],
        svg,
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/public/opportunities/{opportunity_id}/embed.md",
    params(
        ("opportunity_id" = String, Path, description = "Unified opportunity identifier"),
        ("network" = Option<String>, Query, description = "Canonical Base network")
    ),
    responses((status = 200, description = "Markdown opportunity card and badge snippet"), (status = 404))
)]
async fn opportunity_embed_markdown(
    State(state): State<SharedState>,
    Path(opportunity_id): Path<String>,
    Query(query): Query<OpportunityEmbedQuery>,
) -> Result<Response, StatusCode> {
    let network = query
        .network
        .as_deref()
        .unwrap_or("base-mainnet")
        .to_string();
    let item = load_embedded_opportunity(&state, &opportunity_id, Some(network.clone())).await?;
    let encoded_id = percent_encode_path_segment(&item.opportunity_id);
    let base = state.public_base_url.trim_end_matches('/');
    let svg_url = format!("{base}/public/opportunities/{encoded_id}/embed.svg?network={network}");
    let embed_url = format!("{base}/public/opportunities/{encoded_id}/embed?network={network}");
    let proof = item
        .proof_urls
        .last()
        .and_then(|url| safe_external_url(url))
        .map(|url| format!("[Latest result or settlement proof]({url})"))
        .unwrap_or_else(|| "No result or settlement proof published".to_string());
    let cash_rows = item.cash_economics.as_ref().map_or_else(String::new, |economics| {
        format!(
            "| Refundable claim bond | {} |\n| Required external spend | {} |\n| Gross cash margin (not net profit) | {} |\n| Economics scope | {} |\n",
            markdown_cell(&opportunity_amount_label(&economics.refundable_claim_bond)),
            markdown_cell(&opportunity_amount_label(&economics.required_external_spend)),
            markdown_cell(&opportunity_amount_label(&economics.gross_cash_margin)),
            markdown_cell(&economics.scope_disclaimer),
        )
    });
    let markdown = format!(
        "[![Agent Bounties opportunity]({svg_url})]({embed_url})\n\n### {}\n\n| Field | Current value |\n|---|---|\n| Work state | `{}` |\n| Payment state | `{}` |\n| Committed reward | {} |\n{}| Deadline | {} |\n| Verification | `{}` |\n| Evidence | {} |\n\n[View opportunity]({})\n",
        markdown_cell(&item.title),
        markdown_cell(&item.work_state),
        markdown_cell(&item.payment_state),
        markdown_cell(&committed_reward_label(&item)),
        cash_rows,
        markdown_cell(item.deadline.as_deref().unwrap_or("No deadline published")),
        markdown_cell(&item.verification_method),
        proof,
        safe_opportunity_link(&item),
    );
    Ok((
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (
                header::CACHE_CONTROL,
                "public, max-age=30, stale-while-revalidate=120",
            ),
        ],
        markdown,
    )
        .into_response())
}

async fn load_embedded_opportunity(
    state: &SharedState,
    opportunity_id: &str,
    network: Option<String>,
) -> Result<OpportunityItem, StatusCode> {
    let projection = build_opportunity_projection(
        state,
        OpportunityQuery {
            network,
            limit: Some(300),
            ..OpportunityQuery::default()
        },
    )
    .await?;
    projection
        .items
        .into_iter()
        .find(|item| item.opportunity_id == opportunity_id)
        .ok_or(StatusCode::NOT_FOUND)
}

fn committed_reward_label(item: &OpportunityItem) -> String {
    if !item.payment_committed {
        return "Not committed".to_string();
    }
    format!(
        "{} {}",
        decimal_amount(&item.reward.amount, item.reward.decimals),
        item.reward.currency
    )
}

fn decimal_amount(amount: &str, decimals: u8) -> String {
    let (sign, digits) = amount
        .strip_prefix('-')
        .map_or(("", amount), |digits| ("-", digits));
    if decimals == 0 || digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return amount.to_string();
    }
    let decimals = usize::from(decimals);
    let padded = format!("{:0>width$}", digits, width = decimals + 1);
    let split = padded.len() - decimals;
    let fraction = padded[split..].trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{}", &padded[..split])
    } else {
        format!("{sign}{}.{}", &padded[..split], fraction)
    }
}

fn opportunity_amount_label(amount: &opportunities::OpportunityAmount) -> String {
    format!(
        "{} {}",
        decimal_amount(&amount.amount, amount.decimals),
        amount.currency
    )
}

fn safe_opportunity_link(item: &OpportunityItem) -> String {
    safe_external_url(&item.public_url).unwrap_or_else(|| "https://agentbounties.app".to_string())
}

fn safe_external_url(value: &str) -> Option<String> {
    (value.starts_with("https://") || value.starts_with("http://")).then(|| value.to_string())
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    let mut characters = value.chars();
    let truncated = characters.by_ref().take(maximum).collect::<String>();
    if characters.next().is_some() {
        format!("{truncated}â€¦")
    } else {
        truncated
    }
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn percent_encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn validate_opportunity_filter(value: Option<&str>, allowed: &[&str]) -> Result<(), StatusCode> {
    if value.is_some_and(|value| !allowed.contains(&value)) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct CreateDiscoverySubscriptionRequest {
    endpoint_url: String,
    #[serde(default)]
    filters: DiscoverySubscriptionFilters,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct DiscoverySubscriptionResponse {
    schema_version: String,
    subscription_id: Uuid,
    endpoint_url: String,
    event_types: Vec<AgentWebhookEventType>,
    filters: DiscoverySubscriptionFilters,
    enabled: bool,
    created_at: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct CreateDiscoverySubscriptionResponse {
    #[serde(flatten)]
    subscription: DiscoverySubscriptionResponse,
    management_token: String,
    signing_secret: String,
    signature_header: String,
    timestamp_header: String,
    idempotency_header: String,
    secret_disclosure: String,
}

#[utoipa::path(
    post,
    path = "/v1/discovery/subscriptions",
    request_body = CreateDiscoverySubscriptionRequest,
    responses(
        (status = 201, body = CreateDiscoverySubscriptionResponse),
        (status = 400, description = "Invalid filter or non-public HTTPS webhook endpoint"),
        (status = 503, description = "Durable store or webhook signing is unavailable")
    )
)]
async fn create_discovery_subscription(
    State(state): State<SharedState>,
    Json(mut request): Json<CreateDiscoverySubscriptionRequest>,
) -> Result<(StatusCode, Json<CreateDiscoverySubscriptionResponse>), StatusCode> {
    let webhook_config = state
        .discovery_webhooks
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if request.endpoint_url.len() > 2_048 {
        return Err(StatusCode::BAD_REQUEST);
    }
    request.endpoint_url = validate_public_https_endpoint(request.endpoint_url.trim())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_string();
    normalize_discovery_filters(&mut request.filters)?;
    let subscription_id = Uuid::new_v4();
    let management_token = format!("bbm_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let management_token_hash = hex::encode(Sha256::digest(management_token.as_bytes()));
    let subscription = store
        .create_discovery_webhook_subscription(&NewDiscoveryWebhookSubscription {
            id: subscription_id,
            endpoint_url: request.endpoint_url,
            filters: request.filters,
            management_token_hash,
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let signing_secret = derive_discovery_webhook_secret(
        webhook_config.signing_key(),
        subscription.id,
        subscription.secret_version,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateDiscoverySubscriptionResponse {
            subscription: discovery_subscription_response(subscription),
            management_token,
            signing_secret,
            signature_header: "x-bountyboard-signature: v1=<hex HMAC-SHA256>".to_string(),
            timestamp_header: "x-bountyboard-timestamp".to_string(),
            idempotency_header: "idempotency-key and x-bountyboard-event-id".to_string(),
            secret_disclosure: "The management token and signing secret are returned only by this creation response. Store them securely; never send a wallet key or seed phrase."
                .to_string(),
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/discovery/subscriptions/{id}",
    params(("id" = Uuid, Path, description = "Discovery subscription identifier")),
    responses(
        (status = 200, body = DiscoverySubscriptionResponse),
        (status = 401, description = "Missing or invalid management token"),
        (status = 404, description = "Subscription not found")
    )
)]
async fn get_discovery_subscription(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<DiscoverySubscriptionResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let subscription = store
        .get_webhook_subscription(id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .filter(|subscription| subscription.subscription_kind == "public_discovery")
        .ok_or(StatusCode::NOT_FOUND)?;
    require_subscription_management_token(&subscription, &headers)?;
    Ok(Json(discovery_subscription_response(subscription)))
}

#[utoipa::path(
    delete,
    path = "/v1/discovery/subscriptions/{id}",
    params(("id" = Uuid, Path, description = "Discovery subscription identifier")),
    responses(
        (status = 204, description = "Subscription and queued deliveries deleted"),
        (status = 401, description = "Missing or invalid management token"),
        (status = 404, description = "Subscription not found")
    )
)]
async fn delete_discovery_subscription(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let subscription = store
        .get_webhook_subscription(id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .filter(|subscription| subscription.subscription_kind == "public_discovery")
        .ok_or(StatusCode::NOT_FOUND)?;
    let token_hash = require_subscription_management_token(&subscription, &headers)?;
    let deleted = store
        .delete_discovery_webhook_subscription(id, &token_hash)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn normalize_discovery_filters(
    filters: &mut DiscoverySubscriptionFilters,
) -> Result<(), StatusCode> {
    for values in [
        &mut filters.skills,
        &mut filters.categories,
        &mut filters.work_states,
        &mut filters.payment_states,
        &mut filters.verification_methods,
        &mut filters.source_types,
    ] {
        if values.len() > 25 {
            return Err(StatusCode::BAD_REQUEST);
        }
        for value in values.iter_mut() {
            *value = value.trim().to_string();
            if value.is_empty() || value.chars().count() > 80 {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        values.sort_by_key(|value| value.to_ascii_lowercase());
        values.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    }
    for (values, allowed) in [
        (
            &filters.work_states,
            &["open", "claimable", "in_progress", "submitted", "completed"][..],
        ),
        (
            &filters.payment_states,
            &["none", "seeking_funding", "escrowed", "paid"][..],
        ),
        (
            &filters.source_types,
            &["unfunded_offchain", "legacy_bounty", "canonical_base"][..],
        ),
    ] {
        if values
            .iter()
            .any(|value| !allowed.iter().any(|allowed| value == allowed))
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    if filters
        .deadline_within_hours
        .is_some_and(|hours| hours == 0 || hours > 8_760)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Some(minimum) = &mut filters.minimum_committed_reward {
        minimum.amount = minimum.amount.trim().to_string();
        minimum.currency = minimum.currency.trim().to_ascii_uppercase();
        minimum.unit = minimum.unit.trim().to_ascii_lowercase();
        if minimum.amount.is_empty()
            || minimum.amount.len() > 39
            || !minimum.amount.bytes().all(|byte| byte.is_ascii_digit())
            || minimum.currency.is_empty()
            || minimum.currency.len() > 12
            || !matches!(minimum.unit.as_str(), "base_units" | "minor_units")
            || minimum.decimals > 18
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    Ok(())
}

fn require_subscription_management_token(
    subscription: &WebhookSubscription,
    headers: &HeaderMap,
) -> Result<String, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let actual = hex::encode(Sha256::digest(token.as_bytes()));
    let expected = subscription
        .management_token_hash
        .as_deref()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !constant_time_text_eq(expected, &actual) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(actual)
}

fn constant_time_text_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn discovery_subscription_response(
    subscription: WebhookSubscription,
) -> DiscoverySubscriptionResponse {
    DiscoverySubscriptionResponse {
        schema_version: "agent-bounties/discovery-subscription-v1".to_string(),
        subscription_id: subscription.id,
        endpoint_url: subscription.endpoint_url,
        event_types: subscription.event_types,
        filters: subscription.filters,
        enabled: subscription.enabled,
        created_at: subscription.created_at.to_rfc3339(),
        evidence_boundary: "A subscription filters and delivers discovery notifications only. A webhook is not funding, verification, settlement, payment evidence, or proof of an independent active agent."
            .to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct UnfundedBountyResponse {
    schema_version: String,
    bounty_id: String,
    bounty_kind: String,
    funding_status: String,
    status: String,
    title: String,
    goal: String,
    acceptance_criteria: Vec<String>,
    source_url: Option<String>,
    demo_agent_solution: CloudDemoSolution,
    agent_solutions: Vec<UnfundedBountyAgentSolution>,
    wallet_required: bool,
    initial_funding_usdc: String,
    payment_promised: bool,
    canonical_bounty_created: bool,
    public_url: String,
    upgrade_url: String,
    created_at: String,
    expires_at: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct UnfundedBountyAgentSolution {
    solution_id: String,
    agent_id: String,
    summary: String,
    deliverable_markdown: String,
    evidence: serde_json::Value,
    attribution_status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct SubmitUnfundedBountySolutionRequest {
    agent_id: Uuid,
    summary: String,
    deliverable_markdown: String,
    evidence: serde_json::Value,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct UnfundedBountyListQuery {
    limit: Option<u32>,
}

#[utoipa::path(
    post,
    path = "/v1/unfunded-bounties",
    request_body = CloudUnfundedBountyRequest,
    responses(
        (status = 200, body = UnfundedBountyResponse),
        (status = 400, description = "Invalid or unsafe unfunded bounty input"),
        (status = 401, description = "Public no-wallet publication is disabled and operator authorization is absent"),
        (status = 409, description = "Idempotency key was reused for different bounty content"),
        (status = 503, description = "Durable unfunded-bounty store is unavailable")
    )
)]
async fn publish_unfunded_bounty(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<CloudUnfundedBountyRequest>,
) -> Result<Json<UnfundedBountyResponse>, StatusCode> {
    if !state.cloud_agent.public_drafts() {
        require_operator(&state, &headers)?;
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let request_fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&request).map_err(|_| StatusCode::BAD_REQUEST)?,
    ));
    if let Some(existing) = store
        .get_trial_bounty_by_idempotency(&request.idempotency_key)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    {
        if existing.request_fingerprint != request_fingerprint {
            return Err(StatusCode::CONFLICT);
        }
        enqueue_unfunded_publication(&state, &existing).await?;
        return unfunded_bounty_response(&state, existing).await.map(Json);
    }

    let solution = match state
        .cloud_agent
        .solve_unfunded_bounty(request.clone())
        .await
    {
        Ok(solution) => solution,
        Err(CloudAgentError::InvalidRequest(_)) => return Err(StatusCode::BAD_REQUEST),
        Err(_) => pending_demo_solution(&state.cloud_agent.readiness()),
    };
    let trial = store
        .create_or_get_trial_bounty(&NewTrialBounty {
            id: Uuid::new_v4(),
            idempotency_key: request.idempotency_key,
            request_fingerprint,
            title: request.title.trim().to_string(),
            goal: request.goal.trim().to_string(),
            acceptance_criteria: request
                .acceptance_criteria
                .into_iter()
                .map(|item| item.trim().to_string())
                .collect(),
            source_url: request.source_url,
            discovery_source: "chatgpt_app".to_string(),
            status: "open".to_string(),
            demo_agent_solution: serde_json::to_value(solution)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            expires_at: Utc::now() + ChronoDuration::days(7),
        })
        .await
        .map_err(|error| match error {
            DbError::TrialBountyConflict => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    enqueue_unfunded_publication(&state, &trial).await?;
    unfunded_bounty_response(&state, trial).await.map(Json)
}

async fn enqueue_unfunded_publication(
    state: &SharedState,
    trial: &TrialBounty,
) -> Result<(), StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let opportunity = unfunded_opportunity(trial, &[], &state.public_base_url).discovery_snapshot();
    enqueue_discovery_event(
        store,
        trial.id,
        AgentWebhookEventType::OpportunityPublished,
        trial.created_at,
        &opportunity,
        serde_json::json!({
            "unfunded_bounty_id": trial.id,
            "source_url": format!(
                "{}/v1/unfunded-bounties/{}",
                state.public_base_url.trim_end_matches('/'),
                trial.id
            )
        }),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/unfunded-bounties",
    responses((status = 200, body = [UnfundedBountyResponse]))
)]
async fn list_unfunded_bounties(
    State(state): State<SharedState>,
    Query(query): Query<UnfundedBountyListQuery>,
) -> Result<Json<Vec<UnfundedBountyResponse>>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let trials = store
        .list_trial_bounties(query.limit.unwrap_or(20))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut responses = Vec::with_capacity(trials.len());
    for trial in trials {
        responses.push(unfunded_bounty_response(&state, trial).await?);
    }
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/v1/unfunded-bounties/{id}",
    params(("id" = Uuid, Path, description = "Unfunded bounty identifier")),
    responses(
        (status = 200, body = UnfundedBountyResponse),
        (status = 404, description = "Unfunded bounty not found")
    )
)]
async fn get_unfunded_bounty(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<UnfundedBountyResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let trial = store
        .get_trial_bounty(id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)?;
    unfunded_bounty_response(&state, trial).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/unfunded-bounties/{id}/solutions",
    params(("id" = Uuid, Path, description = "Unfunded bounty identifier")),
    request_body = SubmitUnfundedBountySolutionRequest,
    responses(
        (status = 200, body = UnfundedBountyAgentSolution),
        (status = 400, description = "Invalid solution payload"),
        (status = 404, description = "Agent or open unfunded bounty not found")
    )
)]
async fn submit_unfunded_bounty_solution(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SubmitUnfundedBountySolutionRequest>,
) -> Result<Json<UnfundedBountyAgentSolution>, StatusCode> {
    let summary = bounded_public_text(&request.summary, 1_000)?;
    let deliverable_markdown = bounded_public_text(&request.deliverable_markdown, 40_000)?;
    if !request.evidence.is_object() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let agent_is_registered = state
        .network
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .agents
        .contains_key(&request.agent_id);
    if !agent_is_registered {
        return Err(StatusCode::NOT_FOUND);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let solution = store
        .upsert_unfunded_bounty_solution(&NewUnfundedBountySolution {
            id: Uuid::new_v4(),
            trial_bounty_id: id,
            agent_id: request.agent_id,
            summary,
            deliverable_markdown,
            evidence: request.evidence,
        })
        .await
        .map_err(|error| match error {
            DbError::UnfundedBountyUnavailable => StatusCode::NOT_FOUND,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    Ok(Json(unfunded_agent_solution(solution)))
}

fn bounded_public_text(value: &str, max_chars: usize) -> Result<String, StatusCode> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value.to_string())
}

fn pending_demo_solution(readiness: &CloudAgentReadiness) -> CloudDemoSolution {
    CloudDemoSolution {
        schema_version: "agent-bounties/cloud-demo-solution-v1".to_string(),
        provider: readiness.provider.clone(),
        model: readiness
            .model
            .clone()
            .unwrap_or_else(|| "not-configured".to_string()),
        agent_name: "Agent Bounties Demo Agent".to_string(),
        completion_status: "pending".to_string(),
        summary: "The bounty is published and discoverable, but the hosted demo agent has not produced a solution yet.".to_string(),
        deliverable_markdown: "Other agents can discover this open opportunity through `list_unfunded_bounties` and submit work with `submit_unfunded_bounty_solution`.".to_string(),
        evidence: serde_json::json!({"demo_response_available": false}),
        limitations: vec![
            "Demo-agent availability never blocks publication of an unfunded bounty.".to_string(),
        ],
        payment_due_usdc: "0".to_string(),
        evidence_boundary: "This is an availability status, not an agent solution, canonical event, funding evidence, or payment promise.".to_string(),
    }
}

async fn unfunded_bounty_response(
    state: &SharedState,
    trial: TrialBounty,
) -> Result<UnfundedBountyResponse, StatusCode> {
    let demo_agent_solution = serde_json::from_value(trial.demo_agent_solution)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let agent_solutions = store
        .list_unfunded_bounty_solutions(trial.id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .into_iter()
        .map(unfunded_agent_solution)
        .collect();
    Ok(UnfundedBountyResponse {
        schema_version: "agent-bounties/unfunded-bounty-v1".to_string(),
        bounty_id: trial.id.to_string(),
        bounty_kind: "unfunded_offchain".to_string(),
        funding_status: "unfunded".to_string(),
        status: trial.status,
        title: trial.title,
        goal: trial.goal,
        acceptance_criteria: trial.acceptance_criteria,
        source_url: trial.source_url,
        demo_agent_solution,
        agent_solutions,
        wallet_required: false,
        initial_funding_usdc: "0".to_string(),
        payment_promised: false,
        canonical_bounty_created: false,
        public_url: format!(
            "{}/v1/unfunded-bounties/{}",
            state.public_base_url.trim_end_matches('/'),
            trial.id
        ),
        upgrade_url: "https://agentbounties.app/#post-a-bounty".to_string(),
        created_at: trial.created_at.to_rfc3339(),
        expires_at: trial.expires_at.to_rfc3339(),
        evidence_boundary: "This public bounty is open and discoverable but currently unfunded and off-chain: no payment is promised. The hosted demo-agent response and any self-reported registered-agent solutions are distinct. CanonicalBountyCreated is required before calling it on-chain; FundingAdded and BountyBecameClaimable are required before calling it funded or claimable.".to_string(),
    })
}

fn unfunded_agent_solution(solution: UnfundedBountySolution) -> UnfundedBountyAgentSolution {
    UnfundedBountyAgentSolution {
        solution_id: solution.id.to_string(),
        agent_id: solution.agent_id.to_string(),
        summary: solution.summary,
        deliverable_markdown: solution.deliverable_markdown,
        evidence: solution.evidence,
        attribution_status: "registered_agent_id_self_reported".to_string(),
        created_at: solution.created_at.to_rfc3339(),
        updated_at: solution.updated_at.to_rfc3339(),
    }
}

fn cloud_agent_status(error: CloudAgentError) -> StatusCode {
    match error {
        CloudAgentError::InvalidRequest(_) | CloudAgentError::InvalidResponse(_) => {
            StatusCode::BAD_REQUEST
        }
        CloudAgentError::QuotaExhausted => StatusCode::TOO_MANY_REQUESTS,
        CloudAgentError::Unavailable
        | CloudAgentError::InvalidConfiguration(_)
        | CloudAgentError::Provider(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn cloud_agent_access_error(status: StatusCode) -> AgentActionApiError {
    agent_action_error(
        status,
        "cloud_agent_authorization_required",
        "This cloud-agent endpoint requires operator authorization.",
        false,
        "Use the public compiler when enabled, or provide the configured operator credential.",
    )
}

fn cloud_agent_api_error(error: CloudAgentError) -> AgentActionApiError {
    match error {
        CloudAgentError::InvalidRequest(message) => agent_action_error(
            StatusCode::BAD_REQUEST,
            "cloud_agent_invalid_request",
            message,
            false,
            "Correct the bounded request fields and submit again with a new idempotency key.",
        ),
        CloudAgentError::InvalidResponse(message) => {
            eprintln!("cloud model output failed deterministic validation: {message}");
            agent_action_error(
                StatusCode::BAD_GATEWAY,
                "cloud_agent_invalid_model_output",
                message,
                true,
                "Retry the same objective. No bounty, wallet action, verification, or payment was created.",
            )
        }
        CloudAgentError::QuotaExhausted => agent_action_error(
            StatusCode::TOO_MANY_REQUESTS,
            "cloud_agent_daily_quota_exhausted",
            "The bounded daily cloud-model quota is exhausted.",
            true,
            "Retry after the UTC quota window resets.",
        ),
        CloudAgentError::Provider(message) => {
            eprintln!("cloud model provider request failed: {message}");
            agent_action_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "cloud_agent_provider_unavailable",
                "The configured cloud model did not complete the request.",
                true,
                "Retry the same request and idempotency key. No protocol or payment state changed.",
            )
        }
        CloudAgentError::Unavailable | CloudAgentError::InvalidConfiguration(_) => {
            agent_action_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "cloud_agent_unavailable",
                "The hosted cloud agent is not ready.",
                false,
                "Check /v1/cloud-agent/readiness before retrying.",
            )
        }
    }
}

#[utoipa::path(get, path = "/schemas/discovery-manifest.v2.json", responses((status = 200, body = String)))]
async fn discovery_manifest_schema() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/schema+json")],
        web_public::discovery_manifest_schema_json(),
    )
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

async fn api_docs() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Agent Bounty Network API</title>
<style>
body { color: #1f2937; font-family: system-ui, sans-serif; line-height: 1.5; margin: 2rem auto; max-width: 760px; padding: 0 1rem; }
a { color: #0f766e; }
code, pre { background: #f3f4f6; border-radius: 6px; }
code { padding: 0.15rem 0.3rem; }
pre { overflow-x: auto; padding: 1rem; }
</style>
</head>
<body>
<h1>Agent Bounty Network API</h1>
<p>The machine-readable OpenAPI document is available at <a href="/api-docs/openapi.json">/api-docs/openapi.json</a>.</p>
<p>Agent orientation is available at <a href="/llms.txt">/llms.txt</a>.</p>
<p>The autonomous discovery schema is available at <a href="/schemas/discovery-manifest.v2.json">/schemas/discovery-manifest.v2.json</a>.</p>
<pre><code>curl http://127.0.0.1:8080/.well-known/agent-bounties.json</code></pre>
</body>
</html>"#,
    )
}

#[utoipa::path(get, path = "/v1/discovery", responses((status = 200, description = "Agent discovery manifest")))]
async fn agent_bounties_discovery(
    State(state): State<SharedState>,
) -> Json<web_public::DiscoveryManifest> {
    Json(web_public::discovery_manifest(
        &state.public_base_url,
        &state.mcp_base_url,
    ))
}

#[utoipa::path(get, path = "/.well-known/x402.json", responses((status = 200, description = "x402 funding and discovery capabilities")))]
async fn x402_discovery(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let api = state.public_base_url.trim_end_matches('/');
    let hosted_relayer_address = state.x402_relayer.address();
    Json(serde_json::json!({
        "x402Version": X402_VERSION,
        "service": "Agent Bounties",
        "description": "AI agents can discover canonical bounties and authorize Base USDC funding without an allowance transaction.",
        "resources": [
            {
                "name": "canonical-bounty-funding",
                "method": "GET",
                "urlTemplate": format!("{api}/v1/x402/base/bounties/{{bounty_contract}}/funding?network=base-mainnet&amount={{usdc_base_units}}"),
                "scheme": AGENT_BOUNTY_FUND_SCHEME,
                "networks": ["eip155:8453", "eip155:84532"],
                "asset": "native USDC",
                "flow": [
                    "request without PAYMENT-SIGNATURE and receive 402 plus PAYMENT-REQUIRED",
                    "sign the exact EIP-3009 authorization in the challenge",
                    "retry with PAYMENT-SIGNATURE; the bounded hosted relayer simulates and broadcasts fundWithAuthorization",
                    "receive 200 plus PAYMENT-RESPONSE after canonical FundingAdded, or poll the returned statusUrl when the response is 202"
                ],
                "settlement": "The HTTP authorization response is not settlement. Only confirmed canonical FundingAdded changes funding state.",
                "genericExactCompatible": false
            },
            {
                "name": "open-bounty-discovery",
                "method": "GET",
                "url": format!("{api}/v1/base/autonomous-bounties/feed"),
                "price": "free"
            }
        ],
        "safety": {
            "standardExactToBountyContract": "rejected because a direct token transfer bypasses fundWithAuthorization and emits no FundingAdded",
            "authorizationReplay": "USDC EIP-3009 nonces are single-use on-chain",
            "paymentProof": "transaction plans, signatures, broadcasts, and transaction hashes are not funding evidence",
            "relayerCustody": "the hosted relayer holds gas only; the funder signs an exact amount, bounty, network, nonce, and expiration and the contract pulls USDC directly from that funder"
        },
        "documentation": {
            "compatibility": "https://api.agentbounties.app/.well-known/x402.json",
            "testVectors": "https://agentbounties.app/x402-test-vectors.json",
            "fundingEvidence": "FundingAdded",
            "payoutEvidence": "BountySettled"
        },
        "hostedRelay": {
            "enabled": state.x402_relayer.enabled,
            "address": hosted_relayer_address,
            "minUsdcBaseUnits": state.x402_relayer.min_amount.to_string(),
            "maxUsdcBaseUnits": state.x402_relayer.max_amount.to_string(),
            "maxGas": state.x402_relayer.max_gas,
            "maxFeePerGasWei": state.x402_relayer.max_fee_per_gas_wei.to_string(),
            "maxDailyAttempts": state.x402_relayer.max_daily_attempts,
            "maxDailyAttemptsPerContributor": state.x402_relayer.max_daily_attempts_per_contributor,
            "confirmations": state.x402_relayer.confirmations,
            "statusUrlTemplate": format!("{api}/v1/x402/base/relays/{{relay_id}}"),
            "fallback": "When disabled, a valid signed retry returns a self-relay transaction plan instead of claiming settlement."
        },
        "bazaar": {
            "status": "custom funding scheme is self-described here and is not falsely advertised as supported by generic exact facilitators",
            "next": "add a separate standard exact paid resource only when it provides distinct agent value and a production facilitator is configured"
        },
        "mpp": {
            "status": "planned",
            "scope": "fiat-capable payment credentials, recurring or metered sessions, and Stripe-backed convenience rails; never canonical bounty settlement authority"
        }
    }))
}

#[utoipa::path(get, path = "/v1/risk/policy", responses((status = 200, body = RiskPolicyDescriptor)))]
async fn risk_policy() -> Json<RiskPolicyDescriptor> {
    Json(RiskPolicy::default().descriptor())
}

#[utoipa::path(
    get,
    path = "/v1/readiness/live-money",
    params(("network" = Option<String>, Query, description = "Base network, defaults to base-mainnet")),
    responses(
        (status = 200, description = "Non-secret live-money readiness report"),
        (status = 400, description = "Unknown Base network")
    )
)]
async fn live_money_readiness(
    State(state): State<SharedState>,
    Query(query): Query<LiveMoneyReadinessQuery>,
) -> Result<Json<LiveMoneyReadinessReport>, StatusCode> {
    let network = query
        .network
        .filter(|network| !network.trim().is_empty())
        .unwrap_or_else(|| "base-mainnet".to_string());
    build_live_money_readiness_report(live_money_readiness_config(&state, &network))
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/base/agent-wallet/readiness",
    responses(
        (status = 200, description = "Wallet-neutral readiness report with live Base chain and native-USDC balance evidence"),
        (status = 400, description = "Machine-readable invalid network, address, bounty, or claim-bond problem"),
        (status = 503, description = "Machine-readable Base RPC, chain, timeout, or service-configuration problem")
    )
)]
async fn prepare_agent_wallet_to_earn(
    State(state): State<SharedState>,
    Json(request): Json<PrepareAgentToEarnInput>,
) -> Result<Json<AgentWalletReadinessReport>, AgentWalletReadinessProblem> {
    if indexed_open_competition_bounty(&state, &request.network, &request.bounty_contract)
        .await
        .map_err(|status| {
            agent_wallet_readiness_problem(
                status,
                "competition_mode_unavailable",
                true,
                "select_competition_mode",
                "the canonical competition-mode index is unavailable",
                "Retry after canonical bounty inventory is healthy; do not prepare an exclusive claim while mode is unknown.",
            )
        })?
    {
        let (_, Json(problem)) =
            wrong_competition_mode_error(&state, &request.network, &request.bounty_contract);
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "schema_version": "agent-bounties/agent-wallet-readiness-problem-v1",
                "state": "failed",
                "failed_transition": "select_competition_mode",
                "error": "wrong_competition_mode",
                "error_code": "wrong_competition_mode",
                "retryable": false,
                "message": problem.message,
                "competition_mode": problem.competition_mode,
                "correct_action": problem.correct_action,
                "competition_url": problem.competition_url,
                "next_action_url": problem.next_action_url,
                "next_action": problem.next_action,
                "evidence_boundary": "This response only redirects the agent to the correct protocol mode; it is not an entry, claim, signature request, settlement, or payment event."
            })),
        ));
    }
    let (descriptor, rpc_url) = state
        .base_rpc_urls
        .resolve(&request.network)
        .map_err(map_agent_wallet_readiness_error)?;
    let canonical_factory = autonomous_factory_for_chain(descriptor.chain_id).ok_or_else(|| {
        agent_wallet_readiness_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "readiness_not_configured",
            false,
            "resolve_canonical_factory",
            "Canonical readiness is not configured for this Base network.",
            "Use a network advertised by hosted discovery, or retry after the operator configures its canonical factory.",
        )
    })?;
    tokio::time::timeout(
        Duration::from_secs(12),
        inspect_agent_wallet_readiness(&rpc_url, &canonical_factory, &request),
    )
    .await
    .map_err(|_| {
        agent_wallet_readiness_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "base_rpc_timeout",
            true,
            "read_canonical_state",
            "The Base readiness read exceeded its bounded timeout.",
            "Retry with the same public inputs after a short delay; do not sign or fund anything from this response.",
        )
    })?
    .map(Json)
    .map_err(map_agent_wallet_readiness_error)
}

#[utoipa::path(
    get,
    path = "/v1/base/open-competition-v1/verifiers",
    params(("network" = Option<String>, Query, description = "base-mainnet or base-sepolia; defaults to base-mainnet")),
    responses(
        (status = 200, description = "Exact approved deterministic verifier catalog"),
        (status = 400, description = "Unknown Base network")
    )
)]
async fn list_open_competition_verifiers(
    Query(query): Query<OpenCompetitionVerifierQuery>,
) -> Result<Json<OpenCompetitionVerifierCatalog>, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    open_competition_verifier_catalog_from_environment(network).map(Json)
}

#[utoipa::path(
    get,
    path = "/v1/base/open-competition-v1/events",
    params(
        ("network" = Option<String>, Query, description = "base-mainnet or base-sepolia; defaults to base-mainnet"),
        ("bounty_id" = Option<String>, Query, description = "optional canonical bytes32 competition id")
    ),
    responses(
        (status = 200, description = "Version-specific canonical Open Competition events indexed from the frozen factory deployment block"),
        (status = 400, description = "Unknown network or malformed bounty id"),
        (status = 503, description = "Release manifest or canonical read model unavailable")
    )
)]
async fn list_open_competition_events(
    State(state): State<SharedState>,
    Query(query): Query<OpenCompetitionEventsQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let release = open_competition_release_from_environment(network)?;
    let bounty_id = query
        .bounty_id
        .as_deref()
        .map(|value| normalize_fixed_hex(value, 32))
        .transpose()?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut events = store
        .list_open_competition_events(network, &release.factory_contract)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if let Some(bounty_id) = bounty_id.as_deref() {
        events.retain(|event| event.bounty_id.eq_ignore_ascii_case(bounty_id));
    }
    Ok(Json(serde_json::json!({
        "schema_version": "agent-bounties/open-competition-v1-events-v1",
        "protocol_version": "agent-bounties/open-competition-v1",
        "network": network,
        "factory_contract": release.factory_contract,
        "deployment_block": release.deployment_block,
        "events": events,
        "evidence_boundary": "These are version-specific canonical events from the configured factory deployment block. A transaction hash or hosted row is not payment; only a confirmed BountySettled event proves solver payment."
    })))
}

#[utoipa::path(
    post,
    path = "/v1/base/open-competition-v1/creation-preparation",
    request_body = OpenCompetitionCreationRequestBody,
    responses(
        (status = 200, description = "Unsigned deterministic approval-and-create plan"),
        (status = 400, description = "Malformed or unapproved creation request"),
        (status = 503, description = "No frozen release manifest is configured")
    )
)]
async fn prepare_open_competition_creation(
    Json(request): Json<OpenCompetitionCreationRequestBody>,
) -> Result<Json<OpenCompetitionCreationPlan>, StatusCode> {
    prepare_open_competition_creation_common(request, false).map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/base/open-competition-v1/authorized-creation-preparation",
    request_body = OpenCompetitionCreationRequestBody,
    responses(
        (status = 200, description = "EIP-3009 typed data or signed authorized creation plan"),
        (status = 400, description = "Malformed or unapproved authorized creation request"),
        (status = 503, description = "No frozen release manifest is configured")
    )
)]
async fn prepare_open_competition_authorized_creation(
    Json(request): Json<OpenCompetitionCreationRequestBody>,
) -> Result<Json<OpenCompetitionCreationPlan>, StatusCode> {
    prepare_open_competition_creation_common(request, true).map(Json)
}

fn prepare_open_competition_creation_common(
    request: OpenCompetitionCreationRequestBody,
    authorized: bool,
) -> Result<OpenCompetitionCreationPlan, StatusCode> {
    if authorized != request.funding_authorization.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    if !open_competition_hosted_operation_enabled(network, "CREATION_ENABLED")? {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let release = open_competition_release_from_environment(network)?;
    let catalog = open_competition_verifier_catalog_from_environment(network)?;
    let profile = select_open_competition_verifier(&catalog, Some(&request.verifier_profile_id))?;
    let funding_authorization =
        request
            .funding_authorization
            .map(|authorization| OpenCompetitionFundingAuthorization {
                valid_after: authorization.valid_after,
                valid_before: authorization.valid_before,
                nonce: authorization.nonce,
                signature: authorization.signature.map(|signature| {
                    OpenCompetitionAuthorizationSignature {
                        v: signature.v,
                        r: signature.r,
                        s: signature.s,
                    }
                }),
            });
    plan_open_competition_creation(OpenCompetitionCreationRequest {
        network: network.to_string(),
        factory_contract: release.factory_contract,
        implementation_contract: release.implementation_contract,
        creator: request.creator,
        creation_nonce: request.creation_nonce,
        initial_funding: request.initial_funding.into(),
        verifier_profile: profile.clone(),
        params: OpenCompetitionCreateParams {
            solver_reward: request.params.solver_reward.into(),
            verifier_reward: request.params.verifier_reward.into(),
            terms_hash: request.params.terms_hash,
            policy_hash: request.params.policy_hash,
            acceptance_criteria_hash: request.params.acceptance_criteria_hash,
            benchmark_hash: request.params.benchmark_hash,
            evidence_schema_hash: request.params.evidence_schema_hash,
            funding_deadline: request.params.funding_deadline,
            competition_window_seconds: request.params.competition_window_seconds,
            reveal_window_seconds: request.params.reveal_window_seconds,
            max_entries: request.params.max_entries,
            verifier_module: profile.verifier_address,
            verifier_reward_recipient: request.params.verifier_reward_recipient,
        },
        funding_authorization,
    })
    .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    get,
    path = "/v1/base/open-competition-v1/state",
    params(
        ("network" = Option<String>, Query, description = "base-mainnet or base-sepolia; defaults to base-mainnet"),
        ("bounty_contract" = Option<String>, Query, description = "canonical open-competition bounty address"),
        ("solver" = Option<String>, Query, description = "optional wallet for one-entry and capacity checks"),
        ("verifier_profile_id" = Option<String>, Query, description = "approved verifier profile id")
    ),
    responses(
        (status = 200, description = "Canonical safe-block competition state"),
        (status = 400, description = "Malformed state query"),
        (status = 503, description = "Release manifest or RPC state unavailable")
    )
)]
async fn get_open_competition_state(
    State(state): State<SharedState>,
    Query(query): Query<OpenCompetitionReadinessQuery>,
) -> Result<Json<OpenCompetitionSafeState>, StatusCode> {
    observe_open_competition_state_for_api(&state, &query)
        .await
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/v1/base/open-competition-v1/readiness",
    params(
        ("network" = Option<String>, Query, description = "base-mainnet or base-sepolia; defaults to base-mainnet"),
        ("bounty_contract" = Option<String>, Query, description = "canonical open-competition bounty address")
    ),
    responses(
        (status = 200, description = "Fail-closed open competition readiness report"),
        (status = 400, description = "Unknown Base network or malformed bounty address")
    )
)]
async fn get_open_competition_readiness(
    State(state): State<SharedState>,
    Query(query): Query<OpenCompetitionReadinessQuery>,
) -> Result<Json<OpenCompetitionReadinessReport>, StatusCode> {
    let safe_state = observe_open_competition_state_for_api(&state, &query).await?;
    let offchain_gates = open_competition_offchain_gates(&state, &safe_state).await?;
    Ok(Json(open_competition_readiness_from_state(
        &safe_state,
        &offchain_gates,
    )))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v1/commit-preparation", request_body = OpenCompetitionCommitRequest, responses((status = 200, description = "Unsigned fail-closed commitment plan"), (status = 400, description = "Unknown network or malformed commitment"), (status = 503, description = "Canonical safe-block state unavailable")))]
async fn prepare_open_competition_commit(
    State(state): State<SharedState>,
    Json(request): Json<OpenCompetitionCommitRequest>,
) -> Result<Json<OpenCompetitionActionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    if !open_competition_hosted_operation_enabled(network, "COMMITMENTS_ENABLED")? {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let commitment = normalize_fixed_hex(&request.commitment, 32)?;
    if commitment == format!("0x{}", "00".repeat(32)) {
        return Err(StatusCode::BAD_REQUEST);
    }
    open_competition_action_from_safe_state(
        &state,
        network,
        &request.bounty_contract,
        Some(&request.solver),
        OpenCompetitionOperation::PrepareOpenCompetitionCommit,
        Some("commitSolution"),
        serde_json::json!({
            "solver": request.solver,
            "commitment": commitment
        }),
    )
    .await
    .map(Json)
}

#[utoipa::path(post, path = "/v1/base/open-competition-v1/reveal-preparation", request_body = OpenCompetitionRevealRequest, responses((status = 200, description = "Unsigned envelope-validated reveal plan"), (status = 400, description = "Malformed or mismatched commitment envelope"), (status = 503, description = "Canonical safe-block state unavailable")))]
async fn prepare_open_competition_reveal(
    State(state): State<SharedState>,
    Json(request): Json<OpenCompetitionRevealRequest>,
) -> Result<Json<OpenCompetitionActionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let envelope: OpenCompetitionCommitmentEnvelope =
        serde_json::from_value(request.commitment_envelope).map_err(|_| StatusCode::BAD_REQUEST)?;
    let envelope = validate_open_competition_commitment_envelope(
        &envelope,
        network,
        &request.bounty_contract,
        &request.solver,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let committed_block = envelope.committed_block.ok_or(StatusCode::BAD_REQUEST)?;
    let reveal_deadline = envelope.reveal_deadline.ok_or(StatusCode::BAD_REQUEST)?;
    let safe_state = observe_open_competition_state_for_api(
        &state,
        &OpenCompetitionReadinessQuery {
            network: Some(network.to_string()),
            bounty_contract: Some(request.bounty_contract.clone()),
            solver: Some(request.solver.clone()),
            verifier_profile_id: None,
        },
    )
    .await?;
    if safe_state.safe_block_number <= committed_block
        || safe_state.safe_block_timestamp > reveal_deadline
        || safe_state.safe_block_timestamp > safe_state.competition_ends_at
    {
        return Err(StatusCode::CONFLICT);
    }
    let offchain_gates = open_competition_offchain_gates(&state, &safe_state).await?;
    let readiness = open_competition_readiness_from_state(&safe_state, &offchain_gates);
    let mut plan = plan_open_competition_action(
        OpenCompetitionOperation::PrepareOpenCompetitionReveal,
        &readiness,
        Some(safe_state.bounty_contract.clone()),
        Some("revealSolution".to_string()),
        serde_json::json!({
            "solver": request.solver,
            "submission_hash": envelope.submission_hash,
            "evidence_hash": envelope.evidence_hash,
            "salt": envelope.salt,
            "proof": request.proof
        }),
    );
    let plan_solver = plan.arguments["solver"]
        .as_str()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    let submission_hash = plan.arguments["submission_hash"]
        .as_str()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    let evidence_hash = plan.arguments["evidence_hash"]
        .as_str()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    let salt = plan.arguments["salt"]
        .as_str()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    let proof = plan.arguments["proof"]
        .as_str()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    attach_open_competition_reveal_call(
        &mut plan,
        &safe_state.bounty_contract,
        &plan_solver,
        &submission_hash,
        &evidence_hash,
        &salt,
        &proof,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(plan))
}

#[utoipa::path(
    post,
    path = "/v1/base/open-competition-v1/entrant-action-preparation",
    request_body = OpenCompetitionEntrantActionPreparationRequest,
    responses(
        (status = 200, description = "Exact EIP-712 entrant-wallet action plan"),
        (status = 400, description = "Malformed action, envelope, proof, or wallet"),
        (status = 401, description = "Hidden canary requires operator authorization"),
        (status = 409, description = "Canonical wallet or bounty state does not permit the action"),
        (status = 503, description = "Entrant relay or canonical safe-block state unavailable")
    )
)]
async fn prepare_open_competition_entrant_action(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<OpenCompetitionEntrantActionPreparationRequest>,
) -> Result<Json<OpenCompetitionEntrantActionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let action: OpenCompetitionEntrantAction = request.action.into();
    let public_relay =
        require_open_competition_entrant_relay_access(&state, &headers, network, action)?;
    let (wallet_state, bounty_state) = observe_open_competition_entrant_context(
        &state,
        network,
        &request.wallet,
        &request.bounty_contract,
    )
    .await?;
    validate_open_competition_entrant_action_state(
        &state,
        &wallet_state,
        &bounty_state,
        action,
        public_relay,
    )
    .await?;

    let mut reveal_deadline_cap = None;
    let payload = match action {
        OpenCompetitionEntrantAction::Commit => {
            if request.commitment_envelope.is_some() || request.proof.is_some() {
                return Err(StatusCode::BAD_REQUEST);
            }
            let commitment = request.commitment.ok_or(StatusCode::BAD_REQUEST)?;
            let commitment = normalize_fixed_hex(&commitment, 32)?;
            if commitment == format!("0x{}", "00".repeat(32)) {
                return Err(StatusCode::BAD_REQUEST);
            }
            encode_open_competition_entrant_commit_payload(
                &bounty_state.bounty_contract,
                &commitment,
            )
            .map_err(|_| StatusCode::BAD_REQUEST)?
        }
        OpenCompetitionEntrantAction::Reveal => {
            if request.commitment.is_some() {
                return Err(StatusCode::BAD_REQUEST);
            }
            let envelope: OpenCompetitionCommitmentEnvelope =
                serde_json::from_value(request.commitment_envelope.ok_or(StatusCode::BAD_REQUEST)?)
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
            let envelope = validate_open_competition_commitment_envelope(
                &envelope,
                network,
                &bounty_state.bounty_contract,
                &wallet_state.wallet,
            )
            .map_err(|_| StatusCode::BAD_REQUEST)?;
            let committed_block = envelope.committed_block.ok_or(StatusCode::BAD_REQUEST)?;
            let reveal_deadline = envelope.reveal_deadline.ok_or(StatusCode::BAD_REQUEST)?;
            if wallet_state.safe_block_number <= committed_block
                || wallet_state.safe_block_timestamp > reveal_deadline
                || bounty_state.solver_entry_committed_block != Some(committed_block)
                || bounty_state.solver_entry_reveal_deadline != Some(reveal_deadline)
                || bounty_state
                    .solver_entry_commitment
                    .as_deref()
                    .is_none_or(|commitment| !commitment.eq_ignore_ascii_case(&envelope.commitment))
            {
                return Err(StatusCode::CONFLICT);
            }
            reveal_deadline_cap = Some(reveal_deadline);
            let proof = request.proof.ok_or(StatusCode::BAD_REQUEST)?;
            if proof.len() > 32_770 {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
            encode_open_competition_entrant_reveal_payload(
                &bounty_state.bounty_contract,
                &envelope.submission_hash,
                &envelope.evidence_hash,
                &envelope.salt,
                &proof,
            )
            .map_err(|_| StatusCode::BAD_REQUEST)?
        }
        OpenCompetitionEntrantAction::WithdrawBond => {
            if request.commitment.is_some()
                || request.commitment_envelope.is_some()
                || request.proof.is_some()
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            encode_open_competition_entrant_withdraw_payload(&bounty_state.bounty_contract)
                .map_err(|_| StatusCode::BAD_REQUEST)?
        }
    };
    let deadline_seconds = request.deadline_seconds.unwrap_or(300);
    if !(30..=600).contains(&deadline_seconds) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut deadline = wallet_state
        .safe_block_timestamp
        .checked_add(deadline_seconds)
        .ok_or(StatusCode::BAD_REQUEST)?
        .min(wallet_state.valid_until);
    if action == OpenCompetitionEntrantAction::Reveal {
        deadline = deadline
            .min(bounty_state.competition_ends_at)
            .min(reveal_deadline_cap.ok_or(StatusCode::BAD_REQUEST)?);
    }
    if deadline <= wallet_state.safe_block_timestamp.saturating_add(10) {
        return Err(StatusCode::CONFLICT);
    }
    plan_open_competition_entrant_action(
        network,
        &wallet_state.wallet,
        &wallet_state.delegate,
        &wallet_state.policy_hash,
        wallet_state.policy_version,
        action,
        wallet_state.delegate_nonce,
        deadline,
        &payload,
    )
    .map(Json)
    .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/base/open-competition-v1/entrant-action-relays",
    request_body = OpenCompetitionEntrantRelayRequest,
    responses(
        (status = 200, description = "Durable canonical entrant relay state"),
        (status = 202, description = "Relay queued, broadcasting, or awaiting a safe block"),
        (status = 400, description = "Malformed plan or signature"),
        (status = 409, description = "Idempotency or canonical state conflict"),
        (status = 429, description = "Bounded relay quota reached"),
        (status = 503, description = "Hosted relayer, database, or Base RPC unavailable")
    )
)]
async fn relay_open_competition_entrant_action(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<OpenCompetitionEntrantRelayRequest>,
) -> Result<Response, StatusCode> {
    if request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 128
        || !request
            .idempotency_key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_:".contains(character))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let plan: OpenCompetitionEntrantActionPlan =
        serde_json::from_value(request.plan.clone()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let public_relay = require_open_competition_entrant_relay_access(
        &state,
        &headers,
        &plan.network,
        plan.action,
    )?;
    let bounty_contract = open_competition_entrant_payload_bounty(plan.action, &plan.payload)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let (wallet_state, bounty_state) = observe_open_competition_entrant_context(
        &state,
        &plan.network,
        &plan.wallet,
        &bounty_contract,
    )
    .await?;
    validate_open_competition_entrant_action_state(
        &state,
        &wallet_state,
        &bounty_state,
        plan.action,
        public_relay,
    )
    .await?;
    if plan.deadline <= wallet_state.safe_block_timestamp
        || plan.deadline > wallet_state.safe_block_timestamp.saturating_add(600)
        || plan.deadline > wallet_state.valid_until
        || plan.nonce != wallet_state.delegate_nonce
        || plan.policy_version != wallet_state.policy_version
        || !plan
            .policy_hash
            .eq_ignore_ascii_case(&wallet_state.policy_hash)
        || !plan.delegate.eq_ignore_ascii_case(&wallet_state.delegate)
    {
        return Err(StatusCode::CONFLICT);
    }
    let rebuilt = plan_open_competition_entrant_action(
        &plan.network,
        &wallet_state.wallet,
        &wallet_state.delegate,
        &wallet_state.policy_hash,
        wallet_state.policy_version,
        plan.action,
        wallet_state.delegate_nonce,
        plan.deadline,
        &plan.payload,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    if serde_json::to_value(&rebuilt).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        != request.plan
    {
        return Err(StatusCode::CONFLICT);
    }
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|_| state.x402_relayer.enabled)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let intent = attach_open_competition_entrant_relay_signature(
        &rebuilt,
        &relayer.address(),
        &request.signature,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_open_competition_entrant_relay_intent(
        &intent,
        &wallet_state.wallet,
        &relayer.address(),
    )?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let request_fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&serde_json::json!({
            "plan": request.plan,
            "signature": request.signature
        }))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ));
    let relay = store
        .reserve_open_competition_entrant_relay(
            &NewOpenCompetitionEntrantRelay {
                id: Uuid::new_v4(),
                idempotency_key: request.idempotency_key,
                network: plan.network,
                wallet: wallet_state.wallet,
                bounty_contract,
                delegate: wallet_state.delegate,
                action: plan.action_code,
                wallet_nonce: plan.nonce,
                deadline: plan.deadline,
                payload_hash: plan.payload_hash,
                request_fingerprint,
                relayer_address: relayer.address(),
            },
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_open_competition_entrant_relay_db_error)?;
    let relay = process_open_competition_entrant_relay(&state, relay, &intent).await?;
    open_competition_entrant_relay_response(&relay)
}

#[utoipa::path(
    get,
    path = "/v1/base/open-competition-v1/entrant-action-relays/{relay_id}",
    params(("relay_id" = Uuid, Path, description = "Durable entrant relay ID")),
    responses(
        (status = 200, description = "Canonical entrant relay state"),
        (status = 202, description = "Relay awaiting a safe block"),
        (status = 404, description = "Relay not found")
    )
)]
async fn get_open_competition_entrant_relay(
    State(state): State<SharedState>,
    Path(relay_id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut relay = store
        .get_open_competition_entrant_relay(relay_id)
        .await
        .map_err(map_open_competition_entrant_relay_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if relay.status == OpenCompetitionEntrantRelayStatus::Broadcast {
        relay = reconcile_open_competition_entrant_relay(&state, relay).await?;
    }
    open_competition_entrant_relay_response(&relay)
}

#[utoipa::path(post, path = "/v1/base/open-competition-v1/status", request_body = OpenCompetitionActionRequest, responses((status = 200, description = "Canonical competition status read plan"), (status = 400, description = "Unknown network or malformed bounty address"), (status = 503, description = "Canonical safe-block state unavailable")))]
async fn get_open_competition_status(
    State(state): State<SharedState>,
    Json(request): Json<OpenCompetitionActionRequest>,
) -> Result<Json<OpenCompetitionActionPlan>, StatusCode> {
    let solver = request
        .arguments
        .get("solver")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    open_competition_action_from_safe_state(
        &state,
        request.network.as_deref().unwrap_or("base-mainnet"),
        &request.bounty_contract,
        solver.as_deref(),
        OpenCompetitionOperation::GetOpenCompetitionStatus,
        Some("competitionStatus"),
        request.arguments,
    )
    .await
    .map(Json)
}

#[utoipa::path(post, path = "/v1/base/open-competition-v1/bond-withdrawal-preparation", request_body = OpenCompetitionActionRequest, responses((status = 200, description = "Unsigned losing-entry bond withdrawal plan"), (status = 400, description = "Unknown network or malformed bounty address"), (status = 503, description = "Canonical safe-block state unavailable")))]
async fn withdraw_open_competition_bond(
    State(state): State<SharedState>,
    Json(request): Json<OpenCompetitionActionRequest>,
) -> Result<Json<OpenCompetitionActionPlan>, StatusCode> {
    let solver = request
        .arguments
        .get("solver")
        .and_then(|value| value.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    open_competition_action_from_safe_state(
        &state,
        request.network.as_deref().unwrap_or("base-mainnet"),
        &request.bounty_contract,
        Some(&solver),
        OpenCompetitionOperation::WithdrawOpenCompetitionBond,
        Some("withdrawEntryBond"),
        request.arguments,
    )
    .await
    .map(Json)
}

async fn open_competition_action_from_safe_state(
    state: &AppState,
    network: &str,
    bounty_contract: &str,
    solver: Option<&str>,
    operation: OpenCompetitionOperation,
    function: Option<&str>,
    arguments: serde_json::Value,
) -> Result<OpenCompetitionActionPlan, StatusCode> {
    let safe_state = observe_open_competition_state_for_api(
        state,
        &OpenCompetitionReadinessQuery {
            network: Some(network.to_string()),
            bounty_contract: Some(bounty_contract.to_string()),
            solver: solver.map(str::to_string),
            verifier_profile_id: None,
        },
    )
    .await?;
    let offchain_gates = open_competition_offchain_gates(state, &safe_state).await?;
    let readiness = open_competition_readiness_from_state(&safe_state, &offchain_gates);
    let mut plan = plan_open_competition_action(
        operation,
        &readiness,
        Some(safe_state.bounty_contract.clone()),
        function.map(str::to_string),
        arguments,
    );
    match operation {
        OpenCompetitionOperation::PrepareOpenCompetitionCommit => {
            let solver = solver.ok_or(StatusCode::BAD_REQUEST)?;
            let commitment = plan.arguments["commitment"]
                .as_str()
                .ok_or(StatusCode::BAD_REQUEST)?
                .to_string();
            attach_open_competition_commit_calls(
                &mut plan,
                &safe_state.settlement_token,
                &safe_state.bounty_contract,
                solver,
                &commitment,
                safe_state.entry_bond,
            )
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        }
        OpenCompetitionOperation::WithdrawOpenCompetitionBond => {
            attach_open_competition_withdrawal_call(
                &mut plan,
                &safe_state.bounty_contract,
                solver.ok_or(StatusCode::BAD_REQUEST)?,
            )
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        }
        OpenCompetitionOperation::PrepareOpenCompetitionReveal
        | OpenCompetitionOperation::GetOpenCompetitionStatus => {}
    }
    Ok(plan)
}

async fn observe_open_competition_state_for_api(
    state: &AppState,
    query: &OpenCompetitionReadinessQuery,
) -> Result<OpenCompetitionSafeState, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let bounty_contract = query
        .bounty_contract
        .as_deref()
        .ok_or(StatusCode::BAD_REQUEST)?;
    let release = open_competition_release_from_environment(network)?;
    let catalog = open_competition_verifier_catalog_from_environment(network)?;
    let profile = select_open_competition_verifier(&catalog, query.verifier_profile_id.as_deref())?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    tokio::time::timeout(
        Duration::from_secs(12),
        observe_open_competition_safe_state(
            &rpc_url,
            &OpenCompetitionStateQuery {
                release,
                bounty_contract: bounty_contract.to_string(),
                solver: query.solver.clone(),
                verifier_profile: profile,
            },
        ),
    )
    .await
    .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
    .map_err(|error| base_rpc_fetch_status(&error))
}

fn open_competition_release_from_environment(
    network: &str,
) -> Result<OpenCompetitionReleaseManifest, StatusCode> {
    let prefix = open_competition_environment_prefix(network)?;
    let raw = env::var(format!("{prefix}_RELEASE_MANIFEST_JSON"))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let release: OpenCompetitionReleaseManifest =
        serde_json::from_str(&raw).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if release.network != network {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(release)
}

fn open_competition_entrant_release_from_environment(
    network: &str,
) -> Result<OpenCompetitionEntrantWalletReleaseManifest, StatusCode> {
    let prefix = open_competition_environment_prefix(network)?;
    let raw = env::var(format!("{prefix}_ENTRANT_WALLET_RELEASE_MANIFEST_JSON"))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let release: OpenCompetitionEntrantWalletReleaseManifest =
        serde_json::from_str(&raw).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if release.network != network {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(release)
}

fn require_open_competition_entrant_relay_access(
    state: &SharedState,
    headers: &HeaderMap,
    network: &str,
    action: OpenCompetitionEntrantAction,
) -> Result<bool, StatusCode> {
    let prefix = open_competition_environment_prefix(network)?;
    open_competition_entrant_release_from_environment(network)?;
    if !state.x402_relayer.enabled || state.x402_relayer.relayer.is_none() || state.store.is_none()
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let public_relay = env_flag(&format!("{prefix}_RELAY_SUPPORT_AVAILABLE"))
        && env_flag(&format!("{prefix}_GAS_SPONSORSHIP_AVAILABLE"));
    if public_relay {
        return Ok(true);
    }
    let recovery_relay = action != OpenCompetitionEntrantAction::Commit
        && env_flag(&format!("{prefix}_ENTRANT_RECOVERY_RELAY_ENABLED"));
    if recovery_relay {
        return Ok(false);
    }
    if !env_flag(&format!("{prefix}_ENTRANT_RELAY_CANARY_ENABLED")) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    require_operator(state, headers)?;
    Ok(false)
}

async fn observe_open_competition_entrant_context(
    state: &SharedState,
    network: &str,
    wallet: &str,
    bounty_contract: &str,
) -> Result<
    (
        OpenCompetitionEntrantWalletSafeState,
        OpenCompetitionSafeState,
    ),
    StatusCode,
> {
    let competition_release = open_competition_release_from_environment(network)?;
    let entrant_release = open_competition_entrant_release_from_environment(network)?;
    let catalog = open_competition_verifier_catalog_from_environment(network)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    // Each observer pins all of its reads to one exact Base safe block. A safe
    // block can advance between the calls, so retry the pair once rather than
    // combining facts from different canonical snapshots.
    for _ in 0..2 {
        let wallet_state = tokio::time::timeout(
            Duration::from_secs(12),
            observe_open_competition_entrant_wallet_safe_state(
                &rpc_url,
                &entrant_release,
                &competition_release,
                wallet,
            ),
        )
        .await
        .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
        .map_err(|error| base_rpc_fetch_status(&error))?;
        let profile = select_open_competition_verifier_for_entrant(&catalog, &wallet_state)?;
        let bounty_state = tokio::time::timeout(
            Duration::from_secs(12),
            observe_open_competition_safe_state(
                &rpc_url,
                &OpenCompetitionStateQuery {
                    release: competition_release.clone(),
                    bounty_contract: bounty_contract.to_string(),
                    solver: Some(wallet_state.wallet.clone()),
                    verifier_profile: profile,
                },
            ),
        )
        .await
        .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
        .map_err(|error| base_rpc_fetch_status(&error))?;
        if wallet_state.safe_block_number == bounty_state.safe_block_number
            && wallet_state.safe_block_hash == bounty_state.safe_block_hash
        {
            return Ok((wallet_state, bounty_state));
        }
    }
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

async fn validate_open_competition_entrant_action_state(
    state: &SharedState,
    wallet: &OpenCompetitionEntrantWalletSafeState,
    bounty: &OpenCompetitionSafeState,
    action: OpenCompetitionEntrantAction,
    require_public_readiness: bool,
) -> Result<(), StatusCode> {
    if require_public_readiness && !wallet.deployment_state.permits_public_inventory() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let action_allowed = wallet.allowed_actions & (1_u8 << action.code()) != 0;
    let identity_ready = wallet.onchain_ready_to_relay
        && bounty.factory_registered_bounty
        && bounty.factory_runtime_matches
        && bounty.implementation_identity_matches
        && bounty.bounty_runtime_matches
        && bounty.settlement_token_matches
        && bounty.verifier_runtime_matches
        && bounty.verifier_commitments_match
        && bounty.fully_funded
        && wallet.competition_factory == bounty.factory_contract
        && wallet.settlement_token == bounty.settlement_token
        && wallet.verifier_module == bounty.verifier_module
        && wallet.verifier_policy_hash == bounty.policy_hash
        && wallet.acceptance_criteria_hash == bounty.acceptance_criteria_hash
        && wallet.benchmark_hash == bounty.benchmark_hash
        && wallet.evidence_schema_hash == bounty.evidence_schema_hash
        && wallet.max_bounty_target >= bounty.target_amount
        && action_allowed;
    if !identity_ready {
        return Err(StatusCode::CONFLICT);
    }
    match action {
        OpenCompetitionEntrantAction::Commit => {
            let current_bucket = wallet.safe_block_timestamp / wallet.period_seconds;
            let effective_period_spent = if current_bucket == wallet.period_bucket {
                wallet.period_spent
            } else {
                0
            };
            let spend_ready = bounty.onchain_ready_to_enter
                && wallet.token_balance >= bounty.entry_bond
                && wallet.max_per_action >= bounty.entry_bond
                && effective_period_spent
                    .checked_add(bounty.entry_bond)
                    .is_some_and(|spent| spent <= wallet.max_per_period)
                && wallet
                    .lifetime_spent
                    .checked_add(bounty.entry_bond)
                    .is_some_and(|spent| spent <= wallet.max_lifetime_spend);
            if !spend_ready {
                return Err(StatusCode::CONFLICT);
            }
            if require_public_readiness {
                let gates = open_competition_offchain_gates(state, bounty).await?;
                if !open_competition_readiness_from_state(bounty, &gates).ready_to_compete {
                    return Err(StatusCode::SERVICE_UNAVAILABLE);
                }
            }
        }
        OpenCompetitionEntrantAction::Reveal => {
            if bounty.status != 1
                || bounty.solver_has_entered != Some(true)
                || bounty.solver_entry_state != Some(1)
                || bounty
                    .solver_entry_commitment
                    .as_deref()
                    .is_none_or(|commitment| {
                        commitment[2..].chars().all(|character| character == '0')
                    })
                || bounty
                    .solver_entry_committed_block
                    .is_none_or(|committed| wallet.safe_block_number <= committed)
                || bounty
                    .solver_entry_reveal_deadline
                    .is_none_or(|deadline| wallet.safe_block_timestamp > deadline)
                || wallet.safe_block_timestamp > bounty.competition_ends_at
            {
                return Err(StatusCode::CONFLICT);
            }
        }
        OpenCompetitionEntrantAction::WithdrawBond => {
            if bounty.status != 2
                || bounty.solver_has_entered != Some(true)
                || bounty.solver_entry_state != Some(1)
                || bounty.solver_entry_bond.is_none_or(|bond| bond == 0)
            {
                return Err(StatusCode::CONFLICT);
            }
        }
    }
    Ok(())
}

fn validate_open_competition_entrant_relay_intent(
    intent: &EvmTransactionIntent,
    wallet: &str,
    relayer: &str,
) -> Result<(), StatusCode> {
    if intent.value_wei != 0
        || intent.function != "executeWithSignature(uint8,bytes,uint256,uint256,bytes)"
        || !intent.to.eq_ignore_ascii_case(wallet)
        || intent
            .from
            .as_deref()
            .is_none_or(|from| !from.eq_ignore_ascii_case(relayer))
        || intent.data.len() > 80_002
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok(())
}

async fn process_open_competition_entrant_relay(
    state: &SharedState,
    mut relay: OpenCompetitionEntrantRelay,
    intent: &EvmTransactionIntent,
) -> Result<OpenCompetitionEntrantRelay, StatusCode> {
    if relay.status == OpenCompetitionEntrantRelayStatus::Broadcast {
        return reconcile_open_competition_entrant_relay(state, relay).await;
    }
    if relay.status == OpenCompetitionEntrantRelayStatus::Confirmed
        || (relay.status == OpenCompetitionEntrantRelayStatus::Failed && !relay.retryable)
    {
        return Ok(relay);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&relay.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|_| state.x402_relayer.enabled)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(lease_token) = store
        .acquire_x402_relayer_lease(&relay.network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_open_competition_entrant_relay_db_error)?
    else {
        return store
            .get_open_competition_entrant_relay(relay.id)
            .await
            .map_err(map_open_competition_entrant_relay_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    };
    let claimed = store
        .claim_open_competition_entrant_relay(
            relay.id,
            lease_token,
            state.x402_relayer.lease_seconds,
        )
        .await
        .map_err(map_open_competition_entrant_relay_db_error)?;
    if claimed.is_none() {
        store
            .release_x402_relayer_lease(&relay.network, lease_token)
            .await
            .map_err(map_open_competition_entrant_relay_db_error)?;
        return store
            .get_open_competition_entrant_relay(relay.id)
            .await
            .map_err(map_open_competition_entrant_relay_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    }
    let relay_result = match tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            base_network_descriptor(&relay.network)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .chain_id,
            intent,
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(ChainBaseError::RelayerProvider(
            "entrant relay RPC deadline exceeded".to_string(),
        )),
    };
    let persisted = match relay_result {
        Ok(transaction) => {
            store
                .mark_open_competition_entrant_relay_broadcast(
                    relay.id,
                    lease_token,
                    &transaction.tx_hash,
                    transaction.estimated_gas,
                    transaction.gas_limit,
                )
                .await
        }
        Err(error) => {
            let retryable = x402_relay_error_is_retryable(&error);
            store
                .mark_open_competition_entrant_relay_failed(
                    relay.id,
                    Some(lease_token),
                    retryable,
                    "relay_rejected",
                    &error.to_string(),
                )
                .await
        }
    }
    .map_err(map_open_competition_entrant_relay_db_error);
    let release = store
        .release_x402_relayer_lease(&relay.network, lease_token)
        .await
        .map_err(map_open_competition_entrant_relay_db_error);
    relay = persisted?;
    release?;
    // Return the durable relay id as soon as the broadcast is persisted. A
    // Base safe block can take longer than the HTTP gateway timeout, and the
    // client already polls the status route for canonical confirmation.
    Ok(relay)
}

async fn reconcile_open_competition_entrant_relay(
    state: &SharedState,
    relay: OpenCompetitionEntrantRelay,
) -> Result<OpenCompetitionEntrantRelay, StatusCode> {
    tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        reconcile_open_competition_entrant_relay_inner(state, relay),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

async fn reconcile_open_competition_entrant_relay_inner(
    state: &SharedState,
    relay: OpenCompetitionEntrantRelay,
) -> Result<OpenCompetitionEntrantRelay, StatusCode> {
    if relay.status != OpenCompetitionEntrantRelayStatus::Broadcast {
        return Ok(relay);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let tx_hash = relay
        .tx_hash
        .as_deref()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&relay.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let response = fetch_transaction_receipt(&rpc_url, tx_hash, 1)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let Some(receipt) = response.result else {
        return Ok(relay);
    };
    if receipt.succeeded().map_err(|_| StatusCode::BAD_GATEWAY)? != Some(true) {
        return store
            .mark_open_competition_entrant_relay_failed(
                relay.id,
                None,
                false,
                "transaction_reverted",
                "entrant relay transaction reverted",
            )
            .await
            .map_err(map_open_competition_entrant_relay_db_error);
    }
    let receipt_block = receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .ok_or(StatusCode::BAD_GATEWAY)?;
    let receipt_block_hash = normalize_fixed_hex(
        receipt
            .block_hash
            .as_deref()
            .ok_or(StatusCode::BAD_GATEWAY)?,
        32,
    )?;
    let safe = fetch_safe_block_identity(&rpc_url, 2)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;
    if safe.number < receipt_block {
        return Ok(relay);
    }
    let exact = fetch_exact_block_identity(&rpc_url, receipt_block, 3)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;
    if exact.hash != receipt_block_hash {
        return Ok(relay);
    }
    let (canonical_event, payment_proven) =
        validate_open_competition_entrant_relay_receipt(&relay, &receipt)?;
    store
        .mark_open_competition_entrant_relay_confirmed(
            relay.id,
            receipt_block,
            &receipt_block_hash,
            safe.number,
            &safe.hash,
            canonical_event,
            payment_proven,
        )
        .await
        .map_err(map_open_competition_entrant_relay_db_error)
}

fn validate_open_competition_entrant_relay_receipt(
    relay: &OpenCompetitionEntrantRelay,
    receipt: &RpcTransactionReceipt,
) -> Result<(&'static str, bool), StatusCode> {
    let wallet_topic = format!("0x{}{}", "00".repeat(12), &relay.wallet[2..]);
    let delegate_topic = format!("0x{}{}", "00".repeat(12), &relay.delegate[2..]);
    let relayer_topic = format!("0x{}{}", "00".repeat(12), &relay.relayer_address[2..]);
    let action_topic = format!("0x{:064x}", relay.action);
    let action_event = event_topic("EntrantActionExecuted(uint8,address,address,uint256,bytes32)");
    let exact_action = receipt.logs.iter().any(|log| {
        if !log.address.eq_ignore_ascii_case(&relay.wallet)
            || log.topics.len() != 4
            || !log.topics[0].eq_ignore_ascii_case(&action_event)
            || !log.topics[1].eq_ignore_ascii_case(&action_topic)
            || !log.topics[2].eq_ignore_ascii_case(&delegate_topic)
            || !log.topics[3].eq_ignore_ascii_case(&relayer_topic)
        {
            return false;
        }
        let Some(data) = log.data.strip_prefix("0x") else {
            return false;
        };
        if data.len() != 128 {
            return false;
        }
        data[..48].chars().all(|character| character == '0')
            && u64::from_str_radix(&data[48..64], 16).ok() == Some(relay.wallet_nonce)
            && data[64..].eq_ignore_ascii_case(&relay.payload_hash[2..])
    });
    if !exact_action {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let has_bounty_event = |signature: &str, topic_count: usize, solver_topic: usize| {
        let topic = event_topic(signature);
        receipt.logs.iter().any(|log| {
            log.address.eq_ignore_ascii_case(&relay.bounty_contract)
                && log.topics.len() == topic_count
                && log.topics[0].eq_ignore_ascii_case(&topic)
                && log.topics[solver_topic].eq_ignore_ascii_case(&wallet_topic)
        })
    };
    match relay.action {
        0 if has_bounty_event(
            "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)",
            4,
            2,
        ) =>
        {
            Ok(("SolutionCommitted", false))
        }
        1 => {
            let settled = has_bounty_event(
                "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
                4,
                3,
            );
            let rejected = has_bounty_event(
                "CompetitionSubmissionRejected(bytes32,uint64,address,uint256,bytes32)",
                4,
                3,
            );
            match (settled, rejected) {
                (true, false) => Ok(("BountySettled", true)),
                (false, true) => Ok(("CompetitionSubmissionRejected", false)),
                _ => Err(StatusCode::BAD_GATEWAY),
            }
        }
        2 if has_bounty_event("EntryBondWithdrawn(bytes32,address,uint256)", 3, 2) => {
            Ok(("EntryBondWithdrawn", false))
        }
        _ => Err(StatusCode::BAD_GATEWAY),
    }
}

fn open_competition_entrant_relay_response(
    relay: &OpenCompetitionEntrantRelay,
) -> Result<Response, StatusCode> {
    let (status, status_name, next_action) = match relay.status {
        OpenCompetitionEntrantRelayStatus::Prepared => (
            StatusCode::ACCEPTED,
            "prepared",
            "Replay the same idempotency key and signed plan; do not sign another nonce.",
        ),
        OpenCompetitionEntrantRelayStatus::Relaying => (
            StatusCode::ACCEPTED,
            "relaying",
            "Poll this relay ID; do not sign or broadcast another action.",
        ),
        OpenCompetitionEntrantRelayStatus::Broadcast => (
            StatusCode::ACCEPTED,
            "broadcast",
            "Poll this relay ID until its receipt is canonical at a Base safe block.",
        ),
        OpenCompetitionEntrantRelayStatus::Confirmed => (
            StatusCode::OK,
            "confirmed",
            if relay.payment_proven {
                "Canonical BountySettled proves this solver payment."
            } else {
                "The canonical action is complete; only BountySettled is payment evidence."
            },
        ),
        OpenCompetitionEntrantRelayStatus::Failed => (
            StatusCode::OK,
            "failed",
            if relay.retryable {
                "Retry the same idempotency key and signed plan; do not advance the wallet nonce."
            } else {
                "Inspect canonical wallet state and prepare a new action only if the nonce did not advance."
            },
        ),
    };
    let mut response = (
        status,
        Json(OpenCompetitionEntrantRelayResponse {
            schema_version: "agent-bounties/open-competition-entrant-relay-v1".to_string(),
            id: relay.id,
            network: relay.network.clone(),
            wallet: relay.wallet.clone(),
            bounty_contract: relay.bounty_contract.clone(),
            action: relay.action,
            wallet_nonce: relay.wallet_nonce,
            status: status_name.to_string(),
            retryable: relay.retryable,
            transaction_hash: relay.tx_hash.clone(),
            receipt_block: relay.receipt_block,
            receipt_block_hash: relay.receipt_block_hash.clone(),
            canonical_safe_block: relay.canonical_safe_block,
            canonical_safe_block_hash: relay.canonical_safe_block_hash.clone(),
            canonical_event: relay.canonical_event.clone(),
            payment_proven: relay.payment_proven,
            next_action: next_action.to_string(),
            evidence_boundary: "A prepared or broadcast relay is not entry or payment evidence. Canonical action events prove execution; only canonical BountySettled proves solver payment.".to_string(),
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

fn map_open_competition_entrant_relay_db_error(error: DbError) -> StatusCode {
    match error {
        DbError::OpenCompetitionEntrantRelayConflict(_) => StatusCode::CONFLICT,
        DbError::OpenCompetitionEntrantRelayQuotaExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn open_competition_verifier_catalog_from_environment(
    network: &str,
) -> Result<OpenCompetitionVerifierCatalog, StatusCode> {
    let prefix = open_competition_environment_prefix(network)?;
    if let Ok(raw) = env::var(format!("{prefix}_VERIFIER_CATALOG_JSON")) {
        let catalog: OpenCompetitionVerifierCatalog =
            serde_json::from_str(&raw).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if catalog.network != network {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        return Ok(catalog);
    }
    built_in_open_competition_verifier_catalog(network).map_err(|_| StatusCode::BAD_REQUEST)
}

fn select_open_competition_verifier(
    catalog: &OpenCompetitionVerifierCatalog,
    profile_id: Option<&str>,
) -> Result<OpenCompetitionVerifierProfile, StatusCode> {
    match profile_id {
        Some(profile_id) => catalog
            .profiles
            .iter()
            .find(|profile| profile.profile_id == profile_id)
            .cloned()
            .ok_or(StatusCode::BAD_REQUEST),
        None if catalog.profiles.len() == 1 => Ok(catalog.profiles[0].clone()),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn select_open_competition_verifier_for_entrant(
    catalog: &OpenCompetitionVerifierCatalog,
    wallet: &OpenCompetitionEntrantWalletSafeState,
) -> Result<OpenCompetitionVerifierProfile, StatusCode> {
    let mut matches = catalog.profiles.iter().filter(|profile| {
        profile.network == wallet.network
            && profile.chain_id == wallet.chain_id
            && profile
                .verifier_address
                .eq_ignore_ascii_case(&wallet.verifier_module)
            && profile
                .runtime_code_hash
                .eq_ignore_ascii_case(&wallet.verifier_runtime_code_hash)
            && profile
                .benchmark_hash
                .eq_ignore_ascii_case(&wallet.benchmark_hash)
            && profile
                .evidence_schema_hash
                .eq_ignore_ascii_case(&wallet.evidence_schema_hash)
    });
    let profile = matches
        .next()
        .cloned()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if matches.next().is_some() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(profile)
}

async fn open_competition_offchain_gates(
    state: &AppState,
    safe_state: &OpenCompetitionSafeState,
) -> Result<OpenCompetitionOffchainGates, StatusCode> {
    let prefix = open_competition_environment_prefix(&safe_state.network)?;
    let monitoring_configured = env_flag(&format!("{prefix}_MONITORING_ACTIVE"));
    let monitoring_active = if monitoring_configured {
        match &state.store {
            Some(store) => store
                .get_base_indexer_heartbeat(&safe_state.network, &safe_state.factory_contract)
                .await
                .ok()
                .flatten()
                .is_some_and(|heartbeat| {
                    open_competition_monitoring_is_fresh(
                        &heartbeat,
                        safe_state.safe_block_number,
                        Utc::now(),
                    )
                }),
            None => false,
        }
    } else {
        false
    };
    let entrant_relay_runtime_available = state.store.is_some()
        && state.x402_relayer.enabled
        && state.x402_relayer.relayer.is_some()
        && open_competition_entrant_release_from_environment(&safe_state.network).is_ok();
    Ok(OpenCompetitionOffchainGates {
        gas_sponsorship_available: entrant_relay_runtime_available
            && env_flag(&format!("{prefix}_GAS_SPONSORSHIP_AVAILABLE")),
        relay_support_available: entrant_relay_runtime_available
            && env_flag(&format!("{prefix}_RELAY_SUPPORT_AVAILABLE")),
        r4_release_evidence_complete: env_flag(&format!("{prefix}_R4_EVIDENCE_COMPLETE")),
        monitoring_active,
    })
}

fn open_competition_monitoring_is_fresh(
    heartbeat: &BaseIndexerHeartbeat,
    safe_block_number: u64,
    now: DateTime<Utc>,
) -> bool {
    const MAX_HEARTBEAT_AGE_SECONDS: i64 = 90;
    const MAX_CURSOR_LAG_BLOCKS: u64 = 20;

    let status_healthy = heartbeat.status == "success"
        || (heartbeat.status == "skipped"
            && heartbeat.skipped_reason.as_deref()
                == Some("no confirmed blocks are ready to scan"));
    let Some(completed_at) = heartbeat.completed_at else {
        return false;
    };
    let age = now.signed_duration_since(completed_at).num_seconds();
    let latest_block_healthy = heartbeat
        .latest_block
        .is_some_and(|latest| latest.saturating_add(MAX_CURSOR_LAG_BLOCKS) >= safe_block_number);
    let cursor_healthy = heartbeat
        .persisted_cursor_block
        .is_some_and(|cursor| cursor.saturating_add(MAX_CURSOR_LAG_BLOCKS) >= safe_block_number);

    status_healthy
        && heartbeat.error_message.is_none()
        && (0..=MAX_HEARTBEAT_AGE_SECONDS).contains(&age)
        && latest_block_healthy
        && cursor_healthy
}

fn open_competition_hosted_operation_enabled(
    network: &str,
    operation: &str,
) -> Result<bool, StatusCode> {
    let prefix = open_competition_environment_prefix(network)?;
    Ok(env_flag(&format!("{prefix}_{operation}")))
}

fn open_competition_environment_prefix(network: &str) -> Result<&'static str, StatusCode> {
    match network {
        "base-mainnet" => Ok("BASE_MAINNET_OPEN_COMPETITION_V1"),
        "base-sepolia" => Ok("BASE_SEPOLIA_OPEN_COMPETITION_V1"),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

#[utoipa::path(
    get,
    path = "/v1/base/standing-meta-v4/readiness",
    params(("network" = Option<String>, Query, description = "base-mainnet or base-sepolia; defaults to base-mainnet")),
    responses(
        (status = 200, description = "Fail-closed Standing Meta V4 readiness report"),
        (status = 400, description = "Unknown Base network")
    )
)]
async fn get_standing_meta_v4_readiness(
    Query(query): Query<StandingMetaV4ReadinessQuery>,
) -> Result<Json<StandingMetaV4ReadinessReport>, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    standing_meta_v4_readiness_from_environment(network).map(Json)
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/claim-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned fail-closed V4 action plan"), (status = 400, description = "Unknown Base network")))]
async fn prepare_standing_meta_v4_claim(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::PrepareStandingMetaV4Claim,
        "PARENT_FACTORY",
        Some("claimAndCreateChild"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/stake-registration-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned stake-registration plan"), (status = 400, description = "Unknown Base network")))]
async fn prepare_anonymous_stake_registration(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::PrepareAnonymousStakeRegistration,
        "STAKE_POOL",
        Some("register"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/stake-availability-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned stake-availability plan"), (status = 400, description = "Unknown Base network")))]
async fn set_anonymous_stake_availability(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::SetAnonymousStakeAvailability,
        "STAKE_POOL",
        Some("setAvailability"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/verification-assignments", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Assignment-read plan"), (status = 400, description = "Unknown Base network")))]
async fn list_verification_assignments(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::ListVerificationAssignments,
        "APPEALABLE_VERIFIER",
        Some("caseParties"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/primary-verdict-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned primary-verdict plan"), (status = 400, description = "Unknown Base network")))]
async fn submit_primary_verdict(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::SubmitPrimaryVerdict,
        "APPEALABLE_VERIFIER",
        Some("submitPrimaryVerdict"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/appeal-waiver-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned immediate appeal-waiver plan"), (status = 400, description = "Unknown Base network")))]
async fn waive_verification_appeal(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::WaiveVerificationAppeal,
        "APPEALABLE_VERIFIER",
        Some("waiveAppeal"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/appeal-opening-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned appeal-opening plan"), (status = 400, description = "Unknown Base network")))]
async fn open_verification_appeal(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::OpenVerificationAppeal,
        "APPEALABLE_VERIFIER",
        Some("openAppeal"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/appeal-vote-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned appeal-vote plan"), (status = 400, description = "Unknown Base network")))]
async fn submit_appeal_vote(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::SubmitAppealVote,
        "APPEALABLE_VERIFIER",
        Some("submitAppealVote"),
    )
}

#[utoipa::path(post, path = "/v1/base/standing-meta-v4/finalization-preparation", request_body = StandingMetaV4ActionRequest, responses((status = 200, description = "Unsigned case-finalization plan"), (status = 400, description = "Unknown Base network")))]
async fn finalize_verification_case(
    Json(request): Json<StandingMetaV4ActionRequest>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    let function = match request
        .arguments
        .get("mode")
        .and_then(|value| value.as_str())
    {
        Some("unappealed") => Some("finalizeUnappealed"),
        Some("appeal") => Some("finalizeAppeal"),
        Some("timeout") => Some("timeoutAppeal"),
        _ => None,
    };
    standing_meta_v4_action_from_environment(
        request,
        StandingMetaV4Operation::FinalizeVerificationCase,
        "APPEALABLE_VERIFIER",
        function,
    )
}

fn standing_meta_v4_action_from_environment(
    request: StandingMetaV4ActionRequest,
    operation: StandingMetaV4Operation,
    component: &str,
    function: Option<&str>,
) -> Result<Json<StandingMetaV4ActionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let prefix = standing_meta_v4_environment_prefix(network)?;
    let readiness = standing_meta_v4_readiness_from_environment(network)?;
    let target = optional_evm_address(&format!("{prefix}_{component}"))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(plan_standing_meta_v4_action(
        operation,
        &readiness,
        target,
        function.map(str::to_string),
        request.arguments,
    )))
}

fn standing_meta_v4_readiness_from_environment(
    network: &str,
) -> Result<StandingMetaV4ReadinessReport, StatusCode> {
    let prefix = standing_meta_v4_environment_prefix(network)?;
    let components = [
        "PARENT_FACTORY",
        "CHILD_FACTORY",
        "STAKE_POOL",
        "VERIFIER_SORTITION",
        "SOLVER_SORTITION",
        "APPEALABLE_VERIFIER",
        "TERMS_REGISTRY",
    ];
    let canonical_components_configured = components.iter().all(|component| {
        optional_evm_address(&format!("{prefix}_{component}"))
            .ok()
            .flatten()
            .is_some()
    });
    let evidence = StandingMetaV4ReadinessEvidence {
        economics: StandingMetaV4EconomicsEvidence::default(),
        canonical_components_configured,
        valid_terms: env_flag(&format!("{prefix}_VALID_TERMS")),
        gas_sponsorship_available: env_flag(&format!("{prefix}_GAS_SPONSORSHIP_AVAILABLE")),
        vrf_subscription_funded: env_flag(&format!("{prefix}_VRF_SUBSCRIPTION_FUNDED")),
        vrf_consumers_authorized: env_flag(&format!("{prefix}_VRF_CONSUMERS_AUTHORIZED")),
        official_vrf_configuration_revalidated: env_flag(&format!(
            "{prefix}_VRF_CONFIGURATION_REVALIDATED"
        )),
        eligible_verifier_wallets: env_u64(&format!("{prefix}_ELIGIBLE_VERIFIER_WALLETS"), 0)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .try_into()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        eligible_child_solver_wallets_after_exclusions: env_u64(
            &format!("{prefix}_ELIGIBLE_CHILD_SOLVER_WALLETS"),
            0,
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .try_into()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        safe_timing: env_flag(&format!("{prefix}_SAFE_TIMING")),
        appeal_path_executable: env_flag(&format!("{prefix}_APPEAL_PATH_EXECUTABLE")),
        r4_release_evidence_complete: env_flag(&format!("{prefix}_R4_EVIDENCE_COMPLETE")),
        monitoring_active: env_flag(&format!("{prefix}_MONITORING_ACTIVE")),
    };
    Ok(standing_meta_v4_readiness(&evidence))
}

fn standing_meta_v4_environment_prefix(network: &str) -> Result<&'static str, StatusCode> {
    match network {
        "base-mainnet" => Ok("BASE_MAINNET_STANDING_META_V4"),
        "base-sepolia" => Ok("BASE_SEPOLIA_STANDING_META_V4"),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

type AgentWalletReadinessProblem = (StatusCode, Json<serde_json::Value>);

fn agent_wallet_readiness_problem(
    status: StatusCode,
    code: &str,
    retryable: bool,
    failed_transition: &str,
    message: &str,
    next_action: &str,
) -> AgentWalletReadinessProblem {
    if status.is_server_error() {
        eprintln!("agent wallet readiness failed: {code}");
    }
    (
        status,
        Json(serde_json::json!({
            "schema_version": "agent-bounties/agent-wallet-readiness-problem-v1",
            "state": "failed",
            "failed_transition": failed_transition,
            "error": code,
            "retryable": retryable,
            "message": message,
            "next_action": next_action,
            "evidence_boundary": "No readiness error is a claim, signature request, funding instruction, or settlement event."
        })),
    )
}

fn map_agent_wallet_readiness_error(error: ChainBaseError) -> AgentWalletReadinessProblem {
    match error {
        ChainBaseError::UnknownNetwork(_)
        | ChainBaseError::InvalidAddress(_)
        | ChainBaseError::InvalidAmount => agent_wallet_readiness_problem(
            StatusCode::BAD_REQUEST,
            "invalid_readiness_request",
            false,
            "validate_request_or_bounty",
            "The network, public address, canonical bounty, or expected bond is invalid.",
            "Refresh canonical earning inventory and retry with its network and bounty contract plus a valid public wallet address.",
        ),
        ChainBaseError::RelayerChainMismatch { .. } => agent_wallet_readiness_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "base_rpc_chain_mismatch",
            false,
            "verify_rpc_chain",
            "The configured RPC does not serve the requested Base network.",
            "Do not continue with this endpoint until hosted discovery and the configured RPC agree.",
        ),
        ChainBaseError::RpcHttpStatus(429) => agent_wallet_readiness_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "base_rpc_rate_limited",
            true,
            "read_canonical_state",
            "The Base RPC rate-limited the bounded readiness read.",
            "Retry with the same public inputs after a short delay; do not create parallel retries.",
        ),
        ChainBaseError::RpcProviderError { code, message }
            if code == -32016 || message.to_ascii_lowercase().contains("rate limit") =>
        {
            agent_wallet_readiness_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "base_rpc_rate_limited",
                true,
                "read_canonical_state",
                "The Base RPC rate-limited the bounded readiness read.",
                "Retry with the same public inputs after a short delay; do not create parallel retries.",
            )
        }
        ChainBaseError::InvalidRpcResponse(_) => agent_wallet_readiness_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "base_rpc_invalid_response",
            true,
            "decode_canonical_state",
            "The Base RPC returned a response that failed strict readiness validation.",
            "Refresh canonical inventory and retry once; if this persists, use another advertised RPC path.",
        ),
        _ => agent_wallet_readiness_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "base_rpc_unavailable",
            true,
            "read_canonical_state",
            "Canonical Base state could not be read.",
            "Retry with the same public inputs after a short delay; never replace them with wallet secrets.",
        ),
    }
}

fn live_money_readiness_config(state: &SharedState, network: &str) -> LiveMoneyReadinessConfig {
    service_runtime::live_money_readiness_config(
        network,
        LiveMoneyRuntimeSettings {
            stripe_secret_key: state.stripe_secret_key.as_deref(),
            stripe_live_execution_enabled: state.stripe_live_execution_enabled,
            stripe_payment_method_configuration_configured: state
                .stripe_payment_method_configuration
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            stripe_webhook_secret_configured: state.stripe_webhook_secret.is_some(),
            allow_unsigned_stripe_webhooks: state.allow_unsigned_stripe_webhooks,
            operator_auth_configured: state.operator_api_token.is_some(),
            base_rpc_url_configured: state.base_rpc_urls.resolve(network).is_ok(),
            base_broadcast_enabled: state.base_broadcast_enabled,
        },
    )
}

#[utoipa::path(get, path = "/v1/risk/events", responses((status = 200, body = Vec<RiskEvent>)))]
async fn list_risk_events(
    State(state): State<SharedState>,
    Query(filter): Query<RiskEventFilter>,
) -> Json<Vec<RiskEvent>> {
    let network = state.network.lock().expect("state poisoned");
    Json(network.list_risk_events(filter))
}

#[utoipa::path(get, path = "/v1/risk/reviews", responses((status = 200, body = Vec<RiskReviewRecord>)))]
async fn list_risk_reviews(State(state): State<SharedState>) -> Json<Vec<RiskReviewRecord>> {
    let network = state.network.lock().expect("state poisoned");
    Json(network.list_risk_reviews())
}

#[utoipa::path(
    post,
    path = "/v1/risk/bounty-approvals",
    responses(
        (status = 200, description = "Reviewed bounty approved into claimable state"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn approve_risk_bounty(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<ApproveRiskBountyRequest>,
) -> Result<Json<ReviewedBountyApproval>, StatusCode> {
    require_operator(&state, &headers)?;
    let approval =
        service_runtime::approve_risk_bounty(state.store.as_ref(), &state.network, request)
            .await
            .map_err(mutation_status)?;
    Ok(Json(approval))
}

#[utoipa::path(
    post,
    path = "/v1/risk/payout-approvals",
    responses(
        (status = 200, body = RiskReviewRecord),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn approve_risk_payout(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<ApproveRiskPayoutRequest>,
) -> Result<Json<RiskReviewRecord>, StatusCode> {
    require_operator(&state, &headers)?;
    let review =
        service_runtime::approve_risk_payout(state.store.as_ref(), &state.network, request)
            .await
            .map_err(mutation_status)?;
    Ok(Json(review))
}

#[utoipa::path(
    post,
    path = "/v1/risk/events/{id}/reject",
    responses(
        (status = 200, body = RiskReviewRecord),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn reject_risk_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut request): Json<RejectRiskEventRequest>,
) -> Result<Json<RiskReviewRecord>, StatusCode> {
    require_operator(&state, &headers)?;
    request.risk_event_id = id;
    let review = service_runtime::reject_risk_event(state.store.as_ref(), &state.network, request)
        .await
        .map_err(mutation_status)?;
    Ok(Json(review))
}

#[utoipa::path(post, path = "/v1/route-blocked-goal", request_body = RouteRequest, responses((status = 200, body = RouteDecision)))]
async fn route_blocked_goal(
    State(state): State<SharedState>,
    Json(request): Json<RouteRequest>,
) -> Json<RouteDecision> {
    let requester = Agent::new("api-requester");
    let help_request = HelpRequest::new(
        requester.id,
        request.goal,
        request.context,
        Money::new(request.budget_minor, request.currency).unwrap_or_else(|_| Money {
            amount: 1,
            currency: "usdc".to_string(),
        }),
        request.privacy,
    );

    let capabilities: Vec<Capability> = state
        .network
        .lock()
        .expect("state poisoned")
        .capabilities
        .values()
        .cloned()
        .collect();
    Json(BountyRouter::default().route_blocked_goal(&help_request, &capabilities))
}

#[utoipa::path(get, path = "/v1/evals/bountybench", responses((status = 200, body = EvalSuiteResult)))]
async fn run_bountybench(
    State(state): State<SharedState>,
) -> Result<Json<EvalSuiteResult>, StatusCode> {
    let result = BountyBench::default()
        .run(&bundled_fixtures())
        .expect("bundled fixtures pass");
    record_eval_run(&state, eval_run_from_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/abusebench", responses((status = 200, body = EvalSuiteResult)))]
async fn run_abusebench(
    State(state): State<SharedState>,
) -> Result<Json<EvalSuiteResult>, StatusCode> {
    let result = AbuseBench::default()
        .run(&bundled_abuse_fixtures())
        .expect("bundled abuse fixtures pass");
    record_eval_run(&state, eval_run_from_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/judgebench", responses((status = 200, body = EvalSuiteResult)))]
async fn run_judgebench(
    State(state): State<SharedState>,
) -> Result<Json<EvalSuiteResult>, StatusCode> {
    let result = JudgeBench::default()
        .run(&bundled_judge_fixtures())
        .expect("bundled judge fixtures pass");
    record_eval_run(&state, eval_run_from_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/loops", responses((status = 200, body = LoopSuiteResult)))]
async fn run_eval_loop_suite(
    State(state): State<SharedState>,
) -> Result<Json<LoopSuiteResult>, StatusCode> {
    let result = run_eval_loops().await.expect("bundled eval loops pass");
    record_eval_run(&state, eval_run_from_loop_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/runs", responses((status = 200, body = Vec<EvalRun>)))]
async fn list_eval_runs(State(state): State<SharedState>) -> Json<Vec<EvalRun>> {
    let runs = state.eval_runs.lock().expect("state poisoned").clone();
    Json(runs)
}

#[utoipa::path(post, path = "/v1/agents")]
async fn register_agent(
    State(state): State<SharedState>,
    Json(request): Json<RegisterAgentRequest>,
) -> Result<Json<domain::Agent>, StatusCode> {
    let agent = service_runtime::register_agent(state.store.as_ref(), &state.network, request)
        .await
        .map_err(mutation_status)?;
    Ok(Json(agent))
}

#[utoipa::path(get, path = "/v1/agents/{id}/paid-status")]
async fn agent_paid_status(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    let status = network
        .agent_payout_status(id)
        .map_err(|error| match error {
            app::AppError::AgentNotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_REQUEST,
        })?;
    let paid = status.payouts.iter().find(|payout| {
        payout.status == PayoutStatus::Paid && payout.rail != PaymentRail::Simulated
    });
    let evidence_payout = paid.or_else(|| status.payouts.first());
    let trigger = if paid.is_some() {
        Some(web_public::PostValueTrigger::ReconciledPayout)
    } else if evidence_payout.is_some() || !status.reputation_events.is_empty() {
        Some(web_public::PostValueTrigger::VerifiedCompletion)
    } else {
        None
    };
    let share_url = evidence_payout
        .map(|payout| {
            format!(
                "{}/public/proofs/{}",
                state.public_base_url.trim_end_matches('/'),
                payout.proof_record_id
            )
        })
        .unwrap_or_else(|| {
            format!(
                "{}/public/agents/{id}",
                state.public_base_url.trim_end_matches('/')
            )
        });
    let mut response =
        serde_json::to_value(status).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(object) = response.as_object_mut() {
        object.insert(
            "post_value_loop".to_string(),
            trigger
                .map(|trigger| {
                    serde_json::to_value(web_public::post_value_loop(
                        Some(trigger),
                        Some(&share_url),
                    ))
                })
                .transpose()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .unwrap_or(serde_json::Value::Null),
        );
    }
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/contributor-contacts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = ContributorContact),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn upsert_contributor_contact(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<UpsertContributorContactRequest>,
) -> Result<Json<ContributorContact>, StatusCode> {
    require_operator(&state, &headers)?;
    let contact = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .upsert_contributor_contact(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_contributor_contact(&contact)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(contact))
}

#[utoipa::path(
    get,
    path = "/v1/contributor-contacts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<ContributorContact>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_contributor_contacts(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ContributorContact>>, StatusCode> {
    require_operator(&state, &headers)?;
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_contributor_contacts()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/members",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = AudienceMember),
        (status = 400, description = "Invalid public identity record"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn upsert_audience_member(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<UpsertAudienceMemberRequest>,
) -> Result<Json<AudienceMember>, StatusCode> {
    require_operator(&state, &headers)?;
    let member = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .upsert_audience_member(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_audience_member(&member)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(member))
}

#[utoipa::path(
    get,
    path = "/v1/audience/members",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<AudienceMember>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_audience_members(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AudienceMember>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_audience_members()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_audience_members()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/interactions",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = AudienceInteraction),
        (status = 400, description = "Invalid or unknown audience interaction"),
        (status = 409, description = "Provider event ID conflicts with an immutable stored event"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn record_audience_interaction(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<RecordAudienceInteractionRequest>,
) -> Result<Json<AudienceInteraction>, StatusCode> {
    require_operator(&state, &headers)?;
    let (interaction, member) = {
        let mut network = state.network.lock().expect("state poisoned");
        let interaction = network
            .record_audience_interaction(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let member = network
            .audience_members
            .get(&interaction.audience_member_id)
            .cloned()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        (interaction, member)
    };
    if let Some(store) = &state.store {
        store
            .upsert_audience_interaction_with_member(&member, &interaction)
            .await
            .map_err(|error| match error {
                DbError::AudienceConflict(_) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            })?;
    }
    Ok(Json(interaction))
}

#[utoipa::path(
    get,
    path = "/v1/audience/interactions",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<AudienceInteraction>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_audience_interactions(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AudienceInteraction>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_audience_interactions()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_audience_interactions()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/discovery-responses",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = DiscoveryResponse),
        (status = 400, description = "Response lacks a public source or private-storage consent"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn record_discovery_response(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<RecordDiscoveryResponseRequest>,
) -> Result<Json<DiscoveryResponse>, StatusCode> {
    require_operator(&state, &headers)?;
    let response = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .record_discovery_response(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_discovery_response(&response)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/audience/discovery-responses",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<DiscoveryResponse>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_discovery_responses(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DiscoveryResponse>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_discovery_responses()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_discovery_responses()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/outreach-attempts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = OutreachAttempt),
        (status = 400, description = "Private outreach lacks explicit consent or public outreach lacks a URL"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn record_outreach_attempt(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<RecordOutreachAttemptRequest>,
) -> Result<Json<OutreachAttempt>, StatusCode> {
    require_operator(&state, &headers)?;
    let attempt = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .record_outreach_attempt(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_outreach_attempt(&attempt)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(attempt))
}

#[utoipa::path(
    get,
    path = "/v1/audience/outreach-attempts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<OutreachAttempt>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_outreach_attempts(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<OutreachAttempt>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_outreach_attempts()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_outreach_attempts()))
}

#[utoipa::path(
    get,
    path = "/v1/audience/report",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = AudienceReport),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn audience_report(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<AudienceReport>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        let members = store
            .list_audience_members()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let interactions = store
            .list_audience_interactions()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let responses = store
            .list_discovery_responses()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let attempts = store
            .list_outreach_attempts()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(Json(build_audience_report(
            &members,
            &interactions,
            &responses,
            &attempts,
        )));
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.audience_report()))
}

#[utoipa::path(
    post,
    path = "/v1/objectives/creation-plans",
    request_body = ObjectiveCreationDraft,
    responses(
        (status = 200, body = ObjectiveCreationPlan),
        (status = 400, description = "Invalid objective declaration or unsupported privacy claim")
    )
)]
async fn plan_objective_creation(
    Json(draft): Json<ObjectiveCreationDraft>,
) -> Result<Json<ObjectiveCreationPlan>, StatusCode> {
    Objective::plan_creation(draft)
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives",
    request_body = SignedObjectiveCreation,
    responses(
        (status = 200, body = ObjectiveView),
        (status = 400, description = "Invalid declaration or wallet approval"),
        (status = 409, description = "Objective id already exists or plan is stale")
    )
)]
async fn create_objective(
    State(state): State<SharedState>,
    Json(request): Json<SignedObjectiveCreation>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    let now = Utc::now();
    let objective = Objective::create(request, now).map_err(map_objective_error)?;
    persist_new_objective(&state, &objective).await?;
    Ok(Json(
        objective
            .view(&ObjectiveCanonicalEvidence::default(), now)
            .map_err(map_objective_error)?,
    ))
}

#[utoipa::path(
    get,
    path = "/v1/objectives",
    responses((status = 200, body = Vec<ObjectiveView>))
)]
async fn list_objectives(
    State(state): State<SharedState>,
) -> Result<Json<Vec<ObjectiveView>>, StatusCode> {
    let objectives = load_objectives(&state).await?;
    let evidence = load_objective_canonical_evidence(&state, &objectives).await?;
    let now = Utc::now();
    objectives
        .iter()
        .map(|objective| objective.view(&evidence, now).map_err(map_objective_error))
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/v1/objectives/{id}",
    params(("id" = Uuid, Path, description = "Objective id")),
    responses(
        (status = 200, body = ObjectiveView),
        (status = 404, description = "Objective not found")
    )
)]
async fn get_objective(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    let objective = load_objective(&state, id).await?;
    let evidence =
        load_objective_canonical_evidence(&state, std::slice::from_ref(&objective)).await?;
    objective
        .view(&evidence, Utc::now())
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives/{id}/action-plans",
    params(("id" = Uuid, Path, description = "Objective id")),
    request_body = ObjectiveAction,
    responses(
        (status = 200, body = ObjectiveActionPlan),
        (status = 404, description = "Objective or referenced record not found"),
        (status = 409, description = "Action is invalid in the current state")
    )
)]
async fn plan_objective_action(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(action): Json<ObjectiveAction>,
) -> Result<Json<ObjectiveActionPlan>, StatusCode> {
    let objective = load_objective(&state, id).await?;
    objective
        .plan_action(action, Utc::now())
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives/{id}/actions",
    params(("id" = Uuid, Path, description = "Objective id")),
    request_body = SignedObjectiveAction,
    responses(
        (status = 200, body = ObjectiveView),
        (status = 400, description = "Invalid wallet approval"),
        (status = 404, description = "Objective or referenced record not found"),
        (status = 409, description = "Stale revision, invalid transition, or unmet readiness requirement")
    )
)]
async fn apply_objective_action(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SignedObjectiveAction>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    if request.plan.objective_id != id {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut objective = load_objective(&state, id).await?;
    let expected_revision = objective.revision;
    let evidence =
        load_objective_canonical_evidence(&state, std::slice::from_ref(&objective)).await?;
    let now = Utc::now();
    objective
        .apply_action(request, now, &evidence)
        .map_err(map_objective_error)?;
    persist_objective_replacement(&state, &objective, expected_revision).await?;
    objective
        .view(&evidence, now)
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives/{id}/reconcile",
    params(("id" = Uuid, Path, description = "Objective id")),
    responses(
        (status = 200, body = ObjectiveView, description = "Objective refreshed only from confirmed canonical bounty evidence"),
        (status = 404, description = "Objective not found"),
        (status = 409, description = "Concurrent objective update; reload and retry")
    )
)]
async fn reconcile_objective(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    let mut objective = load_objective(&state, id).await?;
    let expected_revision = objective.revision;
    let evidence =
        load_objective_canonical_evidence(&state, std::slice::from_ref(&objective)).await?;
    let now = Utc::now();
    if objective
        .reconcile_canonical_evidence(&evidence, now)
        .map_err(map_objective_error)?
    {
        persist_objective_replacement(&state, &objective, expected_revision).await?;
    }
    objective
        .view(&evidence, now)
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(post, path = "/v1/capabilities")]
async fn register_capability(
    State(state): State<SharedState>,
    Json(request): Json<RegisterCapabilityRequest>,
) -> Result<Json<domain::Capability>, StatusCode> {
    let capability =
        service_runtime::register_capability(state.store.as_ref(), &state.network, request)
            .await
            .map_err(mutation_status)?;
    Ok(Json(capability))
}

#[utoipa::path(post, path = "/v1/help-requests")]
async fn create_help_request(
    State(state): State<SharedState>,
    Json(request): Json<CreateHelpRequestRequest>,
) -> Result<Json<domain::HelpRequest>, StatusCode> {
    let help_request =
        service_runtime::create_help_request(state.store.as_ref(), &state.network, request)
            .await
            .map_err(mutation_status)?;
    Ok(Json(help_request))
}

#[utoipa::path(post, path = "/v1/help-requests/{id}/quotes")]
async fn request_quotes(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<QuoteSet>, StatusCode> {
    let quote_set = service_runtime::request_quotes(
        state.store.as_ref(),
        &state.network,
        RequestQuotesRequest {
            help_request_id: id,
        },
    )
    .await
    .map_err(mutation_status)?;
    Ok(Json(quote_set))
}

#[utoipa::path(post, path = "/v1/quotes/{id}/fund-bounty")]
async fn fund_quote(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<FundQuoteRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    request.quote_id = id;
    let bounty =
        service_runtime::fund_quote_as_bounty(state.store.as_ref(), &state.network, request)
            .await
            .map_err(mutation_status)?;
    Ok(Json(bounty))
}

#[utoipa::path(get, path = "/v1/bounties/claimable")]
async fn list_claimable_bounties(State(state): State<SharedState>) -> Json<Vec<domain::Bounty>> {
    let network = state.network.lock().expect("state poisoned");
    Json(network.list_claimable_bounties())
}

#[utoipa::path(get, path = "/v1/bounties/feed", responses((status = 200, description = "Public claimable bounty feed")))]
async fn public_bounty_feed(
    State(state): State<SharedState>,
) -> Json<Vec<web_public::PublicBountyFeedItem>> {
    let bounties = {
        let network = state.network.lock().expect("state poisoned");
        network.list_claimable_bounties()
    };
    Json(web_public::public_bounty_feed(
        &bounties,
        &state.public_base_url,
    ))
}

#[utoipa::path(get, path = "/v1/bounties/funding-feed", responses((status = 200, description = "Public bounties that still need funding")))]
async fn public_funding_feed(
    State(state): State<SharedState>,
) -> Json<Vec<web_public::PublicFundingFeedItem>> {
    let items = {
        let network = state.network.lock().expect("state poisoned");
        public_funding_feed_items(&network, &state.public_base_url)
    };
    Json(items)
}

#[utoipa::path(
    get,
    path = "/v1/x402/base/bounties/{bounty_contract}/funding",
    params(
        ("bounty_contract" = String, Path, description = "Canonical autonomous-v1 bounty contract"),
        ("network" = Option<String>, Query, description = "base-mainnet or base-sepolia"),
        ("amount" = Option<u64>, Query, description = "USDC base units; defaults to the remaining funding gap"),
        ("relayer" = Option<String>, Query, description = "Optional gas-paying Base address used in the returned transaction intent")
    ),
    responses(
        (status = 200, description = "Canonical FundingAdded confirmed; PAYMENT-RESPONSE contains the x402 settlement result"),
        (status = 202, description = "x402 envelope validated; the contract still verifies the EIP-3009 signature when the relay transaction is broadcast"),
        (status = 402, description = "PAYMENT-REQUIRED contains the exact x402 v2 funding challenge"),
        (status = 404, description = "Canonical indexed bounty not found"),
        (status = 409, description = "Bounty cannot accept the requested contribution"),
        (status = 413, description = "Requested amount exceeds the hosted relay cap"),
        (status = 422, description = "Authorization or hosted relay policy is invalid"),
        (status = 429, description = "Rolling hosted relay quota is exhausted"),
        (status = 503, description = "Hosted relayer or canonical RPC is temporarily unavailable")
    )
)]
async fn x402_base_bounty_funding(
    State(state): State<SharedState>,
    Path(bounty_contract): Path<String>,
    Query(query): Query<X402FundingQuery>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let requested_network = query.network.as_deref().unwrap_or("base-mainnet");
    let network =
        base_network_descriptor(requested_network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let network_key = match network.chain_id {
        8_453 => "base-mainnet",
        84_532 => "base-sepolia",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let caip2_network = format!("eip155:{}", network.chain_id);
    let bounty_contract =
        normalize_evm_address(&bounty_contract).map_err(|_| StatusCode::BAD_REQUEST)?;
    let item = indexed_autonomous_bounty(&state, network_key, &bounty_contract).await?;
    if !item.terms_valid {
        return Err(StatusCode::CONFLICT);
    }
    let amount = resolve_x402_funding_amount(
        &item.status,
        &item.target_amount,
        &item.funded_amount,
        query.amount,
    )?;
    if state.x402_relayer.enabled && amount > state.x402_relayer.max_amount {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    if state.x402_relayer.enabled && amount < state.x402_relayer.min_amount {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let requested_relayer = query
        .relayer
        .as_deref()
        .map(normalize_evm_address)
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let hosted_relayer = state.x402_relayer.address();
    if state.x402_relayer.enabled
        && requested_relayer.as_deref().is_some_and(|requested| {
            hosted_relayer
                .as_deref()
                .is_none_or(|hosted| !requested.eq_ignore_ascii_case(hosted))
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let relayer = hosted_relayer.or(requested_relayer);
    let mut resource_url = format!(
        "{}/v1/x402/base/bounties/{}/funding?network={network_key}&amount={amount}",
        state.public_base_url.trim_end_matches('/'),
        bounty_contract
    );
    if let Some(relayer) = &relayer {
        resource_url.push_str("&relayer=");
        resource_url.push_str(relayer);
    }
    let challenge = base_usdc_funding_challenge(
        resource_url,
        caip2_network,
        &network.native_usdc_token_address,
        &bounty_contract,
        amount,
        300,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(header) = headers.get(PAYMENT_SIGNATURE_HEADER) else {
        return x402_payment_required_response(challenge);
    };
    let payload = match header
        .to_str()
        .map_err(|_| payments_x402::X402Error::InvalidBase64)
        .and_then(decode_payment_signature_header)
    {
        Ok(payload) => payload,
        Err(error) => return x402_payment_required_error(challenge, &error.to_string()),
    };
    let now =
        u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let authorization = match validate_funding_payload(&payload, &challenge, now) {
        Ok(authorization) => authorization,
        Err(error) => return x402_payment_required_error(challenge, &error.to_string()),
    };
    let contribution = AutonomousBountyContribution {
        bounty_contract: authorization.bounty_contract.clone(),
        contributor: authorization.contributor.clone(),
        amount: Money::new(authorization.amount as i64, "usdc")
            .map_err(|_| StatusCode::BAD_REQUEST)?,
        authorization_nonce: Some(authorization.nonce.clone()),
        authorization_valid_before: Some(authorization.valid_before),
    };
    let signature = AutonomousBountyAuthorizationSignature {
        v: authorization.v,
        r: authorization.r.clone(),
        s: authorization.s.clone(),
    };
    let plan = configured_autonomous_planner(network_key)?
        .plan_authorized_contribution(network_key, &contribution, &signature, relayer.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if !state.x402_relayer.enabled {
        return x402_self_relay_response(&authorization, plan);
    }
    validate_hosted_x402_intent(
        &plan.relay_transaction,
        relayer.as_deref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
        &bounty_contract,
    )?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let request_fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ));
    let attempt = store
        .reserve_x402_relay_attempt(
            &NewX402RelayAttempt {
                id: Uuid::new_v4(),
                idempotency_key: format!(
                    "x402:{network_key}:{}:{}",
                    authorization.bounty_contract, authorization.nonce
                ),
                network: network_key.to_string(),
                bounty_contract: authorization.bounty_contract,
                contributor: authorization.contributor,
                amount: authorization.amount,
                authorization_nonce: authorization.nonce,
                authorization_valid_before: authorization.valid_before,
                request_fingerprint,
                relayer_address: relayer.ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
            },
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    let attempt = process_x402_hosted_relay(&state, attempt, &plan.relay_transaction).await?;
    x402_relay_response(&state, &attempt)
}

#[utoipa::path(
    get,
    path = "/v1/x402/base/relays/{relay_id}",
    params(("relay_id" = Uuid, Path, description = "Durable hosted x402 relay attempt ID")),
    responses(
        (status = 200, description = "Canonical FundingAdded confirmed"),
        (status = 202, description = "Relay is queued, broadcasting, or awaiting confirmation"),
        (status = 404, description = "Relay attempt not found")
    )
)]
async fn get_x402_relay(
    State(state): State<SharedState>,
    Path(relay_id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut attempt = store
        .get_x402_relay_attempt(relay_id)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if attempt.status == X402RelayStatus::Broadcast {
        attempt = reconcile_x402_relay(&state, attempt).await?;
    }
    x402_relay_response(&state, &attempt)
}

fn x402_self_relay_response(
    authorization: &payments_x402::ValidatedFundingAuthorization,
    plan: chain_base::AutonomousBountyAuthorizedContributionPlan,
) -> Result<Response, StatusCode> {
    let mut response = (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "x402Version": X402_VERSION,
            "scheme": AGENT_BOUNTY_FUND_SCHEME,
            "status": "self_relay_required",
            "settled": false,
            "contributor": authorization.contributor,
            "bountyContract": authorization.bounty_contract,
            "amount": authorization.amount.to_string(),
            "authorizationNonce": authorization.nonce,
            "plan": plan,
            "nextStep": "Hosted relay is disabled. Simulate and broadcast plan.relay_transaction from the chosen gas-paying Base wallet, then wait for confirmed canonical FundingAdded.",
            "canonicalSuccessEvent": "FundingAdded",
            "paymentResponseHeaderPresent": false
        })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

fn validate_hosted_x402_intent(
    intent: &EvmTransactionIntent,
    relayer: &str,
    bounty_contract: &str,
) -> Result<(), StatusCode> {
    if intent.value_wei != 0
        || intent.function != AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION
        || !intent.to.eq_ignore_ascii_case(bounty_contract)
        || intent
            .from
            .as_deref()
            .is_none_or(|from| !from.eq_ignore_ascii_case(relayer))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let calldata = intent
        .data
        .strip_prefix("0x")
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    if calldata.len() != 8 + (8 * 64)
        || !calldata.starts_with(AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok(())
}

async fn process_x402_hosted_relay(
    state: &SharedState,
    mut attempt: X402RelayAttempt,
    intent: &EvmTransactionIntent,
) -> Result<X402RelayAttempt, StatusCode> {
    if attempt.status == X402RelayStatus::Broadcast {
        return reconcile_x402_relay(state, attempt).await;
    }
    if attempt.status == X402RelayStatus::Confirmed
        || (attempt.status == X402RelayStatus::Failed && !attempt.retryable)
    {
        return Ok(attempt);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&attempt.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(lease_token) = store
        .acquire_x402_relayer_lease(&attempt.network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?
    else {
        return store
            .get_x402_relay_attempt(attempt.id)
            .await
            .map_err(map_x402_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    };
    let claimed = store
        .claim_x402_relay_attempt(attempt.id, lease_token, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?;
    let Some(_claimed) = claimed else {
        store
            .release_x402_relayer_lease(&attempt.network, lease_token)
            .await
            .map_err(map_x402_db_error)?;
        return store
            .get_x402_relay_attempt(attempt.id)
            .await
            .map_err(map_x402_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    };
    let relay_result = match tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            base_network_descriptor(&attempt.network)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .chain_id,
            intent,
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(ChainBaseError::RelayerProvider(
            "relay RPC deadline exceeded".to_string(),
        )),
    };
    let persisted_result = match relay_result {
        Ok(transaction) => store
            .mark_x402_relay_broadcast(
                attempt.id,
                lease_token,
                &transaction.tx_hash,
                transaction.estimated_gas,
                transaction.gas_limit,
            )
            .await
            .map_err(map_x402_db_error),
        Err(error) => {
            let retryable = x402_relay_error_is_retryable(&error);
            store
                .mark_x402_relay_failed(
                    attempt.id,
                    Some(lease_token),
                    retryable,
                    "relay_rejected",
                    &error.to_string(),
                )
                .await
                .map_err(map_x402_db_error)
        }
    };
    let release_result = store
        .release_x402_relayer_lease(&attempt.network, lease_token)
        .await
        .map_err(map_x402_db_error);
    attempt = persisted_result?;
    release_result?;
    if attempt.status != X402RelayStatus::Broadcast {
        r×~µ×fòµë(š+myÖGW&&ÆR6Æ–Ò6æF–FFRF—6V&VB&Vf÷&R&VÆ’"À¢$Fòæ÷B6–vâ÷"gVæBv–ã²&W÷'BF†R6æF–FFR”Bâ"À¢¢Ò“ó°¢ÆWBGW&&ÆU÷7öç6÷'6†—Ò7F÷&P¢ævWEö&öæE÷7öç6÷'6†—öf÷%ö6æF–FFR†6æF–FFRæ–B¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ð¢æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤4ôädÄ”5BÀ¢'7öç6÷'6†—öÖ—76–ær"À¢'&VÆ•öFöÖ–5ö6Æ–Ò"À¢'F†RGW&&ÆR7öç6÷'6†—F—6V&VB&Vf÷&R&VÆ’"À¢$Fòæ÷B6–vâ÷"gVæBv–ã²&W÷'BF†R6æF–FFR”Bâ"À¢¢Ò“ó°¢ö²‚†GW&&ÆUö6æF–FFRÂGW&&ÆU÷7öç6÷'6†—’¢Ð¢æv—C°¢ÆWB†GW&&ÆUö6æF–FFRÂGW&&ÆU÷7öç6÷'6†—’ÒÖF6‚GW&&ÆU÷7FFR°¢ö²‡7FFR’Óâ7FFRÀ¢W'"‡&ö&ÆVÒ’Óâ°¢ÆWBòÒ7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—C°¢&WGW&âW'"‡&ö&ÆVÒ“°¢Ð¢Ó°¢–bGW&&ÆUö6æF–FFRç7FGW2ÓÒ6Æ–Ô6æF–FFU7FGW3£¤6Æ–ÖVB°¢7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢&WGW&âö²†GW&&ÆUö6æF–FFR“°¢Ð¢–bGW&&ÆUö6æF–FFRç7FGW2ÓÒ6Æ–Ô6æF–FFU7FGW3£¥&VÆ––æp¢bbGW&&ÆU÷7öç6÷'6†—ç7FGW2ÓÒ&öæE7öç6÷'6†—7FGW3£¤'&öF67@¢°¢7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢&WGW&â&V6öæ6–ÆUövVçEöæF—fUö6Æ–Ò‡7FFRÂGW&&ÆUö6æF–FFRÂ6Æ–Õö&öæB’æv—C°¢Ð¢–bGW&&ÆU÷7öç6÷'6†—ç7FGW2Ò&öæE7öç6÷'6†—7FGW3£¥&W6W'fV@¢ÇÂÖF6†W2€¢GW&&ÆUö6æF–FFRç7FGW2À¢6Æ–Ô6æF–FFU7FGW3£¤W†6ÇW6—fP¢Â6Æ–Ô6æF–FFU7FGW3£¥7öç6÷&–æp¢Â6Æ–Ô6æF–FFU7FGW3£¤WF†÷&—¦F–öå&VG¢¢°¢7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤4ôädÄ”5BÀ¢'7öç6÷'6†—÷7FFUö6†ævVB"À¢'&VÆ•öFöÖ–5ö6Æ–Ò"À¢'F†RGW&&ÆR6æF–FFR÷"7öç6÷'6†—6†ævVB&Vf÷&R&VÆ’"À¢%&WÆ’F†R6ÖR&WVW7C²Fòæ÷B6–vâ÷"gVæBv–ââ"À¢’“°¢Ð¢ÆWB&VÆ•÷&W7VÇBÒFö¶–ó£§F–ÖS£§F–ÖV÷WB€¢GW&F–öã£¦g&öÕ÷6V72‡7FFRæ&öæE÷7öç6÷"ç'5÷F–ÖV÷WE÷6V6öæG2’À¢&VÆ–W"ç6–×VÆFUöæEö'&öF67B€¢g'5÷W&ÂÀ¢FW67&—F÷"æ6†–åö–BÀ¢gÆâç&VÆ•÷G&ç67F–öâÀ¢7FFRæ&öæE÷7öç6÷"æÖ…öv2À¢7FFRæ&öæE÷7öç6÷"æÖ…öfVU÷W%öv5÷vV’À¢’À¢¢æv—C°¢ÆWBG&ç67F–öâÒÖF6‚&VÆ•÷&W7VÇB°¢ö²„ö²‡G&ç67F–öâ’’ÓâG&ç67F–öâÀ¢ö²„W'"…ò’’Óâ°¢7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢&FöÖ–5ö6Æ–Õö'&öF67E÷Væ¶æ÷vâ"À¢'&VÆ•öFöÖ–5ö6Æ–Ò"À¢'F†RFöÖ–26Æ–Òv2æ÷B&WGW&æVB2&V6÷&FVB'&öF67B"À¢%&WG'’F†R6ÖR&WVW7BæB6–væGW&W3²F†Rw&çBæBU4D2æöæ6W2&WfVçBGWÆ–6FRW6Râ"À¢’“°¢Ð¢W'"…ò’Óâ°¢7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢&FöÖ–5ö6Æ–Õ÷'5÷F–ÖV÷WB"À¢'&VÆ•öFöÖ–5ö6Æ–Ò"À¢'F†R&VÆ’%2FVFÆ–æRVÆ6VBv—F†÷WB&WGW&æ–ærG&ç67F–öâ†6‚"À¢%&WG'’F†R6ÖR&WVW7BæB6–væGW&W3²Fòæ÷B7&VFRæ÷F†W"7öç6÷'6†—â"À¢’“°¢Ð¢Ó°¢ÆWBÖ&µ÷&W7VÇBÒ7F÷&P¢æÖ&µöFöÖ–5÷7öç6÷&VEö6Æ–Õö'&öF67B†6æF–FFRæ–BÂ7öç6÷'6†—æ–BÂgG&ç67F–öâçG…ö†6‚¢æv—C°¢ÆWB&VÆV6U÷&W7VÇBÒ7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—C°¢ÆWB†6æF–FFRÂò’ÒÖF6‚Ö&µ÷&W7VÇB°¢ö²‡fÇVR’ÓâfÇVRÀ¢W'"†W'&÷"’Óâ°¢ÆWBòÒ&VÆV6U÷&W7VÇC°¢&WGW&âW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"†W'&÷"’“°¢Ð¢Ó°¢&VÆV6U÷&W7VÇBæÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢&V6öæ6–ÆUövVçEöæF—fUö6Æ–Ò‡7FFRÂ6æF–FFRÂ6Æ–Õö&öæB’æv—@§Ð ¦fâæW‡Eö6Æ–Õ÷&÷VæB†—FVÓ¢dWFöæöÖ÷W4&÷VçG”fVVD—FVÒ’Óâ&W7VÇCÇScBÂvVçD6Æ–Õ&ö&ÆVÓâ°¢—FVÒæWfVçG0¢æ—FW"‚¢æf–ÇFW%öÖ‡ÆWfVçGÂWfVçBæFFævWB‚'&÷VæB"’ææE÷F†Vâ‡6W&FUö§6öã£¥fÇVS£¦5÷ScB’¢æÖ‚‚¢çVçw&ö÷"ƒ¢æ6†V6¶VEöFBƒ¢æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢'&÷VæEö÷fW&fÆ÷r"À¢'&VÆ•öFöÖ–5ö6Æ–Ò"À¢'F†R6æöæ–6Â&÷VçG’&÷VæB6ææ÷BGfæ6R6fVÇ’"À¢$Fòæ÷B6–vâ÷"'&öF67C²&W÷'BF†R&÷VçG’6öçG&7Bâ"À¢¢Ò§Ð ¦fâFöÖ–5÷7öç6÷'6†—öw&çEöæöæ6R‡7öç6÷'6†—¢d&öæE7öç6÷'6†—’Óâ7G&–ær°¢ÆWB×WB†6†W"Ò6†#Sc£¦æWr‚“°¢†6†W"çWFFR†"&vVçBÖ&÷VçF–W2öFöÖ–2×7öç6÷"Öw&çB×c"“°¢†6†W"çWFFR‡7öç6÷'6†—æ–Bæ5ö'—FW2‚’“°¢†6†W"çWFFR‡7öç6÷'6†—æ6Æ–Õö6æF–FFUö–Bæ5ö'—FW2‚’“°¢†6†W"çWFFR‡7öç6÷'6†—ææWGv÷&²æ5ö'—FW2‚’“°¢†6†W"çWFFR‡7öç6÷'6†—æ&÷VçG•ö6öçG&7Bæ5ö'—FW2‚’“°¢†6†W"çWFFR‡7öç6÷'6†—ç6öÇfW%÷vÆÆWBæ5ö'—FW2‚’“°¢f÷&ÖB‚#‡·Ò"Â†Wƒ£¦Væ6öFR††6†W"æf–æÆ—¦R‚’’§Ð ¦fâfÆ–FFUöFöÖ–5÷7öç6÷&VEö6Æ–Õö–çFVçB€¢–çFVçC¢dWfÕG&ç67F–öä–çFVçBÀ¢7öç6÷%ö6öçG&7C¢g7G"À¢&VÆ–W#¢g7G"À¢’Óâ&W7VÇCÂ‚’ÂvVçD6Æ–Õ&ö&ÆVÓâ°¢ÆWB6ÆÆFFÒ–çFVçBæFFç7G&—÷&Vf—‚‚#‚"’çVçw&ö÷%öFVfVÇB‚“°¢–b–çFVçBçfÇVU÷vV’Ò ¢ÇÂ–çFVçBægVæ7F–öà¢Ò'7öç6÷$æD6Æ–Ò‚†FG&W72ÆFG&W72ÇV–çCcBÇV–çC#SbÆ'—FW33"Æ'—FW33"Æ'—FW33"ÇV–çC#SbÇV–çC#SbÆ'—FW33"ÇV–çC#Sb’Æ'—FW2ÇV–çC‚Æ'—FW33"Æ'—FW33"’ ¢ÇÂ–çFVçBçFòæWö–væ÷&Uö66–•ö66R‡7öç6÷%ö6öçG&7B¢ÇÂ–çFVç@¢æg&öÐ¢æ5öFW&Vb‚¢æ—5öæöæUö÷"‡Æg&ö×Âg&öÒæWö–væ÷&Uö66–•ö66R‡&VÆ–W"’¢ÇÂ6ÆÆFFç7F'G5÷v—F‚‚&&6FFVFB"¢ÇÂ6ÆÆFFæÆVâ‚’Òc"¢ ¢°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’À¢&FöÖ–5ö6Æ–Õö–çFVçEö–çfÆ–B"À¢'&VÆ•öFöÖ–5ö6Æ–Ò"À¢&vVæW&FVBG&ç67F–öâW†6VVFVBF†RW†7B¦W&òÔUD‚7öç6÷$æD6Æ–ÒöÆ–7’"À¢$Fòæ÷B'&öF67C²&W÷'BF†R6æF–FFR”Bâ"À¢’“°¢Ð¢ö²‚‚’§Ð ¦7–æ2fâ&VÆ•övVçEöæF—fUö6Æ–Ò€¢7FFS¢e6†&VE7FFRÀ¢6æF–FFS¢d6Æ–Ô6æF–FFRÀ¢6Æ–Õö&öæC¢ScBÀ¢æöæ6S¢g7G"À¢fÆ–Eö&Vf÷&S¢ScBÀ¢6–væGW&S¢dWFöæöÖ÷W4&÷VçG”WF†÷&—¦F–öå6–væGW&RÀ¢’Óâ&W7VÇCÄ6Æ–Ô6æF–FFRÂvVçD6Æ–Õ&ö&ÆVÓâ°¢–b6æF–FFRç7FGW2ÓÒ6Æ–Ô6æF–FFU7FGW3£¥&VÆ––ær°¢&WGW&â&V6öæ6–ÆUövVçEöæF—fUö6Æ–Ò‡7FFRÂ6æF–FFRæ6ÆöæR‚’Â6Æ–Õö&öæB’æv—C°¢Ð¢ÆWB&VÆ–W"Ò7FFP¢çƒC%÷&VÆ–W ¢ç&VÆ–W ¢æ5÷&Vb‚¢æf–ÇFW"‡Å÷Â7FFRçƒC%÷&VÆ–W"æVæ&ÆVB¢æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢&6Æ–Õ÷&VÆ–W%÷Væf–Æ&ÆR"À¢'&VÆ•ö6Æ–Ò"À¢'F†R†÷7FVBv2&VÆ–W"—2Væf–Æ&ÆR"À¢%W6RF†RF—&V7BvÆÆWEö6ÆÇ2g&öÒÆåöWFöæöÖ÷W5ö&÷VçG•ö6Æ–Òâ"À¢¢Ò“ó°¢ÆWBÆææW"Ò6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"‚f6æF–FFRææWGv÷&²’æÖöW'"‡Ç7FGW7Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW2À¢'ÆææW%÷Væf–Æ&ÆR"À¢'&VÆ•ö6Æ–Ò"À¢'F†R6æöæ–6Â6Æ–ÒÆææW"—2Væf–Æ&ÆR"À¢$Fòæ÷B6–vâ&&—G&'’6ÆÆFF²&WG'’ÆFW"â"À¢¢Ò“ó°¢ÆWBÆâÒÆææW ¢çÆåöWF†÷&—¦VEö6Æ–Ò€¢f6æF–FFRææWGv÷&²À¢f6æF–FFRæ&÷VçG•ö6öçG&7BÀ¢f6æF–FFRç6öÇfW%÷vÆÆWBÀ¢S#ƒ£¦g&öÒ†6Æ–Õö&öæB’À¢æöæ6RÀ¢fÆ–Eö&Vf÷&RÀ¢6–væGW&RÀ¢6öÖR‚g&VÆ–W"æFG&W72‚’’À¢¢æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’À¢&6Æ–Õ÷Æåö–çfÆ–B"À¢'&VÆ•ö6Æ–Ò"À¢'F†R6–væVB6Æ–Ò6÷VÆBæ÷B&R6öçfW'FVB–çFòF†RW†7B&VÆ’G&ç67F–öâ"À¢%6–vâöæÇ’F†R&WGW&æVB6–væ–æu÷–ÆöBæB&WG'’v—F‚F†R6ÖR–FV×÷FVæ7•ö¶W’â"À¢¢Ò“ó°¢fÆ–FFUövVçEö6Æ–Õ÷&VÆ•ö–çFVçB€¢gÆâç&VÆ•÷G&ç67F–öâÀ¢f6æF–FFRæ&÷VçG•ö6öçG&7BÀ¢g&VÆ–W"æFG&W72‚’À¢“ó°¢ÆWBFW67&—F÷"Ò&6UöæWGv÷&µöFW67&—F÷"‚f6æF–FFRææWGv÷&²’æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤$Eõ$UTU5BÀ¢&æWGv÷&µö–çfÆ–B"À¢'&VÆ•ö6Æ–Ò"À¢&6æF–FFRæWGv÷&²—2Vç7W÷'FVB"À¢$Fòæ÷B6–vâ÷"'&öF67Bç—F†–ærâ"À¢¢Ò“ó°¢ÆWB…òÂ'5÷W&Â’Ò7FFP¢æ&6U÷'5÷W&Ç0¢ç&W6öÇfR‚f6æF–FFRææWGv÷&²¢æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢''5÷Væf–Æ&ÆR"À¢'&VÆ•ö6Æ–Ò"À¢$&6R%2—2Væf–Æ&ÆR"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ó°¢ÆWB7F÷&RÒ7FFRç7F÷&Ræ5÷&Vb‚’æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢&6ö÷&F–æF–öå÷Væf–Æ&ÆR"À¢'&VÆ•ö6Æ–Ò"À¢&GW&&ÆR6Æ–Ò7FFR—2Væf–Æ&ÆR"À¢$6ÆÂÆåöWFöæöÖ÷W5ö&÷VçG•ö6Æ–Òâ7V&Ö—B—G2W†7BF—&V7B×vÆÆWB6ÆÇ2â"À¢¢Ò“ó°¢ÆWBÆV6RÒ7F÷&P¢æ7V—&U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²Â7FFRçƒC%÷&VÆ–W"æÆV6U÷6V6öæG2¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ð¢æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢'&VÆ•ö'W7’"À¢'&VÆ•ö6Æ–Ò"À¢'F†R&÷VæFVBv2&VÆ’—2'W7’"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ó°¢ÆWB&VÆ•÷&W7VÇBÒFö¶–ó£§F–ÖS£§F–ÖV÷WB€¢GW&F–öã£¦g&öÕ÷6V72‡7FFRçƒC%÷&VÆ–W"ç'5÷F–ÖV÷WE÷6V6öæG2’À¢&VÆ–W"ç6–×VÆFUöæEö'&öF67B€¢g'5÷W&ÂÀ¢FW67&—F÷"æ6†–åö–BÀ¢gÆâç&VÆ•÷G&ç67F–öâÀ¢7FFRçƒC%÷&VÆ–W"æÖ…öv2À¢7FFRçƒC%÷&VÆ–W"æÖ…öfVU÷W%öv5÷vV’À¢’À¢¢æv—C°¢ÆWB&VÆV6U÷&W7VÇBÒ7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R‚f6æF–FFRææWGv÷&²ÂÆV6R¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“°¢ÆWBG&ç67F–öâÒÖF6‚&VÆ•÷&W7VÇB°¢ö²„ö²‡G&ç67F–öâ’’ÓâG&ç67F–öâÀ¢ö²„W'"†W'&÷"’’Óâ°¢&VÆV6U÷&W7VÇCó°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢–bW'&÷"çFõ÷7G&–ær‚’çFõö66–•öÆ÷vW&66R‚’æ6öçF–ç2‚'&WfW'B"’°¢7FGW46öFS£¤4ôädÄ”5@¢ÒVÇ6R°¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄP¢ÒÀ¢&6Æ–Õö'&öF67Eöf–ÆVB"À¢'&VÆ•ö6Æ–Ò"À¢'F†RW†7B6Æ–ÒG&ç67F–öâf–ÆVB6–×VÆF–öâ÷"'&öF67B"À¢$6†V6²6æöæ–6Â6Æ–Ö&–Æ—G’æB6öÇfW"&öæB&Ææ6RÂF†Vâ&WG'’F†R6ÖR6–væVB&WVW7B&Vf÷&R—BW‡—&W2â"À¢’“°¢Ð¢W'"…ò’Óâ°¢&VÆV6U÷&W7VÇCó°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢&6Æ–Õ÷'5÷F–ÖV÷WB"À¢'&VÆ•ö6Æ–Ò"À¢'F†R&VÆ’%2FVFÆ–æRVÆ6VB&Vf÷&R'&öF67Bv2&V6÷&FVB"À¢%&WG'’F†R6ÖR6–væVB&WVW7C²T•Ó3’æöæ6R&WW6R6ææ÷BF÷V&ÆRÖ6Æ–Òâ"À¢’“°¢Ð¢Ó°¢&VÆV6U÷&W7VÇCó°¢ÆWB6æF–FFRÒ7F÷&P¢æÖ&µö6Æ–Õö6æF–FFU÷&VÆ––ær†6æF–FFRæ–BÂgG&ç67F–öâçG…ö†6‚¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢&V6öæ6–ÆUövVçEöæF—fUö6Æ–Ò‡7FFRÂ6æF–FFRÂ6Æ–Õö&öæB’æv—@§Ð ¦fâfÆ–FFUövVçEö6Æ–Õ÷&VÆ•ö–çFVçB€¢–çFVçC¢dWfÕG&ç67F–öä–çFVçBÀ¢&÷VçG•ö6öçG&7C¢g7G"À¢&VÆ–W#¢g7G"À¢’Óâ&W7VÇCÂ‚’ÂvVçD6Æ–Õ&ö&ÆVÓâ°¢ÆWB6ÆÆFFÒ–çFVçBæFFç7G&—÷&Vf—‚‚#‚"’çVçw&ö÷%öFVfVÇB‚“°¢–b–çFVçBçfÇVU÷vV’Ò ¢ÇÂ–çFVçBægVæ7F–öà¢Ò&6Æ–Õv—F„WF†÷&—¦F–öâ†FG&W72ÇV–çC#SbÇV–çC#SbÆ'—FW33"ÇV–çC‚Æ'—FW33"Æ'—FW33"’ ¢ÇÂ–çFVçBçFòæWö–væ÷&Uö66–•ö66R†&÷VçG•ö6öçG&7B¢ÇÂ–çFVç@¢æg&öÐ¢æ5öFW&Vb‚¢æ—5öæöæUö÷"‡Æg&ö×Âg&öÒæWö–væ÷&Uö66–•ö66R‡&VÆ–W"’¢ÇÂ6ÆÆFFæÆVâ‚’Ò‚²r¢c@¢°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’À¢&6Æ–Õö–çFVçEö–çfÆ–B"À¢'&VÆ•ö6Æ–Ò"À¢&vVæW&FVB6Æ–ÒG&ç67F–öâW†6VVFVBF†RW†7BæòÔUD‚6Æ–Õv—F„WF†÷&—¦F–öâöÆ–7’"À¢$Fòæ÷B'&öF67C²&W÷'BF†R6æF–FFR”Bâ"À¢’“°¢Ð¢ö²‚‚’§Ð ¦7–æ2fâ&V6öæ6–ÆUövVçEöæF—fUö6Æ–Ò€¢7FFS¢e6†&VE7FFRÀ¢6æF–FFS¢6Æ–Ô6æF–FFRÀ¢6Æ–Õö&öæC¢ScBÀ¢’Óâ&W7VÇCÄ6Æ–Ô6æF–FFRÂvVçD6Æ–Õ&ö&ÆVÓâ°¢–b6æF–FFRç7FGW2ÓÒ6Æ–Ô6æF–FFU7FGW3£¤6Æ–ÖVB°¢&WGW&âö²†6æF–FFR“°¢Ð¢ÆWBG…ö†6‚Ò6æF–FFRæ6Æ–Õ÷G&ç67F–öåö†6‚æ5öFW&Vb‚’æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢&6Æ–Õ÷G…öÖ—76–ær"À¢&6öæf—&Õö6Æ–Ò"À¢'&VÆ––ær6æF–FFR†2æòG&ç67F–öâ†6‚"À¢%&WG'’v—F‚F†R6ÖR–FV×÷FVæ7•ö¶W’â"À¢¢Ò“ó°¢ÆWB…òÂ'5÷W&Â’Ò7FFP¢æ&6U÷'5÷W&Ç0¢ç&W6öÇfR‚f6æF–FFRææWGv÷&²¢æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢''5÷Væf–Æ&ÆR"À¢&6öæf—&Õö6Æ–Ò"À¢$&6R%2—2Væf–Æ&ÆR"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ó°¢ÆWBFVFÆ–æRÒ–ç7FçC£¦æ÷r‚’²GW&F–öã£¦g&öÕ÷6V72‡7FFRçƒC%÷&VÆ–W"çv—E÷6V6öæG2“°¢Æö÷°¢ÆWB&V6V—BÒfWF6…÷G&ç67F–öå÷&V6V—B‚g'5÷W&ÂÂG…ö†6‚Â¢æv—@¢æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢''5÷Væf–Æ&ÆR"À¢&6öæf—&Õö6Æ–Ò"À¢'F†R6Æ–Ò&V6V—B6÷VÆBæ÷B&RfWF6†VB"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ð¢ç&W7VÇC°¢–bÆWB6öÖR‡&V6V—B’Ò&V6V—B°¢–b&V6V—Bç7V66VVFVB‚’æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤$EôtDUt’À¢'&V6V—Eö–çfÆ–B"À¢&6öæf—&Õö6Æ–Ò"À¢'F†R6Æ–Ò&V6V—B7FGW2—2–çfÆ–B"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“òÓÒ6öÖR†fÇ6R¢°¢ÆWB7F÷&RÒ7FFRç7F÷&Ræ5÷&Vb‚’æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢&6ö÷&F–æF–öå÷Væf–Æ&ÆR"À¢&6öæf—&Õö6Æ–Ò"À¢&GW&&ÆR6Æ–Ò7FFR—2Væf–Æ&ÆR"À¢%&WG'’ÆFW"â"À¢¢Ò“ó°¢ÆWB7öç6÷'6†—Ò7F÷&P¢ævWEö&öæE÷7öç6÷'6†—öf÷%ö6æF–FFR†6æF–FFRæ–B¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢–bÆWB6öÖR‡7öç6÷'6†—’Ð¢7öç6÷'6†—æf–ÇFW"‡Æ—FV×Â—FVÒç7FGW2ÓÒ&öæE7öç6÷'6†—7FGW3£¤'&öF67B¢°¢7F÷&P¢æÖ&µöFöÖ–5÷7öç6÷&VEö6Æ–Õöf–ÆVB€¢6æF–FFRæ–BÀ¢7öç6÷'6†—æ–BÀ¢'G&ç67F–öå÷&WfW'FVB"À¢%F†R6öæf—&ÖVBFöÖ–26Æ–ÒG&ç67F–öâ&WfW'FVC²æò&öæBÖ÷fVBæBæò6æöæ–6Â6Æ–Òv27&VFVBâ"À¢¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢ÒVÇ6R°¢7F÷&P¢æÖ&µö6Æ–Õö6æF–FFUöf–ÆVB€¢6æF–FFRæ–BÀ¢'G&ç67F–öå÷&WfW'FVB"À¢%F†R6öæf—&ÖVB6Æ–ÒG&ç67F–öâ&WfW'FVC²æò6æöæ–6Â6Æ–Òv27&VFVBâ"À¢¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢Ð¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤$EôtDUt’À¢&6Æ–Õ÷&WfW'FVB"À¢&6öæf—&Õö6Æ–Ò"À¢'F†R6öæf—&ÖVBG&ç67F–öâ&WfW'FVBæBVÖ—GFVBæò6æöæ–6Â6Æ–Ò"À¢$–bF†R&÷VçG’—27F–ÆÂ6Æ–Ö&ÆRÂ7F'Bg&W6‚6æF–FFRv—F‚æWr–FV×÷FVæ7•ö¶W’â"À¢’“°¢Ð¢–bÆWB6öÖR†&Æö6²’Ò&V6V—Bæ&Æö6µöçVÖ&W"‚’æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤$EôtDUt’À¢'&V6V—Eö–çfÆ–B"À¢&6öæf—&Õö6Æ–Ò"À¢'F†R6Æ–Ò&V6V—B&Æö6²—2–çfÆ–B"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ò°¢ÆWBWfVçG2Ð¢FV6öFUöWFöæöÖ÷W5ö&÷VçG•öÆöw2‡&V6V—BæÆöw5÷FõöWfÕöÆöw2‚’æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤$EôtDUt’À¢'&V6V—Eö–çfÆ–B"À¢&6öæf—&Õö6Æ–Ò"À¢'F†R6Æ–Ò&V6V—BÆöw2&R–çfÆ–B"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ò¢æÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤$EôtDUt’À¢&6Æ–ÕöWfVçEö–çfÆ–B"À¢&6öæf—&Õö6Æ–Ò"À¢'F†R6Æ–Ò&V6V—B6÷VÆBæ÷B&RFV6öFVB"À¢$Fòæ÷BG&VBF†R&÷VæB26Æ–ÖVC²&W÷'BF†RG&ç67F–öâ†6‚â"À¢¢Ò“ó°¢ÆWB6Æ–ÒÒWfVçG2æ—FW"‚’æf–æB‡ÆWfVçGÂ°¢WfVçBæ¶–æBÓÒWFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG”6Æ–ÖV@¢bbWfVç@¢æ6öçG&7EöFG&W70¢æWö–væ÷&Uö66–•ö66R‚f6æF–FFRæ&÷VçG•ö6öçG&7B¢bbWfVçBæFF²'6öÇfW"%Òæ5÷7G"‚’æ—5÷6öÖUöæB‡Ç6öÇfW'Â°¢6öÇfW"æWö–væ÷&Uö66–•ö66R‚f6æF–FFRç6öÇfW%÷vÆÆWB¢Ò¢bb§6öå÷S#‚‚fWfVçBæFF²&6Æ–Õö&öæB%Ò’ÓÒ6öÖR‡S#ƒ£¦g&öÒ†6Æ–Õö&öæB’¢Ò“°¢ÆWB6öÖR†6Æ–Ò’Ò6Æ–ÒVÇ6R°¢&WGW&âW'"†vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¤$EôtDUt’À¢&6Æ–ÕöWfVçEöÖ—6ÖF6‚"À¢&6öæf—&Õö6Æ–Ò"À¢'F†R6öæf—&ÖVBG&ç67F–öâF–Bæ÷BVÖ—BF†RW†7B6æöæ–6Â&÷VçG”6Æ–ÖVBWfVçB"À¢$Fòæ÷B7F'Bv÷&³²&W÷'BF†RG&ç67F–öâ†6‚â"À¢’“°¢Ó°¢ÆWBÆFW7BÒfWF6…ö&Æö6µöçVÖ&W"‚g'5÷W&ÂÂ"’æv—BæÖöW'"‡Å÷Â°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢''5÷Væf–Æ&ÆR"À¢&6öæf—&Õö6Æ–Ò"À¢&ÆFW7B&6R&Æö6²6÷VÆBæ÷B&RfWF6†VB"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ó°¢–bÆFW7Bç6GW&F–æu÷7V"†&Æö6²’ç6GW&F–æuöFBƒ¢ãÒ7FFRçƒC%÷&VÆ–W"æ6öæf—&ÖF–öç0¢°¢ÆWB7F÷&RÒ7FFRç7F÷&Ræ5÷&Vb‚’æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö6Æ–Õ÷&ö&ÆVÒ€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢&6ö÷&F–æF–öå÷Væf–Æ&ÆR"À¢&6öæf—&Õö6Æ–Ò"À¢&GW&&ÆR6Æ–Ò7FFR—2Væf–Æ&ÆR"À¢%&WG'’F†R6ÖR6–væVB&WVW7Bâ"À¢¢Ò“ó°¢f÷"WfVçB–âWfVçG2æ—FW"‚’æf–ÇFW"‡ÆWfVçGÂ°¢WfVç@¢æ6öçG&7EöFG&W70¢æWö–væ÷&Uö66–•ö66R‚f6æF–FFRæ&÷VçG•ö6öçG&7B¢Ò’°¢7F÷&P¢çW6W'EöWFöæöÖ÷W5ö&÷VçG•öWfVçB‚f6æF–FFRææWGv÷&²ÂWfVçB¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢Ð¢ÆWB7öç6÷'6†—Ò7F÷&P¢ævWEö&öæE÷7öç6÷'6†—öf÷%ö6æF–FFR†6æF–FFRæ–B¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“ó°¢–bÆWB6öÖR‡7öç6÷'6†—’Ð¢7öç6÷'6†—æf–ÇFW"‡Æ—FV×Â—FVÒç7FGW2ÓÒ&öæE7öç6÷'6†—7FGW3£¤'&öF67B¢°¢&WGW&â7F÷&P¢æÖ&µöFöÖ–5÷7öç6÷&VEö6Æ–Õö6öæf—&ÖVB€¢6æF–FFRæ–BÀ¢7öç6÷'6†—æ–BÀ¢6Æ–Òæ–BÀ¢&Æö6²À¢¢æv—@¢æÖ‡Â†6æF–FFRÂò—Â6æF–FFR¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“°¢Ð¢&WGW&â7F÷&P¢æÖ&µö6Æ–Õö6æF–FFUö6Æ–ÖVB†6æF–FFRæ–BÂ6Æ–Òæ–B¢æv—@¢æÖöW'"†ÖövVçEö6Æ–ÕöF%öW'&÷"“°¢Ð¢Ð¢Ð¢–b–ç7FçC£¦æ÷r‚’ãÒFVFÆ–æR°¢&WGW&âö²†6æF–FFR“°¢Ð¢6ÆVW„GW&F–öã£¦g&öÕ÷6V72ƒ’’æv—C°¢Ð§Ð ¦fâ&WV—&Uö6Æ–Ö&ÆUöWFöæöÖ÷W5ö—FVÒ†—FVÓ¢dWFöæöÖ÷W4&÷VçG”fVVD—FVÒ’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢–bWFöæöÖ÷W5ö&÷VçG•ö—5öV&æ–æu÷&VG’†—FVÒ’°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢ö²‚‚’§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâ×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ%Vç6–væVB7V&Ö—76–öâ6öÖÖ—FÖVçBÆâf÷"WFöæöÖ÷W2fW&–f–6F–öâ"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5ö&÷VçG•÷7V&Ö—76–öâ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4&÷VçG•7V&Ö—76–öå&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢&WV—&Uö–æFW†VEö6æöæ–6Åö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆå÷7V&Ö—76–öâ€¢g&WVW7Bæ&÷VçG•ö6öçG&7BÀ¢g&WVW7Bç6öÇfW"À¢g&WVW7Bç7V&Ö—76–öåö†6‚À¢g&WVW7BæWf–FVæ6Uö†6‚À¢¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâ×&W&F–öâ"À¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$6æöæ–6Â7F—fRÖ6Æ–ÒfÆ–FF–öâÂFWFW&Ö–æ—7F–27V&Ö—76–öâ6öÖÖ—FÖVçG2ÂW†7BT•Ós"–ÆöBÂæBVç6–væVB&VÆ’öWf–FVæ6RFV×ÆFW2"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$ÖÆf÷&ÖVBvÆÆWBÂ'F–f7B&VfW&Væ6RÂWf–FVæ6Rö&¦V7BÂ÷"æWGv÷&²"’À¢‡7FGW2ÒCBÂFW67&—F–öâÒ$&÷VçG’6öçG&7B—2æ÷Bâ–æFW†VB6æöæ–6Â–ç7Fæ6R"’À¢‡7FGW2ÒC’ÂFW67&—F–öâÒ$&÷VçG’—2æ÷BâW†V7WF&ÆR7F—fR6Æ–Ò÷væVB'’F†—26öÇfW"÷"W‡—&W2Föò6ööâ"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ$6æöæ–6Â–æFW†VB7FFR÷"ÆææW"6öæf–wW&F–öâ—2Væf–Æ&ÆR"¢¢•Ð¦7–æ2fâ&W&UöWFöæöÖ÷W5ö&÷VçG•÷7V&Ö—76–öâ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅ&W&TWFöæöÖ÷W4&÷VçG•7V&Ö—76–öå&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W4&÷VçG•7V&Ö—76–öå&W&F–öãâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢ÆWBö'6W'fVEöE÷Væ—‚Ð¢ScC£§G'•ög&öÒ…WF3£¦æ÷r‚’çF–ÖW7F×‚’’æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢'V–ÆEöWFöæöÖ÷W5÷7V&Ö—76–öå÷&W&F–öâ€¢f6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“òÀ¢æWGv÷&²À¢f—FVÒÀ¢g&WVW7Bç6öÇfW%÷vÆÆWBÀ¢g&WVW7Bæ'F–f7E÷&VfW&Væ6RÀ¢&WVW7BæWf–FVæ6RÀ¢ö'6W'fVEöE÷Væ—‚À¢¢æÖ„§6öâ¢æÖöW'"‡ÆW'&÷'ÂÖF6‚W'&÷"°¢6†–ä&6TW'&÷#£¤–çfÆ–E7V&Ö—76–öå&W&F–öâ…ò’Óâ7FGW46öFS£¤4ôädÄ”5BÀ¢6†–ä&6TW'&÷#£¤–çfÆ–E7V&Ö—76–öäWf–FVæ6R…ò¢Â6†–ä&6TW'&÷#£¤–çfÆ–DFG&W72…ò¢Â6†–ä&6TW'&÷#£¤–çfÆ–D6æöæ–6Ä§6öâ…ò¢Â6†–ä&6TW'&÷#£¥Væ¶æ÷väæWGv÷&²…ò’Óâ7FGW46öFS£¤$Eõ$UTU5BÀ¢òÓâ7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢Ò§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâÖWF†÷&—¦F–öâ×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$W†7BT•Ós"7V&Ö—76–öâWF†÷&—¦F–öâf÷"v2×7öç6÷&VB7V&Ö—Ev—F…6–væGW&R&VÆ’"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5ö&÷VçG•÷7V&Ö—76–öåöWF†÷&—¦F–öâ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4&÷VçG•7V&Ö—76–öäWF†÷&—¦F–öå&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W4&÷VçG•7V&Ö—76–öäWF†÷&—¦F–öåG—VDFFâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢&WV—&Uö–æFW†VEö6æöæ–6Åö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bç7V&Ö—76–öâæ&÷VçG•ö6öçG&7B’æv—Có°¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆå÷7V&Ö—76–öåöWF†÷&—¦F–öâ†æWGv÷&²Âg&WVW7Bç7V&Ö—76–öâ¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷fW&–f–6F–öâÖGFW7FF–öâ×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$W†7BT•Ós"–ÆöBf÷"öæR6öÖÖ—GFVBfW&–f–W"Fò6–vâ"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5÷fW&–f–6F–öåöGFW7FF–öâ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W5fW&–f–6F–öäGFW7FF–öå&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W5fW&–f–6F–öäGFW7FF–öåG—VDFFâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÐ¢–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7BæGFW7FF–öâæ&÷VçG•ö6öçG&7B’æv—Có°¢ÆWBö'6W'fVEöBÒScC£§G'•ög&öÒ…WF3£¦æ÷r‚’çF–ÖW7F×‚’’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢fÆ–FFUöGFW7FF–öå÷&WVW7Eöv–ç7EöfVVB‚f—FVÒÂg&WVW7BæGFW7FF–öâÂö'6W'fVEöB¢æÖöW'"‡Å÷Â7FGW46öFS£¤4ôädÄ”5B“ó°¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆå÷fW&–f–6F–öåöGFW7FF–öâ†æWGv÷&²Âg&WVW7BæGFW7FF–öâ¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öÖöGVÆR×6WGFÆVÖVçB×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ%W&Ö—76–öæÆW72FWFW&Ö–æ—7F–2fW&–f–W"6ÆÂF†BFöÖ–6ÆÇ’6WGFÆW2öâ72"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5öÖöGVÆU÷6WGFÆVÖVçB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4ÖöGVÆU6WGFÆVÖVçE&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢&WV—&UöWFöæöÖ÷W5ö—FVÕöÖöFR‚f—FVÒÂ&FWFW&Ö–æ—7F–5öÖöGVÆR"“ó°¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆåöÖöGVÆU÷6WGFÆVÖVçB€¢g&WVW7Bæ&÷VçG•ö6öçG&7BÀ¢&WVW7Bæ6ÆÆW"æ5öFW&Vb‚’À¢g&WVW7Bç&ööbÀ¢¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öGFW7FF–öâ×6WGFÆVÖVçB×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ%W&Ö—76–öæÆW726öÖÖ—GFVBfW&–f–W"V÷'VÒ&VÆ’F†B6WGFÆW2÷"&V÷Vç2FöÖ–6ÆÇ’"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5öGFW7FF–öå÷6WGFÆVÖVçB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4GFW7FF–öå6WGFÆVÖVçE&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢–b—FVÒçFW&×5÷fÆ–B°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢ÆWBÖöFRÒWFöæöÖ÷W5ö—FVÕöÖöFR‚f—FVÒ“ó°¢–bÖöFRÒ'6–væVE÷V÷'VÒ"bbÖöFRÒ&•ö§VFvU÷V÷'VÒ"°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢ÆWBöÆ–7’Ò—FVÐ¢çFW&×0¢æ5÷&Vb‚¢æÖ‡ÇFW&×7ÂgFW&×2æFö7VÖVçBçfW&–f–6F–öå÷öÆ–7’¢æöµö÷"…7FGW46öFS£¤4ôädÄ”5B“ó°¢ÆWBF‡&W6†öÆBÒöÆ–7¢ævWB‚'F‡&W6†öÆB"¢ææE÷F†Vâ‡6W&FUö§6öã£¥fÇVS£¦5÷ScB¢ææE÷F†Vâ‡ÇfÇVWÂW6—¦S£§G'•ög&öÒ‡fÇVR’æö²‚’¢æöµö÷"…7FGW46öFS£¤4ôädÄ”5B“ó°¢–b&WVW7BæGFW7FF–öç2æÆVâ‚’ÒF‡&W6†öÆB°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢ÆWBÆÆ÷vVBÒöÆ–7¢ævWB‚'fW&–f–W'2"¢ææE÷F†Vâ‡6W&FUö§6öã£¥fÇVS£¦5ö'&’¢æöµö÷"…7FGW46öFS£¤4ôädÄ”5B“ó°¢–b&WVW7BæGFW7FF–öç2æ—FW"‚’æç’‡ÆGFW7FF–öçÂ°¢ÆÆ÷vVBæ—FW"‚’æç’‡ÇfÇVWÂ°¢fÇVP¢æ5÷7G"‚¢æ—5÷6öÖUöæB‡ÇfW&–f–W'ÂfW&–f–W"æWö–væ÷&Uö66–•ö66R‚fGFW7FF–öâçfW&–f–W"’¢Ò¢Ò’°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆåöGFW7FF–öå÷6WGFÆVÖVçB€¢g&WVW7Bæ&÷VçG•ö6öçG&7BÀ¢&WVW7Bæ6ÆÆW"æ5öFW&Vb‚’À¢g&WVW7BæGFW7FF–öç2À¢¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öW‡—&RÖ6Æ–Ò×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ%W&Ö—76–öæÆW72W‡—&VBÖ6Æ–Ò&VÆV6RÆâ"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5öW‡—&Uö6Æ–Ò€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4Æ–fV7–6ÆU&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢–b—FVÒç7FGW2Ò&6Æ–ÖVB"°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆåöW‡—&Uö6Æ–Ò‚g&WVW7Bæ&÷VçG•ö6öçG&7BÂ&WVW7Bæ6ÆÆW"æ5öFW&Vb‚’¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öW‡—&R×7V&Ö—76–öâ×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ%W&Ö—76–öæÆW72W‡—&VB×7V&Ö—76–öâ&VÆV6RÆâ"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5öW‡—&U÷7V&Ö—76–öâ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4Æ–fV7–6ÆU&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢–b—FVÒç7FGW2Ò'7V&Ö—GFVB"°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆåöW‡—&U÷7V&Ö—76–öâ‚g&WVW7Bæ&÷VçG•ö6öçG&7BÂ&WVW7Bæ6ÆÆW"æ5öFW&Vb‚’¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷F–ÖV÷WB×&VÆ’"À¢&WVW7Eö&öG’Ò&VÆ”WFöæöÖ÷W5F–ÖV÷WE&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$6æöæ–6ÂF–ÖV÷WBWfVçB6öæf—&ÖVB"Â&öG’Ò&VÆ”WFöæöÖ÷W5F–ÖV÷WE&W7öç6R’À¢‡7FGW2Ò#"ÂFW67&—F–öâÒ$&÷VæFVBF–ÖV÷WBG&ç67F–öâ'&öF67C²6öæf—&ÖF–öâVæF–ær"Â&öG’Ò&VÆ”WFöæöÖ÷W5F–ÖV÷WE&W7öç6R’À¢‡7FGW2ÒCBÂFW67&—F–öâÒ$6æöæ–6Â–æFW†VB&÷VçG’æ÷Bf÷VæB"’À¢‡7FGW2ÒC’ÂFW67&—F–öâÒ%&WVW7FVBG&ç6—F–öâFöW2æ÷BÖF6‚F†R–æFW†VB&÷VçG’7FFR÷"–Ö×WF&ÆRFVFÆ–æR"’À¢‡7FGW2ÒC#"ÂFW67&—F–öâÒ$vVæW&FVB&VÆ’–çFVçBf–öÆFVBF†R&÷VæFVBF–ÖV÷WBöÆ–7’"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ$†÷7FVBv2&VÆ–W"ÂFF&6RÆV6RÂ÷"&6R%2Væf–Æ&ÆR"¢¢•Ð¦7–æ2fâ&VÆ•öWFöæöÖ÷W5÷F–ÖV÷WB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅ&VÆ”WFöæöÖ÷W5F–ÖV÷WE&WVW7CâÀ¢’Óâ&W7VÇCÅ&W7öç6RÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWBFW67&—F÷"Ò&6UöæWGv÷&µöFW67&—F÷"†æWGv÷&²’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWBæWGv÷&²ÒÖF6‚FW67&—F÷"æ6†–åö–B°¢…óCS2Óâ&&6RÖÖ–ææWB"À¢ƒEóS3"Óâ&&6R×6WöÆ–"À¢òÓâ&WGW&âW'"…7FGW46öFS£¤$Eõ$UTU5B’À¢Ó°¢ÆWB&÷VçG•ö6öçG&7BÐ¢æ÷&ÖÆ—¦UöWfÕöFG&W72‚g&WVW7Bæ&÷VçG•ö6öçG&7B’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âf&÷VçG•ö6öçG&7B’æv—Có°¢–b—FVÒçFW&×5÷fÆ–BÇÂ—FVÒç7FGW2Ò&WVW7Bæ7F–öâç&Wf–÷W5ö&÷VçG•÷7FFR‚’°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð ¢ÆWB&VÆ–W"Ò7FFP¢çƒC%÷&VÆ–W ¢ç&VÆ–W ¢æ5÷&Vb‚¢æf–ÇFW"‡Å÷Â7FFRçƒC%÷&VÆ–W"æVæ&ÆVB¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWB&VÆ–W%öFG&W72Ò&VÆ–W"æFG&W72‚“°¢ÆWBÆææW"Ò6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ó°¢ÆWB–çFVçBÒÖF6‚&WVW7Bæ7F–öâ°¢WFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&T6Æ–ÒÓâ°¢ÆææW"çÆåöW‡—&Uö6Æ–Ò‚f&÷VçG•ö6öçG&7BÂ6öÖR‚g&VÆ–W%öFG&W72’¢Ð¢WFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&U7V&Ö—76–öâÓâ°¢ÆææW"çÆåöW‡—&U÷7V&Ö—76–öâ‚f&÷VçG•ö6öçG&7BÂ6öÖR‚g&VÆ–W%öFG&W72’¢Ð¢Ð¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢fÆ–FFUöWFöæöÖ÷W5÷F–ÖV÷WEö–çFVçB€¢f–çFVçBÀ¢f&÷VçG•ö6öçG&7BÀ¢g&VÆ–W%öFG&W72À¢&WVW7Bæ7F–öâÀ¢“ó° ¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWB…òÂ'5÷W&Â’Ò7FFP¢æ&6U÷'5÷W&Ç0¢ç&W6öÇfR†æWGv÷&²¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBÆV6U÷Fö¶VâÒ7F÷&P¢æ7V—&U÷ƒC%÷&VÆ–W%öÆV6R†æWGv÷&²Â7FFRçƒC%÷&VÆ–W"æÆV6U÷6V6öæG2¢æv—@¢æÖöW'"†Ö÷ƒC%öF%öW'&÷"“ð¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWB&VÆ•÷&W7VÇBÒFö¶–ó£§F–ÖS£§F–ÖV÷WB€¢GW&F–öã£¦g&öÕ÷6V72‡7FFRçƒC%÷&VÆ–W"ç'5÷F–ÖV÷WE÷6V6öæG2’À¢&VÆ–W"ç6–×VÆFUöæEö'&öF67B€¢g'5÷W&ÂÀ¢FW67&—F÷"æ6†–åö–BÀ¢f–çFVçBÀ¢7FFRçƒC%÷&VÆ–W"æÖ…öv2À¢7FFRçƒC%÷&VÆ–W"æÖ…öfVU÷W%öv5÷vV’À¢’À¢¢æv—C°¢ÆWB&VÆV6U÷&W7VÇBÒ7F÷&P¢ç&VÆV6U÷ƒC%÷&VÆ–W%öÆV6R†æWGv÷&²ÂÆV6U÷Fö¶Vâ¢æv—@¢æÖöW'"†Ö÷ƒC%öF%öW'&÷"“°¢ÆWBG&ç67F–öâÒÖF6‚&VÆ•÷&W7VÇB°¢ö²„ö²‡G&ç67F–öâ’’ÓâG&ç67F–öâÀ¢ö²„W'"†W'&÷"’’Óâ&WGW&âW'"‡F–ÖV÷WE÷&VÆ•÷7FGW2‚fW'&÷"’’À¢W'"…ò’Óâ&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR’À¢Ó°¢&VÆV6U÷&W7VÇCó° ¢ÆWB6öæf—&ÖF–öâÒv—Eöf÷%÷F–ÖV÷WEö6öæf—&ÖF–öâ€¢g'5÷W&ÂÀ¢f&÷VçG•ö6öçG&7BÀ¢&WVW7Bæ7F–öâÀ¢gG&ç67F–öâÀ¢7FFRçƒC%÷&VÆ–W"æ6öæf—&ÖF–öç2À¢7FFRçƒC%÷&VÆ–W"çv—E÷6V6öæG2À¢¢æv—Có°¢ÆWB&W7öç6RÒ&VÆ”WFöæöÖ÷W5F–ÖV÷WE&W7öç6R°¢æWGv÷&³¢æWGv÷&²çFõ÷7G&–ær‚’À¢&÷VçG•ö6öçG&7BÀ¢7F–öã¢&WVW7Bæ7F–öâÀ¢&Wf–÷W5ö&÷VçG•÷7FFS¢&WVW7Bæ7F–öâç&Wf–÷W5ö&÷VçG•÷7FFR‚’çFõ÷7G&–ær‚’À¢W‡V7FVEö&÷VçG•÷7FFS¢&WVW7Bæ7F–öâæW‡V7FVEö&÷VçG•÷7FFR‚’çFõ÷7G&–ær‚’À¢W‡V7FVEö6æöæ–6ÅöWfVçC¢&WVW7Bæ7F–öâæW‡V7FVEöWfVçEöæÖR‚’çFõ÷7G&–ær‚’À¢G&ç67F–öåö†6ƒ¢G&ç67F–öâçG…ö†6‚À¢&VÆ–W#¢G&ç67F–öâç&VÆ–W"À¢6öæf—&ÖVC¢6öæf—&ÖF–öâæ6öæf—&ÖVBÀ¢6öæf—&ÖVEö&Æö6³¢6öæf—&ÖF–öâæ6öæf—&ÖVEö&Æö6²À¢6æöæ–6ÅöWfVçEö–C¢6öæf—&ÖF–öâæ6æöæ–6ÅöWfVçEö–BÀ¢Wf–FVæ6Uö&÷VæF'“¢f÷&ÖB€¢$öæÇ’6öæf—&ÖVB·ÒWfVçB&÷fW2F†—2F–ÖV÷WBG&ç6—F–öââ—B—2&öæBöÆ–fV7–6ÆRWf–FVæ6RÂæ÷B&÷VçG’–÷WB÷"&÷VçG•6WGFÆVBWf–FVæ6Râ"À¢&WVW7Bæ7F–öâæW‡V7FVEöWfVçEöæÖR‚¢’À¢Ó°¢ö²‚€¢–b&W7öç6Ræ6öæf—&ÖVB°¢7FGW46öFS£¤ô°¢ÒVÇ6R°¢7FGW46öFS£¤44UDT@¢ÒÀ¢§6öâ‡&W7öç6R’À¢¢æ–çFõ÷&W7öç6R‚’§Ð ¦–×ÂWFöæöÖ÷W5F–ÖV÷WD7F–öâ°¢fâ&Wf–÷W5ö&÷VçG•÷7FFR‡6VÆb’Óâbw7FF–27G"°¢ÖF6‚6VÆb°¢6VÆc£¤W‡—&T6Æ–ÒÓâ&6Æ–ÖVB"À¢6VÆc£¤W‡—&U7V&Ö—76–öâÓâ'7V&Ö—GFVB"À¢Ð¢Ð ¢fâW‡V7FVEö&÷VçG•÷7FFR‡6VÆb’Óâbw7FF–27G"°¢&6Æ–Ö&ÆR ¢Ð ¢fâgVæ7F–öâ‡6VÆb’Óâbw7FF–27G"°¢ÖF6‚6VÆb°¢6VÆc£¤W‡—&T6Æ–ÒÓâ&W‡—&T6Æ–Ò‚’"À¢6VÆc£¤W‡—&U7V&Ö—76–öâÓâ&W‡—&U7V&Ö—76–öâ‚’"À¢Ð¢Ð ¢fâ6ÆÆFF‡6VÆb’Óâbw7FF–27G"°¢ÖF6‚6VÆb°¢6VÆc£¤W‡—&T6Æ–ÒÓâ#ƒ#SvC&3‚"À¢6VÆc£¤W‡—&U7V&Ö—76–öâÓâ#†c“#SV3r"À¢Ð¢Ð ¢fâW‡V7FVEöWfVçEöæÖR‡6VÆb’Óâbw7FF–27G"°¢ÖF6‚6VÆb°¢6VÆc£¤W‡—&T6Æ–ÒÓâ$6Æ–ÔW‡—&VB"À¢6VÆc£¤W‡—&U7V&Ö—76–öâÓâ%7V&Ö—76–öäW‡—&VB"À¢Ð¢Ð ¢fâW‡V7FVEöWfVçEö¶–æB‡6VÆb’ÓâWFöæöÖ÷W4&÷VçG”WfVçD¶–æB°¢ÖF6‚6VÆb°¢6VÆc£¤W‡—&T6Æ–ÒÓâWFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤6Æ–ÔW‡—&VBÀ¢6VÆc£¤W‡—&U7V&Ö—76–öâÓâWFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¥7V&Ö—76–öäW‡—&VBÀ¢Ð¢Ð§Ð ¦fâfÆ–FFUöWFöæöÖ÷W5÷F–ÖV÷WEö–çFVçB€¢–çFVçC¢dWfÕG&ç67F–öä–çFVçBÀ¢&÷VçG•ö6öçG&7C¢g7G"À¢&VÆ–W#¢g7G"À¢7F–öã¢WFöæöÖ÷W5F–ÖV÷WD7F–öâÀ¢’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢–b–çFVçBçfÇVU÷vV’Ò ¢ÇÂ–çFVçBægVæ7F–öâÒ7F–öâægVæ7F–öâ‚¢ÇÂ–çFVçBçFòæWö–væ÷&Uö66–•ö66R†&÷VçG•ö6öçG&7B¢ÇÂ–çFVç@¢æg&öÐ¢æ5öFW&Vb‚¢æ—5öæöæUö÷"‡Æg&ö×Âg&öÒæWö–væ÷&Uö66–•ö66R‡&VÆ–W"’¢°¢&WGW&âW'"…7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’“°¢Ð¢–b–çFVçBæFFæWö–væ÷&Uö66–•ö66R†7F–öâæ6ÆÆFF‚’’°¢&WGW&âW'"…7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’“°¢Ð¢ö²‚‚’§Ð ¦fâF–ÖV÷WE÷&VÆ•÷7FGW2†W'&÷#¢d6†–ä&6TW'&÷"’Óâ7FGW46öFR°¢ÖF6‚W'&÷"°¢6†–ä&6TW'&÷#£¥&VÆ–W%&÷f–FW"†ÖW76vR¢–bÖW76vRçFõö66–•öÆ÷vW&66R‚’æ6öçF–ç2‚'&WfW'B"’Óà¢°¢7FGW46öFS£¤4ôädÄ”5@¢Ð¢6†–ä&6TW'&÷#£¤–çfÆ–E&VÆ”–çFVçB…ò¢Â6†–ä&6TW'&÷#£¥&VÆ–W$6†–äÖ—6ÖF6‚²ââÐ¢Â6†–ä&6TW'&÷#£¥&VÆ–W$v4Æ–Ö—DW†6VVFVB²ââÐ¢Â6†–ä&6TW'&÷#£¥&VÆ–W$fVT6W†6VVFVB²ââÒÓâ7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’À¢òÓâ7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢Ð§Ð ¢5¶FW&—fR„FV'Vr•Ð§7G'V7BF–ÖV÷WE&VÆ”6öæf—&ÖF–öâ°¢6öæf—&ÖVC¢&ööÂÀ¢6öæf—&ÖVEö&Æö6³¢÷F–öãÇScCâÀ¢6æöæ–6ÅöWfVçEö–C¢÷F–öãÅ7G&–æsâÀ§Ð ¦7–æ2fâv—Eöf÷%÷F–ÖV÷WEö6öæf—&ÖF–öâ€¢'5÷W&Ã¢g7G"À¢&÷VçG•ö6öçG&7C¢g7G"À¢7F–öã¢WFöæöÖ÷W5F–ÖV÷WD7F–öâÀ¢G&ç67F–öã¢d&6U&VÆ–VEG&ç67F–öâÀ¢&WV—&VEö6öæf—&ÖF–öç3¢ScBÀ¢v—E÷6V6öæG3¢ScBÀ¢’Óâ&W7VÇCÅF–ÖV÷WE&VÆ”6öæf—&ÖF–öâÂ7FGW46öFSâ°¢ÆWBFVFÆ–æRÒ–ç7FçC£¦æ÷r‚’²GW&F–öã£¦g&öÕ÷6V72‡v—E÷6V6öæG2“°¢Æö÷°¢ÆWB&V6V—BÒfWF6…÷G&ç67F–öå÷&V6V—B‡'5÷W&ÂÂgG&ç67F–öâçG…ö†6‚Â¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ð¢ç&W7VÇC°¢–bÆWB6öÖR‡&V6V—B’Ò&V6V—B°¢–b&V6V—@¢ç7V66VVFVB‚¢æÖöW'"‡ÆW'&÷'Â&6U÷'5öfWF6…÷7FGW2‚fW'&÷"’“ð¢ÓÒ6öÖR†fÇ6R¢°¢&WGW&âW'"…7FGW46öFS£¤$EôtDUt’“°¢Ð¢ÆWB&Æö6µöçVÖ&W"Ò&V6V—@¢æ&Æö6µöçVÖ&W"‚¢æÖöW'"‡ÆW'&÷'Â&6U÷'5öfWF6…÷7FGW2‚fW'&÷"’“ð¢æöµö÷"…7FGW46öFS£¤$EôtDUt’“ó°¢ÆWBWfVçG2ÒFV6öFUöWFöæöÖ÷W5ö&÷VçG•öÆöw2€¢&V6V—@¢æÆöw5÷FõöWfÕöÆöw2‚¢æÖöW'"‡ÆW'&÷'Â&6U÷'5öfWF6…÷7FGW2‚fW'&÷"’“òÀ¢¢æÖöW'"‡ÆW'&÷'Â&6U÷'5öfWF6…÷7FGW2‚fW'&÷"’“ó°¢ÆWBWfVçBÒWfVçG2æ–çFõö—FW"‚’æf–æB‡ÆWfVçGÂ°¢WfVçBæ¶–æBÓÒ7F–öâæW‡V7FVEöWfVçEö¶–æB‚¢bbWfVçBæ6öçG&7EöFG&W72æWö–væ÷&Uö66–•ö66R†&÷VçG•ö6öçG&7B¢Ò“°¢ÆWBWfVçBÒWfVçBæöµö÷"…7FGW46öFS£¤$EôtDUt’“ó°¢ÆWBÆFW7Eö&Æö6²ÒfWF6…ö&Æö6µöçVÖ&W"‡'5÷W&ÂÂ"¢æv—@¢æÖöW'"‡ÆW'&÷'Â&6U÷'5öfWF6…÷7FGW2‚fW'&÷"’“ó°¢ÆWB6öæf—&ÖF–öç2ÒÆFW7Eö&Æö6²ç6GW&F–æu÷7V"†&Æö6µöçVÖ&W"’ç6GW&F–æuöFBƒ“°¢–b6öæf—&ÖF–öç2ãÒ&WV—&VEö6öæf—&ÖF–öç2°¢&WGW&âö²…F–ÖV÷WE&VÆ”6öæf—&ÖF–öâ°¢6öæf—&ÖVC¢G'VRÀ¢6öæf—&ÖVEö&Æö6³¢6öÖR†&Æö6µöçVÖ&W"’À¢6æöæ–6ÅöWfVçEö–C¢6öÖR†WfVçBæÆöuö¶W’’À¢Ò“°¢Ð¢Ð¢–b–ç7FçC£¦æ÷r‚’ãÒFVFÆ–æR°¢&WGW&âö²…F–ÖV÷WE&VÆ”6öæf—&ÖF–öâ°¢6öæf—&ÖVC¢fÇ6RÀ¢6öæf—&ÖVEö&Æö6³¢æöæRÀ¢6æöæ–6ÅöWfVçEö–C¢æöæRÀ¢Ò“°¢Ð¢6ÆVW„GW&F–öã£¦g&öÕ÷6V72ƒ’’æv—C°¢Ð§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö6æ6VÂ×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$7&VF÷"÷"÷7BÖFVFÆ–æR6æ6VÆÆF–öâÆâ"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5ö6æ6VÂ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4Æ–fV7–6ÆU&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢ÆWB6ÆÆW"Ò&WVW7Bæ6ÆÆW"æ5öFW&Vb‚’æöµö÷"…7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWBgVæF–æuöFVFÆ–æRÒ—FVÐ¢çFW&×0¢æ5÷&Vb‚¢ææE÷F†Vâ‡ÇFW&×7ÂFW&×2æFö7VÖVçBæ6öçG&7E÷FW&×5²&gVæF–æuöFVFÆ–æR%Òæ5÷ScB‚’“°¢ÆWBö'6W'fVEöBÐ¢ScC£§G'•ög&öÒ…WF3£¦æ÷r‚’çF–ÖW7F×‚’’æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWB6ÆÆW"ÒfÆ–FFUöWFöæöÖ÷W5ö6æ6VÅöWF†÷&—G’€¢f—FVÒç7FGW2À¢f—FVÒæ7&VF÷"À¢6ÆÆW"À¢gVæF–æuöFVFÆ–æRÀ¢ö'6W'fVEöBÀ¢¢æÖöW'"‡ÆW'&÷'ÂÖF6‚W'&÷"°¢6†–ä&6TW'&÷#£¤–çfÆ–DFG&W72…ò’Óâ7FGW46öFS£¤$Eõ$UTU5BÀ¢6†–ä&6TW'&÷#£¤–çfÆ–EfW&–f–6F–öä6öæf–wW&F–öâ†ÖW76vR¢–bÖW76vRæ6öçF–ç2‚&6ææ÷B&R6æ6VÆÆVB"’Óà¢°¢7FGW46öFS£¤4ôädÄ”5@¢Ð¢òÓâ7FGW46öFS£¤dõ$$”DDTâÀ¢Ò“ó°¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆåö6æ6VÂ‚g&WVW7Bæ&÷VçG•ö6öçG&7BÂ6öÖR‚f6ÆÆW"’¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷&VgVæB×v—F†G&vÂ×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$6öçG&–'WF÷"VÆÂ×&VgVæBG&ç67F–öâÆâgFW"6æ6VÆÆF–öâ"’’•Ð¦7–æ2fâÆåöWFöæöÖ÷W5÷&VgVæE÷v—F†G&vÂ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäWFöæöÖ÷W4Æ–fV7–6ÆU&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢–b—FVÒç7FGW2Ò&6æ6VÆÆVB"°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢ÆWB6öçG&–'WF÷"Ò&WVW7Bæ6ÆÆW"æ5öFW&Vb‚’æöµö÷"…7FGW46öFS£¤$Eõ$UTU5B“ó°¢6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ð¢çÆå÷&VgVæE÷v—F†G&vÂ‚g&WVW7Bæ&÷VçG•ö6öçG&7BÂ6öçG&–'WF÷"¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö&÷VæFVB×vÆÆWBÖ6æ6VÂ×&VgVæB×Æâ"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$÷væW"ÖöæÇ’FöÖ–2&÷VæFVB×vÆÆWBc"6æ6VÆÆF–öâæB7&VF÷"&VgVæBÆâ"’’•Ð¦7–æ2fâÆåö&÷VæFVE÷vÆÆWEö6æ6VÅ÷&VgVæB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆä&÷VæFVEvÆÆWD6æ6VÅ&VgVæE&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWfÕG&ç67F–öä–çFVçCâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7BææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢–b—FVÒæ7&VF÷"æWö–væ÷&Uö66–•ö66R‚g&WVW7Bæ&÷VæFVE÷vÆÆWB’°¢&WGW&âW'"…7FGW46öFS£¤dõ$$”DDTâ“°¢Ð¢ÆWBÆææW"Ò6öæf–wW&VEöWFöæöÖ÷W5÷ÆææW"†æWGv÷&²“ó°¢ÆWBÆâÒÖF6‚—FVÒç7FGW2æ5÷7G"‚’°¢&÷Vâ"Â&6Æ–Ö&ÆR"ÓâÆææW"çÆåö&÷VæFVE÷vÆÆWEö6æ6VÅ÷&VgVæB€¢g&WVW7Bæ&÷VæFVE÷vÆÆWBÀ¢g&WVW7Bæ&÷VçG•ö6öçG&7BÀ¢g&WVW7Bæ6ÆÆW"À¢’À¢&6æ6VÆÆVB"ÓâÆææW"çÆåö&÷VæFVE÷vÆÆWE÷&VgVæB€¢g&WVW7Bæ&÷VæFVE÷vÆÆWBÀ¢g&WVW7Bæ&÷VçG•ö6öçG&7BÀ¢g&WVW7Bæ6ÆÆW"À¢’À¢òÓâ&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B’À¢Ó°¢ÆâæÖ„§6öâ’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¦fâWFöæöÖ÷W5ö—FVÕöÖöFR†—FVÓ¢dWFöæöÖ÷W4&÷VçG”fVVD—FVÒ’Óâ&W7VÇCÂg7G"Â7FGW46öFSâ°¢—FVÒçFW&×0¢æ5÷&Vb‚¢ææE÷F†Vâ‡ÇFW&×7ÂFW&×2æFö7VÖVçBçfW&–f–6F–öå÷öÆ–7’ævWB‚&ÖV6†æ—6Ò"’¢ææE÷F†Vâ‡6W&FUö§6öã£¥fÇVS£¦5÷7G"¢æöµö÷"…7FGW46öFS£¤4ôädÄ”5B§Ð ¦fâ&WV—&UöWFöæöÖ÷W5ö—FVÕöÖöFR€¢—FVÓ¢dWFöæöÖ÷W4&÷VçG”fVVD—FVÒÀ¢W‡V7FVC¢g7G"À¢’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢–b—FVÒçFW&×5÷fÆ–Bbb—FVÒç7FGW2ÓÒ'7V&Ö—GFVB"bbWFöæöÖ÷W5ö—FVÕöÖöFR†—FVÒ“òÓÒW‡V7FVB°¢ö²‚‚’¢ÒVÇ6R°¢W'"…7FGW46öFS£¤4ôädÄ”5B¢Ð§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öFV6öFRÖWfVçG2"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$FV6öFVBWFöæöÖ÷W2f7F÷'’ÂgVæF–ærÂ6Æ–ÒÂ7V&Ö—76–öâÂ6WGFÆVÖVçBÂæB&VgVæBWf–FVæ6R"’’•Ð¦7–æ2fâFV6öFUöWFöæöÖ÷W5ö&÷VçG•öWfVçG2€¢§6öâ‡&WVW7B“¢§6öãÄFV6öFTWFöæöÖ÷W4&÷VçG”WfVçG5&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÅfV3ÄWFöæöÖ÷W4&÷VçG”WfVçCãâÂ7FGW46öFSâ°¢FV6öFUöWFöæöÖ÷W5ö&÷VçG•öÆöw2‡&WVW7BæÆöw2¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚†vWBÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öWfVçG2"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ%W'6—7FVB6öæf—&ÖVBWFöæöÖ÷W2&÷VçG’WfVçG2"’’•Ð¦7–æ2fâÆ—7EöWFöæöÖ÷W5ö&÷VçG•öWfVçG2€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢VW'’‡VW'’“¢VW'“ÄWFöæöÖ÷W4&÷VçG”WfVçG5VW'“âÀ¢’Óâ&W7VÇCÄ§6öãÅfV3ÄWFöæöÖ÷W4&÷VçG”WfVçCãâÂ7FGW46öFSâ°¢ÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&RVÇ6R°¢&WGW&âö²„§6öâ…fV3£¦æWr‚’’“°¢Ó°¢ÆWBæWGv÷&²ÒVW'’ææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWB×WBWfVçG2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•öWfVçG2†æWGv÷&²¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢–bÆWB6öÖR†&÷VçG•ö–B’ÒVW'’æ&÷VçG•ö–B°¢WfVçG2ç&WF–â‡ÆWfVçGÂWfVçBæ&÷VçG•ö–BæWö–væ÷&Uö66–•ö66R‚f&÷VçG•ö–B’“°¢Ð¢ö²„§6öâ†WfVçG2’§Ð ¦fâWFöæöÖ÷W5÷FW&×5÷&V6÷&B€¢&WVW7C¢V&Æ—6„WFöæöÖ÷W4&÷VçG•FW&×5&WVW7BÀ¢’Óâ&W7VÇCÄWFöæöÖ÷W4&÷VçG•FW&×5&V6÷&BÂvVçD7F–öä”W'&÷#â°¢'V–ÆEöWFöæöÖ÷W5ö&÷VçG•÷FW&×5÷&V6÷&B‚g&WVW7Bæ7&VF÷%÷vÆÆWBÂ&WVW7BæFö7VÖVçBÂWF3£¦æ÷r‚’¢æÖöW'"‡ÆW'&÷'Â°¢ÆWB‡7FGW2Â6öFR’ÒÖF6‚fW'&÷"°¢6†–ä&6TW'&÷#£¥FW&×4Fö7VÖVçEFöôÆ&vRÓâ°¢…7FGW46öFS£¥”ÄôEõDôõôÄ$tRÂ'FW&×5öFö7VÖVçE÷FöõöÆ&vR"¢Ð¢òÓâ…7FGW46öFS£¤$Eõ$UTU5BÂ&–çfÆ–EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2"’À¢Ó°¢vVçEö7F–öåöW'&÷"€¢7FGW2À¢6öFRÀ¢W'&÷"çFõ÷7G&–ær‚’À¢fÇ6RÀ¢$6÷'&V7BF†RFW&×2Fö7VÖVçBæBV&Æ—6‚—B&Vf÷&R7&VF–ær÷"gVæF–ær&÷VçG’âF†R&WGW&æVBFW&×2†6‚×W7B&R6öÖÖ—GFVBöâÖ6†–âVæ6†ævVBâ"À¢¢Ò§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷FW&×2"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$6öçFVçBÖFG&W76VBV&Æ–2&÷VçG’FW&×2æB6öçG&7B†6‚6öÖÖ—FÖVçG2"’’•Ð¦7–æ2fâV&Æ—6…öWFöæöÖ÷W5ö&÷VçG•÷FW&×2€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢§6öâ‡&WVW7B“¢§6öãÅV&Æ—6„WFöæöÖ÷W4&÷VçG•FW&×5&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W4&÷VçG•FW&×5&V6÷&CâÂvVçD7F–öä”W'&÷#â°¢ÆWB&V6÷&BÒWFöæöÖ÷W5÷FW&×5÷&V6÷&B‡&WVW7B“ó°¢ÆWB7F÷&RÒ7FFRç7F÷&Ræ5÷&Vb‚’æöµö÷%öVÇ6R‡ÇÂ°¢vVçEö7F–öåöW'&÷"€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢'FW&×5÷7F÷&U÷Væf–Æ&ÆR"À¢$DD$4UõU$Â—2&WV—&VBFòV&Æ—6‚V&Æ–2&÷VçG’FW&×2"À¢G'VRÀ¢%&WG'’gFW"F†R†÷7FVBFW&×27F÷&R—2†VÇF‡“²Fòæ÷B7&VFRF†R&÷VçG’VçF–ÂV&Æ–6F–öâ7V66VVG2â"À¢¢Ò“ó°¢7F÷&P¢çW6W'EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2‚g&V6÷&B¢æv—@¢æÖöW'"‡ÆW'&÷'Â°¢vVçEö7F–öåöW'&÷"€¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢'FW&×5÷7F÷&U÷w&—FUöf–ÆVB"À¢W'&÷"çFõ÷7G&–ær‚’À¢G'VRÀ¢%&WG'’V&Æ–6F–öâv—F‚F†R–FVçF–6ÂFö7VÖVçBâFòæ÷BÇFW"F†RFö7VÖVçB÷"7&VFRF†R&÷VçG’VçF–ÂF†RFW&×2†6‚6â&R&VB&6²â"À¢¢Ò“ó°¢F—7G&–'WF–öã£¦&–æE÷FW&×5öGG&–'WF–öâ€¢g7FFRÀ¢f†VFW'2À¢g&V6÷&BçFW&×5ö†6‚À¢g&V6÷&Bæ7&VF÷%÷vÆÆWBÀ¢¢æv—@¢æÖöW'"‡Ç7FGW7Â°¢vVçEö7F–öåöW'&÷"€¢7FGW2À¢&F—7G&–'WF–öåöGG&–'WF–öåö&–æF–æuöf–ÆVB"À¢%F†R7WÆ–VB7V—6—F–öâæB†æFöfb–FVçF–f–W'2F–Bæ÷BÖF6‚öæRGW&&ÆR&W&VB†æFöfbâ"À¢7FGW2ÓÒ7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢%&WG'’FW&×2V&Æ–6F–öâv—F‚F†R–FVçF–6ÂFö7VÖVçBæB&÷F‚÷&–v–æÂGG&–'WF–öâ†VFW'2âGG&–'WF–öâ–FVçF–f–W'2w&çBæòvÆÆWB÷"–ÖVçBWF†÷&—G’â"À¢¢Ò“ó°¢ö²„§6öâ‡&V6÷&B’§Ð ¢5·WFö—£§F‚†vWBÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷FW&×2÷·FW&×5ö†6‡Ò"Â&×2‚‚'FW&×5ö†6‚"Ò7G&–ærÂF‚ÂFW67&—F–öâÒ#‚×&Vf—†VB¶V66²†6‚&WGW&æVB'’FW&×2V&Æ–6F–öâæB6öÖÖ—GFVBöâÖ6†–â"’’Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$6æöæ–6ÂV&Æ–2&÷VçG’FW&×2"’Â‡7FGW2ÒCBÂFW67&—F–öâÒ%Væ¶æ÷vâFW&×2†6‚"’’•Ð¦7–æ2fâvWEöWFöæöÖ÷W5ö&÷VçG•÷FW&×2€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚‡FW&×5ö†6‚“¢FƒÅ7G&–æsâÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W4&÷VçG•FW&×5&V6÷&CâÂ7FGW46öFSâ°¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢7F÷&P¢ævWEöWFöæöÖ÷W5ö&÷VçG•÷FW&×2‚gFW&×5ö†6‚¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ð¢æÖ„§6öâ¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB§Ð ¢5·WFö—£§F‚‡÷7BÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâÖWf–FVæ6R"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$–Ö×WF&ÆRV&Æ–2&V–ÖvW2ÖF6†–ærF†R7W'&VçB6æöæ–6Â7V&Ö—76–öäFFVB†6†W2"’’•Ð¦7–æ2fâV&Æ—6…öWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6R€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅV&Æ—6„WFöæöÖ÷W57V&Ö—76–öäWf–FVæ6U&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W57V&Ö—76–öäWf–FVæ6U&V6÷&CâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò&WVW7@¢ææWGv÷&°¢æ6ÆöæR‚¢çVçw&ö÷%öVÇ6R‡ÇÂ&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’“°¢ÆWB—FVÒÒ–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂfæWGv÷&²Âg&WVW7Bæ&÷VçG•ö6öçG&7B’æv—Có°¢ÆWB&V6÷&BÒWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6U÷&V6÷&B‚fæWGv÷&²Âf—FVÒÂ&WVW7B“ó°¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢7F÷&P¢çW6W'EöWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6R‚g&V6÷&B¢æv—@¢æÖ„§6öâ¢æÖöW'"‡ÆW'&÷'ÂÖF6‚W'&÷"°¢F$W'&÷#£¤WFöæöÖ÷W4Wf–FVæ6T6öæfÆ–7B…ò’Óâ7FGW46öFS£¤4ôädÄ”5BÀ¢òÓâ7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢Ò§Ð ¢5·WFö—£§F‚†vWBÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâÖWf–FVæ6R÷¶&÷VçG•ö6öçG&7GÒ÷·&÷VæGÒ"Â&×2‚‚&&÷VçG•ö6öçG&7B"Ò7G&–ærÂF‚ÂFW67&—F–öâÒ$6æöæ–6Â&÷VçG’6öçG&7B"’Â‚'&÷VæB"ÒScBÂF‚ÂFW67&—F–öâÒ%÷6—F—fR7V&Ö—76–öâ&÷VæB"’’Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$†6‚Ö6†V6¶VBV&Æ–27V&Ö—76–öâWf–FVæ6R"’Â‡7FGW2ÒCBÂFW67&—F–öâÒ$Wf–FVæ6Ræ÷BV&Æ—6†VB"’’•Ð¦7–æ2fâvWEöWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6R€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚‚†&÷VçG•ö6öçG&7BÂ&÷VæB’“¢FƒÂ…7G&–ærÂScB“âÀ¢VW'’‡VW'’“¢VW'“ÄWFöæöÖ÷W57V&Ö—76–öäWf–FVæ6UVW'“âÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W57V&Ö—76–öäWf–FVæ6U&V6÷&CâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²ÒVW'’ææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢–æFW†VEöWFöæöÖ÷W5ö&÷VçG’‚g7FFRÂæWGv÷&²Âf&÷VçG•ö6öçG&7B’æv—Có°¢7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ð¢ævWEöWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6R†æWGv÷&²Âf&÷VçG•ö6öçG&7BÂ&÷VæB¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ð¢æÖ„§6öâ¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB§Ð ¦fâWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6U÷&V6÷&B€¢æWGv÷&³¢g7G"À¢—FVÓ¢dWFöæöÖ÷W4&÷VçG”fVVD—FVÒÀ¢&WVW7C¢V&Æ—6„WFöæöÖ÷W57V&Ö—76–öäWf–FVæ6U&WVW7BÀ¢’Óâ&W7VÇCÄWFöæöÖ÷W57V&Ö—76–öäWf–FVæ6U&V6÷&BÂ7FGW46öFSâ°¢'V–ÆEöWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6U÷&V6÷&B€¢æWGv÷&²À¢—FVÒÀ¢g&WVW7Bæ&÷VçG•ö6öçG&7BÀ¢g&WVW7Bæ&÷VçG•ö–BÀ¢&WVW7Bç&÷VæBÀ¢g&WVW7Bç6öÇfW%÷vÆÆWBÀ¢g&WVW7Bæ'F–f7E÷&VfW&Væ6RÀ¢&WVW7BæWf–FVæ6RÀ¢WF3£¦æ÷r‚’À¢¢æÖöW'"‡Å÷Â7FGW46öFS£¤4ôädÄ”5B§Ð ¢5·WFö—£§F‚†vWBÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öfVVB"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$6æöæ–6ÂöâÖ6†–â&÷VçF–W2¦ö–æVBFò6öçFVçBÖFG&W76VBV&Æ–2FW&×2"’’•Ð¦7–æ2fâWFöæöÖ÷W5ö&÷VçG•öfVVB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢VW'’‡VW'’“¢VW'“ÄWFöæöÖ÷W4&÷VçG”fVVEVW'“âÀ¢’Óâ&W7VÇCÄ§6öãÅfV3ÄWFöæöÖ÷W4&÷VçG”fVVD—FVÓãâÂ7FGW46öFSâ°¢ÆöEöWFöæöÖ÷W5ö&÷VçG•öfVVB€¢g7FFRÀ¢VW'’ææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"’À¢VW'’æ6Æ–Ö&ÆUööæÇ’çVçw&ö÷"†fÇ6R’À¢¢æv—@¢æÖ„§6öâ§Ð ¦7–æ2fâÆöEöWFöæöÖ÷W5ö&÷VçG•öfVVB€¢7FFS¢e6†&VE7FFRÀ¢æWGv÷&³¢g7G"À¢6Æ–Ö&ÆUööæÇ“¢&ööÂÀ¢’Óâ&W7VÇCÅfV3ÄWFöæöÖ÷W4&÷VçG”fVVD—FVÓâÂ7FGW46öFSâ°¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBWfVçG2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•öWfVçG2†æWGv÷&²¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWBFW&×2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2‚¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWB×WBfVVBÒ'V–ÆEöWFöæöÖ÷W5ö&÷VçG•öfVVB†WfVçG2ÂFW&×2ÂfÇ6R¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢7FFRç&V6÷fW'•÷&W6W'fF–öç2æÇ’‚f×WBfVVBÂ6Æ–Ö&ÆUööæÇ’“°¢ö²†fVVB§Ð ¦7–æ2fâÆöE÷fW&–f–VEöWFöæöÖ÷W5ö&÷VçG•öfVVB€¢7FFS¢e6†&VE7FFRÀ¢æWGv÷&³¢g7G"À¢6Æ–Ö&ÆUööæÇ“¢&ööÂÀ¢’Óâ&W7VÇCÅfV3ÄWFöæöÖ÷W4&÷VçG”fVVD—FVÓâÂ7FGW46öFSâ°¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBWfVçG2Ò7F÷&P¢æÆ—7E÷fW&–f–VEöWFöæöÖ÷W5ö&÷VçG•öWfVçG2†æWGv÷&²¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWBFW&×2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2‚¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWB×WBfVVBÒ'V–ÆEöWFöæöÖ÷W5ö&÷VçG•öfVVB†WfVçG2ÂFW&×2ÂfÇ6R¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢7FFRç&V6÷fW'•÷&W6W'fF–öç2æÇ’‚f×WBfVVBÂ6Æ–Ö&ÆUööæÇ’“°¢ö²†fVVB§Ð ¢5·WFö—£§F‚€¢vWBÀ¢F‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öÆVFW&&ö&B"À¢&×2€¢‚&æWGv÷&²"Ò÷F–öãÅ7G&–æsâÂVW'’ÂFW67&—F–öâÒ$&6RæWGv÷&³²FVfVÇG2Fò&6RÖÖ–ææWB"’À¢‚&B"Ò÷F–öãÅ7G&–æsâÂVW'’ÂFW67&—F–öâÒ%$d3333’–ç7FçB6VÆV7F–ærF†RUD2F’æBÖöæF’×FòÕ7VæF’vVV²"¢’À¢&W7öç6W2‚‡7FGW2Ò#Â&öG’Ò6öÇfW$ÆVFW&&ö&E&W7öç6R’¢•Ð¦7–æ2fâ6öÇfW%öÆVFW&&ö&B€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢VW'’‡VW'’“¢VW'“Å6öÇfW$ÆVFW&&ö&EVW'“âÀ¢’Óâ&W7VÇCÄ§6öãÅ6öÇfW$ÆVFW&&ö&E&W7öç6SâÂ7FGW46öFSâ°¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBæWGv÷&²ÒVW'’ææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWBæWGv÷&µöFW67&—F÷"Ð¢&6UöæWGv÷&µöFW67&—F÷"†æWGv÷&²’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB&VfW&Væ6UöBÒVW'¢æ@¢æ5öFW&Vb‚¢æÖ„FFUF–ÖS£§'6Uög&öÕ÷&f3333’¢çG&ç7÷6R‚¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ð¢æÖ‡ÇfÇVWÂfÇVRçv—F…÷F–ÖW¦öæR‚eWF2’¢çVçw&ö÷%öVÇ6R…WF3£¦æ÷r“°¢ÆWBF–Ç•÷W&–öBÒÆVFW&&ö&E÷W&–öB„ÆVFW&&ö&EW&–öD¶–æC£¤F–Ç’Â&VfW&Væ6UöB“°¢ÆWBvVV¶Ç•÷W&–öBÒÆVFW&&ö&E÷W&–öB„ÆVFW&&ö&EW&–öD¶–æC£¥vVV¶Ç’Â&VfW&Væ6UöB“°¢ÆWB×WB6ö×ÆWF–öç2Ò7F÷&P¢æÆ—7Eö6æöæ–6Å÷6öÇfW%ö6ö×ÆWF–öç2†æWGv÷&²ÂvVV¶Ç•÷W&–öBç7F'G5öBÂvVV¶Ç•÷W&–öBæVæG5öB¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢6ö×ÆWF–öç2ç&WF–â‡Æ6ö×ÆWF–öçÂ°¢7FFP¢ç&V6÷fW'•÷&W6W'fF–öç0¢æ6öçF–ç2‚f6ö×ÆWF–öâæ&÷VçG•ö6öçG&7B¢Ò“°¢ÆWBF–Ç•÷&æ¶–ærÒ&æµ÷6öÇfW%ö6ö×ÆWF–öç2†F–Ç•÷W&–öBÂ6ö×ÆWF–öç2æ6ÆöæR‚’“°¢ÆWBvVV¶Ç•÷&æ¶–ærÒ&æµ÷6öÇfW%ö6ö×ÆWF–öç2‡vVV¶Ç•÷W&–öBÂ6ö×ÆWF–öç2“°¢ÆWB&Wv&Eö6öçG&7EöVçbÒÖF6‚æWGv÷&µöFW67&—F÷"æ6†–åö–B°¢…óCS2Óâ$$4UôÔ”ääUEôÄTDU$$ô$Eõ$Ut$Eô4ôåE$5B"À¢ƒEóS3"Óâ$$4Uõ4UôÄ”ôÄTDU$$ô$Eõ$Ut$Eô4ôåE$5B"À¢òÓâ&WGW&âW'"…7FGW46öFS£¤$Eõ$UTU5B’À¢Ó°¢ÆWB&Wv&Eö6öçG&7BÒVçc£§f"‡&Wv&Eö6öçG&7EöVçb¢æö²‚¢ææE÷F†Vâ†æöåöV×G•÷6V7&WB¢ææE÷F†Vâ‡ÇfÇVWÂæ÷&ÖÆ—¦UöWfÕöFG&W72‚gfÇVR’æö²‚’“°¢ÆWB×WB&Wv&E÷ööÂÒÖF6‚&Wv&Eö6öçG&7Bæ5öFW&Vb‚’°¢æöæRÓâ6öÇfW$ÆVFW&&ö&E&Wv&EööÅ&W7öç6R°¢6öçG&7C¢æöæRÀ¢6WGFÆVÖVçE÷Fö¶Vã¢æWGv÷&µöFW67&—F÷"ææF—fU÷W6F5÷Fö¶VåöFG&W72æ6ÆöæR‚’À¢gVæF–æu÷7FGW3¢&æ÷Eö6öæf–wW&VB"çFõ÷7G&–ær‚’À¢&Ææ6U÷W6F5ö&6U÷Væ—G3¢æöæRÀ¢&Ææ6U÷W6F3¢æöæRÀ¢7W'&VçEöF–Ç•öæE÷vVV¶Ç•÷&WV—&VE÷W6F3¢##’ã"çFõ÷7G&–ær‚’À¢Ö†–×VÕögVÆÅ÷vVV·5öEö7W'&VçEö&Ææ6S¢æöæRÀ¢ö'6W'fVE÷6fUö&Æö6³¢æöæRÀ¢ö'6W'fVE÷6fUö&Æö6µö†6ƒ¢æöæRÀ¢ö'6W'fF–öåöW'&÷#¢æöæRÀ¢Wf–FVæ6Uö&÷VæF'“¢$æò&Wv&B6öçG&7B—26öæf–wW&VBâ&æ¶–æw2&VÖ–â–æf÷&ÖF–öæÂæBæò&—¦R—2&W&W6VçFVB2gVæFVBâ"çFõ÷7G&–ær‚’À¢ÒÀ¢6öÖR†6öçG&7B’ÓâÖF6‚7FFRæ&6U÷'5÷W&Ç2ç&W6öÇfR†æWGv÷&²’°¢W'"…ò’Óâ6öÇfW$ÆVFW&&ö&E&Wv&EööÅ&W7öç6R°¢6öçG&7C¢6öÖR†6öçG&7BçFõ÷7G&–ær‚’’À¢6WGFÆVÖVçE÷Fö¶Vã¢æWGv÷&µöFW67&—F÷"ææF—fU÷W6F5÷Fö¶VåöFG&W72æ6ÆöæR‚’À¢gVæF–æu÷7FGW3¢'VçfW&–f–VB"çFõ÷7G&–ær‚’À¢&Ææ6U÷W6F5ö&6U÷Væ—G3¢æöæRÀ¢&Ææ6U÷W6F3¢æöæRÀ¢7W'&VçEöF–Ç•öæE÷vVV¶Ç•÷&WV—&VE÷W6F3¢##’ã"çFõ÷7G&–ær‚’À¢Ö†–×VÕögVÆÅ÷vVV·5öEö7W'&VçEö&Ææ6S¢æöæRÀ¢ö'6W'fVE÷6fUö&Æö6³¢æöæRÀ¢ö'6W'fVE÷6fUö&Æö6µö†6ƒ¢æöæRÀ¢ö'6W'fF–öåöW'&÷#¢6öÖR‚$&6R%2—2æ÷B6öæf–wW&VBâ"çFõ÷7G&–ær‚’’À¢Wf–FVæ6Uö&÷VæF'“¢%F†R&Wv&BFG&W72—26öæf–wW&VBÂ'WB—G2æF—fRU4D2&Ææ6Rv2æ÷BfW&–f–VBB&6R6fR&Æö6²â"çFõ÷7G&–ær‚’À¢ÒÀ¢ö²‚†FW67&—F÷"Â'5÷W&Â’’ÓâÖF6‚ö'6W'fUöW&3#ö&Ææ6U÷6fR€¢g'5÷W&ÂÀ¢fFW67&—F÷"ææF—fU÷W6F5÷Fö¶VåöFG&W72À¢6öçG&7BÀ¢“óÀ¢¢æv—@¢°¢W'"…ò’Óâ6öÇfW$ÆVFW&&ö&E&Wv&EööÅ&W7öç6R°¢6öçG&7C¢6öÖR†6öçG&7BçFõ÷7G&–ær‚’’À¢6WGFÆVÖVçE÷Fö¶Vã¢FW67&—F÷"ææF—fU÷W6F5÷Fö¶VåöFG&W72À¢gVæF–æu÷7FGW3¢'VçfW&–f–VB"çFõ÷7G&–ær‚’À¢&Ææ6U÷W6F5ö&6U÷Væ—G3¢æöæRÀ¢&Ææ6U÷W6F3¢æöæRÀ¢7W'&VçEöF–Ç•öæE÷vVV¶Ç•÷&WV—&VE÷W6F3¢##’ã"çFõ÷7G&–ær‚’À¢Ö†–×VÕögVÆÅ÷vVV·5öEö7W'&VçEö&Ææ6S¢æöæRÀ¢ö'6W'fVE÷6fUö&Æö6³¢æöæRÀ¢ö'6W'fVE÷6fUö&Æö6µö†6ƒ¢æöæRÀ¢ö'6W'fF–öåöW'&÷#¢6öÖR‚$&6R6fRÖ&Æö6²&Ææ6R&VBf–ÆVBâ"çFõ÷7G&–ær‚’’À¢Wf–FVæ6Uö&÷VæF'“¢%F†R&Wv&BFG&W72—26öæf–wW&VBÂ'WB—G2æF—fRU4D2&Ææ6Rv2æ÷BfW&–f–VBB&6R6fR&Æö6²â"çFõ÷7G&–ær‚’À¢ÒÀ¢ö²†ö'6W'fF–öâ’Óâ°¢ÆWBgVæF–æu÷7FGW2Ò–bö'6W'fF–öâæ&Ææ6RãÒ#•óó°¢&gVæFVB ¢ÒVÇ6R–bö'6W'fF–öâæ&Ææ6Râ°¢''F–ÆÇ•ögVæFVB ¢ÒVÇ6R°¢'VægVæFVB ¢Ó°¢6öÇfW$ÆVFW&&ö&E&Wv&EööÅ&W7öç6R°¢6öçG&7C¢6öÖR†ö'6W'fF–öâæ66÷VçB’À¢6WGFÆVÖVçE÷Fö¶Vã¢ö'6W'fF–öâçFö¶VâÀ¢gVæF–æu÷7FGW3¢gVæF–æu÷7FGW2çFõ÷7G&–ær‚’À¢&Ææ6U÷W6F5ö&6U÷Væ—G3¢6öÖR†ö'6W'fF–öâæ&Ææ6RçFõ÷7G&–ær‚’’À¢&Ææ6U÷W6F3¢6öÖR†f÷&ÖE÷W6F5ö&6U÷Væ—G2†ö'6W'fF–öâæ&Ææ6R’’À¢7W'&VçEöF–Ç•öæE÷vVV¶Ç•÷&WV—&VE÷W6F3¢##’ã"çFõ÷7G&–ær‚’À¢Ö†–×VÕögVÆÅ÷vVV·5öEö7W'&VçEö&Ææ6S¢6öÖR€¢ScC£§G'•ög&öÒ†ö'6W'fF–öâæ&Ææ6RòCuóó¢çVçw&ö÷"‡ScC£¤Ô‚’À¢’À¢ö'6W'fVE÷6fUö&Æö6³¢6öÖR†ö'6W'fF–öâç6fUö&Æö6µöçVÖ&W"’À¢ö'6W'fVE÷6fUö&Æö6µö†6ƒ¢6öÖR†ö'6W'fF–öâç6fUö&Æö6µö†6‚’À¢ö'6W'fF–öåöW'&÷#¢æöæRÀ¢Wf–FVæ6Uö&÷VæF'“¢%F†—2—2F†R&Wv&B6öçG&7Bw2æF—fRU4D2&Ææ6RBöæR&6R6fR&Æö6²âgVæFVB7FGW2ÖVç2F†R&Ææ6R6÷fW'2F†R6†÷vâ2U4D2F–Ç’æB#bU4D2vVV¶Ç’&—¦W2–bæòV&Æ–W"v&B6öç7VÖW2—Bf—'7BâöæÇ’F†R–B×v–ææW"&V6÷&BB&6R6fR&Æö6²&÷fW2&—¦R–ÖVçBâ"çFõ÷7G&–ær‚’À¢Ð¢Ð¢ÒÀ¢ÒÀ¢Ó°¢ÆWB†F–Ç•÷–÷WEöö'6W'fF–öâÂvVV¶Ç•÷–÷WEöö'6W'fF–öâ’ÒÖF6‚€¢&Wv&Eö6öçG&7Bæ5öFW&Vb‚’À¢7FFRæ&6U÷'5÷W&Ç2ç&W6öÇfR†æWGv÷&²’À¢’°¢…6öÖR†6öçG&7B’Âö²‚…òÂ'5÷W&Â’’’Óâ€¢ö'6W'fUöÆVFW&&ö&E÷–÷WB‚g'5÷W&ÂÂ6öçG&7BÂfF–Ç•÷&æ¶–ærÂ“ó¢æv—@¢æö²‚’À¢ö'6W'fUöÆVFW&&ö&E÷–÷WB‚g'5÷W&ÂÂ6öçG&7BÂgvVV¶Ç•÷&æ¶–ærÂ“ó#¢æv—@¢æö²‚’À¢’À¢òÓâ„æöæRÂæöæR’À¢Ó°¢–b&Wv&Eö6öçG&7Bæ—5÷6öÖR‚¢bb†F–Ç•÷–÷WEöö'6W'fF–öâæ—5öæöæR‚’ÇÂvVV¶Ç•÷–÷WEöö'6W'fF–öâæ—5öæöæR‚’¢°¢&Wv&E÷ööÂægVæF–æu÷7FGW2Ò'VçfW&–f–VB"çFõ÷7G&–ær‚“°¢&Wv&E÷ööÂæö'6W'fF–öåöW'&÷"Ò6öÖR€¢%F†R6öæf–wW&VBFG&W72F–Bæ÷B&WGW&â&÷F‚–B×v–ææW"&V6÷&G2B&6R6fR&Æö6²â ¢çFõ÷7G&–ær‚’À¢“°¢&Wv&E÷ööÂæWf–FVæ6Uö&÷VæF'’Ò%U4D2&Ææ6RÆöæRFöW2æ÷B&÷fRfÆ–B&Wv&B6öçG&7Bâ&æ¶–æw2&VÖ–â–æf÷&ÖF–öæÂVçF–ÂF†R&Ææ6RæB&÷F‚–B×v–ææW"vWGFW'2&RfW&–f–VBB&6R6fR&Æö6·2â"çFõ÷7G&–ær‚“°¢Ð¢ÆWB&Wv&EögVæF–æu÷7FGW2Ò&Wv&E÷ööÂægVæF–æu÷7FGW2æ6ÆöæR‚“°¢ÆWBvVæW&FVEöBÒWF3£¦æ÷r‚“° ¢ö²„§6öâ…6öÇfW$ÆVFW&&ö&E&W7öç6R°¢66†VÖ÷fW'6–öã¢&vVçBÖ&÷VçF–W2÷6öÇfW"ÖÆVFW&&ö&B×c"çFõ÷7G&–ær‚’À¢æWGv÷&³¢æWGv÷&²çFõ÷7G&–ær‚’À¢vVæW&FVEöBÀ¢&VfW&Væ6UöBÀ¢&Wv&E÷ööÂÀ¢F–Ç“¢ÆVFW&&ö&E÷W&–öE÷&W7öç6R€¢F–Ç•÷&æ¶–ærÀ¢vVæW&FVEöBÀ¢&Wv&Eö6öçG&7Bæ6ÆöæR‚’À¢g&Wv&EögVæF–æu÷7FGW2À¢F–Ç•÷–÷WEöö'6W'fF–öâÀ¢’À¢vVV¶Ç“¢ÆVFW&&ö&E÷W&–öE÷&W7öç6R€¢vVV¶Ç•÷&æ¶–ærÀ¢vVæW&FVEöBÀ¢&Wv&Eö6öçG&7BÀ¢g&Wv&EögVæF–æu÷7FGW2À¢vVV¶Ç•÷–÷WEöö'6W'fF–öâÀ¢’À¢æW‡Eö7F–öã¢$6Æ–ÒgVæFVB&÷VçG’v÷'F‚BÆV7B"U4D2Â6ö×ÆWFR—BÂæB6öæf—&Ò&÷VçG•6WGFÆVB&Vf÷&RF†RW&–öBVæG2â"çFõ÷7G&–ær‚’À¢Wf–FVæ6Uö&÷VæF'“¢%&æ¶–æw26÷VçB–æFW†VB6æöæ–6Â6WGFÆVÖVçG2v—F‚fW&–f–VB&6R&Æö6²F–ÖRâ6öæf–wW&VB÷"gVæFVB&Wv&B—2æ÷B–ÖVçBâöæÇ’6öæf—&ÖVB&Wv&BG&ç6fW"&÷fW2&—¦R–ÖVçBâ"çFõ÷7G&–ær‚’À¢Ò’§Ð ¦7–æ2fâö'6W'fUöÆVFW&&ö&E÷–÷WB€¢'5÷W&Ã¢g7G"À¢6öçG&7C¢g7G"À¢&æ¶–æs¢e6öÇfW$ÆVFW&&ö&E&æ¶–ærÀ¢&WVW7Eö–C¢ScBÀ¢’Óâ&W7VÇCÅ6öÇfW$ÆVFW&&ö&Dv&E6fTö'6W'fF–öâÂ6†–ä&6TW'&÷#â°¢ÆWBW&–öEö¶–æBÒÖF6‚&æ¶–ærçW&–öBæ¶–æB°¢ÆVFW&&ö&EW&–öD¶–æC£¤F–Ç’ÓâÀ¢ÆVFW&&ö&EW&–öD¶–æC£¥vVV¶Ç’ÓâÀ¢Ó°¢ÆWB7F'G5öBÒScC£§G'•ög&öÒ‡&æ¶–ærçW&–öBç7F'G5öBçF–ÖW7F×‚’’æÖöW'"‡Å÷Â°¢6†–ä&6TW'&÷#£¤–çfÆ–EfW&–f–6F–öä6öæf–wW&F–öâ€¢&ÆVFW&&ö&BW&–öB7F'G2&Vf÷&RVæ—‚Wö6‚"çFõ÷7G&–ær‚’À¢¢Ò“ó°¢ÆWBv&Eö–BÒ6öÇfW%öÆVFW&&ö&Eöv&Eö–B‡W&–öEö¶–æBÂ7F'G5öB“ó°¢ö'6W'fU÷6öÇfW%öÆVFW&&ö&E÷–E÷v–ææW%÷6fR‡'5÷W&ÂÂ6öçG&7BÂfv&Eö–BÂ&WVW7Eö–B’æv—@§Ð ¦fâÆVFW&&ö&E÷W&–öE÷&W7öç6R€¢&æ¶–æs¢6öÇfW$ÆVFW&&ö&E&æ¶–ærÀ¢æ÷s¢FFUF–ÖSÅWF3âÀ¢&Wv&Eö6öçG&7C¢÷F–öãÅ7G&–æsâÀ¢&Wv&EögVæF–æu÷7FGW3¢g7G"À¢–÷WEöö'6W'fF–öã¢÷F–öãÅ6öÇfW$ÆVFW&&ö&Dv&E6fTö'6W'fF–öãâÀ¢’Óâ6öÇfW$ÆVFW&&ö&EW&–öE&W7öç6R°¢ÆWB6Æ÷6VBÒæ÷rãÒ&æ¶–ærçW&–öBæVæG5öC°¢ÆWB†5÷v–ææW"Ò&æ¶–æræÆVFW%÷vÆÆWBæ—5÷6öÖR‚“°¢ÆWB–E÷vÆÆWBÒ–÷WEöö'6W'fF–öà¢æ5÷&Vb‚¢ææE÷F†Vâ‡Æö'6W'fF–öçÂö'6W'fF–öâç–E÷v–ææW"æ6ÆöæR‚’“°¢ÆWB–÷WE÷7FGW2Ò–b6Æ÷6VB°¢&æ÷EöGVR ¢ÒVÇ6R–b†5÷v–ææW"°¢&æõ÷v–ææW" ¢ÒVÇ6R–bÆWB6öÖR‡–B’Ò–E÷vÆÆWBæ5öFW&Vb‚’°¢–b&æ¶–æp¢æÆVFW%÷vÆÆW@¢æ5öFW&Vb‚¢æ—5÷6öÖUöæB‡ÆÆVFW'ÂÆVFW"æWö–væ÷&Uö66–•ö66R‡–B’¢°¢'–B ¢ÒVÇ6R°¢'–E÷FõöF–ffW&VçE÷vÆÆWB ¢Ð¢ÒVÇ6R–b&Wv&Eö6öçG&7Bæ—5öæöæR‚’°¢'&Wv&Eöæ÷Eö6öæf–wW&VB ¢ÒVÇ6R–b–÷WEöö'6W'fF–öâæ—5öæöæR‚’°¢'–÷WE÷VçfW&–f–VB ¢ÒVÇ6R–b&Wv&EögVæF–æu÷7FGW2Ò&gVæFVB"°¢&v—F–æu÷fW&–f–VEögVæF–ær ¢ÒVÇ6R°¢&v—F–æuöf–æÆ—¦F–öâ ¢Ó°¢6öÇfW$ÆVFW&&ö&EW&–öE&W7öç6R°¢W&–öE÷7FGW3¢–b6Æ÷6VB²&6Æ÷6VB"ÒVÇ6R²&÷Vâ"ÒçFõ÷7G&–ær‚’À¢&Wv&E÷W6F3¢f÷&ÖE÷W6F5ö&6U÷Væ—G2‡&æ¶–ærçW&–öBæ¶–æBç&Wv&E÷W6F5ö&6U÷Væ—G2‚’æ–çFò‚’’À¢&Wv&EögVæF–æu÷7FGW3¢&Wv&EögVæF–æu÷7FGW2çFõ÷7G&–ær‚’À¢&Wv&E÷–÷WE÷7FGW3¢–÷WE÷7FGW2çFõ÷7G&–ær‚’À¢&Wv&Eö6öçG&7BÀ¢&Wv&E÷–E÷vÆÆWC¢–E÷vÆÆWBÀ¢&Wv&E÷–÷WEöö'6W'fVE÷6fUö&Æö6³¢–÷WEöö'6W'fF–öà¢æ5÷&Vb‚¢æÖ‡Æö'6W'fF–öçÂö'6W'fF–öâç6fUö&Æö6µöçVÖ&W"’À¢&Wv&E÷–÷WEöö'6W'fVE÷6fUö&Æö6µö†6ƒ¢–÷WEöö'6W'fF–öà¢æÖ‡Æö'6W'fF–öçÂö'6W'fF–öâç6fUö&Æö6µö†6‚’À¢&æ¶–ærÀ¢Ð§Ð ¢5·WFö—£§F‚€¢vWBÀ¢F‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö–çfVçF÷'’×7VÖÖ'’"À¢&W7öç6W2‚‡7FGW2Ò#Â&öG’ÒWFöæöÖ÷W4&÷VçG”–çfVçF÷'•7VÖÖ'’’¢•Ð¦7–æ2fâWFöæöÖ÷W5ö&÷VçG•ö–çfVçF÷'•÷7VÖÖ'’€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢VW'’‡VW'’“¢VW'“ÄWFöæöÖ÷W4&÷VçG”fVVEVW'“âÀ¢’Óâ&W7VÇCÄ§6öãÄWFöæöÖ÷W4&÷VçG”–çfVçF÷'•7VÖÖ'“âÂ7FGW46öFSâ°¢ÆWBæWGv÷&²ÒVW'’ææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWBfVVBÐ¢ÆöEöWFöæöÖ÷W5ö&÷VçG•öfVVB‚g7FFRÂæWGv÷&²ÂVW'’æ6Æ–Ö&ÆUööæÇ’çVçw&ö÷"‡G'VR’’æv—Có°¢'V–ÆEöWFöæöÖ÷W5ö–çfVçF÷'•÷7VÖÖ'’‚g7FFRÂæWGv÷&²ÂfVVB’æÖ„§6öâ§Ð ¢5·WFö—£§F‚€¢vWBÀ¢F‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö–çfVçF÷'’Ö&FvRç7fr"À¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$Æ—fR6æöæ–6Â6Æ–Ö&ÆR–çfVçF÷'’&FvR"’¢•Ð¦7–æ2fâWFöæöÖ÷W5ö&÷VçG•ö–çfVçF÷'•ö&FvR€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢VW'’‡VW'’“¢VW'“ÄWFöæöÖ÷W4&÷VçG”fVVEVW'“âÀ¢’Óâ&W7VÇCÅ&W7öç6RÂ7FGW46öFSâ°¢ÆWBæWGv÷&²ÒVW'’ææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWBfVVBÒÆöEöWFöæöÖ÷W5ö&÷VçG•öfVVB‚g7FFRÂæWGv÷&²ÂG'VR’æv—Có°¢ÆWB7VÖÖ'’Ò'V–ÆEöWFöæöÖ÷W5ö–çfVçF÷'•÷7VÖÖ'’‚g7FFRÂæWGv÷&²ÂfVVB“ó°¢ÆWBÖW76vRÒf÷&ÖB€¢'·Ò6Æ–Ö&ÆRÂ·ÒU4D2"À¢7VÖÖ'’æ6Æ–Ö&ÆUö&÷VçG•ö6÷VçBÂ7VÖÖ'’ægVæFVE÷W6F0¢“°¢ÆWB7frÒf÷&ÖB€¢"22#Ç7fr†ÖÆç3Ò&‡GG¢ò÷wwrçs2æ÷&ró#÷7fr"v–GFƒÒ##“"†V–v‡CÒ##"&öÆSÒ&–Ör"&–ÖÆ&VÃÒ$vVçB&÷VçF–W3¢¶ÖW76vWÒ#ãÇF—FÆSävVçB&÷VçF–W3¢¶ÖW76vWÓÂ÷F—FÆSãÆÆ–æV$w&F–VçB–CÒ'2"ƒ#Ò#"“#Ò#R#ãÇ7F÷öfg6WCÒ#"7F÷Ö6öÆ÷#Ò"6ffb"7F÷Ö÷6—G“Ò"ãb"óãÇ7F÷öfg6WCÒ#"7F÷Ö÷6—G“Ò"ã‚"óãÂöÆ–æV$w&F–VçCãÆ6Æ—F‚–CÒ'"#ãÇ&V7Bv–GFƒÒ##“"†V–v‡CÒ##"'ƒÒ#2"f–ÆÃÒ"6ffb"óãÂö6Æ—FƒãÆr6Æ—×FƒÒ'W&Â‚7"’#ãÇ&V7Bv–GFƒÒ#"†V–v‡CÒ##"f–ÆÃÒ"3##c&R"óãÇ&V7BƒÒ#"v–GFƒÒ#ƒ"†V–v‡CÒ##"f–ÆÃÒ"3ƒvcV""óãÇ&V7Bv–GFƒÒ##“"†V–v‡CÒ##"f–ÆÃÒ'W&Â‚72’"óãÂösãÆrf–ÆÃÒ"6ffb"FW‡BÖæ6†÷#Ò&Ö–FFÆR"föçBÖfÖ–Ç“Ò%fW&FæÄvVæWfÄFV¦gR6ç2Ç6ç2×6W&–b"föçB×6—¦SÒ##ãÇFW‡BƒÒ#SR"“Ò#R"f–ÆÃÒ"3"f–ÆÂÖ÷6—G“Ò"ã2#æ–çfVçF÷'“Â÷FW‡CãÇFW‡BƒÒ#SR"“Ò#B#æ–çfVçF÷'“Â÷FW‡CãÇFW‡BƒÒ##"“Ò#R"f–ÆÃÒ"3"f–ÆÂÖ÷6—G“Ò"ã2#ç¶ÖW76vWÓÂ÷FW‡CãÇFW‡BƒÒ##"“Ò#B#ç¶ÖW76vWÓÂ÷FW‡CãÂösãÂ÷7fsâ"20¢“°¢ö²‚€¢°¢††VFW#£¤4ôåDTåEõE•RÂ&–ÖvR÷7fr·†ÖÃ²6†'6WC×WFbÓ‚"’À¢€¢†VFW#£¤44„Uô4ôåE$ôÂÀ¢'V&Æ–2ÂÖ‚ÖvSÓRÂ7FÆR×v†–ÆR×&WfÆ–FFSÓCR"À¢’À¢ÒÀ¢7frÀ¢¢æ–çFõ÷&W7öç6R‚’§Ð ¦fâ'V–ÆEöWFöæöÖ÷W5ö–çfVçF÷'•÷7VÖÖ'’€¢7FFS¢e6†&VE7FFRÀ¢æWGv÷&³¢g7G"À¢fVVC¢fV3ÄWFöæöÖ÷W4&÷VçG”fVVD—FVÓâÀ¢’Óâ&W7VÇCÄWFöæöÖ÷W4&÷VçG”–çfVçF÷'•7VÖÖ'’Â7FGW46öFSâ°¢ÆWB7VÒÒÆf–VÆC¢fâ‚dWFöæöÖ÷W4&÷VçG”fVVD—FVÒ’Óâg7G'Â°¢fVVBæ—FW"‚’çG'•öföÆBƒ÷S#‚ÂÇF÷FÂÂ—FV×Â°¢f–VÆB†—FVÒ¢ç'6S££ÇS#ƒâ‚¢æö²‚¢ææE÷F†Vâ‡ÆÖ÷VçGÂF÷FÂæ6†V6¶VEöFB†Ö÷VçB’¢æöµö÷"…7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"¢Ò¢Ó°¢ÆWBgVæFVBÒ7VÒ‡Æ—FV×Âf—FVÒægVæFVEöÖ÷VçB“ó°¢ÆWB6öÇfW"Ò7VÒ‡Æ—FV×Âf—FVÒç6öÇfW%÷&Wv&B“ó°¢ÆWBfW&–f–W"Ò7VÒ‡Æ—FV×Âf—FVÒçfW&–f–W%÷&Wv&B“ó°¢ÆWBfW&–f–6F–öå÷&VG•ö&÷VçG•ö6÷VçBÐ¢fVVBæ—FW"‚’æf–ÇFW"‡Æ—FV×Â—FVÒçfW&–f–6F–öå÷&VG’’æ6÷VçB‚“°¢ÆWB7FæF–æuöÖWFö&÷VçG•ö6÷VçBÒfVV@¢æ—FW"‚¢æf–ÇFW"‡Æ—FV×Â7FæF–æuöÖWF÷c%÷&VçEö6öçFW‡B†—FVÒ’æ—5öö²‚’¢æ6÷VçB‚“°¢ÆWB—FV×2ÒfVV@¢æ—FW"‚¢æÖ‡Æ—FV×ÂWFöæöÖ÷W4&÷VçG”–çfVçF÷'”—FVÒ°¢&÷VçG•ö–C¢—FVÒæ&÷VçG•ö–Bæ6ÆöæR‚’À¢&÷VçG•ö6öçG&7C¢—FVÒæ&÷VçG•ö6öçG&7Bæ6ÆöæR‚’À¢F—FÆS¢—FVÐ¢çFW&×0¢æ5÷&Vb‚¢æÖ‡ÇFW&×7ÂFW&×2æFö7VÖVçBçF—FÆRæ6ÆöæR‚’’À¢7FGW3¢—FVÒç7FGW2æ6ÆöæR‚’À¢gVæFVE÷W6F5ö&6U÷Væ—G3¢—FVÒægVæFVEöÖ÷VçBæ6ÆöæR‚’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢—FVÒç6öÇfW%÷&Wv&Bæ6ÆöæR‚’À¢fW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢—FVÒçfW&–f–W%÷&Wv&Bæ6ÆöæR‚’À¢fW&–f–6F–öå÷&VG“¢—FVÒçfW&–f–6F–öå÷&VG’À¢7FæF–æuöÖWFö&÷VçG“¢7FæF–æuöÖWF÷c%÷&VçEö6öçFW‡B†—FVÒ’æ—5öö²‚’À¢Ò¢æ6öÆÆV7B‚“°¢ö²„WFöæöÖ÷W4&÷VçG”–çfVçF÷'•7VÖÖ'’°¢66†VÖ÷fW'6–öã¢&vVçBÖ&÷VçF–W2ö–çfVçF÷'’×7VÖÖ'’×c"çFõ÷7G&–ær‚’À¢æWGv÷&³¢æWGv÷&²çFõ÷7G&–ær‚’À¢vVæW&FVEöC¢WF3£¦æ÷r‚’çFõ÷&f3333’‚’À¢6æöæ–6Å÷6÷W&6S¢f÷&ÖB€¢'·Ò÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öfVVCöæWGv÷&³×¶æWGv÷&·Òf6Æ–Ö&ÆUööæÇ“×G'VR"À¢7FFRçV&Æ–5ö&6U÷W&ÂçG&–ÕöVæEöÖF6†W2‚ròr¢’À¢6Æ–Ö&ÆUö&÷VçG•ö6÷VçC¢fVVBæÆVâ‚’À¢fW&–f–6F–öå÷&VG•ö&÷VçG•ö6÷VçBÀ¢7FæF–æuöÖWFö&÷VçG•ö6÷VçBÀ¢gVæFVE÷W6F5ö&6U÷Væ—G3¢gVæFVBçFõ÷7G&–ær‚’À¢gVæFVE÷W6F3¢f÷&ÖE÷W6F5ö&6U÷Væ—G2†gVæFVB’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢6öÇfW"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F3¢f÷&ÖE÷W6F5ö&6U÷Væ—G2‡6öÇfW"’À¢fW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢fW&–f–W"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&E÷W6F3¢f÷&ÖE÷W6F5ö&6U÷Væ—G2‡fW&–f–W"’À¢—FV×2À¢Wf–FVæ6Uö&÷VæF'“¢%F†—27VÖÖ'’—2FW&—fVBB&WVW7BF–ÖRg&öÒ6öæf—&ÖVB6æöæ–6ÂWfVçG2æBfÆ–FFVB6öçFVçBÖFG&W76VBFW&×2–âF†R†÷7FVB–æFW‚â—B&÷fW27W'&VçB–æFW†VB–çfVçF÷'’Âæ÷BgWGW&R6Æ–ÒÂ6ö×ÆWF–öâÂ÷"–÷WBâöæÇ’&÷VçG•6WGFÆVB&÷fW2–ÖVçBâ"çFõ÷7G&–ær‚’À¢Ò§Ð ¦7–æ2fâ'V–ÆEö÷Våö6ö×WF—F–öåö–çfVçF÷'•÷7VÖÖ'’€¢7FFS¢e6†&VE7FFRÀ¢æWGv÷&³¢g7G"À¢’Óâ&W7VÇCÄ÷Vä6ö×WF—F–öä–çfVçF÷'•7VÖÖ'’Â7FGW46öFSâ°¢ÆWB†FW67&—F÷"Âò’Ò7FFP¢æ&6U÷'5÷W&Ç0¢ç&W6öÇfR†æWGv÷&²¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB&VÆV6RÒ÷Våö6ö×WF—F–öå÷&VÆV6Uög&öÕöVçf—&öæÖVçB†æWGv÷&²¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢–b&VÆV6RæFWÆ÷–ÖVçE÷7FFRÒ÷Vä6ö×WF—F–öäFWÆ÷–ÖVçE7FFS£¤7F—fU&VG•FôV&â°¢&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ð¢ÆWB&Vf—‚Ò÷Våö6ö×WF—F–öåöVçf—&öæÖVçE÷&Vf—‚†æWGv÷&²¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBV&Æ–5ö7F—fF–öåö&Æö6²ÒVçc£§f"†f÷&ÖB‚'·&Vf—‡ÕõT$Ä”5ô5D•dD”ôåô$Äô4²"’¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ð¢ç'6S££ÇScCâ‚¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWB6FÆörÒ÷Våö6ö×WF—F–öå÷fW&–f–W%ö6FÆöuög&öÕöVçf—&öæÖVçB†æWGv÷&²¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWB·&öf–ÆUÒÒ6FÆörç&öf–ÆW2æ5÷6Æ–6R‚’VÇ6R°¢&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ó°¢–b&öf–ÆRçV&Æ–5ö–çfVçF÷'•öVÆ–v–&ÆP¢ÇÂ&öf–ÆRæFWÆ÷–ÖVçE÷7FFRÒ÷Vä6ö×WF—F–öäFWÆ÷–ÖVçE7FFS£¤7F—fU&VG•FôV&à¢°¢&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ð¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBWfVçG2Ò7F÷&P¢æÆ—7E÷fW&–f–VEö÷Våö6ö×WF—F–öåöWfVçG2†æWGv÷&²Âg&VÆV6Ræf7F÷'•ö6öçG&7B¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBvVæW&FVEöBÒWF3£¦æ÷r‚“°¢ÆWBvV'6—FUö&6U÷W&ÂÐ¢ÆVvÅ÷vV'6—FUö&6U÷W&Â†Vçc£§f"‚%tT%4•DUô$4UõU$Â"’æö²‚’Âg7FFRçV&Æ–5ö&6U÷W&Â“°¢ÆWB—FV×2Ò÷Våö6ö×WF—F–öåöF—66÷fW'•ö—FV×2€¢fWfVçG2À¢&öf–ÆRÀ¢æWGv÷&²À¢FW67&—F÷"æ6†–åö–BÀ¢g7FFRçV&Æ–5ö&6U÷W&ÂÀ¢gvV'6—FUö&6U÷W&ÂÀ¢V&Æ–5ö7F—fF–öåö&Æö6²À¢vVæW&FVEöBÀ¢¢æÖöW'"‡Å÷Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWB&VG’Ò—FV×2æ—FW"‚’æf–ÇFW"‡Æ—FV×Â—FVÒç&VG•÷FõöV&â“°¢ÆWB7VÒÒÆf–VÆC¢fâ‚fv—F‡V%öF—66÷fW'“£¤v—D‡V$F—66÷fW'”—FVÒ’Óâg7G'Â°¢&VG’æ6ÆöæR‚’çG'•öföÆBƒ÷S#‚ÂÇF÷FÂÂ—FV×Â°¢f–VÆB†—FVÒ¢ç'6S££ÇS#ƒâ‚¢æö²‚¢ææE÷F†Vâ‡ÆÖ÷VçGÂF÷FÂæ6†V6¶VEöFB†Ö÷VçB’¢æöµö÷"…7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"¢Ò¢Ó°¢ö²„÷Vä6ö×WF—F–öä–çfVçF÷'•7VÖÖ'’°¢vVæW&FVEöC¢vVæW&FVEöBçFõ÷&f3333’‚’À¢&VG•÷FõöV&åö6÷VçC¢—FV×2æ—FW"‚’æf–ÇFW"‡Æ—FV×Â—FVÒç&VG•÷FõöV&â’æ6÷VçB‚’À¢gVæFVE÷W6F5ö&6U÷Væ—G3¢7VÒ‡Æ—FV×Âf—FVÒægVæFVE÷W6F5ö&6U÷Væ—G2“òçFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢7VÒ‡Æ—FV×Âf—FVÒç&Wv&E÷W6F5ö&6U÷Væ—G2“òçFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢7VÒ‡Æ—FV×Âf—FVÒçfW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G2“ð¢çFõ÷7G&–ær‚’À¢Ò§Ð ¦7–æ2fâ'V–ÆEö÷Våö6ö×WF—F–öå÷c%ö–çfVçF÷'•÷7VÖÖ'’€¢7FFS¢e6†&VE7FFRÀ¢æWGv÷&³¢g7G"À¢’Óâ&W7VÇCÄ÷Vä6ö×WF—F–öä–çfVçF÷'•7VÖÖ'’Â7FGW46öFSâ°¢ÆWBvVæW&FVEöBÒWF3£¦æ÷r‚“°¢ÆWB—FV×2ÒÆöE÷V&Æ–5ö÷Våö6ö×WF—F–öå÷c%ö÷÷'GVæ—F–W2€¢7FFRÀ¢æWGv÷&²À¢7FFRçV&Æ–5ö&6U÷W&ÂçG&–ÕöVæEöÖF6†W2‚ròr’À¢vVæW&FVEöBÀ¢¢æv—Có°¢ÆWB&VG’Ò—FV×0¢æ—FW"‚¢æf–ÇFW"‡Æ—FV×Â°¢—FVÒçv÷&µ÷7FFRÓÒ&6Æ–Ö&ÆR ¢bb—FVÒç–ÖVçE÷7FFRÓÒ&W67&÷vVB ¢bb—FVÒç–ÖVçEö6öÖÖ—GFV@¢bb—FVÒçfW&–f–6F–öå÷&VG¢bb—FVÐ¢æ66…öV6öæöÖ–70¢æ5÷&Vb‚¢æ—5÷6öÖUöæB‡ÆV6öæöÖ–77ÂV6öæöÖ–72æw&÷75ö66…öÖ&v–å÷÷6—F—fR¢Ò¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢ÆWB7VÒÒÆf–VÆC¢fâ‚d÷÷'GVæ—G”—FVÒ’Óâg7G'Â°¢&VG’æ—FW"‚’çG'•öföÆBƒ÷S#‚ÂÇF÷FÂÂ—FV×Â°¢f–VÆB†—FVÒ¢ç'6S££ÇS#ƒâ‚¢æö²‚¢ææE÷F†Vâ‡ÆÖ÷VçGÂF÷FÂæ6†V6¶VEöFB†Ö÷VçB’¢æöµö÷"…7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"¢Ò¢Ó°¢ö²„÷Vä6ö×WF—F–öä–çfVçF÷'•7VÖÖ'’°¢vVæW&FVEöC¢vVæW&FVEöBçFõ÷&f3333’‚’À¢&VG•÷FõöV&åö6÷VçC¢&VG’æÆVâ‚’À¢gVæFVE÷W6F5ö&6U÷Væ—G3¢7VÒ‡Æ—FV×Âf—FVÒægVæFVEöÖ÷VçBæÖ÷VçB“òçFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢7VÒ‡Æ—FV×Âf—FVÒç&Wv&BæÖ÷VçB“òçFõ÷7G&–ær‚’À¢òò÷Vâ6ö×WF—F–öâc"&W6W'fW2¶VWW"&Wv&B&F†W"F†âfW&–f–W ¢òò&Wv&Bâ¶VWF†—2W†—7F–ær&öÆR×7V6–f–2V&Æ–2f–VÆBG'WF†gVÂv†–ÆP¢òòF†RgVÆÂW67&÷r&VÖ–ç2–æ6ÇVFVB–âf–Æ&ÆUögVæF–æu÷W6F2à¢fW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢Ò§Ð ¦fâf÷&ÖE÷W6F5ö&6U÷Væ—G2†Ö÷VçC¢S#‚’Óâ7G&–ær°¢f÷&ÖB€¢'·Òç³£'Ò"À¢Ö÷VçBòóóÀ¢†Ö÷VçBRóó’òó ¢§Ð ¢5·WFö—£§F‚†vWBÂF‚Ò"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷fW&–f–6F–öâÖ¦ö'2"Â&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$Æ—fRfW&–f–W"¦ö'2¦ö–æVBFò–Ö×WF&ÆRFW&×2æB†6‚ÖÖF6†VBWf–FVæ6R&V–ÖvW2"’’•Ð¦7–æ2fâWFöæöÖ÷W5÷fW&–f–6F–öåö¦ö'2€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢VW'’‡VW'’“¢VW'“ÄWFöæöÖ÷W5fW&–f–6F–öä¦ö'5VW'“âÀ¢’Óâ&W7VÇCÄ§6öãÅfV3ÄWFöæöÖ÷W5fW&–f–6F–öä¦ö#ãâÂ7FGW46öFSâ°¢ÆWB7F÷&RÒ7FFP¢ç7F÷&P¢æ5÷&Vb‚¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢ÆWBæWGv÷&²ÒVW'’ææWGv÷&²æ5öFW&Vb‚’çVçw&ö÷"‚&&6RÖÖ–ææWB"“°¢ÆWBWfVçG2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•öWfVçG2†æWGv÷&²¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWBFW&×2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2‚¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWBWf–FVæ6RÒ7F÷&P¢æÆ—7EöWFöæöÖ÷W5÷7V&Ö—76–öåöWf–FVæ6R†æWGv÷&²¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢ÆWB×WBfVVBÒ'V–ÆEöWFöæöÖ÷W5ö&÷VçG•öfVVB†WfVçG2ÂFW&×2ÂfÇ6R¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢7FFP¢ç&V6÷fW'•÷&W6W'fF–öç0¢æW†6ÇVFUög&öÕ÷fW&–f–6F–öåö¦ö'2‚f×WBfVVB“°¢ÆWBö'6W'fVEöBÒScC£§G'•ög&öÒ…WF3£¦æ÷r‚’çF–ÖW7F×‚’’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB×WB¦ö'2Ò'V–ÆEöWFöæöÖ÷W5÷fW&–f–6F–öåö¦ö'2†æWGv÷&²ÂfVVBÂWf–FVæ6RÂö'6W'fVEöB¢æÖöW'"‡Å÷Â7FGW46öFS£¤4ôädÄ”5B“ó°¢–bÆWB6öÖR‡fW&–f–W"’ÒVW'’çfW&–f–W"°¢ÆWBfW&–f–W"Òæ÷&ÖÆ—¦UöWfÕöFG&W72‡fW&–f–W"’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢¦ö'2ç&WF–â‡Æ¦ö'Â°¢¦ö"çfW&–f–6F–öåöÖöFRÓÒ&FWFW&Ö–æ—7F–5öÖöGVÆR ¢ÇÂ¦ö ¢æVÆ–v–&ÆU÷fW&–f–W'0¢æ—FW"‚¢æç’‡Æ6æF–FFWÂ6æF–FFRæWö–væ÷&Uö66–•ö66R‚gfW&–f–W"’¢Ò“°¢Ð¢ö²„§6öâ†¦ö'2’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—Rö6†V6¶÷WB×F÷×W2"À¢&WVW7Eö&öG’ÒÆå7G&—T6†V6¶÷WEF÷W&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ%7G&—R6†V6¶÷WB6W76–öâ&WVW7B–çFVçB"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–BF÷×W&WVW7B÷"Ö÷VçB&VÆ÷r7G&—RÖ–æ–×VÒ"¢¢•Ð¦7–æ2fâÆå÷7G&—Uö6†V6¶÷WE÷F÷÷W€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆå7G&—T6†V6¶÷WEF÷W&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÇ–ÖVçG5÷7G&—S£¥7G&—U&WVW7D–çFVçCâÂ7FGW46öFSâ°¢7G&—Uö6†V6¶÷WE÷F÷÷Wö–çFVçB‚g7FFRÂ&WVW7B’æÖ„§6öâ§Ð ¦fâ7G&—Uö6†V6¶÷WE÷F÷÷Wö–çFVçB€¢7FFS¢e6†&VE7FFRÀ¢&WVW7C¢Æå7G&—T6†V6¶÷WEF÷W&WVW7BÀ¢’Óâ&W7VÇCÅ7G&—U&WVW7D–çFVçBÂ7FGW46öFSâ°¢ÆWBÆå7G&—T6†V6¶÷WEF÷W&WVW7B°¢÷&væ—¦F–öåö–BÀ¢Ö÷VçEöÖ–æ÷"À¢7W'&Væ7’À¢7V66W75÷W&ÂÀ¢6æ6VÅ÷W&ÂÀ¢ÒÒ&WVW7C°¢ÆWBÆFf÷&Õö&6U÷W&ÂÒ7FFRçV&Æ–5ö&6U÷W&Âæ6ÆöæR‚“°¢ÆWBÆææW"Ò7G&—U÷ÆææW%öf÷%÷7FFR‡7FFRÂÆFf÷&Õö&6U÷W&Âæ6ÆöæR‚’“°¢ÆWBÖ÷VçBÒÖöæW“£¦æWr†Ö÷VçEöÖ–æ÷"Â7W'&Væ7’’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆææW ¢æ6†V6¶÷WE÷F÷÷W‚d6†V6¶÷WEF÷W&WVW7B°¢÷&væ—¦F–öåö–BÀ¢Ö÷VçBÀ¢7V66W75÷W&Ã¢7V66W75÷W&À¢çVçw&ö÷%öVÇ6R‡ÇÂf÷&ÖB‚'·ÆFf÷&Õö&6U÷W&ÇÒ÷7G&—R÷7V66W72"’’À¢6æ6VÅ÷W&Ã¢6æ6VÅ÷W&ÂçVçw&ö÷%öVÇ6R‡ÇÂf÷&ÖB‚'·ÆFf÷&Õö&6U÷W&ÇÒ÷7G&—Rö6æ6VÂ"’’À¢Ò¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¦fâ7G&—U÷ÆææW%öf÷%÷7FFR€¢7FFS¢e6†&VE7FFRÀ¢ÆFf÷&Õö&6U÷W&Ã¢–×Â–çFóÅ7G&–æsâÀ¢’Óâ7G&—UÆææW"°¢ÆWBÆææW"Ò7G&—UÆææW#£¦æWr‡ÆFf÷&Õö&6U÷W&Â“°¢–bÆWB6öÖR‡–ÖVçEöÖWF†öEö6öæf–wW&F–öâ’Ò7FFRç7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâæ5öFW&Vb‚¢°¢ÆææW"çv—F…÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ‡–ÖVçEöÖWF†öEö6öæf–wW&F–öâ¢ÒVÇ6R°¢ÆææW ¢Ð§Ð ¦fâÇ•÷7FFUö6†V6¶÷WE÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ€¢7FFS¢e6†&VE7FFRÀ¢–çFVçC¢f×WB7G&—U&WVW7D–çFVçBÀ¢’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢Ç•ö6†V6¶÷WE÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ€¢–çFVçBÀ¢7FFRç7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâæ5öFW&Vb‚’À¢¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—Rö6öææV7BÖ66÷VçG2"À¢&WVW7Eö&öG’ÒÆå7G&—T6öææV7D66÷VçE&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ%7G&—R66÷VçG2c"7&VFR&WVW7B–çFVçB"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–B6öææV7B66÷VçBÆææ–ær&WVW7B"¢¢•Ð¦7–æ2fâÆå÷7G&—Uö6öææV7Eö66÷VçB€¢§6öâ‡&WVW7B“¢§6öãÅÆå7G&—T6öææV7D66÷VçE&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÇ–ÖVçG5÷7G&—S£¤6öææV7D66÷VçEc$7&VFT–çFVçCâÂ7FGW46öFSâ°¢7G&—Uö6öææV7Eö66÷VçEö–çFVçB‡&WVW7B¢æÖ„§6öâ¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¦fâ7G&—Uö6öææV7Eö66÷VçEö–çFVçB€¢&WVW7C¢Æå7G&—T6öææV7D66÷VçE&WVW7BÀ¢’Óâ&W7VÇCÇ–ÖVçG5÷7G&—S£¤6öææV7D66÷VçEc$7&VFT–çFVçBÂ–ÖVçG5÷7G&—S£¥7G&—T–çFVw&F–öäW'&÷#à§°¢7G&—UÆææW#£¦æWr‚&‡GG¢òó#rããã£ƒƒ"’æ6öææV7Eö66÷VçE÷c"‡&WVW7BævVçEö–B§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—Rö6öææV7B×G&ç6fW'2"À¢&WVW7Eö&öG’ÒÆå7G&—T6öææV7EG&ç6fW%&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ%7G&—R6öææV7BG&ç6fW"&WVW7B–çFVçBf÷"f–B–÷WB"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–B–÷WB–çFVçB÷"G&ç6fW"Æææ–ær&WVW7B"¢¢•Ð¦7–æ2fâÆå÷7G&—Uö6öææV7E÷G&ç6fW"€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆå7G&—T6öææV7EG&ç6fW%&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÅ7G&—UG&ç6fW%ÆãâÂ7FGW46öFSâ°¢7G&—Uö6öææV7E÷G&ç6fW%÷Æâ‚g7FFRÂ&WVW7B’æÖ„§6öâ§Ð ¦fâ7G&—Uö6öææV7E÷G&ç6fW%÷Æâ€¢7FFS¢e6†&VE7FFRÀ¢&WVW7C¢Æå7G&—T6öææV7EG&ç6fW%&WVW7BÀ¢’Óâ&W7VÇCÅ7G&—UG&ç6fW%ÆâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&°¢çÆå÷7G&—U÷G&ç6fW"€¢Æå7G&—UG&ç6fW%&WVW7B°¢–÷WEö–çFVçEö–C¢&WVW7Bç–÷WEö–çFVçEö–BÀ¢6öææV7FVEö66÷VçEö–C¢&WVW7Bæ6öææV7FVEö66÷VçEö–BÀ¢ÒÀ¢7FFRçV&Æ–5ö&6U÷W&Âæ6ÆöæR‚’À¢¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—RöÆ—fRö6†V6¶÷WB×F÷×W2"À¢&WVW7Eö&öG’ÒÆå7G&—T6†V6¶÷WEF÷W&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$Æ—fR7G&—R6†V6¶÷WB6W76–öâW†V7WF–öâ&W÷'B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–BF÷×W&WVW7B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$÷W&F÷"Fö¶Vâ&WV—&VBv†VâõU$Dõ%ô•õDô´Tâ—26öæf–wW&VB"’À¢‡7FGW2ÒS"ÂFW67&—F–öâÒ%7G&—R’W†V7WF–öâf–ÆVB"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ$Æ—fR7G&—RW†V7WF–öâ—2F—6&ÆVB÷"æ÷B6öæf–wW&VB"¢’À¢6V7W&—G’‚‚&÷W&F÷%ö•÷Fö¶Vâ"ÒµÒ’Â‚&÷W&F÷%ö&V&W""ÒµÒ’¢•Ð¦7–æ2fâW†V7WFU÷7G&—Uö6†V6¶÷WE÷F÷÷W€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢§6öâ‡&WVW7B“¢§6öãÅÆå7G&—T6†V6¶÷WEF÷W&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÅ7G&—TW†V7WF–öå&W÷'CâÂ7FGW46öFSâ°¢&WV—&Uö÷W&F÷"‚g7FFRÂf†VFW'2“ó°¢ÆWB–çFVçBÒ7G&—Uö6†V6¶÷WE÷F÷÷Wö–çFVçB‚g7FFRÂ&WVW7B“ó°¢W†V7WFU÷7G&—Uö–çFVçB‚g7FFRÂ–çFVçB’æv—BæÖ„§6öâ§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—RöÆ—fRögVæF–ærÖ–çFVçG2÷¶–GÒö6†V6¶÷WB×6W76–öâ"À¢&×2‚‚&–B"ÒWV–BÂF‚ÂFW67&—F–öâÒ%7G&—Rf–BgVæF–ær–çFVçB–B"’’À¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$Æ—fR7G&—R6†V6¶÷WB6W76–öâW†V7WF–öâ&W÷'Bf÷"&÷VçG’gVæF–ær–çFVçB"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ%Væ¶æ÷vâÂæöâÕ7G&—RÂÇ&VG’ÖÆ–VBÂ÷"–çfÆ–BgVæF–ær–çFVçB"’À¢‡7FGW2ÒS"ÂFW67&—F–öâÒ%7G&—R’W†V7WF–öâf–ÆVB"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ%V&Æ–27G&—R6†V6¶÷WBW†V7WF–öâ—2F—6&ÆVB÷"Æ—fR7G&—RW†V7WF–öâ—2æ÷B6öæf–wW&VB"¢¢•Ð¦7–æ2fâW†V7WFU÷7G&—UögVæF–æuö–çFVçEö6†V6¶÷WB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚†–B“¢FƒÅWV–CâÀ¢’Óâ&W7VÇCÄ§6öãÅ7G&—TW†V7WF–öå&W÷'CâÂ7FGW46öFSâ°¢–b7FFRç7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVB°¢&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ð¢ÆWB–çFVçBÒ°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&²ç7G&—Uö6†V6¶÷WEöf÷%ögVæF–æuö–çFVçB†–BÂ7FFRçV&Æ–5ö&6U÷W&Âæ6ÆöæR‚’¢Ð¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB×WB–çFVçBÒ–çFVçC°¢Ç•÷7FFUö6†V6¶÷WE÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ‚g7FFRÂf×WB–çFVçB“ó°¢W†V7WFU÷7G&—Uö–çFVçB‚g7FFRÂ–çFVçB’æv—BæÖ„§6öâ§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—RöÆ—fRö6öææV7BÖ66÷VçG2"À¢&WVW7Eö&öG’ÒÆå7G&—T6öææV7D66÷VçE&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$Æ—fR7G&—R66÷VçG2c"W†V7WF–öâ&W÷'B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–B6öææV7B&WVW7B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$÷W&F÷"Fö¶Vâ&WV—&VBv†VâõU$Dõ%ô•õDô´Tâ—26öæf–wW&VB"’À¢‡7FGW2ÒS"ÂFW67&—F–öâÒ%7G&—R’W†V7WF–öâf–ÆVB"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ$Æ—fR7G&—RW†V7WF–öâ—2F—6&ÆVB÷"æ÷B6öæf–wW&VB"¢’À¢6V7W&—G’‚‚&÷W&F÷%ö•÷Fö¶Vâ"ÒµÒ’Â‚&÷W&F÷%ö&V&W""ÒµÒ’¢•Ð¦7–æ2fâW†V7WFU÷7G&—Uö6öææV7Eö66÷VçB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢§6öâ‡&WVW7B“¢§6öãÅÆå7G&—T6öææV7D66÷VçE&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÅ7G&—TW†V7WF–öå&W÷'CâÂ7FGW46öFSâ°¢&WV—&Uö÷W&F÷"‚g7FFRÂf†VFW'2“ó°¢ÆWB–çFVçBÒ7G&—Uö6öææV7Eö66÷VçEö–çFVçB‡&WVW7B¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ð¢ç&WVW7C°¢W†V7WFU÷7G&—Uö–çFVçB‚g7FFRÂ–çFVçB’æv—BæÖ„§6öâ§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—RöÆ—fRö6öææV7B×G&ç6fW'2"À¢&WVW7Eö&öG’ÒÆå7G&—T6öææV7EG&ç6fW%&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$Æ—fR7G&—R6öææV7BG&ç6fW"W†V7WF–öâ&W÷'B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–BG&ç6fW"&WVW7B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$÷W&F÷"Fö¶Vâ&WV—&VBv†VâõU$Dõ%ô•õDô´Tâ—26öæf–wW&VB"’À¢‡7FGW2ÒS"ÂFW67&—F–öâÒ%7G&—R’W†V7WF–öâf–ÆVB"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ$Æ—fR7G&—RW†V7WF–öâ—2F—6&ÆVB÷"æ÷B6öæf–wW&VB"¢’À¢6V7W&—G’‚‚&÷W&F÷%ö•÷Fö¶Vâ"ÒµÒ’Â‚&÷W&F÷%ö&V&W""ÒµÒ’¢•Ð¦7–æ2fâW†V7WFU÷7G&—Uö6öææV7E÷G&ç6fW"€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢§6öâ‡&WVW7B“¢§6öãÅÆå7G&—T6öææV7EG&ç6fW%&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÅ7G&—TW†V7WF–öå&W÷'CâÂ7FGW46öFSâ°¢&WV—&Uö÷W&F÷"‚g7FFRÂf†VFW'2“ó°¢ÆWBÆâÒ7G&—Uö6öææV7E÷G&ç6fW%÷Æâ‚g7FFRÂ&WVW7B“ó°¢W†V7WFU÷7G&—Uö–çFVçB‚g7FFRÂÆâç&WVW7B’æv—BæÖ„§6öâ§Ð ¦7–æ2fâW†V7WFU÷7G&—Uö–çFVçB€¢7FFS¢e6†&VE7FFRÀ¢–çFVçC¢7G&—U&WVW7D–çFVçBÀ¢’Óâ&W7VÇCÅ7G&—TW†V7WF–öå&W÷'BÂ7FGW46öFSâ°¢–b7FFRç7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVB°¢&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ð¢ÆWB6V7&WEö¶W’Ò7FFP¢ç7G&—U÷6V7&WEö¶W¢æ5öFW&Vb‚¢æf–ÇFW"‡Ç6V7&WGÂ6V7&WBçG&–Ò‚’æ—5öV×G’‚’¢æöµö÷"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“ó°¢W†V7WFU÷7G&—U÷&WVW7B‚f–çFVçBÂ6V7&WEö¶W’Âg7FFRç7G&—Uö•ö&6U÷W&Â¢æv—@¢æÖöW'"‡7G&—UöW†V7WF–öå÷7FGW2§Ð ¦fâ7G&—UöW†V7WF–öå÷7FGW2†W'&÷#¢–ÖVçG5÷7G&—S£¥7G&—T–çFVw&F–öäW'&÷"’Óâ7FGW46öFR°¢ÖF6‚W'&÷"°¢–ÖVçG5÷7G&—S£¥7G&—T–çFVw&F–öäW'&÷#£¥&WVW7Df–ÆVB²ââÐ¢Â–ÖVçG5÷7G&—S£¥7G&—T–çFVw&F–öäW'&÷#£¤‡GGG&ç7÷'B…ò’Óâ7FGW46öFS£¤$EôtDUt’À¢òÓâ7FGW46öFS£¤$Eõ$UTU5BÀ¢Ð§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—Rö6öææV7B×6æ6†÷G2"À¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ%&V6öæ6–ÆVB7G&—R6öææV7B–÷WBVÆ–v–&–Æ—G’6æ6†÷B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–B6öææV7B6æ6†÷B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$÷W&F÷"Fö¶Vâ&WV—&VBv†VâõU$Dõ%ô•õDô´Tâ—26öæf–wW&VB"¢’À¢6V7W&—G’‚‚&÷W&F÷%ö•÷Fö¶Vâ"ÒµÒ’Â‚&÷W&F÷%ö&V&W""ÒµÒ’¢•Ð¦7–æ2fâ&V6öæ6–ÆU÷7G&—Uö6öææV7E÷6æ6†÷B€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢§6öâ‡6æ6†÷B“¢§6öãÄ6öææV7D66÷VçE6æ6†÷CâÀ¢’Óâ&W7VÇCÄ§6öãÆ£¥7G&—T6öææV7E–÷WE&V6öæ6–Æ–F–öãâÂ7FGW46öFSâ°¢&WV—&Uö÷W&F÷"‚g7FFRÂf†VFW'2“ó°¢ÆWB&V6öæ6–Æ–F–öâÒ°¢ÆWB×WBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&°¢æÇ•÷7G&—Uö6öææV7E÷6æ6†÷B‡6æ6†÷B¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ð¢Ó°¢–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢f÷"&÷VçG’–âg&V6öæ6–Æ–F–öâæ&÷VçF–W2°¢7F÷&P¢çW6W'Eö&÷VçG’†&÷VçG’¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢f÷"6WGFÆVÖVçB–âg&V6öæ6–Æ–F–öâç6WGFÆVÖVçG2°¢7F÷&P¢çW6W'E÷6WGFÆVÖVçB‡6WGFÆVÖVçB¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢W'6—7EöÆVFvW%öVçG&–W2‡7F÷&RÂg&V6öæ6–Æ–F–öâæÆVFvW%öVçG&–W2’æv—Có°¢Ð¢ö²„§6öâ‡&V6öæ6–Æ–F–öâ’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—R÷G&ç6fW"ÖWfVçG2"À¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ%&V6öæ6–ÆVB7G&—RG&ç6fW"WfVçB2f–B–÷WBWf–FVæ6R"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–BG&ç6fW"WfVçB–ÆöB÷"6–væGW&R"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ%vV&†öö²6–væGW&RfW&–f–6F–öâ—2æ÷B6öæf–wW&VB"¢¢•Ð¦7–æ2fâ&V6öæ6–ÆU÷7G&—U÷G&ç6fW%öWfVçB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢&öG“¢'—FW2À¢’Óâ&W7VÇCÄ§6öãÅ7G&—UG&ç6fW%&V6öæ6–Æ–F–öãâÂ7FGW46öFSâ°¢ÖF6‚g7FFRç7G&—U÷vV&†ööµ÷6V7&WB°¢6öÖR‡6V7&WB’Óâ°¢ÆWB6–væGW&RÒ†VFW'0¢ævWB‚'7G&—R×6–væGW&R"¢ææE÷F†Vâ‡ÇfÇVWÂfÇVRçFõ÷7G"‚’æö²‚’¢æöµö÷"…7FGW46öFS£¤$Eõ$UTU5B“ó°¢fW&–g•÷vV&†ööµ÷6–væGW&R‚f&öG’Â6–væGW&RÂ6V7&WB¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢Ð¢æöæR–b7FFRæÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·2Óâ·Ð¢æöæRÓâ&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR’À¢Ð¢ÆWBWfVçC¢7G&—UvV&†öö´WfVçBÐ¢6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚f&öG’’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWBWf–FVæ6RÒ7G&—TWfVçDFVGWW#£¦FVfVÇB‚¢æÇ•ö6öææV7E÷G&ç6fW"‚fWfVçB¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB&V6öæ6–Æ–F–öâÒ°¢ÆWB×WBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&°¢æÇ•÷7G&—U÷G&ç6fW%öWf–FVæ6R†Wf–FVæ6R¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ð¢Ó° ¢–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢–b&V6öæ6–Æ–F–öâæGWÆ–6FR°¢7F÷&P¢çW6W'E÷–ÖVçEöWfVçB‚g&V6öæ6–Æ–F–öâæWf–FVæ6Rç–ÖVçEöWfVçB¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢–bÆWB6öÖR‡6WGFÆVÖVçB’Òg&V6öæ6–Æ–F–öâç6WGFÆVÖVçB°¢7F÷&P¢çW6W'E÷6WGFÆVÖVçB‡6WGFÆVÖVçB¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢–bÆWB6öÖR†&÷VçG’’Òg&V6öæ6–Æ–F–öâæ&÷VçG’°¢7F÷&P¢çW6W'Eö&÷VçG’†&÷VçG’¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢W'6—7EöÆVFvW%öVçG&–W2‡7F÷&RÂg&V6öæ6–Æ–F–öâæÆVFvW%öVçG&–W2’æv—Có°¢Ð¢ö²„§6öâ‡&V6öæ6–Æ–F–öâ’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷7G&—Rö6†V6¶÷WB×vV&†öö·2"À¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ%&V6öæ6–ÆVB–B7G&—R6†V6¶÷WBF÷×WvV&†öö²"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$–çfÆ–BvV&†öö²–ÆöB÷"6–væGW&R"’À¢‡7FGW2ÒS2ÂFW67&—F–öâÒ%vV&†öö²6–væGW&RfW&–f–6F–öâ—2æ÷B6öæf–wW&VB"¢¢•Ð¦7–æ2fâ&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢&öG“¢'—FW2À¢’Óâ&W7VÇCÄ§6öãÆ£¥7G&—TgVæF–æu&V6öæ6–Æ–F–öãâÂ7FGW46öFSâ°¢ÖF6‚g7FFRç7G&—U÷vV&†ööµ÷6V7&WB°¢6öÖR‡6V7&WB’Óâ°¢ÆWB6–væGW&RÒ†VFW'0¢ævWB‚'7G&—R×6–væGW&R"¢ææE÷F†Vâ‡ÇfÇVWÂfÇVRçFõ÷7G"‚’æö²‚’¢æöµö÷"…7FGW46öFS£¤$Eõ$UTU5B“ó°¢fW&–g•÷vV&†ööµ÷6–væGW&R‚f&öG’Â6–væGW&RÂ6V7&WB¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢Ð¢æöæR–b7FFRæÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·2Óâ·Ð¢æöæRÓâ&WGW&âW'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR’À¢Ð¢ÆWBWfVçC¢7G&—UvV&†öö´WfVçBÐ¢6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚f&öG’’æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWBgVæF–æuö7&VF—BÒ7G&—TWfVçDFVGWW#£¦FVfVÇB‚¢æÇ•ö6†V6¶÷WE÷F÷÷W‚fWfVçB¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB&V6öæ6–Æ–F–öâÒ°¢ÆWB×WBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&°¢æÇ•÷7G&—UögVæF–æuö7&VF—B†gVæF–æuö7&VF—B¢æÖöW'"‡Å÷Â7FGW46öFS£¤$Eõ$UTU5B“ð¢Ó° ¢–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢–b&V6öæ6–Æ–F–öâæGWÆ–6FR°¢7F÷&P¢çW6W'E÷–ÖVçEöWfVçB‚g&V6öæ6–Æ–F–öâægVæF–æuö7&VF—Bç–ÖVçEöWfVçB¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢–bÆWB6öÖR†–çFVçB’Òg&V6öæ6–Æ–F–öâægVæF–æuö–çFVçB°¢7F÷&P¢çW6W'EögVæF–æuö–çFVçB†–çFVçB¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢–bÆWB6öÖR‡&W÷'B’Òg&V6öæ6–Æ–F–öâægVæF–æu÷&W÷'B°¢7F÷&P¢çW6W'Eö&÷VçG’‚g&W÷'Bæ&÷VçG’¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢7F÷&P¢çW6W'EögVæF–æuö6öçG&–'WF–öâ‚g&W÷'Bæ6öçG&–'WF–öâ¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢Ð¢W'6—7EöÆVFvW%öVçG&–W2‡7F÷&RÂg&V6öæ6–Æ–F–öâæÆVFvW%öVçG&–W2’æv—Có°¢Ð ¢ö²„§6öâ‡&V6öæ6–Æ–F–öâ’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"ö—77VRÖ&÷VçG’×Æâ"À¢&WVW7Eö&öG’ÒÆäv—D‡V$—77VT&÷VçG•&WVW7BÀ¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"—77VRÖf÷&Ò&÷VçG’'6RæB6†V6²×'VâÆâ"’¢•Ð¦7–æ2fâÆåöv—F‡V%ö—77VUö&÷VçG’€¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V$—77VT&÷VçG•&WVW7CâÀ¢’Óâ§6öãÄv—D‡V$—77VT&÷VçG•Æãâ°¢§6öâ†v—F‡V%ö—77VUö&÷VçG•÷Æâ‡&WVW7B’§Ð ¦fâv—F‡V%ö—77VUö&÷VçG•÷Æâ‡&WVW7C¢Æäv—D‡V$—77VT&÷VçG•&WVW7B’Óâv—D‡V$—77VT&÷VçG•Æâ°¢ÆWB'6VBÒ'6Uö—77VUöf÷&Õö&÷VçG’€¢g&WVW7Bç&W÷6—F÷'’À¢g&WVW7Bæ—77VU÷W&ÂÀ¢g&WVW7BçF—FÆRÀ¢g&WVW7Bæ&öG’À¢“°¢ÖF6‚'6VB°¢ö²†&÷VçG’’Óâv—D‡V$—77VT&÷VçG•Æâ°¢&VG“¢G'VRÀ¢6†V6³¢&÷VçG•ö6†V6µö÷WGWB„ö²‚f&÷VçG’’’À¢'6VC¢6öÖR†&÷VçG’’À¢W'&÷#¢æöæRÀ¢ÒÀ¢W'"†W'&÷"’Óâv—D‡V$—77VT&÷VçG•Æâ°¢&VG“¢fÇ6RÀ¢6†V6³¢&÷VçG•ö6†V6µö÷WGWB„W'"‚fW'&÷"’’À¢'6VC¢æöæRÀ¢W'&÷#¢6öÖR†W'&÷"çFõ÷7G&–ær‚’’À¢ÒÀ¢Ð§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"ö—77VRÖ’×7–æ2×Æâ"À¢&WVW7Eö&öG’ÒÆäv—D‡V$—77VT•7–æ5&WVW7BÀ¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"—77VRFò†÷7FVB’&÷VçG’7–æ2Æâ"’¢•Ð¦7–æ2fâÆåöv—F‡V%ö—77VUö•÷7–æ2€¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V$—77VT•7–æ5&WVW7CâÀ¢’Óâ§6öãÄv—D‡V$—77VT•7–æ5Æãâ°¢§6öâ†—77VUö•÷7–æ5÷Æâ„v—D‡V$—77VT•7–æ4–çWB°¢&W÷6—F÷'“¢&WVW7Bç&W÷6—F÷'’À¢—77VU÷W&Ã¢&WVW7Bæ—77VU÷W&ÂÀ¢F—FÆS¢&WVW7BçF—FÆRÀ¢&öG“¢&WVW7Bæ&öG’À¢•ö&6U÷W&Ã¢&WVW7Bæ•ö&6U÷W&ÂÀ¢W†—7F–æuö&÷VçG•ö–G3¢&WVW7BæW†—7F–æuö&÷VçG•ö–G2À¢†÷7FVEö•öW'&÷#¢&WVW7Bæ†÷7FVEö•öW'&÷"À¢Ò’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"ö—77VRÖ’×7–æ2"À¢&WVW7Eö&öG’ÒÆäv—D‡V$—77VT•7–æ5&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"—77VR7–æ6VB–çFò†÷7FVB&÷VçG’&V6÷&B"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$—77VR—2–çfÆ–B÷"†÷7FVB’7FFR6÷VÆBæ÷B&RÆææVB"’À¢‡7FGW2ÒCÂFW67&—F–öâÒ$÷W&F÷"Fö¶Vâ&WV—&VBv†VâõU$Dõ%ô•õDô´Tâ—26öæf–wW&VB"¢’À¢6V7W&—G’‚‚&÷W&F÷%ö•÷Fö¶Vâ"ÒµÒ’Â‚&÷W&F÷%ö&V&W""ÒµÒ’¢•Ð¦7–æ2fâ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V$—77VT•7–æ5&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÆFöÖ–ã£¤&÷VçG“âÂ7FGW46öFSâ°¢&WV—&Uö÷W&F÷"‚g7FFRÂf†VFW'2“ó°¢ÆWBÆâÒ—77VUö•÷7–æ5÷Æâ„v—D‡V$—77VT•7–æ4–çWB°¢&W÷6—F÷'“¢&WVW7Bç&W÷6—F÷'’À¢—77VU÷W&Ã¢&WVW7Bæ—77VU÷W&ÂÀ¢F—FÆS¢&WVW7BçF—FÆRÀ¢&öG“¢&WVW7Bæ&öG’À¢•ö&6U÷W&Ã¢&WVW7Bæ•ö&6U÷W&ÂÀ¢W†—7F–æuö&÷VçG•ö–G3¢&WVW7BæW†—7F–æuö&÷VçG•ö–G2À¢†÷7FVEö•öW'&÷#¢&WVW7Bæ†÷7FVEö•öW'&÷"À¢Ò“°¢–bÆâç&VG’°¢&WGW&âW'"…7FGW46öFS£¤$Eõ$UTU5B“°¢Ð¢ÆWB'6VBÒÆâç'6VBæöµö÷"…7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB&÷VçG•ö–BÒÆâæ&÷VçG•ö–Bæöµö÷"…7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB–FV×÷FVæ7•ö¶W’ÒÆâæ–FV×÷FVæ7•ö¶W’æöµö÷"…7FGW46öFS£¤$Eõ$UTU5B“ó°¢ÆWB&WVW7BÒ÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢'6VBç&WVW7BçF—FÆRÀ¢FV×ÆFU÷6ÇVs¢'6VBçFV×ÆFU÷6ÇVrÀ¢F&vWEöÖ÷VçEöÖ–æ÷#¢'6VBæÖ÷VçBæÖ÷VçBÀ¢7W'&Væ7“¢'6VBæÖ÷VçBæ7W'&Væ7’À¢gVæF–æuöÖöFS¢'6VBægVæF–æuöÖöFRÀ¢&—f7“¢'6VBç&—f7’À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ó° ¢ÆWB&÷VçG’Ò–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢ÆWB6æF–FFRÒ°¢ÆWB×WBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&²æ'V–ÆEöv—F‡V%ö—77VU÷ööÆVEö&÷VçG’‡&WVW7BÂ&÷VçG•ö–BÂ–FV×÷FVæ7•ö¶W’¢Ó°¢ÆWB6æF–FFRÒÖF6‚6æF–FFR°¢ö²†&÷VçG’’Óâ&÷VçG’À¢W'"…ò’Óâ°¢W'6—7EöÆÅ÷&—6µöWfVçG2‚g7FFR’æv—Có°¢&WGW&âW'"…7FGW46öFS£¤$Eõ$UTU5B“°¢Ð¢Ó°¢ÖF6‚7F÷&P¢çW6W'Eöv—F‡V%ö—77VU÷7–æ5ö&÷VçG’‚f6æF–FFR¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ð¢°¢v—D‡V$—77VU7–æ4&÷VçG•W6W'C£¥W6W'FVB†&÷VçG’’Óâ°¢7FFP¢ææWGv÷&°¢æÆö6²‚¢æW‡V7B‚'7FFRö—6öæVB"¢æ&÷VçF–W0¢æ–ç6W'B†&÷VçG’æ–BÂ&÷VçG’æ6ÆöæR‚’“°¢&÷VçG¢Ð¢v—D‡V$—77VU7–æ4&÷VçG•W6W'C£¤&Æö6¶VD'”7F—f—G’†W†—7F–ær’Óâ°¢ÆWB–BÒW†—7F–æræ–C°¢7FFP¢ææWGv÷&°¢æÆö6²‚¢æW‡V7B‚'7FFRö—6öæVB"¢æ&÷VçF–W0¢æ–ç6W'B†–BÂW†—7F–ær“°¢&WGW&âW'"…7FGW46öFS£¤$Eõ$UTU5B“°¢Ð¢Ð¢ÒVÇ6R°¢ÆWB&W7VÇBÒ°¢ÆWB×WBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&²çW6W'Eöv—F‡V%ö—77VU÷ööÆVEö&÷VçG’‡&WVW7BÂ&÷VçG•ö–BÂ–FV×÷FVæ7•ö¶W’¢Ó°¢ÆWB&÷VçG’ÒÖF6‚&W7VÇB°¢ö²†&÷VçG’’Óâ&÷VçG’À¢W'"…ò’Óâ°¢W'6—7EöÆÅ÷&—6µöWfVçG2‚g7FFR’æv—Có°¢&WGW&âW'"…7FGW46öFS£¤$Eõ$UTU5B“°¢Ð¢Ó°¢W'6—7Eö&÷VçG•öæEöÆVFvW"‚g7FFRÂf&÷VçG’ÂeµÒ’æv—Có°¢&÷VçG¢Ó°¢ö²„§6öâ†&÷VçG’’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"ö7&VFRÖ6öÖÖVçB×Æâ"À¢&WVW7Eö&öG’ÒÆäv—D‡V$7&VFT6öÖÖVçE&WVW7BÀ¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"—77VR6öÖÖVçBFò&Wf–Wv&ÆR6æöæ–6Â&÷VçG’†æFöfb"’¢•Ð¦7–æ2fâÆåöv—F‡V%ö7&VFUö6öÖÖVçB€¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V$7&VFT6öÖÖVçE&WVW7CâÀ¢’Óâ§6öãÄv—D‡V$7&VFT6öÖÖVçEÆãâ°¢§6öâ†7&VFUö6öÖÖVçE÷Æâ„v—D‡V$7&VFT6öÖÖVçD–çWB°¢&W÷6—F÷'“¢&WVW7Bç&W÷6—F÷'’À¢—77VU÷W&Ã¢&WVW7Bæ—77VU÷W&ÂÀ¢F—FÆS¢&WVW7BçF—FÆRÀ¢&öG“¢&WVW7Bæ&öG’À¢6öÖÖVçEö&öG“¢&WVW7Bæ6öÖÖVçEö&öG’À¢6öçG&–'WF÷%öÆöv–ã¢&WVW7Bæ6öçG&–'WF÷%öÆöv–âÀ¢6öÖÖVçEö–C¢&WVW7Bæ6öÖÖVçEö–BÀ¢W†—7F–æuö–FV×÷FVæ7•ö¶W—3¢&WVW7BæW†—7F–æuö–FV×÷FVæ7•ö¶W—2À¢Ò’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷6ö6–ÂöÖVçF–öâÖG&gB×Æâ"À¢&WVW7Eö&öG’ÒÆå6ö6–ÄÖVçF–öäG&gE&WVW7BÀ¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ%&öÆÆ÷WBÖvFVB6ö6–ÂÖVçF–öâ&Wf–WrG&gBÆâ"’¢•Ð¦7–æ2fâÆå÷6ö6–ÅöÖVçF–öåöG&gB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆå6ö6–ÄÖVçF–öäG&gE&WVW7CâÀ¢’Óâ§6öãÅ6ö6–ÄÖVçF–öäG&gEÆãâ°¢§6öâ†7W'&VçE÷6ö6–ÅöÖVçF–öå÷Æâ‚g7FFRÂ&WVW7B’æv—B§Ð ¦7–æ2fâ7W'&VçE÷6ö6–ÅöÖVçF–öå÷Æâ€¢7FFS¢e6†&VE7FFRÀ¢&WVW7C¢Æå6ö6–ÄÖVçF–öäG&gE&WVW7BÀ¢’Óâ6ö6–ÄÖVçF–öäG&gEÆâ°¢ÆWB÷W&F÷%öVæ&ÆVBÒVçeöfÆr‚$tTåEô$õTåD”U5õ4ô4”ÅôÔTåD”ôåôE$eE5ôTä$ÄTB"“°¢ÆWBv—F‡V%ö6öçfW'6–öâÒ–b÷W&F÷%öVæ&ÆVB°¢ÖF6‚ÆöEöWFöæöÖ÷W5ö&÷VçG•öfVVB‡7FFRÂ&&6RÖÖ–ææWB"ÂfÇ6R’æv—B°¢ö²†×WBfVVB’Óâ°¢7FFP¢ç&V6÷fW'•÷&W6W'fF–öç0¢æW†6ÇVFUög&öÕ÷&W÷'FVEö÷WF6öÖW2‚f×WBfVVB“°¢v—F‡V%ö—77VUö6öçfW'6–öåöWf–FVæ6R‚ffVVB¢Ð¢W'"…ò’ÓâVæf–Æ&ÆUöv—F‡V%ö6öçfW'6–öåöWf–FVæ6R‚’À¢Ð¢ÒVÇ6R°¢Væf–Æ&ÆUöv—F‡V%ö6öçfW'6–öåöWf–FVæ6R‚¢Ó°¢6ö6–ÅöÖVçF–öåöG&gE÷Æâ…6ö6–ÄÖVçF–öäG&gD–çWB°¢6÷W&6UöæWGv÷&³¢&WVW7Bç6÷W&6UöæWGv÷&²À¢ÖVçF–öå÷W&Ã¢&WVW7BæÖVçF–öå÷W&ÂÀ¢ÖVçF–öåö–C¢&WVW7BæÖVçF–öåö–BÀ¢ÖVçF–öå÷FW‡C¢&WVW7BæÖVçF–öå÷FW‡BÀ¢WF†÷%ö†æFÆS¢&WVW7BæWF†÷%ö†æFÆRÀ¢÷W&F÷%öVæ&ÆVBÀ¢v—F‡V%ö6öçfW'6–öâÀ¢Ò§Ð ¢5·WFö—£§F‚€¢vWBÀ¢F‚Ò"÷c÷6ö6–ÂöÖVçF–öâÖ–ævW7F–öâ÷&VF–æW72"À¢&W7öç6W2‚‡7FGW2Ò#Â&öG’Ò6ö6–ÄÖVçF–öä–ævW7F–öå&VF–æW72’¢•Ð¦7–æ2fâ6ö6–ÅöÖVçF–öåö–ævW7F–öå÷&VF–æW72€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢’Óâ§6öãÅ6ö6–ÄÖVçF–öä–ævW7F–öå&VF–æW73â°¢ÆWBÆâÒ7W'&VçE÷6ö6–ÅöÖVçF–öå÷Æâ€¢g7FFRÀ¢Æå6ö6–ÄÖVçF–öäG&gE&WVW7B°¢6÷W&6UöæWGv÷&³¢&f&67FW""çFõ÷7G&–ær‚’À¢ÖVçF–öå÷W&Ã ¢&‡GG3¢òöf&67FW"ç‡—¢÷&VF–æW72óƒ ¢çFõ÷7G&–ær‚’À¢ÖVçF–öåö–C¢'&VF–æW72"çFõ÷7G&–ær‚’À¢ÖVçF–öå÷FW‡C¢"övVçBÖ&÷VçG’7&VFRU4D2&VF–æW72&ö&R"çFõ÷7G&–ær‚’À¢WF†÷%ö†æFÆS¢6öÖR‚'&VF–æW72"çFõ÷7G&–ær‚’’À¢ÒÀ¢¢æv—C°¢ÆWBvV&†ööµö6öæf–wW&VBÒ7FFRææW–æ%÷6ö6–Âæ—5÷6öÖR‚“°¢ÆWB&WÇ•ö6öæf–wW&VBÒ7FFP¢ææW–æ%÷6ö6–À¢æ5÷&Vb‚¢æ—5÷6öÖUöæB‡Æ6öæf–wÂ6öæf–rç&WÇ•ö6öæf–wW&VB‚’“°¢ÆWBVæ&ÆVBÐ¢ÆâævFRç76VBbb7FFRç7F÷&Ræ—5÷6öÖR‚’bbvV&†ööµö6öæf–wW&VBbb&WÇ•ö6öæf–wW&VC°¢§6öâ…6ö6–ÄÖVçF–öä–ævW7F–öå&VF–æW72°¢66†VÖ÷fW'6–öã¢&vVçBÖ&÷VçF–W2÷6ö6–ÂÖÖVçF–öâÖ–ævW7F–öâ×&VF–æW72×c"çFõ÷7G&–ær‚’À¢&÷f–FW#¢&æW–æ""çFõ÷7G&–ær‚’À¢6÷W&6UöæWGv÷&³¢&f&67FW""çFõ÷7G&–ær‚’À¢Væ&ÆVBÀ¢÷W&F÷%öVæ&ÆVC¢ÆâævFRæ÷W&F÷%öVæ&ÆVBÀ¢FF&6Uö6öæf–wW&VC¢7FFRç7F÷&Ræ—5÷6öÖR‚’À¢vV&†ööµö6öæf–wW&VBÀ¢&WÇ•ö6öæf–wW&VBÀ¢&÷Eöf–C¢7FFRææW–æ%÷6ö6–Âæ5÷&Vb‚’æÖ‡Æ6öæf–wÂ6öæf–ræ&÷Eöf–B’À¢&÷E÷W6W&æÖS¢7FFP¢ææW–æ%÷6ö6–À¢æ5÷&Vb‚¢æÖ‡Æ6öæf–wÂ6öæf–ræ&÷E÷W6W&æÖRæ6ÆöæR‚’’À¢vV&†ööµ÷Fƒ¢"÷c÷6ö6–Â÷vV&†öö·2öæW–æ""çFõ÷7G&–ær‚’À¢vFU÷76VC¢ÆâævFRç76VBÀ¢v—F‡V%ö÷&–v–æFVEö6æöæ–6ÅögVæFVC¢ÆâævFRæv—F‡V%ö÷&–v–æFVEö6æöæ–6ÅögVæFVBÀ¢v—F‡V%ö÷&–v–æFVEö6æöæ–6Å÷6WGFÆVC¢ÆâævFRæv—F‡V%ö÷&–v–æFVEö6æöæ–6Å÷6WGFÆVBÀ¢&V6öã¢–bVæ&ÆVB°¢'6–væVBÖVçF–öâ–ævW7F–öâÂGW&&ÆRG&gG2ÂæB&÷B&WÆ–W2&R&VG’"çFõ÷7G&–ær‚¢ÒVÇ6R–bÆâævFRç76VB°¢ÆâævFRç&V6öà¢ÒVÇ6R–b7FFRç7F÷&Ræ—5öæöæR‚’°¢$DD$4UõU$Â—2&WV—&VBf÷"GW&&ÆR&WÆ’&÷FV7F–öâ"çFõ÷7G&–ær‚¢ÒVÇ6R–bvV&†ööµö6öæf–wW&VB°¢$æW–æ"vV&†öö²æB&÷B–FVçF—G’&Ræ÷B6öæf–wW&VB"çFõ÷7G&–ær‚¢ÒVÇ6R°¢$æW–æ"’¶W’æB&÷fVB6–væW"&R&WV—&VBf÷"&÷B&WÆ–W2"çFõ÷7G&–ær‚¢ÒÀ¢Wf–FVæ6Uö&÷VæF'“¢%F†—2&VF–æW72&W÷'BFöW2æ÷B&÷fRF†B&÷f–FW"vV&†öö²—2&Vv—7FW&VBâG&gB—2æWfW"V&Æ—6†VBÂgVæFVBÂfW&–f–VBÂ÷"6WGFÆVB&÷VçG’â ¢çFõ÷7G&–ær‚’À¢Ò§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷c÷6ö6–Â÷vV&†öö·2öæW–æ""À¢&WVW7Eö&öG’Ò7G&–ærÀ¢&W7öç6W2€¢‡7FGW2Ò#Â&öG’Ò6ö6–ÄÖVçF–öåvV&†ööµ&W7öç6RÂFW67&—F–öâÒ%6–væVBÖVçF–öâ–ævW7FVBæB&WÇ’6ö×ÆWFVB÷"&WÆ–VB"’À¢‡7FGW2Ò#"Â&öG’Ò6ö6–ÄÖVçF–öåvV&†ööµ&W7öç6RÂFW67&—F–öâÒ%6–væVBWfVçB–væ÷&VBÂ&Æö6¶VBÂ7F÷&VBf÷"&Wf–WrÂ÷"Ç&VG’&V–ær&ö6W76VB"’À¢‡7FGW2ÒCÂ&öG’Ò6ö6–ÄÖVçF–öåvV&†ööµ&W7öç6RÂFW67&—F–öâÒ$Ö—76–ær÷"–çfÆ–BæW–æ"6–væGW&R"’À¢‡7FGW2ÒS2Â&öG’Ò6ö6–ÄÖVçF–öåvV&†ööµ&W7öç6RÂFW67&—F–öâÒ$GW&&ÆR–ævW7F–öâ—2æ÷B6öæf–wW&VB"¢¢•Ð¦7–æ2fâ–ævW7EöæW–æ%÷6ö6–ÅöÖVçF–öâ€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢†VFW'3¢†VFW$ÖÀ¢&öG“¢'—FW2À¢’Óâ…7FGW46öFRÂ§6öãÅ6ö6–ÄÖVçF–öåvV&†ööµ&W7öç6Sâ’°¢ÆWB6öÖR†6öæf–r’Ò7FFRææW–æ%÷6ö6–Âæ5÷&Vb‚’VÇ6R°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢fÇ6RÀ¢fÇ6RÀ¢'Væ6öæf–wW&VB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢$æW–æ"vV&†öö²–FVçF—G’—2æ÷B6öæf–wW&VB"À¢“°¢Ó°¢ÆWB6öÖR‡7F÷&R’Ò7FFRç7F÷&Ræ5÷&Vb‚’VÇ6R°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄRÀ¢fÇ6RÀ¢fÇ6RÀ¢'Væ6öæf–wW&VB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢$DD$4UõU$Â—2&WV—&VBf÷"GW&&ÆR–ævW7F–öâ"À¢“°¢Ó°¢ÆWB6öÖR‡6–væGW&R’Ò†VFW'0¢ævWB‚'‚ÖæW–æ"×6–væGW&R"¢ææE÷F†Vâ‡ÇfÇVWÂfÇVRçFõ÷7G"‚’æö²‚’¢VÇ6R°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¥TäUD„õ$•¤TBÀ¢fÇ6RÀ¢fÇ6RÀ¢'&V¦V7FVB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢&Ö—76–æræW–æ"6–væGW&R"À¢“°¢Ó°¢–bfW&–g•öæW–æ%÷6–væGW&R‚f&öG’Â6–væGW&RÂf6öæf–rçvV&†ööµ÷6V7&WB’°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¥TäUD„õ$•¤TBÀ¢fÇ6RÀ¢fÇ6RÀ¢'&V¦V7FVB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢&–çfÆ–BæW–æ"6–væGW&R"À¢“°¢Ð¢ÆWBWfVçC¢æW–æ%vV&†öö´WfVçBÒÖF6‚6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚f&öG’’°¢ö²†WfVçB’ÓâWfVçBÀ¢W'"…ò’Óâ°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤$Eõ$UTU5BÀ¢fÇ6RÀ¢fÇ6RÀ¢'&V¦V7FVB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢&–çfÆ–BæW–æ"WfVçB¥4ôâ"À¢¢Ð¢Ó°¢–bWfVçBæWfVçE÷G—RÒ&67Bæ7&VFVB"ÇÂWfVçBæFFæö&¦V7BÒ&67B"°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤44UDTBÀ¢fÇ6RÀ¢fÇ6RÀ¢&–væ÷&VB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢'6–væVBWfVçB—2æ÷B7&VFVB67B"À¢“°¢Ð¢ÆWB67BÒWfVçBæFF°¢–bfÆ–Eöf&67FW%ö†6‚‚f67Bæ†6‚¢ÇÂ67BæWF†÷"æf–BÃÒ ¢ÇÂ67BæWF†÷"çW6W&æÖRæ—5öV×G’‚¢ÇÂ67BæWF†÷"çW6W&æÖRæÆVâ‚’âc@¢ÇÂ67BæWF†÷"çW6W&æÖRæ6†'2‚’æÆÂ‡Æ6†&7FW'Â°¢6†&7FW"æ—5ö66–•öÇ†çVÖW&–2‚’ÇÂÖF6†W2†6†&7FW"ÂuòrÂrÒrÂrâr¢Ò¢ÇÂ67BçFW‡Bæ—5öV×G’‚¢ÇÂ67BçFW‡BæÆVâ‚’â…ó ¢°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤$Eõ$UTU5BÀ¢fÇ6RÀ¢fÇ6RÀ¢'&V¦V7FVB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢&7&VFVB67Bf–VÆG2&R–çfÆ–B"À¢“°¢Ð¢ÆWBF—&V7FÇ•öÖVçF–öç5ö&÷BÒ67@¢æÖVçF–öæVE÷&öf–ÆW0¢æ—FW"‚¢æç’‡Ç&öf–ÆWÂ&öf–ÆRæf–BÓÒ6öæf–ræ&÷Eöf–B¢ÇÂFW‡EöÖVçF–öç5ö&÷B‚f67BçFW‡BÂf6öæf–ræ&÷E÷W6W&æÖR“°¢ÆWBÖVçF–öåö–BÒ67Bæ†6‚çFõö66–•öÆ÷vW&66R‚“°¢ÆWBÖVçF–öå÷W&ÂÒf÷&ÖB€¢&‡GG3¢òöf&67FW"ç‡—¢÷·Ò÷·Ò"À¢67BæWF†÷"çW6W&æÖRÂÖVçF–öåö–@¢“°¢ÆWB×WBÆâÒ7W'&VçE÷6ö6–ÅöÖVçF–öå÷Æâ€¢g7FFRÀ¢Æå6ö6–ÄÖVçF–öäG&gE&WVW7B°¢6÷W&6UöæWGv÷&³¢&f&67FW""çFõ÷7G&–ær‚’À¢ÖVçF–öå÷W&Ã¢ÖVçF–öå÷W&Âæ6ÆöæR‚’À¢ÖVçF–öåö–C¢ÖVçF–öåö–Bæ6ÆöæR‚’À¢ÖVçF–öå÷FW‡C¢67BçFW‡Bæ6ÆöæR‚’À¢WF†÷%ö†æFÆS¢6öÖR†67BæWF†÷"çW6W&æÖRæ6ÆöæR‚’’À¢ÒÀ¢¢æv—C°¢ÆWB–BÒWV–C£¦æWu÷cB‚“°¢ÆWBG&gEö†æFöfe÷W&ÂÒÆà¢ç&VG¢çF†Vâ‡ÇÂ6öæf–ræG&gEö†æFöfe÷W&Â†–B’¢æf–ÇFW"‡Å÷ÂF—&V7FÇ•öÖVçF–öç5ö&÷B“°¢–bÆWB…6öÖR†G&gB’Â6öÖR††æFöfe÷W&Â’’Ò‚f×WBÆâæG&gBÂfG&gEö†æFöfe÷W&Â’°¢G&gBæG&gEö†æFöfe÷W&ÂÒ†æFöfe÷W&Âæ6ÆöæR‚“°¢Ð¢ÆWB7FGW2Ò–bF—&V7FÇ•öÖVçF–öç5ö&÷BÇÂ‡ÆâævFRç76VBbbÆâç&VG’’°¢&–væ÷&VB ¢ÒVÇ6R–bÆâævFRç76VB°¢&&Æö6¶VB ¢ÒVÇ6R–b6öæf–rç&WÇ•ö6öæf–wW&VB‚’°¢'&WÇ•÷VæF–ær ¢ÒVÇ6R°¢&G&gE÷&VG’ ¢Ó°¢ÆWBG&gBÒ–bF—&V7FÇ•öÖVçF–öç5ö&÷BbbÆâç&VG’°¢ÆâæG&g@¢æ5÷&Vb‚¢ææE÷F†Vâ‡ÆG&gGÂ6W&FUö§6öã£§Fõ÷fÇVR†G&gB’æö²‚’¢ÒVÇ6R°¢æöæP¢Ó°¢ÆWB&W6W'fF–öâÒÖF6‚7F÷&P¢ç&W6W'fU÷6ö6–ÅöÖVçF–öåö–ævW7F–öâ‚dæWu6ö6–ÄÖVçF–öä–ævW7F–öâ°¢–BÀ¢&÷f–FW#¢&æW–æ""çFõ÷7G&–ær‚’À¢&÷f–FW%öWfVçEö–C¢f÷&ÖB‚&67Bæ7&VFVC§¶ÖVçF–öåö–GÒ"’À¢6÷W&6UöæWGv÷&³¢&f&67FW""çFõ÷7G&–ær‚’À¢ÖVçF–öåö–BÀ¢ÖVçF–öå÷W&ÂÀ¢WF†÷%öf–C¢67BæWF†÷"æf–BÀ¢WF†÷%ö†æFÆS¢6öÖR†67BæWF†÷"çW6W&æÖR’À¢ÖVçF–öå÷FW‡C¢67BçFW‡BÀ¢7FGW3¢7FGW2çFõ÷7G&–ær‚’À¢G&gBÀ¢–FV×÷FVæ7•ö¶W“¢Æâæ–FV×÷FVæ7•ö¶W’æ6ÆöæR‚’À¢&V6V—fVEöC¢WF3£¦æ÷r‚’À¢Ò¢æv—@¢°¢ö²‡&W6W'fF–öâ’Óâ&W6W'fF–öâÀ¢W'"…ò’Óâ°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢fÇ6RÀ¢fÇ6RÀ¢&f–ÆVB"À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢&6÷VÆBæ÷BGW&&Ç’&W6W'fRF†RÖVçF–öâ"À¢¢Ð¢Ó°¢ÆWB&V6÷&BÒ&W6W'fF–öâç&V6÷&C°¢ÆWBW'6—7FVEö†æFöfe÷W&ÂÒ&V6÷&@¢æG&g@¢æ5÷&Vb‚¢ææE÷F†Vâ‡ÆG&gGÂG&gBævWB‚&G&gEö†æFöfe÷W&Â"’¢ææE÷F†Vâ‡6W&FUö§6öã£¥fÇVS£¦5÷7G"¢æÖ‡7G#£§Fõ÷7G&–ær“°¢–bÖF6†W2€¢&V6÷&Bç7FGW2æ5÷7G"‚’À¢&–væ÷&VB"Â&&Æö6¶VB"Â&G&gE÷&VG’ ¢’°¢ÆWBÖW76vRÒÖF6‚&V6÷&Bç7FGW2æ5÷7G"‚’°¢&–væ÷&VB"Óâ'6–væVB67BF–Bæ÷B6öçF–âfÆ–BF—&V7B&÷B6öÖÖæB"À¢&&Æö6¶VB"Óâ&6æöæ–6Âv—D‡V"6öçfW'6–öâvFR—2æ÷B7W'&VçFÇ’6F—6f–VB"À¢òÓâ'&Wf–WrG&gB7F÷&VC²&÷B&WÇ’7&VFVçF–Ç2&Ræ÷B6öæf–wW&VB"À¢Ó°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤44UDTBÀ¢&V6÷&Bç7FGW2ÓÒ&G&gE÷&VG’"À¢&W6W'fF–öâæ–ç6W'FVBÀ¢g&V6÷&Bç7FGW2À¢6öÖR‡&V6÷&Bæ–B’À¢W'6—7FVEö†æFöfe÷W&ÂÀ¢&V6÷&Bç&WÇ•ö67Eö†6‚À¢ÖW76vRÀ¢“°¢Ð¢–b&V6÷&Bç7FGW2ÓÒ'&WÆ–VB"°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤ô²À¢G'VRÀ¢G'VRÀ¢'&WÆ–VB"À¢6öÖR‡&V6÷&Bæ–B’À¢W'6—7FVEö†æFöfe÷W&ÂÀ¢&V6÷&Bç&WÇ•ö67Eö†6‚À¢&ÖVçF–öâv2Ç&VG’6öçfW'FVBæB&WÆ–VBFò"À¢“°¢Ð¢ÆWBÆV6U÷Fö¶VâÒWV–C£¦æWu÷cB‚“°¢ÆWB6Æ–ÖVBÒÖF6‚7F÷&P¢æ6Æ–Õ÷6ö6–ÅöÖVçF–öå÷&WÇ’‡&V6÷&Bæ–BÂÆV6U÷Fö¶VâÂCR¢æv—@¢°¢ö²†6Æ–ÖVB’Óâ6Æ–ÖVBÀ¢W'"…ò’Óâ°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢fÇ6RÀ¢&W6W'fF–öâæ–ç6W'FVBÀ¢&f–ÆVB"À¢6öÖR‡&V6÷&Bæ–B’À¢W'6—7FVEö†æFöfe÷W&ÂÀ¢æöæRÀ¢&6÷VÆBæ÷B7V—&RF†R&WÇ’ÆV6R"À¢¢Ð¢Ó°¢ÆWB6öÖR†6Æ–ÖVB’Ò6Æ–ÖVBVÇ6R°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤44UDTBÀ¢G'VRÀ¢&W6W'fF–öâæ–ç6W'FVBÀ¢g&V6÷&Bç7FGW2À¢6öÖR‡&V6÷&Bæ–B’À¢W'6—7FVEö†æFöfe÷W&ÂÀ¢&V6÷&Bç&WÇ•ö67Eö†6‚À¢&æ÷F†W"v÷&¶W"÷vç2F†R7F—fR&WÇ’ÆV6R"À¢“°¢Ó°¢ÆWB†æFöfe÷W&ÂÒW'6—7FVEö†æFöfe÷W&À¢æ5öFW&Vb‚¢çVçw&ö÷"‚&‡GG3¢òövVçF&÷VçF–W2æò7÷7BÖÖ&÷VçG’"“°¢ÖF6‚V&Æ—6…öæW–æ%öG&gE÷&WÇ’†6öæf–rÂf6Æ–ÖVBÂ†æFöfe÷W&Â’æv—B°¢ö²‡&WÇ•ö67Eö†6‚’Óâ°¢ÆWB6ö×ÆWFVBÒ7F÷&P¢æ6ö×ÆWFU÷6ö6–ÅöÖVçF–öå÷&WÇ’€¢6Æ–ÖVBæ–BÀ¢ÆV6U÷Fö¶VâÀ¢6öÖR‚g&WÇ•ö67Eö†6‚’À¢æöæRÀ¢¢æv—@¢æö²‚¢æfÆGFVâ‚“°¢–b6ö×ÆWFVBæ—5öæöæR‚’°¢&WGW&â6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢fÇ6RÀ¢&W6W'fF–öâæ–ç6W'FVBÀ¢&f–ÆVB"À¢6öÖR†6Æ–ÖVBæ–B’À¢W'6—7FVEö†æFöfe÷W&ÂÀ¢6öÖR‡&WÇ•ö67Eö†6‚’À¢'&WÇ’v2V&Æ—6†VB'WBGW&&ÆR6ö×ÆWF–öâ6÷VÆBæ÷B&R&V6÷&FVB"À¢“°¢Ð¢6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤ô²À¢G'VRÀ¢&W6W'fF–öâæ–ç6W'FVBÀ¢'&WÆ–VB"À¢6öÖR†6Æ–ÖVBæ–B’À¢W'6—7FVEö†æFöfe÷W&ÂÀ¢6öÖR‡&WÇ•ö67Eö†6‚’À¢'&Wf–WrG&gB7F÷&VBæB&÷B&WÇ’V&Æ—6†VB"À¢¢Ð¢W'"†W'&÷"’Óâ°¢ÆWBòÒ7F÷&P¢æ6ö×ÆWFU÷6ö6–ÅöÖVçF–öå÷&WÇ’†6Æ–ÖVBæ–BÂÆV6U÷Fö¶VâÂæöæRÂ6öÖR‚fW'&÷"’¢æv—C°¢6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW46öFS£¤$EôtDUt’À¢fÇ6RÀ¢&W6W'fF–öâæ–ç6W'FVBÀ¢'&WÇ•öf–ÆVB"À¢6öÖR†6Æ–ÖVBæ–B’À¢W'6—7FVEö†æFöfe÷W&ÂÀ¢æöæRÀ¢&G&gBv27F÷&VB'WBF†R&÷f–FW"&WÇ’f–ÆVC²&WG'’—26fR"À¢¢Ð¢Ð§Ð ¢òò¶VW–ærF†RWf–FVæ6RÖ&V&–ær&W7öç6Rf–VÆG2W‡Æ–6—BÖ¶W26V7W&—G’×6Vç6—F—fP¢òòV&Ç’&WGW&ç2&Wf–Wv&ÆRBV6‚6ÆÂ6—FRà¢5¶ÆÆ÷r†6Æ—“£§FöõöÖç•ö&wVÖVçG2•Ð¦fâ6ö6–Å÷vV&†ööµ÷&W7öç6R€¢7FGW5ö6öFS¢7FGW46öFRÀ¢66WFVC¢&ööÂÀ¢GWÆ–6FS¢&ööÂÀ¢7FGW3¢g7G"À¢–ævW7F–öåö–C¢÷F–öãÅWV–CâÀ¢G&gEö†æFöfe÷W&Ã¢÷F–öãÅ7G&–æsâÀ¢&WÇ•ö67Eö†6ƒ¢÷F–öãÅ7G&–æsâÀ¢ÖW76vS¢g7G"À¢’Óâ…7FGW46öFRÂ§6öãÅ6ö6–ÄÖVçF–öåvV&†ööµ&W7öç6Sâ’°¢€¢7FGW5ö6öFRÀ¢§6öâ…6ö6–ÄÖVçF–öåvV&†ööµ&W7öç6R°¢66†VÖ÷fW'6–öã¢&vVçBÖ&÷VçF–W2÷6ö6–ÂÖÖVçF–öâ×vV&†öö²×c"çFõ÷7G&–ær‚’À¢66WFVBÀ¢GWÆ–6FRÀ¢7FGW3¢7FGW2çFõ÷7G&–ær‚’À¢–ævW7F–öåö–BÀ¢G&gEö†æFöfe÷W&ÂÀ¢&WÇ•ö67Eö†6‚À¢ÖW76vS¢ÖW76vRçFõ÷7G&–ær‚’À¢Wf–FVæ6Uö&÷VæF'“¢%F†—2&V6÷&G2&Wf–WrG&gBæB÷F–öæÂ6ö6–Â&WÇ’öæÇ’â—BFöW2æ÷BV&Æ—6‚ÂgVæBÂfW&–g’Â6WGFÆRÂ÷"&÷fR–ÖVçBf÷"&÷VçG’â ¢çFõ÷7G&–ær‚’À¢Ò’À¢§Ð ¦fâfW&–g•öæW–æ%÷6–væGW&R†&öG“¢e·S…ÒÂ6–væGW&S¢g7G"Â6V7&WC¢e·S…Ò’Óâ&ööÂ°¢ÆWBö²‡6–væGW&R’Ò†Wƒ£¦FV6öFR‡6–væGW&RçG&–Ò‚’’VÇ6R°¢&WGW&âfÇ6S°¢Ó°¢ÆWBö²†×WBÖ2’Ò†Ö3££Å6†S#ã£¦æWuög&öÕ÷6Æ–6R‡6V7&WB’VÇ6R°¢&WGW&âfÇ6S°¢Ó°¢Ö2çWFFR†&öG’“°¢Ö2çfW&–g•÷6Æ–6R‚g6–væGW&R’æ—5öö²‚§Ð ¦fâfÆ–Eöf&67FW%ö†6‚††6ƒ¢g7G"’Óâ&ööÂ°¢†6‚æÆVâ‚’ÓÒC ¢bb†6‚ç7F'G5÷v—F‚‚#‚"¢bb†6…³"âåÐ¢æ6†'2‚¢æÆÂ‡Æ6†&7FW'Â6†&7FW"æ—5ö66–•ö†W†F–v—B‚’§Ð ¦fâFW‡EöÖVçF–öç5ö&÷B‡FW‡C¢g7G"Â&÷E÷W6W&æÖS¢g7G"’Óâ&ööÂ°¢ÆWBF&vWBÒf÷&ÖB‚$·Ò"Â&÷E÷W6W&æÖRçFõö66–•öÆ÷vW&66R‚’“°¢FW‡Bç7Æ—E÷v†—FW76R‚’æç’‡ÇFö¶VçÂ°¢Fö¶Và¢çG&–ÕöÖF6†W2‡Æ6†&7FW#¢6†'Â°¢6†&7FW"æ—5ö66–•öÇ†çVÖW&–2‚’bbÖF6†W2†6†&7FW"ÂtrÂuòrÂrÒr¢Ò¢æWö–væ÷&Uö66–•ö66R‚gF&vWB¢Ò§Ð ¦7–æ2fâV&Æ—6…öæW–æ%öG&gE÷&WÇ’€¢6öæf–s¢dæW–æ%6ö6–Ä–ævW7F–öä6öæf–rÀ¢–ævW7F–öã¢e6ö6–ÄÖVçF–öä–ævW7F–öâÀ¢†æFöfe÷W&Ã¢g7G"À¢’Óâ&W7VÇCÅ7G&–ærÂ7G&–æsâ°¢ÆWB•ö¶W’Ò6öæf–p¢æ•ö¶W¢æ5öFW&Vb‚¢æöµö÷%öVÇ6R‡ÇÂ$äU”ä%ô•ô´U’—2æ÷B6öæf–wW&VB"çFõ÷7G&–ær‚’“ó°¢ÆWB6–væW%÷WV–BÒ6öæf–p¢ç6–væW%÷WV–@¢æ5öFW&Vb‚¢æöµö÷%öVÇ6R‡ÇÂ$äU”ä%õ4”täU%õUT”B—2æ÷B6öæf–wW&VB"çFõ÷7G&–ær‚’“ó°¢ÆWB&WÇ•÷FW‡BÒf÷&ÖB‚$G&gB&VG’f÷"&Wf–Wr†æ÷BV&Æ—6†VB÷"gVæFVB“¢¶†æFöfe÷W&ÇÒ"“°¢ÆWB–FVÕö†6‚Ò†Wƒ£¦Væ6öFR…6†#Sc£¦F–vW7B€¢f÷&ÖB‚&æW–æ"×&WÇ“§·Ò"Â–ævW7F–öâæ–B’æ5ö'—FW2‚’À¢’“°¢ÆWB–FVÒÒf–FVÕö†6…²âãeÓ°¢ÆWB&W7öç6RÒ6öæf–p¢æ6Æ–Vç@¢ç÷7B†f÷&ÖB‚'·Ò÷c"öf&67FW"ö67Bò"Â6öæf–ræ•ö&6U÷W&Â’¢æ†VFW"‚'‚Ö’Ö¶W’"Â•ö¶W’¢æ§6öâ‚dæW–æ%V&Æ—6„67E&WVW7B°¢6–væW%÷WV–BÀ¢FW‡C¢g&WÇ•÷FW‡BÀ¢&VçC¢f–ævW7F–öâæÖVçF–öåö–BÀ¢&VçEöWF†÷%öf–C¢–ævW7F–öâæWF†÷%öf–BÀ¢–FVÒÀ¢Ò¢ç6VæB‚¢æv—@¢æÖöW'"‡ÆW'&÷'Âf÷&ÖB‚$æW–æ"&WVW7Bf–ÆVC¢¶W'&÷'Ò"’“ó°¢ÆWB7FGW2Ò&W7öç6Rç7FGW2‚“°¢–b7FGW2æ—5÷7V66W72‚’°¢&WGW&âW'"†f÷&ÖB‚$æW–æ"&WGW&æVB…EE·7FGW7Ò"’“°¢Ð¢ÆWBV&Æ—6†VBÒ&W7öç6P¢æ§6öã££ÄæW–æ%V&Æ—6„67E&W7öç6Sâ‚¢æv—@¢æÖöW'"‡ÆW'&÷'Âf÷&ÖB‚$æW–æ"&W7öç6Rv2–çfÆ–C¢¶W'&÷'Ò"’“ó°¢–bfÆ–Eöf&67FW%ö†6‚‚gV&Æ—6†VBæ67Bæ†6‚’°¢&WGW&âW'"‚$æW–æ"&W7öç6R6öçF–æVBâ–çfÆ–B67B†6‚"çFõ÷7G&–ær‚’“°¢Ð¢ö²‡V&Æ—6†VBæ67Bæ†6‚§Ð ¢5·WFö—£§F‚€¢vWBÀ¢F‚Ò"÷c÷6ö6–ÂöÖVçF–öâÖG&gG2÷¶–GÒ"À¢&×2‚‚&–B"ÒWV–BÂF‚ÂFW67&—F–öâÒ%W'6—7FVB6ö6–ÂÖVçF–öâ–ævW7F–öâ”B"’’À¢&W7öç6W2€¢‡7FGW2Ò#Â&öG’Ò6ö6–ÄÖVçF–öäG&gE&W7öç6R’À¢‡7FGW2ÒCBÂFW67&—F–öâÒ$G&gBæ÷Bf÷VæB"¢¢•Ð¦7–æ2fâvWE÷6ö6–ÅöÖVçF–öåöG&gB€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚†–B“¢FƒÅWV–CâÀ¢’Óâ&W7VÇCÄ§6öãÅ6ö6–ÄÖVçF–öäG&gE&W7öç6SâÂ7FGW46öFSâ°¢ÆWB7F÷&RÒ7FFRç7F÷&Ræ5÷&Vb‚’æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢ÆWB&V6÷&BÒ7F÷&P¢ævWE÷6ö6–ÅöÖVçF–öåö–ævW7F–öâ†–B¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ð¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢ÆWBG&gBÒ&V6÷&BæG&gBæöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢ö²„§6öâ…6ö6–ÄÖVçF–öäG&gE&W7öç6R°¢66†VÖ÷fW'6–öã¢&vVçBÖ&÷VçF–W2÷6ö6–ÂÖÖVçF–öâÖG&gB×c"çFõ÷7G&–ær‚’À¢–ævW7F–öåö–C¢&V6÷&Bæ–BÀ¢7FGW3¢&V6÷&Bç7FGW2À¢6÷W&6UöæWGv÷&³¢&V6÷&Bç6÷W&6UöæWGv÷&²À¢ÖVçF–öå÷W&Ã¢&V6÷&BæÖVçF–öå÷W&ÂÀ¢WF†÷%ö†æFÆS¢&V6÷&BæWF†÷%ö†æFÆRÀ¢G&gBÀ¢Wf–FVæ6Uö&÷VæF'“ ¢%&Wf–Wr&WV—&VC¢F†—2G&gB†2æ÷B&VVâV&Æ—6†VBÂgVæFVBÂfW&–f–VBÂ÷"6WGFÆVBâ ¢çFõ÷7G&–ær‚’À¢Ò’§Ð ¦fâVæf–Æ&ÆUöv—F‡V%ö6öçfW'6–öåöWf–FVæ6R‚’Óâv—D‡V$6æöæ–6Ä6öçfW'6–öäWf–FVæ6R°¢v—D‡V$6æöæ–6Ä6öçfW'6–öäWf–FVæ6R°¢Wf–FVæ6Uöf–Æ&ÆS¢fÇ6RÀ¢v—F‡V%ö÷&–v–æFVEö6æöæ–6ÅögVæFVC¢À¢v—F‡V%ö÷&–v–æFVEö6æöæ–6Å÷6WGFÆVC¢À¢Wf–FVæ6U÷6÷W&6S¢&–æFW†VB6öæf—&ÖVB&6RWfVçG2¦ö–æVBFòV&Æ–2v—D‡V"ÖGG&–'WFVBFW&×2 ¢çFõ÷7G&–ær‚’À¢Ð§Ð ¦fâv—F‡V%ö—77VUö6öçfW'6–öåöWf–FVæ6R€¢fVVC¢e´WFöæöÖ÷W4&÷VçG”fVVD—FVÕÒÀ¢’Óâv—D‡V$6æöæ–6Ä6öçfW'6–öäWf–FVæ6R°¢ÆWBv—F‡V%ö—FV×2ÒfVVBæ—FW"‚’æf–ÇFW"‡Æ—FV×Â°¢—FVÒçFW&×2æ5÷&Vb‚’æ—5÷6öÖUöæB‡ÇFW&×7Â°¢FW&×2æFö7VÖVçBç6÷W&6U÷W&Âæ5öFW&Vb‚’æ—5÷6öÖUöæB‡ÇW&ÇÂ°¢W&Âç7F'G5÷v—F‚‚&‡GG3¢òöv—F‡V"æ6öÒò"’bbW&Âæ6öçF–ç2‚"ö—77VW2ò"¢Ò¢Ò¢Ò“°¢ÆWB×WBgVæFVBÒ÷S3#°¢ÆWB×WB6WGFÆVBÒ÷S3#°¢f÷"—FVÒ–âv—F‡V%ö—FV×2°¢–b—FVÐ¢æWfVçG0¢æ—FW"‚¢æç’‡ÆWfVçGÂWfVçBæ¶–æBÓÒWFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG”&V6ÖT6Æ–Ö&ÆR¢°¢gVæFVBÒgVæFVBç6GW&F–æuöFBƒ“°¢Ð¢–b—FVÐ¢æWfVçG0¢æ—FW"‚¢æç’‡ÆWfVçGÂWfVçBæ¶–æBÓÒWFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG•6WGFÆVB¢°¢6WGFÆVBÒ6WGFÆVBç6GW&F–æuöFBƒ“°¢Ð¢Ð¢v—D‡V$6æöæ–6Ä6öçfW'6–öäWf–FVæ6R°¢Wf–FVæ6Uöf–Æ&ÆS¢G'VRÀ¢v—F‡V%ö÷&–v–æFVEö6æöæ–6ÅögVæFVC¢gVæFVBÀ¢v—F‡V%ö÷&–v–æFVEö6æöæ–6Å÷6WGFÆVC¢6WGFÆVBÀ¢Wf–FVæ6U÷6÷W&6S¢&–æFW†VB6öæf—&ÖVB&6RWfVçG2¦ö–æVBFòV&Æ–2v—D‡V"ÖGG&–'WFVBFW&×2 ¢çFõ÷7G&–ær‚’À¢Ð§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"ögVæF–ærÖ6öÖÖVçB×Æâ"À¢&WVW7Eö&öG’ÒÆäv—D‡V$gVæF–æt6öÖÖVçE&WVW7BÀ¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"V&Æ–2gVæF–ærÖ6öÖÖVçB6–væÂÆâ"’¢•Ð¦7–æ2fâÆåöv—F‡V%ögVæF–æuö6öÖÖVçB€¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V$gVæF–æt6öÖÖVçE&WVW7CâÀ¢’Óâ§6öãÄv—D‡V$gVæF–æt6öÖÖVçEÆãâ°¢§6öâ†gVæF–æuö6öÖÖVçE÷Æâ„v—D‡V$gVæF–æt6öÖÖVçD–çWB°¢&W÷6—F÷'“¢&WVW7Bç&W÷6—F÷'’À¢—77VU÷W&Ã¢&WVW7Bæ—77VU÷W&ÂÀ¢F—FÆS¢&WVW7BçF—FÆRÀ¢&öG“¢&WVW7Bæ&öG’À¢6öÖÖVçEö&öG“¢&WVW7Bæ6öÖÖVçEö&öG’À¢6öçG&–'WF÷%öÆöv–ã¢&WVW7Bæ6öçG&–'WF÷%öÆöv–âÀ¢6öÖÖVçEö–C¢&WVW7Bæ6öÖÖVçEö–BÀ¢gVæF–æuö•ö&6U÷W&Ã¢&WVW7BægVæF–æuö•ö&6U÷W&ÂÀ¢W†—7F–æuö–FV×÷FVæ7•ö¶W—3¢&WVW7BæW†—7F–æuö–FV×÷FVæ7•ö¶W—2À¢Ò’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"ö6Æ–ÒÖ6öÖÖVçB×Æâ"À¢&WVW7Eö&öG’ÒÆäv—D‡V$6Æ–Ô6öÖÖVçE&WVW7BÀ¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"V&Æ–26Æ–ÒÖ6öÖÖVçB&W6W'fF–öâÆâ"’¢•Ð¦7–æ2fâÆåöv—F‡V%ö6Æ–Õö6öÖÖVçB€¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V$6Æ–Ô6öÖÖVçE&WVW7CâÀ¢’Óâ§6öãÄv—D‡V$6Æ–Ô6öÖÖVçEÆãâ°¢§6öâ†6Æ–Õö6öÖÖVçE÷Æâ„v—D‡V$6Æ–Ô6öÖÖVçD–çWB°¢&W÷6—F÷'“¢&WVW7Bç&W÷6—F÷'’À¢—77VU÷W&Ã¢&WVW7Bæ—77VU÷W&ÂÀ¢F—FÆS¢&WVW7BçF—FÆRÀ¢&öG“¢&WVW7Bæ&öG’À¢6öÖÖVçEö&öG“¢&WVW7Bæ6öÖÖVçEö&öG’À¢6öçG&–'WF÷%öÆöv–ã¢&WVW7Bæ6öçG&–'WF÷%öÆöv–âÀ¢6öÖÖVçEö–C¢&WVW7Bæ6öÖÖVçEö–BÀ¢6Æ–ÕövUöÖ–çWFW3¢&WVW7Bæ6Æ–ÕövUöÖ–çWFW2À¢&öw&W75÷6–væÅö6÷VçC¢&WVW7Bç&öw&W75÷6–væÅö6÷VçBÀ¢7F—fUö6Æ–ÕöÆöv–ã¢&WVW7Bæ7F—fUö6Æ–ÕöÆöv–âÀ¢Ò’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"÷&ööbÖ6öÖÖVçB×Æâ"À¢&WVW7Eö&öG’ÒÆäv—D‡V%&ööd6öÖÖVçE&WVW7BÀ¢&W7öç6W2‚‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"&ööb6öÖÖVçBÖ&¶F÷vâæB6†V6²×'VâÆâ"’¢•Ð¦7–æ2fâÆåöv—F‡V%÷&ööeö6öÖÖVçB€¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V%&ööd6öÖÖVçE&WVW7CâÀ¢’Óâ§6öãÄv—D‡V%&ööd6öÖÖVçEÆãâ°¢ÆWB6öÖÖVçBÒv—D‡V%&ööd6öÖÖVçB°¢&÷VçG•ö–C¢&WVW7Bæ&÷VçG•ö–BÀ¢&ööe÷W&Ã¢&WVW7Bç&ööe÷W&ÂÀ¢fW&–f–W%÷7VÖÖ'“¢&WVW7BçfW&–f–W%÷7VÖÖ'’À¢6WGFÆVÖVçE÷W&Ã¢&WVW7Bç6WGFÆVÖVçE÷W&ÂÀ¢Ó°¢§6öâ‡&ööeö6öÖÖVçE÷Æâ†6öÖÖVçB’§Ð ¢5·WFö—£§F‚€¢÷7BÀ¢F‚Ò"÷cöv—F‡V"÷&ööbÖ6öÖÖVçB×ÆâÖg&öÒ×&ööb"À¢&WVW7Eö&öG’ÒÆäv—D‡V%&ööd6öÖÖVçDg&öÕ&ööe&WVW7BÀ¢&W7öç6W2€¢‡7FGW2Ò#ÂFW67&—F–öâÒ$v—D‡V"&ööb6öÖÖVçBÆâFW&—fVBg&öÒ7F÷&VBV&Æ–2&ööb"’À¢‡7FGW2ÒCBÂFW67&—F–öâÒ%&ööbæ÷Bf÷VæBÂ&—fFRÂ÷"Ö—76–ærfW&–f–W"&W7VÇB"¢¢•Ð¦7–æ2fâÆåöv—F‡V%÷&ööeö6öÖÖVçEög&öÕ÷&ööb€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢§6öâ‡&WVW7B“¢§6öãÅÆäv—D‡V%&ööd6öÖÖVçDg&öÕ&ööe&WVW7CâÀ¢’Óâ&W7VÇCÄ§6öãÄv—D‡V%&ööd6öÖÖVçEÆãâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢v—F‡V%÷&ööeö6öÖÖVçE÷Æåög&öÕ÷&ööb€¢fæWGv÷&²À¢g7FFRçV&Æ–5ö&6U÷W&ÂÀ¢&WVW7Bç&ööeö–BÀ¢&WVW7Bç6WGFÆVÖVçE÷W&ÂÀ¢¢æÖ„§6öâ§Ð ¦fâv—F‡V%÷&ööeö6öÖÖVçE÷Æåög&öÕ÷&ööb€¢æWGv÷&³¢d&÷VçG”æWGv÷&²À¢V&Æ–5ö&6U÷W&Ã¢g7G"À¢&ööeö–C¢WV–BÀ¢6WGFÆVÖVçE÷W&Ã¢÷F–öãÅ7G&–æsâÀ¢’Óâ&W7VÇCÄv—D‡V%&ööd6öÖÖVçEÆâÂ7FGW46öFSâ°¢ÆWB&ööbÒæWGv÷&²ç&öög2ævWB‚g&ööeö–B’æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢–b&ööbç&—f7’ÓÒ&—f7”ÆWfVÃ£¥&—fFR°¢&WGW&âW'"…7FGW46öFS£¤äõEôdõTäB“°¢Ð¢ÆWBfW&–f–W"ÒæWGv÷&°¢çfW&–f–W%÷&W7VÇG0¢ævWB‚g&ööbçfW&–f–W%÷&W7VÇEö–B¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢ÆWBfW&–f–W%÷7VÖÖ'’Ò–bfW&–f–W"ç7VÖÖ'’çG&–Ò‚’æ—5öV×G’‚’°¢f÷&ÖB‚'³£÷ÒfW&–f–W"66WFVB"ÂfW&–f–W"æ¶–æB¢ÒVÇ6R°¢f÷&ÖB‚'³£÷Ó¢·Ò"ÂfW&–f–W"æ¶–æBÂfW&–f–W"ç7VÖÖ'’çG&–Ò‚’¢Ó°¢ÆWB6öÖÖVçBÒv—D‡V%&ööd6öÖÖVçB°¢&÷VçG•ö–C¢&ööbæ&÷VçG•ö–BÀ¢&ööe÷W&Ã¢f÷&ÖB€¢'·Ò÷V&Æ–2÷&öög2÷·Ò"À¢V&Æ–5ö&6U÷W&ÂçG&–ÕöVæEöÖF6†W2‚ròr’À¢&ööbæ–@¢’À¢fW&–f–W%÷7VÖÖ'’À¢6WGFÆVÖVçE÷W&ÂÀ¢Ó°¢ö²‡&ööeö6öÖÖVçE÷Æâ†6öÖÖVçB’§Ð ¢5·WFö—£§F‚†vWBÂF‚Ò"÷cö&÷VçF–W2÷¶–GÒ"•Ð¦7–æ2fâ&÷VçG•÷7FGW2€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚†–B“¢FƒÅWV–CâÀ¢’Óâ&W7VÇCÄ§6öãÄ&÷VçG•7FGW5&W7öç6SâÂ7FGW46öFSâ°¢&÷VçG•÷7FGW5÷6æ6†÷B‚g7FFRÂ–B’æv—BæÖ„§6öâ§Ð ¦7–æ2fâ&÷VçG•÷7FGW5÷6æ6†÷B€¢7FFS¢e6†&VE7FFRÀ¢–C¢WV–BÀ¢’Óâ&W7VÇCÄ&÷VçG•7FGW5&W7öç6RÂ7FGW46öFSâ°¢6W'f–6U÷'VçF–ÖS£¦&÷VçG•÷7FGW2‡7FFRç7F÷&Ræ5÷&Vb‚’Âg7FFRææWGv÷&²Â–B¢æv—@¢æÖöW'"‡ÆW'&÷'Â°¢–bW'&÷"ç&WG'–&ÆR‚’°¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ ¢ÒVÇ6R°¢7FGW46öFS£¤äõEôdõTä@¢Ð¢Ò§Ð ¦7–æ2fâV&Æ–5÷&ööe÷vR€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚†–B“¢FƒÅWV–CâÀ¢’Óâ&W7VÇCÄ‡FÖÃÅ7G&–æsâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢ÆWB&ööbÒæWGv÷&°¢ç&öög0¢ævWB‚f–B¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ð¢æ6ÆöæR‚“°¢–b&ööbç&—f7’ÓÒ&—f7”ÆWfVÃ£¥&—fFR°¢&WGW&âW'"…7FGW46öFS£¤äõEôdõTäB“°¢Ð¢ÆWBfW&–f–W"ÒæWGv÷&°¢çfW&–f–W%÷&W7VÇG0¢ævWB‚g&ööbçfW&–f–W%÷&W7VÇEö–B¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ð¢æ6ÆöæR‚“° ¢ö²„‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%÷&ööe÷vR‚g&ööbÂgfW&–f–W"’’§Ð ¦7–æ2fâV&Æ–5övVçE÷&öf–ÆR€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚†–B“¢FƒÅWV–CâÀ¢’Óâ&W7VÇCÄ‡FÖÃÅ7G&–æsâÂ7FGW46öFSâ°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢ÆWBvVçBÒæWGv÷&°¢ævVçG0¢ævWB‚f–B¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ð¢æ6ÆöæR‚“°¢ÆWB&WWFF–öå÷66÷&RÒæWGv÷&°¢ç&WWFF–öåöWfVçG0¢çfÇVW2‚¢æf–ÇFW"‡ÆWfVçGÂWfVçBævVçEö–BÓÒ–B¢æÖ‡ÆWfVçGÂWfVçBæFVÇF¢ç7VÒ‚“°¢ÆWB66WFVEö6÷VçBÒæWGv÷&°¢ç&WWFF–öåöWfVçG0¢çfÇVW2‚¢æf–ÇFW"‡ÆWfVçGÂWfVçBævVçEö–BÓÒ–BbbWfVçBæFVÇFâ¢æ6÷VçB‚“°¢ÆWB&VÅ÷–Eö–çFVçG2ÒæWGv÷&°¢ç6WGFÆVÖVçG0¢çfÇVW2‚¢æfÆEöÖ‡Ç6WGFÆVÖVçGÂg6WGFÆVÖVçBç–÷WEö–çFVçG2¢æf–ÇFW"‡Æ–çFVçGÂ°¢–çFVçBç&V6—–VçEövVçEö–BÓÒ–@¢bb–çFVçBç7FGW2ÓÒ–÷WE7FGW3£¥–@¢bb–çFVçBç&–ÂÒ–ÖVçE&–Ã£¥6–×VÆFV@¢Ò¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢ÆWB–Eö7W'&Væ7’Ò&VÅ÷–Eö–çFVçG0¢æ—FW"‚¢æf–æB‡Æ–çFVçGÂ–çFVçBæÖ÷VçBæ7W'&Væ7’ÓÒ'W6F2"¢æ÷%öVÇ6R‡ÇÂ&VÅ÷–Eö–çFVçG2æf—'7B‚’¢æÖ‡Æ–çFVçGÂ–çFVçBæÖ÷VçBæ7W'&Væ7’æ6ÆöæR‚’¢çVçw&ö÷%öVÇ6R‡ÇÂ'W6F2"çFõ÷7G&–ær‚’“°¢ÆWB–EöÖ–æ÷"Ò&VÅ÷–Eö–çFVçG0¢æ—FW"‚¢æf–ÇFW"‡Æ–çFVçGÂ–çFVçBæÖ÷VçBæ7W'&Væ7’ÓÒ–Eö7W'&Væ7’æ5÷7G"‚’¢æÖ‡Æ–çFVçGÂ–çFVçBæÖ÷VçBæÖ÷VçB¢ç7VÒ‚“° ¢ö²„‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%övVçE÷&öf–ÆR€¢fvVçBÀ¢66WFVEö6÷VçBÀ¢&WWFF–öå÷66÷&RÀ¢–EöÖ–æ÷"À¢g–Eö7W'&Væ7’À¢’’§Ð ¦7–æ2fâV&Æ–5÷fW&–f–W%÷&öf–ÆR€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚†¶–æB“¢FƒÅ7G&–æsâÀ¢’Óâ&W7VÇCÄ‡FÖÃÅ7G&–æsâÂ7FGW46öFSâ°¢ÆWBfW&–f–W%ö¶–æBÒ'6U÷fW&–f–W%ö¶–æB‚f¶–æB’æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢ÆWB7FG2Ò°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢ÆWB&W7VÇG2ÒæWGv÷&°¢çfW&–f–W%÷&W7VÇG0¢çfÇVW2‚¢æf–ÇFW"‡Ç&W7VÇGÂ&W7VÇBæ¶–æBÓÒfW&–f–W%ö¶–æB¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢ÆWBF÷FÅö6†V6·2Ò&W7VÇG2æÆVâ‚“°¢ÆWB66WFVEö6÷VçBÒ&W7VÇG0¢æ—FW"‚¢æf–ÇFW"‡Ç&W7VÇGÂ&W7VÇBæFV6—6–öâÓÒfW&–f–6F–öäFV6—6–öã£¤66WFVB¢æ6÷VçB‚“°¢ÆWB&V¦V7FVEö6÷VçBÒ&W7VÇG0¢æ—FW"‚¢æf–ÇFW"‡Ç&W7VÇGÂ&W7VÇBæFV6—6–öâÓÒfW&–f–6F–öäFV6—6–öã£¥&V¦V7FVB¢æ6÷VçB‚“°¢ÆWBæVVG5÷&Wf–Wuö6÷VçBÒ&W7VÇG0¢æ—FW"‚¢æf–ÇFW"‡Ç&W7VÇGÂ&W7VÇBæFV6—6–öâÓÒfW&–f–6F–öäFV6—6–öã£¤æVVG5&Wf–Wr¢æ6÷VçB‚“°¢ÆWBfW&vUö6öæf–FVæ6RÒ–bF÷FÅö6†V6·2ÓÒ°¢ã ¢ÒVÇ6R°¢&W7VÇG2æ—FW"‚’æÖ‡Ç&W7VÇGÂ&W7VÇBæ6öæf–FVæ6R’ç7VÓ££Æc3#â‚’òF÷FÅö6†V6·22c3 ¢Ó°¢vV%÷V&Æ–3£¥fW&–f–W%&öf–ÆU7FG2°¢F÷FÅö6†V6·2À¢66WFVEö6÷VçBÀ¢&V¦V7FVEö6÷VçBÀ¢æVVG5÷&Wf–Wuö6÷VçBÀ¢fW&vUö6öæf–FVæ6RÀ¢Ð¢Ó°¢ö²„‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%÷fW&–f–W%÷&öf–ÆR€¢ff÷&ÖB‚'·fW&–f–W%ö¶–æC£÷Ò"’À¢g7FG2À¢’’§Ð ¦7–æ2fâV&Æ–5ö&÷VçG•öfVVE÷vR…7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâ’Óâ‡FÖÃÅ7G&–æsâ°¢ÆWB&÷VçF–W2Ò°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&²æÆ—7Eö6Æ–Ö&ÆUö&÷VçF–W2‚¢Ó°¢ÆWB—FV×2ÒvV%÷V&Æ–3£§V&Æ–5ö&÷VçG•öfVVB‚f&÷VçF–W2Âg7FFRçV&Æ–5ö&6U÷W&Â“°¢‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%ö&÷VçG•öfVVE÷vR‚f—FV×2’§Ð ¦7–æ2fâV&Æ–5ögVæF–æuöfVVE÷vR…7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâ’Óâ‡FÖÃÅ7G&–æsâ°¢ÆWB—FV×2Ò°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢V&Æ–5ögVæF–æuöfVVEö—FV×2‚fæWGv÷&²Âg7FFRçV&Æ–5ö&6U÷W&Â¢Ó°¢‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%ögVæF–æuöfVVE÷vR‚f—FV×2’§Ð ¦7–æ2fâV&Æ–5ö&÷VçG•÷vR€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚†–B“¢FƒÅWV–CâÀ¢’Óâ&W7VÇCÄ‡FÖÃÅ7G&–æsâÂ7FGW46öFSâ°¢ÆWB7FGW2Ò°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢æWGv÷&²ç7FGW2†–B’æÖöW'"‡Å÷Â7FGW46öFS£¤äõEôdõTäB“ð¢Ó°¢–b7FGW2æ&÷VçG’ç&—f7’ÓÒ&—f7”ÆWfVÃ£¥&—fFR°¢&WGW&âW'"…7FGW46öFS£¤äõEôdõTäB“°¢Ð¢ö²„‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%÷V&Æ–5ö&÷VçG•÷vR€¢gV&Æ–5ö&÷VçG•÷vUöÖöFVÂ‚g7FGW2Âg7FFRçV&Æ–5ö&6U÷W&Â’À¢’’§Ð ¦fâV&Æ–5ögVæF–æuöfVVEö—FV×2€¢æWGv÷&³¢d&÷VçG”æWGv÷&²À¢V&Æ–5ö&6U÷W&Ã¢g7G"À¢’ÓâfV3ÇvV%÷V&Æ–3£¥V&Æ–4gVæF–ætfVVD—FVÓâ°¢ÆWB×WB—FV×2ÒæWGv÷&°¢æ&÷VçF–W0¢çfÇVW2‚¢æf–ÇFW%öÖ‡Æ&÷VçG—ÂæWGv÷&²ç7FGW2†&÷VçG’æ–B’æö²‚’¢æf–ÇFW"‡V&Æ–5÷7FGW5ö66WG5ögVæF–ær¢æÖ‡Ç7FGW7ÂV&Æ–5ögVæF–æuöfVVEö—FVÒ‚g7FGW2ÂV&Æ–5ö&6U÷W&Â’¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢—FV×2ç6÷'Eö'’‡ÆÆVgBÂ&–v‡GÂ°¢V&Æ–5÷&VÖ–æ–æu÷'F—F–öåö6÷VçB‡&–v‡B¢æ6×‚gV&Æ–5÷&VÖ–æ–æu÷'F—F–öåö6÷VçB†ÆVgB’¢çF†Vå÷v—F‚‡ÇÂ&–v‡Bæ7&VFVEöBæ6×‚fÆVgBæ7&VFVEöB’¢çF†Vå÷v—F‚‡ÇÂÆVgBæ&÷VçG•ö–Bæ6×‚g&–v‡Bæ&÷VçG•ö–B’¢Ò“°¢—FV×0§Ð ¦fâV&Æ–5÷7FGW5ö66WG5ögVæF–ær‡7FGW3¢d&÷VçG•7FGW5&W7öç6R’Óâ&ööÂ°¢ÆWBV&Æ–5öæöå÷FW&Ö–æÂÒ7FGW2æ&÷VçG’ç&—f7’Ò&—f7”ÆWfVÃ£¥&—fFP¢bbÖF6†W2€¢7FGW2æ&÷VçG’ç7FGW2À¢&÷VçG•7FGW3£¥–@¢Â&÷VçG•7FGW3£¥&VgVæFV@¢Â&÷VçG•7FGW3£¤F—7WFV@¢Â&÷VçG•7FGW3£¤W‡—&V@¢“°¢ÆWB'F—F–öå÷&VÖ–æ–ærÒ7FGW0¢ægVæF–æu÷7VÖÖ'¢ç'F—F–öç0¢æ—FW"‚¢æç’‡Ç'F—F–öçÂ'F—F–öâç&VÖ–æ–æræÖ÷VçBâ“°¢V&Æ–5öæöå÷FW&Ö–æÂbb‡'F—F–öå÷&VÖ–æ–ærÇÂ7FGW2ægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræÖ÷VçBâ§Ð ¦fâV&Æ–5÷&VÖ–æ–æu÷'F—F–öåö6÷VçB†—FVÓ¢gvV%÷V&Æ–3£¥V&Æ–4gVæF–ætfVVD—FVÒ’ÓâW6—¦R°¢—FVÒægVæF–æu÷'F—F–öç0¢æ—FW"‚¢æf–ÇFW"‡Ç'F—F–öçÂ'F—F–öâç&VÖ–æ–æuöÖ–æ÷"â¢æ6÷VçB‚§Ð ¦fâV&Æ–5ögVæF–æuöfVVEö—FVÒ€¢7FGW3¢d&÷VçG•7FGW5&W7öç6RÀ¢V&Æ–5ö&6U÷W&Ã¢g7G"À¢’ÓâvV%÷V&Æ–3£¥V&Æ–4gVæF–ætfVVD—FVÒ°¢ÆWB’ÒV&Æ–5ö&6U÷W&ÂçG&–ÕöVæEöÖF6†W2‚ròr“°¢ÆWB&÷VçG’Òg7FGW2æ&÷VçG“°¢ÆWBgVæF–æuö–çFVçE÷W&ÂÒf÷&ÖB‚'¶—Ò÷cö&÷VçF–W2÷·ÒögVæF–ærÖ–çFVçG2"Â&÷VçG’æ–B“°¢ÆWBV&Æ–5÷W&ÂÒf÷&ÖB‚'¶—Ò÷V&Æ–2ö&÷VçF–W2÷·Ò"Â&÷VçG’æ–B“°¢ÆWBgVæF–æu÷'F—F–öç2Ò7FGW0¢ægVæF–æu÷7VÖÖ'¢ç'F—F–öç0¢æ—FW"‚¢æÖ‡Ç'F—F–öçÂvV%÷V&Æ–3£¥V&Æ–4gVæF–æu'F—F–öâ°¢&–Ã¢f÷&ÖB‚'³£÷Ò"Â'F—F–öâç&–Â’À¢F&vWEöÖ–æ÷#¢'F—F–öâçF&vWBæÖ÷VçBÀ¢6öæf—&ÖVEöÖ–æ÷#¢'F—F–öâæ6öæf—&ÖVBæÖ÷VçBÀ¢&VÖ–æ–æuöÖ–æ÷#¢'F—F–öâç&VÖ–æ–æræÖ÷VçBÀ¢7W'&Væ7“¢'F—F–öâçF&vWBæ7W'&Væ7’æ6ÆöæR‚’À¢6öçG&–'WF–öåö6÷VçC¢'F—F–öâæ6öçG&–'WF–öåö6÷VçBÀ¢W67&÷uö6÷VçC¢'F—F–öâæW67&÷uö6÷VçBÀ¢6Æ–Ö&ÆS¢'F—F–öâæ6Æ–Ö&ÆRÀ¢Ò¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢ÆWBgVæF–æuö–çFVçEöW†×ÆW2ÒvV%÷V&Æ–3£§V&Æ–5ögVæF–æuö–çFVçEöW†×ÆW2€¢f&÷VçG’æ–BçFõ÷7G&–ær‚’À¢fgVæF–æuö–çFVçE÷W&ÂÀ¢gV&Æ–5÷W&ÂÀ¢ff÷&ÖB‚'³£÷Ò"Â&÷VçG’ægVæF–æuöÖöFR’À¢7FGW2ægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræÖ÷VçBÀ¢g7FGW2ægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræ7W'&Væ7’À¢fgVæF–æu÷'F—F–öç2À¢“°¢vV%÷V&Æ–3£¥V&Æ–4gVæF–ætfVVD—FVÒ°¢&÷VçG•ö–C¢&÷VçG’æ–BçFõ÷7G&–ær‚’À¢F—FÆS¢&÷VçG’çF—FÆRæ6ÆöæR‚’À¢FV×ÆFU÷6ÇVs¢&÷VçG’çFV×ÆFU÷6ÇVræ6ÆöæR‚’À¢Ö÷VçEöÖ–æ÷#¢&÷VçG’æÖ÷VçBæÖ÷VçBÀ¢7W'&Væ7“¢&÷VçG’æÖ÷VçBæ7W'&Væ7’æ6ÆöæR‚’À¢gVæF–æuöÖöFS¢f÷&ÖB‚'³£÷Ò"Â&÷VçG’ægVæF–æuöÖöFR’À¢7FGW3¢f÷&ÖB‚'³£÷Ò"Â&÷VçG’ç7FGW2’À¢&—f7“¢f÷&ÖB‚'³£÷Ò"Â&÷VçG’ç&—f7’’À¢FW&×5ö†6ƒ¢&÷VçG’çFW&×5ö†6‚æ6ÆöæR‚’À¢7&VFVEöC¢&÷VçG’æ7&VFVEöBçFõ÷&f3333’‚’À¢6Æ–Ö&ÆS¢7FGW2ægVæF–æu÷7VÖÖ'’æ6Æ–Ö&ÆRÀ¢gVæF–æu÷F&vWEöÖ–æ÷#¢7FGW2ægVæF–æu÷7VÖÖ'’çF&vWBæÖ÷VçBÀ¢gVæF–æuöÆ–VEöÖ–æ÷#¢7FGW2ægVæF–æu÷7VÖÖ'’æÆ–VBæÖ÷VçBÀ¢gVæF–æu÷&VÖ–æ–æuöÖ–æ÷#¢7FGW2ægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræÖ÷VçBÀ¢6öçG&–'WF–öåö6÷VçC¢7FGW2ægVæF–æu÷7VÖÖ'’æ6öçG&–'WF–öåö6÷VçBÀ¢V&Æ–5÷W&ÂÀ¢7FGW5÷W&Ã¢f÷&ÖB‚'¶—Ò÷cö&÷VçF–W2÷·Ò"Â&÷VçG’æ–B’À¢FV×ÆFU÷W&Ã¢f÷&ÖB‚'¶—Ò÷V&Æ–2÷FV×ÆFW2÷·Ò"Â&÷VçG’çFV×ÆFU÷6ÇVr’À¢gVæF–æuö–çFVçE÷W&ÂÀ¢gVæF–æuö6öçG&–'WF–öå÷W&Ã¢f÷&ÖB‚'¶—Ò÷cö&÷VçF–W2÷·ÒögVæF–ærÖ6öçG&–'WF–öç2"Â&÷VçG’æ–B’À¢gVæF–æu÷'F—F–öç2À¢gVæF–æuö–çFVçEöW†×ÆW2À¢Ð§Ð ¦fâV&Æ–5ö&÷VçG•÷vUöÖöFVÂ€¢7FGW3¢d&÷VçG•7FGW5&W7öç6RÀ¢V&Æ–5ö&6U÷W&Ã¢g7G"À¢’ÓâvV%÷V&Æ–3£¥V&Æ–4&÷VçG•vR°¢ÆWB’ÒV&Æ–5ö&6U÷W&ÂçG&–ÕöVæEöÖF6†W2‚ròr“°¢ÆWB&÷VçG’Òg7FGW2æ&÷VçG“°¢ÆWBfW&–f–6F–öå÷G—RÒ7FGW0¢çfW&–f–W%÷&W7VÇG0¢æ—FW"‚¢æÖ…ö'•ö¶W’‡Ç&W7VÇGÂ&W7VÇBæ7&VFVEöB¢æÖ‡Ç&W7VÇGÂf÷&ÖB‚'³£÷Ò"Â&W7VÇBæ¶–æB’¢æ÷%öVÇ6R‡ÇÂ°¢vV%÷V&Æ–3£¦&÷VçG•÷FV×ÆFW2‚¢æ–çFõö—FW"‚¢æf–æB‡ÇFV×ÆFWÂFV×ÆFRç6ÇVrÓÒ&÷VçG’çFV×ÆFU÷6ÇVr¢æÖ‡ÇFV×ÆFWÂFV×ÆFRçfW&–f–W"çFõ÷7G&–ær‚’¢Ò¢çVçw&ö÷%öVÇ6R‡ÇÂ%Væ¶æ÷vâ"çFõ÷7G&–ær‚’“°¢ÆWB&ööe÷W&Ç2Ò7FGW0¢ç&öög0¢æ—FW"‚¢æf–ÇFW"‡Ç&öögÂ&ööbç&—f7’Ò&—f7”ÆWfVÃ£¥&—fFR¢æÖ‡Ç&öögÂf÷&ÖB‚'¶—Ò÷V&Æ–2÷&öög2÷·Ò"Â&ööbæ–B’¢æ6öÆÆV7B‚“°¢ÆWBV&Æ–5÷W&ÂÒf÷&ÖB‚'¶—Ò÷V&Æ–2ö&÷VçF–W2÷·Ò"Â&÷VçG’æ–B“°¢ÆWBgVæF–æu÷'F—F–öç2Ò7FGW0¢ægVæF–æu÷7VÖÖ'¢ç'F—F–öç0¢æ—FW"‚¢æÖ‡Ç'F—F–öçÂvV%÷V&Æ–3£¥V&Æ–4gVæF–æu'F—F–öâ°¢&–Ã¢f÷&ÖB‚'³£÷Ò"Â'F—F–öâç&–Â’À¢F&vWEöÖ–æ÷#¢'F—F–öâçF&vWBæÖ÷VçBÀ¢6öæf—&ÖVEöÖ–æ÷#¢'F—F–öâæ6öæf—&ÖVBæÖ÷VçBÀ¢&VÖ–æ–æuöÖ–æ÷#¢'F—F–öâç&VÖ–æ–æræÖ÷VçBÀ¢7W'&Væ7“¢'F—F–öâçF&vWBæ7W'&Væ7’æ6ÆöæR‚’À¢6öçG&–'WF–öåö6÷VçC¢'F—F–öâæ6öçG&–'WF–öåö6÷VçBÀ¢W67&÷uö6÷VçC¢'F—F–öâæW67&÷uö6÷VçBÀ¢6Æ–Ö&ÆS¢'F—F–öâæ6Æ–Ö&ÆRÀ¢Ò¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢ÆWBgVæF–æuö–çFVçE÷W&ÂÒf÷&ÖB‚'¶—Ò÷cö&÷VçF–W2÷·ÒögVæF–ærÖ–çFVçG2"Â&÷VçG’æ–B“°¢ÆWBgVæF–æuö–çFVçEöW†×ÆW2ÒvV%÷V&Æ–3£§V&Æ–5ögVæF–æuö–çFVçEöW†×ÆW2€¢f&÷VçG’æ–BçFõ÷7G&–ær‚’À¢fgVæF–æuö–çFVçE÷W&ÂÀ¢gV&Æ–5÷W&ÂÀ¢ff÷&ÖB‚'³£÷Ò"Â&÷VçG’ægVæF–æuöÖöFR’À¢7FGW2ægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræÖ÷VçBÀ¢g7FGW2ægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræ7W'&Væ7’À¢fgVæF–æu÷'F—F–öç2À¢“°¢ÆWBfW&–f–W%÷&W7VÇEöÆ–æ·2Ò7FGW0¢çfW&–f–W%÷&W7VÇG0¢æ—FW"‚¢æÖ‡Ç&W7VÇGÂvV%÷V&Æ–3£¥V&Æ–4&÷VçG•&V6÷&DÆ–æ²°¢Æ&VÃ¢f÷&ÖB€¢'³£÷Ò³£÷ÒfW&–f–W"&W7VÇB·Ò"À¢&W7VÇBæ¶–æBÂ&W7VÇBæFV6—6–öâÂ&W7VÇBæ–@¢’À¢W&Ã¢f÷&ÖB‚'·V&Æ–5÷W&ÇÒ7fW&–f–W"×&W7VÇG2"’À¢Ò¢æ6öÆÆV7B‚“°¢ÆWB6WGFÆVÖVçEöÆ–æ·2Ò7FGW0¢ç6WGFÆVÖVçG0¢æ—FW"‚¢æÖ‡Ç6WGFÆVÖVçGÂ°¢ÆWB–E÷–÷WG2Ò6WGFÆVÖVç@¢ç–÷WEö–çFVçG0¢æ—FW"‚¢æf–ÇFW"‡Æ–çFVçGÂ–çFVçBç7FGW2ÓÒ–÷WE7FGW3£¥–B¢æ6÷VçB‚“°¢ÆWBF÷FÅ÷–÷WG2Ò6WGFÆVÖVçBç–÷WEö–çFVçG2æÆVâ‚“°¢vV%÷V&Æ–3£¥V&Æ–4&÷VçG•&V6÷&DÆ–æ²°¢Æ&VÃ¢f÷&ÖB€¢'³£÷Ò6WGFÆVÖVçB·Ò‡·–E÷–÷WG7Ò÷·F÷FÅ÷–÷WG7Ò–÷WG2–B’"À¢6WGFÆVÖVçBç&–ÂÂ6WGFÆVÖVçBæ–@¢’À¢W&Ã¢f÷&ÖB‚'·V&Æ–5÷W&ÇÒ76WGFÆVÖVçG2"’À¢Ð¢Ò¢æ6öÆÆV7B‚“°¢ÆWBFV×ÆFU÷6–væÅöÆ–æ·2Ò7FGW0¢çFV×ÆFU÷6–væÇ0¢æ—FW"‚¢æÖ‡Ç6–væÇÂvV%÷V&Æ–3£¥V&Æ–4&÷VçG•&V6÷&DÆ–æ²°¢Æ&VÃ¢f÷&ÖB‚'·ÒFV×ÆFR6–væÂ·Ò"Â6–væÂçFV×ÆFU÷6ÇVrÂ6–væÂæ–B’À¢W&Ã¢f÷&ÖB‚'¶—Ò÷V&Æ–2÷FV×ÆFW2÷·Ò"Â6–væÂçFV×ÆFU÷6ÇVr’À¢Ò¢æ6öÆÆV7B‚“°¢vV%÷V&Æ–3£¥V&Æ–4&÷VçG•vR°¢&÷VçG•ö–C¢&÷VçG’æ–BçFõ÷7G&–ær‚’À¢F—FÆS¢&÷VçG’çF—FÆRæ6ÆöæR‚’À¢FV×ÆFU÷6ÇVs¢&÷VçG’çFV×ÆFU÷6ÇVræ6ÆöæR‚’À¢Ö÷VçEöÖ–æ÷#¢&÷VçG’æÖ÷VçBæÖ÷VçBÀ¢7W'&Væ7“¢&÷VçG’æÖ÷VçBæ7W'&Væ7’æ6ÆöæR‚’À¢gVæF–æuöÖöFS¢f÷&ÖB‚'³£÷Ò"Â&÷VçG’ægVæF–æuöÖöFR’À¢&—f7“¢f÷&ÖB‚'³£÷Ò"Â&÷VçG’ç&—f7’’À¢7FGW3¢f÷&ÖB‚'³£÷Ò"Â&÷VçG’ç7FGW2’À¢FW&×5ö†6ƒ¢&÷VçG’çFW&×5ö†6‚æ6ÆöæR‚’À¢7&VFVEöC¢&÷VçG’æ7&VFVEöBçFõ÷&f3333’‚’À¢fW&–f–6F–öå÷G—RÀ¢6Æ–Ö&ÆS¢7FGW2ægVæF–æu÷7VÖÖ'’æ6Æ–Ö&ÆRÀ¢gVæF–æu÷F&vWEöÖ–æ÷#¢7FGW2ægVæF–æu÷7VÖÖ'’çF&vWBæÖ÷VçBÀ¢gVæF–æuöÆ–VEöÖ–æ÷#¢7FGW2ægVæF–æu÷7VÖÖ'’æÆ–VBæÖ÷VçBÀ¢gVæF–æu÷&VÖ–æ–æuöÖ–æ÷#¢7FGW2ægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræÖ÷VçBÀ¢6öçG&–'WF–öåö6÷VçC¢7FGW2ægVæF–æu÷7VÖÖ'’æ6öçG&–'WF–öåö6÷VçBÀ¢V&Æ–5÷W&ÂÀ¢6Æ–Õ÷W&Ã¢f÷&ÖB‚'¶—Ò÷cö&÷VçF–W2÷·Òö6Æ–Ò"Â&÷VçG’æ–B’À¢7FGW5÷W&Ã¢f÷&ÖB‚'¶—Ò÷cö&÷VçF–W2÷·Ò"Â&÷VçG’æ–B’À¢FV×ÆFU÷W&Ã¢f÷&ÖB‚'¶—Ò÷V&Æ–2÷FV×ÆFW2÷·Ò"Â&÷VçG’çFV×ÆFU÷6ÇVr’À¢gVæF–æuö–çFVçE÷W&ÂÀ¢gVæF–æuö6öçG&–'WF–öå÷W&Ã¢f÷&ÖB‚'¶—Ò÷cö&÷VçF–W2÷·ÒögVæF–ærÖ6öçG&–'WF–öç2"Â&÷VçG’æ–B’À¢&ööe÷W&Ç2À¢gVæF–æu÷'F—F–öç2À¢gVæF–æuö–çFVçEöW†×ÆW2À¢fW&–f–W%÷&W7VÇEöÆ–æ·2À¢6WGFÆVÖVçEöÆ–æ·2À¢FV×ÆFU÷6–væÅöÆ–æ·2À¢Ð§Ð ¦7–æ2fâV&Æ–5ö6&–Æ—G•öfVVE÷vR…7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâ’Óâ‡FÖÃÅ7G&–æsâ°¢ÆWB†6&–Æ—F–W2ÂvVçG2Â&WWFF–öåöWfVçG2Â6WGFÆVÖVçG2’Ò°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢€¢æWGv÷&²æ6&–Æ—F–W2çfÇVW2‚’æ6ÆöæVB‚’æ6öÆÆV7C££ÅfV3Åóãâ‚’À¢æWGv÷&²ævVçG2çfÇVW2‚’æ6ÆöæVB‚’æ6öÆÆV7C££ÅfV3Åóãâ‚’À¢æWGv÷&°¢ç&WWFF–öåöWfVçG0¢çfÇVW2‚¢æ6ÆöæVB‚¢æ6öÆÆV7C££ÅfV3Åóãâ‚’À¢æWGv÷&²ç6WGFÆVÖVçG2çfÇVW2‚’æ6ÆöæVB‚’æ6öÆÆV7C££ÅfV3Åóãâ‚’À¢¢Ó°¢ÆWB—FV×2ÒvV%÷V&Æ–3£§V&Æ–5ö6&–Æ—G•öfVVB€¢f6&–Æ—F–W2À¢fvVçG2À¢g&WWFF–öåöWfVçG2À¢g6WGFÆVÖVçG2À¢g7FFRçV&Æ–5ö&6U÷W&ÂÀ¢“°¢‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%ö6&–Æ—G•öfVVE÷vR‚f—FV×2’§Ð ¦7–æ2fâV&Æ–5÷FV×ÆFUö–æFW‚‚’Óâ‡FÖÃÅ7G&–æsâ°¢‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%÷FV×ÆFUö–æFW‚€¢gvV%÷V&Æ–3£¦&÷VçG•÷FV×ÆFW2‚’À¢’§Ð ¦7–æ2fâV&Æ–5÷FV×ÆFU÷vR€¢7FFR‡7FFR“¢7FFSÅ6†&VE7FFSâÀ¢F‚‡6ÇVr“¢FƒÅ7G&–æsâÀ¢’Óâ&W7VÇCÄ‡FÖÃÅ7G&–æsâÂ7FGW46öFSâ°¢ÆWBFV×ÆFRÒvV%÷V&Æ–3£¦&÷VçG•÷FV×ÆFW2‚¢æ–çFõö—FW"‚¢æf–æB‡ÇFV×ÆFWÂFV×ÆFRç6ÇVrÓÒ6ÇVr¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢ÆWB7FG2Ò°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢ÆWBÖF6†–ærÒæWGv÷&°¢çFV×ÆFU÷6–væÇ0¢çfÇVW2‚¢æf–ÇFW"‡Ç6–væÇÂ6–væÂçFV×ÆFU÷6ÇVrÓÒ6ÇVrbb6–væÂç7V66W72¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢ÆWB7W'&Væ7’ÒÖF6†–æp¢æf—'7B‚¢æÖ‡Ç6–væÇÂ6–væÂæÖ÷VçBæ7W'&Væ7’æ6ÆöæR‚’¢çVçw&ö÷%öVÇ6R‡ÇÂ'W6F2"çFõ÷7G&–ær‚’“°¢ÆWB66WFVE÷fÇVUöÖ–æ÷"ÒÖF6†–æp¢æ—FW"‚¢æf–ÇFW"‡Ç6–væÇÂ6–væÂæÖ÷VçBæ7W'&Væ7’ÓÒ7W'&Væ7’¢æÖ‡Ç6–væÇÂ6–væÂæÖ÷VçBæÖ÷VçB¢ç7VÒ‚“°¢vV%÷V&Æ–3£¥FV×ÆFU7FG2°¢66WFVEö6÷VçC¢ÖF6†–æræÆVâ‚’À¢66WFVE÷fÇVUöÖ–æ÷"À¢7W'&Væ7’À¢Ð¢Ó°¢ö²„‡FÖÂ‡vV%÷V&Æ–3£§&VæFW%÷FV×ÆFU÷vR€¢gFV×ÆFRÀ¢6öÖR‚g7FG2’À¢’’§Ð ¦fâ'6U÷fW&–f–W%ö¶–æB†¶–æC¢g7G"’Óâ÷F–öãÅfW&–f–W$¶–æCâ°¢ÖF6‚¶–æBçFõö66–•öÆ÷vW&66R‚’ç&WÆ6R…²rÒrÂuòuÒÂ""’æ5÷7G"‚’°¢&ÖçVÂ"Óâ6öÖR…fW&–f–W$¶–æC£¤ÖçVÂ’À¢&§6öç66†VÖ"Óâ6öÖR…fW&–f–W$¶–æC£¤§6öå66†VÖ’À¢&Fö6¶W&6öÖÖæB"Óâ6öÖR…fW&–f–W$¶–æC£¤Fö6¶W$6öÖÖæB’À¢&v—F‡V&6’"Óâ6öÖR…fW&–f–W$¶–æC£¤v—D‡V$6’’À¢&‡GG6ÆÆ&6²"Óâ6öÖR…fW&–f–W$¶–æC£¤‡GG6ÆÆ&6²’À¢&–§VFvVf–ÇFW""Óâ6öÖR…fW&–f–W$¶–æC£¤”§VFvTf–ÇFW"’À¢òÓâæöæRÀ¢Ð§Ð ¦7–æ2fâ&V6÷&EöWfÅ÷'Vâ‡7FFS¢e6†&VE7FFRÂ'Vã¢WfÅ'Vâ’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢6W'f–6U÷'VçF–ÖS£§&V6÷&EöWfÅ÷'Vâ‡7FFRç7F÷&Ræ5÷&Vb‚’Âg7FFRæWfÅ÷'Vç2Â'Vâ¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"§Ð ¦fâ×WFF–öå÷7FGW2†W'&÷#¢6W'f–6U÷'VçF–ÖS£¤×WFF–öäW'&÷"’Óâ7FGW46öFR°¢–bW'&÷"æ—5ö–çfÆ–B‚’°¢7FGW46öFS£¤$Eõ$UTU5@¢ÒVÇ6R°¢7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ ¢Ð§Ð ¦7–æ2fâ‡–G&FUöæWGv÷&²‡7F÷&S¢e÷7Fw&W57F÷&R’Óâç–†÷s£¥&W7VÇCÄ&÷VçG”æWGv÷&³â°¢6W'f–6U÷'VçF–ÖS£¦‡–G&FUö&÷VçG•öæWGv÷&²‡7F÷&R’æv—@§Ð ¦fâÖöö&¦V7F—fUöW'&÷"†W'&÷#¢ö&¦V7F—fTW'&÷"’Óâ7FGW46öFR°¢ÖF6‚W'&÷"°¢ö&¦V7F—fTW'&÷#£¥&÷÷6Äæ÷Df÷VæB…ò¢Âö&¦V7F—fTW'&÷#£¤6öçG&–'WF–öäæVVDæ÷Df÷VæB…ò¢Âö&¦V7F—fTW'&÷#£¤6öçG&–'WF–öäöffW$æ÷Df÷VæB…ò¢Âö&¦V7F—fTW'&÷#£¥Væ¶æ÷vå'F–6—çB…ò’Óâ7FGW46öFS£¤äõEôdõTäBÀ¢ö&¦V7F—fTW'&÷#£¥7FÆT7F–öà¢Âö&¦V7F—fTW'&÷#£¥&÷÷6ÄW‡—&V@¢Âö&¦V7F—fTW'&÷#£¥&÷÷6ÄÇ&VG”66WFV@¢Âö&¦V7F—fTW'&÷#£¤–çfÆ–D7F–öâ…òÂò¢Âö&¦V7F—fTW'&÷#£¤æ÷E&VG’…ò¢Âö&¦V7F—fTW'&÷#£¤ÖVæFÖVçG5Væf–Æ&ÆRÓâ7FGW46öFS£¤4ôädÄ”5BÀ¢òÓâ7FGW46öFS£¤$Eõ$UTU5BÀ¢Ð§Ð ¦fâÖöö&¦V7F—fUöF%öW'&÷"†W'&÷#¢F$W'&÷"’Óâ7FGW46öFR°¢ÖF6‚W'&÷"°¢F$W'&÷#£¤ö&¦V7F—fTÇ&VG”W†—7G2…ò’ÂF$W'&÷#£¤ö&¦V7F—fU&Wf—6–öä6öæfÆ–7B²ââÒÓâ°¢7FGW46öFS£¤4ôädÄ”5@¢Ð¢F$W'&÷#£¤ö&¦V7F—fTæ÷Df÷VæB…ò’Óâ7FGW46öFS£¤äõEôdõTäBÀ¢òÓâ7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"À¢Ð§Ð ¦7–æ2fâÆöEöö&¦V7F—fR‡7FFS¢e6†&VE7FFRÂ–C¢–B’Óâ&W7VÇCÄö&¦V7F—fRÂ7FGW46öFSâ°¢–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢&WGW&â7F÷&P¢ævWEöö&¦V7F—fR†–B¢æv—@¢æÖöW'"†Ööö&¦V7F—fUöF%öW'&÷"“ð¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“°¢Ð¢7FFP¢ææWGv÷&°¢æÆö6²‚¢æW‡V7B‚'7FFRö—6öæVB"¢æö&¦V7F—fW0¢ævWB‚f–B¢æ6ÆöæVB‚¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB§Ð ¦7–æ2fâÆöEöö&¦V7F—fW2‡7FFS¢e6†&VE7FFR’Óâ&W7VÇCÅfV3Äö&¦V7F—fSâÂ7FGW46öFSâ°¢–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢&WGW&â7F÷&P¢æÆ—7Eöö&¦V7F—fW2‚¢æv—@¢æÖöW'"†Ööö&¦V7F—fUöF%öW'&÷"“°¢Ð¢ÆWB×WBö&¦V7F—fW2Ò7FFP¢ææWGv÷&°¢æÆö6²‚¢æW‡V7B‚'7FFRö—6öæVB"¢æö&¦V7F—fW0¢çfÇVW2‚¢æ6ÆöæVB‚¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢ö&¦V7F—fW2ç6÷'Eö'’‡ÆÆVgBÂ&–v‡GÂ°¢&–v‡@¢æ7&VFVEö@¢æ6×‚fÆVgBæ7&VFVEöB¢çF†Vå÷v—F‚‡ÇÂÆVgBæ–Bæ6×‚g&–v‡Bæ–B’¢Ò“°¢ö²†ö&¦V7F—fW2§Ð ¦7–æ2fâW'6—7EöæWuöö&¦V7F—fR€¢7FFS¢e6†&VE7FFRÀ¢ö&¦V7F—fS¢dö&¦V7F—fRÀ¢’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢7F÷&P¢æ7&VFUöö&¦V7F—fR†ö&¦V7F—fR¢æv—@¢æÖöW'"†Ööö&¦V7F—fUöF%öW'&÷"“ó°¢ÒVÇ6R°¢ÆWB×WBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢–bæWGv÷&²æö&¦V7F—fW2æ6öçF–ç5ö¶W’‚fö&¦V7F—fRæ–B’°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢æWGv÷&²æö&¦V7F—fW2æ–ç6W'B†ö&¦V7F—fRæ–BÂö&¦V7F—fRæ6ÆöæR‚’“°¢&WGW&âö²‚‚’“°¢Ð¢7FFP¢ææWGv÷&°¢æÆö6²‚¢æW‡V7B‚'7FFRö—6öæVB"¢æö&¦V7F—fW0¢æ–ç6W'B†ö&¦V7F—fRæ–BÂö&¦V7F—fRæ6ÆöæR‚’“°¢ö²‚‚’§Ð ¦7–æ2fâW'6—7Eöö&¦V7F—fU÷&WÆ6VÖVçB€¢7FFS¢e6†&VE7FFRÀ¢ö&¦V7F—fS¢dö&¦V7F—fRÀ¢W‡V7FVE÷&Wf—6–öã¢ScBÀ¢’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢–bÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&R°¢7F÷&P¢ç&WÆ6Uöö&¦V7F—fR†ö&¦V7F—fRÂW‡V7FVE÷&Wf—6–öâ¢æv—@¢æÖöW'"†Ööö&¦V7F—fUöF%öW'&÷"“ó°¢ÒVÇ6R°¢ÆWB×WBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢ÆWB7W'&VçE÷&Wf—6–öâÒæWGv÷&°¢æö&¦V7F—fW0¢ævWB‚fö&¦V7F—fRæ–B¢æÖ‡Æ7W'&VçGÂ7W'&VçBç&Wf—6–öâ¢æöµö÷"…7FGW46öFS£¤äõEôdõTäB“ó°¢–b7W'&VçE÷&Wf—6–öâÒW‡V7FVE÷&Wf—6–öâ°¢&WGW&âW'"…7FGW46öFS£¤4ôädÄ”5B“°¢Ð¢æWGv÷&²æö&¦V7F—fW2æ–ç6W'B†ö&¦V7F—fRæ–BÂö&¦V7F—fRæ6ÆöæR‚’“°¢&WGW&âö²‚‚’“°¢Ð¢7FFP¢ææWGv÷&°¢æÆö6²‚¢æW‡V7B‚'7FFRö—6öæVB"¢æö&¦V7F—fW0¢æ–ç6W'B†ö&¦V7F—fRæ–BÂö&¦V7F—fRæ6ÆöæR‚’“°¢ö²‚‚’§Ð ¦7–æ2fâÆöEöö&¦V7F—fUö6æöæ–6ÅöWf–FVæ6R€¢7FFS¢e6†&VE7FFRÀ¢ö&¦V7F—fW3¢e´ö&¦V7F—fUÒÀ¢’Óâ&W7VÇCÄö&¦V7F—fT6æöæ–6ÄWf–FVæ6RÂ7FGW46öFSâ°¢ÆWB6öÖR‡7F÷&R’Òg7FFRç7F÷&RVÇ6R°¢&WGW&âö²„ö&¦V7F—fT6æöæ–6ÄWf–FVæ6S£¦FVfVÇB‚’“°¢Ó°¢ÆWB×WBæWGv÷&·2Ò%G&VU6WC£¦æWr‚“°¢f÷"ö&¦V7F—fR–âö&¦V7F—fW2°¢ÆWB6öÖR†'VæFÆR’Òö&¦V7F—fRæ66WFVE÷fÇVUö'VæFÆRæ5÷&Vb‚’VÇ6R°¢6öçF–çVS°¢Ó°¢–bÆWB6öÖR‡–ÖVçB’Òf'VæFÆRæÖöæWF'•÷–ÖVçB°¢æWGv÷&·2æ–ç6W'B‡–ÖVçBæ&÷VçG’ææWGv÷&²æ6ÆöæR‚’“°¢Ð¢f÷"æVVB–âf'VæFÆRæ6öçG&–'WF–öåöæVVG2°¢–bÆWBFöÖ–ã£¤6öçG&–'WF–öä6ö×Vç6F–öã£¥–B²–ÖVçBÒÒfæVVBæ6ö×Vç6F–öâ°¢æWGv÷&·2æ–ç6W'B‡–ÖVçBæ&÷VçG’ææWGv÷&²æ6ÆöæR‚’“°¢Ð¢Ð¢Ð¢–bæWGv÷&·2æ—5öV×G’‚’°¢&WGW&âö²„ö&¦V7F—fT6æöæ–6ÄWf–FVæ6S£¦FVfVÇB‚’“°¢Ð¢ÆWBFW&×2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2‚¢æv—@¢æÖöW'"†Ööö&¦V7F—fUöF%öW'&÷"“ó°¢ÆWB×WBWf–FVæ6RÒö&¦V7F—fT6æöæ–6ÄWf–FVæ6S£¦FVfVÇB‚“°¢f÷"æWGv÷&²–âæWGv÷&·2°¢ÆWBWfVçG2Ò7F÷&P¢æÆ—7EöWFöæöÖ÷W5ö&÷VçG•öWfVçG2‚fæWGv÷&²¢æv—@¢æÖöW'"†Ööö&¦V7F—fUöF%öW'&÷"“ó°¢ÆWB×WBfVVBÒ'V–ÆEöWFöæöÖ÷W5ö&÷VçG•öfVVB†WfVçG2ÂFW&×2æ6ÆöæR‚’ÂfÇ6R¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"“ó°¢7FFRç&V6÷fW'•÷&W6W'fF–öç2æÇ’‚f×WBfVVBÂfÇ6R“°¢7FFP¢ç&V6÷fW'•÷&W6W'fF–öç0¢æW†6ÇVFUög&öÕ÷&W÷'FVEö÷WF6öÖW2‚f×WBfVVB“°¢ÆWB×WBæWGv÷&µöWf–FVæ6RÒ'V–ÆEöö&¦V7F—fUö6æöæ–6ÅöWf–FVæ6R‚fæWGv÷&²ÂffVVB“°¢Wf–FVæ6RægVæF–æræVæB‚f×WBæWGv÷&µöWf–FVæ6RægVæF–ær“°¢Wf–FVæ6P¢ç6WGFÆVÖVçG0¢æVæB‚f×WBæWGv÷&µöWf–FVæ6Rç6WGFÆVÖVçG2“°¢Ð¢ö²†Wf–FVæ6R§Ð ¦7–æ2fâW'6—7Eö&÷VçG•öæEöÆVFvW"€¢7FFS¢e6†&VE7FFRÀ¢&÷VçG“¢fFöÖ–ã£¤&÷VçG’À¢ÆVFvW%öVçG&–W3¢e¶ÆVFvW#£¤ÆVFvW$VçG'•ÒÀ¢’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢6W'f–6U÷'VçF–ÖS£§W'6—7Eö&÷VçG•öæEöÆVFvW"€¢7FFRç7F÷&Ræ5÷&Vb‚’À¢g7FFRææWGv÷&²À¢&÷VçG’À¢ÆVFvW%öVçG&–W2À¢¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"§Ð ¦7–æ2fâW'6—7EöÆVFvW%öVçG&–W2€¢7F÷&S¢e÷7Fw&W57F÷&RÀ¢VçG&–W3¢e¶ÆVFvW#£¤ÆVFvW$VçG'•ÒÀ¢’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢6W'f–6U÷'VçF–ÖS£§W'6—7EöÆVFvW%öVçG&–W2‡7F÷&RÂVçG&–W2¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"§Ð ¦7–æ2fâW'6—7EöÆÅ÷&—6µöWfVçG2‡7FFS¢e6†&VE7FFR’Óâ&W7VÇCÂ‚’Â7FGW46öFSâ°¢6W'f–6U÷'VçF–ÖS£§W'6—7EöÆÅ÷&—6µöWfVçG2‡7FFRç7F÷&Ræ5÷&Vb‚’Âg7FFRææWGv÷&²¢æv—@¢æÖöW'"‡Å÷Â7FGW46öFS£¤”åDU$äÅõ4U%dU%ôU%$õ"§Ð ¢5¶ÆÆ÷r†FVEö6öFR•Ð¦fâW‡V7FVEöF–vW7Eöf÷%ö&öG’†&öG“¢g7G"’Óâ7G&–ær°¢†6…ö'F–f7B†&öG’§Ð ¢5¶6fr‡FW7B•Ð¦ÖöBFW7G2°¢W6R7WW#£¢£°¢W6RÆÆ÷“£§°¢&–Ö—F—fW3£¤##SbÀ¢6–væW'3£§¶Æö6Ã£¥&—fFT¶W•6–væW"Â6–væW%7–æ7ÒÀ¢Ó°¢W6R£§°¢FDgVæF–æt6öçG&–'WF–öå&WVW7BÂ6Æ–Ô&÷VçG•&WVW7BÂ7&VFTgVæF–æt–çFVçE&WVW7BÀ¢÷VåööÆVD&÷VçG•&WVW7BÂ÷7D&÷VçG•&WVW7BÂ&Vv—7FW$vVçE&WVW7BÀ¢&Vv—7FW$6&–Æ—G•&WVW7BÂ7V&Ö—E&W7VÇE&WVW7BÂfW&–g•7V&Ö—76–öå&WVW7BÀ¢Ó°¢W6R6‡&öæó£¥F–ÖU¦öæS°¢W6RF#£§°¢ÆFf÷&Ô6Æ–Ô6ö†÷'E7FG2ÂÆFf÷&ÔF–Ç•7FG2ÂÆFf÷&Ô–FVçF—G•7FG2À¢ÆFf÷&ÔÖWG&–746÷fW&vU7FG2ÂÆFf÷&Õ–÷WE7FG2À¢Ó°¢W6RFöÖ–ã£§°¢ffV7FVE'G”FV6Æ&F–öâÂ&÷VçG’Â&÷VçG•7FGW2Â6&–Æ—G”6Æ72ÂFVÆ—fW&&ÆT66W75öÆ–7’À¢W‡V7FVDVffV7BÂgVæF–æt–çFVçE7FGW2ÂgVæF–ætÖöFRÂ–FVçF—G”F—66Æ÷7W&RÂö&¦V7F—fTWF†÷&—G’À¢ö&¦V7F—fTWF†÷&—G”¶–æBÂö&¦V7F—fU'F–6—çBÂö&¦V7F—fU&—f7”FV6Æ&F–öâÂö&¦V7F—fU7FGW2À¢ö&¦V7F—fUfW&–f–6F–öäÖV6†æ—6ÒÂö&¦V7F—fUfW&–f–6F–öåöÆ–7’Â'F–6—çD¶–æBÀ¢–ÖVçDWfVçE7FGW2Â–ÖVçE&–ÂÂ–÷WE7FGW2Â&ööe&V6÷&BÂV&Æ–4Wf–FVæ6UöÆ–7’À¢&–v‡G5öÆ–7’ÂfW&–f–W$¶–æBÀ¢Ó°¢W6Rv—F‡V%ö£¤v—D‡V$6†V6´6öæ6ÇW6–öã°¢W6R†Ö3£§´†Ö2ÂÖ7Ó°¢W6R6†#£¥6†#Sc°¢W6R7FC£§°¢–ó£§µ&VBÂw&—FWÒÀ¢æWC£¥F7Æ—7FVæW"À¢7G#£¤g&öÕ7G"À¢F‡&VBÀ¢Ó°¢W6RF÷vW#£¥6W'f–6TW‡C° ¢G—RFW7D†Ö56†#SbÒ†Ö3Å6†#Scã° ¢5·FW7EÐ¢fâF—7G&–'WF–öå÷&W÷'F–æu÷&WV—&W5öWfW'•÷vÆÆWEöW†6ÇW6–öåö6Æ72‚’°¢ÆWBFVfVÇG2Ò'6UöF—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W2„æöæR’çVçw&‚“°¢76W'EöW†FVfVÇG2æÆVâ‚’ÂF#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U2æÆVâ‚’“°¢76W'B‡'6UöF—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W2…6öÖR€¢&Ö–çF–æW"Æ÷W&F÷"ÇFW7BÇ7–çF†WF–5ö6æ'’Ç7öç6÷&VBÆ6—&7VÆ%ögVæF–ærÆ÷W&F÷%ögVæFVEöFWfVÆ÷ÖVçB ¢’¢çVçw&öW'"‚¢çFõ÷7G&–ær‚¢æ6öçF–ç2‚&WfW'’&WV—&VB6Æ72"’“°¢Ð ¢fâ÷Våö6ö×WF—F–öåö†V'F&VB€¢æ÷s¢FFUF–ÖSÅWF3âÀ¢7FGW3¢g7G"À¢7W'6÷#¢÷F–öãÇScCâÀ¢vU÷6V6öæG3¢“cBÀ¢’Óâ&6T–æFW†W$†V'F&VB°¢&6T–æFW†W$†V'F&VB°¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢W67&÷uö6öçG&7C¢#ƒ–S“3ƒ&&V#†#CV#s3vCCƒF#VVfv#ƒss–CF6R"çFõ÷7G&–ær‚’À¢7FGW3¢7FGW2çFõ÷7G&–ær‚’À¢7F'FVEöC¢æ÷rÒ6‡&öæôGW&F–öã£§6V6öæG2†vU÷6V6öæG2²’À¢6ö×ÆWFVEöC¢6öÖR†æ÷rÒ6‡&öæôGW&F–öã£§6V6öæG2†vU÷6V6öæG2’’À¢ÆFW7Eö&Æö6³¢7W'6÷"æÖ‡Æ&Æö6·Â&Æö6²ç6GW&F–æuöFBƒ"’’À¢6öæf—&ÖVE÷Fõö&Æö6³¢7W'6÷"À¢g&öÕö&Æö6³¢7W'6÷"À¢Fõö&Æö6³¢7W'6÷"À¢fWF6†VEöÆöw3¢À¢W'6—7FVEö7W'6÷%ö&Æö6³¢7W'6÷"À¢6¶—VE÷&V6öã¢æöæRÀ¢W'&÷%öÖW76vS¢æöæRÀ¢WFFVEöC¢æ÷rÒ6‡&öæôGW&F–öã£§6V6öæG2†vU÷6V6öæG2’À¢Ð¢Ð ¢5·FW7EÐ¢fâ÷Våö6ö×WF—F–öåöÖöæ—F÷&–æu÷&WV—&W5ög&W6…ö†VÇF‡•ö7W'6÷%öWf–FVæ6R‚’°¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂ‚ÂrÂ"ÂÂ’çVçw&‚“°¢ÆWB6fUö&Æö6²ÒSóó°¢ÆWB†VÇF‡’Ò÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'7V66W72"Â6öÖR‡6fUö&Æö6²’ÂR“°¢76W'B†÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö—5ög&W6‚€¢f†VÇF‡’Â6fUö&Æö6²Âæ÷p¢’“° ¢ÆWB7FÆRÒ÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'7V66W72"Â6öÖR‡6fUö&Æö6²’Â““°¢76W'B‚÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö—5ög&W6‚€¢g7FÆRÂ6fUö&Æö6²Âæ÷p¢’“° ¢ÆWBÆvv–ærÒ÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'7V66W72"Â6öÖR‡6fUö&Æö6²Ò#’ÂR“°¢76W'B‚÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö—5ög&W6‚€¢fÆvv–ærÂ6fUö&Æö6²Âæ÷p¢’“° ¢ÆWBf–ÆVBÒ÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ&f–ÆVB"Â6öÖR‡6fUö&Æö6²’ÂR“°¢76W'B‚÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö—5ög&W6‚€¢ff–ÆVBÂ6fUö&Æö6²Âæ÷p¢’“°¢Ð ¢5·FW7EÐ¢fâ÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö66WG5ööæÇ•÷F†Uö6Vv‡E÷W÷6¶—÷&V6öâ‚’°¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂ‚ÂrÂ"ÂÂ’çVçw&‚“°¢ÆWB6fUö&Æö6²ÒSóó°¢ÆWB×WB6Vv‡E÷WÒ÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'6¶—VB"Â6öÖR‡6fUö&Æö6²’ÂR“°¢6Vv‡E÷Wç6¶—VE÷&V6öâÒ6öÖR‚&æò6öæf—&ÖVB&Æö6·2&R&VG’Fò66â"çFõ÷7G&–ær‚’“°¢76W'B†÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö—5ög&W6‚€¢f6Vv‡E÷WÂ6fUö&Æö6²Âæ÷p¢’“° ¢6Vv‡E÷Wç6¶—VE÷&V6öâÐ¢6öÖR‚&ÆFW7B&Æö6²—2&VÆ÷r6öæf–wW&VB6öæf—&ÖF–öç2"çFõ÷7G&–ær‚’“°¢76W'B‚÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö—5ög&W6‚€¢f6Vv‡E÷WÂ6fUö&Æö6²Âæ÷p¢’“° ¢6Vv‡E÷Wç6¶—VE÷&V6öâÒ6öÖR‚&æò6öæf—&ÖVB&Æö6·2&R&VG’Fò66â"çFõ÷7G&–ær‚’“°¢6Vv‡E÷WæW'&÷%öÖW76vRÒ6öÖR‚'&VF7FVBf–ÇW&R"çFõ÷7G&–ær‚’“°¢76W'B‚÷Våö6ö×WF—F–öåöÖöæ—F÷&–æuö—5ög&W6‚€¢f6Vv‡E÷WÂ6fUö&Æö6²Âæ÷p¢’“°¢Ð ¢5·FW7EÐ¢fâV&Æ–5öÖWG&–75öÖ&·5ö–æFW†W%ö†V'F&VG5öFVÆ–VEögFW%öf—fUöÖ–çWFW2‚’°¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂ‚ÂrÂ"ÂÂ’çVçw&‚“°¢ÆWBg&W6‚Ò÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'7V66W72"Â6öÖRƒSóó’Â3“°¢76W'B‡V&Æ–5öÖWG&–75ö–æFW†W%ö†V'F&VEög&W6‚‚fg&W6‚Âæ÷r’“° ¢ÆWB7FÆRÒ÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'7V66W72"Â6öÖRƒSóó’Â3“°¢76W'B‚V&Æ–5öÖWG&–75ö–æFW†W%ö†V'F&VEög&W6‚‚g7FÆRÂæ÷r’“° ¢ÆWB×WBÆvv–ærÒ÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'7V66W72"Â6öÖRƒSóó’Â“°¢Ævv–æræÆFW7Eö&Æö6²Ò6öÖRƒSóó#“°¢76W'B‚V&Æ–5öÖWG&–75ö–æFW†W%ö†V'F&VEög&W6‚‚fÆvv–ærÂæ÷r’“° ¢ÆWB×WBf–ÆVBÒ÷Våö6ö×WF—F–öåö†V'F&VB†æ÷rÂ'7V66W72"Â6öÖRƒSóó’Â“°¢f–ÆVBæW'&÷%öÖW76vRÒ6öÖR‚'&VF7FVB"çFõ÷7G&–ær‚’“°¢76W'B‚V&Æ–5öÖWG&–75ö–æFW†W%ö†V'F&VEög&W6‚‚ff–ÆVBÂæ÷r’“°¢Ð ¢fâVçG&çE÷&VÆ•öf—‡GW&R†7F–öã¢S‚’Óâ÷Vä6ö×WF—F–öäVçG&çE&VÆ’°¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂ‚ÂrÂ"ÂÂ’çVçw&‚“°¢÷Vä6ö×WF—F–öäVçG&çE&VÆ’°¢–C¢WV–C£¦æWu÷cB‚’À¢–FV×÷FVæ7•ö¶W“¢f÷&ÖB‚&VçG&çB×&V6V—B×¶7F–öçÒ"’À¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢vÆÆWC¢#ƒ"çFõ÷7G&–ær‚’À¢&÷VçG•ö6öçG&7C¢#ƒ#######################################""çFõ÷7G&–ær‚’À¢FVÆVvFS¢#ƒ3333333333333333333333333333333333333332"çFõ÷7G&–ær‚’À¢7F–öâÀ¢vÆÆWEöæöæ6S¢rÀ¢FVFÆ–æS¢ósƒeó…ósÀ¢–ÆöEö†6ƒ¢f÷&ÖB‚#‡·Ò"Â&"ç&WVBƒ3"’’À¢&WVW7Eöf–ævW'&–çC¢&f—‡GW&R"çFõ÷7G&–ær‚’À¢&VÆ–W%öFG&W73¢#ƒCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCB"çFõ÷7G&–ær‚’À¢7FGW3¢÷Vä6ö×WF—F–öäVçG&çE&VÆ•7FGW3£¤'&öF67BÀ¢&WG'–&ÆS¢G'VRÀ¢GFV×Eö6÷VçC¢À¢G…ö†6ƒ¢6öÖR†f÷&ÖB‚#‡·Ò"Â#SR"ç&WVBƒ3"’’’À¢W7F–ÖFVEöv3¢6öÖRƒ“ó’À¢v5öÆ–Ö—C¢6öÖRƒ#ó’À¢W'&÷%ö6öFS¢æöæRÀ¢W'&÷%öÖW76vS¢æöæRÀ¢&V6V—Eö&Æö6³¢æöæRÀ¢&V6V—Eö&Æö6µö†6ƒ¢æöæRÀ¢6æöæ–6Å÷6fUö&Æö6³¢æöæRÀ¢6æöæ–6Å÷6fUö&Æö6µö†6ƒ¢æöæRÀ¢6æöæ–6ÅöWfVçC¢æöæRÀ¢–ÖVçE÷&÷fVã¢fÇ6RÀ¢7&VFVEöC¢æ÷rÀ¢WFFVEöC¢æ÷rÀ¢Ð¢Ð ¢fâFG&W75÷F÷–2†FG&W73¢g7G"’Óâ7G&–ær°¢f÷&ÖB‚#‡·×·Ò"Â#"ç&WVBƒ"’ÂfFG&W75³"âåÒ¢Ð ¢fâVçG&çE÷&VÆ•÷&V6V—B€¢&VÆ“¢d÷Vä6ö×WF—F–öäVçG&çE&VÆ’À¢&÷VçG•÷6–væGW&S¢g7G"À¢&÷VçG•÷F÷–75ögFW%÷6–væGW&S¢fV3Å7G&–æsâÀ¢’Óâ'5G&ç67F–öå&V6V—B°¢ÆWBG&ç67F–öåö†6‚Ò&VÆ’çG…ö†6‚æ6ÆöæR‚’çVçw&‚“°¢ÆWB7F–öåöÆörÒ6†–åö&6S£¥'4WfÔÆör°¢FG&W73¢&VÆ’çvÆÆWBæ6ÆöæR‚’À¢F÷–73¢fV2°¢WfVçE÷F÷–2‚$VçG&çD7F–öäW†V7WFVB‡V–çC‚ÆFG&W72ÆFG&W72ÇV–çC#SbÆ'—FW33"’"’À¢f÷&ÖB‚#‡³£cG‡Ò"Â&VÆ’æ7F–öâ’À¢FG&W75÷F÷–2‚g&VÆ’æFVÆVvFR’À¢FG&W75÷F÷–2‚g&VÆ’ç&VÆ–W%öFG&W72’À¢ÒÀ¢FF¢f÷&ÖB‚#‡³£cG‡×·Ò"Â&VÆ’çvÆÆWEöæöæ6RÂg&VÆ’ç–ÆöEö†6…³"âåÒ’À¢G&ç67F–öåö†6ƒ¢G&ç67F–öåö†6‚æ6ÆöæR‚’À¢&Æö6µöçVÖ&W#¢#ƒcB"çFõ÷7G&–ær‚’À¢Æöuö–æFWƒ¢#ƒ"çFõ÷7G&–ær‚’À¢Ó°¢ÆWB×WBF÷–72ÒfV2¶WfVçE÷F÷–2†&÷VçG•÷6–væGW&R•Ó°¢F÷–72æW‡FVæB†&÷VçG•÷F÷–75ögFW%÷6–væGW&R“°¢ÆWB&÷VçG•öÆörÒ6†–åö&6S£¥'4WfÔÆör°¢FG&W73¢&VÆ’æ&÷VçG•ö6öçG&7Bæ6ÆöæR‚’À¢F÷–72À¢FF¢#‚"çFõ÷7G&–ær‚’À¢G&ç67F–öåö†6ƒ¢G&ç67F–öåö†6‚æ6ÆöæR‚’À¢&Æö6µöçVÖ&W#¢#ƒcB"çFõ÷7G&–ær‚’À¢Æöuö–æFWƒ¢#ƒ"çFõ÷7G&–ær‚’À¢Ó°¢'5G&ç67F–öå&V6V—B°¢G&ç67F–öåö†6‚À¢&Æö6µöçVÖ&W#¢6öÖR‚#ƒcB"çFõ÷7G&–ær‚’’À¢&Æö6µö†6ƒ¢6öÖR†f÷&ÖB‚#‡·Ò"Â#cb"ç&WVBƒ3"’’’À¢7FGW3¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢Æöw3¢fV2¶7F–öåöÆörÂ&÷VçG•öÆöuÒÀ¢Ð¢Ð ¢5·FW7EÐ¢fâVçG&çE÷&VÆ•÷&V6V—G5÷&WV—&UöW†7Eö7F–öåöæE÷6öÇfW%÷F÷–72‚’°¢ÆWB&÷VçG•ö–BÒf÷&ÖB‚#‡·Ò"Â#sr"ç&WVBƒ3"’“°¢ÆWB6WVVæ6RÒf÷&ÖB‚#‡³£cG‡Ò"Â“°¢ÆWBVçG'•öçVÖ&W"Òf÷&ÖB‚#‡³£cG‡Ò"Â“°¢ÆWB66W2Ò°¢€¢À¢%6öÇWF–öä6öÖÖ—GFVB†'—FW33"ÆFG&W72ÇV–çC‚Æ'—FW33"ÇV–çCcBÇV–çCcBÇV–çC#Sb’"À¢fV2°¢&÷VçG•ö–Bæ6ÆöæR‚’À¢FG&W75÷F÷–2‚#ƒ"’À¢VçG'•öçVÖ&W"À¢ÒÀ¢%6öÇWF–öä6öÖÖ—GFVB"À¢fÇ6RÀ¢’À¢€¢À¢$&÷VçG•6WGFÆVB†'—FW33"ÇV–çCcBÆFG&W72ÇV–çC#SbÇV–çC#SbÇV–çC#SbÇV–çC#SbÆ'—FW33"Æ'—FW33"Æ'—FW33"Æ'—FW33"’"À¢fV2°¢&÷VçG•ö–Bæ6ÆöæR‚’À¢6WVVæ6Ræ6ÆöæR‚’À¢FG&W75÷F÷–2‚#ƒ"’À¢ÒÀ¢$&÷VçG•6WGFÆVB"À¢G'VRÀ¢’À¢€¢À¢$6ö×WF—F–öå7V&Ö—76–öå&V¦V7FVB†'—FW33"ÇV–çCcBÆFG&W72ÇV–çC#SbÆ'—FW33"’"À¢fV2°¢&÷VçG•ö–Bæ6ÆöæR‚’À¢6WVVæ6RÀ¢FG&W75÷F÷–2‚#ƒ"’À¢ÒÀ¢$6ö×WF—F–öå7V&Ö—76–öå&V¦V7FVB"À¢fÇ6RÀ¢’À¢€¢"À¢$VçG'”&öæEv—F†G&vâ†'—FW33"ÆFG&W72ÇV–çC#Sb’"À¢fV2°¢&÷VçG•ö–Bæ6ÆöæR‚’À¢FG&W75÷F÷–2‚#ƒ"’À¢ÒÀ¢$VçG'”&öæEv—F†G&vâ"À¢fÇ6RÀ¢’À¢Ó°¢f÷"†7F–öâÂ6–væGW&RÂF÷–72ÂW‡V7FVEöWfVçBÂW‡V7FVE÷–ÖVçB’–â66W2°¢ÆWB&VÆ’ÒVçG&çE÷&VÆ•öf—‡GW&R†7F–öâ“°¢ÆWB&V6V—BÒVçG&çE÷&VÆ•÷&V6V—B‚g&VÆ’Â6–væGW&RÂF÷–72“°¢76W'EöW€¢fÆ–FFUö÷Våö6ö×WF—F–öåöVçG&çE÷&VÆ•÷&V6V—B‚g&VÆ’Âg&V6V—B’çVçw&‚’À¢†W‡V7FVEöWfVçBÂW‡V7FVE÷–ÖVçB¢“°¢Ð ¢ÆWB&VÆ’ÒVçG&çE÷&VÆ•öf—‡GW&Rƒ“°¢ÆWBÖ—7Æ6VE÷vÆÆWBÒVçG&çE÷&VÆ•÷&V6V—B€¢g&VÆ’À¢%6öÇWF–öä6öÖÖ—GFVB†'—FW33"ÆFG&W72ÇV–çC‚Æ'—FW33"ÇV–çCcBÇV–çCcBÇV–çC#Sb’"À¢fV2°¢FG&W75÷F÷–2‚g&VÆ’çvÆÆWB’À¢FG&W75÷F÷–2‚#ƒ“““““““““““““““““““““““““““““““““““““““’"’À¢f÷&ÖB‚#‡³£cG‡Ò"Â’À¢ÒÀ¢“°¢76W'EöW€¢fÆ–FFUö÷Våö6ö×WF—F–öåöVçG&çE÷&VÆ•÷&V6V—B‚g&VÆ’ÂfÖ—7Æ6VE÷vÆÆWB’À¢W'"…7FGW46öFS£¤$EôtDUt’¢“° ¢ÆWB×WB†–v…öæöæ6RÒVçG&çE÷&VÆ•÷&V6V—B€¢g&VÆ’À¢%6öÇWF–öä6öÖÖ—GFVB†'—FW33"ÆFG&W72ÇV–çC‚Æ'—FW33"ÇV–çCcBÇV–çCcBÇV–çC#Sb’"À¢fV2°¢&÷VçG•ö–BÀ¢FG&W75÷F÷–2‚g&VÆ’çvÆÆWB’À¢f÷&ÖB‚#‡³£cG‡Ò"Â’À¢ÒÀ¢“°¢†–v…öæöæ6RæÆöw5³ÒæFFÒf÷&ÖB€¢#ƒ·×³£g‡×·Ò"À¢#"ç&WVBƒ#2’À¢&VÆ’çvÆÆWEöæöæ6RÀ¢g&VÆ’ç–ÆöEö†6…³"âåÐ¢“°¢76W'EöW€¢fÆ–FFUö÷Våö6ö×WF—F–öåöVçG&çE÷&VÆ•÷&V6V—B‚g&VÆ’Âf†–v…öæöæ6R’À¢W'"…7FGW46öFS£¤$EôtDUt’¢“°¢Ð ¢5·FW7EÐ¢fâ&VÆ–W%öv5ö6÷&V¦V7F–öå÷&WV—&W5ö6öæf–wW&F–öåö6†ævR‚’°¢76W'B‚ƒC%÷&VÆ•öW'&÷%ö—5÷&WG'–&ÆR€¢d6†–ä&6TW'&÷#£¥&VÆ–W$v4Æ–Ö—DW†6VVFVB°¢W7F–ÖFVC¢ScUó“CÀ¢Ö†–×VÓ¢3óÀ¢Ð¢’“°¢76W'B‡ƒC%÷&VÆ•öW'&÷%ö—5÷&WG'–&ÆR€¢d6†–ä&6TW'&÷#£¥&VÆ–W%&÷f–FW"‚'FV×÷&'’%2f–ÇW&R"çFõ÷7G&–ær‚’¢’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâö&¦V7F—fUö•÷&WV—&W5÷6–væVEö7&VF–öåöæE÷&W6W'fW5÷&öÆUö&÷VæF&–W2‚’°¢ÆWB6–væW#¢&—fFT¶W•6–væW"Ð¢#ƒS–3c““VS““†c“vVCC“cfc“CS3ƒ–F3–SƒfFSƒ†3vƒC&cCc6#f#sƒc“B ¢ç'6R‚¢çVçw&‚“°¢ÆWB'F–6—çEö–BÒWV–C£¦æWu÷cB‚“°¢ÆWBG&gBÒö&¦V7F—fT7&VF–öäG&gB°¢–C¢WV–C£¦æWu÷cB‚’À¢F—FÆS¢%V&Æ—6‚fW&–f–VBV&Æ–2&W÷'B"çFõ÷7G&–ær‚’À¢FW6—&VEö÷WF6öÖS¢$6÷W&6RÖÆ–æ¶VB&W÷'B76W2—G26öÖÖ—GFVB&Wf–Wrâ"çFõ÷7G&–ær‚’À¢‡VÖå÷W'÷6S¢$Væ&ÆRâ–æf÷&ÖVBFV6—6–öâ'’F†RæÖVB&VæVf–6–'’â"çFõ÷7G&–ær‚’À¢'F–6—çG3¢fV2´ö&¦V7F—fU'F–6—çB°¢–C¢'F–6—çEö–BÀ¢¶–æC¢'F–6—çD¶–æC£¤÷&væ—¦F–öâÀ¢F—7Æ•öæÖS¢%&WVW7F–ær÷&væ—¦F–öâ"çFõ÷7G&–ær‚’À¢vÆÆWC¢f÷&ÖB‚'³¢7‡Ò"Â6–væW"æFG&W72‚’’À¢–FVçF—G•öF—66Æ÷7W&S¢–FVçF—G”F—66Æ÷7W&S£¥6WVFöç–Ö÷W2À¢V&Æ–5ö–FVçF—G•÷&VfW&Væ6S¢æöæRÀ¢ÕÒÀ¢&WVW7F–æu÷'G•ö–C¢'F–6—çEö–BÀ¢&VæVf–6–'•ö–G3¢fV2·'F–6—çEö–EÒÀ¢ffV7FVE÷'F–W3¢fV2´ffV7FVE'G”FV6Æ&F–öâ°¢'F–6—çEö–BÀ¢W‡V7FVEöVffV7C¢W‡V7FVDVffV7C£¤Ö—†VBÀ¢FW67&—F–öã¢%&V6V—fW2F†R&W7VÇBæB&V'2F†RFV6—6–öâ&—6²â"çFõ÷7G&–ær‚’À¢ÕÒÀ¢WF†÷&—G“¢ö&¦V7F—fTWF†÷&—G’°¢¶–æC¢ö&¦V7F—fTWF†÷&—G”¶–æC£¤÷&væ—¦F–öåvÆÆWBÀ¢ÖVÖ&W%ö–G3¢fV2·'F–6—çEö–EÒÀ¢F‡&W6†öÆC¢À¢V&Æ–5÷7FFVÖVçC ¢$öæRFV6Æ&VB÷&væ—¦F–öâvÆÆWB6öçG&öÇ2&–æF–ærö&¦V7F—fRFV6—6–öç2â ¢çFõ÷7G&–ær‚’À¢ÒÀ¢f–Æ&ÆU÷&W6÷W&6W3¢fV3£¦æWr‚’À¢W‡V7FVEöf–æÅöFVÆ—fW&&ÆS¢%V&Æ–2&W÷'BæBWf–FVæ6R6¶vR"çFõ÷7G&–ær‚’À¢&WVW7FVEö66W75÷öÆ–7“¢FVÆ—fW&&ÆT66W75öÆ–7“£¥V&Æ–2À¢&WVW7FVE÷&–v‡G5÷öÆ–7“¢&–v‡G5öÆ–7’°¢÷væW%ö–G3¢fV2·'F–6—çEö–EÒÀ¢Æ–6Vç6Uö÷%÷FW&×3¢$42Ô%’ÓBã"çFõ÷7G&–ær‚’À¢&W7G&–7F–öç3¢fV3£¦æWr‚’À¢ÒÀ¢&WVW7FVEöf–æÅ÷fW&–f–6F–öã¢ö&¦V7F—fUfW&–f–6F–öåöÆ–7’°¢ÖV6†æ—6Ó¢ö&¦V7F—fUfW&–f–6F–öäÖV6†æ—6Ó£¤6öÖÖ—GFVEfW&–f–W"°¢fW&–f–W%ö–C¢'F–6—çEö–BÀ¢ÒÀ¢66WFæ6Uö7&—FW&–¢fV2²$WfW'’6Æ–ÒÆ–æ·2Fò–ç7V7F&ÆRWf–FVæ6Râ"çFõ÷7G&–ær‚•ÒÀ¢Wf–FVæ6U÷66†VÖ¢&‡GG3¢òöW†×ÆRçFW7B÷&W÷'BÖWf–FVæ6Rç66†VÖæ§6öâ"çFõ÷7G&–ær‚’À¢Wf–FVæ6U÷66†VÖö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒcB’’À¢G'W7Eö77V×F–öç3¢fV2°¢%F†RæÖVBfW&–f–W"vÆÆWBföÆÆ÷w2F†RV&Æ–27&—FW&–â"çFõ÷7G&–ær‚¢ÒÀ¢ÒÀ¢&—f7“¢ö&¦V7F—fU&—f7”FV6Æ&F–öâ°¢&Æö6¶6†–åö–æf÷&ÖF–öåö—5÷V&Æ–3¢G'VRÀ¢Wf–FVæ6U÷öÆ–7“¢V&Æ–4Wf–FVæ6UöÆ–7“£¥V&Æ–2À¢&VF7F–öåöÆ–Ö—G3¢$æò&—fFRFF—266WFVB'’F†—2V&Æ–2ö&¦V7F—fRâ ¢çFõ÷7G&–ær‚’À¢ÒÀ¢Ó°¢ÆWBÆâÒÆåöö&¦V7F—fUö7&VF–öâ„§6öâ†G&gB’’æv—BçVçw&‚’ã°¢ÆWB6öÖÖ—FÖVçBÒ##Sc£¦g&öÕ÷7G"‚gÆâæ6öÖÖ—FÖVçEö†6‚’çVçw&‚“°¢ÆWB6–væGW&RÒ6–væW"ç6–våöÖW76vU÷7–æ2†6öÖÖ—FÖVçBæ5÷6Æ–6R‚’’çVçw&‚“°¢ÆWB6–væVBÒ6–væVDö&¦V7F—fT7&VF–öâ°¢ÆâÀ¢&÷fÇ3¢fV2¶FöÖ–ã£¥vÆÆWD&÷fÂ°¢'F–6—çEö–BÀ¢6–væGW&S¢6–væGW&RçFõ÷7G&–ær‚’À¢ÕÒÀ¢Ó°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB7&VFVBÒ7&VFUöö&¦V7F—fR…7FFR‡7FFRæ6ÆöæR‚’’Â§6öâ‡6–væVBæ6ÆöæR‚’’¢æv—@¢çVçw&‚¢ã°¢76W'EöW†7&VFVBæö&¦V7F—fRç7FGW2Âö&¦V7F—fU7FGW3£¤÷Väf÷%&÷÷6Ç2“°¢76W'EöW†7&VFVBæö&¦V7F—fRç&WVW7F–æu÷'G•ö–BÂ'F–6—çEö–B“°¢76W'EöW†7&VFVBæö&¦V7F—fRæWF†÷&—G’æÖVÖ&W%ö–G2ÂfV2·'F–6—çEö–EÒ“°¢76W'B‚7&VFVBç&VF–æW72ç&VG’“°¢76W'B†7&VFV@¢ç&VF–æW70¢æ&Æö6¶W'0¢æ—FW"‚¢æç’‡Æ&Æö6¶W'Â&Æö6¶W"æ6öçF–ç2‚'&÷f–FW"&÷÷6Â"’’“° ¢ÆWBÆ—7FVBÒÆ—7Eöö&¦V7F—fW2…7FFR‡7FFRæ6ÆöæR‚’’’æv—BçVçw&‚’ã°¢76W'EöW†Æ—7FVBæÆVâ‚’Â“°¢76W'EöW†Æ—7FVE³Òæö&¦V7F—fRæ–BÂ7&VFVBæö&¦V7F—fRæ–B“°¢76W'EöW€¢7&VFUöö&¦V7F—fR…7FFR‡7FFR’Â§6öâ‡6–væVB’¢æv—@¢çVçw&öW'"‚’À¢7FGW46öFS£¤4ôädÄ”5@¢“°¢Ð ¢5·FW7EÐ¢fâ†VÇF…ö–FVçF–f–W5÷&÷Fö6öÅöæEöFWÆ÷–VE÷&Wf—6–öâ‚’°¢ÆWB&W7öç6RÒ†VÇF…÷&W7öç6R‚##3CScsƒ–&6FVc#3CScsƒ–&6FVc#3CScr"’æ–çFõ÷&W7öç6R‚“° ¢76W'EöW€¢&W7öç6Ræ†VFW'2‚•²'‚ÖvVçBÖ&÷VçF–W2×&Wf—6–öâ%ÒÀ¢##3CScsƒ–&6FVc#3CScsƒ–&6FVc#3CScr ¢“°¢76W'EöW€¢&W7öç6Ræ†VFW'2‚•²'‚ÖvVçBÖ&÷VçF–W2×&÷Fö6öÂ%ÒÀ¢&vVçBÖ&÷VçF–W2öWFöæöÖ÷W2×c ¢“°¢Ð ¢fâFW7Eö6†FwEö7F–öåö–çFVçB†7F–öã¢g7G"’ÓâF$6†FwD7F–öä–çFVçB°¢F$6†FwD7F–öä–çFVçB°¢–C¢WV–C£¦æWu÷cB‚’À¢–FV×÷FVæ7•ö¶W“¢f÷&ÖB‚&6†FwB×FW7B×·Ò"ÂWV–C£¦æWu÷cB‚’’À¢7F–öã¢7F–öâçFõ÷7G&–ær‚’À¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢÷÷'GVæ—G•ö–C¢6öÖR€¢&6æöæ–6Åö&6S¦&6RÖÖ–ææWC£ƒ ¢çFõ÷7G&–ær‚’À¢’À¢&÷VçG•ö6öçG&7C¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢&÷VçG•ö–C¢6öÖR†f÷&ÖB‚#‡·Ò"Â##""ç&WVBƒ3"’’’À¢7F÷%÷vÆÆWC¢6öÖR‚#ƒ3333333333333333333333333333333333333332"çFõ÷7G&–ær‚’’À¢Ö÷VçEö&6U÷Væ—G3¢†7F–öâÓÒ&gVæB"’çF†Vå÷6öÖRƒóó’À¢FWF–Ç3¢6W&FUö§6öã£¦§6öâ‡·Ò’À¢&WVW7Eöf–ævW'&–çC¢#CB"ç&WVBƒ3"’À¢7FGW3¢'VæF–æuö6öæf—&ÖF–öâ"çFõ÷7G&–ær‚’À¢G&ç67F–öåö†6ƒ¢6öÖR†f÷&ÖB‚#‡·Ò"Â#SR"ç&WVBƒ3"’’’À¢6æöæ–6ÅöWfVçEö–C¢æöæRÀ¢6æöæ–6ÅöWfVçEö¶–æC¢æöæRÀ¢6öæf—&ÖVEö&Æö6³¢æöæRÀ¢W‡—&W5öC¢WF3£¦æ÷r‚’²6‡&öæôGW&F–öã£¦†÷W'2ƒ’À¢7&VFVEöC¢WF3£¦æ÷r‚’Ò6‡&öæôGW&F–öã£§6V6öæG2ƒ’À¢WFFVEöC¢WF3£¦æ÷r‚’À¢Ð¢Ð ¢5·FW7EÐ¢fâ6†FwEö7F–öå÷&WVW7E÷&WV—&W5ö7F–öå÷7V6–f–5öf–VÆG2‚’°¢ÆWBfÆ–EögVæBÒæ÷&ÖÆ—¦Uö6†FwEö7F–öå÷&WVW7B„7&VFT6†FwD7F–öä–çFVçE&WVW7B°¢–FV×÷FVæ7•ö¶W“¢&6†FwBÖgVæBÓ#2"çFõ÷7G&–ær‚’À¢7F–öã¢6†FwD7F–öä¶–æC£¤gVæBÀ¢æWGv÷&³¢6öÖR‚&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’’À¢÷÷'GVæ—G•ö–C¢6öÖR‚&6æöæ–6Åö&6S¦&6RÖÖ–ææWC£†&2"çFõ÷7G&–ær‚’’À¢&÷VçG•ö6öçG&7C¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢&÷VçG•ö–C¢æöæRÀ¢7F÷%÷vÆÆWC¢æöæRÀ¢Ö÷VçEö&6U÷Væ—G3¢6öÖRƒóó’À¢FWF–Ç3¢6W&FUö§6öã£¦§6öâ‡²'F—FÆR#¢$gVæBFW7B'Ò’À¢Ò¢çVçw&‚“°¢76W'EöW‡fÆ–EögVæBææWGv÷&²æ5öFW&Vb‚’Â6öÖR‚&&6RÖÖ–ææWB"’“°¢76W'EöW€¢fÆ–EögVæBæ&÷VçG•ö6öçG&7Bæ5öFW&Vb‚’À¢6öÖR‚#ƒ"¢“° ¢ÆWBÖ—76–æuöÖ÷VçBÒæ÷&ÖÆ—¦Uö6†FwEö7F–öå÷&WVW7B„7&VFT6†FwD7F–öä–çFVçE&WVW7B°¢Ö÷VçEö&6U÷Væ—G3¢æöæRÀ¢âçfÆ–EögVæBæ6ÆöæR‚¢Ò“°¢76W'EöW†Ö—76–æuöÖ÷VçBçVçw&öW'"‚’Â7FGW46öFS£¤$Eõ$UTU5B“° ¢ÆWBÖ÷VçEööåö6ö×ÆWF–öâÐ¢æ÷&ÖÆ—¦Uö6†FwEö7F–öå÷&WVW7B„7&VFT6†FwD7F–öä–çFVçE&WVW7B°¢7F–öã¢6†FwD7F–öä¶–æC£¤6ö×ÆWFRÀ¢Ö÷VçEö&6U÷Væ—G3¢6öÖRƒ’À¢âçfÆ–EögVæ@¢Ò“°¢76W'EöW†Ö÷VçEööåö6ö×ÆWF–öâçVçw&öW'"‚’Â7FGW46öFS£¤$Eõ$UTU5B“°¢Ð ¢5·FW7EÐ¢fâ6†FwEö7F–öåöFWF–Ç5ö&Uö&÷VæFVEöæE÷&V¦V7E÷6Vç6—F—fUöf–VÆG2‚’°¢fÆ–FFUö6†FwEö7F–öåöFWF–Ç2€¢6†FwD7F–öä¶–æC£¥÷7BÀ¢g6W&FUö§6öã£¦§6öâ‡°¢&G&gB#¢°¢'F—FÆR#¢%V&Æ—6‚V&Æ–2FW7B"À¢&vöÂ#¢%&öGV6R&÷VæFVB'F–f7Bâ"À¢&66WFæ6Uö7&—FW&–#¢²%F†RV&Æ–26†V6²76W2â%ÒÀ¢'6öÇfW%÷&Wv&E÷W6F2#¢#"ã"À¢'fW&–f–W%÷&Wv&E÷W6F2#¢#ã"À¢'6÷W&6U÷W&Â#¢çVÆÂÀ¢&7&÷vFgVæB#¢G'VRÀ¢&F—66÷fW'•÷6÷W&6R#¢$6†DuB ¢Ð¢Ò’À¢¢çVçw&‚“°¢fÆ–FFUö6†FwEö7F–öåöFWF–Ç2€¢6†FwD7F–öä¶–æC£¤6ö×ÆWFRÀ¢g6W&FUö§6öã£¦§6öâ‡°¢&'F–f7E÷&VfW&Væ6R#¢&‡GG3¢òöv—F‡V"æ6öÒöW†×ÆR÷&Wòö6öÖÖ—Bö&2"À¢&Wf–FVæ6R#¢°¢'G&ç67F–öåö†6‚#¢f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’’À¢'6÷W&6U÷6æ6†÷EöF–vW7B#¢'6†#Sc§V&Æ–2 ¢Ð¢Ò’À¢¢çVçw&‚“° ¢f÷"6Vç6—F—fR–â°¢'&—fFUö¶W’"À¢'6VVB×‡&6R"À¢'vÆÆWB6–væGW&R"À¢'–ÖVçEöWF†÷&—¦F–öâ"À¢&6&EöçVÖ&W""À¢&66W75÷Fö¶Vâ"À¢Ò°¢ÆWBFWF–Ç2Ò6W&FUö§6öã£¦§6öâ‡°¢&'F–f7E÷&VfW&Væ6R#¢&‡GG3¢òöW†×ÆRæ6öÒö'F–f7B"À¢&Wf–FVæ6R#¢²‡6Vç6—F—fR“¢&×W7BÖæ÷BÖ&R×7F÷&VB'Ð¢Ò“°¢76W'EöW€¢fÆ–FFUö6†FwEö7F–öåöFWF–Ç2„6†FwD7F–öä¶–æC£¤6ö×ÆWFRÂfFWF–Ç2’çVçw&öW'"‚’À¢7FGW46öFS£¤$Eõ$UTU5BÀ¢'·6Vç6—F—fWÒ×W7B&R&V¦V7FVB ¢“°¢Ð ¢76W'EöW€¢fÆ–FFUö6†FwEö7F–öåöFWF–Ç2€¢6†FwD7F–öä¶–æC£¤gVæBÀ¢g6W&FUö§6öã£¦§6öâ‡²&'F–f7E÷&VfW&Væ6R#¢'w&öær7F–öâf–VÆB'Ò¢¢çVçw&öW'"‚’À¢7FGW46öFS£¤$Eõ$UTU5@¢“°¢76W'EöW€¢fÆ–FFUö6†FwEö7F–öåöFWF–Ç2€¢6†FwD7F–öä¶–æC£¤6ö×ÆWFRÀ¢g6W&FUö§6öã£¦§6öâ‡°¢&'F–f7E÷&VfW&Væ6R#¢&‡GG3¢òöW†×ÆRæ6öÒö'F–f7B"À¢&Wf–FVæ6R#¢²&—FV×2#¢fV2²'‚#²S×Ð¢Ò¢¢çVçw&öW'"‚’À¢7FGW46öFS£¤$Eõ$UTU5@¢“°¢Ð ¢5·FW7EÐ¢fâ6†FwEö7F–öåöFWF–Ç5÷&V¦V7EöW†6W76—fUöæW7F–ær‚’°¢ÆWB×WBæW7FVBÒ6W&FUö§6öã£¦§6öâ‚&ÆVb"“°¢f÷"ò–ââã‚°¢æW7FVBÒ6W&FUö§6öã£¦§6öâ‡²&ÆWfVÂ#¢æW7FVGÒ“°¢Ð¢ÆWBFWF–Ç2Ò6W&FUö§6öã£¦§6öâ‡°¢&'F–f7E÷&VfW&Væ6R#¢&‡GG3¢òöW†×ÆRæ6öÒö'F–f7B"À¢&Wf–FVæ6R#¢æW7FV@¢Ò“°¢76W'EöW€¢fÆ–FFUö6†FwEö7F–öåöFWF–Ç2„6†FwD7F–öä¶–æC£¤6ö×ÆWFRÂfFWF–Ç2’çVçw&öW'"‚’À¢7FGW46öFS£¤$Eõ$UTU5@¢“°¢Ð ¢5·FW7EÐ¢fâ6†FwEö7F–öåö6öæf—&ÖF–öå÷&WV—&W5öW†7E÷G&ç67F–öåö7F÷%öÖ÷VçEöæE÷F–ÖR‚’°¢ÆWB–çFVçBÒFW7Eö6†FwEö7F–öåö–çFVçB‚&gVæB"“°¢ÆWBÖF6†–ærÒWFöæöÖ÷W4&÷VçG”WfVçB°¢–C¢WV–C£¦æWu÷cB‚’À¢Æöuö¶W“¢&&6RÖÖ–ææWC££"çFõ÷7G&–ær‚’À¢G…ö†6ƒ¢–çFVçBçG&ç67F–öåö†6‚æ6ÆöæR‚’çVçw&‚’À¢&Æö6µöçVÖ&W#¢À¢Æöuö–æFWƒ¢À¢6öçG&7EöFG&W73¢–çFVçBæ&÷VçG•ö6öçG&7Bæ6ÆöæR‚’çVçw&‚’À¢&÷VçG•ö–C¢–çFVçBæ&÷VçG•ö–Bæ6ÆöæR‚’çVçw&‚’À¢¶–æC¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤gVæF–ætFFVBÀ¢FF¢6W&FUö§6öã£¦§6öâ‡°¢&6öçG&–'WF÷"#¢–çFVçBæ7F÷%÷vÆÆWBæ6ÆöæR‚’çVçw&‚’À¢&Ö÷VçB#¢óó ¢Ò’À¢ö67W'&VEöC¢WF3£¦æ÷r‚’À¢Ó°¢76W'B†6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB‚f–çFVçBÂfÖF6†–ær’“° ¢ÆWB×WBw&öæu÷G&ç67F–öâÒÖF6†–æræ6ÆöæR‚“°¢w&öæu÷G&ç67F–öâçG…ö†6‚Òf÷&ÖB‚#‡·Ò"Â#cb"ç&WVBƒ3"’“°¢76W'B‚6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB€¢f–çFVçBÀ¢gw&öæu÷G&ç67F–öà¢’“° ¢ÆWB×WBw&öæuö7F÷"ÒÖF6†–æræ6ÆöæR‚“°¢w&öæuö7F÷"æFF²&6öçG&–'WF÷"%ÒÐ¢6W&FUö§6öã£¦§6öâ‚#ƒsssssssssssssssssssssssssssssssssssssssr"“°¢76W'B‚6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB€¢f–çFVçBÀ¢gw&öæuö7F÷ ¢’“° ¢ÆWB×WBw&öæuöÖ÷VçBÒÖF6†–æræ6ÆöæR‚“°¢w&öæuöÖ÷VçBæFF²&Ö÷VçB%ÒÒ6W&FUö§6öã£¦§6öâƒ““•ó““’“°¢76W'B‚6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB€¢f–çFVçBÀ¢gw&öæuöÖ÷Vç@¢’“° ¢ÆWB×WB†—7F÷&–6Å÷&WÆ’ÒÖF6†–æs°¢†—7F÷&–6Å÷&WÆ’æö67W'&VEöBÒ–çFVçBæ7&VFVEöBÒ6‡&öæôGW&F–öã£§6V6öæG2ƒ“°¢76W'B‚6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB€¢f–çFVçBÀ¢f†—7F÷&–6Å÷&WÆ¢’“°¢Ð ¢5·FW7EÐ¢fâfW&–g•ö7F–öåö66WG5ööæÇ•ö6æöæ–6Å÷6WGFÆVÖVçEö÷%÷&V¦V7F–öâ‚’°¢ÆWB×WB–çFVçBÒFW7Eö6†FwEö7F–öåö–çFVçB‚'fW&–g’"“°¢–çFVçBæÖ÷VçEö&6U÷Væ—G2ÒæöæS°¢ÆWBWfVçBÒÆ¶–æGÂWFöæöÖ÷W4&÷VçG”WfVçB°¢–C¢WV–C£¦æWu÷cB‚’À¢Æöuö¶W“¢f÷&ÖB‚&&6RÖÖ–ææWC§·Ó£"ÂWV–C£¦æWu÷cB‚’’À¢G…ö†6ƒ¢–çFVçBçG&ç67F–öåö†6‚æ6ÆöæR‚’çVçw&‚’À¢&Æö6µöçVÖ&W#¢"À¢Æöuö–æFWƒ¢À¢6öçG&7EöFG&W73¢–çFVçBæ&÷VçG•ö6öçG&7Bæ6ÆöæR‚’çVçw&‚’À¢&÷VçG•ö–C¢–çFVçBæ&÷VçG•ö–Bæ6ÆöæR‚’çVçw&‚’À¢¶–æBÀ¢FF¢6W&FUö§6öã£¦§6öâ‡·Ò’À¢ö67W'&VEöC¢WF3£¦æ÷r‚’À¢Ó°¢76W'B†6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB€¢f–çFVçBÀ¢fWfVçB„WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG•6WGFÆVB¢’“°¢76W'B†6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB€¢f–çFVçBÀ¢fWfVçB„WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¥7V&Ö—76–öå&V¦V7FVB¢’“°¢76W'B‚6æöæ–6ÅöWfVçEöÖF6†W5ö6†FwEö–çFVçB€¢f–çFVçBÀ¢fWfVçB„WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¥7V&Ö—76–öäFFVB¢’“°¢Ð ¢5·FW7EÐ¢fâ6Æ÷VEövVçEöW'&÷'5ö&UöÖ6†–æU÷&VF&ÆUöæE÷&÷f–FW%÷6fR‚’°¢ÆWB‡7FGW2Â§6öâ†W'&÷"’’Ò6Æ÷VEövVçEö•öW'&÷"„6Æ÷VDvVçDW'&÷#£¤–çfÆ–E&W7öç6R€¢&ö&¦V7F—fRF6²FWVæFVæ6–W26öçF–â7–6ÆR"çFõ÷7G&–ær‚’À¢’“°¢76W'EöW‡7FGW2Â7FGW46öFS£¤$EôtDUt’“°¢76W'EöW†W'&÷"æW'&÷%ö6öFRÂ&6Æ÷VEövVçEö–çfÆ–EöÖöFVÅö÷WGWB"“°¢76W'B†W'&÷"ç&WG'–&ÆR“°¢76W'B†W'&÷"æÖW76vRæ6öçF–ç2‚&7–6ÆR"’“° ¢ÆWB‡7FGW2Â§6öâ†W'&÷"’’Ò6Æ÷VEövVçEö•öW'&÷"„6Æ÷VDvVçDW'&÷#£¥&÷f–FW"€¢'&÷f–FW"&WGW&æVB…EECv—F‚&—fFRF–væ÷7F–72"çFõ÷7G&–ær‚’À¢’“°¢76W'EöW‡7FGW2Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢76W'EöW†W'&÷"æW'&÷%ö6öFRÂ&6Æ÷VEövVçE÷&÷f–FW%÷Væf–Æ&ÆR"“°¢76W'B†W'&÷"ç&WG'–&ÆR“°¢76W'B‚W'&÷"æÖW76vRæ6öçF–ç2‚'&—fFRF–væ÷7F–72"’“°¢Ð ¢5·FW7EÐ¢fâ6—FUöæÇ—F–75ö66WG5ööæÇ•öf—'7E÷'G•ö÷&–v–ç5öæEöÖ–æ–Ö—¦VEöf–VÆG2‚’°¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B€¢†VFW#£¤õ$”t”âÀ¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚&‡GG3¢òövVçF&÷VçF–W2æ"’À¢“°¢76W'B‡6—FUöæÇ—F–75ö÷&–v–åöÆÆ÷vVB‚f†VFW'2’“°¢†VFW'2æ–ç6W'B€¢†VFW#£¤õ$”t”âÀ¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚&‡GG3¢òö&÷VçG–&ö&BævÆö&Â"’À¢“°¢76W'B‡6—FUöæÇ—F–75ö÷&–v–åöÆÆ÷vVB‚f†VFW'2’“°¢†VFW'2æ–ç6W'B€¢†VFW#£¤õ$”t”âÀ¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚&‡GG3¢òövVçF&÷VçF–W2ææWf–ÂæW†×ÆR"’À¢“°¢76W'B‚6—FUöæÇ—F–75ö÷&–v–åöÆÆ÷vVB‚f†VFW'2’“° ¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂrÂ’Â‚ÂÂ’çVçw&‚“°¢ÆWBWfVçBÒfÆ–FFVE÷6—FUöæÇ—F–75öWfVçB€¢6—FTæÇ—F–74WfVçE&WVW7B°¢WfVçEö–C¢WV–C£¦æWu÷cB‚’À¢f—6—F÷%ö–C¢WV–C£¦æWu÷cB‚’À¢6W76–öåö–C¢WV–C£¦æWu÷cB‚’À¢WfVçEöæÖS¢&6ö×WF—F–öåöVçG'•ö6öæf—&ÖVB"çFõ÷7G&–ær‚’À¢vU÷Fƒ¢"ö6ö×WF—F–öâæ‡FÖÂ"çFõ÷7G&–ær‚’À¢6÷W&6S¢6öÖR‚$v—D‡V""çFõ÷7G&–ær‚’’À¢6×–vã¢6öÖR‚&ÆVæ6‚Ó##b"çFõ÷7G&–ær‚’’À¢&VfW'&W%ö†÷7C¢6öÖR‚$v—D‡V"æ6öÒ"çFõ÷7G&–ær‚’’À¢÷÷'GVæ—G•ö–C¢6öÖR‚&V—SS£ƒCS3¦vVçBÖ&÷VçF–W2ö÷VâÖ6ö×WF—F–öâ×c£ƒ"çFõ÷7G&–ær‚’’À¢&÷VçG•ö6öçG&7C¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢ö67W'&VEöC¢æ÷rÀ¢ÒÀ¢æ÷rÀ¢¢çVçw&‚“°¢76W'EöW†WfVçBç6÷W&6Ræ5öFW&Vb‚’Â6öÖR‚&v—F‡V""’“°¢76W'EöW†WfVçBç&VfW'&W%ö†÷7Bæ5öFW&Vb‚’Â6öÖR‚&v—F‡V"æ6öÒ"’“°¢76W'EöW†WfVçBçvU÷F‚Â"ö6ö×WF—F–öâæ‡FÖÂ"“°¢Ð ¢5·FW7EÐ¢fâ–çFW&f6UöGG&–'WF–öåö66WG5ööæÇ•öW‡Æ–6—Eö•ö÷%ö6Æ•÷fÇVW2‚’°¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢76W'EöW†GG&–'WFVEö•ö–çFW&f6R‚f†VFW'2’ÂæöæR“° ¢†VFW'2æ–ç6W'B€¢”åDU$d4UôEE$”%UD”ôåô„TDU"À¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚&’"’À¢“°¢76W'EöW€¢GG&–'WFVEö•ö–çFW&f6R‚f†VFW'2’À¢6öÖR„ö'6W'fVD–çFW&f6S£¤’¢“° ¢†VFW'2æ–ç6W'B€¢”åDU$d4UôEE$”%UD”ôåô„TDU"À¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚$4Ä’"’À¢“°¢76W'EöW€¢GG&–'WFVEö•ö–çFW&f6R‚f†VFW'2’À¢6öÖR„ö'6W'fVD–çFW&f6S£¤6Æ’¢“° ¢†VFW'2æ–ç6W'B€¢”åDU$d4UôEE$”%UD”ôåô„TDU"À¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚'vV'6—FR"’À¢“°¢76W'EöW†GG&–'WFVEö•ö–çFW&f6R‚f†VFW'2’ÂæöæR“°¢Ð ¢5·FW7EÐ¢fâ–çFW&f6UöæÇ—F–75öW†6ÇW6–öå÷&WV—&W5ö÷fW&–f–VE÷66÷VEö÷%ö÷W&F÷%÷Fö¶Vâ‚’°¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B€¢äÅ•D”55ôU„4ÅU4”ôåô„TDU"À¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚'w&öær×Fö¶Vâ"’À¢“°¢76W'B‚æÇ—F–75öW†6ÇW6–öåö—5öWF†÷&—¦VB€¢6öÖR‚&æÇ—F–72×6V7&WB"’À¢6öÖR‚&÷W&F÷"×6V7&WB"’À¢f†VFW'2À¢’“° ¢†VFW'2æ–ç6W'B€¢äÅ•D”55ôU„4ÅU4”ôåô„TDU"À¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚&æÇ—F–72×6V7&WB"’À¢“°¢76W'B†æÇ—F–75öW†6ÇW6–öåö—5öWF†÷&—¦VB€¢6öÖR‚&æÇ—F–72×6V7&WB"’À¢6öÖR‚&÷W&F÷"×6V7&WB"’À¢f†VFW'2À¢’“° ¢†VFW'2ç&VÖ÷fR„äÅ•D”55ôU„4ÅU4”ôåô„TDU"“°¢†VFW'2æ–ç6W'B€¢†VFW#£¤UD„õ$•¤D”ôâÀ¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚$&V&W"÷W&F÷"×6V7&WB"’À¢“°¢76W'B†æÇ—F–75öW†6ÇW6–öåö—5öWF†÷&—¦VB€¢6öÖR‚&æÇ—F–72×6V7&WB"’À¢6öÖR‚&÷W&F÷"×6V7&WB"’À¢f†VFW'2À¢’“°¢76W'B‚æÇ—F–75öW†6ÇW6–öåö—5öWF†÷&—¦VB„æöæRÂæöæRÂf†VFW'2Â’“° ¢ÆWB×WB&W7öç6RÒ7FGW46öFS£¤ô²æ–çFõ÷&W7öç6R‚“°¢&W7öç6P¢æ†VFW'5ö×WB‚¢æ–ç6W'B„äÅ•D”55ôU„4ÅTDTEô„TDU"Â†VFW%fÇVS£¦g&öÕ÷7FF–2‚'G'VR"’“°¢76W'EöW€¢&W7öç6P¢æ†VFW'2‚¢ævWB„äÅ•D”55ôU„4ÅTDTEô„TDU"¢ææE÷F†Vâ‡ÇfÇVWÂfÇVRçFõ÷7G"‚’æö²‚’’À¢6öÖR‚'G'VR"¢“°¢Ð ¢5·FW7EÐ¢fâÖ&¶WF–æuöFöÖ–ç5÷&VF—&V7Eööæ6UöæE÷&W6W'fUöFVW÷F‡5öæE÷VW&–W2‚’°¢76W'EöW€¢Ö&¶WF–æuöFöÖ–åöFW7F–æF–öâ‚&vVçF&÷VçF–W2çv÷&²"Âb"ò"ç'6R‚’çVçw&‚’’À¢6öÖR‚&‡GG3¢òövVçF&÷VçF–W2æ÷F6·2ò"çFõ÷7G&–ær‚’¢“°¢76W'EöW€¢Ö&¶WF–æuöFöÖ–åöFW7F–æF–öâ€¢%uurätTåD$õTåD”U2äDUc£CC2"À¢b"÷6F²÷'W7C÷fW'6–öãÓ"ç'6R‚’çVçw&‚¢’À¢6öÖR‚&‡GG3¢òövVçF&÷VçF–W2æ÷6F²÷'W7C÷fW'6–öãÓ"çFõ÷7G&–ær‚’¢“°¢76W'EöW€¢Ö&¶WF–æuöFöÖ–åöFW7F–æF–öâ€¢&&÷VçG–&ö&BævÆö&Â"À¢b"öV&âæ‡FÖÃög&öÓÖÆVv7’"ç'6R‚’çVçw&‚¢’À¢6öÖR‚&‡GG3¢òövVçF&÷VçF–W2æò"çFõ÷7G&–ær‚’¢“°¢76W'EöW€¢Ö&¶WF–æuöFöÖ–åöFW7F–æF–öâ‚&’ævVçF&÷VçF–W2æ"Âb"ö†VÇF‚"ç'6R‚’çVçw&‚’’À¢æöæP¢“°¢76W'EöW€¢Ö&¶WF–æuöFöÖ–åöFW7F–æF–öâ‚'7FGW2ævVçF&÷VçF–W2æ"Âb"ò"ç'6R‚’çVçw&‚’’À¢6öÖR‚&‡GG3¢òö’ævVçF&÷VçF–W2æö†VÇF‚"çFõ÷7G&–ær‚’¢“°¢Ð ¢5·FW7EÐ¢fâ6—FUöæÇ—F–75÷&V¦V7G5÷VW'•÷7G&–æw5÷Væ¶æ÷våöWfVçG5öæE÷7FÆU÷F–ÖW7F×2‚’°¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂrÂ’Â‚ÂÂ’çVçw&‚“°¢ÆWB&WVW7BÒÆWfVçEöæÖS¢g7G"ÂvU÷Fƒ¢g7G"Âö67W'&VEöGÂ6—FTæÇ—F–74WfVçE&WVW7B°¢WfVçEö–C¢WV–C£¦æWu÷cB‚’À¢f—6—F÷%ö–C¢WV–C£¦æWu÷cB‚’À¢6W76–öåö–C¢WV–C£¦æWu÷cB‚’À¢WfVçEöæÖS¢WfVçEöæÖRçFõ÷7G&–ær‚’À¢vU÷Fƒ¢vU÷F‚çFõ÷7G&–ær‚’À¢6÷W&6S¢æöæRÀ¢6×–vã¢æöæRÀ¢&VfW'&W%ö†÷7C¢æöæRÀ¢÷÷'GVæ—G•ö–C¢æöæRÀ¢&÷VçG•ö6öçG&7C¢æöæRÀ¢ö67W'&VEöBÀ¢Ó°¢76W'B€¢fÆ–FFVE÷6—FUöæÇ—F–75öWfVçB‡&WVW7B‚'vU÷f–Wr"Â"ó÷6V7&WCÓ"Âæ÷r’Âæ÷r’æ—5öW'"‚¢“°¢76W'B‡fÆ–FFVE÷6—FUöæÇ—F–75öWfVçB‡&WVW7B‚&&&—G&'’"Â"ò"Âæ÷r’Âæ÷r’æ—5öW'"‚’“°¢76W'B‡fÆ–FFVE÷6—FUöæÇ—F–75öWfVçB€¢&WVW7B‚'vU÷f–Wr"Â"ò"Âæ÷rÒ6‡&öæôGW&F–öã£¦F—2ƒ‚’’À¢æ÷rÀ¢¢æ—5öW'"‚’“°¢Ð ¢5·FW7EÐ¢fâ6—FUöæÇ—F–75ö66WG5÷F†U÷&—f7•öÖ–æ–Ö—¦VEööæ&ö&F–æuögVææVÂ‚’°¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂ‚Â#BÂ‚ÂÂ’çVçw&‚“°¢f÷"WfVçEöæÖR–â°¢&WF…ö6ö×ÆWFVB"À¢'vÆÆWEöÆ–æµ÷7F'FVB"À¢'vÆÆWEöÆ–æµö6öæf—&ÖVB"À¢'vÆÆWEöÖ—76–æuöFWFV7FVB"À¢'vÆÆWEö6öææV7FVB"À¢'vÆÆWE÷VægVæFVEöFWFV7FVB"À¢'vÆÆWEögVæFVEöö'6W'fVB"À¢&6æöæ–6Å÷÷7Eö†æFöfe÷f–WvVB"À¢&öç&×÷f–WvVB"À¢&öç&×öÖööç•÷7F'FVB"À¢&öç&×öÖWFÖ6µ÷7F'FVB"À¢&öç&×ö6ö–æ&6U÷7F'FVB"À¢&öç&×÷&WGW&æVB"À¢Ò°¢ÆWBWfVçBÒfÆ–FFVE÷6—FUöæÇ—F–75öWfVçB€¢6—FTæÇ—F–74WfVçE&WVW7B°¢WfVçEö–C¢WV–C£¦æWu÷cB‚’À¢f—6—F÷%ö–C¢WV–C£¦æWu÷cB‚’À¢6W76–öåö–C¢WV–C£¦æWu÷cB‚’À¢WfVçEöæÖS¢WfVçEöæÖRçFõ÷7G&–ær‚’À¢vU÷Fƒ¢"ööç&×æ‡FÖÂ"çFõ÷7G&–ær‚’À¢6÷W&6S¢æöæRÀ¢6×–vã¢æöæRÀ¢&VfW'&W%ö†÷7C¢æöæRÀ¢÷÷'GVæ—G•ö–C¢æöæRÀ¢&÷VçG•ö6öçG&7C¢æöæRÀ¢ö67W'&VEöC¢æ÷rÀ¢ÒÀ¢æ÷rÀ¢¢çVçw&‚“°¢76W'EöW†WfVçBæWfVçEöæÖRÂWfVçEöæÖR“°¢76W'EöW†WfVçBçvU÷F‚Â"ööç&×æ‡FÖÂ"“°¢Ð¢Ð ¢5·FW7EÐ¢fâ6—FUöæÇ—F–75öæ÷&ÖÆ—¦W5ö¶æ÷våöF—66÷fW'•÷6÷W&6W5÷v—F†÷WEö–æfW'&–æuöÖ7‚’°¢f÷"6÷W&6R–â°¢&6†FwBæ6öÒ"À¢&6†Bæ÷Væ’æ6öÒ"À¢&Æ–æ·2æ6†FwBæ6öÒ"À¢&÷Væ’"À¢Ò°¢76W'EöW€¢æ÷&ÖÆ—¦U÷6—FUöæÇ—F–75÷6÷W&6R…6öÖR‡6÷W&6RçFõ÷7G&–ær‚’’’çVçw&‚’À¢6öÖR‚&6†FwB"çFõ÷7G&–ær‚’¢“°¢Ð¢f÷"‡6÷W&6RÂW‡V7FVB’–â°¢‚'wwrævöövÆRæ6öÒ"Â&vöövÆR"’À¢‚&&–æræ6öÒ"Â&&–ær"’À¢‚&v—F‡V"æ6öÒ"Â&v—F‡V""’À¢‚&FWbçFò"Â'7–æF–6FVB"’À¢‚&æWw2ç–6öÖ&–æF÷"æ6öÒ"Â'7–æF–6FVB"’À¢Ò°¢76W'EöW€¢æ÷&ÖÆ—¦U÷6—FUöæÇ—F–75÷6÷W&6R…6öÖR‡6÷W&6RçFõ÷7G&–ær‚’’’çVçw&‚’À¢6öÖR†W‡V7FVBçFõ÷7G&–ær‚’¢“°¢Ð¢76W'EöW€¢æ÷&ÖÆ—¦U÷6—FUöæÇ—F–75÷6÷W&6R…6öÖR‚&Ö7"çFõ÷7G&–ær‚’’’çVçw&‚’À¢6öÖR‚&Ö7"çFõ÷7G&–ær‚’¢“°¢Ð ¢5·FW7EÐ¢fâF—66÷fW'•÷&÷WFUö6Æ76–f–6F–öåö—5öW‡Æ–6—EöæEöf–ÇW&Uö6Æ÷6VB‚’°¢ÆWBV×G’Ò†VFW$Ö£¦æWr‚“°¢76W'EöW€¢F—66÷fW'•÷&÷WFUöGG&–'WF–öâ‚"òçvVÆÂÖ¶æ÷vâövVçBÖ6&Bæ§6öâ"ÂfV×G’’À¢6öÖR‚€¢F—66÷fW'”–çFW&f6S£¤&À¢F—66÷fW'•&÷WFTfÖ–Ç“£¤vVçD6&BÀ¢GG&–'WF–öå&VÆ–&–Æ—G“£¤ö'6W'fVBÀ¢’¢“°¢76W'EöW€¢F—66÷fW'•÷&÷WFUöGG&–'WF–öâ‚"÷cö÷÷'GVæ—F–W2öfVVBæ§6öâ"ÂfV×G’’À¢6öÖR‚€¢F—66÷fW'”–çFW&f6S£¤fVVBÀ¢F—66÷fW'•&÷WFTfÖ–Ç“£¤÷÷'GVæ—G”Æ—7BÀ¢GG&–'WF–öå&VÆ–&–Æ—G“£¤ö'6W'fVBÀ¢’¢“°¢76W'EöW€¢F—66÷fW'•÷&÷WFUöGG&–'WF–öâ‚"÷cö÷÷'GVæ—F–W2"ÂfV×G’’À¢æöæP¢“° ¢ÆWB×WBFV6Æ&VBÒ†VFW$Ö£¦æWr‚“°¢FV6Æ&VBæ–ç6W'B€¢”åDU$d4UôEE$”%UD”ôåô„TDU"À¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚&6Æ’"’À¢“°¢76W'EöW€¢F—66÷fW'•÷&÷WFUöGG&–'WF–öâ‚"÷cö÷÷'GVæ—F–W2"ÂfFV6Æ&VB’À¢6öÖR‚€¢F—66÷fW'”–çFW&f6S£¤6Æ’À¢F—66÷fW'•&÷WFTfÖ–Ç“£¤÷÷'GVæ—G”Æ—7BÀ¢GG&–'WF–öå&VÆ–&–Æ—G“£¤FV6Æ&VBÀ¢’¢“°¢FV6Æ&VBæ–ç6W'B€¢”åDU$d4UôEE$”%UD”ôåô„TDU"À¢†VFW%fÇVS£¦g&öÕ÷7FF–2‚&Ö7"’À¢“°¢76W'EöW€¢F—66÷fW'•÷&÷WFUöGG&–'WF–öâ‚"÷cö÷÷'GVæ—F–W2"ÂfFV6Æ&VB’À¢æöæP¢“°¢76W'EöW€¢F—66÷fW'•÷&÷WFUöGG&–'WF–öâ‚"÷c÷Vç&VÆFVB"ÂfFV6Æ&VB’À¢æöæP¢“°¢Ð ¢5·FW7EÐ¢fâÆVvÅ÷öÆ–7•ö—5ö†6…ö&÷VæEöæEö66WFæ6Uö—5ö7F–öå÷vÆÆWEöæE÷F–ÖUö&÷VæB‚’°¢ÆWBöÆ–7’Ò'V–ÆEöÆVvÅ÷öÆ–7’‚&‡GG3¢òövVçF&÷VçF–W2æò"“°¢76W'EöW‡öÆ–7’çFW&×5÷fW'6–öâÂÄTtÅõDU$Õ5õdU%4”ôâ“°¢76W'EöW‡öÆ–7’ç7FFVÖVçEö†6‚ÂÆVvÅ÷7FFVÖVçEö†6‚‚’“°¢76W'EöW‡öÆ–7’çFW&×5÷W&ÂÂ&‡GG3¢òövVçF&÷VçF–W2æ÷FW&×2æ‡FÖÂ"“°¢76W'B‡öÆ–7¢ç7W÷'FVEö7F–öç0¢æ—FW"‚¢æç’‡Æ7F–öçÂ7F–öâÓÒ'÷7Eö&÷VçG’"’“°¢76W'B‡öÆ–7¢ç7W÷'FVEö7F–öç0¢æ—FW"‚¢æç’‡Æ7F–öçÂ7F–öâÓÒ&6æ6VÅö&÷VçG’"’“°¢76W'EöW€¢ÆVvÅ÷vV'6—FUö&6U÷W&Â„æöæRÂ&‡GG3¢òö’ævVçF&÷VçF–W2æò"’À¢&‡GG3¢òövVçF&÷VçF–W2æ ¢“°¢76W'EöW€¢ÆVvÅ÷vV'6—FUö&6U÷W&Â€¢6öÖR‚"‡GG3¢ò÷&Wf–WræW†×ÆR"çFõ÷7G&–ær‚’’À¢&‡GG3¢òö’ævVçF&÷VçF–W2æ ¢’À¢&‡GG3¢ò÷&Wf–WræW†×ÆR ¢“° ¢ÆWBæ÷rÒWF2çv—F…÷–ÖEöæEö†×2ƒ##bÂrÂ‚Â‚ÂÂ’çVçw&‚“°¢ÆWB×WB&WVW7BÒ&V6÷&DÆVvÄ66WFæ6U&WVW7B°¢FW&×5÷fW'6–öã¢ÄTtÅõDU$Õ5õdU%4”ôâçFõ÷7G&–ær‚’À¢&—f7•÷fW'6–öã¢ÄTtÅõ$•d5•õdU%4”ôâçFõ÷7G&–ær‚’À¢7F–öã¢'÷7Eö&÷VçG’"çFõ÷7G&–ær‚’À¢vÆÆWEöFG&W73¢#ƒ"çFõ÷7G&–ær‚’À¢7FFVÖVçEö†6ƒ¢ÆVvÅ÷7FFVÖVçEö†6‚‚’À¢66WFæ6UöÖWF†öC¢&•öW‡Æ–6—B"çFõ÷7G&–ær‚’À¢66WFVEöC¢æ÷rÀ¢Ó°¢76W'EöW€¢fÆ–FFUöÆVvÅö66WFæ6U÷&WVW7B‚g&WVW7BÂæ÷r’çVçw&‚’À¢&WVW7BçvÆÆWEöFG&W70¢“° ¢&WVW7Bæ7F–öâÒ'6WGFÆU÷v—F†÷WE÷fW&–f–6F–öâ"çFõ÷7G&–ær‚“°¢76W'EöW€¢fÆ–FFUöÆVvÅö66WFæ6U÷&WVW7B‚g&WVW7BÂæ÷r’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢&WVW7Bæ7F–öâÒ'÷7Eö&÷VçG’"çFõ÷7G&–ær‚“°¢&WVW7Bæ66WFVEöBÒæ÷rÒ6‡&öæôGW&F–öã£¦Ö–çWFW2ƒb“°¢76W'EöW€¢fÆ–FFUöÆVvÅö66WFæ6U÷&WVW7B‚g&WVW7BÂæ÷r’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢Ð ¢5·FW7EÐ¢fâF–ÖV÷WE÷&VÆ•ö66WG5ööæÇ•öW†7E÷W&Ö—76–öæÆW75÷G&ç6—F–öåö–çFVçB‚’°¢ÆWB&÷VçG’Ò#ƒ#°¢ÆWB&VÆ–W"Ò#ƒ#######################################"#°¢ÆWB×WB–çFVçBÒWfÕG&ç67F–öä–çFVçB°¢g&öÓ¢6öÖR‡&VÆ–W"çFõ÷7G&–ær‚’’À¢Fó¢&÷VçG’çFõ÷7G&–ær‚’À¢fÇVU÷vV“¢À¢FF¢WFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&U7V&Ö—76–öà¢æ6ÆÆFF‚¢çFõ÷7G&–ær‚’À¢gVæ7F–öã¢WFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&U7V&Ö—76–öà¢ægVæ7F–öâ‚¢çFõ÷7G&–ær‚’À¢Ó° ¢76W'B‡fÆ–FFUöWFöæöÖ÷W5÷F–ÖV÷WEö–çFVçB€¢f–çFVçBÀ¢&÷VçG’À¢&VÆ–W"À¢WFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&U7V&Ö—76–öâÀ¢¢æ—5öö²‚’“° ¢–çFVçBæFFÒWFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&T6Æ–Òæ6ÆÆFF‚’çFõ÷7G&–ær‚“°¢76W'EöW€¢fÆ–FFUöWFöæöÖ÷W5÷F–ÖV÷WEö–çFVçB€¢f–çFVçBÀ¢&÷VçG’À¢&VÆ–W"À¢WFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&U7V&Ö—76–öâÀ¢’À¢W'"…7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’¢“°¢–çFVçBæFFÒWFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&U7V&Ö—76–öà¢æ6ÆÆFF‚¢çFõ÷7G&–ær‚“°¢–çFVçBçfÇVU÷vV’Ò°¢76W'EöW€¢fÆ–FFUöWFöæöÖ÷W5÷F–ÖV÷WEö–çFVçB€¢f–çFVçBÀ¢&÷VçG’À¢&VÆ–W"À¢WFöæöÖ÷W5F–ÖV÷WD7F–öã£¤W‡—&U7V&Ö—76–öâÀ¢’À¢W'"…7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’¢“°¢Ð ¢5·FW7EÐ¢fâFöÖ–5÷7öç6÷'6†—ö66WG5ööæÇ•ööæUöW†7E÷fVÇEö6ÆÂ‚’°¢ÆWB7öç6÷"Ò#ƒ#######################################"#°¢ÆWB&VÆ–W"Ò#ƒ3333333333333333333333333333333333333332#°¢ÆWB×WB–çFVçBÒWfÕG&ç67F–öä–çFVçB°¢g&öÓ¢6öÖR‡&VÆ–W"çFõ÷7G&–ær‚’’À¢Fó¢7öç6÷"çFõ÷7G&–ær‚’À¢fÇVU÷vV“¢À¢FF¢f÷&ÖB‚#†&6FFVFG·Ò"Â#"ç&WVBƒc‚’’À¢gVæ7F–öã¢'7öç6÷$æD6Æ–Ò‚†FG&W72ÆFG&W72ÇV–çCcBÇV–çC#SbÆ'—FW33"Æ'—FW33"Æ'—FW33"ÇV–çC#SbÇV–çC#SbÆ'—FW33"ÇV–çC#Sb’Æ'—FW2ÇV–çC‚Æ'—FW33"Æ'—FW33"’"çFõ÷7G&–ær‚’À¢Ó° ¢76W'B‡fÆ–FFUöFöÖ–5÷7öç6÷&VEö6Æ–Õö–çFVçB‚f–çFVçBÂ7öç6÷"Â&VÆ–W"’æ—5öö²‚’“°¢–çFVçBçfÇVU÷vV’Ò°¢76W'B‡fÆ–FFUöFöÖ–5÷7öç6÷&VEö6Æ–Õö–çFVçB‚f–çFVçBÂ7öç6÷"Â&VÆ–W"’æ—5öW'"‚’“°¢–çFVçBçfÇVU÷vV’Ò°¢–çFVçBæFFç&WÆ6U÷&ævRƒ"âãÂ&FVF&VVb"“°¢76W'B‡fÆ–FFUöFöÖ–5÷7öç6÷&VEö6Æ–Õö–çFVçB‚f–çFVçBÂ7öç6÷"Â&VÆ–W"’æ—5öW'"‚’“°¢Ð ¢5·FW7EÐ¢fâFöÖ–5÷7öç6÷'6†—öæöæ6Uö—5÷7F&ÆUöæEö6æF–FFUö&÷VæB‚’°¢ÆWBæ÷rÒWF3£¦æ÷r‚“°¢ÆWB×WB7öç6÷'6†—Ò&öæE7öç6÷'6†—°¢–C¢WV–C£¦æWu÷cB‚’À¢6Æ–Õö6æF–FFUö–C¢WV–C£¦æWu÷cB‚’À¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢&÷VçG•ö6öçG&7C¢#ƒ"çFõ÷7G&–ær‚’À¢6öÇfW%÷vÆÆWC¢#ƒ#######################################""çFõ÷7G&–ær‚’À¢7öç6÷%÷vÆÆWC¢#ƒ3333333333333333333333333333333333333332"çFõ÷7G&–ær‚’À¢Ö÷VçC¢óÀ¢7FGW3¢&öæE7öç6÷'6†—7FGW3£¥&W6W'fVBÀ¢G&ç67F–öåö†6ƒ¢æöæRÀ¢6öæf—&ÖVEö&Æö6³¢æöæRÀ¢f–ÇW&Uö6öFS¢æöæRÀ¢f–ÇW&UöÖW76vS¢æöæRÀ¢7&VFVEöC¢æ÷rÀ¢WFFVEöC¢æ÷rÀ¢Ó°¢ÆWBf—'7BÒFöÖ–5÷7öç6÷'6†—öw&çEöæöæ6R‚g7öç6÷'6†—“°¢76W'EöW†f—'7BÂFöÖ–5÷7öç6÷'6†—öw&çEöæöæ6R‚g7öç6÷'6†—’“°¢76W'EöW†f—'7BæÆVâ‚’Âcb“°¢76W'B‡6†÷VÆE÷W6UöFöÖ–5÷7öç6÷'6†—†fÇ6RÂ6öÖR‚g7öç6÷'6†—’’“°¢76W'B‚6†÷VÆE÷W6UöFöÖ–5÷7öç6÷'6†—†fÇ6RÂæöæR’“°¢7öç6÷'6†—æ6Æ–Õö6æF–FFUö–BÒWV–C£¦æWu÷cB‚“°¢76W'EöæR†f—'7BÂFöÖ–5÷7öç6÷'6†—öw&çEöæöæ6R‚g7öç6÷'6†—’“°¢Ð ¢5·FW7EÐ¢fâ–æFW†VEö6Æ–Õ÷&V6÷fW'•÷&WV—&W5ö7W'&VçE÷&÷VæEöæEöW†7E÷66÷R‚’°¢ÆWBæ÷rÒWF3£¦æ÷r‚“°¢ÆWB&÷VçG•ö6öçG&7BÒ#ƒ#°¢ÆWB6öÇfW%÷vÆÆWBÒ#ƒ#######################################"#°¢ÆWBFW&×5ö†6‚Òf÷&ÖB‚#‡·Ò"Â#32"ç&WVBƒ3"’“°¢ÆWBöÆ–7•ö†6‚Òf÷&ÖB‚#‡·Ò"Â#CB"ç&WVBƒ3"’“°¢ÆWB6æF–FFRÒ6Æ–Ô6æF–FFR°¢–C¢WV–C£¦æWu÷cB‚’À¢–FV×÷FVæ7•ö¶W“¢'&V6÷fW"Ö–æFW†VBÖ6Æ–Ò"çFõ÷7G&–ær‚’À¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢&÷VçG•ö6öçG&7C¢&÷VçG•ö6öçG&7BçFõ÷7G&–ær‚’À¢6öÇfW%÷vÆÆWC¢6öÇfW%÷vÆÆWBçFõ÷7G&–ær‚’À¢vVçEö–C¢æöæRÀ¢VÆ–v–&–Æ—G•öWf–FVæ6S¢vVçDVÆ–v–&–Æ—G”Wf–FVæ6R°¢vVçEö–C¢æöæRÀ¢6öÇfW%÷vÆÆWC¢6öÇfW%÷vÆÆWBçFõ÷7G&–ær‚’À¢6&–Æ—F–W3¢fV3£¦æWr‚’À¢–Eö6ö×ÆWF–öç3¢À¢–E÷W6F5ö&6U÷Væ—G3¢À¢ÒÀ¢VÆ–v–&–Æ—G•öFV6—6–öã¢vVçDVÆ–v–&–Æ—G”FV6—6–öâ°¢VÆ–v–&ÆS¢G'VRÀ¢&V6öç3¢fV3£¦æWr‚’À¢ÒÀ¢7FGW3¢6Æ–Ô6æF–FFU7FGW3£¤WF†÷&—¦F–öå&VG’À¢W†6ÇW6—fU÷VçF–Ã¢6öÖR†æ÷r’À¢WF†÷&—¦F–öåöæöæ6S¢6öÖR†f÷&ÖB‚#‡·Ò"Â#SR"ç&WVBƒ3"’’’À¢WF†÷&—¦F–öå÷fÆ–Eö&Vf÷&S¢6öÖRƒóƒóó’À¢6Æ–Õ÷G&ç67F–öåö†6ƒ¢æöæRÀ¢6æöæ–6ÅöWfVçEö–C¢æöæRÀ¢f–ÇW&Uö6öFS¢æöæRÀ¢f–ÇW&UöÖW76vS¢æöæRÀ¢7&VFVEöC¢æ÷rÀ¢WFFVEöC¢æ÷rÀ¢Ó°¢ÆWBÖF6†–æuö6Æ–ÒÒWFöæöÖ÷W4&÷VçG”WfVçB°¢–C¢WV–C£¦æWu÷cB‚’À¢Æöuö¶W“¢&&6RÖÖ–ææWC¦6Æ–Ó£"çFõ÷7G&–ær‚’À¢G…ö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#cb"ç&WVBƒ3"’’À¢&Æö6µöçVÖ&W#¢À¢Æöuö–æFWƒ¢À¢6öçG&7EöFG&W73¢&÷VçG•ö6öçG&7BçFõ÷7G&–ær‚’À¢&÷VçG•ö–C¢f÷&ÖB‚#‡·Ò"Â#sr"ç&WVBƒ3"’’À¢¶–æC¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG”6Æ–ÖVBÀ¢FF¢6W&FUö§6öã£¦§6öâ‡°¢'&÷VæB#¢À¢'6öÇfW"#¢6öÇfW%÷vÆÆWBÀ¢'FW&×5ö†6‚#¢FW&×5ö†6‚À¢'öÆ–7•ö†6‚#¢öÆ–7•ö†6‚À¢&6Æ–Õö&öæB#¢óScBÀ¢&6Æ–ÕöW‡—&W5öB#¢óƒóóSc@¢Ò’À¢ö67W'&VEöC¢æ÷rÀ¢Ó°¢ÆWB×WB—FVÒÒWFöæöÖ÷W4&÷VçG”fVVD—FVÒ°¢&÷VçG•ö–C¢ÖF6†–æuö6Æ–Òæ&÷VçG•ö–Bæ6ÆöæR‚’À¢&÷VçG•ö6öçG&7C¢&÷VçG•ö6öçG&7BçFõ÷7G&–ær‚’À¢7&VF÷#¢#ƒ“““““““““““““““““““““““““““““““““““““““’"çFõ÷7G&–ær‚’À¢7FGW3¢&6Æ–Ö&ÆR"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&C¢##"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&C¢#"çFõ÷7G&–ær‚’À¢6Æ–Õö&öæC¢#"çFõ÷7G&–ær‚’À¢F–ÖV÷WEö&öæE÷ööÃ¢#"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçC¢##"çFõ÷7G&–ær‚’À¢gVæFVEöÖ÷VçC¢##"çFõ÷7G&–ær‚’À¢&WV—&VEöW‡FW&æÅ÷7VæC¢#"çFõ÷7G&–ær‚’À¢w&÷75ö66…öÖ&v–ã¢##"çFõ÷7G&–ær‚’À¢FW&×5ö†6ƒ¢FW&×5ö†6‚æ6ÆöæR‚’À¢FW&×3¢æöæRÀ¢FW&×5÷fÆ–C¢G'VRÀ¢fW&–f–6F–öåöÖöFS¢&FWFW&Ö–æ—7F–2"çFõ÷7G&–ær‚’À¢fW&–f–W%öÖöGVÆS¢æöæRÀ¢fW&–f–W%÷6WEö†6ƒ¢æöæRÀ¢fW&–f–W%÷F‡&W6†öÆC¢6öÖRƒ’À¢'VææW%ö–FVçF–f–W#¢6öÖR‚'FW7Eöf—‡GW&R"çFõ÷7G&–ær‚’’À¢fW&–f–6F–öå÷&VG“¢G'VRÀ¢fW&–f–6F–öå÷&VF–æW75÷&V6öã¢'&VG’"çFõ÷7G&–ær‚’À¢fÆ–FF–öåöW'&÷'3¢fV3£¦æWr‚’À¢WfVçG3¢fV2¶ÖF6†–æuö6Æ–Òæ6ÆöæR‚•ÒÀ¢Ó° ¢76W'B€¢7W'&VçEö–æFW†VEö6Æ–Õöf÷%ö6æF–FFR‚f—FVÒÂgöÆ–7•ö†6‚Âf6æF–FFRÂó’æ—5öæöæR‚¢“°¢—FVÒç7FGW2Ò&6Æ–ÖVB"çFõ÷7G&–ær‚“°¢76W'EöW€¢7W'&VçEö–æFW†VEö6Æ–Õöf÷%ö6æF–FFR‚f—FVÒÂgöÆ–7•ö†6‚Âf6æF–FFRÂó¢æÖ‡ÆWfVçGÂWfVçBæ–B’À¢6öÖR†ÖF6†–æuö6Æ–Òæ–B¢“°¢76W'B†7W'&VçEö–æFW†VEö6Æ–Õöf÷%ö6æF–FFR€¢f—FVÒÀ¢ff÷&ÖB‚#‡·Ò"Â#ƒ‚"ç&WVBƒ3"’’À¢f6æF–FFRÀ¢ó ¢¢æ—5öæöæR‚’“° ¢ÆWB×WBæWvW%ö÷F†W%÷6öÇfW"ÒÖF6†–æuö6Æ–Ó°¢æWvW%ö÷F†W%÷6öÇfW"æ–BÒWV–C£¦æWu÷cB‚“°¢æWvW%ö÷F†W%÷6öÇfW"æ&Æö6µöçVÖ&W"Ò°¢æWvW%ö÷F†W%÷6öÇfW"æFF²'&÷VæB%ÒÒ6W&FUö§6öã£¦§6öâƒ"“°¢æWvW%ö÷F†W%÷6öÇfW"æFF²'6öÇfW"%ÒÐ¢6W&FUö§6öã£¦§6öâ‚#†"“°¢—FVÒæWfVçG2çW6‚†æWvW%ö÷F†W%÷6öÇfW"“°¢76W'B€¢7W'&VçEö–æFW†VEö6Æ–Õöf÷%ö6æF–FFR‚f—FVÒÂgöÆ–7•ö†6‚Âf6æF–FFRÂó’æ—5öæöæR‚¢“°¢Ð ¢5·FW7EÐ¢fâW'6—7FVEö6Æ–Õö–FV×÷FVæ7•ö—5ö&÷VæE÷Fõö÷&–v–æÅ÷66÷R‚’°¢ÆWBæ÷rÒWF3£¦æ÷r‚“°¢ÆWBvVçEö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB6æF–FFRÒ6Æ–Ô6æF–FFR°¢–C¢WV–C£¦æWu÷cB‚’À¢–FV×÷FVæ7•ö¶W“¢&6Æ–ÒÖöæ6R"çFõ÷7G&–ær‚’À¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢&÷VçG•ö6öçG&7C¢#ƒ"çFõ÷7G&–ær‚’À¢6öÇfW%÷vÆÆWC¢#ƒ#######################################""çFõ÷7G&–ær‚’À¢vVçEö–C¢6öÖR†vVçEö–B’À¢VÆ–v–&–Æ—G•öWf–FVæ6S¢vVçDVÆ–v–&–Æ—G”Wf–FVæ6R°¢vVçEö–C¢6öÖR†vVçEö–B’À¢6öÇfW%÷vÆÆWC¢#ƒ#######################################""çFõ÷7G&–ær‚’À¢6&–Æ—F–W3¢fV3£¦æWr‚’À¢–Eö6ö×ÆWF–öç3¢À¢–E÷W6F5ö&6U÷Væ—G3¢À¢ÒÀ¢VÆ–v–&–Æ—G•öFV6—6–öã¢vVçDVÆ–v–&–Æ—G”FV6—6–öâ°¢VÆ–v–&ÆS¢G'VRÀ¢&V6öç3¢fV3£¦æWr‚’À¢ÒÀ¢7FGW3¢6Æ–Ô6æF–FFU7FGW3£¥&VÆ––ærÀ¢W†6ÇW6—fU÷VçF–Ã¢6öÖR†æ÷r’À¢WF†÷&—¦F–öåöæöæ6S¢6öÖR†f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’’’À¢WF†÷&—¦F–öå÷fÆ–Eö&Vf÷&S¢6öÖRƒóƒóó’À¢6Æ–Õ÷G&ç67F–öåö†6ƒ¢6öÖR†f÷&ÖB‚#‡·Ò"Â##""ç&WVBƒ3"’’’À¢6æöæ–6ÅöWfVçEö–C¢æöæRÀ¢f–ÇW&Uö6öFS¢æöæRÀ¢f–ÇW&UöÖW76vS¢æöæRÀ¢7&VFVEöC¢æ÷rÀ¢WFFVEöC¢æ÷rÀ¢Ó° ¢76W'B‡fÆ–FFU÷W'6—7FVEö6Æ–Õö6æF–FFU÷66÷R€¢f6æF–FFRÀ¢&&6RÖÖ–ææWB"À¢#ƒ"À¢#ƒ#######################################""À¢6öÖR†vVçEö–B’À¢¢æ—5öö²‚’“°¢76W'B‡fÆ–FFU÷W'6—7FVEö6Æ–Õö6æF–FFU÷66÷R€¢f6æF–FFRÀ¢&&6R×6WöÆ–"À¢#ƒ"À¢#ƒ#######################################""À¢6öÖR†vVçEö–B’À¢¢æ—5öW'"‚’“°¢76W'B‡fÆ–FFU÷W'6—7FVEö6Æ–Õö6æF–FFU÷66÷R€¢f6æF–FFRÀ¢&&6RÖÖ–ææWB"À¢#ƒ"À¢#ƒ3333333333333333333333333333333333333332"À¢6öÖR†vVçEö–B’À¢¢æ—5öW'"‚’“°¢76W'B‡fÆ–FFU÷W'6—7FVEö6Æ–Õö6æF–FFU÷66÷R€¢f6æF–FFRÀ¢&&6RÖÖ–ææWB"À¢#ƒ"À¢#ƒ#######################################""À¢æöæRÀ¢¢æ—5öW'"‚’“°¢Ð ¢5·FW7EÐ¢fâvVçEö6Æ–Õ÷&VÆ•÷&V¦V7G5÷fÇVUö÷%÷w&öæu÷6†R‚’°¢ÆWB&÷VçG’Ò#ƒ#°¢ÆWB&VÆ–W"Ò#ƒ#######################################"#°¢ÆWB×WB–çFVçBÒWfÕG&ç67F–öä–çFVçB°¢g&öÓ¢6öÖR‡&VÆ–W"çFõ÷7G&–ær‚’’À¢Fó¢&÷VçG’çFõ÷7G&–ær‚’À¢fÇVU÷vV“¢À¢FF¢f÷&ÖB‚#‡³£ãCSgÒ"Â&&6C#3B"’À¢gVæ7F–öã ¢&6Æ–Õv—F„WF†÷&—¦F–öâ†FG&W72ÇV–çC#SbÇV–çC#SbÆ'—FW33"ÇV–çC‚Æ'—FW33"Æ'—FW33"’ ¢çFõ÷7G&–ær‚’À¢Ó°¢76W'B‡fÆ–FFUövVçEö6Æ–Õ÷&VÆ•ö–çFVçB‚f–çFVçBÂ&÷VçG’Â&VÆ–W"’æ—5öö²‚’“°¢–çFVçBçfÇVU÷vV’Ò°¢76W'B‡fÆ–FFUövVçEö6Æ–Õ÷&VÆ•ö–çFVçB‚f–çFVçBÂ&÷VçG’Â&VÆ–W"’æ—5öW'"‚’“°¢Ð ¢5·FW7EÐ¢fâÖÆf÷&ÖVE÷6öÇfW%÷6–væGW&Uö—5÷&V¦V7FVEö&Vf÷&U÷7öç6÷'6†—‚’°¢ÆWB6–væGW&RÒWFöæöÖ÷W4&÷VçG”WF†÷&—¦F–öå6–væGW&R°¢c¢#rÀ¢#¢#ƒ"çFõ÷7G&–ær‚’À¢3¢f÷&ÖB‚#‡³£cG‡Ò"Â"’À¢Ó°¢ÆWBW'&÷"Ò¦ö–æVE÷6–væGW&R‚g6–væGW&R’çVçw&öW'"‚“°¢76W'EöW†W'&÷"ãÂ7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’“°¢Ð ¢5·FW7EÐ¢fâæF—fU÷vÆÆWE÷6–væGW&Uö—5÷7Æ—EöæEöæ÷&ÖÆ—¦VB‚’°¢f÷"†–çWE÷bÂW‡V7FVE÷b’–â²ƒ÷S‚Â#u÷S‚’ÂƒÂ#‚’Âƒ#rÂ#r’Âƒ#‚Â#‚•Ò°¢ÆWBVæ6öFVBÒf÷&ÖB‚#‡·×·×³£'‡Ò"Â#"ç&WVBƒ3"’Â##""ç&WVBƒ3"’Â–çWE÷b“° ¢ÆWB'6VBÒ'6UöæF—fU÷vÆÆWE÷6–væGW&R‚fVæ6öFVB’çVçw&‚“° ¢76W'EöW‡'6VBçbÂW‡V7FVE÷b“°¢76W'EöW‡'6VBç"Âf÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’’“°¢76W'EöW‡'6VBç2Âf÷&ÖB‚#‡·Ò"Â##""ç&WVBƒ3"’’“°¢76W'EöW€¢¦ö–æVE÷6–væGW&R‚g'6VB’çVçw&‚’À¢f÷&ÖB‚#‡·×·×³£'‡Ò"Â#"ç&WVBƒ3"’Â##""ç&WVBƒ3"’ÂW‡V7FVE÷b¢“°¢Ð¢Ð ¢5·FW7EÐ¢fâÖÆf÷&ÖVEöæF—fU÷vÆÆWE÷6–væGW&W5ö&U÷&V¦V7FVB‚’°¢ÆWB–çfÆ–BÒ°¢#ƒ"çFõ÷7G&–ær‚’À¢f÷&ÖB‚'·×·Ó""Â#"ç&WVBƒ3"’Â##""ç&WVBƒ3"’’À¢f÷&ÖB‚#‡·×§¢"Â#"ç&WVBƒcB’’À¢f÷&ÖB‚#‡·×·Ó""Â#"ç&WVBƒ3"’Â##""ç&WVBƒ3"’’À¢Ó° ¢f÷"6–væGW&R–â–çfÆ–B°¢ÆWBW'&÷"Ò'6UöæF—fU÷vÆÆWE÷6–væGW&R‚g6–væGW&R’çVçw&öW'"‚“°¢76W'EöW†W'&÷"ãÂ7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E’“°¢Ð¢Ð ¢5·FW7EÐ¢fâvVçEö6Æ–Õ÷&V¦V7G5öÖ&–wV÷W5÷6–væGW&Uöf÷&×5öæE÷&W6W'fW5öÆVv7•öf÷&Ò‚’°¢ÆWBÆVv7’ÒWFöæöÖ÷W4&÷VçG”WF†÷&—¦F–öå6–væGW&R°¢c¢#rÀ¢#¢f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’’À¢3¢f÷&ÖB‚#‡·Ò"Â##""ç&WVBƒ3"’’À¢Ó°¢ÆWB×WB&WVW7BÒvVçDæF—fT6Æ–Õ&WVW7B°¢–FV×÷FVæ7•ö¶W“¢&æF—fR×6–væGW&R×FW7B"çFõ÷7G&–ær‚’À¢æWGv÷&³¢6öÖR‚&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’’À¢&÷VçG•ö6öçG&7C¢#ƒ"çFõ÷7G&–ær‚’À¢6öÇfW%÷vÆÆWC¢#ƒ#######################################""çFõ÷7G&–ær‚’À¢vVçEö–C¢æöæRÀ¢&WVW7Eö&öæE÷7öç6÷'6†—¢G'VRÀ¢6–væGW&S¢6öÖR†ÆVv7’æ6ÆöæR‚’’À¢vÆÆWE÷6–væGW&S¢æöæRÀ¢6÷W&6S¢6öÖR‚'FW7B"çFõ÷7G&–ær‚’’À¢Ó° ¢ÆWB&W6öÇfVBÒ&W6öÇfUövVçEö6Æ–Õ÷6–væGW&R‚g&WVW7B’çVçw&‚’çVçw&‚“°¢76W'EöW‡&W6öÇfVBçbÂÆVv7’çb“°¢76W'EöW‡&W6öÇfVBç"ÂÆVv7’ç"“°¢76W'EöW‡&W6öÇfVBç2ÂÆVv7’ç2“° ¢&WVW7BçvÆÆWE÷6–væGW&RÒ6öÖR†f÷&ÖB‚#‡·×·Ó""Â#"ç&WVBƒ3"’Â##""ç&WVBƒ3"’’“°¢ÆWBW'&÷"ÒfÆ–FFUövVçEöæF—fUö6Æ–Õ÷&WVW7B‚g&WVW7B’çVçw&öW'"‚“°¢76W'EöW†W'&÷"ãÂ7FGW46öFS£¤$Eõ$UTU5B“°¢Ð ¢5·FW7EÐ¢fâvVçEö6Æ–Õ÷&WGW&ç5öåöW†7EöV—“5÷vÆÆWE÷&WVW7EöæE÷&WÆ•÷F‚‚’°¢ÆWB–ÆöC¢V—3”WF†÷&—¦F–öåG—VDFFÒ6W&FUö§6öã£¦g&öÕ÷fÇVR‡6W&FUö§6öã£¦§6öâ‡°¢'G—W2#¢·ÒÀ¢&FöÖ–â#¢°¢&æÖR#¢%U4B6ö–â"À¢'fW'6–öâ#¢#""À¢&6†–ä–B#¢ƒCS2À¢'fW&–g––æt6öçG&7B#¢#ƒƒ33Sƒ–d4CfTF#dS†cF3t33$CFcs#SF&D#“2 ¢ÒÀ¢'&–Ö'•G—R#¢%&V6V—fUv—F„WF†÷&—¦F–öâ"À¢&ÖW76vR#¢°¢&g&öÒ#¢#ƒ#######################################""À¢'Fò#¢#ƒ"À¢'fÇVR#¢#"À¢'fÆ–DgFW"#¢#"À¢'fÆ–D&Vf÷&R#¢#ƒ"À¢&æöæ6R#¢f÷&ÖB‚#‡·Ò"Â#32"ç&WVBƒ3"’¢Ð¢Ò’¢çVçw&‚“°¢ÆWB6öÇfW"Ò#ƒ#######################################"#° ¢ÆWBvÆÆWE÷&WVW7BÒV—“5÷vÆÆWE÷&WVW7B‡6öÇfW"Âg–ÆöB“° ¢76W'EöW‡vÆÆWE÷&WVW7E²&ÖWF†öB%ÒÂ&WF…÷6–våG—VDFF÷cB"“°¢76W'EöW‡vÆÆWE÷&WVW7E²'&×2%Õ³ÒÂ6öÇfW"“°¢ÆWBVæ6öFVE÷–ÆöBÒvÆÆWE÷&WVW7E²'&×2%Õ³Òæ5÷7G"‚’çVçw&‚“°¢76W'EöW€¢6W&FUö§6öã£¦g&öÕ÷7G#££Ç6W&FUö§6öã£¥fÇVSâ†Væ6öFVE÷–ÆöB’çVçw&‚’À¢6W&FUö§6öã£¦§6öâ‡–ÆöB¢“° ¢ÆWB&WVW7BÒvVçDæF—fT6Æ–Õ&WVW7B°¢–FV×÷FVæ7•ö¶W“¢'vÆÆWB×&WVW7B×FW7B"çFõ÷7G&–ær‚’À¢æWGv÷&³¢6öÖR‚&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’’À¢&÷VçG•ö6öçG&7C¢#ƒ"çFõ÷7G&–ær‚’À¢6öÇfW%÷vÆÆWC¢6öÇfW"çFõ÷7G&–ær‚’À¢vVçEö–C¢æöæRÀ¢&WVW7Eö&öæE÷7öç6÷'6†—¢G'VRÀ¢6–væGW&S¢æöæRÀ¢vÆÆWE÷6–væGW&S¢æöæRÀ¢6÷W&6S¢6öÖR‚'FW7B"çFõ÷7G&–ær‚’’À¢Ó°¢ÆWB&WÆ’Ò6–væVEö6Æ–Õ÷&WVW7E÷FV×ÆFR‚g&WVW7B“°¢76W'EöW‡&WÆ•²&–ç6W'E÷6–væGW&UöB%ÒÂ&&öG’çvÆÆWE÷6–væGW&R"“°¢76W'B‡&WÆ•²&&öG’%Õ²'vÆÆWE÷6–væGW&R%Ð¢æ5÷7G"‚¢çVçw&‚¢æ6öçF–ç2‚'vÆÆWE÷&WVW7B"’“°¢Ð ¢5·FW7EÐ¢fâƒC%÷–ÖVçE÷&WV—&VE÷&W7öç6U÷W6W5÷c%÷v—&Uö†VFW%öæEöæõ÷7F÷&R‚’°¢ÆWB6†ÆÆVævRÒ&6U÷W6F5ögVæF–æuö6†ÆÆVævR€¢&‡GG3¢òö’æW†×ÆR÷c÷ƒC"ö&6Rö&÷VçF–W2óƒögVæF–æsöæWGv÷&³Ö&6RÖÖ–ææWBfÖ÷VçCÓS"À¢&V—SS£ƒCS2"À¢#ƒƒ33Sƒ–d4CfTF#dS†cF3t33$CFcs#SF&D#“2"À¢#ƒ"À¢SóÀ¢3À¢¢çVçw&‚“° ¢ÆWB&W7öç6RÒƒC%÷–ÖVçE÷&WV—&VE÷&W7öç6R†6†ÆÆVævRæ6ÆöæR‚’’çVçw&‚“° ¢76W'EöW‡&W7öç6Rç7FGW2‚’Â7FGW46öFS£¥”ÔTåEõ$UT•$TB“°¢76W'EöW€¢&W7öç6Ræ†VFW'2‚•¶†VFW#£¤44„Uô4ôåE$ôÅÒÀ¢&æò×7F÷&RÂ&—fFR ¢“°¢ÆWBFV6öFVBÒ–ÖVçG5÷ƒC#£¦FV6öFU÷–ÖVçE÷&WV—&VEö†VFW"€¢&W7öç6Ræ†VFW'2‚•µ”ÔTåEõ$UT•$TEô„TDU%Ð¢çFõ÷7G"‚¢çVçw&‚’À¢¢çVçw&‚“°¢76W'EöW†FV6öFVBÂ6†ÆÆVævR“°¢76W'EöW†FV6öFVBæ66WG5³Òç66†VÖRÂtTåEô$õTåE•ôeTäEõ44„TÔR“°¢Ð ¢5·FW7EÐ¢fâƒC%ögVæF–æuöÖ÷VçEöFVfVÇG5÷FõövöæE÷&V¦V7G5ö÷fW&gVæF–æuö÷%÷w&öæu÷7FFR‚’°¢76W'EöW€¢&W6öÇfU÷ƒC%ögVæF–æuöÖ÷VçB‚&÷Vâ"Â##"Â#S"ÂæöæR’çVçw&‚’À¢óƒSó ¢“°¢76W'EöW€¢&W6öÇfU÷ƒC%ögVæF–æuöÖ÷VçB‚&÷Vâ"Â##"Â#S"Â6öÖRƒ#S’’çVçw&‚’À¢#Só ¢“°¢76W'EöW€¢&W6öÇfU÷ƒC%ögVæF–æuöÖ÷VçB‚&÷Vâ"Â##"Â#S"Â6öÖRƒóƒSó’’çVçw&öW'"‚’À¢7FGW46öFS£¤4ôädÄ”5@¢“°¢76W'EöW€¢&W6öÇfU÷ƒC%ögVæF–æuöÖ÷VçB‚&6Æ–Ö&ÆR"Â##"Â##"ÂæöæR’çVçw&öW'"‚’À¢7FGW46öFS£¤4ôädÄ”5@¢“°¢Ð ¢5·FW7EÐ¢fâ†÷7FVE÷ƒC%ö–çFVçEöÆÆ÷w5ööæÇ•öW†7E÷¦W&õ÷fÇVUögVæF–æuö6ÆÂ‚’°¢ÆWB&VÆ–W"Ò#ƒ#######################################"#°¢ÆWB&÷VçG’Ò#ƒ#°¢ÆWB×WB–çFVçBÒWfÕG&ç67F–öä–çFVçB°¢g&öÓ¢6öÖR‡&VÆ–W"çFõ÷7G&–ær‚’’À¢Fó¢&÷VçG’çFõ÷7G&–ær‚’À¢fÇVU÷vV“¢À¢FF¢f÷&ÖB€¢#‡·×·Ò"À¢UDôäôÔõU5ôeTäEõt•D…ôUD„õ$•¤D”ôåõ4TÄT5Dõ"À¢#"ç&WVBƒ‚¢3"¢’À¢gVæ7F–öã¢UDôäôÔõU5ôeTäEõt•D…ôUD„õ$•¤D”ôåôeTä5D”ôâçFõ÷7G&–ær‚’À¢Ó°¢76W'B‡fÆ–FFUö†÷7FVE÷ƒC%ö–çFVçB‚f–çFVçBÂ&VÆ–W"Â&÷VçG’’æ—5öö²‚’“° ¢–çFVçBçfÇVU÷vV’Ò°¢76W'EöW€¢fÆ–FFUö†÷7FVE÷ƒC%ö–çFVçB‚f–çFVçBÂ&VÆ–W"Â&÷VçG’’çVçw&öW'"‚’À¢7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E¢“°¢–çFVçBçfÇVU÷vV’Ò°¢–çFVçBæFFÒ#†FVF&VVb"çFõ÷7G&–ær‚“°¢76W'EöW€¢fÆ–FFUö†÷7FVE÷ƒC%ö–çFVçB‚f–çFVçBÂ&VÆ–W"Â&÷VçG’’çVçw&öW'"‚’À¢7FGW46öFS£¥Tå$ô4U54$ÄUôTåD•E¢“°¢Ð ¢5·FW7EÐ¢fâ6öæf—&ÖVE÷ƒC%÷&VÆ•öVÖ—G5÷–ÖVçE÷&W7öç6UööæÇ•ögFW%ö6æöæ–6ÅöWfVçB‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWBæ÷rÒWF3£¦æ÷r‚“°¢ÆWBGFV×BÒƒC%&VÆ”GFV×B°¢–C¢WV–C£¦æWu÷cB‚’À¢–FV×÷FVæ7•ö¶W“¢'ƒC#§FW7B"çFõ÷7G&–ær‚’À¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢&÷VçG•ö6öçG&7C¢#ƒ"çFõ÷7G&–ær‚’À¢6öçG&–'WF÷#¢#ƒ3333333333333333333333333333333333333332"çFõ÷7G&–ær‚’À¢Ö÷VçC¢SóÀ¢WF†÷&—¦F–öåöæöæ6S¢f÷&ÖB‚#‡·Ò"Â#CB"ç&WVBƒ3"’’À¢WF†÷&—¦F–öå÷fÆ–Eö&Vf÷&S¢%óóóÀ¢&WVW7Eöf–ævW'&–çC¢&f–ævW'&–çB"çFõ÷7G&–ær‚’À¢&VÆ–W%öFG&W73¢#ƒ#######################################""çFõ÷7G&–ær‚’À¢7FGW3¢ƒC%&VÆ•7FGW3£¤6öæf—&ÖVBÀ¢&WG'–&ÆS¢fÇ6RÀ¢GFV×Eö6÷VçC¢À¢G…ö†6ƒ¢6öÖR†f÷&ÖB‚#‡·Ò"Â#SR"ç&WVBƒ3"’’’À¢W7F–ÖFVEöv3¢6öÖRƒó’À¢v5öÆ–Ö—C¢6öÖRƒ#ó’À¢W'&÷%ö6öFS¢æöæRÀ¢W'&÷%öÖW76vS¢æöæRÀ¢6æöæ–6ÅöWfVçEö–C¢6öÖR…WV–C£¦æWu÷cB‚’’À¢6öæf—&ÖVEö&Æö6³¢6öÖRƒ#2’À¢7&VFVEöC¢æ÷rÀ¢WFFVEöC¢æ÷rÀ¢Ó° ¢ÆWBV&Æ–5÷&VÆ’ÒƒC%÷V&Æ–5÷&VÆ’‚fGFV×B’çFõ÷7G&–ær‚“°¢76W'B‚V&Æ–5÷&VÆ’æ6öçF–ç2‚fGFV×Bæ–FV×÷FVæ7•ö¶W’’“°¢76W'B‚V&Æ–5÷&VÆ’æ6öçF–ç2‚fGFV×BæWF†÷&—¦F–öåöæöæ6R’“°¢76W'B‚V&Æ–5÷&VÆ’æ6öçF–ç2‚fGFV×Bç&WVW7Eöf–ævW'&–çB’“° ¢ÆWB&W7öç6RÒƒC%÷&VÆ•÷&W7öç6R‚g7FFRÂfGFV×B’çVçw&‚“°¢76W'EöW‡&W7öç6Rç7FGW2‚’Â7FGW46öFS£¤ô²“°¢ÆWB6WGFÆVÖVçBÒ–ÖVçG5÷ƒC#£¦FV6öFU÷–ÖVçE÷&W7öç6Uö†VFW"€¢&W7öç6Ræ†VFW'2‚•µ”ÔTåEõ$U5ôå4Uô„TDU%Ð¢çFõ÷7G"‚¢çVçw&‚’À¢¢çVçw&‚“°¢76W'B‡6WGFÆVÖVçBç7V66W72“°¢76W'EöW‡6WGFÆVÖVçBæÖ÷VçBæ5öFW&Vb‚’Â6öÖR‚#S"’“°¢76W'EöW‡6WGFÆVÖVçBææWGv÷&²Â&V—SS£ƒCS2"“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâƒC%öF—66÷fW'•ö—5öW‡Æ–6—Eö&÷WEö7W7FöÕögVæF–æuöæEö×ö&÷VæF'’‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWBFö7VÖVçBÒƒC%öF—66÷fW'’…7FFR‡7FFR’’æv—Bã° ¢76W'EöW†Fö7VÖVçE²'ƒC%fW'6–öâ%ÒÂƒC%õdU%4”ôâ“°¢76W'EöW†Fö7VÖVçE²'&W6÷W&6W2%Õ³Õ²'66†VÖR%ÒÂtTåEô$õTåE•ôeTäEõ44„TÔR“°¢76W'EöW†Fö7VÖVçE²'&W6÷W&6W2%Õ³Õ²&vVæW&–4W†7D6ö×F–&ÆR%ÒÂfÇ6R“°¢76W'EöW†Fö7VÖVçE²&†÷7FVE&VÆ’%Õ²&Væ&ÆVB%ÒÂfÇ6R“°¢76W'EöW†Fö7VÖVçE²&†÷7FVE&VÆ’%Õ²&Ö–åW6F4&6UVæ—G2%ÒÂ#"“°¢76W'EöW†Fö7VÖVçE²&†÷7FVE&VÆ’%Õ²&Ö„F–Ç”GFV×G2%ÒÂ“°¢76W'EöW€¢Fö7VÖVçE²&†÷7FVE&VÆ’%Õ²&Ö„F–Ç”GFV×G5W$6öçG&–'WF÷"%ÒÀ¢ ¢“°¢76W'B†Fö7VÖVçE²&†÷7FVE&VÆ’%Õ²'7FGW5W&ÅFV×ÆFR%Ð¢æ5÷7G"‚¢çVçw&‚¢æ6öçF–ç2‚"÷c÷ƒC"ö&6R÷&VÆ—2÷·&VÆ•ö–GÒ"’“°¢76W'EöW†Fö7VÖVçE²&×%Õ²'7FGW2%ÒÂ'ÆææVB"“°¢76W'B†Fö7VÖVçE²'6fWG’%Õ²'7FæF&DW†7EFô&÷VçG”6öçG&7B%Ð¢æ5÷7G"‚¢çVçw&‚¢æ6öçF–ç2‚$gVæF–ætFFVB"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâvVçE÷–E÷7FGW5öVæGö–çE÷7VÖÖ&—¦W5÷6öÇfW%÷&V6V—f&ÆW2‚’°¢ÆWB†æWGv÷&²Âö&÷VçG’Â÷&ööb’Ò6ö×ÆWFVE÷6–×VÆFVEö&÷VçG’‚’æv—C°¢ÆWB6öÇfW%ö–BÒæWGv÷&°¢ç6WGFÆVÖVçG0¢çfÇVW2‚¢æfÆEöÖ‡Ç6WGFÆVÖVçGÂg6WGFÆVÖVçBç–÷WEö–çFVçG2¢æf–æB‡Æ–çFVçGÂ–çFVçBæÖ÷VçBæ7W'&Væ7’ÓÒ'W6F2"¢æW‡V7B‚'6öÇfW"–÷WB–çFVçBW†—7G2"¢ç&V6—–VçEövVçEö–C°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWB&W7öç6RÒvVçE÷–E÷7FGW2…7FFR‡7FFR’ÂF‚‡6öÇfW%ö–B’¢æv—@¢çVçw&‚¢ã°¢ÆWB÷7E÷fÇVRÒg&W7öç6U²'÷7E÷fÇVUöÆö÷%Ó°¢76W'EöW‡÷7E÷fÇVU²'G&–vvW"%ÒÂ'fW&–f–VEö6ö×ÆWF–öâ"“°¢76W'B‡÷7E÷fÇVU²'6VÆeö–çFW&W7B%Ð¢æ5÷7G"‚¢çVçw&‚¢æ6öçF–ç2‚&Ö÷&RæB†–v†W"×fÇVRgVæFVB&÷VçF–W2"’“°¢76W'B‡÷7E÷fÇVU²&7F–öç2%Ð¢æ5ö'&’‚¢çVçw&‚¢æ—FW"‚¢æç’‡Æ7F–öçÂ7F–öå²&¶–æB%ÒÓÒ'FVÆÅ÷–÷W%ö‡VÖâ"’“°¢ÆWB&W7öç6S¢£¤vVçE–÷WE7FGW5&W7öç6RÒ6W&FUö§6öã£¦g&öÕ÷fÇVR‡&W7öç6R’çVçw&‚“° ¢76W'EöW‡&W7öç6RævVçBæ–BÂ6öÇfW%ö–B“°¢76W'EöW‡&W7öç6Rç–÷WG2æÆVâ‚’Â“°¢76W'EöW‡&W7öç6Rç–÷WG5³Òç7FGW2Â–÷WE7FGW3£¥–B“°¢76W'EöW‡&W7öç6RçF÷FÇ5³Òæ7W'&Væ7’Â'W6F2"“°¢76W'EöW‡&W7öç6RçF÷FÇ5³ÒçVæF–æuöÖ–æ÷"Â“°¢76W'EöW‡&W7öç6RçF÷FÇ5³Òç–EöÖ–æ÷"Âóó“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ6–×VÆFVE÷–E÷7FGW5÷W6W5÷fW&–f–VEö6ö×ÆWF–öåö6÷’‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB6öÇfW"ÒæWGv÷&²ç&Vv—7FW%övVçB…&Vv—7FW$vVçE&WVW7B°¢†æFÆS¢'6–×VÆFVB×6öÇfW""çFõ÷7G&–ær‚’À¢–÷WE÷vÆÆWC¢æöæRÀ¢Ò“°¢ÆWB&÷VçG’ÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢%fW&–g’6–×VÆFVBF—7G&–'WF–öâ6÷’"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'6ÖÆÂÖ6öFRÖ6†ævR"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò¢çVçw&‚“°¢æWGv÷&°¢æ6Æ–Õö&÷VçG’„6Æ–Ô&÷VçG•&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öÇfW%övVçEö–C¢6öÇfW"æ–BÀ¢Ò¢çVçw&‚“°¢ÆWB'F–f7BÒ'µÂ'6–×VÆFVEÂ#§G'VWÒ#°¢ÆWB7V&Ö—76–öâÒæWGv÷&°¢ç7V&Ö—E÷&W7VÇB…7V&Ö—E&W7VÇE&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öÇfW%övVçEö–C¢6öÇfW"æ–BÀ¢'F–f7E÷W&“¢'33¢ò÷FW7G2÷6–×VÆFVBæ§6öâ"çFõ÷7G&–ær‚’À¢'F–f7Eö&öG“¢'F–f7BçFõ÷7G&–ær‚’À¢Ò¢çVçw&‚“°¢æWGv÷&°¢çfW&–g•÷7V&Ö—76–öâ…fW&–g•7V&Ö—76–öå&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢7V&Ö—76–öåö–C¢7V&Ö—76–öâæ–BÀ¢W‡V7FVEö'F–f7EöF–vW7C¢†6…ö'F–f7B†'F–f7B’À¢fW&–f–W%ö¶–æC¢6öÖR…fW&–f–W$¶–æC£¤§6öå66†VÖ’À¢'V'&–3¢æöæRÀ¢Wf–FVæ6S¢æöæRÀ¢&÷fVE÷&—6µöWfVçEö–C¢æöæRÀ¢Ò¢æv—@¢çVçw&‚“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWB&W7öç6RÒvVçE÷–E÷7FGW2…7FFR‡7FFR’ÂF‚‡6öÇfW"æ–B’¢æv—@¢çVçw&‚¢ã° ¢76W'EöW€¢&W7öç6U²'÷7E÷fÇVUöÆö÷%Õ²'G&–vvW"%ÒÀ¢'fW&–f–VEö6ö×ÆWF–öâ ¢“°¢76W'B‚&W7öç6U²'÷7E÷fÇVUöÆö÷%Õ²'fÇVU÷7FFVÖVçB%Ð¢æ5÷7G"‚¢çVçw&‚¢æ6öçF–ç2‚'&V6V—fVB&V6öæ6–ÆVB–÷WB"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ7G&—Uö6†V6¶÷WE÷vV&†ööµö7&VF—G5÷ÆFf÷&Õö&Ææ6Uööæ6R‚’°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…÷Vç6–væVE÷7G&—U÷vV&†öö·2„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB&öG’Ò7G&—Uö6†V6¶÷WEöWfVçEö&öG’‚&WgE÷–B"Â&75÷–B"Â÷&væ—¦F–öåö–B“° ¢ÆWBf—'7BÒ&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢'—FW3£¦g&öÒ†&öG’æ6ÆöæR‚’’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'B‚f—'7BæGWÆ–6FR“°¢76W'EöW†f—'7BæÆVFvW%öVçG&–W2æÆVâ‚’Â“°¢76W'EöW€¢f—'7BægVæF–æuö7&VF—Bç–ÖVçEöWfVçBç7FGW2À¢–ÖVçDWfVçE7FGW3£¤Æ–V@¢“° ¢ÆWB&WÆ’Ò&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢'—FW3£¦g&öÒ†&öG’’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'B‡&WÆ’æGWÆ–6FR“°¢76W'B‡&WÆ’æÆVFvW%öVçG&–W2æ—5öV×G’‚’“°¢76W'EöW€¢&WÆ’ægVæF–æuö7&VF—Bç–ÖVçEöWfVçBç7FGW2À¢–ÖVçDWfVçE7FGW3£¤–væ÷&VDGWÆ–6FP¢“°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’æW‡V7B‚'7FFRö—6öæVB"“°¢76W'EöW†æWGv÷&²ç–ÖVçEöWfVçG2æÆVâ‚’Â“°¢76W'EöW†æWGv÷&²æÆVFvW"æVçG&–W2‚’æÆVâ‚’Â“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ7G&—Uö6†V6¶÷WE÷vV&†ööµ÷&V¦V7G5÷Vç6–væVE÷v†Våöæ÷EöW‡Æ–6—FÇ•öÆÆ÷vVB‚’°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB&öG’Ð¢7G&—Uö6†V6¶÷WEöWfVçEö&öG’‚&WgE÷Vç6–væVE÷–B"Â&75÷Vç6–væVE÷–B"Â÷&væ—¦F–öåö–B“° ¢76W'EöW€¢&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²…7FFR‡7FFR’Â†VFW$Ö£¦æWr‚’Â'—FW3£¦g&öÒ†&öG’’Â¢æv—@¢çVçw&öW'"‚’À¢7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄP¢“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ7G&—Uö6†V6¶÷WE÷vV&†ööµ÷&WV—&W5÷fÆ–E÷6–væGW&U÷v†Vå÷6V7&WEö6öæf–wW&VB‚’°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB6V7&WBÒ"'v‡6V5÷FW7B#°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…÷7G&—U÷vV&†ööµ÷6V7&WB„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â6V7&WB“°¢ÆWB&öG’Ò7G&—Uö6†V6¶÷WEöWfVçEö&öG’‚&WgE÷6–væVE÷–B"Â&75÷6–væVE÷–B"Â÷&væ—¦F–öåö–B“° ¢76W'EöW€¢&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢'—FW3£¦g&öÒ†&öG’æ6ÆöæR‚’’À¢¢æv—@¢çVçw&öW'"‚’À¢7FGW46öFS£¤$Eõ$UTU5@¢“° ¢ÆWB×WB&Eö†VFW'2Ò†VFW$Ö£¦æWr‚“°¢&Eö†VFW'2æ–ç6W'B‚'7G&—R×6–væGW&R"Â'CÓsÇcÓ"ç'6R‚’çVçw&‚’“°¢76W'EöW€¢&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²€¢7FFR‡7FFRæ6ÆöæR‚’’À¢&Eö†VFW'2À¢'—FW3£¦g&öÒ†&öG’æ6ÆöæR‚’’À¢¢æv—@¢çVçw&öW'"‚’À¢7FGW46öFS£¤$Eõ$UTU5@¢“° ¢ÆWB×WB6–væVEö†VFW'2Ò†VFW$Ö£¦æWr‚“°¢6–væVEö†VFW'2æ–ç6W'B€¢'7G&—R×6–væGW&R"À¢7G&—U÷6–væGW&Uö†VFW"‚f&öG’Â6V7&WB’ç'6R‚’çVçw&‚’À¢“°¢ÆWB6–væVBÒ&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²€¢7FFR‡7FFRæ6ÆöæR‚’’À¢6–væVEö†VFW'2À¢'—FW3£¦g&öÒ†&öG’’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'B‚6–væVBæGWÆ–6FR“°¢76W'EöW‡6–væVBæÆVFvW%öVçG&–W2æÆVâ‚’Â“°¢76W'EöW€¢6–væVBægVæF–æuö7&VF—Bç–ÖVçEöWfVçBç7FGW2À¢–ÖVçDWfVçE7FGW3£¤Æ–V@¢“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ7G&—Uö6†V6¶÷WE÷F÷÷WöVæGö–çE÷Æç5ö6†V6¶÷WE÷6W76–öâ‚’°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“° ¢ÆWB–çFVçBÒÆå÷7G&—Uö6†V6¶÷WE÷F÷÷W€¢7FFR‡7FFR’À¢§6öâ…Æå7G&—T6†V6¶÷WEF÷W&WVW7B°¢÷&væ—¦F–öåö–BÀ¢Ö÷VçEöÖ–æ÷#¢UóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢7V66W75÷W&Ã¢æöæRÀ¢6æ6VÅ÷W&Ã¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW†–çFVçBæVæGö–çBÂ"÷cö6†V6¶÷WB÷6W76–öç2"“°¢76W'B†–çFVçBæ–FV×÷FVæ7•ö¶W’æ6öçF–ç2‚&6†V6¶÷WE÷F÷÷W"’“°¢76W'EöW†–çFVçBæ&öG•²&ÖöFR%ÒÂ'–ÖVçB"“°¢76W'EöW€¢–çFVçBæ&öG•²&6Æ–VçE÷&VfW&Væ6Uö–B%ÒÀ¢÷&væ—¦F–öåö–BçFõ÷7G&–ær‚¢“°¢76W'EöW€¢–çFVçBæ&öG•²'7V66W75÷W&Â%ÒÀ¢&‡GG¢òó#rããã£ƒƒ÷7G&—R÷7V66W72 ¢“°¢76W'EöW€¢–çFVçBæ&öG•²&6æ6VÅ÷W&Â%ÒÀ¢&‡GG¢òó#rããã£ƒƒ÷7G&—Rö6æ6VÂ ¢“°¢76W'B†–çFVçBæ&öG’ævWB‚'–ÖVçEöÖWF†öE÷G—W2"’æ—5öæöæR‚’“°¢76W'B†–çFVçBæ&öG’ævWB‚'–ÖVçEöÖWF†öEö6öæf–wW&F–öâ"’æ—5öæöæR‚’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ7G&—Uö6†V6¶÷WE÷F÷÷WöVæGö–çEöÆ–W5÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ‚’°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…÷7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ€¢&÷VçG”æWGv÷&³£¦FVfVÇB‚’À¢'Ö5÷—ÅöVæ&ÆVB"À¢“° ¢ÆWB–çFVçBÒÆå÷7G&—Uö6†V6¶÷WE÷F÷÷W€¢7FFR‡7FFR’À¢§6öâ…Æå7G&—T6†V6¶÷WEF÷W&WVW7B°¢÷&væ—¦F–öåö–BÀ¢Ö÷VçEöÖ–æ÷#¢UóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢7V66W75÷W&Ã¢æöæRÀ¢6æ6VÅ÷W&Ã¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW€¢–çFVçBæ&öG•²'–ÖVçEöÖWF†öEö6öæf–wW&F–öâ%ÒÀ¢'Ö5÷—ÅöVæ&ÆVB ¢“°¢76W'B†–çFVçBæ&öG’ævWB‚'–ÖVçEöÖWF†öE÷G—W2"’æ—5öæöæR‚’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ7G&—Uö6†V6¶÷WE÷F÷÷WöVæGö–çE÷&V¦V7G5ö&VÆ÷uöÖ–æ–×VÒ‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“° ¢ÆWBW'&÷"ÒÆå÷7G&—Uö6†V6¶÷WE÷F÷÷W€¢7FFR‡7FFR’À¢§6öâ…Æå7G&—T6†V6¶÷WEF÷W&WVW7B°¢÷&væ—¦F–öåö–C¢WV–C£¦æWu÷cB‚’À¢Ö÷VçEöÖ–æ÷#¢C’À¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢7V66W75÷W&Ã¢æöæRÀ¢6æ6VÅ÷W&Ã¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¤$Eõ$UTU5B“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ7G&—Uö6öææV7Eö66÷VçEöVæGö–çE÷W6W5ö66÷VçG5÷c"‚’°¢ÆWBvVçEö–BÒWV–C£¦æWu÷cB‚“° ¢ÆWB–çFVçBÐ¢Æå÷7G&—Uö6öææV7Eö66÷VçB„§6öâ…Æå7G&—T6öææV7D66÷VçE&WVW7B²vVçEö–BÒ’¢æv—@¢çVçw&‚¢ã° ¢76W'EöW†–çFVçBç&WVW7BæVæGö–çBÂ"÷c"ö6÷&Rö66÷VçG2"“°¢76W'EöW€¢–çFVçBç&WVW7Bæ&öG•²&ÖWFFF%Õ²&vVçEö–B%ÒÀ¢vVçEö–BçFõ÷7G&–ær‚¢“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâÆ—fU÷7G&—Uö6†V6¶÷WEöVæGö–çE÷&WGW&ç5öW†V7WF–öå÷&W÷'B‚’°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB7G&—Uö•ö&6U÷W&ÂÒ7vå÷'5÷&W7öç6R‡6W&FUö§6öã£¦§6öâ‡°¢&–B#¢&75÷FW7EöÆ—fR"À¢&ö&¦V7B#¢&6†V6¶÷WBç6W76–öâ"À¢'W&Â#¢&‡GG3¢òö6†V6¶÷WBç7G&—Ræ6öÒö2÷’ö75÷FW7EöÆ—fR"À¢&Æ—fVÖöFR#¢fÇ6P¢Ò’“°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…÷7G&—UöÆ—fR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â7G&—Uö•ö&6U÷W&Â“° ¢ÆWB&W÷'BÒW†V7WFU÷7G&—Uö6†V6¶÷WE÷F÷÷W€¢7FFR‡7FFR’À¢†VFW$Ö£¦æWr‚’À¢§6öâ…Æå7G&—T6†V6¶÷WEF÷W&WVW7B°¢÷&væ—¦F–öåö–BÀ¢Ö÷VçEöÖ–æ÷#¢UóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢7V66W75÷W&Ã¢æöæRÀ¢6æ6VÅ÷W&Ã¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW‡&W÷'Bç7FGW2Â#“°¢76W'EöW‡&W÷'Bç7G&—Uö–Bæ5öFW&Vb‚’Â6öÖR‚&75÷FW7EöÆ—fR"’“°¢76W'EöW€¢&W÷'BçW&Âæ5öFW&Vb‚’À¢6öÖR‚&‡GG3¢òö6†V6¶÷WBç7G&—Ræ6öÒö2÷’ö75÷FW7EöÆ—fR"¢“°¢76W'EöW‡&W÷'Bç&WVW7BæVæGö–çBÂ"÷cö6†V6¶÷WB÷6W76–öç2"“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5÷7G&—UögVæF–æuö–çFVçEö6†V6¶÷WEöW†V7WFW5ö&÷VçG•ö6†V6¶÷WB‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB&÷VçG’ÒæWGv÷&°¢æ÷Vå÷ööÆVEö&÷VçG’„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$FV&—B6&BgVæFVB&÷VçG’"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'6ÖÆÂÖ6öFRÖ6†ævR"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢UóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢FöÖ–ã£¤gVæF–ætÖöFS£¥7G&—Tf–DÆVFvW"À¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò¢çVçw&‚“°¢ÆWBgVæF–æuö–çFVçBÒæWGv÷&°¢æ7&VFUögVæF–æuö–çFVçB€¢7&VFTgVæF–æt–çFVçE&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢6öÖR†÷&væ—¦F–öåö–B’À¢Ö÷VçEöÖ–æ÷#¢UóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢&–Ã¢FöÖ–ã£¥–ÖVçE&–Ã£¥7G&—Tf–BÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR‚&6&BÖgVæF–ær×FW7B"çFõ÷7G&–ær‚’’À¢7G&—U÷7V66W75÷W&Ã¢6öÖR‚&‡GG3¢òövVçF&÷VçF–W2æ÷7V66W72æ‡FÖÂ"çFõ÷7G&–ær‚’’À¢7G&—Uö6æ6VÅ÷W&Ã¢6öÖR‚&‡GG3¢òövVçF&÷VçF–W2æö6æ6VÂæ‡FÖÂ"çFõ÷7G&–ær‚’’À¢ÒÀ¢&‡GG¢òó#rããã£ƒƒ"À¢¢çVçw&‚¢æ–çFVçC°¢ÆWB7G&—Uö•ö&6U÷W&ÂÒ7vå÷'5÷&W7öç6R‡6W&FUö§6öã£¦§6öâ‡°¢&–B#¢&75÷FW7Eö&÷VçG’"À¢&ö&¦V7B#¢&6†V6¶÷WBç6W76–öâ"À¢'W&Â#¢&‡GG3¢òö6†V6¶÷WBç7G&—Ræ6öÒö2÷’ö75÷FW7Eö&÷VçG’"À¢&Æ—fVÖöFR#¢fÇ6P¢Ò’“°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…÷7G&—U÷V&Æ–5ö6†V6¶÷WEöæE÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ€¢æWGv÷&²À¢7G&—Uö•ö&6U÷W&ÂÀ¢'Ö5÷—ÅöVæ&ÆVB"À¢“° ¢ÆWB&W÷'BÒW†V7WFU÷7G&—UögVæF–æuö–çFVçEö6†V6¶÷WB…7FFR‡7FFR’ÂF‚†gVæF–æuö–çFVçBæ–B’¢æv—@¢çVçw&‚¢ã° ¢76W'EöW‡&W÷'Bç7FGW2Â#“°¢76W'EöW‡&W÷'Bç&WVW7BæVæGö–çBÂ"÷cö6†V6¶÷WB÷6W76–öç2"“°¢76W'EöW€¢&W÷'Bç&WVW7Bæ–FV×÷FVæ7•ö¶W’À¢f÷&ÖB‚&&÷VçG•ögVæF–æuö–çFVçC§·Ò"ÂgVæF–æuö–çFVçBæ–B¢“°¢76W'EöW€¢&W÷'Bç&WVW7Bæ&öG•²'7V66W75÷W&Â%ÒÀ¢&‡GG3¢òövVçF&÷VçF–W2æ÷7V66W72æ‡FÖÂ ¢“°¢76W'EöW€¢&W÷'Bç&WVW7Bæ&öG•²&6æ6VÅ÷W&Â%ÒÀ¢&‡GG3¢òövVçF&÷VçF–W2æö6æ6VÂæ‡FÖÂ ¢“°¢76W'EöW€¢&W÷'Bç&WVW7Bæ&öG•²&ÖWFFF%Õ²&&÷VçG•ö–B%ÒÀ¢&÷VçG’æ–BçFõ÷7G&–ær‚¢“°¢76W'EöW€¢&W÷'Bç&WVW7Bæ&öG•²&ÖWFFF%Õ²&gVæF–æuö–çFVçEö–B%ÒÀ¢gVæF–æuö–çFVçBæ–BçFõ÷7G&–ær‚¢“°¢76W'EöW€¢&W÷'Bç&WVW7Bæ&öG•²'–ÖVçEöÖWF†öEö6öæf–wW&F–öâ%ÒÀ¢'Ö5÷—ÅöVæ&ÆVB ¢“°¢76W'B‡&W÷'Bç&WVW7Bæ&öG’ævWB‚'–ÖVçEöÖWF†öE÷G—W2"’æ—5öæöæR‚’“°¢76W'EöW€¢&W÷'BçW&Âæ5öFW&Vb‚’À¢6öÖR‚&‡GG3¢òö6†V6¶÷WBç7G&—Ræ6öÒö2÷’ö75÷FW7Eö&÷VçG’"¢“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5÷7G&—UögVæF–æuö–çFVçEö6†V6¶÷WEö—5öF—6&ÆVEö'•öFVfVÇB‚’°¢ÆWB7FFRÐ¢FW7E÷7FFU÷v—F…÷7G&—UöÆ—fR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â&‡GG¢òó#rããã£’"çFõ÷7G&–ær‚’“° ¢ÆWBW'&÷"ÒW†V7WFU÷7G&—UögVæF–æuö–çFVçEö6†V6¶÷WB…7FFR‡7FFR’ÂF‚…WV–C£¦æWu÷cB‚’’¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâÆ—fU÷7G&—UöW†V7WF–öåö—5öF—6&ÆVEö'•öFVfVÇB‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“° ¢ÆWBW'&÷"ÒW†V7WFU÷7G&—Uö6öææV7Eö66÷VçB€¢7FFR‡7FFR’À¢†VFW$Ö£¦æWr‚’À¢§6öâ…Æå7G&—T6öææV7D66÷VçE&WVW7B°¢vVçEö–C¢WV–C£¦æWu÷cB‚’À¢Ò’À¢¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ö—77VUö&÷VçG•÷Æå÷'6W5÷fÆ–Eö—77VUöf÷&Ò‚’°¢ÆWBÆâÒÆåöv—F‡V%ö—77VUö&÷VçG’„§6öâ…Æäv—D‡V$—77VT&÷VçG•&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2ór"çFõ÷7G&–ær‚’À¢F—FÆS¢%¶&÷VçG•Ó¢f—‚4’"çFõ÷7G&–ær‚’À¢&öG“¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢Ò’¢æv—@¢ã° ¢76W'B‡Æâç&VG’“°¢76W'B‡ÆâæW'&÷"æ—5öæöæR‚’“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¥7V66W72“°¢ÆWB'6VBÒÆâç'6VBæW‡V7B‚''6VB&÷VçG’"“°¢76W'EöW‡'6VBçFV×ÆFU÷6ÇVrÂ&f—‚Ö6’Öf–ÇW&R"“°¢76W'EöW‡'6VBæÖ÷VçBæÖ÷VçBÂóó“°¢76W'EöW‡'6VBæÖ÷VçBæ7W'&Væ7’Â'W6F2"“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ö—77VUö&÷VçG•÷Æå÷&WGW&ç5ö7F–öå÷&WV—&VEöf÷%ö&Eö—77VUöf÷&Ò‚’°¢ÆWBÆâÒÆåöv—F‡V%ö—77VUö&÷VçG’„§6öâ…Æäv—D‡V$—77VT&÷VçG•&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2ó‚"çFõ÷7G&–ær‚’À¢F—FÆS¢%¶&÷VçG•Ó¢Ö—76–ærf–VÆG2"çFõ÷7G&–ær‚’À¢&öG“¢"222vöÅÆäf—‚4’"çFõ÷7G&–ær‚’À¢Ò’¢æv—@¢ã° ¢76W'B‚Æâç&VG’“°¢76W'B‡Æâç'6VBæ—5öæöæR‚’“°¢76W'B‡ÆâæW'&÷"æW‡V7B‚&W'&÷""’æ6öçF–ç2‚&Ö—76–ær&WV—&VB"’“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¤7F–öå&WV—&VB“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5÷ööÆVEö&÷VçG•ö6ææ÷Eö÷fW'w&—FUöW†—7F–æu÷VægVæFVEö&÷VçG’‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWBW†—7F–ærÒ÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$÷&–v–æÂV&Æ–2&÷VçG’"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢FöÖ–ã£¤gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢ÆWBW'&÷"Ò÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢6öÖR†W†—7F–æræ–B’À¢–FV×÷FVæ7•ö¶W“¢6öÖR†f÷&ÖB€¢&v—F‡V"Ö—77VR×7–æ3¦vVçBÖ&÷VçF–W2öW†×ÆS§·Ò"À¢W†—7F–æræ–@¢’’À¢F—FÆS¢$÷fW'w&—FRV&Æ–2&÷VçG’"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢%óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢FöÖ–ã£¤gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥&—fFRÀ¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¤$Eõ$UTU5B“°¢ÆWB7FGW2Ò&÷VçG•÷7FGW2…7FFR‡7FFR’ÂF‚†W†—7F–æræ–B’¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡7FGW2æ&÷VçG’çF—FÆRÂ$÷&–v–æÂV&Æ–2&÷VçG’"“°¢76W'EöW‡7FGW2æ&÷VçG’ç&—f7’Â&—f7”ÆWfVÃ£¥V&Æ–2“°¢76W'EöW‡7FGW2æ&÷VçG’æÖ÷VçBæÖ÷VçBÂó“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ö—77VUö•÷7–æ5ö—5ö÷W&F÷%övFVEöæE÷F—FÆUöVF—E÷7F&ÆR‚’°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶Vâ„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â'6V7&WB×Fö¶Vâ"“°¢ÆWBFVæ–VBÒ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢RÀ¢%¶&÷VçG•Ó¢7–æ2v—D‡V"—77VR–çFò’"À¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢’’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†FVæ–VBÂ7FGW46öFS£¥TäUD„õ$•¤TB“° ¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B„õU$Dõ%õDô´Tåô„TDU"Â'6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWBf—'7BÒ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢RÀ¢%¶&÷VçG•Ó¢7–æ2v—D‡V"—77VR–çFò’"À¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢’’À¢¢æv—@¢çVçw&‚¢ã° ¢ÆWBVF—FVBÒ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢RÀ¢%¶&÷VçG•Ó¢7–æ2†÷7FVBv—D‡V"—77VR&÷VçG’&V6÷&G2"À¢fÆ–Eöv—F‡V%ö—77VUö&öG•÷v—F…övöÂ€¢$¶VWF†R6ÖR†÷7FVB&÷VçG’gFW"â—77VRF—FÆRVF—Bâ"À¢’À¢’’À¢¢æv—@¢çVçw&‚¢ã° ¢ÆWB÷F†W%ö—77VRÒ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7FFR’À¢†VFW'2À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢bÀ¢%¶&÷VçG•Ó¢7–æ2†÷7FVBv—D‡V"—77VR&÷VçG’&V6÷&G2"À¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢’’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW†VF—FVBæ–BÂf—'7Bæ–B“°¢76W'EöæR†÷F†W%ö—77VRæ–BÂf—'7Bæ–B“°¢76W'EöW€¢VF—FVBçF—FÆRÀ¢%¶&÷VçG•Ó¢7–æ2†÷7FVBv—D‡V"—77VR&÷VçG’&V6÷&G2 ¢“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢5¶–væ÷&RÒ'&WV—&W2tTåEô$õTåD”U5õDU5EôDD$4UõU$Â%Ð¢7–æ2fâVF–Væ6UöVF—E÷W'6—7G5ö–FV×÷FVçFÇ•ö7&÷75÷&ö6W76W2‚’°¢ÆWBFF&6U÷W&ÂÒ÷7Fw&W5÷FW7EöFF&6U÷W&Â‚“°¢ÆWBf—'7E÷7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢f—'7E÷7F÷&RæÖ–w&FR‚’æv—BçVçw&‚“°¢ÆWBf—'7E÷7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R€¢&÷VçG”æWGv÷&³£¦FVfVÇB‚’À¢'6V7&WB×Fö¶Vâ"À¢f—'7E÷7F÷&RÀ¢“°¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B„õU$Dõ%õDô´Tåô„TDU"Â'6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWBVæ—VRÒWV–C£¦æWu÷cB‚“° ¢ÆWBÖVÖ&W"ÒW6W'EöVF–Væ6UöÖVÖ&W"€¢7FFR†f—'7E÷7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…W6W'DVF–Væ6TÖVÖ&W%&WVW7B°¢&÷f–FW#¢FöÖ–ã£¤VF–Væ6U&÷f–FW#£¤v—F‡V"À¢W‡FW&æÅö–C¢f÷&ÖB‚&v—F‡V"×W6W"×·Væ—VWÒ"’À¢†æFÆS¢f÷&ÖB‚&VF–Væ6R×·Væ—VWÒ"’À¢V&Æ–5÷&öf–ÆU÷W&Ã¢6öÖR†f÷&ÖB‚&‡GG3¢òöv—F‡V"æ6öÒöVF–Væ6R×·Væ—VWÒ"’’À¢&öÆW3¢fV2µÒÀ¢ö'6W'fVEöC¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWB7FÆU÷7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢ÆWB7FÆUöæWGv÷&²Ò‡–G&FUöæWGv÷&²‚g7FÆU÷7F÷&R’æv—BçVçw&‚“°¢ÆWB7FÆU÷7FFRÐ¢FW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R‡7FÆUöæWGv÷&²Â'6V7&WB×Fö¶Vâ"Â7FÆU÷7F÷&R“°¢ÆWBWfVçEö–BÒf÷&ÖB‚'VÆÂ×&WVW7C§·Væ—VWÒ"“°¢ÆWB–çFW&7F–öâÒ&V6÷&EöVF–Væ6Uö–çFW&7F–öâ€¢7FFR†f—'7E÷7FFR’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…&V6÷&DVF–Væ6T–çFW&7F–öå&WVW7B°¢VF–Væ6UöÖVÖ&W%ö–C¢ÖVÖ&W"æ–BÀ¢&÷f–FW%öWfVçEö–C¢WfVçEö–Bæ6ÆöæR‚’À¢¶–æC¢FöÖ–ã£¤VF–Væ6T–çFW&7F–öä¶–æC£¥VÆÅ&WVW7D÷VæVBÀ¢V&Æ–5÷W&Ã¢6öÖR†f÷&ÖB€¢&‡GG3¢òöv—F‡V"æ6öÒôå5s2övVçBÖ&÷VçF–W2÷VÆÂ÷·Væ—VWÒ ¢’’À¢ö67W'&VEöC¢æöæRÀ¢&VfW'&W%÷W&Ã¢æöæRÀ¢6×–vã¢6öÖR‚'÷7Fw&W2ÖVF–Væ6R×FW7B"çFõ÷7G&–ær‚’’À¢6÷W&6Uö–çFW&7F–öåö–C¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWB7F"Ò&V6÷&EöVF–Væ6Uö–çFW&7F–öâ€¢7FFR‡7FÆU÷7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…&V6÷&DVF–Væ6T–çFW&7F–öå&WVW7B°¢VF–Væ6UöÖVÖ&W%ö–C¢ÖVÖ&W"æ–BÀ¢&÷f–FW%öWfVçEö–C¢f÷&ÖB‚'7F#§·Væ—VWÒ"’À¢¶–æC¢FöÖ–ã£¤VF–Væ6T–çFW&7F–öä¶–æC£¥&Wõ7F'&VBÀ¢V&Æ–5÷W&Ã¢6öÖR‚&‡GG3¢òöv—F‡V"æ6öÒôå5s2övVçBÖ&÷VçF–W2÷7F&v¦W'2"çFõ÷7G&–ær‚’’À¢ö67W'&VEöC¢æöæRÀ¢&VfW'&W%÷W&Ã¢æöæRÀ¢6×–vã¢6öÖR‚'÷7Fw&W2ÖVF–Væ6R×FW7B"çFõ÷7G&–ær‚’’À¢6÷W&6Uö–çFW&7F–öåö–C¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢76W'EöæR‡7F"æ–BÂ–çFW&7F–öâæ–B“°¢ÆWB6öæfÆ–7F–æu÷&WÆ’Ò&V6÷&EöVF–Væ6Uö–çFW&7F–öâ€¢7FFR‡7FÆU÷7FFR’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…&V6÷&DVF–Væ6T–çFW&7F–öå&WVW7B°¢VF–Væ6UöÖVÖ&W%ö–C¢ÖVÖ&W"æ–BÀ¢&÷f–FW%öWfVçEö–C¢WfVçEö–Bæ6ÆöæR‚’À¢¶–æC¢FöÖ–ã£¤VF–Væ6T–çFW&7F–öä¶–æC£¤gVæF–æu6–væÆVBÀ¢V&Æ–5÷W&Ã¢–çFW&7F–öâçV&Æ–5÷W&Âæ6ÆöæR‚’À¢ö67W'&VEöC¢6öÖR†–çFW&7F–öâæö67W'&VEöB’À¢&VfW'&W%÷W&Ã¢æöæRÀ¢6×–vã¢6öÖR‚'÷7Fw&W2ÖVF–Væ6R×FW7B"çFõ÷7G&–ær‚’’À¢6÷W&6Uö–çFW&7F–öåö–C¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†6öæfÆ–7F–æu÷&WÆ’Â7FGW46öFS£¤4ôädÄ”5B“° ¢ÆWB6V6öæE÷7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢ÆWB6V6öæEöæWGv÷&²Ò‡–G&FUöæWGv÷&²‚g6V6öæE÷7F÷&R’æv—BçVçw&‚“°¢ÆWB6V6öæE÷7FFRÐ¢FW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R‡6V6öæEöæWGv÷&²Â'6V7&WB×Fö¶Vâ"Â6V6öæE÷7F÷&R“°¢ÆWB&WÆ’Ò&V6÷&EöVF–Væ6Uö–çFW&7F–öâ€¢7FFR‡6V6öæE÷7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…&V6÷&DVF–Væ6T–çFW&7F–öå&WVW7B°¢VF–Væ6UöÖVÖ&W%ö–C¢ÖVÖ&W"æ–BÀ¢&÷f–FW%öWfVçEö–C¢WfVçEö–BÀ¢¶–æC¢FöÖ–ã£¤VF–Væ6T–çFW&7F–öä¶–æC£¥VÆÅ&WVW7D÷VæVBÀ¢V&Æ–5÷W&Ã¢–çFW&7F–öâçV&Æ–5÷W&Âæ6ÆöæR‚’À¢ö67W'&VEöC¢6öÖR†–çFW&7F–öâæö67W'&VEöB’À¢&VfW'&W%÷W&Ã¢æöæRÀ¢6×–vã¢6öÖR‚'÷7Fw&W2ÖVF–Væ6R×FW7B"çFõ÷7G&–ær‚’’À¢6÷W&6Uö–çFW&7F–öåö–C¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡&WÆ’æ–BÂ–çFW&7F–öâæ–B“° ¢ÆWBW'6—7FVBÒÆ—7EöVF–Væ6Uö–çFW&7F–öç2…7FFR‡6V6öæE÷7FFRæ6ÆöæR‚’’Â†VFW'2æ6ÆöæR‚’¢æv—@¢çVçw&‚¢ã ¢æ–çFõö—FW"‚¢æf–ÇFW"‡Æ6æF–FFWÂ6æF–FFRæVF–Væ6UöÖVÖ&W%ö–BÓÒÖVÖ&W"æ–B¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢76W'EöW‡W'6—7FVBæÆVâ‚’Â"“°¢ÆWBW'6—7FVEöÖVÖ&W"ÒÆ—7EöVF–Væ6UöÖVÖ&W'2…7FFR‡6V6öæE÷7FFRæ6ÆöæR‚’’Â†VFW'2æ6ÆöæR‚’¢æv—@¢çVçw&‚¢ã ¢æ–çFõö—FW"‚¢æf–æB‡Æ6æF–FFWÂ6æF–FFRæ–BÓÒÖVÖ&W"æ–B¢çVçw&‚“°¢76W'B‡W'6—7FVEöÖVÖ&W ¢ç&öÆW0¢æ6öçF–ç2‚fFöÖ–ã£¤VF–Væ6U&öÆS£¤6öçG&–'WF÷"’“°¢76W'B‡W'6—7FVEöÖVÖ&W ¢ç&öÆW0¢æ6öçF–ç2‚fFöÖ–ã£¤VF–Væ6U&öÆS£¥&öÖ÷FW"’“°¢76W'EöW€¢W'6—7FVEöÖVÖ&W"æÆ–fV7–6ÆU÷7FvRÀ¢FöÖ–ã£¤VF–Væ6TÆ–fV7–6ÆU7FvS£¥&WF–æV@¢“°¢ÆWB&W÷'BÒVF–Væ6U÷&W÷'B…7FFR‡6V6öæE÷7FFR’Â†VFW'2¢æv—@¢çVçw&‚¢ã°¢76W'B‡&W÷'BçF÷FÅöÖVÖ&W'2ãÒ“°¢76W'B‡&W÷'BçF÷FÅö–çFW&7F–öç2ãÒ“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢5¶–væ÷&RÒ'&WV—&W2tTåEô$õTåD”U5õDU5EôDD$4UõU$Â%Ð¢7–æ2fâv—F‡V%ö—77VUö•÷7–æ5÷÷7Fw&W5÷&V¦V7G5÷7FÆUö7&÷75÷&ö6W75ö7F—f—G’‚’°¢ÆWBFF&6U÷W&ÂÒ÷7Fw&W5÷FW7EöFF&6U÷W&Â‚“°¢ÆWB7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢7F÷&RæÖ–w&FR‚’æv—BçVçw&‚“°¢ÆWB7–æ5÷7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R€¢&÷VçG”æWGv÷&³£¦FVfVÇB‚’À¢'6V7&WB×Fö¶Vâ"À¢7F÷&Ræ6ÆöæR‚’À¢“°¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B„õU$Dõ%õDô´Tåô„TDU"Â'6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWB—77VUöçVÖ&W"Ò…WV–C£¦æWu÷cB‚’æ5÷S#‚‚’Róóóó’2ScB²° ¢ÆWBf—'7BÒ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7–æ5÷7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢—77VUöçVÖ&W"À¢%¶&÷VçG•Ó¢7–æ2v—D‡V"—77VR–çFò’"À¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢’’À¢¢æv—@¢çVçw&‚¢ã° ¢ÆWBVF—FVBÒ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7–æ5÷7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢—77VUöçVÖ&W"À¢%¶&÷VçG•Ó¢7–æ2†÷7FVBv—D‡V"—77VR&÷VçG’&V6÷&G2"À¢fÆ–Eöv—F‡V%ö—77VUö&öG•÷v—F…övöÂ€¢$¶VWF†R6ÖR†÷7FVB&÷VçG’gFW"â—77VRF—FÆRVF—Bâ"À¢’À¢’’À¢¢æv—@¢çVçw&‚¢ã°¢76W'EöW†VF—FVBæ–BÂf—'7Bæ–B“°¢76W'EöW€¢VF—FVBçF—FÆRÀ¢%¶&÷VçG•Ó¢7–æ2†÷7FVBv—D‡V"—77VR&÷VçG’&V6÷&G2 ¢“° ¢ÆWBÖ7÷7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢ÆWBÖ7öæWGv÷&²Ò‡–G&FUöæWGv÷&²‚fÖ7÷7F÷&R’æv—BçVçw&‚“°¢76W'B†Ö7öæWGv÷&²æ&÷VçF–W2æ6öçF–ç5ö¶W’‚ff—'7Bæ–B’“°¢ÆWBÖ7÷7FFRÐ¢FW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R†Ö7öæWGv÷&²Â'6V7&WB×Fö¶Vâ"ÂÖ7÷7F÷&R“°¢ÆWBgVæF–æu÷&W÷'BÒ7&VFUögVæF–æuö–çFVçB€¢7FFR†Ö7÷7FFR’À¢F‚†f—'7Bæ–B’À¢§6öâ„7&VFTgVæF–æt–çFVçE&WVW7B°¢&÷VçG•ö–C¢f—'7Bæ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢6öÖR…WV–C£¦æWu÷cB‚’’À¢Ö÷VçEöÖ–æ÷#¢f—'7BæÖ÷VçBæÖ÷VçBÀ¢7W'&Væ7“¢f—'7BæÖ÷VçBæ7W'&Væ7’æ6ÆöæR‚’À¢&–Ã¢–ÖVçE&–Ã£¥7G&—Tf–BÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR†f÷&ÖB‚'7FÆR×7–æ2×¶—77VUöçVÖ&W'Ò"’’À¢7G&—U÷7V66W75÷W&Ã¢æöæRÀ¢7G&—Uö6æ6VÅ÷W&Ã¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚“°¢76W'EöW†gVæF–æu÷&W÷'Bãæ–çFVçBæ&÷VçG•ö–BÂf—'7Bæ–B“°¢76W'EöW€¢gVæF–æu÷&W÷'Bãæ–çFVçBç7FGW2À¢gVæF–æt–çFVçE7FGW3£¤v—F–ætWf–FVæ6P¢“° ¢ÆWB&V¦V7FVBÒ7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡7–æ5÷7FFRæ6ÆöæR‚’’À¢†VFW'2À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢—77VUöçVÖ&W"À¢%¶&÷VçG•Ó¢Vç6fRVF—BgFW"gVæF–ær7F—f—G’"À¢fÆ–Eöv—F‡V%ö—77VUö&öG•÷v—F…övöÂ€¢%F†—27FÆR’&ö6W72×W7Bæ÷B÷fW'w&—FRgVæFVB&÷râ"À¢’À¢’’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW‡&V¦V7FVBÂ7FGW46öFS£¤$Eõ$UTU5B“° ¢ÆWBW'6—7FVBÒ7F÷&P¢æÆ—7Eö&÷VçF–W2‚¢æv—@¢çVçw&‚¢æ–çFõö—FW"‚¢æf–æB‡Æ&÷VçG—Â&÷VçG’æ–BÓÒf—'7Bæ–B¢çVçw&‚“°¢76W'EöW‡W'6—7FVBçF—FÆRÂVF—FVBçF—FÆR“°¢76W'EöW‡W'6—7FVBæÖ÷VçBæÖ÷VçBÂVF—FVBæÖ÷VçBæÖ÷VçB“°¢76W'EöW‡W'6—7FVBç7FGW2Â&÷VçG•7FGW3£¥VægVæFVB“° ¢ÆWBgVæF–æuö–çFVçG2Ò7F÷&P¢æÆ—7EögVæF–æuö–çFVçG2‚¢æv—@¢çVçw&‚¢æ–çFõö—FW"‚¢æf–ÇFW"‡Æ–çFVçGÂ–çFVçBæ&÷VçG•ö–BÓÒf—'7Bæ–B¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢76W'EöW†gVæF–æuö–çFVçG2æÆVâ‚’Â“° ¢ÆWB7FÆU÷7FGW2Ò&÷VçG•÷7FGW2…7FFR‡7–æ5÷7FFR’ÂF‚†f—'7Bæ–B’¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡7FÆU÷7FGW2æ&÷VçG’çF—FÆRÂVF—FVBçF—FÆR“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢5¶–væ÷&RÒ'&WV—&W2tTåEô$õTåD”U5õDU5EôDD$4UõU$Â%Ð¢7–æ2fâv—F‡V%ö—77VUö•÷7–æ5÷÷7Fw&W5÷6W&–Æ—¦W5ö6öæ7W'&VçEö–æ—F–Å÷7–æ2‚’°¢ÆWBFF&6U÷W&ÂÒ÷7Fw&W5÷FW7EöFF&6U÷W&Â‚“°¢ÆWBf—'7E÷7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢f—'7E÷7F÷&RæÖ–w&FR‚’æv—BçVçw&‚“°¢ÆWB6V6öæE÷7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢ÆWBf—'7E÷7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R€¢&÷VçG”æWGv÷&³£¦FVfVÇB‚’À¢'6V7&WB×Fö¶Vâ"À¢f—'7E÷7F÷&Ræ6ÆöæR‚’À¢“°¢ÆWB6V6öæE÷7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R€¢&÷VçG”æWGv÷&³£¦FVfVÇB‚’À¢'6V7&WB×Fö¶Vâ"À¢6V6öæE÷7F÷&RÀ¢“°¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B„õU$Dõ%õDô´Tåô„TDU"Â'6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWB—77VUöçVÖ&W"Ò…WV–C£¦æWu÷cB‚’æ5÷S#‚‚’Róóóó’2ScB²° ¢ÆWBf—'7E÷7–æ2Ò7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR†f—'7E÷7FFR’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢—77VUöçVÖ&W"À¢%¶&÷VçG•Ó¢7–æ2v—D‡V"—77VR–çFò’"À¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢’’À¢“°¢ÆWB6V6öæE÷7–æ2Ò7–æ5öv—F‡V%ö—77VUö•ö&÷VçG’€¢7FFR‡6V6öæE÷7FFR’À¢†VFW'2À¢§6öâ†v—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢—77VUöçVÖ&W"À¢%¶&÷VçG•Ó¢7–æ2v—D‡V"—77VR–çFò’"À¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢’’À¢“°¢ÆWB†f—'7E÷&W7VÇBÂ6V6öæE÷&W7VÇB’ÒFö¶–ó£¦¦ö–â†f—'7E÷7–æ2Â6V6öæE÷7–æ2“°¢ÆWBf—'7BÒf—'7E÷&W7VÇBçVçw&‚’ã°¢ÆWB6V6öæBÒ6V6öæE÷&W7VÇBçVçw&‚’ã° ¢76W'EöW†f—'7Bæ–BÂ6V6öæBæ–B“°¢76W'EöW†f—'7BçF—FÆRÂ%¶&÷VçG•Ó¢7–æ2v—D‡V"—77VR–çFò’"“°¢76W'EöW‡6V6öæBçF—FÆRÂ%¶&÷VçG•Ó¢7–æ2v—D‡V"—77VR–çFò’"“° ¢ÆWBÖF6†–æuö&÷VçF–W2Òf—'7E÷7F÷&P¢æÆ—7Eö&÷VçF–W2‚¢æv—@¢çVçw&‚¢æ–çFõö—FW"‚¢æf–ÇFW"‡Æ&÷VçG—Â&÷VçG’æ–BÓÒf—'7Bæ–B¢æ6öÆÆV7C££ÅfV3Åóãâ‚“°¢76W'EöW†ÖF6†–æuö&÷VçF–W2æÆVâ‚’Â“°¢76W'EöW†ÖF6†–æuö&÷VçF–W5³Òç7FGW2Â&÷VçG•7FGW3£¥VægVæFVB“°¢76W'EöW†ÖF6†–æuö&÷VçF–W5³ÒæÖ÷VçBæÖ÷VçBÂf—'7BæÖ÷VçBæÖ÷VçB“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ö7&VFUö6öÖÖVçE÷Æå÷&WGW&ç5÷&Wf–WuööæÇ•÷vÆÆWEö†æFöfb‚’°¢ÆWBÆâÒÆåöv—F‡V%ö7&VFUö6öÖÖVçB„§6öâ…Æäv—D‡V$7&VFT6öÖÖVçE&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2óS"çFõ÷7G&–ær‚’À¢F—FÆS¢$f—‚6æöæ–6Â&V6V—B&V6öæ6–Æ–F–öâ"çFõ÷7G&–ær‚’À¢&öG“¢%F†R&V6V—Bv÷&¶W"G&÷26öæf—&ÖVBÆörgFW"&W7F'Bâ"çFõ÷7G&–ær‚’À¢6öÖÖVçEö&öG“¢"övVçBÖ&÷VçG’7&VFR#RU4D2"çFõ÷7G&–ær‚’À¢6öçG&–'WF÷%öÆöv–ã¢6öÖR‚&Ö–çF–æW""çFõ÷7G&–ær‚’’À¢6öÖÖVçEö–C¢6öÖR‚#“"çFõ÷7G&–ær‚’’À¢W†—7F–æuö–FV×÷FVæ7•ö¶W—3¢fV2µÒÀ¢Ò’¢æv—@¢ã° ¢76W'B‡Æâç&VG’“°¢ÆWB6–væÂÒÆâç6–væÂæW‡V7B‚&7&VFR6–væÂ"“°¢76W'EöW‡6–væÂæG&gBç7FFRÂ'&Wf–Wu÷&WV—&VEöæ÷E÷V&Æ—6†VB"“°¢76W'B‡6–væÂæG&gBæ66WFæ6Uö7&—FW&–æ—5öV×G’‚’“°¢76W'B‡6–væÂæG&gBæG&gEö†æFöfe÷W&Âæ6öçF–ç2‚&g&öÓÖv—F‡V"Ö—77VR"’“°¢76W'B‚6–væÂæG&gBæ&÷VçG•ö7&VFVB“°¢76W'B‚6–væÂæG&gBæ6æöæ–6ÅögVæF–æuö6öæf—&ÖVB“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¥7V66W72“°¢Ð ¢5·FW7EÐ¢fâ6ö6–Å÷&öÆÆ÷WEö6÷VçG5ö6æöæ–6ÅöWfVçG5÷v—F…öv—F‡V%ö—77VU÷&÷fVææ6R‚’°¢ÆWBæ÷rÒWF3£¦æ÷r‚“°¢ÆWB×WBFö7VÖVçC¢WFöæöÖ÷W4&÷VçG•FW&×4Fö7VÖVçBÐ¢6W&FUö§6öã£¦g&öÕ÷7G"†–æ6ÇVFU÷7G"‚"ââòââòââö&÷VçF–W2öWFöæöÖ÷W2×có#CBæ§6öâ"’’çVçw&‚“°¢Fö7VÖVçBç6÷W&6U÷W&ÂÐ¢6öÖR‚&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2óS"çFõ÷7G&–ær‚’“°¢Fö7VÖVçBæF—66÷fW'•÷6÷W&6RÒ6öÖR‚$v—D‡V"övVçBÖ&÷VçG’7&VFR"çFõ÷7G&–ær‚’“°¢ÆWBFW&×2ÒWFöæöÖ÷W4&÷VçG•FW&×5&V6÷&B°¢FW&×5ö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#CB"ç&WVBƒ3"’’À¢öÆ–7•ö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#CR"ç&WVBƒ3"’’À¢66WFæ6Uö7&—FW&–ö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#Cb"ç&WVBƒ3"’’À¢&Væ6†Ö&µö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#Cr"ç&WVBƒ3"’’À¢Wf–FVæ6U÷66†VÖö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#C‚"ç&WVBƒ3"’’À¢7&VF÷%÷vÆÆWC¢f÷&ÖB‚#‡·Ò"Â#32"ç&WVBƒ#’’À¢Fö7VÖVçBÀ¢7&VFVEöC¢æ÷rÀ¢Ó°¢ÆWBWfVçBÒÆ¶–æBÂÆöuö–æFW‡ÂWFöæöÖ÷W4&÷VçG”WfVçB°¢–C¢WV–C£¦æWu÷cB‚’À¢Æöuö¶W“¢f÷&ÖB‚&&6RÖÖ–ææWC£S§¶Æöuö–æFW‡Ò"’À¢G…ö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#SR"ç&WVBƒ3"’’À¢&Æö6µöçVÖ&W#¢SÀ¢Æöuö–æFW‚À¢6öçG&7EöFG&W73¢f÷&ÖB‚#‡·Ò"Â##""ç&WVBƒ#’’À¢&÷VçG•ö–C¢f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’’À¢¶–æBÀ¢FF¢6W&FUö§6öã£¦§6öâ‡·Ò’À¢ö67W'&VEöC¢æ÷rÀ¢Ó°¢ÆWB—FVÒÒWFöæöÖ÷W4&÷VçG”fVVD—FVÒ°¢&÷VçG•ö–C¢f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’’À¢&÷VçG•ö6öçG&7C¢f÷&ÖB‚#‡·Ò"Â##""ç&WVBƒ#’’À¢7&VF÷#¢f÷&ÖB‚#‡·Ò"Â#32"ç&WVBƒ#’’À¢7FGW3¢'6WGFÆVB"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&C¢##"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&C¢#"çFõ÷7G&–ær‚’À¢6Æ–Õö&öæC¢#"çFõ÷7G&–ær‚’À¢F–ÖV÷WEö&öæE÷ööÃ¢#"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçC¢##"çFõ÷7G&–ær‚’À¢gVæFVEöÖ÷VçC¢##"çFõ÷7G&–ær‚’À¢&WV—&VEöW‡FW&æÅ÷7VæC¢#"çFõ÷7G&–ær‚’À¢w&÷75ö66…öÖ&v–ã¢##"çFõ÷7G&–ær‚’À¢FW&×5ö†6ƒ¢FW&×2çFW&×5ö†6‚æ6ÆöæR‚’À¢FW&×3¢6öÖR‡FW&×2’À¢FW&×5÷fÆ–C¢G'VRÀ¢fW&–f–6F–öåöÖöFS¢'6–væVE÷V÷'VÒ"çFõ÷7G&–ær‚’À¢fW&–f–W%öÖöGVÆS¢æöæRÀ¢fW&–f–W%÷6WEö†6ƒ¢æöæRÀ¢fW&–f–W%÷F‡&W6†öÆC¢6öÖRƒ"’À¢'VææW%ö–FVçF–f–W#¢6öÖR‚'6æF&÷†VE÷&Vw&W76–öå÷c"çFõ÷7G&–ær‚’’À¢fW&–f–6F–öå÷&VG“¢G'VRÀ¢fW&–f–6F–öå÷&VF–æW75÷&V6öã¢'&VG’"çFõ÷7G&–ær‚’À¢fÆ–FF–öåöW'&÷'3¢fV2µÒÀ¢WfVçG3¢fV2°¢WfVçB„WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG”&V6ÖT6Æ–Ö&ÆRÂ’À¢WfVçB„WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG•6WGFÆVBÂ"’À¢ÒÀ¢Ó° ¢ÆWB×WBÆVv7•öv—F‡V%ö—FVÒÒ—FVÒæ6ÆöæR‚“°¢ÆVv7•öv—F‡V%ö—FVÐ¢çFW&×0¢æ5ö×WB‚¢çVçw&‚¢æFö7VÖVç@¢æF—66÷fW'•÷6÷W&6RÒ6öÖR‚&ÖçVÂv—D‡V"Æ–æ²"çFõ÷7G&–ær‚’“° ¢ÆWB×WB–væ÷&VEöæöåöv—F‡V%ö—FVÒÒ—FVÒæ6ÆöæR‚“°¢–væ÷&VEöæöåöv—F‡V%ö—FVÐ¢çFW&×0¢æ5ö×WB‚¢çVçw&‚¢æFö7VÖVç@¢ç6÷W&6U÷W&ÂÒ6öÖR‚&‡GG3¢òöW†×ÆRæ6öÒ÷F6·2óS"çFõ÷7G&–ær‚’“° ¢ÆWBWf–FVæ6RÐ¢v—F‡V%ö—77VUö6öçfW'6–öåöWf–FVæ6R‚e¶—FVÒÂÆVv7•öv—F‡V%ö—FVÒÂ–væ÷&VEöæöåöv—F‡V%ö—FVÕÒ“°¢76W'B†Wf–FVæ6RæWf–FVæ6Uöf–Æ&ÆR“°¢76W'EöW†Wf–FVæ6Ræv—F‡V%ö÷&–v–æFVEö6æöæ–6ÅögVæFVBÂ"“°¢76W'EöW†Wf–FVæ6Ræv—F‡V%ö÷&–v–æFVEö6æöæ–6Å÷6WGFÆVBÂ"“°¢Ð ¢5·FW7EÐ¢fâæW–æ%÷6–væGW&U÷W6W5÷&uö&öG•ö†Ö5÷6†S%öæE÷&V¦V7G5÷F×W&–ær‚’°¢ÆWB6V7&WBÒ"&æW–æ"×vV&†öö²×6V7&WB#°¢ÆWB&öG’Ò'"2'²'G—R#¢&67Bæ7&VFVB"Â&FF#§²&†6‚#¢#ƒC#C"'×Ò"3°¢ÆWB×WBÖ2Ò†Ö3££Ç6†#£¥6†S#ã£¦æWuög&öÕ÷6Æ–6R‡6V7&WB’çVçw&‚“°¢Ö2çWFFR†&öG’“°¢ÆWB6–væGW&RÒ†Wƒ£¦Væ6öFR†Ö2æf–æÆ—¦R‚’æ–çFõö'—FW2‚’“° ¢76W'B‡fW&–g•öæW–æ%÷6–væGW&R†&öG’Âg6–væGW&RÂ6V7&WB’“°¢76W'B‚fW&–g•öæW–æ%÷6–væGW&R€¢'"2'²'G—R#¢&67Bæ7&VFVB"Â&FF#§²&†6‚#¢#ƒ#C#B'×Ò"2À¢g6–væGW&RÀ¢6V7&W@¢’“°¢76W'B‚fW&–g•öæW–æ%÷6–væGW&R†&öG’Â&æ÷BÖ†W‚"Â6V7&WB’“°¢Ð ¢5·FW7EÐ¢fâf&67FW%ö&÷EöÖVçF–öåöÖF6†–æu÷&W7V7G5÷Fö¶Våö&÷VæF&–W2‚’°¢76W'B‡FW‡EöÖVçF–öç5ö&÷B€¢$&÷VçG–&ö&BövVçBÖ&÷VçG’7&VFR#RU4D26†——B"À¢&&÷VçG–&ö&B ¢’“°¢76W'B‡FW‡EöÖVçF–öç5ö&÷B€¢%ÆV6R6²„&÷VçG–&ö&B’ÂF†Vâ7&VFRF†RG&gB"À¢&&÷VçG–&ö&B ¢’“°¢76W'B‚FW‡EöÖVçF–öç5ö&÷B€¢$&÷VçG–&ö&B×66ÒövVçBÖ&÷VçG’7&VFR#RU4D2"À¢&&÷VçG–&ö&B ¢’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢5¶–væ÷&RÒ'&WV—&W2tTåEô$õTåD”U5õDU5EôDD$4UõU$Â%Ð¢7–æ2fâæW–æ%÷vV&†ööµ÷W'6—7G5ööæU÷6†÷'EöG&gEöæEööæU÷&WÇ•ö7&÷75÷&WG&–W2‚’°¢ÆWBFF&6U÷W&ÂÒ÷7Fw&W5÷FW7EöFF&6U÷W&Â‚“°¢ÆWB7F÷&RÒ÷7Fw&W57F÷&S£¦6öææV7B‚fFF&6U÷W&Â’æv—BçVçw&‚“°¢7F÷&RæÖ–w&FR‚’æv—BçVçw&‚“°¢6VVE÷6ö6–Å÷&öÆÆ÷WEöWf–FVæ6R‚g7F÷&R’æv—C° ¢ÆWB&WÇ•ö†6‚Òf÷&ÖB‚#‡·Ò"Â##B"ç&WVBƒ#’“°¢ÆWBæW–æ%ö•ö&6RÒ7vå÷'5÷&W7öç6R‡6W&FUö§6öã£¦§6öâ‡°¢&67B#¢²&†6‚#¢&WÇ•ö†6‡Ð¢Ò’“°¢ÆWBvV&†ööµ÷6V7&WBÒ"&æW–æ"×vV&†öö²×6V7&WB#°¢ÆWB×WB7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R€¢&÷VçG”æWGv÷&³£¦FVfVÇB‚’À¢'VçW6VBÖ÷W&F÷"×Fö¶Vâ"À¢7F÷&RÀ¢“°¢&3£¦vWEö×WB‚f×WB7FFR’çVçw&‚’ææW–æ%÷6ö6–ÂÐ¢6öÖR„&3£¦æWr„æW–æ%6ö6–Ä–ævW7F–öä6öæf–r°¢vV&†ööµ÷6V7&WC¢vV&†ööµ÷6V7&WBçFõ÷fV2‚’À¢&÷Eöf–C¢%ó3CRÀ¢&÷E÷W6W&æÖS¢&&÷VçG–&ö&B"çFõ÷7G&–ær‚’À¢•ö¶W“¢6öÖR‚'FW7BÖæW–æ"Ö¶W’"çFõ÷7G&–ær‚’’À¢6–væW%÷WV–C¢6öÖR‚##6SCScrÖSƒ–"ÓC&C2ÖCSbÓC#ccCsC"çFõ÷7G&–ær‚’’À¢•ö&6U÷W&Ã¢æW–æ%ö•ö&6RÀ¢vV'6—FUö&6U÷W&Ã¢&‡GG3¢òövVçF&÷VçF–W2æ"çFõ÷7G&–ær‚’À¢6Æ–VçC¢&WvW7C£¤6Æ–VçC£¦æWr‚’À¢Ò’“° ¢ÆWB67Eö†6‚Òf÷&ÖB‚#‡³£C‡Ò"ÂWV–C£¦æWu÷cB‚’æ5÷S#‚‚’“°¢ÆWB&öG’Ò6W&FUö§6öã£¦§6öâ‡°¢&7&VFVEöB#¢WF3£¦æ÷r‚’çF–ÖW7F×‚’À¢'G—R#¢&67Bæ7&VFVB"À¢&FF#¢°¢&ö&¦V7B#¢&67B"À¢&†6‚#¢67Eö†6‚À¢&WF†÷"#¢²&f–B#¢C"Â'W6W&æÖR#¢'&WVW7FW"'ÒÀ¢'FW‡B#¢$&÷VçG–&ö&EÆâövVçBÖ&÷VçG’7&VFR#RU4D5Ææ–×ÆVÖVçBFWFW&Ö–æ—7F–2&WG&–W2"À¢&ÖVçF–öæVE÷&öf–ÆW2#¢·²&f–B#¢#3CWÕÐ¢Ð¢Ò¢çFõ÷7G&–ær‚“°¢ÆWB×WBÖ2Ò†Ö3££Ç6†#£¥6†S#ã£¦æWuög&öÕ÷6Æ–6R‡vV&†ööµ÷6V7&WB’çVçw&‚“°¢Ö2çWFFR†&öG’æ5ö'—FW2‚’“°¢ÆWB6–væGW&RÒ†Wƒ£¦Væ6öFR†Ö2æf–æÆ—¦R‚’æ–çFõö'—FW2‚’“°¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B‚'‚ÖæW–æ"×6–væGW&R"Â6–væGW&Rç'6R‚’çVçw&‚’“°¢ÆWB&Wf–÷W5övFRÒ7FC£¦Vçc£§f"‚$tTåEô$õTåD”U5õ4ô4”ÅôÔTåD”ôåôE$eE5ôTä$ÄTB"’æö²‚“°¢7FC£¦Vçc£§6WE÷f"‚$tTåEô$õTåD”U5õ4ô4”ÅôÔTåD”ôåôE$eE5ôTä$ÄTB"Â'G'VR"“° ¢ÆWB†f—'7E÷7FGW2Â§6öâ†f—'7B’’Ò–ævW7EöæW–æ%÷6ö6–ÅöÖVçF–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢'—FW3£¦g&öÒ†&öG’æ6ÆöæR‚’’À¢¢æv—C°¢76W'EöW€¢f—'7E÷7FGW2À¢7FGW46öFS£¤ô²À¢'VæW‡V7FVBvV&†öö²&W7VÇC¢·Ò"À¢6W&FUö§6öã£§Fõ÷7G&–ær‚ff—'7B’çVçw&‚¢“°¢76W'B†f—'7Bæ66WFVB“°¢76W'B‚f—'7BæGWÆ–6FR“°¢76W'EöW†f—'7Bç7FGW2Â'&WÆ–VB"“°¢76W'EöW†f—'7Bç&WÇ•ö67Eö†6‚æ5öFW&Vb‚’Â6öÖR‡&WÇ•ö†6‚æ5÷7G"‚’’“°¢ÆWB–ævW7F–öåö–BÒf—'7Bæ–ævW7F–öåö–BçVçw&‚“°¢ÆWB†æFöfbÒf—'7BæG&gEö†æFöfe÷W&ÂçVçw&‚“°¢76W'EöW€¢†æFöfbÀ¢f÷&ÖB€¢&‡GG3¢òövVçF&÷VçF–W2æóög&öÓ×6ö6–ÂÖÖVçF–öâg6ö6–ÄG&gC×¶–ævW7F–öåö–GÒ7÷7BÖÖ&÷VçG’ ¢¢“° ¢ÆWB‡&WÆ•÷7FGW2Â§6öâ‡&WÆ’’’Ð¢–ævW7EöæW–æ%÷6ö6–ÅöÖVçF–öâ…7FFR‡7FFRæ6ÆöæR‚’’Â†VFW'2Â'—FW3£¦g&öÒ†&öG’’’æv—C°¢76W'EöW‡&WÆ•÷7FGW2Â7FGW46öFS£¤ô²“°¢76W'B‡&WÆ’æ66WFVB“°¢76W'B‡&WÆ’æGWÆ–6FR“°¢76W'EöW‡&WÆ’æ–ævW7F–öåö–BÂ6öÖR†–ævW7F–öåö–B’“°¢76W'EöW‡&WÆ’ç&WÇ•ö67Eö†6‚æ5öFW&Vb‚’Â6öÖR‡&WÇ•ö†6‚æ5÷7G"‚’’“° ¢ÆWBG&gBÒvWE÷6ö6–ÅöÖVçF–öåöG&gB…7FFR‡7FFR’ÂF‚†–ævW7F–öåö–B’¢æv—@¢çVçw&‚¢ã°¢76W'EöW†G&gBç7FGW2Â'&WÆ–VB"“°¢76W'EöW†G&gBæG&gE²'7FFR%ÒÂ'&Wf–Wu÷&WV—&VEöæ÷E÷V&Æ—6†VB"“°¢76W'EöW†G&gBæG&gE²&G&gEö†æFöfe÷W&Â%ÒÂ†æFöfb“°¢76W'EöW†G&gBæG&gE²&&÷VçG•ö7&VFVB%ÒÂfÇ6R“°¢76W'EöW†G&gBæG&gE²&6æöæ–6ÅögVæF–æuö6öæf—&ÖVB%ÒÂfÇ6R“° ¢–bÆWB6öÖR‡fÇVR’Ò&Wf–÷W5övFR°¢7FC£¦Vçc£§6WE÷f"‚$tTåEô$õTåD”U5õ4ô4”ÅôÔTåD”ôåôE$eE5ôTä$ÄTB"ÂfÇVR“°¢ÒVÇ6R°¢7FC£¦Vçc£§&VÖ÷fU÷f"‚$tTåEô$õTåD”U5õ4ô4”ÅôÔTåD”ôåôE$eE5ôTä$ÄTB"“°¢Ð¢Ð ¢7–æ2fâ6VVE÷6ö6–Å÷&öÆÆ÷WEöWf–FVæ6R‡7F÷&S¢e÷7Fw&W57F÷&R’°¢ÆWBæ÷rÒWF3£¦æ÷r‚“°¢f÷"–æFW‚–â÷ScBâã2°¢ÆWB×WBFö7VÖVçC¢WFöæöÖ÷W4&÷VçG•FW&×4Fö7VÖVçBÐ¢6W&FUö§6öã£¦g&öÕ÷7G"†–æ6ÇVFU÷7G"‚"ââòââòââö&÷VçF–W2öWFöæöÖ÷W2×có#CBæ§6öâ"’¢çVçw&‚“°¢Fö7VÖVçBç6÷W&6U÷W&ÂÒ6öÖR†f÷&ÖB€¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2÷·Ò"À¢WV–C£¦æWu÷cB‚’æ5÷S#‚‚¢’“°¢ÆWB&V6÷&BÒ'V–ÆEöWFöæöÖ÷W5ö&÷VçG•÷FW&×5÷&V6÷&B€¢#ƒƒƒCƒ3DSƒƒFCdS“3Cc#cST#ƒ#CC4ScsCv$2"À¢Fö7VÖVçBÀ¢æ÷rÀ¢¢çVçw&‚“°¢7F÷&RçW6W'EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2‚g&V6÷&B’æv—BçVçw&‚“°¢ÆWB&÷VçG•ö–BÒf÷&ÖB‚#‡³£cG‡Ò"ÂWV–C£¦æWu÷cB‚’æ5÷S#‚‚’“°¢ÆWB&÷VçG•ö6öçG&7BÒf÷&ÖB‚#‡³£C‡Ò"ÂWV–C£¦æWu÷cB‚’æ5÷S#‚‚’“°¢ÆWB6öçG&7E÷FW&×2Òg&V6÷&BæFö7VÖVçBæ6öçG&7E÷FW&×3°¢ÆWBWfVçG2Ò°¢€¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤6æöæ–6Ä&÷VçG”7&VFVBÀ¢6W&FUö§6öã£¦§6öâ‡°¢&&÷VçG•ö6öçG&7B#¢&÷VçG•ö6öçG&7BÀ¢&7&VF÷"#¢&V6÷&Bæ7&VF÷%÷vÆÆWBÀ¢'FW&×5ö†6‚#¢&V6÷&BçFW&×5ö†6‚À¢'öÆ–7•ö†6‚#¢&V6÷&BçöÆ–7•ö†6‚À¢&7&VF–öåöæöæ6R#¢6öçG&7E÷FW&×5²&7&VF–öåöæöæ6R%Ð¢Ò’À¢’À¢€¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤6æöæ–6Ä&÷VçG•FW&×46öÖÖ—GFVBÀ¢6W&FUö§6öã£¦§6öâ‡°¢&66WFæ6Uö7&—FW&–ö†6‚#¢&V6÷&Bæ66WFæ6Uö7&—FW&–ö†6‚À¢&&Væ6†Ö&µö†6‚#¢&V6÷&Bæ&Væ6†Ö&µö†6‚À¢&Wf–FVæ6U÷66†VÖö†6‚#¢&V6÷&BæWf–FVæ6U÷66†VÖö†6€¢Ò’À¢’À¢€¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤6æöæ–6Ä&÷VçG”V6öæöÖ–746öæf–wW&VBÀ¢6W&FUö§6öã£¦§6öâ‡°¢'6öÇfW%÷&Wv&B#¢6öçG&7E÷FW&×5²'6öÇfW%÷&Wv&B%Õ²&Ö÷VçB%ÒÀ¢'fW&–f–W%÷&Wv&B#¢6öçG&7E÷FW&×5²'fW&–f–W%÷&Wv&B%Õ²&Ö÷VçB%ÒÀ¢&6Æ–Õö&öæB#¢6öçG&7E÷FW&×5²&6Æ–Õö&öæB%Õ²&Ö÷VçB%ÒÀ¢'F&vWEöÖ÷VçB#¢6öçG&7E÷FW&×5²&–æ—F–ÅögVæF–ær%Õ²&Ö÷VçB%ÒÀ¢&–æ—F–ÅögVæF–ær#¢6öçG&7E÷FW&×5²&–æ—F–ÅögVæF–ær%Õ²&Ö÷VçB%ÒÀ¢&gVæF–æuöFVFÆ–æR#¢6öçG&7E÷FW&×5²&gVæF–æuöFVFÆ–æR%ÒÀ¢&6Æ–Õ÷v–æF÷u÷6V6öæG2#¢6öçG&7E÷FW&×5²&6Æ–Õ÷v–æF÷u÷6V6öæG2%ÒÀ¢'fW&–f–6F–öå÷v–æF÷u÷6V6öæG2#¢6öçG&7E÷FW&×5²'fW&–f–6F–öå÷v–æF÷u÷6V6öæG2%Ð¢Ò’À¢’À¢€¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤6æöæ–6Ä&÷VçG•fW&–f–6F–öä6öæf–wW&VBÀ¢6W&FUö§6öã£¦§6öâ‡°¢'fW&–f–6F–öåöÖöFR#¢À¢'fW&–f–W%öÖöGVÆR#¢&V6÷&BæFö7VÖVçBçfW&–f–6F–öå÷öÆ–7•²'fW&–f–W%öÖöGVÆR%ÒÀ¢'fW&–f–W%÷&Wv&E÷&V6—–VçB#¢&V6÷&BæFö7VÖVçBçfW&–f–6F–öå÷öÆ–7•²'fW&–f–W%÷&Wv&E÷&V6—–VçB%ÒÀ¢'F‡&W6†öÆB#¢À¢'fW&–f–W%÷6WEö†6‚#¢f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’¢Ò’À¢’À¢€¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG”&V6ÖT6Æ–Ö&ÆRÀ¢6W&FUö§6öã£¦§6öâ‡²&gVæFVEöÖ÷VçB#¢6öçG&7E÷FW&×5²&–æ—F–ÅögVæF–ær%Õ²&Ö÷VçB%×Ò’À¢’À¢€¢WFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG•6WGFÆVBÀ¢6W&FUö§6öã£¦§6öâ‡·Ò’À¢’À¢Ó°¢f÷"†WfVçEö–æFW‚Â†¶–æBÂFF’’–âWfVçG2æ–çFõö—FW"‚’æVçVÖW&FR‚’°¢–b–æFW‚ÓÒ"bb¶–æBÓÒWFöæöÖ÷W4&÷VçG”WfVçD¶–æC£¤&÷VçG•6WGFÆVB°¢6öçF–çVS°¢Ð¢7F÷&P¢çW6W'EöWFöæöÖ÷W5ö&÷VçG•öWfVçB€¢&&6RÖÖ–ææWB"À¢dWFöæöÖ÷W4&÷VçG”WfVçB°¢–C¢WV–C£¦æWu÷cB‚’À¢Æöuö¶W“¢f÷&ÖB‚'6ö6–Â×FW7C§·Ó§¶WfVçEö–æFW‡Ò"ÂWV–C£¦æWu÷cB‚’’À¢G…ö†6ƒ¢f÷&ÖB‚#‡³£cG‡Ò"ÂWV–C£¦æWu÷cB‚’æ5÷S#‚‚’’À¢&Æö6µöçVÖ&W#¢óó²–æFW‚À¢Æöuö–æFWƒ¢ScC£§G'•ög&öÒ†WfVçEö–æFW‚’çVçw&‚’À¢6öçG&7EöFG&W73¢&÷VçG•ö6öçG&7Bæ6ÆöæR‚’À¢&÷VçG•ö–C¢&÷VçG•ö–Bæ6ÆöæR‚’À¢¶–æBÀ¢FFÀ¢ö67W'&VEöC¢æ÷rÀ¢ÒÀ¢¢æv—@¢çVçw&‚“°¢Ð¢Ð¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ögVæF–æuö6öÖÖVçE÷ÆåöfÆw5ö÷W&F÷%÷&V6öæ6–Æ–F–öâ‚’°¢ÆWBÆâÒÆåöv—F‡V%ögVæF–æuö6öÖÖVçB„§6öâ…Æäv—D‡V$gVæF–æt6öÖÖVçE&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2ó#"çFõ÷7G&–ær‚’À¢F—FÆS¢%¶&÷VçG•Ó¢6òÖgVæF–ær"çFõ÷7G&–ær‚’À¢&öG“¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢6öÖÖVçEö&öG“¢"övVçBÖ&÷VçG’gVæBRU4D2f–&6UW6F4W67&÷r"çFõ÷7G&–ær‚’À¢6öçG&–'WF÷%öÆöv–ã¢6öÖR‚'6öÇfW"ÖvVçB"çFõ÷7G&–ær‚’’À¢6öÖÖVçEö–C¢6öÖR‚##2"çFõ÷7G&–ær‚’’À¢gVæF–æuö•ö&6U÷W&Ã¢æöæRÀ¢W†—7F–æuö–FV×÷FVæ7•ö¶W—3¢fV2µÒÀ¢Ò’¢æv—@¢ã° ¢76W'B‡Æâç&VG’“°¢ÆWB6–væÂÒÆâç6–væÂæW‡V7B‚&gVæF–ær6–væÂ"“°¢76W'B‡6–væÂç&WV—&W5ö÷W&F÷%÷&V6öæ6–Æ–F–öâ“°¢76W'EöW‡6–væÂæÖ÷VçBæ7W'&Væ7’Â'W6F2"“°¢76W'B‡6–væÂægVæF–æuö†æFöfe÷W&Âæ—5öæöæR‚’“°¢76W'B‡6–væÂæ–FV×÷FVæ7•ö¶W’æVæG5÷v—F‚‚#¦6öÖÖVçC£#2"’“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¥7V66W72“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ögVæF–æuö6öÖÖVçE÷Æå÷&WGW&ç5÷7G&—Uö†æFöfe÷W&Â‚’°¢ÆWBÆâÒÆåöv—F‡V%ögVæF–æuö6öÖÖVçB„§6öâ…Æäv—D‡V$gVæF–æt6öÖÖVçE&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2ó#"çFõ÷7G&–ær‚’À¢F—FÆS¢%¶&÷VçG•Ó¢6òÖgVæF–ær"çFõ÷7G&–ær‚’À¢&öG“¢fÆ–Eöv—F‡V%ö—77VUö&öG•÷v—F…ögVæF–æuöÖöFR‚%7G&—Tf–DÆVFvW""’À¢6öÖÖVçEö&öG“¢"övVçBÖ&÷VçG’gVæBRU4Bf–7G&—Tf–DÆVFvW""çFõ÷7G&–ær‚’À¢6öçG&–'WF÷%öÆöv–ã¢6öÖR‚&‡VÖâÖgVæFW""çFõ÷7G&–ær‚’’À¢6öÖÖVçEö–C¢6öÖR‚##B"çFõ÷7G&–ær‚’’À¢gVæF–æuö•ö&6U÷W&Ã¢6öÖR‚&‡GG3¢òö’ævVçF&÷VçF–W2æW†×ÆR"çFõ÷7G&–ær‚’’À¢W†—7F–æuö–FV×÷FVæ7•ö¶W—3¢fV2µÒÀ¢Ò’¢æv—@¢ã° ¢76W'B‡Æâç&VG’“°¢ÆWB6–væÂÒÆâç6–væÂæW‡V7B‚&gVæF–ær6–væÂ"“°¢ÆWB†æFöfbÒ6–væÂægVæF–æuö†æFöfe÷W&ÂæW‡V7B‚&†æFöfbW&Â"“°¢76W'B††æFöfbæ6öçF–ç2‚&‡GG3¢òö’ævVçF&÷VçF–W2æ÷V&Æ–2ögVæF–ær"’“°¢76W'B††æFöfbæ6öçF–ç2‚&”&6UW&ÃÖ‡GG2S4S$bS$f’ævVçF&÷VçF–W2æW†×ÆR"’“°¢76W'B††æFöfbæ6öçF–ç2‚'&–ÃÕ7G&—Tf–B"’“°¢76W'B††æFöfbæ6öçF–ç2‚&W‡FW&æÅ&VfW&Væ6SÖv—F‡V"ÖgVæF–ærÖ6öÖÖVçBS4"’“°¢76W'B‡Æâæ6†V6²çFW‡Bæ6öçF–ç2‚%7G&—R6†V6¶÷WBgVæF–ær†æFöfb"’“°¢76W'B‡Æà¢æ6†V6°¢çFW‡@¢æ6öçF–ç2‚'fW&–f–VB7G&—RvV&†öö²&V6öæ6–Æ–F–öâ"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ögVæF–æuö6öÖÖVçE÷Æå÷&V¦V7G5öGWÆ–6FU÷6–væÂ‚’°¢ÆWBW†—7F–æuö¶W’Ð¢&v—F‡V"ÖgVæF–ærÖ6öÖÖVçC¦vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W3¦‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2ó#¦6öÖÖVçC£#2#°¢ÆWBÆâÒÆåöv—F‡V%ögVæF–æuö6öÖÖVçB„§6öâ…Æäv—D‡V$gVæF–æt6öÖÖVçE&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2ó#"çFõ÷7G&–ær‚’À¢F—FÆS¢%¶&÷VçG•Ó¢6òÖgVæF–ær"çFõ÷7G&–ær‚’À¢&öG“¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢6öÖÖVçEö&öG“¢"övVçBÖ&÷VçG’gVæBRU4D2f–&6UW6F4W67&÷r"çFõ÷7G&–ær‚’À¢6öçG&–'WF÷%öÆöv–ã¢æöæRÀ¢6öÖÖVçEö–C¢6öÖR‚##2"çFõ÷7G&–ær‚’’À¢gVæF–æuö•ö&6U÷W&Ã¢æöæRÀ¢W†—7F–æuö–FV×÷FVæ7•ö¶W—3¢fV2¶W†—7F–æuö¶W’çFõ÷7G&–ær‚•ÒÀ¢Ò’¢æv—@¢ã° ¢76W'B‚Æâç&VG’“°¢76W'B‡ÆâæW'&÷"çVçw&‚’æ6öçF–ç2‚&GWÆ–6FRgVæF–ær6–væÂ"’“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¤7F–öå&WV—&VB“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ö6Æ–Õö6öÖÖVçE÷Æå÷&W6W'fW5÷&öw&W75ö&6¶VEö6Æ–Ò‚’°¢ÆWBÆâÒÆåöv—F‡V%ö6Æ–Õö6öÖÖVçB„§6öâ…Æäv—D‡V$6Æ–Ô6öÖÖVçE&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2óS‚"çFõ÷7G&–ær‚’À¢F—FÆS¢%¶&÷VçG•Ó¢FB7FÆRÖ6Æ–Ò6öçG&öÇ2"çFõ÷7G&–ær‚’À¢&öG“¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢6öÖÖVçEö&öG“¢"övVçBÖ&÷VçG’6Æ–ÕÆåÆã¢FBFWFW&Ö–æ—7F–27FÆR6Æ–ÒFW7G2â ¢çFõ÷7G&–ær‚’À¢6öçG&–'WF÷%öÆöv–ã¢6öÖR‚'6öÇfW"ÖvVçB"çFõ÷7G&–ær‚’’À¢6öÖÖVçEö–C¢6öÖR‚#CSb"çFõ÷7G&–ær‚’’À¢6Æ–ÕövUöÖ–çWFW3¢6öÖRƒR’À¢&öw&W75÷6–væÅö6÷VçC¢À¢7F—fUö6Æ–ÕöÆöv–ã¢æöæRÀ¢Ò’¢æv—@¢ã° ¢76W'B‡Æâç&VG’“°¢ÆWB6–væÂÒÆâç6–væÂæW‡V7B‚&6Æ–Ò6–væÂ"“°¢76W'B‚6–væÂç6WGFÆVÖVçEöWF†÷&—G’“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¥7V66W72“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%ö6Æ–Õö6öÖÖVçE÷Æå÷&V¦V7G5÷FV×ÆFVEö6Æ–Õ÷v—F†÷WE÷&öw&W72‚’°¢ÆWBÆâÒÆåöv—F‡V%ö6Æ–Õö6öÖÖVçB„§6öâ…Æäv—D‡V$6Æ–Ô6öÖÖVçE&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2óS‚"çFõ÷7G&–ær‚’À¢F—FÆS¢%¶&÷VçG•Ó¢FB7FÆRÖ6Æ–Ò6öçG&öÇ2"çFõ÷7G&–ær‚’À¢&öG“¢fÆ–Eöv—F‡V%ö—77VUö&öG’‚’À¢6öÖÖVçEö&öG“ ¢"övVçBÖ&÷VçG’6Æ–ÕÆä’vÒ&Wf–Wv–ærF†R6öFV&6RæBv–ÆÂ÷Vâ"6†÷'FÇ’â ¢çFõ÷7G&–ær‚’À¢6öçG&–'WF÷%öÆöv–ã¢6öÖR‚&6Æ–ÒÖ&÷B"çFõ÷7G&–ær‚’’À¢6öÖÖVçEö–C¢6öÖR‚#CSr"çFõ÷7G&–ær‚’’À¢6Æ–ÕövUöÖ–çWFW3¢6öÖRƒ’À¢&öw&W75÷6–væÅö6÷VçC¢À¢7F—fUö6Æ–ÕöÆöv–ã¢æöæRÀ¢Ò’¢æv—@¢ã° ¢76W'B‚Æâç&VG’“°¢76W'B‡Æâç6–væÂæ—5öæöæR‚’“°¢76W'B‡ÆâæW'&÷"çVçw&‚’æ6öçF–ç2‚&6öæ7&WFR&öw&W726–væÂ"’“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¤7F–öå&WV—&VB“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%÷&ööeö6öÖÖVçE÷Æå÷&WGW&ç5öÖ&¶F÷våöæEöf–ævW'&–çB‚’°¢ÆWB&÷VçG•ö–BÒWV–C£¦æWu÷cB‚“°¢ÆWBÆâÒÆåöv—F‡V%÷&ööeö6öÖÖVçB„§6öâ…Æäv—D‡V%&ööd6öÖÖVçE&WVW7B°¢&÷VçG•ö–BÀ¢&ööe÷W&Ã¢&‡GG3¢òövVçF&÷VçF–W2æÆö6Â÷V&Æ–2÷&öög2ö&2"çFõ÷7G&–ær‚’À¢fW&–f–W%÷7VÖÖ'“¢$v—D‡V"4’76VB"çFõ÷7G&–ær‚’À¢6WGFÆVÖVçE÷W&Ã¢6öÖR‚&‡GG3¢òö&6W66âæ÷&r÷G‚ó†&2"çFõ÷7G&–ær‚’’À¢Ò’¢æv—@¢ã° ¢76W'EöW‡Æâæ6öÖÖVçBæ&÷VçG•ö–BÂ&÷VçG•ö–B“°¢76W'B‡ÆâæÖ&¶F÷vâæ6öçF–ç2‚%&ööc¢"’“°¢76W'B‡ÆâæÖ&¶F÷vâæ6öçF–ç2‚%6WGFÆVÖVçC¢"’“°¢76W'EöW‡Æâæf–ævW'&–çBæÆVâ‚’ÂcB“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¥7V66W72“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%÷&ööeö6öÖÖVçE÷Æåög&öÕ÷&ööe÷W6W5÷7F÷&VE÷V&Æ–5÷&ööb‚’°¢ÆWB†æWGv÷&²Â&÷VçG’Â&ööb’Ò6ö×ÆWFVE÷6–×VÆFVEö&÷VçG’‚’æv—C°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“°¢ÆWBÆâÒÆåöv—F‡V%÷&ööeö6öÖÖVçEög&öÕ÷&ööb€¢7FFR‡7FFR’À¢§6öâ…Æäv—D‡V%&ööd6öÖÖVçDg&öÕ&ööe&WVW7B°¢&ööeö–C¢&ööbæ–BÀ¢6WGFÆVÖVçE÷W&Ã¢6öÖR‚&‡GG3¢òö&6W66âæ÷&r÷G‚ó†&2"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW‡Æâæ6öÖÖVçBæ&÷VçG•ö–BÂ&÷VçG’æ–B“°¢76W'EöW€¢Æâæ6öÖÖVçBç&ööe÷W&ÂÀ¢f÷&ÖB‚&‡GG¢òó#rããã£ƒƒ÷V&Æ–2÷&öög2÷·Ò"Â&ööbæ–B¢“°¢76W'B‡Æâæ6öÖÖVçBçfW&–f–W%÷7VÖÖ'’æ6öçF–ç2‚$§6öå66†VÖ"’“°¢76W'B‡ÆâæÖ&¶F÷vâæ6öçF–ç2‚%6WGFÆVÖVçC¢"’“°¢76W'EöW‡Æâæf–ævW'&–çBæÆVâ‚’ÂcB“°¢76W'EöW‡Æâæ6†V6²æ6öæ6ÇW6–öâÂv—D‡V$6†V6´6öæ6ÇW6–öã£¥7V66W72“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâv—F‡V%÷&ööeö6öÖÖVçE÷Æåög&öÕ÷&ööe÷&V¦V7G5÷&—fFU÷&öög2‚’°¢ÆWB†×WBæWGv÷&²Âö&÷VçG’Â×WB&ööb’Ò6ö×ÆWFVE÷6–×VÆFVEö&÷VçG’‚’æv—C°¢&ööbç&—f7’Ò&—f7”ÆWfVÃ£¥&—fFS°¢æWGv÷&²ç&öög2æ–ç6W'B‡&ööbæ–BÂ&ööbæ6ÆöæR‚’“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“°¢ÆWBW'&÷"ÒÆåöv—F‡V%÷&ööeö6öÖÖVçEög&öÕ÷&ööb€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ…Æäv—D‡V%&ööd6öÖÖVçDg&öÕ&ööe&WVW7B°¢&ööeö–C¢&ööbæ–BÀ¢6WGFÆVÖVçE÷W&Ã¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¤äõEôdõTäB“°¢ÆWBV&Æ–5öW'&÷"ÒV&Æ–5÷&ööe÷vR…7FFR‡7FFR’ÂF‚‡&ööbæ–B’¢æv—@¢çVçw&öW'"‚“°¢76W'EöW‡V&Æ–5öW'&÷"Â7FGW46öFS£¤äõEôdõTäB“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâF—66÷fW'•öVæGö–çEöGfW'F—6W5öWFöæöÖ÷W5÷&÷Fö6öÅööæÇ’‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWBÖæ–fW7BÒvVçEö&÷VçF–W5öF—66÷fW'’…7FFR‡7FFR’’æv—Bã° ¢76W'EöW€¢Öæ–fW7Bç66†VÖÀ¢&‡GG3¢òövVçF&÷VçF–W2æ÷66†VÖ2öF—66÷fW'’ÖÖæ–fW7Bçc"æ§6öâ ¢“°¢76W'EöW†Öæ–fW7Bç&÷Fö6öÅ²'fW'6–öâ%ÒÂ&vVçBÖ&÷VçF–W2öWFöæöÖ÷W2×c"“°¢76W'EöW†Öæ–fW7Bç&÷Fö6öÅ²&÷W&F÷%÷6WGFÆVÖVçE÷6–væW"%ÒÂfÇ6R“°¢76W'EöW€¢Öæ–fW7BæVæGö–çG2æWFöæöÖ÷W5ö6æöæ–6Åö6†–ÆE÷FW&×5÷ÆâÀ¢&‡GG¢òó#rããã£ƒƒ÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö6æöæ–6ÂÖ6†–ÆB×FW&×2×Æâ ¢“°¢76W'EöW€¢Öæ–fW7@¢æVæGö–çG0¢æWFöæöÖ÷W5÷7FæF–æuöÖWF÷c%ö6†–ÆE÷&W&F–öâÀ¢&‡GG¢òó#rããã£ƒƒ÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7FæF–ærÖÖWF×c"Ö6†–ÆB×&W&F–öâ ¢“°¢76W'EöW€¢Öæ–fW7BæVæGö–çG2æWFöæöÖ÷W5ö7&VF–öå÷ÆâÀ¢&‡GG¢òó#rããã£ƒƒ÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö7&VF–öâ×Æâ ¢“°¢76W'EöW€¢Öæ–fW7BæVæGö–çG2æWFöæöÖ÷W5ö&÷VçG•öfVVBÀ¢&‡GG¢òó#rããã£ƒƒ÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öfVVB ¢“°¢76W'B†Öæ–fW7@¢ævVçE÷FööÇ0¢æ—FW"‚¢æç’‡ÇFööÇÂFööÂÓÒ'ÆåöWFöæöÖ÷W5ö6æöæ–6Åö6†–ÆE÷FW&×2"’“°¢76W'B†Öæ–fW7@¢ævVçE÷FööÇ0¢æ—FW"‚¢æç’‡ÇFööÇÂFööÂÓÒ'&W&U÷7FæF–æuöÖWF÷c%ö6†–ÆB"’“°¢76W'B†Öæ–fW7@¢ævVçE÷FööÇ0¢æ—FW"‚¢æç’‡ÇFööÇÂFööÂÓÒ'ÆåöWFöæöÖ÷W5ö&÷VçG•÷7V&Ö—76–öâ"’“°¢76W'B†Öæ–fW7@¢ævVçE÷FööÇ0¢æ—FW"‚¢æÆÂ‡ÇFööÇÂFööÂç7F'G5÷v—F‚‚'Æåö&6Uò"’’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ6æöæ–6Åö6†–ÆE÷FW&×5öVæGö–çEöÖF6†W5ö6öçG&7E÷fV7F÷'2‚’°¢ÆWBÆâÒÆåöWFöæöÖ÷W5ö6æöæ–6Åö6†–ÆE÷FW&×2„§6öâ„6æöæ–6Ä6†–ÆD&÷VçG•FW&×5&WVW7B°¢&VçEö&÷VçG•ö–C¢f÷&ÖB‚#‡·Ò"Â&""ç&WVBƒ3"’’À¢&VçE÷&÷VæC¢À¢&VçE÷6öÇfW#¢#ƒ3333333333333333333333333333333333333332"çFõ÷7G&–ær‚’À¢&VçE÷6öÇfW%÷&Wv&C¢ÖöæW“£¦æWrƒ“óÂ'W6F2"’çVçw&‚’À¢6†–ÆEö66WFæ6Uö7&—FW&–¢fV2²%&öGV6RF†R6öÖÖ—GFVBF–v—FÂ'F–f7Bâ"çFõ÷7G&–ær‚•ÒÀ¢fW&–f–W%öÖöGVÆS¢#ƒCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCB"çFõ÷7G&–ær‚’À¢Ò’¢æv—@¢çVçw&‚¢ã° ¢76W'EöW€¢Æâæ66WFæ6Uö7&—FW&–ö†6‚À¢6†–åö&6S£¦¶V66³#Seö6æöæ–6Åö§6öâ‚g6W&FUö§6öã£¦§6öâ‡Æâæ66WFæ6Uö7&—FW&–’¢çVçw&‚¢“°¢76W'EöW‡Æâç&WV—&VEö6†–ÆE÷7FGW2Â'6WGFÆVB"“°¢76W'EöW‡ÆâæÖ–æ–×VÕö6†–ÆE÷F&vWBæÖ÷VçBÂ“ó“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ6æöæ–6Åö6†–ÆE÷FW&×5öVæGö–çEöW‡Æ–ç5÷&V¦V7FVEö6æ'•÷fW&–f–W"‚’°¢ÆWBW'&÷"ÒÆåöWFöæöÖ÷W5ö6æöæ–6Åö6†–ÆE÷FW&×2„§6öâ„6æöæ–6Ä6†–ÆD&÷VçG•FW&×5&WVW7B°¢&VçEö&÷VçG•ö–C¢f÷&ÖB‚#‡·Ò"Â&""ç&WVBƒ3"’’À¢&VçE÷&÷VæC¢À¢&VçE÷6öÇfW#¢#ƒ3333333333333333333333333333333333333332"çFõ÷7G&–ær‚’À¢&VçE÷6öÇfW%÷&Wv&C¢ÖöæW“£¦æWrƒ“óÂ'W6F2"’çVçw&‚’À¢6†–ÆEö66WFæ6Uö7&—FW&–¢fV2²%&öGV6RF†R6öÖÖ—GFVBF–v—FÂ'F–f7Bâ"çFõ÷7G&–ær‚•ÒÀ¢fW&–f–W%öÖöGVÆS¢6†–åö&6S£¤$4UôÔ”ääUEôÄTD”äuõ¤U$õõtõ$µõdU$”d”U"çFõ÷7G&–ær‚’À¢Ò’¢æv—@¢çVçw&öW'"‚“° ¢ÆWB‡7FGW2Â§6öâ†&öG’’’ÒW'&÷#°¢76W'EöW‡7FGW2Â7FGW46öFS£¤$Eõ$UTU5B“°¢76W'EöW†&öG’æW'&÷%ö6öFRÂ&–çfÆ–Eö6æöæ–6Åö6†–ÆE÷FW&×5÷Æâ"“°¢76W'B†&öG¢æÖW76vP¢æ6öçF–ç2‚&ÆVF–ær×¦W&òv÷&²6æ'’6ææ÷BfW&–g’6æöæ–6Â6†–ÆBF6²"’“°¢76W'B‚&öG’ç&WG'–&ÆR“°¢76W'B†&öG’ææW‡Eö7F–öâæ6öçF–ç2‚$Fòæ÷B7&VFR÷"gVæB"’“°¢Ð ¢5·FW7EÐ¢fâFW&×5÷V&Æ–6F–öå÷&WGW&ç5ö7F–öæ&ÆU÷6VÖçF–5öW'&÷"‚’°¢ÆWB×WBFö7VÖVçC¢WFöæöÖ÷W4&÷VçG•FW&×4Fö7VÖVçBÐ¢6W&FUö§6öã£¦g&öÕ÷7G"†–æ6ÇVFU÷7G"‚"ââòââòââö&÷VçF–W2öWFöæöÖ÷W2×có#CBæ§6öâ"’’çVçw&‚“°¢Fö7VÖVçBæ&Væ6†Ö&²Ò6W&FUö§6öã£¦§6öâ‡°¢&Væv–æR#¢&v—F‡V%ö6’"À¢'&WV—&VEö6†V6·2#¢²&6’%Ð¢Ò“° ¢ÆWBW'&÷"ÒWFöæöÖ÷W5÷FW&×5÷&V6÷&B…V&Æ—6„WFöæöÖ÷W4&÷VçG•FW&×5&WVW7B°¢7&VF÷%÷vÆÆWC¢#ƒƒƒCƒ3DSƒƒFCdS“3Cc#cST#ƒ#CC4ScsCv$2"çFõ÷7G&–ær‚’À¢Fö7VÖVçBÀ¢Ò¢çVçw&öW'"‚“° ¢ÆWB‡7FGW2Â§6öâ†&öG’’’ÒW'&÷#°¢76W'EöW‡7FGW2Â7FGW46öFS£¤$Eõ$UTU5B“°¢76W'EöW†&öG’æW'&÷%ö6öFRÂ&–çfÆ–EöWFöæöÖ÷W5ö&÷VçG•÷FW&×2"“°¢76W'B†&öG¢æÖW76vP¢æ6öçF–ç2‚&¶æ÷vâÆVF–ær×¦W&òfW&–f–W"×W7BW6R—G2W†7BbÖ&—B"’“°¢76W'B†&öG’ææW‡Eö7F–öâæ6öçF–ç2‚&&Vf÷&R7&VF–ær÷"gVæF–ær"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâÆ—fUöÖöæW•÷&VF–æW75öVæGö–çE÷&W÷'G5öæöå÷6V7&WEöFVfVÇG2‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB&W÷'BÒÆ—fUöÖöæW•÷&VF–æW72€¢7FFR‡7FFR’À¢VW'’„Æ—fTÖöæW•&VF–æW75VW'’°¢æWGv÷&³¢6öÖR‚&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW‡&W÷'BææWGv÷&²Â$&6R"“°¢76W'EöW‡&W÷'BææWGv÷&µö6†–åö–BÂ…óCS2“°¢76W'EöW‡&W÷'Bç7G&—U÷6V7&WEö¶W•öÖöFRÂ'Vç6WB"“°¢76W'B‚&W÷'Bç7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öåö6öæf–wW&VB“°¢76W'EöW‡&W÷'Bç7WÆ–VE÷W6F5÷Fö¶VåöÖF6†W5öæF—fRÂ6öÖR‡G'VR’“°¢76W'B‚&W÷'BæÆ—fUöÖöæW•÷&VG’“°¢76W'B‡&W÷'@¢æ6†V6·0¢æ—FW"‚¢æç’‡Æ6†V6·Â²6†V6²ææÖRÓÒ$WFöæöÖ÷W2&÷VçG’f7F÷'’"bb6†V6²æ6öæf–wW&VBÒ’“°¢76W'B‡&W÷'@¢æ6†V6·0¢æ—FW"‚¢æç’‡Æ6†V6·Â6†V6²ææÖRÓÒ%7G&—RÆ—fRÖÖöæW’W†V7WF–öâvFR"’“°¢76W'B‡&W÷'@¢æ6†V6·0¢æ—FW"‚¢æç’‡Æ6†V6·Â6†V6²ææÖRÓÒ%7G&—R6†V6¶÷WB–ÖVçBÖÖWF†öB6öæf–wW&F–öâ"’“°¢76W'B‚6W&FUö§6öã£§Fõ÷7G&–ær‚g&W÷'B¢çVçw&‚¢æ6öçF–ç2‚'Ö5÷—ÅöVæ&ÆVB"’“°¢Ð ¢5·FW7EÐ¢fâvVçE÷vÆÆWE÷&VF–æW75÷&FUöÆ–Ö—E÷&ö&ÆVÕö—5÷&WG'–&ÆUöæE÷&VF7FVB‚’°¢ÆWB‡7FGW2Â§6öâ‡&ö&ÆVÒ’’Ð¢ÖövVçE÷vÆÆWE÷&VF–æW75öW'&÷"„6†–ä&6TW'&÷#£¥'5&÷f–FW$W'&÷"°¢6öFS¢Ó3%óbÀ¢ÖW76vS¢&÷fW"&FRÆ–Ö—BB‡GG3¢òö7&VFVçF–ÂæW†×ÆR"çFõ÷7G&–ær‚’À¢Ò“° ¢76W'EöW‡7FGW2Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢76W'EöW‡&ö&ÆVÕ²&W'&÷"%ÒÂ&&6U÷'5÷&FUöÆ–Ö—FVB"“°¢76W'EöW‡&ö&ÆVÕ²'&WG'–&ÆR%ÒÂG'VR“°¢76W'B‚&ö&ÆVÒçFõ÷7G&–ær‚’æ6öçF–ç2‚&7&VFVçF–ÂæW†×ÆR"’“°¢76W'B‡&ö&ÆVÕ²&æW‡Eö7F–öâ%Ð¢æ5÷7G"‚¢çVçw&‚¢æ6öçF–ç2‚&Fòæ÷B7&VFR&ÆÆVÂ&WG&–W2"’“°¢Ð ¢5·FW7EÐ¢fâvVçE÷vÆÆWE÷&VF–æW75ö–çfÆ–Eö&÷VçG•÷&ö&ÆVÕö—5öæ÷E÷&WG'–&ÆR‚’°¢ÆWB‡7FGW2Â§6öâ‡&ö&ÆVÒ’’ÒÖövVçE÷vÆÆWE÷&VF–æW75öW'&÷"€¢6†–ä&6TW'&÷#£¤–çfÆ–DFG&W72‚&æ÷B6æöæ–6Â"çFõ÷7G&–ær‚’’À¢“° ¢76W'EöW‡7FGW2Â7FGW46öFS£¤$Eõ$UTU5B“°¢76W'EöW‡&ö&ÆVÕ²&W'&÷"%ÒÂ&–çfÆ–E÷&VF–æW75÷&WVW7B"“°¢76W'EöW‡&ö&ÆVÕ²'&WG'–&ÆR%ÒÂfÇ6R“°¢76W'EöW‡&ö&ÆVÕ²&f–ÆVE÷G&ç6—F–öâ%ÒÂ'fÆ–FFU÷&WVW7Eö÷%ö&÷VçG’"“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâÆ—fUöÖöæW•÷&VF–æW75öVæGö–çE÷&W÷'G5÷–ÖVçEöÖWF†öEö6öæf–wW&F–öå÷v—F†÷WEö–B‚’°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…÷7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ€¢&÷VçG”æWGv÷&³£¦FVfVÇB‚’À¢'Ö5÷—ÅöVæ&ÆVB"À¢“°¢ÆWB&W÷'BÒÆ—fUöÖöæW•÷&VF–æW72€¢7FFR‡7FFR’À¢VW'’„Æ—fTÖöæW•&VF–æW75VW'’°¢æWGv÷&³¢6öÖR‚&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWB§6öâÒ6W&FUö§6öã£§Fõ÷7G&–ær‚g&W÷'B’çVçw&‚“° ¢76W'B‡&W÷'Bç7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öåö6öæf–wW&VB“°¢76W'B‡&W÷'Bæ6†V6·2æ—FW"‚’æç’‡Æ6†V6·Â°¢6†V6²ææÖRÓÒ%7G&—R6†V6¶÷WB–ÖVçBÖÖWF†öB6öæf–wW&F–öâ ¢bb6†V6²æ6öæf–wW&V@¢bb6†V6²æVçe÷f'2ÓÒfV2²%5E$•Uõ”ÔTåEôÔUD„ôEô4ôäd”uU$D”ôâ"çFõ÷7G&–ær‚•Ð¢Ò’“°¢76W'B‚§6öâæ6öçF–ç2‚'Ö5÷—ÅöVæ&ÆVB"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâÆ—fUöÖöæW•÷&VF–æW75öVæGö–çE÷&V¦V7G5÷Væ¶æ÷våöæWGv÷&²‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWBW'&÷"ÒÆ—fUöÖöæW•÷&VF–æW72€¢7FFR‡7FFR’À¢VW'’„Æ—fTÖöæW•&VF–æW75VW'’°¢æWGv÷&³¢6öÖR‚&÷F–Ö—6Ò"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¤$Eõ$UTU5B“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ•öFö75öVæGö–çE÷ö–çG5÷Fõö÷Væ•ö§6öâ‚’°¢ÆWB‡FÖÂÒ•öFö72‚’æv—Bã° ¢76W'B†‡FÖÂæ6öçF–ç2‚"ö’ÖFö72ö÷Væ’æ§6öâ"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚"öÆÆ×2çG‡B"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚"÷66†VÖ2öF—66÷fW'’ÖÖæ–fW7Bçc"æ§6öâ"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚"òçvVÆÂÖ¶æ÷vâövVçBÖ&÷VçF–W2æ§6öâ"’“°¢Ð ¢5·FW7EÐ¢fâÖ–ææWE÷ÆææW%÷W6W5ööæÇ•÷F†Uö6æöæ–6ÅöGFW7FVEöFWÆ÷–ÖVçB‚’°¢ÆWBW‡V7FVBÒWFöæöÖ÷W5÷ÆææW%öFG&W76W2ƒ…óCS2ÂæöæRÂæöæR’çVçw&‚“°¢76W'EöW†W‡V7FVBãÂ4äôä”4Åô$4UôÔ”ääUEô$õTåE•ôd5Dõ%’“°¢76W'EöW†W‡V7FVBãÂ4äôä”4Åô$4UôÔ”ääUEô$õTåE•ô”ÕÄTÔTåDD”ôâ“° ¢ÆWBÖF6†–ærÒWFöæöÖ÷W5÷ÆææW%öFG&W76W2€¢…óCS2À¢6öÖR„4äôä”4Åô$4UôÔ”ääUEô$õTåE•ôd5Dõ%’çFõ÷WW&66R‚’’À¢6öÖR„4äôä”4Åô$4UôÔ”ääUEô$õTåE•ô”ÕÄTÔTåDD”ôâçFõ÷7G&–ær‚’’À¢¢çVçw&‚“°¢76W'EöW†ÖF6†–ærÂW‡V7FVB“° ¢76W'EöW€¢WFöæöÖ÷W5÷ÆææW%öFG&W76W2€¢…óCS2À¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢æöæRÀ¢’À¢W'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR¢“°¢Ð ¢5·FW7EÐ¢fâvVçEö6Æ–ÕöæWGv÷&µö¶W—5öÖF6…ö–æFW†W%÷7F÷&vUö¶W—2‚’°¢76W'EöW†6æöæ–6Åö&6UöæWGv÷&µö¶W’ƒ…óCS2’Â6öÖR‚&&6RÖÖ–ææWB"’“°¢76W'EöW†6æöæ–6Åö&6UöæWGv÷&µö¶W’ƒƒEóS3"’Â6öÖR‚&&6R×6WöÆ–"’“°¢76W'EöW†6æöæ–6Åö&6UöæWGv÷&µö¶W’ƒ’ÂæöæR“°¢Ð ¢5·FW7EÐ¢fâÖ–ææWE÷&VF–æW75÷W6W5ö6æöæ–6Åöf7F÷'•öæE÷&V¦V7G5öG&–gB‚’°¢76W'EöW€¢6æöæ–6ÅöÖ–ææWEöf7F÷'’„æöæRÂæöæR’æ5öFW&Vb‚’À¢6öÖR„4äôä”4Åô$4UôÔ”ääUEô$õTåE•ôd5Dõ%’¢“°¢76W'EöW€¢6æöæ–6ÅöÖ–ææWEöf7F÷'’€¢6öÖR„4äôä”4Åô$4UôÔ”ääUEô$õTåE•ôd5Dõ%’çFõ÷WW&66R‚’’À¢6öÖR„4äôä”4Åô$4UôÔ”ääUEô$õTåE•ô”ÕÄTÔTåDD”ôâçFõ÷WW&66R‚’’À¢¢æ5öFW&Vb‚’À¢6öÖR„4äôä”4Åô$4UôÔ”ääUEô$õTåE•ôd5Dõ%’¢“°¢76W'EöW€¢6æöæ–6ÅöÖ–ææWEöf7F÷'’€¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢æöæRÀ¢’À¢æöæP¢“°¢76W'EöW€¢6æöæ–6ÅöÖ–ææWEöf7F÷'’€¢æöæRÀ¢6öÖR‚#ƒ#######################################""çFõ÷7G&–ær‚’’À¢’À¢æöæP¢“°¢Ð ¢5·FW7EÐ¢fâ6WöÆ–÷ÆææW%÷7F–ÆÅ÷&WV—&W5öW‡Æ–6—EöFG&W76W2‚’°¢76W'EöW€¢WFöæöÖ÷W5÷ÆææW%öFG&W76W2ƒƒEóS3"ÂæöæRÂæöæR’À¢W'"…7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR¢“°¢76W'B†WFöæöÖ÷W5÷ÆææW%öFG&W76W2€¢ƒEóS3"À¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢6öÖR‚#ƒ#######################################""çFõ÷7G&–ær‚’’À¢¢æ—5öö²‚’“°¢Ð ¢5·FW7EÐ¢fâ÷÷'GVæ—G•ö6öÖÖVçEö–FVçF–f–W%÷fÆ–FF–öåö—5ö&÷VæFVB‚’°¢76W'EöW€¢æ÷&ÖÆ—¦Uö÷÷'GVæ—G•ö6öÖÖVçEö–B‚&6æöæ–6Ã¦&6RÖÖ–ææWC£†&2"’çVçw&‚’À¢&6æöæ–6Ã¦&6RÖÖ–ææWC£†&2 ¢“°¢76W'EöW€¢æ÷&ÖÆ—¦Uö÷÷'GVæ—G•ö6öÖÖVçEö–B‚&&Bö–B"’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢76W'EöW€¢æ÷&ÖÆ—¦Uö÷÷'GVæ—G•ö6öÖÖVçEö–B‚""’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢76W'EöW€¢&÷VæFVE÷V&Æ–5÷FW‡B‚"W6VgVÂ6öçFW‡B"Âc’çVçw&‚’À¢'W6VgVÂ6öçFW‡B ¢“°¢76W'EöW€¢&÷VæFVE÷V&Æ–5÷FW‡B‚b'‚"ç&WVBƒS’ÂS’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢Ð ¢5·FW7EÐ¢fâ÷÷'GVæ—G•öfVVF&6µö—5ö&÷VæFVEöæE÷vÆÆWEöÖFW&–Åö—5÷&VF7FVB‚’°¢ÆWB&WVW7BÒ÷÷'GVæ—G”fVVF&6µ&WVW7B°¢7FvS¢"gVæF–ær"çFõ÷7G&–ær‚’À¢F—66÷fW'•÷6÷W&6S¢6öÖR‚$Ô566ææW""çFõ÷7G&–ær‚’’À¢'F–6—F–öå÷&V6öã¢æöæRÀ¢g&–7F–öã¢6öÖR‚%F†R6æöæ–6Â7F—fF–öâ6öæF—F–öâv2Væ6ÆV"â"çFõ÷7G&–ær‚’’À¢&V6öÖÖVæFF–öã¢6öÖR‚%6†÷r6fRÖ&Æö6²7F—fF–öâ&V6V—Bâ"çFõ÷7G&–ær‚’’À¢Wf–FVæ6U÷&VfW&Væ6S¢6öÖR‚&6æöæ–6Ã¦&6RÖÖ–ææWC£†&2"çFõ÷7G&–ær‚’’À¢vÆÆWC¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢vÆÆWE÷6–væGW&S¢6öÖR†f÷&ÖB‚#‡·Ò"Â##""ç&WVBƒcR’’’À¢Ó°¢ÆWBæ÷&ÖÆ—¦VBÒæ÷&ÖÆ—¦Uö÷÷'GVæ—G•öfVVF&6²‚g&WVW7B’çVçw&‚“°¢76W'EöW†æ÷&ÖÆ—¦VBç7FvRÂ&gVæF–ær"“°¢ÆWBV&Æ–2Ð¢V&Æ–5ö÷÷'GVæ—G•öfVVF&6²…6öÖR‡6W&FUö§6öã£§Fõ÷fÇVR†æ÷&ÖÆ—¦VB’çVçw&‚’’’çVçw&‚“°¢76W'EöW‡V&Æ–2æWf–FVæ6U÷7FGW2Â'6VÆe÷&W÷'FVB"“°¢76W'EöW€¢V&Æ–2çvÆÆWEöWf–FVæ6U÷7FGW2æ5öFW&Vb‚’À¢6öÖR‚'7WÆ–VE÷VçfW&–f–VB"¢“°¢ÆWB6W&–Æ—¦VBÒ6W&FUö§6öã£§Fõ÷7G&–ær‚gV&Æ–2’çVçw&‚“°¢76W'B‚6W&–Æ—¦VBæ6öçF–ç2‚#ƒ"’“°¢76W'B‚6W&–Æ—¦VBæ6öçF–ç2‚b##""ç&WVBƒcR’’“°¢Ð ¢5·FW7EÐ¢fâ÷÷'GVæ—G•öfVVF&6µ÷&V¦V7G5ö–çfÆ–E÷7FvUöV×G•öF—&V7F–öåöæE÷'F–Å÷vÆÆWE÷&ööb‚’°¢ÆWB&6RÒ÷÷'GVæ—G”fVVF&6µ&WVW7B°¢7FvS¢'÷7F–ær"çFõ÷7G&–ær‚’À¢F—66÷fW'•÷6÷W&6S¢æöæRÀ¢'F–6—F–öå÷&V6öã¢æöæRÀ¢g&–7F–öã¢6öÖR‚$gVæF–ær6WVVæ6RFöö²FöòÖç’7FW2â"çFõ÷7G&–ær‚’’À¢&V6öÖÖVæFF–öã¢æöæRÀ¢Wf–FVæ6U÷&VfW&Væ6S¢æöæRÀ¢vÆÆWC¢æöæRÀ¢vÆÆWE÷6–væGW&S¢æöæRÀ¢Ó°¢ÆWB×WB–çfÆ–E÷7FvRÒ&6Ræ6ÆöæR‚“°¢–çfÆ–E÷7FvRç7FvRÒ'G&ff–2"çFõ÷7G&–ær‚“°¢76W'EöW€¢æ÷&ÖÆ—¦Uö÷÷'GVæ—G•öfVVF&6²‚f–çfÆ–E÷7FvR’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢ÆWB×WBV×G’Ò&6Ræ6ÆöæR‚“°¢V×G’æg&–7F–öâÒæöæS°¢76W'EöW€¢æ÷&ÖÆ—¦Uö÷÷'GVæ—G•öfVVF&6²‚fV×G’’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢ÆWB×WB'F–Å÷vÆÆWBÒ&6S°¢'F–Å÷vÆÆWBçvÆÆWBÒ6öÖR‚#ƒ"çFõ÷7G&–ær‚’“°¢76W'EöW€¢æ÷&ÖÆ—¦Uö÷÷'GVæ—G•öfVVF&6²‚g'F–Å÷vÆÆWB’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢Ð ¢5·FW7EÐ¢fâÆFf÷&ÕöÖWG&–5÷v–æF÷w5÷W6UöW†7EöÆVæ6…öæEöf—'7EöÖöçF…ö&÷VæF&–W2‚’°¢ÆWBVæFVEöBÒ'6U÷V&Æ–5öÖWG&–75÷F–ÖW7F×‚###bÓ‚Ó%C#£##£•¢"’çVçw&‚“°¢ÆWB6WfVåöF—2ÒÆFf÷&ÕöÖWG&–5÷v–æF÷r…6öÖR‚#vB"’ÂVæFVEöB’çVçw&‚“°¢76W'EöW€¢6WfVåöF—2ç7F'FVEöBÀ¢'6U÷V&Æ–5öÖWG&–75÷F–ÖW7F×‚###bÓ‚ÓUC#£##£•¢"’çVçw&‚¢“°¢76W'EöW€¢6WfVåöF—2ç&Wf–÷W5÷7F'FVEöBÀ¢'6U÷V&Æ–5öÖWG&–75÷F–ÖW7F×‚###bÓrÓ#•C#£##£•¢"’çVçw&‚¢“°¢76W'EöW€¢6WfVåöF—2æÆVæ6…öBçFõ÷&f3333’‚’À¢###bÓrÓ…C#£##£’³£ ¢“°¢76W'EöW€¢6WfVåöF—2æf—'7EöÖöçF…öVæFVEöBçFõ÷&f3333’‚’À¢###bÓ‚Ó…C#£##£’³£ ¢“° ¢ÆWBÆ–fWF–ÖRÒÆFf÷&ÕöÖWG&–5÷v–æF÷r…6öÖR‚&Æ–fWF–ÖR"’ÂVæFVEöB’çVçw&‚“°¢76W'EöW†Æ–fWF–ÖRç7F'FVEöBÂÆ–fWF–ÖRæÆVæ6…öB“°¢76W'EöW†Æ–fWF–ÖRç&Wf–÷W5÷7F'FVEöBÂÆ–fWF–ÖRæÆVæ6…öB“°¢76W'EöW€¢ÆFf÷&ÕöÖWG&–5÷v–æF÷r…6öÖR‚#3B"’ÂVæFVEöB’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢Ð ¢fâFW7E÷ÆFf÷&ÕöFVÖæEöw&÷wF…÷7FG2‚’ÓâÆFf÷&ÔFVÖæDw&÷wF…7FG2°¢ÆFf÷&ÔFVÖæDw&÷wF…7FG2°¢v×eóvEö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢v×eó#†Eö&6U÷Væ—G3¢##"çFõ÷7G&–ær‚’À¢Æ–fWF–ÖUöv×eö&6U÷Væ—G3¢##"çFõ÷7G&–ær‚’À¢æWu÷÷7FW%ögVæFW%÷vÆÆWG5ó#†C¢"À¢7F—fU÷÷7FW%ögVæFW%÷vÆÆWG5ó#†C¢BÀ¢&WVE÷÷7FW%ögVæFW%÷vÆÆWG5ó#†C¢À¢æöåö÷W&F÷%öGG&–'WFVEöv×eó#†Eö&6U÷Væ—G3¢#c"çFõ÷7G&–ær‚’À¢GG&–'WFVEöv×eó#†Eö&6U÷Væ—G3¢##"çFõ÷7G&–ær‚’À¢Ð¢Ð ¢5·FW7EÐ¢fâV&Æ–5öÖWG&–75÷öÆ–7•öW†6ÇVFW5÷F†UöFV6Æ&VEö÷W&F÷%÷vÆÆWG2‚’°¢ÆWBöÆ–7’ÒV&Æ–5öÖWG&–75÷öÆ–7’‚’çVçw&‚“°¢76W'EöW€¢öÆ–7’æÖ–çF–æW%÷vÆÆWG2À¢fV2°¢#ƒV3cƒss&6csf&3VcFSCsCsccsfS366Scc""À¢#ƒƒƒCƒ3FSƒƒFCfS“3Cc#cSV#ƒ#CC6ScsCv&2"À¢#†f#Sƒ“C“3cVS63fCc&SƒfVF#Fff66cFVcsCsr"À¢#†fCv&SF3c“SC##“vV6S&csFf3C†#ƒ“†63"À¢Ð¢“°¢Ð ¢5·FW7EÐ¢fâÆFf÷&ÕöÖWG&–5ö†—7F÷'•÷W6W5÷öÆ–7•öW†6ÇW6–öç5öæ÷E÷&V6÷fW'•÷&W6W'fF–öç2‚’°¢ÆWB&V6÷fW'•ö6öçG&7BÒ#ƒ“““““““““““““““““““““““““““““““““““““““’#°¢ÆWB&W6W'fF–öç2Ð¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£§'6Uö77b…6öÖR‡&V6÷fW'•ö6öçG&7B’’çVçw&‚“°¢76W'B‡&W6W'fF–öç2æ6öçF–ç2‡&V6÷fW'•ö6öçG&7B’“° ¢ÆWBW†6ÇW6–öç2Ò†—7F÷&–6Å÷ÆFf÷&ÕöÖWG&–5öW†6ÇW6–öç2‚gV&Æ–5öÖWG&–75÷öÆ–7’‚’çVçw&‚’“°¢76W'B‚W†6ÇW6–öç0¢æ—FW"‚¢æç’‡Æ6öçG&7GÂ6öçG&7BÓÒ&V6÷fW'•ö6öçG&7B’“°¢76W'EöW†W†6ÇW6–öç2æÆVâ‚’ÂB“°¢Ð ¢5·FW7EÐ¢fâÆFf÷&ÕöÖWG&–75÷&W7öç6Uö—5övw&VvFUööæÇ•öæE÷&W6W'fW5÷–ÖVçEöÖF‚‚’°¢ÆWBvVæW&FVEöBÒ'6U÷V&Æ–5öÖWG&–75÷F–ÖW7F×‚###bÓ‚Ó%C#£##£•¢"’çVçw&‚“°¢ÆWB7FG2ÒÆFf÷&ÔÖWG&–757FG2°¢vVæW&FVEöBÀ¢–FVçF—F–W3¢ÆFf÷&Ô–FVçF—G•7FG2°¢6VÆV7FVC¢BÀ¢&Wf–÷W3¢"À¢ÆFW7E÷vVV³¢BÀ¢&Wf–÷W5÷vVV³¢"À¢f—'7EöÖöçFƒ¢2À¢Æ–fWF–ÖS¢RÀ¢÷7FW'3¢À¢gVæFW'3¢À¢6öÇfW'3¢"À¢fW&–f–W'3¢À¢6öÖÖVçFW'3¢À¢Ö&¶WGÆ6U÷vÆÆWG3¢2À¢÷÷'GVæ—G•ö6öÖÖVçEöWF†÷'3¢À¢ÒÀ¢–÷WG3¢ÆFf÷&Õ–÷WE7FG2°¢6VÆV7FVE÷F÷FÅö&6U÷Væ—G3¢##S"çFõ÷7G&–ær‚’À¢&Wf–÷W5÷F÷FÅö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢f—'7EöÖöçF…÷F÷FÅö&6U÷Væ—G3¢#“"çFõ÷7G&–ær‚’À¢Æ–fWF–ÖU÷F÷FÅö&6U÷Væ—G3¢#3S"çFõ÷7G&–ær‚’À¢6VÆV7FVE÷6öÇfW%ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢6VÆV7FVE÷fW&–f–W%ö&6U÷Væ—G3¢##"çFõ÷7G&–ær‚’À¢6VÆV7FVEö¶VWW%ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢6VÆV7FVEö&öçW5ö&6U÷Væ—G3¢#S"çFõ÷7G&–ær‚’À¢6VÆV7FVE÷6WGFÆVE÷&÷VæG3¢À¢&Wf–÷W5÷6WGFÆVE÷&÷VæG3¢À¢f—'7EöÖöçF…÷6WGFÆVE÷&÷VæG3¢À¢Æ–fWF–ÖU÷6WGFÆVE÷&÷VæG3¢À¢ÒÀ¢6Æ–Õö6ö†÷'C¢ÆFf÷&Ô6Æ–Ô6ö†÷'E7FG2°¢6WGFÆVC¢À¢ÖGW&S¢"À¢–ÖÖGW&S¢À¢ÒÀ¢F–Ç“¢fV2µÆFf÷&ÔF–Ç•7FG2°¢F“¢###bÓ‚Ó""çFõ÷7G&–ær‚’À¢7F—fUö–FVçF—F–W3¢BÀ¢–÷WEö&6U÷Væ—G3¢##S"çFõ÷7G&–ær‚’À¢6WGFÆVE÷&÷VæG3¢À¢ÕÒÀ¢6÷fW&vS¢ÆFf÷&ÔÖWG&–746÷fW&vU7FG2°¢fW&–f–VEö6æöæ–6ÅöWfVçG3¢’À¢v—F–æuö&Æö6µ÷F–ÖUöWfVçG3¢À¢÷÷'GVæ—G•ö6öÖÖVçG3¢"À¢ÆFW7E÷fW&–f–VEöWfVçEöC¢6öÖR†vVæW&FVEöB’À¢ÆFW7Eö6öÖÖVçEöC¢6öÖR†vVæW&FVEöB’À¢ÒÀ¢Ó°¢ÆWB×WBöÆ–7’ÒV&Æ–5öÖWG&–75÷öÆ–7’‚’çVçw&‚“°¢öÆ–7’æÖ–çF–æW%öv—F‡V%öÆöv–ç2ÒfV2²'&—fFRÖÖ–çF–æW"ÖÆöv–â"çFõ÷7G&–ær‚•Ó°¢öÆ–7’æÖ–çF–æW%÷vÆÆWG2ÒfV2²#ƒ"çFõ÷7G&–ær‚•Ó°¢ÆWB&W7öç6RÒÆFf÷&ÕöÖWG&–75÷&W7öç6R€¢7FG2À¢FW7E÷ÆFf÷&ÕöFVÖæEöw&÷wF…÷7FG2‚’À¢ÆFf÷&ÕöÖWG&–5÷v–æF÷r…6öÖR‚#vB"’ÂvVæW&FVEöB’çVçw&‚’À¢öÆ–7’À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢ÆFf÷&Ô6æöæ–6Å6÷W&6Tg&W6†æW72°¢WFöæöÖ÷W3¢G'VRÀ¢÷Våö6ö×WF—F–öã¢G'VRÀ¢÷Våö6ö×WF—F–öå÷c#¢G'VRÀ¢ÒÀ¢¢çVçw&‚“°¢ÆWB§6öâÒ6W&FUö§6öã£§Fõ÷7G&–ær‚g&W7öç6R’çVçw&‚“° ¢76W'EöW€¢&W7öç6Rç66†VÖ÷fW'6–öâÀ¢&vVçBÖ&÷VçF–W2÷ÆFf÷&ÒÖÖWG&–72×c2 ¢“°¢76W'EöW‡&W7öç6RæÖ&¶WGÆ6U÷–÷WE÷föÇVÖRç6VÆV7FVBçW6F2Â#ã#S"“°¢76W'EöW€¢&W7öç6RæÖ&¶WGÆ6U÷–÷WE÷föÇVÖRç6VÆV7FVEö¶VWW%÷’çW6F2À¢#ã ¢“°¢76W'EöW€¢&W7öç6RæÖGW&Uö6Æ–Õ÷Fõ÷6WGFÆVÖVçBç6WGFÆVÖVçE÷&FRÀ¢6öÖRƒãR¢“°¢76W'EöW‡&W7öç6RçÆFf÷&Õö7F—fUö–FVçF—F–W2ææÖW76W2æÆVâ‚’Â"“°¢76W'EöW€¢&W7öç6RçÆFf÷&Õö7F—fUö–FVçF—F–W2ææÖW76W5³Òæ7F—fUö–FVçF—F–W2À¢0¢“°¢76W'EöW€¢&W7öç6RçÆFf÷&Õö7F—fUö–FVçF—F–W2ææÖW76W5³Òæ7F—fUö–FVçF—F–W2À¢¢“°¢76W'EöW‡&W7öç6Ræ7W'&VçEö–çfVçF÷'’ç7FGW2Â'Væf–Æ&ÆR"“°¢76W'EöW‡&W7öç6RæFVÖæEöw&÷wF‚æv×e÷W6F5óvBçW6F2Â#ã"“°¢76W'EöW€¢&W7öç6RæFVÖæEöw&÷wF‚ç&WVE÷÷7FW%ögVæFW%÷&FUó#†BÀ¢6öÖRƒã#R¢“°¢76W'EöW€¢&W7öç6RæFVÖæEöw&÷wF‚ææöåö÷W&F÷%ögVæFVEöv×e÷6†&Uó#†BÀ¢6öÖRƒãR¢“°¢76W'EöW‡&W7öç6Ræ6÷fW&vRç7FGW2Â''F–Â"“°¢76W'B‡&W7öç6Ræ6÷fW&vRæÖ&¶WGÆ6Uö–æFW†W'5ög&W6‚“°¢76W'B‚&W7öç6Ræ6÷fW&vRæv—F‡V%ö–æ6ÇVFVB“°¢76W'B‚§6öâæ6öçF–ç2‚'&—fFRÖÖ–çF–æW"ÖÆöv–â"’“°¢76W'B‚§6öâæ6öçF–ç2‚#ƒ"’“°¢76W'B‚§6öâæ6öçF–ç2‚&÷Våö6ö×WF—F–öå÷c%ö–æFW†W%ög&W6‚"’“°¢76W'B‚§6öâæ6öçF–ç2‚&÷Våö6ö×WF—F–öåö–æFW†W%ög&W6‚"’“°¢76W'B‚§6öâæ6öçF–ç2‚&WFöæöÖ÷W5ö–æFW†W%ög&W6‚"’“°¢76W'B‚§6öâæ6öçF–ç2‚'7FæF–æuöÖWF"’“°¢76W'B‚§6öâæ6öçF–ç2‚'fW&–f–VEö÷Våö6ö×WF—F–öå÷c""’“°¢Ð ¢5·FW7EÐ¢fâÆFf÷&Õö–çfVçF÷'•ö6öÖ&–æW5öÆÅög&W6…÷6÷W&6W5÷v—F†÷WE÷G—U÷7Æ—B‚’°¢ÆWBWFöæöÖ÷W2ÒWFöæöÖ÷W4&÷VçG”–çfVçF÷'•7VÖÖ'’°¢66†VÖ÷fW'6–öã¢'FW7B"çFõ÷7G&–ær‚’À¢æWGv÷&³¢&&6RÖÖ–ææWB"çFõ÷7G&–ær‚’À¢vVæW&FVEöC¢###bÓ‚Ó%C#££³£"çFõ÷7G&–ær‚’À¢6æöæ–6Å÷6÷W&6S¢'FW7B"çFõ÷7G&–ær‚’À¢6Æ–Ö&ÆUö&÷VçG•ö6÷VçC¢"À¢fW&–f–6F–öå÷&VG•ö&÷VçG•ö6÷VçC¢À¢7FæF–æuöÖWFö&÷VçG•ö6÷VçC¢À¢gVæFVE÷W6F5ö&6U÷Væ—G3¢#3"çFõ÷7G&–ær‚’À¢gVæFVE÷W6F3¢#2ã"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢##C"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F3¢#"ãC"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢#c"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&E÷W6F3¢#ãc"çFõ÷7G&–ær‚’À¢—FV×3¢fV3£¦æWr‚’À¢Wf–FVæ6Uö&÷VæF'“¢'FW7B"çFõ÷7G&–ær‚’À¢Ó°¢ÆWB6ö×WF—F–öâÒ÷Vä6ö×WF—F–öä–çfVçF÷'•7VÖÖ'’°¢vVæW&FVEöC¢###bÓ‚Ó%C#££³£"çFõ÷7G&–ær‚’À¢&VG•÷FõöV&åö6÷VçC¢À¢gVæFVE÷W6F5ö&6U÷Væ—G3¢##"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢##"çFõ÷7G&–ær‚’À¢Ó°¢ÆWB6ö×WF—F–öå÷c"Ò÷Vä6ö×WF—F–öä–çfVçF÷'•7VÖÖ'’°¢vVæW&FVEöC¢###bÓ‚Ó%C#£#£³£"çFõ÷7G&–ær‚’À¢&VG•÷FõöV&åö6÷VçC¢À¢gVæFVE÷W6F5ö&6U÷Væ—G3¢#3C"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢#3"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢Ó°¢ÆWB6öÖ&–æVBÒÆFf÷&Õö–çfVçF÷'•÷&W7öç6R€¢6öÖR†WFöæöÖ÷W2æ6ÆöæR‚’’À¢6öÖR†6ö×WF—F–öâ’À¢6öÖR†6ö×WF—F–öå÷c"’À¢¢çVçw&‚“° ¢76W'EöW†6öÖ&–æVBç7FGW2Â'&VG’"“°¢76W'EöW†6öÖ&–æVBæ7F—fUögVæFVEö÷÷'GVæ—F–W2Â6öÖRƒ2’“°¢76W'EöW€¢6öÖ&–æVBæf–Æ&ÆUögVæF–æu÷W6F2æ5öFW&Vb‚’À¢6öÖR‚#3Bãc"¢“°¢76W'EöW€¢6öÖ&–æVBæf–Æ&ÆU÷6öÇfW%÷&Wv&G5÷W6F2æ5öFW&Vb‚’À¢6öÖR‚#32ãC"¢“°¢ÆWB6öÖ&–æVEö§6öâÒ6W&FUö§6öã£§Fõ÷7G&–ær‚f6öÖ&–æVB’çVçw&‚“°¢76W'B‚6öÖ&–æVEö§6öâæ6öçF–ç2‚&WFöæöÖ÷W5ö6Æ–Ö&ÆUö&÷VçF–W2"’“°¢76W'B‚6öÖ&–æVEö§6öâæ6öçF–ç2‚&÷Våö6ö×WF—F–öç5÷&VG•÷FõöV&â"’“°¢76W'B‚6öÖ&–æVEö§6öâæ6öçF–ç2‚&÷Våö6ö×WF—F–öå÷c""’“°¢76W'B‚6öÖ&–æVEö§6öâæ6öçF–ç2‚'7FæF–æuöÖWFö&÷VçF–W2"’“° ¢ÆWB'F–ÂÒÆFf÷&Õö–çfVçF÷'•÷&W7öç6R…6öÖR†WFöæöÖ÷W2’ÂæöæRÂæöæR’çVçw&‚“°¢76W'EöW‡'F–Âç7FGW2Â''F–Â"“°¢76W'EöW‡'F–Âæ7F—fUögVæFVEö÷÷'GVæ—F–W2ÂæöæR“°¢76W'B‡'F–Âæf–Æ&ÆUögVæF–æu÷W6F2æ—5öæöæR‚’“°¢Ð ¢5·FW7EÐ¢fâÆFf÷&ÕöÖWG&–75÷¦W&õöÖGW&UöFVæöÖ–æF÷%ö†5öæõ÷&FR‚’°¢76W'EöW€¢ÆFf÷&ÕöÖWG&–75÷&W7öç6R€¢ÆFf÷&ÔÖWG&–757FG2°¢vVæW&FVEöC¢'6U÷V&Æ–5öÖWG&–75÷F–ÖW7F×‚###bÓ‚Ó%C#£##£•¢"’çVçw&‚’À¢–FVçF—F–W3¢ÆFf÷&Ô–FVçF—G•7FG2°¢6VÆV7FVC¢À¢&Wf–÷W3¢À¢ÆFW7E÷vVV³¢À¢&Wf–÷W5÷vVV³¢À¢f—'7EöÖöçFƒ¢À¢Æ–fWF–ÖS¢À¢÷7FW'3¢À¢gVæFW'3¢À¢6öÇfW'3¢À¢fW&–f–W'3¢À¢6öÖÖVçFW'3¢À¢Ö&¶WGÆ6U÷vÆÆWG3¢À¢÷÷'GVæ—G•ö6öÖÖVçEöWF†÷'3¢À¢ÒÀ¢–÷WG3¢ÆFf÷&Õ–÷WE7FG2°¢6VÆV7FVE÷F÷FÅö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢&Wf–÷W5÷F÷FÅö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢f—'7EöÖöçF…÷F÷FÅö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢Æ–fWF–ÖU÷F÷FÅö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢6VÆV7FVE÷6öÇfW%ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢6VÆV7FVE÷fW&–f–W%ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢6VÆV7FVEö¶VWW%ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢6VÆV7FVEö&öçW5ö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢6VÆV7FVE÷6WGFÆVE÷&÷VæG3¢À¢&Wf–÷W5÷6WGFÆVE÷&÷VæG3¢À¢f—'7EöÖöçF…÷6WGFÆVE÷&÷VæG3¢À¢Æ–fWF–ÖU÷6WGFÆVE÷&÷VæG3¢À¢ÒÀ¢6Æ–Õö6ö†÷'C¢ÆFf÷&Ô6Æ–Ô6ö†÷'E7FG2°¢6WGFÆVC¢À¢ÖGW&S¢À¢–ÖÖGW&S¢"À¢ÒÀ¢F–Ç“¢fV3£¦æWr‚’À¢6÷fW&vS¢ÆFf÷&ÔÖWG&–746÷fW&vU7FG2°¢fW&–f–VEö6æöæ–6ÅöWfVçG3¢À¢v—F–æuö&Æö6µ÷F–ÖUöWfVçG3¢À¢÷÷'GVæ—G•ö6öÖÖVçG3¢À¢ÆFW7E÷fW&–f–VEöWfVçEöC¢æöæRÀ¢ÆFW7Eö6öÖÖVçEöC¢æöæRÀ¢ÒÀ¢ÒÀ¢ÆFf÷&ÔFVÖæDw&÷wF…7FG2°¢v×eóvEö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢v×eó#†Eö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢Æ–fWF–ÖUöv×eö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢æWu÷÷7FW%ögVæFW%÷vÆÆWG5ó#†C¢À¢7F—fU÷÷7FW%ögVæFW%÷vÆÆWG5ó#†C¢À¢&WVE÷÷7FW%ögVæFW%÷vÆÆWG5ó#†C¢À¢æöåö÷W&F÷%öGG&–'WFVEöv×eó#†Eö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢GG&–'WFVEöv×eó#†Eö&6U÷Væ—G3¢#"çFõ÷7G&–ær‚’À¢ÒÀ¢ÆFf÷&ÕöÖWG&–5÷v–æF÷r€¢6öÖR‚#vB"’À¢'6U÷V&Æ–5öÖWG&–75÷F–ÖW7F×‚###bÓ‚Ó%C#£##£•¢"’çVçw&‚’À¢¢çVçw&‚’À¢V&Æ–5öÖWG&–75÷öÆ–7’‚’çVçw&‚’À¢æöæRÀ¢æöæRÀ¢æöæRÀ¢ÆFf÷&Ô6æöæ–6Å6÷W&6Tg&W6†æW72°¢WFöæöÖ÷W3¢G'VRÀ¢÷Våö6ö×WF—F–öã¢G'VRÀ¢÷Våö6ö×WF—F–öå÷c#¢G'VRÀ¢ÒÀ¢¢çVçw&‚¢æÖGW&Uö6Æ–Õ÷Fõ÷6WGFÆVÖVç@¢ç6WGFÆVÖVçE÷&FRÀ¢æöæP¢“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ÷Væ•ö§6öåöVæGö–çEö6öçF–ç5övVçE÷&÷WFW%÷F‚‚’°¢ÆWBFö7VÖVçBÒ÷Væ•ö§6öâ‚’æv—Bã°¢ÆWBfÇVRÒ6W&FUö§6öã£§Fõ÷fÇVR†Fö7VÖVçB’çVçw&‚“°¢ÆWBf—‡GW&S¢6W&FUö§6öã£¥fÇVRÐ¢6W&FUö§6öã£¦g&öÕ÷7G"†–æ6ÇVFU÷7G"‚"ââöf—‡GW&W2ö÷Væ’Ö6öçG&7Bæ§6öâ"’’çVçw&‚“°¢76W'EöW€¢†6…ö'F–f7B‚g6W&FUö§6öã£§Fõ÷7G&–ær‚gfÇVR’çVçw&‚’’À¢f—‡GW&U²&æ÷&ÖÆ—¦VE÷6†#Sb%Ð¢“°¢ÆWBF‡2ÒfÇVU²'F‡2%Òæ5öö&¦V7B‚’çVçw&‚“° ¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&÷WFRÖ&Æö6¶VBÖvöÂ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"òçvVÆÂÖ¶æ÷vâövVçBÖ6&Bæ§6öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"ö&÷cöÖW76vS§6VæB"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"ö&÷c÷F6·2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"ö&÷c÷F6·2÷¶–GÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"ö&÷c÷F6·2÷¶–GÓ¦6æ6VÂ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"öÆÆ×2çG‡B"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷66†VÖ2öF—66÷fW'’ÖÖæ–fW7Bçc"æ§6öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷W&F÷"öF—7G&–'WF–öâ÷&W÷'B"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷W&F÷"öF—7G&–'WF–öâ÷vÆÆWBÖW†6ÇW6–öç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöF—7G&–'WF–öâ÷7VÖÖ'’"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöF—7G&–'WF–öâö†æFöfg2÷vÆÆWB×&Wf–Wr"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&—6²÷öÆ–7’"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&VF–æW72öÆ—fRÖÖöæW’"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c÷fW&–f–W'2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cöWfVçG2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cö7&VF–öâ×&W&F–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cöWF†÷&—¦VBÖ7&VF–öâ×&W&F–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c÷7FFR"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c÷&VF–æW72"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cö6öÖÖ—B×&W&F–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c÷&WfVÂ×&W&F–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c÷7FGW2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cö&öæB×v—F†G&vÂ×&W&F–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cöVçG&çBÖ7F–öâ×&W&F–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cöVçG&çBÖ7F–öâ×&VÆ—2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×cöVçG&çBÖ7F–öâ×&VÆ—2÷·&VÆ•ö–GÒ"’“°¢76W'B€¢F‡2æ6öçF–ç5ö¶W’‚"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö&÷VæFVB×vÆÆWBÖ6æ6VÂ×&VgVæB×Æâ"¢“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö6Æ÷VBÖvVçBöö&¦V7F—fR×Æç2"’“°¢76W'B€¢fÇVU²'F‡2%Õ²"÷cö6Æ÷VBÖvVçBöö&¦V7F—fR×Æç2%Õ²'÷7B%Õ²'&W7öç6W2%Ð¢ævWB‚#S""¢æ—5÷6öÖR‚¢“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöö&¦V7F—fW2ö7&VF–öâ×Æç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöö&¦V7F—fW2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöö&¦V7F—fW2÷¶–GÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöö&¦V7F—fW2÷¶–GÒö7F–öâ×Æç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöö&¦V7F—fW2÷¶–GÒö7F–öç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöö&¦V7F—fW2÷¶–GÒ÷&V6öæ6–ÆR"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷÷'GVæ—F–W2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"ö&÷VçG’ÖF—66÷fW'’×c"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷÷'GVæ—F–W2÷7G&VÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷÷'GVæ—F–W2÷¶÷÷'GVæ—G•ö–GÒö6öÖÖVçG2"’“°¢76W'B‡fÇVU²&6ö×öæVçG2%Õ²'66†VÖ2%Ð¢ævWB‚$÷÷'GVæ—G”fVVF&6µ&WVW7B"¢æ—5÷6öÖR‚’“°¢76W'B‡fÇVU²&6ö×öæVçG2%Õ²'66†VÖ2%Ð¢ævWB‚%ÆFf÷&ÔFVÖæDw&÷wF…&W7öç6R"¢æ—5÷6öÖR‚’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷÷'GVæ—F–W2öfVVBç'72"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷÷'GVæ—F–W2öfVVBæFöÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷÷'GVæ—F–W2öfVVBæ§6öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷÷'GVæ—F–W2ö6öçfW'6–öâÖgVææVÂ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö6†FwBö7F–öâÖ–çFVçG2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö6†FwBö7F–öâÖ–çFVçG2÷¶–çFVçEö–GÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö6†FwBö7F–öâÖ–çFVçG2÷¶–çFVçEö–GÒöö'6W'fF–öç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöæÇ—F–72öWfVçG2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöæÇ—F–72÷6—FR"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöÖWG&–72÷ÆFf÷&Ò"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷W&F÷"öF—66÷fW&&–Æ—G’÷6æ6†÷G2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö÷W&F÷"öF—66÷fW&&–Æ—G’÷&W÷'B"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöF—66÷fW&&–Æ—G’÷7VÖÖ'’"’“°¢ÆWB–ævW7F–öå÷6V7W&—G’Ð¢F‡5²"÷cö÷W&F÷"öF—66÷fW&&–Æ—G’÷6æ6†÷G2%Õ²'÷7B%Õ²'6V7W&—G’%ÒçFõ÷7G&–ær‚“°¢76W'B†–ævW7F–öå÷6V7W&—G’æ6öçF–ç2‚&F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vâ"’“°¢76W'B‚–ævW7F–öå÷6V7W&—G’æ6öçF–ç2‚&÷W&F÷%ö•÷Fö¶Vâ"’“°¢ÆWB&W÷'E÷6V7W&—G’Ð¢F‡5²"÷cö÷W&F÷"öF—66÷fW&&–Æ—G’÷&W÷'B%Õ²&vWB%Õ²'6V7W&—G’%ÒçFõ÷7G&–ær‚“°¢76W'B‡&W÷'E÷6V7W&—G’æ6öçF–ç2‚&÷W&F÷%ö•÷Fö¶Vâ"’“°¢76W'B‚&W÷'E÷6V7W&—G’æ6öçF–ç2‚&F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vâ"’“°¢76W'B‡fÇVU²&6ö×öæVçG2%Õ²'66†VÖ2%Ð¢ævWB‚$F—66÷fW&&–Æ—G•V&Æ–57VÖÖ'’"¢æ—5÷6öÖR‚’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöF—66÷fW'’÷7V'67&—F–öç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöF—66÷fW'’÷7V'67&—F–öç2÷¶–GÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷V&Æ–2ö÷÷'GVæ—F–W2÷¶÷÷'GVæ—G•ö–GÒöVÖ&VB"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷V&Æ–2ö÷÷'GVæ—F–W2÷¶÷÷'GVæ—G•ö–GÒöVÖ&VBç7fr"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷V&Æ–2ö÷÷'GVæ—F–W2÷¶÷÷'GVæ—G•ö–GÒöVÖ&VBæÖB"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷¶&÷VçG•ö6öçG&7GÒöæÇ—6—2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷VægVæFVBÖ&÷VçF–W2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷VægVæFVBÖ&÷VçF–W2÷¶–GÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷VægVæFVBÖ&÷VçF–W2÷¶–GÒ÷6öÇWF–öç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6RövVçB×vÆÆWB÷&VF–æW72"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&—6²öWfVçG2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&—6²÷&Wf–Ww2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&—6²ö&÷VçG’Ö&÷fÇ2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&—6²÷–÷WBÖ&÷fÇ2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷&—6²öWfVçG2÷¶–GÒ÷&V¦V7B"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cövVçG2÷¶–GÒ÷–B×7FGW2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö6öçG&–'WF÷"Ö6öçF7G2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöVF–Væ6RöÖVÖ&W'2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöVF–Væ6Rö–çFW&7F–öç2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöVF–Væ6RöF—66÷fW'’×&W7öç6W2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöVF–Væ6Rö÷WG&V6‚ÖGFV×G2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöVF–Væ6R÷&W÷'B"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö6&–Æ—F–W2÷6V&6‚"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"òçvVÆÂÖ¶æ÷vâ÷ƒC"æ§6öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷ƒC"ö&6Rö&÷VçF–W2÷¶&÷VçG•ö6öçG&7GÒögVæF–ær"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷ƒC"ö&6R÷&VÆ—2÷·&VÆ•ö–GÒ"’“°¢f÷"c"–â°¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&VÆV6R"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&öf–ÆW2"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷7G'V7GW&VBÖ'F–f7B×&öf–ÆR"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷fÆ–FFR"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2ö7&VF–öâ×&W&F–öâ"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2ögVæF–ær×&W&F–öâ"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2ö–çfVçF÷'’"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2öWfVçG2"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&ööb×V÷FW2"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&ööb×&W&F–öâ"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&ööbÖGG&–'WF–öâ"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2ö7F–öâ×&W&F–öâ"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&ööbÖ¦ö'2÷¶¦ö%ö–GÒ"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&ööbÖ¦ö'2÷¶¦ö%ö–GÒ÷–ÖVçB"À¢"÷cö&6Rö÷VâÖ6ö×WF—F–öâ×c"Ö&WF2÷&ööbÖ¦ö'2÷¶¦ö%ö–GÒ÷&VÆ’ÖWF†÷&—¦F–öâ"À¢Ò°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‡c"’Â&Ö—76–ær·c'Ò"“°¢Ð¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6Rö'&öF67B×6–væVB×G&ç67F–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&6R÷G&ç67F–öâ×&V6V—B"’“°¢f÷"WFöæöÖ÷W2–â°¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö6æöæ–6ÂÖ6†–ÆB×FW&×2×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö7&VF–öâ×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öWF†÷&—¦VBÖ7&VF–öâ×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö6öçG&–'WF–öâ×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öWF†÷&—¦VBÖ6öçG&–'WF–öâ×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö6Æ–×2"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö6Æ–ÒÖgVææVÂ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2ö6Æ–Ò×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öWF†÷&—¦VBÖ6Æ–Ò×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâ×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâ×&W&F–öâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷7V&Ö—76–öâÖWF†÷&—¦F–öâ×Æâ"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷fW&–f–6F–öâÖ¦ö'2"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öFV6öFRÖWfVçG2"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öWfVçG2"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷FW&×2"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2÷FW&×2÷·FW&×5ö†6‡Ò"À¢"÷cö&6RöWFöæöÖ÷W2Ö&÷VçF–W2öfVVB"À¢Ò°¢76W'B‡F‡2æ6öçF–ç5ö¶W’†WFöæöÖ÷W2’Â&Ö—76–ær¶WFöæöÖ÷W7Ò"“°¢Ð¢f÷"ö&¦V7F—fR–â°¢"÷cöö&¦V7F—fW2ö7&VF–öâ×Æç2"À¢"÷cöö&¦V7F—fW2"À¢"÷cöö&¦V7F—fW2÷¶–GÒ"À¢"÷cöö&¦V7F—fW2÷¶–GÒö7F–öâ×Æç2"À¢"÷cöö&¦V7F—fW2÷¶–GÒö7F–öç2"À¢"÷cöö&¦V7F—fW2÷¶–GÒ÷&V6öæ6–ÆR"À¢Ò°¢76W'B‡F‡2æ6öçF–ç5ö¶W’†ö&¦V7F—fR’Â&Ö—76–ær¶ö&¦V7F—fWÒ"“°¢Ð¢f÷"&WF—&VB–â°¢"÷cö&6Rö–æFW†W"×7FGW2"À¢"÷cö&6RöW67&÷rÖWfVçG2"À¢"÷cö&6RöWfÒÖÆöw2"À¢"÷cö&6RöÆör×VW'’"À¢"÷cö&6R÷'2ÖÆöw2"À¢"÷cö&6RöfWF6‚×'2ÖÆöw2"À¢"÷cö&6RögVæF–ær×Æâ"À¢"÷cö&6R÷&VÆV6R×VWVR"À¢"÷cö&6R÷&VÆV6R×Æâ"À¢"÷cö&6R÷&VgVæB×Æâ"À¢"÷cö&6RöF—7WFR×Æâ"À¢Ò°¢76W'B€¢F‡2æ6öçF–ç5ö¶W’‡&WF—&VB’À¢'&WF—&VBF‚ÆV¶VC¢·&WF—&VGÒ ¢“°¢Ð¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷7G&—RöÆ—fRö6†V6¶÷WB×F÷×W2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷7G&—RöÆ—fRögVæF–ærÖ–çFVçG2÷¶–GÒö6†V6¶÷WB×6W76–öâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷7G&—RöÆ—fRö6öææV7BÖ66÷VçG2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷7G&—Rö6öææV7B×6æ6†÷G2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷7G&—Rö6†V6¶÷WB×vV&†öö·2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cö&÷VçF–W2÷¶–GÒögVæF–ærÖ–çFVçG2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"ö—77VRÖ&÷VçG’×Æâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"ö—77VRÖ’×7–æ2×Æâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"ö—77VRÖ’×7–æ2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"ö7&VFRÖ6öÖÖVçB×Æâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"ögVæF–ærÖ6öÖÖVçB×Æâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"ö6Æ–ÒÖ6öÖÖVçB×Æâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷6ö6–ÂöÖVçF–öâÖG&gB×Æâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷6ö6–ÂöÖVçF–öâÖ–ævW7F–öâ÷&VF–æW72"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷6ö6–Â÷vV&†öö·2öæW–æ""’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷c÷6ö6–ÂöÖVçF–öâÖG&gG2÷¶–GÒ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"÷&ööbÖ6öÖÖVçB×Æâ"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöv—F‡V"÷&ööbÖ6öÖÖVçB×ÆâÖg&öÒ×&ööb"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöWfÇ2öÆö÷2"’“°¢76W'B‡F‡2æ6öçF–ç5ö¶W’‚"÷cöWfÇ2÷'Vç2"’“° ¢ÆWB6V7W&—G•÷66†VÖW2ÒfÇVU²&6ö×öæVçG2%Õ²'6V7W&—G•66†VÖW2%Ð¢æ5öö&¦V7B‚¢æW‡V7B‚'6V7W&—G’66†VÖW2"“°¢76W'EöW€¢6V7W&—G•÷66†VÖW5²&÷W&F÷%ö•÷Fö¶Vâ%Õ²&æÖR%ÒÀ¢õU$Dõ%õDô´Tåô„TDU ¢“°¢76W'EöW‡6V7W&—G•÷66†VÖW5²&÷W&F÷%ö•÷Fö¶Vâ%Õ²&–â%ÒÂ&†VFW""“°¢76W'EöW‡6V7W&—G•÷66†VÖW5²&÷W&F÷%ö&V&W"%Õ²'66†VÖR%ÒÂ&&V&W""“° ¢f÷"F‚–â°¢"÷c÷&—6²ö&÷VçG’Ö&÷fÇ2"À¢"÷c÷&—6²÷–÷WBÖ&÷fÇ2"À¢"÷c÷&—6²öWfVçG2÷¶–GÒ÷&V¦V7B"À¢"÷cö6öçG&–'WF÷"Ö6öçF7G2"À¢"÷cö&6Rö'&öF67B×6–væVB×G&ç67F–öâ"À¢"÷c÷7G&—RöÆ—fRö6†V6¶÷WB×F÷×W2"À¢"÷c÷7G&—RöÆ—fRö6öææV7BÖ66÷VçG2"À¢"÷c÷7G&—Rö6öææV7B×6æ6†÷G2"À¢"÷cöv—F‡V"ö—77VRÖ’×7–æ2"À¢Ò°¢ÆWB6V7W&—G’ÒF‡5·F…Õ²'÷7B%Õ²'6V7W&—G’%Òæ5ö'&’‚’çVçw&‚“°¢76W'B€¢6V7W&—G¢æ—FW"‚¢æç’‡Ç&WV—&VÖVçGÂ&WV—&VÖVçBævWB‚&÷W&F÷%ö•÷Fö¶Vâ"’æ—5÷6öÖR‚’’À¢'·F‡ÒÖ—76–ær÷W&F÷%ö•÷Fö¶Vâ6V7W&—G’ ¢“°¢76W'B€¢6V7W&—G¢æ—FW"‚¢æç’‡Ç&WV—&VÖVçGÂ&WV—&VÖVçBævWB‚&÷W&F÷%ö&V&W""’æ—5÷6öÖR‚’’À¢'·F‡ÒÖ—76–ær÷W&F÷%ö&V&W"6V7W&—G’ ¢“°¢76W'B‡F‡5·F…Õ²'÷7B%Õ²'&W7öç6W2%Õ²#C%Òæ—5öö&¦V7B‚’“°¢Ð ¢76W'B‡F‡5²"÷cö&6R÷G&ç67F–öâ×&V6V—B%Õ²'÷7B%Ð¢ævWB‚'6V7W&—G’"¢æ—5öæöæR‚’“°¢76W'B‡F‡5²"÷cö&6R÷G&ç67F–öâ×&V6V—B%Õ²'÷7B%Õ²'&W7öç6W2%Ð¢ævWB‚#C"¢æ—5öæöæR‚’“° ¢76W'B€¢F‡5²"÷c÷7G&—RöÆ—fRögVæF–ærÖ–çFVçG2÷¶–GÒö6†V6¶÷WB×6W76–öâ%Õ²'÷7B%Ð¢ævWB‚'6V7W&—G’"¢æ—5öæöæR‚’À¢%V&Æ–2gVæFW"6†V6¶÷WB×W7Bæ÷B&WV—&R÷W&F÷"WF‚ ¢“°¢76W'B€¢F‡5²"÷c÷7G&—RöÆ—fRögVæF–ærÖ–çFVçG2÷¶–GÒö6†V6¶÷WB×6W76–öâ%Õ²'÷7B%Õ²'&W7öç6W2%Ð¢²#S2%Ð¢æ—5öö&¦V7B‚¢“°¢76W'B€¢F‡5²"÷c÷7G&—Rö6†V6¶÷WB×vV&†öö·2%Õ²'÷7B%Ð¢ævWB‚'6V7W&—G’"¢æ—5öæöæR‚’À¢%7G&—R6†V6¶÷WBvV&†öö²×W7B&VÖ–â6ÆÆ&ÆR'’7G&—Rv—F†÷WB÷W&F÷"WF‚ ¢“°¢76W'B‡F‡5²"÷c÷7G&—Rö6†V6¶÷WB×vV&†öö·2%Õ²'÷7B%Õ²'&W7öç6W2%Õ²#S2%Òæ—5öö&¦V7B‚’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ&÷&÷WFW%÷6W'fW5ö6&EöæEö6÷&U÷F6µö÷W&F–öç2‚’°¢ÆWBÒ&£§&÷WFW"‚’çv—F…÷7FFR‡FW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’’“° ¢ÆWB6&BÒ ¢æ6ÆöæR‚¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢çW&’‚"òçvVÆÂÖ¶æ÷vâövVçBÖ6&Bæ§6öâ"¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢76W'EöW†6&Bç7FGW2‚’Â7FGW46öFS£¤ô²“°¢76W'EöW€¢6&Bæ†VFW'2‚’ævWB††VFW#£¤4ôåDTåEõE•R’çVçw&‚’À¢&Æ–6F–öâö§6öâ ¢“° ¢ÆWB&WVW7Eö&öG’Ò6W&FUö§6öã£¦§6öâ‡°¢&ÖW76vR#¢°¢&ÖW76vT–B#¢'&÷WFW"Ö–çFW&÷W&&–Æ—G’Ó"À¢'&öÆR#¢%$ôÄUõU4U""À¢''G2#¢·²&FF#¢²'6¶–ÆÂ#¢'Vç7W÷'FVB'×ÕÐ¢Ð¢Ò¢çFõ÷7G&–ær‚“°¢ÆWB6VæBÒÆ&öG“¢7G&–æwÂ°¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢æÖWF†öB‚%õ5B"¢çW&’‚"ö&÷cöÖW76vS§6VæB"¢æ†VFW"‚&&×fW'6–öâ"Â#ã"¢æ†VFW"††VFW#£¤4ôåDTåEõE•RÂ&Æ–6F–öâö&¶§6öâ"¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦g&öÒ†&öG’’¢çVçw&‚¢Ó°¢ÆWBf—'7BÒ ¢æ6ÆöæR‚¢æöæW6†÷B‡6VæB‡&WVW7Eö&öG’æ6ÆöæR‚’’¢æv—@¢çVçw&‚“°¢76W'EöW†f—'7Bç7FGW2‚’Â7FGW46öFS£¤ô²“°¢76W'EöW€¢f—'7Bæ†VFW'2‚’ævWB††VFW#£¤4ôåDTåEõE•R’çVçw&‚’À¢&Æ–6F–öâö&¶§6öâ ¢“°¢ÆWBf—'7Eö&öG’Ò‡VÓ£¦&öG“£§Fõö'—FW2†f—'7Bæ–çFõö&öG’‚’Â#B¢#B¢æv—@¢çVçw&‚“°¢ÆWBf—'7Eö§6öã¢6W&FUö§6öã£¥fÇVRÒ6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚ff—'7Eö&öG’’çVçw&‚“°¢76W'EöW€¢f—'7Eö§6öå²'F6²%Õ²'7FGW2%Õ²'7FFR%ÒÀ¢%D4µõ5DDUô”åUEõ$UT•$TB ¢“°¢76W'B†f—'7Eö§6öå²'F6²%Õ²'7FGW2%Õ²'F–ÖW7F×%Ð¢æ5÷7G"‚¢æ—5÷6öÖUöæB‡ÇF–ÖW7F×ÂF–ÖW7F×æVæG5÷v—F‚‚u¢r’bbF–ÖW7F×æ6öçF–ç2‚râr’’“°¢ÆWBF6µö–BÒf—'7Eö§6öå²'F6²%Õ²&–B%Òæ5÷7G"‚’çVçw&‚’çFõ÷7G&–ær‚“°¢ÆWB6öçFW‡Eö–BÒf—'7Eö§6öå²'F6²%Õ²&6öçFW‡D–B%Ð¢æ5÷7G"‚¢çVçw&‚¢çFõ÷7G&–ær‚“° ¢ÆWB&WG'’Òæ6ÆöæR‚’æöæW6†÷B‡6VæB‡&WVW7Eö&öG’’’æv—BçVçw&‚“°¢ÆWB&WG'•ö&öG’Ò‡VÓ£¦&öG“£§Fõö'—FW2‡&WG'’æ–çFõö&öG’‚’Â#B¢#B¢æv—@¢çVçw&‚“°¢ÆWB&WG'•ö§6öã¢6W&FUö§6öã£¥fÇVRÒ6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚g&WG'•ö&öG’’çVçw&‚“°¢76W'EöW‡&WG'•ö§6öå²'F6²%Õ²&–B%ÒÂF6µö–B“° ¢ÆWBvWE÷&W7öç6RÒ ¢æ6ÆöæR‚¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢çW&’†f÷&ÖB‚"ö&÷c÷F6·2÷·F6µö–GÓö†—7F÷'”ÆVæwFƒÓ"’¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢76W'EöW†vWE÷&W7öç6Rç7FGW2‚’Â7FGW46öFS£¤ô²“° ¢ÆWB6æ6VÅ÷&W7öç6RÒ ¢æ6ÆöæR‚¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢æÖWF†öB‚%õ5B"¢çW&’†f÷&ÖB‚"ö&÷c÷F6·2÷·F6µö–GÓ¦6æ6VÂ"’¢æ†VFW"‚&&×fW'6–öâ"Â#ã"¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢76W'EöW†6æ6VÅ÷&W7öç6Rç7FGW2‚’Â7FGW46öFS£¤ô²“°¢ÆWB6æ6VÅö&öG’Ò‡VÓ£¦&öG“£§Fõö'—FW2†6æ6VÅ÷&W7öç6Ræ–çFõö&öG’‚’Â#B¢#B¢æv—@¢çVçw&‚“°¢ÆWB6æ6VÅö§6öã¢6W&FUö§6öã£¥fÇVRÒ6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚f6æ6VÅö&öG’’çVçw&‚“°¢76W'EöW†6æ6VÅö§6öå²'7FGW2%Õ²'7FFR%ÒÂ%D4µõ5DDUô4ä4TÄTB"“° ¢ÆWB&WVFVEö6æ6VÂÒ ¢æ6ÆöæR‚¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢æÖWF†öB‚%õ5B"¢çW&’†f÷&ÖB‚"ö&÷c÷F6·2÷·F6µö–GÓ¦6æ6VÂ"’¢æ†VFW"‚&&×fW'6–öâ"Â#ã"¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢76W'EöW‡&WVFVEö6æ6VÂç7FGW2‚’Â7FGW46öFS£¤ô²“° ¢ÆWBÆ—7E÷&W7öç6RÒ ¢æ6ÆöæR‚¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢çW&’†f÷&ÖB€¢"ö&÷c÷F6·3ö6öçFW‡D–C×¶6öçFW‡Eö–GÒgvU6—¦SÓf†—7F÷'”ÆVæwFƒÓ ¢’¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢76W'EöW†Æ—7E÷&W7öç6Rç7FGW2‚’Â7FGW46öFS£¤ô²“°¢ÆWBÆ—7Eö&öG’Ò‡VÓ£¦&öG“£§Fõö'—FW2†Æ—7E÷&W7öç6Ræ–çFõö&öG’‚’Â#B¢#B¢æv—@¢çVçw&‚“°¢ÆWBÆ—7Eö§6öã¢6W&FUö§6öã£¥fÇVRÒ6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚fÆ—7Eö&öG’’çVçw&‚“°¢76W'EöW†Æ—7Eö§6öå²'F÷FÅ6—¦R%ÒÂ“° ¢ÆWBÖ—76–æu÷F6²Ò ¢æ6ÆöæR‚¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢çW&’‚"ö&÷c÷F6·2öÖ—76–ær×F6²"¢æ†VFW"‚&&×fW'6–öâ"Â#ã"¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢76W'EöW†Ö—76–æu÷F6²ç7FGW2‚’Â7FGW46öFS£¤äõEôdõTäB“°¢76W'EöW€¢Ö—76–æu÷F6²æ†VFW'2‚’ævWB††VFW#£¤4ôåDTåEõE•R’çVçw&‚’À¢&Æ–6F–öâö&¶§6öâ ¢“°¢76W'EöW†Ö—76–æu÷F6²æ†VFW'2‚’ævWB‚&&×fW'6–öâ"’çVçw&‚’Â#ã"“°¢ÆWBÖ—76–æuö&öG’Ò‡VÓ£¦&öG“£§Fõö'—FW2†Ö—76–æu÷F6²æ–çFõö&öG’‚’Â#B¢#B¢æv—@¢çVçw&‚“°¢ÆWBÖ—76–æuö§6öã¢6W&FUö§6öã£¥fÇVRÒ6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚fÖ—76–æuö&öG’’çVçw&‚“°¢76W'EöW†Ö—76–æuö§6öå²&W'&÷"%Õ²'7FGW2%ÒÂ$äõEôdõTäB"“°¢76W'EöW€¢Ö—76–æuö§6öå²&W'&÷"%Õ²&FWF–Ç2%Õ³Õ²'&V6öâ%ÒÀ¢%D4µôäõEôdõTäB ¢“° ¢ÆWBF6…÷fW'6–öâÒ ¢æ6ÆöæR‚¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢çW&’‚"ö&÷c÷F6·2öÖ—76–ær×F6²"¢æ†VFW"‚&&×fW'6–öâ"Â#ãã"¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢76W'EöW‡F6…÷fW'6–öâç7FGW2‚’Â7FGW46öFS£¤$Eõ$UTU5B“°¢76W'EöW€¢F6…÷fW'6–öâæ†VFW'2‚’ævWB††VFW#£¤4ôåDTåEõE•R’çVçw&‚’À¢&Æ–6F–öâ÷&ö&ÆVÒ¶§6öâ ¢“° ¢ÆWBæöç–Ö÷W5öÆ—7BÒ ¢æöæW6†÷B€¢‡VÓ£¦‡GG£¥&WVW7C£¦'V–ÆFW"‚¢çW&’‚"ö&÷c÷F6·3÷vU6—¦SÓ"¢æ&öG’†‡VÓ£¦&öG“£¤&öG“£¦V×G’‚’¢çVçw&‚’À¢¢æv—@¢çVçw&‚“°¢ÆWBæöç–Ö÷W5ö&öG’Ò‡VÓ£¦&öG“£§Fõö'—FW2†æöç–Ö÷W5öÆ—7Bæ–çFõö&öG’‚’Â#B¢#B¢æv—@¢çVçw&‚“°¢ÆWBæöç–Ö÷W5ö§6öã¢6W&FUö§6öã£¥fÇVRÒ6W&FUö§6öã£¦g&öÕ÷6Æ–6R‚fæöç–Ö÷W5ö&öG’’çVçw&‚“°¢76W'EöW†æöç–Ö÷W5ö§6öå²'F÷FÅ6—¦R%ÒÂ“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ6öçG&–'WF÷%ö6öçF7G5ö&Uö÷W&F÷%övFVB‚’°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶Vâ„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â'6V7&WB×Fö¶Vâ"“°¢ÆWBFVæ–VBÒW6W'Eö6öçG&–'WF÷%ö6öçF7B€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢§6öâ…W6W'D6öçG&–'WF÷$6öçF7E&WVW7B°¢v—F‡V%öÆöv–ã¢'–ÇS2"çFõ÷7G&–ær‚’À¢VÖ–Ã¢æöæRÀ¢–÷WE÷vÆÆWC¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢76ö6–FVE÷'3¢fV2²"3#B"çFõ÷7G&–ær‚•ÒÀ¢6öçF7Eö6öç6VçC¢fÇ6RÀ¢vÆÆWEö6öç6VçC¢G'VRÀ¢÷WG&V6…öÆÆ÷vVC¢fÇ6RÀ¢6÷W&6S¢6öÖR‚&v—F‡V"Ö6öÖÖVçBÖ÷BÖ–â"çFõ÷7G&–ær‚’’À¢æ÷FW3¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†FVæ–VBÂ7FGW46öFS£¥TäUD„õ$•¤TB“° ¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B„õU$Dõ%õDô´Tåô„TDU"Â'6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWB6öçF7BÒW6W'Eö6öçG&–'WF÷%ö6öçF7B€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…W6W'D6öçG&–'WF÷$6öçF7E&WVW7B°¢v—F‡V%öÆöv–ã¢'–ÇS2"çFõ÷7G&–ær‚’À¢VÖ–Ã¢æöæRÀ¢–÷WE÷vÆÆWC¢6öÖR‚#ƒ"çFõ÷7G&–ær‚’’À¢76ö6–FVE÷'3¢fV2²"3#B"çFõ÷7G&–ær‚•ÒÀ¢6öçF7Eö6öç6VçC¢fÇ6RÀ¢vÆÆWEö6öç6VçC¢G'VRÀ¢÷WG&V6…öÆÆ÷vVC¢fÇ6RÀ¢6÷W&6S¢6öÖR‚&v—F‡V"Ö6öÖÖVçBÖ÷BÖ–â"çFõ÷7G&–ær‚’’À¢æ÷FW3¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW†6öçF7Bæv—F‡V%öÆöv–âÂ'–ÇS2"“°¢76W'B†6öçF7BçvÆÆWEö6öç6VçB“°¢ÆWB6öçF7G2ÒÆ—7Eö6öçG&–'WF÷%ö6öçF7G2…7FFR‡7FFR’Â†VFW'2¢æv—@¢çVçw&‚¢ã°¢76W'EöW†6öçF7G2æÆVâ‚’Â“°¢76W'EöW†6öçF7G5³Òæ76ö6–FVE÷'2ÂfV2²"3#B"çFõ÷7G&–ær‚•Ò“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâVF–Væ6UöVF—Eö—5ö÷W&F÷%övFVEöæE÷&W÷'G5÷V&Æ–5öGG&–'WF–öâ‚’°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶Vâ„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â'6V7&WB×Fö¶Vâ"“°¢ÆWB&WVW7BÒW6W'DVF–Væ6TÖVÖ&W%&WVW7B°¢&÷f–FW#¢FöÖ–ã£¤VF–Væ6U&÷f–FW#£¤v—F‡V"À¢W‡FW&æÅö–C¢%Uó#2"çFõ÷7G&–ær‚’À¢†æFÆS¢&æW†–7GW&&ò"çFõ÷7G&–ær‚’À¢V&Æ–5÷&öf–ÆU÷W&Ã¢6öÖR‚&‡GG3¢òöv—F‡V"æ6öÒöæW†–7GW&&ò"çFõ÷7G&–ær‚’’À¢&öÆW3¢fV2µÒÀ¢ö'6W'fVEöC¢æöæRÀ¢Ó°¢ÆWBFVæ–VBÒW6W'EöVF–Væ6UöÖVÖ&W"€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢§6öâ‡&WVW7Bæ6ÆöæR‚’’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†FVæ–VBÂ7FGW46öFS£¥TäUD„õ$•¤TB“° ¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B„õU$Dõ%õDô´Tåô„TDU"Â'6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWBÖVÖ&W"ÒW6W'EöVF–Væ6UöÖVÖ&W"…7FFR‡7FFRæ6ÆöæR‚’’Â†VFW'2æ6ÆöæR‚’Â§6öâ‡&WVW7B’¢æv—@¢çVçw&‚¢ã°¢ÆWBòÒ&V6÷&EöVF–Væ6Uö–çFW&7F–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…&V6÷&DVF–Væ6T–çFW&7F–öå&WVW7B°¢VF–Væ6UöÖVÖ&W%ö–C¢ÖVÖ&W"æ–BÀ¢&÷f–FW%öWfVçEö–C¢'VÆÂ×&WVW7C£3‚"çFõ÷7G&–ær‚’À¢¶–æC¢FöÖ–ã£¤VF–Væ6T–çFW&7F–öä¶–æC£¥VÆÅ&WVW7D÷VæVBÀ¢V&Æ–5÷W&Ã¢6öÖR‚&‡GG3¢òöv—F‡V"æ6öÒôå5s2övVçBÖ&÷VçF–W2÷VÆÂó3‚"çFõ÷7G&–ær‚’’À¢ö67W'&VEöC¢æöæRÀ¢&VfW'&W%÷W&Ã¢æöæRÀ¢6×–vã¢6öÖR‚&v—F‡V"Ö&÷VçG’ÖÆ&VÂ"çFõ÷7G&–ær‚’’À¢6÷W&6Uö–çFW&7F–öåö–C¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚“°¢ÆWBòÒ&V6÷&Eö÷WG&V6…öGFV×B€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…&V6÷&D÷WG&V6„GFV×E&WVW7B°¢VF–Væ6UöÖVÖ&W%ö–C¢ÖVÖ&W"æ–BÀ¢&÷f–FW%öWfVçEö–C¢&—77VRÖ6öÖÖVçC¦fVVF&6³£3‚"çFõ÷7G&–ær‚’À¢6†ææVÃ¢FöÖ–ã£¤÷WG&V6„6†ææVÃ£¤v—F‡V%V&Æ–2À¢V&Æ–5÷W&Ã¢6öÖR€¢&‡GG3¢òöv—F‡V"æ6öÒôå5s2övVçBÖ&÷VçF–W2÷VÆÂó3‚6—77VV6öÖÖVçBÓ"çFõ÷7G&–ær‚’À¢’À¢&ö×E÷fW'6–öã¢&F—7G&–'WF–öâ×c"çFõ÷7G&–ær‚’À¢7FGW3¢FöÖ–ã£¤÷WG&V6…7FGW3£¥&W7öæFVBÀ¢6VçEöC¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚“°¢ÆWBòÒ&V6÷&EöF—66÷fW'•÷&W7öç6R€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW'2æ6ÆöæR‚’À¢§6öâ…&V6÷&DF—66÷fW'•&W7öç6U&WVW7B°¢VF–Væ6UöÖVÖ&W%ö–C¢ÖVÖ&W"æ–BÀ¢–çFW&7F–öåö–C¢æöæRÀ¢&÷f–FW%÷&W7öç6Uö–C¢'"Ö&öG“£3‚"çFõ÷7G&–ær‚’À¢V&Æ–5÷6÷W&6U÷W&Ã¢6öÖR€¢&‡GG3¢òöv—F‡V"æ6öÒôå5s2övVçBÖ&÷VçF–W2÷VÆÂó3‚"çFõ÷7G&–ær‚’À¢’À¢f÷VæE÷f–¢$v—D‡V"—77VRÆ—7B"çFõ÷7G&–ær‚’À¢Ö÷F—fF–öã¢&6ÆV"–÷WBÖ–çFVw&—G’66÷R"çFõ÷7G&–ær‚’À¢–×&÷fVÖVçE÷7VvvW7F–öã¢'6†÷rGW&&ÆR–ÖVçBWf–FVæ6R"çFõ÷7G&–ær‚’À¢vVçEö÷%÷FööÃ¢6öÖR‚&6öF–ærvVçB"çFõ÷7G&–ær‚’’À¢&—fFU÷7F÷&vUö6öç6VçC¢fÇ6RÀ¢6GW&VEöC¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚“° ¢ÆWB&W÷'BÒVF–Væ6U÷&W÷'B…7FFR‡7FFR’Â†VFW'2’æv—BçVçw&‚’ã°¢76W'EöW‡&W÷'BçF÷FÅöÖVÖ&W'2Â“°¢76W'EöW‡&W÷'BçF÷FÅö–çFW&7F–öç2Â“°¢76W'EöW‡&W÷'BæÖVÖ&W'5ö6¶VEöf÷%öF—66÷fW'•öfVVF&6²Â“°¢76W'EöW‡&W÷'BæÖVÖ&W'5÷v—F…öF—66÷fW'•÷&W7öç6W2Â“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâWfÅöVæGö–çG5÷&V6÷&EöÆö6Å÷'Våö†—7F÷'’‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“° ¢ÆWB&W7VÇBÒ'Våö&÷VçG–&Væ6‚…7FFR‡7FFRæ6ÆöæR‚’’’æv—BçVçw&‚’ã°¢76W'B‡&W7VÇBç76VB“° ¢ÆWB'Vç2ÒÆ—7EöWfÅ÷'Vç2…7FFR‡7FFR’’æv—Bã°¢76W'EöW‡'Vç2æÆVâ‚’Â“°¢76W'EöW‡'Vç5³Òç7V—FRÂ$&÷VçG”&Væ6‚÷&÷WFW"×c"“°¢76W'B‡'Vç5³Òç76VB“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ&—6µ÷öÆ–7•öVæGö–çEöW‡÷6W5÷6WGFÆVÖVçEöÆ–Ö—G2‚’°¢ÆWBöÆ–7’Ò&—6µ÷öÆ–7’‚’æv—Bã° ¢76W'EöW‡öÆ–7’æÆ÷u÷fÇVU÷W6F5ö6öÖ–æ÷"Âóó“°¢76W'EöW‡öÆ–7’æÆ÷u÷fÇVU÷W6F5ö6ö7W'&Væ7’Â'W6F2"“°¢76W'B‚öÆ–7’æ•ö§VFvW5ö6åöWF†÷&—¦U÷–ÖVçB“°¢76W'B‡öÆ–7¢ç6WGFÆVÖVçEö–çf&–çG0¢æ—FW"‚¢æç’‡Ç'VÆWÂ'VÆRæ6öçF–ç2‚%7G&—RÆVFvW"7&VF—G2"’’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ&—6µöWfVçG5öVæGö–çEöÆ—7G5÷&Wf–Wu÷VWVR‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB&W7VÇBÒæWGv÷&²ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$f—‚FWFW&Ö–æ—7F–2–÷WB&V6öæ6–Æ–F–öâf–ÇW&R"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&f—‚Ö6’Öf–ÇW&R"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢#UóóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¤&6UW6F4W67&÷rÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò“°¢76W'B†ÖF6†W2‡&W7VÇBÂW'"†£¤W'&÷#£¥&—6´æVVG5&Wf–Wr…ò’’’“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWBWfVçG2ÒÆ—7E÷&—6µöWfVçG2€¢7FFR‡7FFR’À¢VW'’…&—6´WfVçDf–ÇFW"°¢7F–öã¢6öÖR†FöÖ–ã£¥&—6´7F–öã£¤æVVG5&Wf–Wr’À¢7W&f6S¢6öÖR†FöÖ–ã£¥&—6µ7W&f6S£¤&÷VçG’’À¢Æ–Ö—C¢6öÖRƒ’À¢âå&—6´WfVçDf–ÇFW#£¦FVfVÇB‚¢Ò’À¢¢æv—@¢ã° ¢76W'EöW†WfVçG2æÆVâ‚’Â“°¢76W'EöW†WfVçG5³Òæ7F–öâÂFöÖ–ã£¥&—6´7F–öã£¤æVVG5&Wf–Wr“°¢76W'B†WfVçG5³Òç&V6öç5³Òæ6öçF–ç2‚&Æ÷r×fÇVR6"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ&—6µö&÷VçG•ö&÷fÅöVæGö–çEö7&VFW5ögVæF–æu÷&VG•ö&÷VçG’‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB&W7VÇBÒæWGv÷&²ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$f—‚FWFW&Ö–æ—7F–2–÷WB&V6öæ6–Æ–F–öâf–ÇW&R"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&f—‚Ö6’Öf–ÇW&R"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢#UóóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¤&6UW6F4W67&÷rÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò“°¢76W'B†ÖF6†W2‡&W7VÇBÂW'"†£¤W'&÷#£¥&—6´æVVG5&Wf–Wr…ò’’’“°¢ÆWB&—6µöWfVçEö–BÒæWGv÷&°¢æÆ—7E÷&—6µöWfVçG2…&—6´WfVçDf–ÇFW"°¢7F–öã¢6öÖR†FöÖ–ã£¥&—6´7F–öã£¤æVVG5&Wf–Wr’À¢7W&f6S¢6öÖR†FöÖ–ã£¥&—6µ7W&f6S£¤&÷VçG’’À¢Æ–Ö—C¢6öÖRƒ’À¢âå&—6´WfVçDf–ÇFW#£¦FVfVÇB‚¢Ò¢æf—'7B‚¢çVçw&‚¢æ–C°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWB&÷fÂÒ&÷fU÷&—6µö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢§6öâ„&÷fU&—6´&÷VçG•&WVW7B°¢&—6µöWfVçEö–BÀ¢F—FÆS¢$f—‚FWFW&Ö–æ—7F–2–÷WB&V6öæ6–Æ–F–öâf–ÇW&R"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&f—‚Ö6’Öf–ÇW&R"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢#UóóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥7G&—Tf–DÆVFvW"À¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢÷W&F÷%ö–C¢&÷W&F÷"Ó"çFõ÷7G&–ær‚’À¢æ÷FS¢$&÷fVBgFW"ÖçVÂ66÷R&Wf–Wr"çFõ÷7G&–ær‚’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW†&÷fÂæ&÷VçG’ç7FGW2Â&÷VçG•7FGW3£¥VægVæFVB“°¢76W'B†&÷fÂæ&÷VçG’çFW&×5ö†6‚æ—5÷6öÖR‚’“°¢76W'EöW†&÷fÂç&Wf–Wræ÷WF6öÖRÂFöÖ–ã£¥&—6µ&Wf–Wt÷WF6öÖS£¤&÷fVB“°¢ÆWB&Wf–Ww2ÒÆ—7E÷&—6µ÷&Wf–Ww2…7FFR‡7FFR’’æv—Bã°¢76W'EöW‡&Wf–Ww2æÆVâ‚’Â“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ&—6µ÷&V¦V7F–öåöVæGö–çE÷&V6÷&G5÷&Wf–Wu÷v—F†÷WEö&÷VçG’‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB&W7VÇBÒæWGv÷&²ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$f—‚FWFW&Ö–æ—7F–2–÷WB&V6öæ6–Æ–F–öâf–ÇW&R"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&f—‚Ö6’Öf–ÇW&R"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢#UóóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¤&6UW6F4W67&÷rÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò“°¢76W'B†ÖF6†W2‡&W7VÇBÂW'"†£¤W'&÷#£¥&—6´æVVG5&Wf–Wr…ò’’’“°¢ÆWB&—6µöWfVçEö–BÒæWGv÷&°¢æÆ—7E÷&—6µöWfVçG2…&—6´WfVçDf–ÇFW"°¢7F–öã¢6öÖR†FöÖ–ã£¥&—6´7F–öã£¤æVVG5&Wf–Wr’À¢7W&f6S¢6öÖR†FöÖ–ã£¥&—6µ7W&f6S£¤&÷VçG’’À¢Æ–Ö—C¢6öÖRƒ’À¢âå&—6´WfVçDf–ÇFW#£¦FVfVÇB‚¢Ò¢æf—'7B‚¢çVçw&‚¢æ–C°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWB&Wf–WrÒ&V¦V7E÷&—6µöWfVçB€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢F‚‡&—6µöWfVçEö–B’À¢§6öâ…&V¦V7E&—6´WfVçE&WVW7B°¢&—6µöWfVçEö–C¢WV–C£¦æ–Â‚’À¢÷W&F÷%ö–C¢&÷W&F÷"Ó"çFõ÷7G&–ær‚’À¢æ÷FS¢%&V¦V7FVBVçF–Â–W"6ö×ÆWFW2ÖçVÂöæ&ö&F–ær"çFõ÷7G&–ær‚’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW‡&Wf–Wræ÷WF6öÖRÂFöÖ–ã£¥&—6µ&Wf–Wt÷WF6öÖS£¥&V¦V7FVB“°¢ÆWBæWGv÷&²Ò7FFRææWGv÷&²æÆö6²‚’çVçw&‚“°¢76W'B†æWGv÷&²æ&÷VçF–W2æ—5öV×G’‚’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâÆÆ×5÷G‡EöVæGö–çE÷ö–çG5övVçG5÷FõöF—66÷fW'•öæEöÖ7‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“° ¢ÆWBFW‡BÒÆÆ×5÷G‡B…7FFR‡7FFR’’æv—C° ¢76W'B‡FW‡Bæ6öçF–ç2‚"2vVçB&÷VçF–W2"’“°¢76W'B‡FW‡Bæ6öçF–ç2‚"òçvVÆÂÖ¶æ÷vâövVçBÖ&÷VçF–W2æ§6öâ"’“°¢76W'B‡FW‡Bæ6öçF–ç2‚&‡GG¢òó#rããã£ƒ“÷FööÇ2"’“°¢76W'B‡FW‡Bæ6öçF–ç2‚'6W'fW"öF—66÷fW""’“°¢76W'B‡FW‡Bæ6öçF–ç2‚'ÆåöWFöæöÖ÷W5ö&÷VçG•ö7&VF–öâ"’“°¢76W'B‡FW‡Bæ6öçF–ç2‚&vVçEöæF—fUö6Æ–Ò"’“°¢76W'B‡FW‡Bæ6öçF–ç2‚$&÷VçG•6WGFÆVB"’“°¢76W'B‚FW‡Bæ6öçF–ç2‚&7&VFTW67&÷r"’“°¢Ð ¢5·FW7EÐ¢fâÆVFW&&ö&E÷&W÷'G5÷–EööæÇ•öf÷%÷F†U÷&æ¶VE÷v–ææW"‚’°¢ÆWB&VfW&Væ6RÒWF0¢çv—F…÷–ÖEöæEö†×2ƒ##bÂrÂrÂ"ÂÂ¢ç6–ævÆR‚¢çVçw&‚“°¢ÆWBW&–öBÒÆVFW&&ö&E÷W&–öB„ÆVFW&&ö&EW&–öD¶–æC£¤F–Ç’Â&VfW&Væ6R“°¢ÆWBVæG5öBÒW&–öBæVæG5öC°¢ÆWB&æ¶–ærÒ&æµ÷6öÇfW%ö6ö×ÆWF–öç2€¢W&–öBÀ¢¶FöÖ–ã£¤6æöæ–6Å6öÇfW$6ö×ÆWF–öâ°¢&÷VçG•ö–C¢&&÷VçG’Ó"çFõ÷7G&–ær‚’À¢&÷VçG•ö6öçG&7C¢#ƒ"çFõ÷7G&–ær‚’À¢6öÇfW%÷vÆÆWC¢#ƒ#######################################""çFõ÷7G&–ær‚’À¢7&VF÷%÷vÆÆWC¢#ƒ3333333333333333333333333333333333333332"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&E÷W6F5ö&6U÷Væ—G3¢%óóÀ¢ö67W'&VEöC¢&VfW&Væ6RÀ¢&Æö6µöçVÖ&W#¢C"À¢Æöuö–æFWƒ¢À¢7FæF–æuöÖWFö&÷VçG“¢fÇ6RÀ¢ÕÒÀ¢“°¢ÆWBVæ6öæf–wW&VBÒÆVFW&&ö&E÷W&–öE÷&W7öç6R€¢&æ¶–æræ6ÆöæR‚’À¢VæG5öB²6‡&öæó£¤GW&F–öã£¦†÷W'2ƒ"’À¢æöæRÀ¢&æ÷Eö6öæf–wW&VB"À¢æöæRÀ¢“°¢76W'EöW‡Væ6öæf–wW&VBç&Wv&E÷–÷WE÷7FGW2Â'&Wv&Eöæ÷Eö6öæf–wW&VB"“° ¢ÆWB–BÒÆVFW&&ö&E÷W&–öE÷&W7öç6R€¢&æ¶–ærÀ¢VæG5öB²6‡&öæó£¤GW&F–öã£¦†÷W'2ƒ"’À¢6öÖR‚#ƒCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCB"çFõ÷7G&–ær‚’’À¢&gVæFVB"À¢6öÖR…6öÇfW$ÆVFW&&ö&Dv&E6fTö'6W'fF–öâ°¢6öçG&7C¢#ƒCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCB"çFõ÷7G&–ær‚’À¢v&Eö–C¢f÷&ÖB‚#‡·Ò"Â#SR"ç&WVBƒ3"’’À¢–E÷v–ææW#¢6öÖR‚#ƒ#######################################""çFõ÷7G&–ær‚’’À¢6fUö&Æö6µöçVÖ&W#¢SÀ¢6fUö&Æö6µö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#cb"ç&WVBƒ3"’’À¢6fUö&Æö6µ÷F–ÖW7F×¢ScC£§G'•ög&öÒ†VæG5öBçF–ÖW7F×‚’’çVçw&‚’À¢Ò’À¢“°¢76W'EöW‡–Bç&Wv&E÷–÷WE÷7FGW2Â'–B"“°¢76W'EöW€¢–Bç&Wv&E÷–E÷vÆÆWBæ5öFW&Vb‚’À¢6öÖR‚#ƒ#######################################""¢“°¢76W'EöW‡–Bç&Wv&E÷–÷WEöö'6W'fVE÷6fUö&Æö6²Â6öÖRƒS’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5ö&÷VçG•öfVVEöW†6ÇVFW5÷&—fFUö&÷VçF–W2‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWBV&Æ–2ÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$f—‚V&Æ–24’"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&f—‚Ö6’Öf–ÇW&R"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢óóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò¢çVçw&‚“°¢ÆWB&—fFRÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢%&—fFRÆVFvW"v÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢%óóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥7G&—Tf–DÆVFvW"À¢&—f7“¢&—f7”ÆWfVÃ£¥&—fFRÀ¢Ò¢çVçw&‚“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWBfVVBÒV&Æ–5ö&÷VçG•öfVVB…7FFR‡7FFR’’æv—Bã° ¢76W'EöW†fVVBæÆVâ‚’Â“°¢76W'EöW†fVVE³Òæ&÷VçG•ö–BÂV&Æ–2æ–BçFõ÷7G&–ær‚’“°¢76W'EöæR†fVVE³Òæ&÷VçG•ö–BÂ&—fFRæ–BçFõ÷7G&–ær‚’“°¢76W'EöW€¢fVVE³Òæ6Æ–Õ÷W&ÂÀ¢f÷&ÖB‚&‡GG¢òó#rããã£ƒƒ÷cö&÷VçF–W2÷·Òö6Æ–Ò"ÂV&Æ–2æ–B¢“°¢76W'EöW€¢fVVE³ÒçV&Æ–5÷W&ÂÀ¢f÷&ÖB‚&‡GG¢òó#rããã£ƒƒ÷V&Æ–2ö&÷VçF–W2÷·Ò"ÂV&Æ–2æ–B¢“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ÷÷'GVæ—G•÷&ö¦V7F–öåö¶VW5÷–ÖVçEöæE÷v÷&µ÷7FFU÷6W&FR‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB6Æ–Ö&ÆRÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$f—‚V&Æ–2’FW7G2"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&f—‚Ö6’Öf–ÇW&R"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢óóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò¢çVçw&‚“°¢ÆWB&—fFRÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢%&—fFRv÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢%óóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥7G&—Tf–DÆVFvW"À¢&—f7“¢&—f7”ÆWfVÃ£¥&—fFRÀ¢Ò¢çVçw&‚“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWB&W7öç6RÒÆ—7Eö÷÷'GVæ—F–W2€¢7FFR‡7FFR’À¢VW'’„÷÷'GVæ—G•VW'’°¢f–Ws¢6öÖR‚'&VG•÷FõöV&â"çFõ÷7G&–ær‚’’À¢âä÷÷'GVæ—G•VW'“£¦FVfVÇB‚¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'B‡&W7öç6RæFVw&FVB“°¢76W'EöW‡&W7öç6RæÆ–VE÷f–Wræ5öFW&Vb‚’Â6öÖR‚'&VG•÷FõöV&â"’“°¢76W'EöW‡&W7öç6Ræ—FV×2æÆVâ‚’Â“°¢76W'EöW‡&W7öç6Ræ—FV×5³Òç6÷W&6Uö–BÂ6Æ–Ö&ÆRæ–BçFõ÷7G&–ær‚’“°¢76W'EöæR‡&W7öç6Ræ—FV×5³Òç6÷W&6Uö–BÂ&—fFRæ–BçFõ÷7G&–ær‚’“°¢76W'EöW‡&W7öç6Ræ—FV×5³Òçv÷&µ÷7FFRÂ&6Æ–Ö&ÆR"“°¢76W'EöW‡&W7öç6Ræ—FV×5³Òç–ÖVçE÷7FFRÂ&W67&÷vVB"“°¢76W'B‡&W7öç6Ræ—FV×5³Òç–ÖVçEö6öÖÖ—GFVB“°¢76W'B‡&W7öç6Ræ—FV×5³Ð¢æF—66÷fW'•öf7F÷'0¢æ—FW"‚¢æç’‡Æf7F÷'Âf7F÷"æ6öçF–ç2‚&6Æ–Ö&ÆR¶W67&÷vVB·fW&–f–6F–öå÷&VG’"’’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ÷÷'GVæ—G•÷&ö¦V7F–öå÷&V¦V7G5÷Væ¶æ÷vå÷f–Ww2‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWBW'&÷"ÒÆ—7Eö÷÷'GVæ—F–W2€¢7FFR‡7FFR’À¢VW'’„÷÷'GVæ—G•VW'’°¢f–Ws¢6öÖR‚&vVçE÷W'6öæ"çFõ÷7G&–ær‚’’À¢âä÷÷'GVæ—G•VW'“£¦FVfVÇB‚¢Ò’À¢¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¤$Eõ$UTU5B“°¢Ð ¢5·FW7EÐ¢fâ÷÷'GVæ—G•öVÖ&VEöÖ÷VçG5öæEöÆ–æ·5ö&UöWf–FVæ6Uö&÷VæB‚’°¢76W'EöW†FV6–ÖÅöÖ÷VçB‚#"Âb’Â#"“°¢76W'EöW†FV6–ÖÅöÖ÷VçB‚##S"Âb’Â#ã#R"“°¢76W'EöW†FV6–ÖÅöÖ÷VçB‚#"Âb’Â#ã"“°¢76W'B‡6fUöW‡FW&æÅ÷W&Â‚&¦f67&—C¦ÆW'Bƒ’"’æ—5öæöæR‚’“°¢76W'EöW‡W&6VçEöVæ6öFU÷F…÷6VvÖVçB‚&ÆVv7“¦ö""’Â&ÆVv7’S4S$f""“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ÷÷'GVæ—G•öVÖ&VE÷&WW6W5öÆ—fU÷&ö¦V7F–öå÷7FFR‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB&÷VçG’ÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$'V–ÆBÇ6fSâ’Fö72"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢ó#SóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò¢çVçw&‚“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“°¢ÆWB–BÒf÷&ÖB‚&ÆVv7“§·Ò"Â&÷VçG’æ–B“° ¢ÆWB‡FÖÂÒ÷÷'GVæ—G•öVÖ&VE÷vR€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†–Bæ6ÆöæR‚’’À¢VW'’„÷÷'GVæ—G”VÖ&VEVW'“£¦FVfVÇB‚’’À¢¢æv—@¢çVçw&‚“°¢76W'EöW†‡FÖÂç7FGW2‚’Â7FGW46öFS£¤ô²“°¢76W'B†‡FÖÂæ†VFW'2‚•¶†VFW#£¤4ôåDTåEõ4T5U$•E•õôÄ”5•Ð¢çFõ÷7G"‚¢çVçw&‚¢æ6öçF–ç2‚&g&ÖRÖæ6W7F÷'2¢"’“°¢ÆWB‡FÖÂÒ‡VÓ£¦&öG“£§Fõö'—FW2†‡FÖÂæ–çFõö&öG’‚’ÂW6—¦S£¤Ô‚¢æv—@¢çVçw&‚“°¢ÆWB‡FÖÂÒ7G&–æs£¦g&öÕ÷WFc‚†‡FÖÂçFõ÷fV2‚’’çVçw&‚“°¢76W'B†‡FÖÂæ6öçF–ç2‚%v÷&³¢6Æ–Ö&ÆR"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚%–ÖVçC¢W67&÷vVB"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚#ã#RU4D2"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚$'V–ÆBfÇC·6fRfwC²’Fö72"’“° ¢ÆWB7frÒ÷÷'GVæ—G•öVÖ&VE÷7fr€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†–Bæ6ÆöæR‚’’À¢VW'’„÷÷'GVæ—G”VÖ&VEVW'“£¦FVfVÇB‚’’À¢¢æv—@¢çVçw&‚“°¢76W'EöW€¢7fræ†VFW'2‚•¶†VFW#£¤4ôåDTåEõE•UÒÀ¢&–ÖvR÷7fr·†ÖÃ²6†'6WC×WFbÓ‚ ¢“° ¢ÆWBÖ&¶F÷vâÒ÷÷'GVæ—G•öVÖ&VEöÖ&¶F÷vâ€¢7FFR‡7FFR’À¢F‚†–B’À¢VW'’„÷÷'GVæ—G”VÖ&VEVW'“£¦FVfVÇB‚’’À¢¢æv—@¢çVçw&‚“°¢76W'EöW€¢Ö&¶F÷vâæ†VFW'2‚•¶†VFW#£¤4ôåDTåEõE•UÒÀ¢'FW‡BöÖ&¶F÷vã²6†'6WC×WFbÓ‚ ¢“°¢Ð ¢5·FW7EÐ¢fâF—66÷fW'•÷7V'67&—F–öåöf–ÇFW'5ö&Uö&÷VæFVEöæEöæ÷&ÖÆ—¦VB‚’°¢ÆWB×WBf–ÇFW'2ÒF—66÷fW'•7V'67&—F–öäf–ÇFW'2°¢6¶–ÆÇ3¢fV2²"'W7B"çFõ÷7G&–ær‚’Â''W7B"çFõ÷7G&–ær‚•ÒÀ¢6FVv÷&–W3¢fV2²&Væv–æVW&–ær"çFõ÷7G&–ær‚•ÒÀ¢Ö–æ–×VÕö6öÖÖ—GFVE÷&Wv&C¢6öÖR†FöÖ–ã£¤F—66÷fW'•&Wv&Df–ÇFW"°¢Ö÷VçC¢#"çFõ÷7G&–ær‚’À¢7W'&Væ7“¢"W6F2"çFõ÷7G&–ær‚’À¢Væ—C¢"$4UõTä•E2"çFõ÷7G&–ær‚’À¢FV6–ÖÇ3¢bÀ¢Ò’À¢v÷&µ÷7FFW3¢fV2²&6Æ–Ö&ÆR"çFõ÷7G&–ær‚•ÒÀ¢–ÖVçE÷7FFW3¢fV2²&W67&÷vVB"çFõ÷7G&–ær‚•ÒÀ¢fW&–f–6F–öåöÖWF†öG3¢fV2²&FWFW&Ö–æ—7F–5öÖöGVÆR"çFõ÷7G&–ær‚•ÒÀ¢6÷W&6U÷G—W3¢fV2²&6æöæ–6Åö&6R"çFõ÷7G&–ær‚•ÒÀ¢FVFÆ–æU÷v—F†–åö†÷W'3¢6öÖRƒs"’À¢Ó°¢æ÷&ÖÆ—¦UöF—66÷fW'•öf–ÇFW'2‚f×WBf–ÇFW'2’çVçw&‚“°¢76W'EöW†f–ÇFW'2ç6¶–ÆÇ2ÂfV2²%'W7B%Ò“°¢ÆWBÖ–æ–×VÒÒf–ÇFW'2æÖ–æ–×VÕö6öÖÖ—GFVE÷&Wv&BçVçw&‚“°¢76W'EöW†Ö–æ–×VÒæ7W'&Væ7’Â%U4D2"“°¢76W'EöW†Ö–æ–×VÒçVæ—BÂ&&6U÷Væ—G2"“° ¢f–ÇFW'2ÒF—66÷fW'•7V'67&—F–öäf–ÇFW'2°¢–ÖVçE÷7FFW3¢fV2²&gVæFVBÖ—6‚"çFõ÷7G&–ær‚•ÒÀ¢âäF—66÷fW'•7V'67&—F–öäf–ÇFW'3£¦FVfVÇB‚¢Ó°¢76W'EöW€¢æ÷&ÖÆ—¦UöF—66÷fW'•öf–ÇFW'2‚f×WBf–ÇFW'2’À¢W'"…7FGW46öFS£¤$Eõ$UTU5B¢“°¢Ð ¢5·FW7EÐ¢fâF—66÷fW'•öÖævVÖVçE÷Fö¶Våö6ö×&—6öåöFöW5öæ÷Eö66WE÷&Vf—†W2‚’°¢76W'B†6öç7FçE÷F–ÖU÷FW‡EöW‚&&3#2"Â&&3#2"’“°¢76W'B‚6öç7FçE÷F–ÖU÷FW‡EöW‚&&3#2"Â&&3#B"’“°¢76W'B‚6öç7FçE÷F–ÖU÷FW‡EöW‚&&3#2"Â&&2"’“°¢Ð ¢5·FW7EÐ¢fâ6öçfW'6–öåö6÷'&VÆF–öåö66WG5ööæÇ•öW†7Eö†÷7FVE÷VægVæFVE÷W&Ç2‚’°¢ÆWB–BÒWV–C£¦æWu÷cB‚“°¢ÆWB&6RÒ&‡GG3¢òö’ævVçF&÷VçF–W2æ#°¢76W'EöW€¢VægVæFVEö&÷VçG•ö–Eög&öÕ÷6÷W&6R‚ff÷&ÖB‚'¶&6WÒ÷c÷VægVæFVBÖ&÷VçF–W2÷¶–GÒ"’Â&6R’À¢6öÖR†–B¢“°¢76W'EöW€¢VægVæFVEö&÷VçG•ö–Eög&öÕ÷6÷W&6R€¢ff÷&ÖB‚&‡GG3¢òöWf–ÂæW†×ÆR÷c÷VægVæFVBÖ&÷VçF–W2÷¶–GÒ"’À¢&6P¢’À¢æöæP¢“°¢76W'EöW€¢VægVæFVEö&÷VçG•ö–Eög&öÕ÷6÷W&6R€¢ff÷&ÖB‚'¶&6WÒ÷c÷VægVæFVBÖ&÷VçF–W2÷¶–GÓ÷7ööc×G'VR"’À¢&6P¢’À¢æöæP¢“°¢Ð ¢5·FW7EÐ¢fâ6öçfW'6–öå÷&W7öç6UöæWfW%ö–æfW'5ö–æFWVæFVçEö7F—fUövVçG2‚’°¢ÆWB&W7öç6RÒ÷÷'GVæ—G•ö6öçfW'6–öå÷&W7öç6R€¢÷÷'GVæ—G”Æ–fV7–6ÆU7FG2°¢V&Æ—6†VC¢À¢6öÇWF–öå÷&V6V—fVC¢bÀ¢gVæF–æu÷&W&VC¢BÀ¢vÆÆWE÷6–væVEöö'6W'fVC¢2À¢6æöæ–6Åö7&VFVC¢2À¢gVæFVC¢"À¢6Æ–ÖVC¢"À¢7V&Ö—GFVC¢À¢6WGFÆVC¢À¢fW&vU÷6V6öæG5÷Fõöf—'7E÷6öÇWF–öã¢6öÖRƒ#ã’À¢ÖVF–å÷6V6öæG5÷Fõöf—'7E÷6öÇWF–öã¢6öÖRƒ“ã’À¢fW&vU÷6V6öæG5ö7&VF–öå÷Fõ÷6WGFÆVÖVçC¢6öÖRƒ5ócã’À¢6æöæ–6Åö7&VFVEö–å÷v–æF÷s¢RÀ¢6æöæ–6Åö6Æ–ÖVEö–å÷v–æF÷s¢BÀ¢6æöæ–6Å÷6WGFÆVEö–å÷v–æF÷s¢2À¢Væ—VUö6æöæ–6Å÷÷7FW%÷vÆÆWG3¢BÀ¢&WVEö6æöæ–6Å÷÷7FW%÷vÆÆWG3¢À¢Væ—VU÷–E÷6öÇfW%÷vÆÆWG3¢2À¢&WVE÷–E÷6öÇfW%÷vÆÆWG3¢À¢ÒÀ¢s#À¢WF3£¦æ÷r‚’Ò6‡&öæôGW&F–öã£¦†÷W'2ƒs#’À¢WF3£¦æ÷r‚’À¢“°¢76W'EöW‡&W7öç6Rç7FvW2æÆVâ‚’Â’“°¢76W'EöW‡&W7öç6Rç&FW5³ÒæÖWG&–2Â'VægVæFVE÷FõögVæFVEö6öçfW'6–öâ"“°¢76W'EöW‡&W7öç6Rç&FW5³ÒçfÇVRÂ6öÖRƒã"’“°¢76W'EöW‡&W7öç6Ræ7F÷'2æ–æFWVæFVçEö7F—fUövVçG2ÂæöæR“°¢76W'B‚&W7öç6Ræ7F÷'2æ–æFWVæFVæ6UöÖV7W&VÖVçEöf–Æ&ÆR“°¢76W'B‡&W7öç6P¢æ7F÷'0¢æWf–FVæ6Uö&÷VæF'¢æ6öçF–ç2‚'vÆÆWB—2æ÷B&ööb"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5ögVæF–æuöfVVEöÆ—7G5ööæÇ•÷V&Æ–5ö&÷VçF–W5÷v—F…÷&VÖ–æ–æuögVæF–ær‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB'F–ÂÒ÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$gVæBV&Æ–2Fö72"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWB÷'F–ÅögVæF–ærÒFEögVæF–æuö6öçG&–'WF–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚‡'F–Âæ–B’À¢§6öâ„FDgVæF–æt6öçG&–'WF–öå&WVW7B°¢&÷VçG•ö–C¢'F–Âæ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢æöæRÀ¢Ö÷VçEöÖ–æ÷#¢CÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢&–Ã¢–ÖVçE&–Ã£¥6–×VÆFVBÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR‚'V&Æ–2ÖgVæF–ærÖfVVB×'F–Â"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWB7G&—RÒ÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$gVæBV&Æ–27G&—Rv÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'–ÖVçB×7FFRÖÖ6†–æR"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢SÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥7G&—Tf–DÆVFvW"À¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWB&—fFRÒ÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$gVæB&—fFRv÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥&—fFRÀ¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWBgVæFVBÒ÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$gVæFVBV&Æ–2v÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWBögVæFVEö6öçG&–'WF–öâÒFEögVæF–æuö6öçG&–'WF–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†gVæFVBæ–B’À¢§6öâ„FDgVæF–æt6öçG&–'WF–öå&WVW7B°¢&÷VçG•ö–C¢gVæFVBæ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢æöæRÀ¢Ö÷VçEöÖ–æ÷#¢óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢&–Ã¢–ÖVçE&–Ã£¥6–×VÆFVBÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR‚'V&Æ–2ÖgVæF–ærÖfVVBÖgVæFVB"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢ÆWBfVVBÒV&Æ–5ögVæF–æuöfVVB…7FFR‡7FFRæ6ÆöæR‚’’’æv—Bã°¢ÆWB–G2ÒfVV@¢æ—FW"‚¢æÖ‡Æ—FV×Â—FVÒæ&÷VçG•ö–Bæ6ÆöæR‚’¢æ6öÆÆV7C££ÅfV3Åóãâ‚“° ¢76W'B†–G2æ6öçF–ç2‚g'F–Âæ–BçFõ÷7G&–ær‚’’“°¢76W'B†–G2æ6öçF–ç2‚g7G&—Ræ–BçFõ÷7G&–ær‚’’“°¢76W'B‚–G2æ6öçF–ç2‚g&—fFRæ–BçFõ÷7G&–ær‚’’“°¢76W'B‚–G2æ6öçF–ç2‚fgVæFVBæ–BçFõ÷7G&–ær‚’’“°¢ÆWB'F–Åö—FVÒÒfVV@¢æ—FW"‚¢æf–æB‡Æ—FV×Â—FVÒæ&÷VçG•ö–BÓÒ'F–Âæ–BçFõ÷7G&–ær‚’¢æW‡V7B‚''F–ÂV&Æ–2&÷VçG’6†÷VÆB&R–âgVæF–ærfVVB"“°¢76W'EöW‡'F–Åö—FVÒægVæF–æu÷&VÖ–æ–æuöÖ–æ÷"Âc“°¢76W'B‡'F–Åö—FVÐ¢ægVæF–æu÷'F—F–öç0¢æ—FW"‚¢æç’‡Ç'F—F–öçÂ'F—F–öâç&VÖ–æ–æuöÖ–æ÷"ÓÒc’“° ¢ÆWB‡FÖÂÒV&Æ–5ögVæF–æuöfVVE÷vR…7FFR‡7FFR’’æv—Bã°¢76W'B†‡FÖÂæ6öçF–ç2‚$gVæF&ÆRvVçB&÷VçF–W2"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚&vVçBÖ&÷VçG’ÖgVæF–ærÖfVVB"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷V&Æ–2ö&÷VçF–W2÷·Ò"Â'F–Âæ–B’’“°¢76W'B†‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷cö&÷VçF–W2÷·ÒögVæF–ærÖ–çFVçG2"Â'F–Âæ–B’’“°¢76W'B‚‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷V&Æ–2ö&÷VçF–W2÷·Ò"Â&—fFRæ–B’’“°¢76W'B‚‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷V&Æ–2ö&÷VçF–W2÷·Ò"ÂgVæFVBæ–B’’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5ö&÷VçG•öFWF–ÅöW‡÷6W5övVçEö7F–öç2‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB&÷VçG’ÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$f—‚V&Æ–2Ä4“â"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&f—‚Ö6’Öf–ÇW&R"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢óóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò¢çVçw&‚“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWB‡FÖÂÒV&Æ–5ö&÷VçG•÷vR…7FFR‡7FFR’ÂF‚†&÷VçG’æ–B’¢æv—@¢çVçw&‚¢ã° ¢76W'B†‡FÖÂæ6öçF–ç2‚$f—‚V&Æ–2fÇC´4’fwC²"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚$gVæF–ær7FFR"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚$gVæF–ær'F—F–öç2"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚&Æ–6F–öâöÆB¶§6öâ"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚&vVçBÖ&÷VçG’×V&Æ–2×7FGW2"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚$Ö6†–æR7FGW2"’“°¢76W'B†‡FÖÂæ6öçF–ç2‡"2&FFÖvVçBÖ7F–öãÒ&6Æ–Ò""2’“°¢76W'B‚‡FÖÂæ6öçF–ç2‚$FBgVæF–ær"’“°¢76W'B‚‡FÖÂæ6öçF–ç2‡"2'&VÃÒ'–ÖVçB""2’“°¢76W'B†‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷V&Æ–2ö&÷VçF–W2÷·Ò"Â&÷VçG’æ–B’’“°¢76W'B†‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷cö&÷VçF–W2÷·Òö6Æ–Ò"Â&÷VçG’æ–B’’“°¢76W'B‚‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷cö&÷VçF–W2÷·ÒögVæF–ærÖ6öçG&–'WF–öç2"Â&÷VçG’æ–B’’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5ö&÷VçG•öFWF–ÅöW‡÷6W5ö6ögVæF–æu÷v†–ÆU÷F&vWE÷&VÖ–ç2‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB&÷VçG’Ò÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$gVæB6†&VBV&Æ–2v÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢óóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢ÆWB'F–ÂÒFEögVæF–æuö6öçG&–'WF–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†&÷VçG’æ–B’À¢§6öâ„FDgVæF–æt6öçG&–'WF–öå&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢æöæRÀ¢Ö÷VçEöÖ–æ÷#¢CóÀ¢7W'&Væ7“¢%U4D2"çFõ÷7G&–ær‚’À¢&–Ã¢–ÖVçE&–Ã£¥6–×VÆFVBÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR‚''F–Â×V&Æ–2×vR"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡'F–Âæ&÷VçG’ç7FGW2Â&÷VçG•7FGW3£¥VægVæFVB“°¢76W'EöW‡'F–ÂægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræÖ÷VçBÂcó“° ¢ÆWB‡FÖÂÒV&Æ–5ö&÷VçG•÷vR…7FFR‡7FFR’ÂF‚†&÷VçG’æ–B’¢æv—@¢çVçw&‚¢ã° ¢76W'B†‡FÖÂæ6öçF–ç2‚''F–ÆÇ’gVæFVB"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚$6òÖgVæF–ær6öÖÖæC¢"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚ff÷&ÖB€¢"övVçBÖ&÷VçG’gVæB·ÒãbU4D2f–6–×VÆFVB"À¢&÷VçG’æ–@¢’’“°¢76W'B†‡FÖÂæ6öçF–ç2‡"2'&VÃÒ'–ÖVçB""2’“°¢76W'B†‡FÖÂæ6öçF–ç2‡"2&FFÖvVçBÖ7F–öãÒ&FEögVæF–æuöWf–FVæ6R""2’“°¢76W'B‚‡FÖÂæ6öçF–ç2‡"2&FFÖvVçBÖ7F–öãÒ&7&VFUögVæF–æuö–çFVçB""2’“°¢76W'B†‡FÖÂæ6öçF–ç2‚ff÷&ÖB‚"÷cö&÷VçF–W2÷·ÒögVæF–ærÖ6öçG&–'WF–öç2"Â&÷VçG’æ–B’’“°¢76W'B‚‡FÖÂæ6öçF–ç2‡"2&FFÖvVçBÖ7F–öãÒ&6Æ–Ò""2’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5ö&÷VçG•öFWF–Åö†–FW5÷&—fFUö&÷VçF–W2‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB&—fFRÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢%&—fFRÆVFvW"v÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢%óóÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥7G&—Tf–DÆVFvW"À¢&—f7“¢&—f7”ÆWfVÃ£¥&—fFRÀ¢Ò¢çVçw&‚“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWBW'&÷"ÒV&Æ–5ö&÷VçG•÷vR…7FFR‡7FFR’ÂF‚‡&—fFRæ–B’¢æv—@¢çVçw&öW'"‚“° ¢76W'EöW†W'&÷"Â7FGW46öFS£¤äõEôdõTäB“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ&6Uö'&öF67E÷6–væVE÷G&ç67F–öåöVæGö–çE÷&WGW&ç5÷G…ö†6‚‚’°¢ÆWB'5÷W&ÂÒ7vå÷'5÷&W7öç6R‡6W&FUö§6öã£¦§6öâ‡°¢&§6öç'2#¢#"ã"À¢&–B#¢2À¢'&W7VÇB#¢f÷&ÖB‚#‡·Ò"Â&62"ç&WVBƒ3"’¢Ò’“°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…ö&6U÷'2„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â'5÷W&Â“° ¢ÆWB&W÷'BÒ'&öF67Eö&6U÷6–væVE÷G&ç67F–öâ€¢7FFR‡7FFR’À¢†VFW$Ö£¦æWr‚’À¢§6öâ„'&öF67D&6U6–væVEG&ç67F–öå&WVW7B°¢6–væVE÷G&ç67F–öã¢#ƒ#2"çFõ÷7G&–ær‚’À¢&WVW7Eö–C¢6öÖRƒ2’À¢æWGv÷&³¢6öÖR‚&&6R×6WöÆ–"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢76W'EöW‡&W÷'BææWGv÷&²æ6†–åö–BÂƒEóS3"“°¢76W'EöW‡&W÷'Bç&WVW7BæÖWF†öBÂ&WF…÷6VæE&uG&ç67F–öâ"“°¢76W'EöW‡&W÷'Bç&WVW7Bç&×5³ÒÂ#ƒ#2"“°¢76W'EöW‡&W÷'BçG…ö†6‚Âf÷&ÖB‚#‡·Ò"Â&62"ç&WVBƒ3"’’“°¢76W'B‡&W÷'BææW‡E÷7FWæ6öçF–ç2‚'G&ç67F–öâ×&V6V—B"’“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâ÷W&F÷%÷Fö¶Våö&Æö6·5÷&÷FV7FVEö•ö6ÆÇ5÷v†Våö6öæf–wW&VB‚’°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶Vâ„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â'6V7&WB×Fö¶Vâ"“° ¢ÆWBW'&÷"Ò'&öF67Eö&6U÷6–væVE÷G&ç67F–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢§6öâ„'&öF67D&6U6–væVEG&ç67F–öå&WVW7B°¢6–væVE÷G&ç67F–öã¢#ƒ#2"çFõ÷7G&–ær‚’À¢&WVW7Eö–C¢6öÖRƒ2’À¢æWGv÷&³¢6öÖR‚&&6R×6WöÆ–"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†W'&÷"Â7FGW46öFS£¥TäUD„õ$•¤TB“° ¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B‚&WF†÷&—¦F–öâ"Â$&V&W"6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWBW'&÷"Ò'&öF67Eö&6U÷6–væVE÷G&ç67F–öâ€¢7FFR‡7FFR’À¢†VFW'2À¢§6öâ„'&öF67D&6U6–væVEG&ç67F–öå&WVW7B°¢6–væVE÷G&ç67F–öã¢#ƒ#2"çFõ÷7G&–ær‚’À¢&WVW7Eö–C¢6öÖRƒ2’À¢æWGv÷&³¢6öÖR‚&&6R×6WöÆ–"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†W'&÷"Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâÆVv7•÷fW&–f–6F–öå÷&WV—&W5ö6öæf–wW&VEö÷W&F÷%öWF†÷&—¦F–öâ‚’°¢ÆWB&÷VçG•ö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB&WVW7BÒfW&–g•7V&Ö—76–öå&WVW7B°¢&÷VçG•ö–C¢WV–C£¦æ–Â‚’À¢7V&Ö—76–öåö–C¢WV–C£¦æWu÷cB‚’À¢W‡V7FVEö'F–f7EöF–vW7C¢#†FVF&VVb"çFõ÷7G&–ær‚’À¢fW&–f–W%ö¶–æC¢6öÖR…fW&–f–W$¶–æC£¤§6öå66†VÖ’À¢'V'&–3¢æöæRÀ¢Wf–FVæ6S¢æöæRÀ¢&÷fVE÷&—6µöWfVçEö–C¢æöæRÀ¢Ó° ¢ÆWBW'&÷"ÒfW&–g•÷7V&Ö—76–öâ€¢7FFR‡FW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’’’À¢F‚†&÷VçG•ö–B’À¢†VFW$Ö£¦æWr‚’À¢§6öâ‡&WVW7Bæ6ÆöæR‚’’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†W'&÷"Â7FGW46öFS£¥4U%d”4UõTäd”Ä$ÄR“° ¢ÆWB7FFRÒFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶Vâ„&÷VçG”æWGv÷&³£¦FVfVÇB‚’Â'6V7&WB×Fö¶Vâ"“°¢ÆWBW'&÷"ÒfW&–g•÷7V&Ö—76–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†&÷VçG•ö–B’À¢†VFW$Ö£¦æWr‚’À¢§6öâ‡&WVW7Bæ6ÆöæR‚’’À¢¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†W'&÷"Â7FGW46öFS£¥TäUD„õ$•¤TB“° ¢ÆWB×WB†VFW'2Ò†VFW$Ö£¦æWr‚“°¢†VFW'2æ–ç6W'B‚&WF†÷&—¦F–öâ"Â$&V&W"6V7&WB×Fö¶Vâ"ç'6R‚’çVçw&‚’“°¢ÆWBW'&÷"ÒfW&–g•÷7V&Ö—76–öâ…7FFR‡7FFR’ÂF‚†&÷VçG•ö–B’Â†VFW'2Â§6öâ‡&WVW7B’¢æv—@¢çVçw&öW'"‚“°¢76W'EöW†W'&÷"Â7FGW46öFS£¤$Eõ$UTU5B“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5ö6&–Æ—G•÷6V&6…öf–æG5÷&Vv—7FW&VE÷6öÇfW'2‚’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB6öÇfW"ÒæWGv÷&²ç&Vv—7FW%övVçB…&Vv—7FW$vVçE&WVW7B°¢†æFÆS¢&6&–Æ—G’×6öÇfW""çFõ÷7G&–ær‚’À¢–÷WE÷vÆÆWC¢6öÖR‚#ƒ#######################################""çFõ÷7G&–ær‚’’À¢Ò“°¢æWGv÷&°¢ç&Vv—7FW%ö6&–Æ—G’…&Vv—7FW$6&–Æ—G•&WVW7B°¢vVçEö–C¢6öÇfW"æ–BÀ¢6Æ73¢6&–Æ—G”6Æ73£¤6öF–ærÀ¢FV×ÆFU÷6ÇVw3¢fV2²'6ÖÆÂÖ6öFRÖ6†ævR"çFõ÷7G&–ær‚•ÒÀ¢Ö–å÷&–6UöÖ–æ÷#¢SóÀ¢Ö…÷&–6UöÖ–æ÷#¢óóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢ÆFVæ7•÷6V6öæG3¢cÀ¢7W÷'FVE÷fW&–f–W'3¢fV2µfW&–f–W$¶–æC£¤§6öå66†VÖÒÀ¢Ò¢çVçw&‚“°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWBfVVBÒV&Æ–5ö6&–Æ—G•öfVVB…7FFR‡7FFRæ6ÆöæR‚’’’æv—Bã°¢76W'EöW†fVVBæÆVâ‚’Â“°¢76W'EöW†fVVE³ÒævVçEö–BÂ6öÇfW"æ–BçFõ÷7G&–ær‚’“°¢76W'EöW€¢fVVE³ÒævVçE÷&öf–ÆU÷W&ÂÀ¢f÷&ÖB‚&‡GG¢òó#rããã£ƒƒ÷V&Æ–2övVçG2÷·Ò"Â6öÇfW"æ–B¢“° ¢ÆWB6V&6‚Ò6V&6…ö6&–Æ—F–W2€¢7FFR‡7FFR’À¢§6öâ…6V&6„6&–Æ—F–W5&WVW7B°¢6Æ73¢6öÖR„6&–Æ—G”6Æ73£¤6öF–ær’À¢FV×ÆFU÷6ÇVs¢6öÖR‚'6ÖÆÂÖ6öFRÖ6†ævR"çFõ÷7G&–ær‚’’À¢7W'&Væ7“¢6öÖR‚%U4D2"çFõ÷7G&–ær‚’’À¢Ö…÷&–6UöÖ–æ÷#¢6öÖRƒcó’À¢Ò’À¢¢æv—@¢ã° ¢76W'EöW‡6V&6‚æÆVâ‚’Â“°¢76W'EöW‡6V&6…³ÒævVçEö†æFÆRÂ&6&–Æ—G’×6öÇfW""“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâööÆVEögVæF–æuöVæGö–çG5öÖ¶Uö&÷VçG•ö6Æ–Ö&ÆUöE÷F&vWB‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“° ¢ÆWB&÷VçG’Ò÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$gVæB6†&VBFö72v÷&²"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢'w&—FRÖFö72Öf÷"Ö&V"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢óÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢ÆWB'F–ÂÒFEögVæF–æuö6öçG&–'WF–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†&÷VçG’æ–B’À¢§6öâ„FDgVæF–æt6öçG&–'WF–öå&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢æöæRÀ¢Ö÷VçEöÖ–æ÷#¢CÀ¢7W'&Væ7“¢%U4D2"çFõ÷7G&–ær‚’À¢&–Ã¢–ÖVçE&–Ã£¥6–×VÆFVBÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR‚&f—'7B"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡'F–Âæ&÷VçG’ç7FGW2Â&÷VçG•7FGW3£¥VægVæFVB“°¢76W'EöW‡'F–ÂægVæF–æu÷7VÖÖ'’ç&VÖ–æ–æræÖ÷VçBÂc“° ¢ÆWBgVæFVBÒFEögVæF–æuö6öçG&–'WF–öâ€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†&÷VçG’æ–B’À¢§6öâ„FDgVæF–æt6öçG&–'WF–öå&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢æöæRÀ¢Ö÷VçEöÖ–æ÷#¢cÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢&–Ã¢–ÖVçE&–Ã£¥6–×VÆFVBÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR‚'6V6öæB"çFõ÷7G&–ær‚’’À¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢76W'EöW†gVæFVBæ&÷VçG’ç7FGW2Â&÷VçG•7FGW3£¤6Æ–Ö&ÆR“°¢76W'B†gVæFVBægVæF–æu÷7VÖÖ'’æ6Æ–Ö&ÆR“°¢76W'EöW†gVæFVBægVæF–æu÷7VÖÖ'’æ6öçG&–'WF–öåö6÷VçBÂ"“° ¢ÆWB7FGW2Ò&÷VçG•÷7FGW2…7FFR‡7FFRæ6ÆöæR‚’’ÂF‚†&÷VçG’æ–B’¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡7FGW2ægVæF–æuö6öçG&–'WF–öç2æÆVâ‚’Â"“°¢76W'EöW‡7FGW2ægVæF–æu÷7VÖÖ'’æÆ–VBæÖ÷VçBÂó“°¢ÆWBfVVBÒÆ—7Eö6Æ–Ö&ÆUö&÷VçF–W2…7FFR‡7FFR’’æv—Bã°¢76W'EöW†fVVBæÆVâ‚’Â“°¢76W'EöW†fVVE³Òæ–BÂ&÷VçG’æ–B“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâgVæF–æuö–çFVçEöVæGö–çE÷v—G5öf÷%÷fW&–f–VE÷7G&—U÷vV&†öö²‚’°¢ÆWB7FFRÒFW7E÷7FFU÷v—F…÷Vç6–væVE÷7G&—U÷vV&†öö·2„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB÷&væ—¦F–öåö–BÒWV–C£¦æWu÷cB‚“°¢ÆWB&÷VçG’Ò÷Vå÷ööÆVEö&÷VçG’€¢7FFR‡7FFRæ6ÆöæR‚’’À¢§6öâ„÷VåööÆVD&÷VçG•&WVW7B°¢&÷VçG•ö–C¢æöæRÀ¢–FV×÷FVæ7•ö¶W“¢æöæRÀ¢F—FÆS¢$gVæB7G&—R’–çFVçB"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&W‡G&7BÖFF×Fò×66†VÖ"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçEöÖ–æ÷#¢SÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥7G&—Tf–DÆVFvW"À¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢gVæF–æu÷F&vWG3¢fV2µÒÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã° ¢ÆWB–çFVçBÒ7&VFUögVæF–æuö–çFVçB€¢7FFR‡7FFRæ6ÆöæR‚’’À¢F‚†&÷VçG’æ–B’À¢§6öâ„7&VFTgVæF–æt–çFVçE&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öçG&–'WF÷%övVçEö–C¢æöæRÀ¢6÷W&6Uö÷&væ—¦F–öåö–C¢6öÖR†÷&væ—¦F–öåö–B’À¢Ö÷VçEöÖ–æ÷#¢SÀ¢7W'&Væ7“¢'W6B"çFõ÷7G&–ær‚’À¢&–Ã¢–ÖVçE&–Ã£¥7G&—Tf–BÀ¢W‡FW&æÅ÷&VfW&Væ6S¢6öÖR‚&’×7G&—RÖ–çFVçB"çFõ÷7G&–ær‚’’À¢7G&—U÷7V66W75÷W&Ã¢æöæRÀ¢7G&—Uö6æ6VÅ÷W&Ã¢æöæRÀ¢Ò’À¢¢æv—@¢çVçw&‚¢ã°¢76W'B†–çFVçBç&WV—&W5÷&V6öæ6–Æ–F–öâ“°¢76W'EöW†–çFVçBægVæF–æu÷7VÖÖ'’æÆ–VBæÖ÷VçBÂ“° ¢ÆWB7FGW5ö&Vf÷&RÒ&÷VçG•÷7FGW2…7FFR‡7FFRæ6ÆöæR‚’’ÂF‚†&÷VçG’æ–B’¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡7FGW5ö&Vf÷&RægVæF–æuö–çFVçG2æÆVâ‚’Â“°¢76W'B‡7FGW5ö&Vf÷&RægVæF–æuö6öçG&–'WF–öç2æ—5öV×G’‚’“° ¢ÆWBWfVçBÒ6W&FUö§6öã£¦§6öâ‡°¢&–B#¢&WgEö•ö–çFVçB"À¢'G—R#¢&6†V6¶÷WBç6W76–öâæ6ö×ÆWFVB"À¢'–ÆöB#¢°¢&–B#¢&75ö•ö–çFVçB"À¢&6Æ–VçE÷&VfW&Væ6Uö–B#¢÷&væ—¦F–öåö–BçFõ÷7G&–ær‚’À¢&Ö÷VçE÷F÷FÂ#¢SÀ¢&7W'&Væ7’#¢'W6B"À¢'–ÖVçE÷7FGW2#¢'–B"À¢'–ÖVçEö–çFVçB#¢'•ö•ö–çFVçB"À¢&ÖWFFF#¢°¢&&÷VçG•ö–B#¢&÷VçG’æ–BçFõ÷7G&–ær‚’À¢&gVæF–æuö–çFVçEö–B#¢–çFVçBæ–çFVçBæ–BçFõ÷7G&–ær‚¢Ð¢Ð¢Ò“°¢ÆWB&V6öæ6–Æ–F–öâÒ&V6öæ6–ÆU÷7G&—Uö6†V6¶÷WE÷vV&†öö²€¢7FFR‡7FFRæ6ÆöæR‚’’À¢†VFW$Ö£¦æWr‚’À¢'—FW3£¦g&öÒ‡6W&FUö§6öã£§Fõ÷fV2‚fWfVçB’çVçw&‚’’À¢¢æv—@¢çVçw&‚¢ã°¢76W'B‡&V6öæ6–Æ–F–öâægVæF–æu÷&W÷'Bæ—5÷6öÖR‚’“°¢76W'EöW‡&V6öæ6–Æ–F–öâæÆVFvW%öVçG&–W2æÆVâ‚’Â"“° ¢ÆWB7FGW5ögFW"Ò&÷VçG•÷7FGW2…7FFR‡7FFR’ÂF‚†&÷VçG’æ–B’¢æv—@¢çVçw&‚¢ã°¢76W'EöW‡7FGW5ögFW"ægVæF–æuö6öçG&–'WF–öç2æÆVâ‚’Â“°¢76W'EöW‡7FGW5ögFW"ægVæF–æu÷7VÖÖ'’æÆ–VBæÖ÷VçBÂS“°¢76W'B‡7FGW5ögFW"ægVæF–æu÷7VÖÖ'’æ6Æ–Ö&ÆR“°¢Ð ¢5·Fö¶–ó£§FW7EÐ¢7–æ2fâV&Æ–5÷fW&–f–W%÷&öf–ÆU÷7VÖÖ&—¦W5÷fW&–f–W%÷&W7VÇG2‚’°¢ÆWB†æWGv÷&²Âö&÷VçG’Â÷&ööb’Ò6ö×ÆWFVE÷6–×VÆFVEö&÷VçG’‚’æv—C°¢ÆWB7FFRÒFW7E÷7FFR†æWGv÷&²“° ¢ÆWB‡FÖÂÒV&Æ–5÷fW&–f–W%÷&öf–ÆR…7FFR‡7FFR’ÂF‚‚$§6öå66†VÖ"çFõ÷7G&–ær‚’’¢æv—@¢çVçw&‚¢ã° ¢76W'B†‡FÖÂæ6öçF–ç2‚$§6öå66†VÖfW&–f–W""’“°¢76W'B†‡FÖÂæ6öçF–ç2‚%F÷FÂ6†V6·2"’“°¢76W'B†‡FÖÂæ6öçF–ç2‚#ÆGCä66WFVCÂöGCãÆFCãÂöFCâ"’“°¢Ð ¢fâFW7E÷7FFR†æWGv÷&³¢&÷VçG”æWGv÷&²’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢æöæRÀ¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢fÇ6RÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&Ã¢5E$•Uô•ô$4UõU$ÂçFõ÷7G&–ær‚’À¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢æöæRÀ¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢æöæRÀ¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7Eö6Æ÷VEövVçB‚’Óâ&3Ä6Æ÷VDvVçE6W'f–6Sâ°¢&3£¦æWr„6Æ÷VDvVçE6W'f–6S£¦g&öÕöVçb‚’æW‡V7B‚&F—6&ÆVBFW7B6Æ÷VBvVçB—2fÆ–B"’¢Ð ¢fâ÷7Fw&W5÷FW7EöFF&6U÷W&Â‚’Óâ7G&–ær°¢7FC£¦Vçc£§f"‚$tTåEô$õTåD”U5õDU5EôDD$4UõU$Â"¢æW‡V7B‚$tTåEô$õTåD”U5õDU5EôDD$4UõU$Â×W7B&R6WBf÷"–væ÷&VB÷7Fw&W27–æ2FW7G2"¢Ð ¢fâFW7E÷7FFU÷v—F…÷Vç6–væVE÷7G&—U÷vV&†öö·2†æWGv÷&³¢&÷VçG”æWGv÷&²’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢G'VRÀ¢7G&—U÷6V7&WEö¶W“¢æöæRÀ¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢fÇ6RÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&Ã¢5E$•Uô•ô$4UõU$ÂçFõ÷7G&–ær‚’À¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢æöæRÀ¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢æöæRÀ¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7E÷7FFU÷v—F…÷7G&—U÷vV&†ööµ÷6V7&WB†æWGv÷&³¢&÷VçG”æWGv÷&²Â6V7&WC¢e·S…Ò’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢6öÖR‡6V7&WBçFõ÷fV2‚’’À¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢æöæRÀ¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢fÇ6RÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&Ã¢5E$•Uô•ô$4UõU$ÂçFõ÷7G&–ær‚’À¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢æöæRÀ¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢æöæRÀ¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶Vâ†æWGv÷&³¢&÷VçG”æWGv÷&²ÂFö¶Vã¢g7G"’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢æöæRÀ¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢fÇ6RÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&Ã¢5E$•Uô•ô$4UõU$ÂçFõ÷7G&–ær‚’À¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢æöæRÀ¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢6öÖR‡Fö¶VâçFõ÷7G&–ær‚’’À¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7E÷7FFU÷v—F…ö÷W&F÷%÷Fö¶VåöæE÷7F÷&R€¢æWGv÷&³¢&÷VçG”æWGv÷&²À¢Fö¶Vã¢g7G"À¢7F÷&S¢÷7Fw&W57F÷&RÀ¢’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢æöæRÀ¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢fÇ6RÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&Ã¢5E$•Uô•ô$4UõU$ÂçFõ÷7G&–ær‚’À¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢æöæRÀ¢7F÷&S¢6öÖR‡7F÷&R’À¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢6öÖR‡Fö¶VâçFõ÷7G&–ær‚’’À¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7E÷7FFU÷v—F…ö&6U÷'2€¢æWGv÷&³¢&÷VçG”æWGv÷&²À¢&6U÷6WöÆ–÷'5÷W&Ã¢7G&–ærÀ¢’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢æöæRÀ¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢fÇ6RÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&Ã¢5E$•Uô•ô$4UõU$ÂçFõ÷7G&–ær‚’À¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢æöæRÀ¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–r°¢&6U÷6WöÆ–¢6öÖR†&6U÷6WöÆ–÷'5÷W&Â’À¢&6UöÖ–ææWC¢æöæRÀ¢ÒÀ¢&6Uö'&öF67EöVæ&ÆVC¢G'VRÀ¢÷W&F÷%ö•÷Fö¶Vã¢æöæRÀ¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7E÷7FFU÷v—F…÷7G&—UöÆ—fR€¢æWGv÷&³¢&÷VçG”æWGv÷&²À¢7G&—Uö•ö&6U÷W&Ã¢7G&–ærÀ¢’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢6öÖR‚'6µ÷FW7EöÖö6²"çFõ÷7G&–ær‚’’À¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢G'VRÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&ÂÀ¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢æöæRÀ¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢æöæRÀ¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7E÷7FFU÷v—F…÷7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ€¢æWGv÷&³¢&÷VçG”æWGv÷&²À¢–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢g7G"À¢’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢æöæRÀ¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢fÇ6RÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢fÇ6RÀ¢7G&—Uö•ö&6U÷W&Ã¢5E$•Uô•ô$4UõU$ÂçFõ÷7G&–ær‚’À¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢6öÖR‡–ÖVçEöÖWF†öEö6öæf–wW&F–öâçFõ÷7G&–ær‚’’À¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢æöæRÀ¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâFW7E÷7FFU÷v—F…÷7G&—U÷V&Æ–5ö6†V6¶÷WEöæE÷–ÖVçEöÖWF†öEö6öæf–wW&F–öâ€¢æWGv÷&³¢&÷VçG”æWGv÷&²À¢7G&—Uö•ö&6U÷W&Ã¢7G&–ærÀ¢–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢g7G"À¢’Óâ6†&VE7FFR°¢&3£¦æWr„7FFR°¢æWGv÷&³¢&3£¦æWr„×WFWƒ£¦æWr†æWGv÷&²’’À¢WfÅ÷'Vç3¢&3£¦æWr„×WFWƒ£¦æWr…fV3£¦æWr‚’’’À¢7G&—U÷vV&†ööµ÷6V7&WC¢æöæRÀ¢ÆÆ÷u÷Vç6–væVE÷7G&—U÷vV&†öö·3¢fÇ6RÀ¢7G&—U÷6V7&WEö¶W“¢6öÖR‚'6µ÷FW7EöÖö6²"çFõ÷7G&–ær‚’’À¢7G&—UöÆ—fUöW†V7WF–öåöVæ&ÆVC¢G'VRÀ¢7G&—U÷V&Æ–5ö6†V6¶÷WEöVæ&ÆVC¢G'VRÀ¢7G&—Uö•ö&6U÷W&ÂÀ¢7G&—U÷–ÖVçEöÖWF†öEö6öæf–wW&F–öã¢6öÖR‡–ÖVçEöÖWF†öEö6öæf–wW&F–öâçFõ÷7G&–ær‚’’À¢7F÷&S¢æöæRÀ¢&6U÷'5÷W&Ç3¢&6U'5W&Ä6öæf–s£¦FVfVÇB‚’À¢&6Uö'&öF67EöVæ&ÆVC¢fÇ6RÀ¢÷W&F÷%ö•÷Fö¶Vã¢æöæRÀ¢F—66÷fW&&–Æ—G•ö–ævW7E÷Fö¶Vã¢æöæRÀ¢æÇ—F–75öW†6ÇW6–öå÷Fö¶Vã¢æöæRÀ¢F—7G&–'WF–öåöGG&–'WF–öå÷6–væ–æu÷6V7&WC¢æöæRÀ¢F—7G&–'WF–öåöW†6ÇVFVE÷vÆÆWEö6Æ76W3¢F#£¤D•5E$”%UD”ôåôU„4ÅU4”ôåô4Ä54U0¢æ—FW"‚¢æÖ‡ÇfÇVWÂ‚§fÇVR’çFõ÷7G&–ær‚’¢æ6öÆÆV7B‚’À¢V&Æ–5ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒƒ"çFõ÷7G&–ær‚’À¢Ö7ö&6U÷W&Ã¢&‡GG¢òó#rããã£ƒ“"çFõ÷7G&–ær‚’À¢ƒC%÷&VÆ–W#¢ƒC$†÷7FVE&VÆ–W$6öæf–s£¦FVfVÇB‚’À¢&öæE÷7öç6÷#¢&öæE7öç6÷$6öæf–s£¦FVfVÇB‚’À¢&V6÷fW'•÷&W6W'fF–öç3¢WFöæöÖ÷W4&÷VçG•&V6÷fW'•&W6W'fF–öç3£¦FVfVÇB‚’À¢6Æ÷VEövVçC¢FW7Eö6Æ÷VEövVçB‚’À¢F—66÷fW'•÷vV&†öö·3¢æöæRÀ¢æW–æ%÷6ö6–Ã¢æöæRÀ¢Ò¢Ð ¢fâ7vå÷'5÷&W7öç6R‡&W7öç6S¢6W&FUö§6öã£¥fÇVR’Óâ7G&–ær°¢ÆWBÆ—7FVæW"ÒF7Æ—7FVæW#£¦&–æB‚##rããã£"’çVçw&‚“°¢ÆWBFG&W72ÒÆ—7FVæW"æÆö6ÅöFG"‚’çVçw&‚“°¢F‡&VC£§7vâ†Ö÷fRÇÂ°¢ÆWB†×WB7G&VÒÂò’ÒÆ—7FVæW"æ66WB‚’çVçw&‚“°¢ÆWB×WB'VffW"Ò³Sƒ²ƒ“%Ó°¢ÆWBòÒ7G&VÒç&VB‚f×WB'VffW"’çVçw&‚“°¢ÆWB&öG’Ò&W7öç6RçFõ÷7G&–ær‚“°¢ÆWB&W7öç6RÒf÷&ÖB€¢$…EEóã#ôµÇ%Ææ6öçFVçB×G—S¢Æ–6F–öâö§6öåÇ%Ææ6öçFVçBÖÆVæwFƒ¢·ÕÇ%Ææ6öææV7F–öã¢6Æ÷6UÇ%ÆåÇ%Æç·Ò"À¢&öG’æÆVâ‚’À¢&öG¢“°¢7G&VÒçw&—FUöÆÂ‡&W7öç6Ræ5ö'—FW2‚’’çVçw&‚“°¢Ò“°¢f÷&ÖB‚&‡GG¢ò÷¶FG&W77Ò"¢Ð ¢5·FW7EÐ¢fâ–çfVçF÷'•÷7VÖÖ'•÷W6W5ö6æöæ–6ÅöfVVEöÖ÷VçG5÷v—F†÷WE÷7FF–5ö6÷VçG2‚’°¢ÆWB7FFRÒFW7E÷7FFR„&÷VçG”æWGv÷&³£¦FVfVÇB‚’“°¢ÆWB7VÖÖ'’Ò'V–ÆEöWFöæöÖ÷W5ö–çfVçF÷'•÷7VÖÖ'’€¢g7FFRÀ¢&&6RÖÖ–ææWB"À¢fV2´WFöæöÖ÷W4&÷VçG”fVVD—FVÒ°¢&÷VçG•ö–C¢f÷&ÖB‚#‡·Ò"Â#"ç&WVBƒ3"’’À¢&÷VçG•ö6öçG&7C¢f÷&ÖB‚#‡·Ò"Â##""ç&WVBƒ#’’À¢7&VF÷#¢f÷&ÖB‚#‡·Ò"Â#32"ç&WVBƒ#’’À¢7FGW3¢&6Æ–Ö&ÆR"çFõ÷7G&–ær‚’À¢6öÇfW%÷&Wv&C¢#“"çFõ÷7G&–ær‚’À¢fW&–f–W%÷&Wv&C¢#"çFõ÷7G&–ær‚’À¢6Æ–Õö&öæC¢#"çFõ÷7G&–ær‚’À¢F–ÖV÷WEö&öæE÷ööÃ¢#"çFõ÷7G&–ær‚’À¢F&vWEöÖ÷VçC¢#"çFõ÷7G&–ær‚’À¢gVæFVEöÖ÷VçC¢#"çFõ÷7G&–ær‚’À¢&WV—&VEöW‡FW&æÅ÷7VæC¢#"çFõ÷7G&–ær‚’À¢w&÷75ö66…öÖ&v–ã¢#“"çFõ÷7G&–ær‚’À¢FW&×5ö†6ƒ¢f÷&ÖB‚#‡·Ò"Â#CB"ç&WVBƒ3"’’À¢FW&×3¢æöæRÀ¢FW&×5÷fÆ–C¢G'VRÀ¢fW&–f–6F–öåöÖöFS¢&FWFW&Ö–æ—7F–5öÖöGVÆR"çFõ÷7G&–ær‚’À¢fW&–f–W%öÖöGVÆS¢æöæRÀ¢fW&–f–W%÷6WEö†6ƒ¢æöæRÀ¢fW&–f–W%÷F‡&W6†öÆC¢6öÖRƒ’À¢'VææW%ö–FVçF–f–W#¢6öÖR‚'FW7Eöf—‡GW&R"çFõ÷7G&–ær‚’’À¢fW&–f–6F–öå÷&VG“¢G'VRÀ¢fW&–f–6F–öå÷&VF–æW75÷&V6öã¢'FW7Bf—‡GW&R"çFõ÷7G&–ær‚’À¢fÆ–FF–öåöW'&÷'3¢fV3£¦æWr‚’À¢WfVçG3¢fV3£¦æWr‚’À¢ÕÒÀ¢¢çVçw&‚“° ¢76W'EöW‡7VÖÖ'’æ6Æ–Ö&ÆUö&÷VçG•ö6÷VçBÂ“°¢76W'EöW‡7VÖÖ'’çfW&–f–6F–öå÷&VG•ö&÷VçG•ö6÷VçBÂ“°¢76W'EöW‡7VÖÖ'’ægVæFVE÷W6F2Â#ã"“°¢76W'EöW‡7VÖÖ'’ç6öÇfW%÷&Wv&E÷W6F2Â#ã“"“°¢76W'EöW‡7VÖÖ'’çfW&–f–W%÷&Wv&E÷W6F2Â#ã"“°¢76W'B‡7VÖÖ'’æ6æöæ–6Å÷6÷W&6Ræ6öçF–ç2‚&6Æ–Ö&ÆUööæÇ“×G'VR"’“°¢Ð ¢5·FW7EÐ¢fâ•ö&–æEöFG%÷&VfW'5öW‡Æ–6—Eö6öæf–u÷F†Våö†÷7E÷÷'B‚’°¢76W'EöW€¢6W'f–6Uö&–æEöFG"…6öÖR‚#ããã£“"’Â6öÖR‚#"’Â##rããã£ƒƒ"’À¢#ããã£“ ¢“°¢76W'EöW€¢6W'f–6Uö&–æEöFG"…6öÖR‚""’Â6öÖR‚#"’Â##rããã£ƒƒ"’À¢#ããã£ ¢“°¢76W'EöW€¢6W'f–6Uö&–æEöFG"„æöæRÂ6öÖR‚""’Â##rããã£ƒƒ"’À¢#ããã£ ¢“°¢76W'EöW€¢6W'f–6Uö&–æEöFG"„æöæRÂæöæRÂ##rããã£ƒƒ"’À¢##rããã£ƒƒ ¢“°¢Ð ¢7–æ2fâ6ö×ÆWFVE÷6–×VÆFVEö&÷VçG’‚’Óâ„&÷VçG”æWGv÷&²Â&÷VçG’Â&ööe&V6÷&B’°¢ÆWB×WBæWGv÷&²Ò&÷VçG”æWGv÷&³£¦FVfVÇB‚“°¢ÆWB6öÇfW"ÒæWGv÷&²ç&Vv—7FW%övVçB…&Vv—7FW$vVçE&WVW7B°¢†æFÆS¢'6öÇfW""çFõ÷7G&–ær‚’À¢–÷WE÷vÆÆWC¢6öÖR‚#ƒ#######################################""çFõ÷7G&–ær‚’’À¢Ò“°¢ÆWB&÷VçG’ÒæWGv÷&°¢ç÷7EögVæFVEö&÷VçG’…÷7D&÷VçG•&WVW7B°¢F—FÆS¢$W‡G&7BFF"çFõ÷7G&–ær‚’À¢FV×ÆFU÷6ÇVs¢&W‡G&7BÖFF×Fò×66†VÖ"çFõ÷7G&–ær‚’À¢Ö÷VçEöÖ–æ÷#¢óóÀ¢7W'&Væ7“¢'W6F2"çFõ÷7G&–ær‚’À¢gVæF–æuöÖöFS¢gVæF–ætÖöFS£¥6–×VÆFVBÀ¢&—f7“¢&—f7”ÆWfVÃ£¥V&Æ–2À¢Ò¢çVçw&‚“°¢æWGv÷&°¢æ6Æ–Õö&÷VçG’„6Æ–Ô&÷VçG•&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öÇfW%övVçEö–C¢6öÇfW"æ–BÀ¢Ò¢çVçw&‚“°¢ÆWB'F–f7BÒ'µÂ&öµÂ#§G'VWÒ#°¢ÆWB7V&Ö—76–öâÒæWGv÷&°¢ç7V&Ö—E÷&W7VÇB…7V&Ö—E&W7VÇE&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢6öÇfW%övVçEö–C¢6öÇfW"æ–BÀ¢'F–f7E÷W&“¢'33¢òö’ö'F–f7Bæ§6öâ"çFõ÷7G&–ær‚’À¢'F–f7Eö&öG“¢'F–f7BçFõ÷7G&–ær‚’À¢Ò¢çVçw&‚“°¢ÆWB&ööbÒæWGv÷&°¢çfW&–g•÷7V&Ö—76–öâ…fW&–g•7V&Ö—76–öå&WVW7B°¢&÷VçG•ö–C¢&÷VçG’æ–BÀ¢7V&Ö—76–öåö–C¢7V&Ö—76–öâæ–BÀ¢W‡V7FVEö'F–f7EöF–vW7C¢W‡V7FVEöF–vW7Eöf÷%ö&öG’†'F–f7B’À¢fW&–f–W%ö¶–æC¢6öÖR†FöÖ–ã£¥fW&–f–W$¶–æC£¤§6öå66†VÖ’À¢'V'&–3¢æöæRÀ¢Wf–FVæ6S¢æöæRÀ¢&÷fVE÷&—6µöWfVçEö–C¢æöæRÀ¢Ò¢æv—@¢çVçw&‚“°¢†æWGv÷&²Â&÷VçG’Â&ööb¢Ð ¢fâ7G&—Uö6†V6¶÷WEöWfVçEö&öG’€¢WfVçEö–C¢g7G"À¢6W76–öåö–C¢g7G"À¢÷&væ—¦F–öåö–C¢WV–BÀ¢’ÓâfV3ÇSƒâ°¢6W&FUö§6öã£§Fõ÷fV2‚g6W&FUö§6öã£¦§6öâ‡°¢&–B#¢WfVçEö–BÀ¢'G—R#¢&6†V6¶÷WBç6W76–öâæ6ö×ÆWFVB"À¢'–ÆöB#¢°¢&–B#¢6W76–öåö–BÀ¢&6Æ–VçE÷&VfW&Væ6Uö–B#¢÷&væ—¦F–öåö–BçFõ÷7G&–ær‚’À¢&Ö÷VçE÷F÷FÂ#¢UóÀ¢&7W'&Væ7’#¢'W6B"À¢'–ÖVçE÷7FGW2#¢'–B"À¢'–ÖVçEö–çFVçB#¢'•÷–B ¢Ð¢Ò’¢çVçw&‚¢Ð ¢fâ7G&—U÷6–væGW&Uö†VFW"‡–ÆöC¢e·S…ÒÂ6V7&WC¢e·S…Ò’Óâ7G&–ær°¢ÆWBF–ÖW7F×ÒWF3£¦æ÷r‚’çF–ÖW7F×‚“°¢ÆWB×WB6–væVE÷–ÆöBÒF–ÖW7F×çFõ÷7G&–ær‚’æ–çFõö'—FW2‚“°¢6–væVE÷–ÆöBçW6‚†"râr“°¢6–væVE÷–ÆöBæW‡FVæEög&öÕ÷6Æ–6R‡–ÆöB“°¢ÆWB×WBÖ2ÒFW7D†Ö56†#Sc£¦æWuög&öÕ÷6Æ–6R‡6V7&WB’çVçw&‚“°¢Ö2çWFFR‚g6–væVE÷–ÆöB“°¢f÷&ÖB€¢'C×·ÒÇc×·Ò"À¢F–ÖW7F×À¢†Wƒ£¦Væ6öFR†Ö2æf–æÆ—¦R‚’æ–çFõö'—FW2‚’¢¢Ð ¢fâfÆ–Eöv—F‡V%ö—77VUö&öG’‚’Óâ7G&–ær°¢fÆ–Eöv—F‡V%ö—77VUö&öG•÷v—F…ögVæF–æuöÖöFR‚%7G&—Tf–DÆVFvW""¢Ð ¢fâv—F‡V%ö—77VUö•÷7–æ5÷&WVW7B€¢—77VUöçVÖ&W#¢ScBÀ¢F—FÆS¢g7G"À¢&öG“¢7G&–ærÀ¢’ÓâÆäv—D‡V$—77VT•7–æ5&WVW7B°¢Æäv—D‡V$—77VT•7–æ5&WVW7B°¢&W÷6—F÷'“¢&vVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2"çFõ÷7G&–ær‚’À¢—77VU÷W&Ã¢f÷&ÖB€¢&‡GG3¢òöv—F‡V"æ6öÒövVçBÖ&÷VçF–W2övVçBÖ&÷VçF–W2ö—77VW2÷¶—77VUöçVÖ&W'Ò ¢’À¢F—FÆS¢F—FÆRçFõ÷7G&–ær‚’À¢&öG’À¢•ö&6U÷W&Ã¢6öÖR‚&‡GG3¢òö’ævVçF&÷VçF–W2æW†×ÆR"çFõ÷7G&–ær‚’’À¢W†—7F–æuö&÷VçG•ö–G3¢fV2µÒÀ¢†÷7FVEö•öW'&÷#¢æöæRÀ¢Ð¢Ð ¢fâfÆ–Eöv—F‡V%ö—77VUö&öG•÷v—F…övöÂ†vöÃ¢g7G"’Óâ7G&–ær°¢f÷&ÖB€¢"2"222vöÀ§¶vöÇÐ ¢22266WFæ6R7&—FW&–¥F†RFW7B¦ö"—2w&VVâæBF†RF6‚W‡Æ–ç2F†Rf–ÇW&Rà ¢222FV×ÆFP¦f—‚Ö6’Öf–ÇW&P ¢2227VvvW7FVBÖ÷Vç@£U4D0 ¢222gVæF–ærÖöFP¥7G&—Tf–DÆVFvW ¢"0¢¢Ð ¢fâfÆ–Eöv—F‡V%ö—77VUö&öG•÷v—F…ögVæF–æuöÖöFR†gVæF–æuöÖöFS¢g7G"’Óâ7G&–ær°¢f÷&ÖB€¢"2"222vöÀ¤f—‚F†Rf–Æ–ær4’6†V6²à ¢22266WFæ6R7&—FW&–¥F†RFW7B¦ö"—2w&VVâæBF†RF6‚W‡Æ–ç2F†Rf–ÇW&Rà ¢222FV×ÆFP¦f—‚Ö6’Öf–ÇW&P ¢2227VvvW7FVBÖ÷Vç@£U4D0 ¢222gVæF–ærÖöFP§¶gVæF–æuöÖöFWÐ¢"0¢¢Ð§Ð