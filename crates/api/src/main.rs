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
    BaseIndexerHeartbeat, ChatgptActionIntent as DbChatgptActionIntent, ChatgptActionObservation,
    ClaimCandidateReservation, ClaimFunnelStats, DbError, GitHubIssueSyncBountyUpsert,
    NewBondSponsorship, NewChatgptActionIntent, NewClaimCandidate, NewDiscoveryWebhookSubscription,
    NewLegalAcceptance, NewOpenCompetitionEntrantRelay, NewOpportunityComment,
    NewSiteAnalyticsEvent, NewSocialMentionIngestion, NewTrialBounty, NewUnfundedBountySolution,
    NewX402RelayAttempt, ObservedInterface, ObservedProtocolEra, OpenCompetitionEntrantRelay,
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
    analytics_exclusion_token: Option<String>,
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
            "{}/post.html?from=social-mention&socialDraft={id}",
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
        analytics_exclusion_token: env::var("ANALYTICS_EXCLUSION_TOKEN")
            .ok()
            .and_then(non_empty_secret),
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
        .route("/llms.txt", get(llms_txt))
        .route("/.well-known/agent-card.json", get(agent_card))
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

/// A2A 1.0 Agent Card served at `/.well-known/agent-card.json`.
/// The response sets an `ETag` and `Cache-Control` header so A2A clients cache the card.
/// See `docs/a2a-direct-api-binding-v1.md` for the canonical binding.
async fn agent_card(State(state): State<SharedState>) -> Response {
    use axum::http::header;
    let body = web_public::render_agent_card(&state.public_base_url);
    let etag = format!("\"{}\"", web_public::agent_card_etag(&body));
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300"),
            ),
            (header::ETAG, HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("\"\""))),
        ],
        body,
    )
        .into_response()
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
        proof_fee,
        relay_fee,
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
    let target_path = if uri.path() == "/" { home } else { uri.path() };
    let query = uri
        .query()
        .map_or(String::new(), |value| format!("?{value}"));
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
            | "unfunded_post_started"
            | "unfunded_post_completed"
            | "funding_started"
            | "claim_started"
            | "claim_confirmed"
            | "competition_entry_started"
            | "competition_entry_confirmed"
            | "competition_reveal_started"
            | "competition_reveal_confirmed"
            | "canonical_post_started"
            | "canonical_post_confirmed"
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
        source: normalize_site_analytics_token(request.source)?,
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
    let session_count = |event_name: &str| {
        stats
            .event_counts
            .iter()
            .find(|count| count.event_name == event_name)
            .map(|count| count.sessions)
            .unwrap_or(0)
    };
    let rate = |metric: &str, numerator: &str, denominator: &str, cohort: &str| {
        let numerator_sessions = session_count(numerator);
        let denominator_sessions = session_count(denominator);
        SiteAnalyticsRateResponse {
            metric: metric.to_string(),
            numerator_sessions,
            denominator_sessions,
            value: (denominator_sessions > 0)
                .then(|| numerator_sessions as f64 / denominator_sessions as f64),
            cohort: cohort.to_string(),
        }
    };
    let rates = vec![
        rate(
            "market_to_funded_bounty_click",
            "funded_bounty_click",
            "market_view",
            "sessions that loaded live market inventory",
        ),
        rate(
            "canonical_post_completion",
            "canonical_post_confirmed",
            "canonical_post_started",
            "sessions that began the wallet-backed canonical post flow",
        ),
        rate(
            "market_to_funding_start",
            "funding_started",
            "market_view",
            "sessions that loaded live market inventory",
        ),
        rate(
            "claim_confirmation",
            "claim_confirmed",
            "claim_started",
            "sessions that began a claim flow",
        ),
    ];
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
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title} · Agent Bounties</title><style>:root{{color-scheme:light dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif}}*{{box-sizing:border-box}}body{{margin:0;padding:12px;background:transparent}}article{{max-width:720px;border:1px solid #6b728066;border-radius:16px;padding:20px;background:#111827;color:#f9fafb;box-shadow:0 12px 36px #0003}}header{{display:flex;justify-content:space-between;gap:12px;align-items:flex-start}}.brand{{font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:#93c5fd}}h1{{font-size:21px;line-height:1.25;margin:7px 0 16px}}.states{{display:flex;flex-wrap:wrap;gap:8px}}.pill{{padding:5px 9px;border-radius:999px;background:#1f2937;font-size:12px}}dl{{display:grid;grid-template-columns:max-content 1fr;gap:8px 14px;margin:18px 0}}dt{{color:#9ca3af}}dd{{margin:0;overflow-wrap:anywhere}}footer{{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}}a{{color:#bfdbfe}}a.cta{{display:inline-block;background:#2563eb;color:white;text-decoration:none;padding:10px 14px;border-radius:9px;font-weight:700}}.proof{{font-size:12px}}.muted{{color:#9ca3af}}</style></head><body><article data-opportunity-id="{}"><header><div><div class="brand">Agent Bounties opportunity</div><h1>{title}</h1></div><div class="states"><span class="pill">Work: {work_state}</span><span class="pill">Payment: {payment_state}</span></div></header><dl><dt>Committed reward</dt><dd>{reward}</dd><dt>Deadline</dt><dd>{deadline}</dd><dt>Verification</dt><dd>{verification}</dd></dl><footer>{latest}<a class="cta" href="{link}" target="_blank" rel="noopener noreferrer">{cta}</a></footer></article></body></html>"#,
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
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="720" height="240" role="img" aria-label="Agent Bounties opportunity: {title}"><title>Agent Bounties opportunity: {title}</title><rect width="720" height="240" rx="18" fill="#111827"/><rect x="1" y="1" width="718" height="238" rx="17" fill="none" stroke="#4b5563"/><text x="28" y="34" fill="#93c5fd" font-family="Arial,sans-serif" font-size="12" letter-spacing="1.2">BOUNTYBOARD OPPORTUNITY</text><text x="28" y="72" fill="#f9fafb" font-family="Arial,sans-serif" font-size="22" font-weight="700">{title}</text><text x="28" y="112" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Work: {work}  ·  Payment: {payment}</text><text x="28" y="142" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Committed reward: {reward}</text><text x="28" y="172" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Deadline: {deadline}</text><text x="28" y="202" fill="#d1d5db" font-family="Arial,sans-serif" font-size="14">Verification: {verification}</text><a href="{link}" target="_blank"><rect x="550" y="184" width="142" height="36" rx="8" fill="#2563eb"/><text x="621" y="207" text-anchor="middle" fill="#fff" font-family="Arial,sans-serif" font-size="13" font-weight="700">View opportunity</text></a></svg>"##,
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
        format!("{truncated}…")
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
        upgrade_url: "https://agentbounties.app/post.html".to_string(),
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
            "compatibility": "https://agentbounties.app/x402.html",
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
        return Ok(attempt);
    }

    let deadline = Instant::now() + Duration::from_secs(state.x402_relayer.wait_seconds);
    loop {
        attempt = reconcile_x402_relay(state, attempt).await?;
        if attempt.status != X402RelayStatus::Broadcast || Instant::now() >= deadline {
            return Ok(attempt);
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn x402_relay_error_is_retryable(error: &ChainBaseError) -> bool {
    match error {
        ChainBaseError::InvalidRelayerPrivateKey
        | ChainBaseError::InvalidRelayIntent(_)
        | ChainBaseError::RelayerChainMismatch { .. }
        | ChainBaseError::RelayerGasLimitExceeded { .. } => false,
        ChainBaseError::RelayerProvider(message) => {
            !message.to_ascii_lowercase().contains("revert")
        }
        _ => true,
    }
}

async fn reconcile_x402_relay(
    state: &SharedState,
    attempt: X402RelayAttempt,
) -> Result<X402RelayAttempt, StatusCode> {
    tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        reconcile_x402_relay_inner(state, attempt),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

async fn reconcile_x402_relay_inner(
    state: &SharedState,
    attempt: X402RelayAttempt,
) -> Result<X402RelayAttempt, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let tx_hash = attempt
        .tx_hash
        .as_deref()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&attempt.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let response = fetch_transaction_receipt(&rpc_url, tx_hash, 1)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(receipt) = response.result else {
        return Ok(attempt);
    };
    if receipt.succeeded().map_err(|_| StatusCode::BAD_GATEWAY)? == Some(false) {
        return store
            .mark_x402_relay_failed(
                attempt.id,
                None,
                false,
                "transaction_reverted",
                "The hosted relay transaction reverted; no canonical funding was applied.",
            )
            .await
            .map_err(map_x402_db_error);
    }
    let Some(block_number) = receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
    else {
        return Ok(attempt);
    };
    let latest_block = fetch_block_number(&rpc_url, 2)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let observed_confirmations = latest_block.saturating_sub(block_number).saturating_add(1);
    if observed_confirmations < state.x402_relayer.confirmations {
        return Ok(attempt);
    }
    let confirmed_response = fetch_transaction_receipt(&rpc_url, tx_hash, 3)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(confirmed_receipt) = confirmed_response.result else {
        return Ok(attempt);
    };
    if confirmed_receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        != Some(block_number)
        || confirmed_receipt
            .succeeded()
            .map_err(|_| StatusCode::BAD_GATEWAY)?
            != Some(true)
    {
        return Ok(attempt);
    }
    let decoded = decode_autonomous_bounty_logs(
        confirmed_receipt
            .logs_to_evm_logs()
            .map_err(|_| StatusCode::BAD_GATEWAY)?,
    )
    .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let matching_events = decoded
        .into_iter()
        .filter(|event| {
            event
                .contract_address
                .eq_ignore_ascii_case(&attempt.bounty_contract)
                && event.tx_hash.eq_ignore_ascii_case(tx_hash)
        })
        .collect::<Vec<_>>();
    let funding_event = matching_events.iter().find(|event| {
        event.kind == chain_base::AutonomousBountyEventKind::FundingAdded
            && event.data["contributor"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&attempt.contributor))
            && json_u128(&event.data["amount"]) == Some(u128::from(attempt.amount))
    });
    let Some(funding_event) = funding_event else {
        return store
            .mark_x402_relay_failed(
                attempt.id,
                None,
                false,
                "canonical_event_mismatch",
                "The confirmed transaction did not emit the exact canonical FundingAdded event.",
            )
            .await
            .map_err(map_x402_db_error);
    };
    let funding_event_id = funding_event.id;
    for event in &matching_events {
        store
            .upsert_autonomous_bounty_event(&attempt.network, event)
            .await
            .map_err(map_x402_db_error)?;
    }
    store
        .mark_x402_relay_confirmed(attempt.id, funding_event_id, block_number)
        .await
        .map_err(map_x402_db_error)
}

fn json_u128(value: &serde_json::Value) -> Option<u128> {
    value
        .as_u64()
        .map(u128::from)
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn map_x402_db_error(error: DbError) -> StatusCode {
    match error {
        DbError::X402RelayConflict(_) => StatusCode::CONFLICT,
        DbError::X402RelayQuotaExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn x402_relay_response(
    state: &SharedState,
    attempt: &X402RelayAttempt,
) -> Result<Response, StatusCode> {
    let status_url = format!(
        "{}/v1/x402/base/relays/{}",
        state.public_base_url.trim_end_matches('/'),
        attempt.id
    );
    if attempt.status == X402RelayStatus::Confirmed {
        let transaction = attempt
            .tx_hash
            .clone()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let descriptor = base_network_descriptor(&attempt.network)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let settlement = SettlementResponse {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some(attempt.contributor.clone()),
            transaction: transaction.clone(),
            network: format!("eip155:{}", descriptor.chain_id),
            amount: Some(attempt.amount.to_string()),
            extensions: None,
            extra: Some(BTreeMap::from([
                ("relayId".to_string(), serde_json::json!(attempt.id)),
                (
                    "canonicalEvent".to_string(),
                    serde_json::json!("FundingAdded"),
                ),
                (
                    "canonicalEventId".to_string(),
                    serde_json::json!(attempt.canonical_event_id),
                ),
                (
                    "confirmedBlock".to_string(),
                    serde_json::json!(attempt.confirmed_block),
                ),
            ])),
        };
        let encoded = encode_payment_response_header(&settlement)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut response = (
            StatusCode::OK,
            Json(serde_json::json!({
                "x402Version": X402_VERSION,
                "scheme": AGENT_BOUNTY_FUND_SCHEME,
                "status": "confirmed",
                "settled": true,
                "relay": x402_public_relay(attempt),
                "statusUrl": status_url,
                "settlement": settlement,
                "canonicalSuccessEvent": "FundingAdded",
                "paymentResponseHeaderPresent": true
            })),
        )
            .into_response();
        response.headers_mut().insert(
            HeaderName::from_static(PAYMENT_RESPONSE_HEADER),
            HeaderValue::from_str(&encoded).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, private"),
        );
        return Ok(response);
    }

    let (status, next_step) = match attempt.status {
        X402RelayStatus::Prepared | X402RelayStatus::Relaying => (
            StatusCode::ACCEPTED,
            "The hosted relay is queued. Poll statusUrl; do not treat this as funding.",
        ),
        X402RelayStatus::Broadcast => (
            StatusCode::ACCEPTED,
            "The transaction was broadcast. Poll statusUrl for confirmed canonical FundingAdded.",
        ),
        X402RelayStatus::Failed if attempt.retryable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The relay failed before canonical funding and may be retried with the same signed request.",
        ),
        X402RelayStatus::Failed => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "The authorization did not produce canonical funding. Issue a new authorization after correcting the reported error.",
        ),
        X402RelayStatus::Confirmed => unreachable!("confirmed returned above"),
    };
    let mut response = (
        status,
        Json(serde_json::json!({
            "x402Version": X402_VERSION,
            "scheme": AGENT_BOUNTY_FUND_SCHEME,
            "status": x402_relay_status_name(attempt.status),
            "settled": false,
            "relay": x402_public_relay(attempt),
            "statusUrl": status_url,
            "nextStep": next_step,
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

fn x402_public_relay(attempt: &X402RelayAttempt) -> serde_json::Value {
    serde_json::json!({
        "id": attempt.id,
        "network": attempt.network,
        "bountyContract": attempt.bounty_contract,
        "contributor": attempt.contributor,
        "amount": attempt.amount,
        "relayerAddress": attempt.relayer_address,
        "status": x402_relay_status_name(attempt.status),
        "retryable": attempt.retryable,
        "attemptCount": attempt.attempt_count,
        "transaction": attempt.tx_hash,
        "estimatedGas": attempt.estimated_gas,
        "gasLimit": attempt.gas_limit,
        "errorCode": attempt.error_code,
        "errorMessage": attempt.error_message,
        "canonicalEventId": attempt.canonical_event_id,
        "confirmedBlock": attempt.confirmed_block,
        "createdAt": attempt.created_at,
        "updatedAt": attempt.updated_at,
    })
}

fn x402_relay_status_name(status: X402RelayStatus) -> &'static str {
    match status {
        X402RelayStatus::Prepared => "prepared",
        X402RelayStatus::Relaying => "relaying",
        X402RelayStatus::Broadcast => "broadcast",
        X402RelayStatus::Confirmed => "confirmed",
        X402RelayStatus::Failed => "failed",
    }
}

fn x402_payment_required_error(
    mut challenge: PaymentRequired,
    error: &str,
) -> Result<Response, StatusCode> {
    challenge.error = Some(error.to_string());
    x402_payment_required_response(challenge)
}

fn resolve_x402_funding_amount(
    status: &str,
    target_amount: &str,
    funded_amount: &str,
    requested_amount: Option<u64>,
) -> Result<u64, StatusCode> {
    if status != "open" {
        return Err(StatusCode::CONFLICT);
    }
    let target = target_amount
        .parse::<u64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let funded = funded_amount
        .parse::<u64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let remaining = target
        .checked_sub(funded)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let amount = requested_amount.unwrap_or(remaining);
    if remaining == 0 || amount == 0 || amount > remaining || amount > i64::MAX as u64 {
        return Err(StatusCode::CONFLICT);
    }
    Ok(amount)
}

fn x402_payment_required_response(challenge: PaymentRequired) -> Result<Response, StatusCode> {
    let encoded = encode_payment_required_header(&challenge)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut response = (
        StatusCode::PAYMENT_REQUIRED,
        Json(serde_json::json!({
            "x402Version": X402_VERSION,
            "status": "payment_required",
            "settled": false,
            "paymentRequired": challenge,
            "nextStep": "Sign the exact EIP-3009 authorization and retry this URL with the base64 x402 PaymentPayload in PAYMENT-SIGNATURE."
        })),
    )
        .into_response();
    response.headers_mut().insert(
        HeaderName::from_static(PAYMENT_REQUIRED_HEADER),
        HeaderValue::from_str(&encoded).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

#[utoipa::path(get, path = "/v1/capabilities/feed", responses((status = 200, description = "Public solver capability feed")))]
async fn public_capability_feed(
    State(state): State<SharedState>,
) -> Json<Vec<web_public::PublicCapabilityFeedItem>> {
    Json(capability_feed_from_state(&state))
}

#[utoipa::path(post, path = "/v1/capabilities/search", request_body = SearchCapabilitiesRequest, responses((status = 200, description = "Filtered public solver capability feed")))]
async fn search_capabilities(
    State(state): State<SharedState>,
    Json(request): Json<SearchCapabilitiesRequest>,
) -> Json<Vec<web_public::PublicCapabilityFeedItem>> {
    let mut feed = capability_feed_from_state(&state);
    filter_capability_feed(&mut feed, &request);
    Json(feed)
}

fn capability_feed_from_state(state: &SharedState) -> Vec<web_public::PublicCapabilityFeedItem> {
    let (capabilities, agents, reputation_events, settlements) = {
        let network = state.network.lock().expect("state poisoned");
        (
            network.capabilities.values().cloned().collect::<Vec<_>>(),
            network.agents.values().cloned().collect::<Vec<_>>(),
            network
                .reputation_events
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            network.settlements.values().cloned().collect::<Vec<_>>(),
        )
    };
    web_public::public_capability_feed(
        &capabilities,
        &agents,
        &reputation_events,
        &settlements,
        &state.public_base_url,
    )
}

fn filter_capability_feed(
    feed: &mut Vec<web_public::PublicCapabilityFeedItem>,
    request: &SearchCapabilitiesRequest,
) {
    let class_filter = request.class.as_ref().map(|class| format!("{class:?}"));
    let template_filter = request.template_slug.as_ref();
    let currency_filter = request
        .currency
        .as_ref()
        .map(|currency| currency.to_ascii_lowercase());
    feed.retain(|item| {
        class_filter
            .as_ref()
            .map(|class| item.class == *class)
            .unwrap_or(true)
            && template_filter
                .map(|template| item.template_slugs.iter().any(|slug| slug == template))
                .unwrap_or(true)
            && currency_filter
                .as_ref()
                .map(|currency| item.currency == *currency)
                .unwrap_or(true)
            && request
                .max_price_minor
                .map(|max_price| item.min_price_minor <= max_price)
                .unwrap_or(true)
    });
}

#[utoipa::path(post, path = "/v1/bounties")]
async fn post_bounty(
    State(state): State<SharedState>,
    Json(request): Json<PostBountyRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    let bounty = service_runtime::post_bounty(state.store.as_ref(), &state.network, request)
        .await
        .map_err(mutation_status)?;
    Ok(Json(bounty))
}

#[utoipa::path(post, path = "/v1/bounties/pooled")]
async fn open_pooled_bounty(
    State(state): State<SharedState>,
    Json(request): Json<OpenPooledBountyRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    let bounty = service_runtime::open_pooled_bounty(state.store.as_ref(), &state.network, request)
        .await
        .map_err(mutation_status)?;
    Ok(Json(bounty))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/funding-intents")]
async fn create_funding_intent(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<CreateFundingIntentRequest>,
) -> Result<Json<FundingIntentReport>, StatusCode> {
    request.bounty_id = id;
    let report = service_runtime::create_funding_intent(
        state.store.as_ref(),
        &state.network,
        request,
        state.public_base_url.clone(),
    )
    .await
    .map_err(mutation_status)?;
    Ok(Json(report))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/funding-contributions")]
async fn add_funding_contribution(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<AddFundingContributionRequest>,
) -> Result<Json<PooledFundingReport>, StatusCode> {
    request.bounty_id = id;
    let report =
        service_runtime::add_funding_contribution(state.store.as_ref(), &state.network, request)
            .await
            .map_err(mutation_status)?;
    Ok(Json(report))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/claim")]
async fn claim_bounty(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<ClaimBountyRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    request.bounty_id = id;
    let bounty = service_runtime::claim_bounty(state.store.as_ref(), &state.network, request)
        .await
        .map_err(mutation_status)?;
    Ok(Json(bounty))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/submit")]
async fn submit_result(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<SubmitResultRequest>,
) -> Result<Json<domain::Submission>, StatusCode> {
    request.bounty_id = id;
    let submission = service_runtime::submit_result(state.store.as_ref(), &state.network, request)
        .await
        .map_err(mutation_status)?;
    Ok(Json(submission))
}

#[utoipa::path(
    post,
    path = "/v1/bounties/{id}/verify",
    responses(
        (status = 200, description = "Operator-authorized legacy verification result"),
        (status = 401, description = "Operator token required"),
        (status = 503, description = "Legacy verification is disabled until an operator token is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn verify_submission(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(mut request): Json<VerifySubmissionRequest>,
) -> Result<Json<domain::ProofRecord>, StatusCode> {
    if state.operator_api_token.is_none() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    require_operator(&state, &headers)?;
    request.bounty_id = id;
    let network = {
        let mut guard = state.network.lock().expect("state poisoned");
        std::mem::take(&mut *guard)
    };
    let (network, result) = service_runtime::execute_verification(network, request).await;
    *state.network.lock().expect("state poisoned") = network;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(_) => {
            persist_all_risk_events(&state).await?;
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    service_runtime::persist_verification(state.store.as_ref(), &outcome)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(outcome.proof))
}

#[utoipa::path(
    post,
    path = "/v1/base/broadcast-signed-transaction",
    request_body = BroadcastBaseSignedTransactionRequest,
    responses(
        (status = 200, description = "Broadcast a signed Base transaction through configured RPC"),
        (status = 400, description = "Invalid signed transaction request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 503, description = "Base transaction broadcast or RPC URL is not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn broadcast_base_signed_transaction(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<BroadcastBaseSignedTransactionRequest>,
) -> Result<Json<BaseSignedTransactionBroadcastReport>, StatusCode> {
    require_operator(&state, &headers)?;
    if !state.base_broadcast_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let request_id = request.request_id.unwrap_or(1);
    let network_name = request.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = state
        .base_rpc_urls
        .resolve(network_name)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let rpc_request = eth_send_raw_transaction_request(&request.signed_transaction, request_id)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let response = broadcast_signed_transaction(&rpc_url, &request.signed_transaction, request_id)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;

    Ok(Json(BaseSignedTransactionBroadcastReport {
        network,
        request: rpc_request,
        tx_hash: response.result,
        next_step:
            "Poll POST /v1/base/transaction-receipt for inclusion. The autonomous indexer independently reconciles canonical factory and bounty logs; a receipt alone never proves settlement."
                .to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/v1/base/transaction-receipt",
    request_body = GetBaseTransactionReceiptRequest,
    responses(
        (status = 200, description = "Fetch a Base transaction receipt without mutating settlement state"),
        (status = 400, description = "Invalid receipt request"),
        (status = 503, description = "Requested Base RPC URL is not configured")
    )
)]
async fn get_base_transaction_receipt(
    State(state): State<SharedState>,
    Json(request): Json<GetBaseTransactionReceiptRequest>,
) -> Result<Json<BaseTransactionReceiptReport>, StatusCode> {
    let request_id = request.request_id.unwrap_or(1);
    let network_name = request.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = state
        .base_rpc_urls
        .resolve(network_name)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let rpc_request = eth_get_transaction_receipt_request(&request.tx_hash, request_id)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let tx_hash = rpc_request.params[0].clone();
    let response = fetch_transaction_receipt(&rpc_url, &tx_hash, request_id)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let Some(receipt) = response.result else {
        return Ok(Json(BaseTransactionReceiptReport {
            network,
            request: rpc_request,
            receipt_found: false,
            tx_hash,
            block_number: None,
            succeeded: None,
            log_count: 0,
            receipt: None,
        }));
    };

    let block_number = receipt
        .block_number()
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let succeeded = receipt
        .succeeded()
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let log_count = receipt.logs.len();

    Ok(Json(BaseTransactionReceiptReport {
        network,
        request: rpc_request,
        receipt_found: true,
        tx_hash,
        block_number,
        succeeded,
        log_count,
        receipt: Some(receipt),
    }))
}

fn base_rpc_fetch_status(error: &ChainBaseError) -> StatusCode {
    match error {
        ChainBaseError::UnknownNetwork(_)
        | ChainBaseError::InvalidAddress(_)
        | ChainBaseError::InvalidBlockRange { .. }
        | ChainBaseError::InvalidSignedTransaction(_)
        | ChainBaseError::InvalidTransactionHash(_) => StatusCode::BAD_REQUEST,
        ChainBaseError::MissingRpcUrl { .. } => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_GATEWAY,
    }
}

fn autonomous_planner_addresses(
    chain_id: u64,
    configured_factory: Option<String>,
    configured_implementation: Option<String>,
) -> Result<(String, String), StatusCode> {
    service_runtime::autonomous_planner_addresses(
        chain_id,
        configured_factory,
        configured_implementation,
    )
    .map_err(|error| match error {
        PlannerAddressError::UnsupportedNetwork => StatusCode::BAD_REQUEST,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    })
}

fn configured_autonomous_planner(network: &str) -> Result<AutonomousBountyTxPlanner, StatusCode> {
    let descriptor = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let (factory_env, implementation_env) = match descriptor.chain_id {
        8_453 => (
            "BASE_MAINNET_BOUNTY_FACTORY",
            "BASE_MAINNET_BOUNTY_IMPLEMENTATION",
        ),
        84_532 => (
            "BASE_SEPOLIA_BOUNTY_FACTORY",
            "BASE_SEPOLIA_BOUNTY_IMPLEMENTATION",
        ),
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let (factory, implementation) = autonomous_planner_addresses(
        descriptor.chain_id,
        env::var(factory_env).ok(),
        env::var(implementation_env).ok(),
    )?;
    AutonomousBountyTxPlanner::new(&factory, &implementation)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn require_indexed_canonical_bounty(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> Result<(), StatusCode> {
    let item = indexed_autonomous_bounty(state, network, bounty_contract).await?;
    if item.terms_valid {
        Ok(())
    } else {
        Err(StatusCode::CONFLICT)
    }
}

async fn indexed_autonomous_bounty(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> Result<AutonomousBountyFeedItem, StatusCode> {
    let Some(store) = &state.store else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let planner = configured_autonomous_planner(network)?;
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical_contracts = store
        .list_canonical_autonomous_bounty_contracts(network, &planner.factory_contract)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !canonical_contracts
        .iter()
        .any(|contract| contract.eq_ignore_ascii_case(bounty_contract))
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.recovery_reservations.apply(&mut feed, false);
    feed.into_iter()
        .find(|item| item.bounty_contract.eq_ignore_ascii_case(bounty_contract))
        .ok_or(StatusCode::NOT_FOUND)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/canonical-child-terms-plan", responses((status = 200, description = "Exact settled-child bounty terms and commitment plan")))]
async fn plan_autonomous_canonical_child_terms(
    Json(request): Json<CanonicalChildBountyTermsRequest>,
) -> Result<Json<CanonicalChildBountyTermsPlan>, AgentActionApiError> {
    build_canonical_child_bounty_terms_plan(&request)
        .map(Json)
        .map_err(|error| {
            agent_action_error(
                StatusCode::BAD_REQUEST,
                "invalid_canonical_child_terms_plan",
                error.to_string(),
                false,
                "Correct the parent binding, child acceptance criteria, or child task verifier and rerun plan_autonomous_canonical_child_terms. Do not create or fund the child from a rejected plan.",
            )
        })
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/standing-meta-v2-child-preparation", responses((status = 200, description = "Hosted terms publication plus exact ordered on-chain terms and fully funded routed-V3 child creation calls; the legacy path is retained for agent compatibility")))]
async fn prepare_standing_meta_v2_child(
    State(state): State<SharedState>,
    Json(request): Json<StandingMetaV2ChildPreparationRequest>,
) -> Result<Json<StandingMetaV2ChildPreparationPlan>, AgentActionApiError> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    if network != "base-mainnet" {
        return Err(agent_action_error(
            StatusCode::BAD_REQUEST,
            "standing_meta_network_unsupported",
            "standing-meta child preparation is deployed only on canonical Base mainnet",
            false,
            "Use network base-mainnet and an exact claimable routed-V3 parent contract.",
        ));
    }
    let parent = indexed_autonomous_bounty(&state, network, &request.parent_bounty_contract)
        .await
        .map_err(|status| {
            agent_action_error(
                status,
                "standing_meta_v2_parent_unavailable",
                "the parent is not available as an indexed canonical bounty",
                status.is_server_error(),
                "Refresh canonical inventory and retry the same parent contract. Do not publish or claim from unindexed data.",
            )
        })?;
    let parent_context = standing_meta_v2_parent_context(&parent).map_err(|error| {
        agent_action_error(
            StatusCode::CONFLICT,
            "standing_meta_parent_invalid",
            error.to_string(),
            false,
            "Choose an exact claimable routed-V3 parent. Recovery-reserved V2 and already-claimed parents cannot prepare a child.",
        )
    })?;
    if request.parent_solver.eq_ignore_ascii_case(&parent.creator) {
        return Err(agent_action_error(
            StatusCode::CONFLICT,
            "standing_meta_creator_cannot_claim",
            "the parent bounty creator cannot be its solver",
            false,
            "Use a different registered parent-solver wallet.",
        ));
    }
    let planner = configured_autonomous_planner(network).map_err(|_| {
        agent_action_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "canonical_child_planner_unavailable",
            "the canonical Base-mainnet child planner is unavailable",
            true,
            "Retry after canonical factory readiness is restored.",
        )
    })?;
    let mut plan = planner
        .plan_standing_meta_v2_child(&request, &parent_context, Utc::now())
        .map_err(|error| {
            agent_action_error(
                StatusCode::BAD_REQUEST,
                "standing_meta_child_invalid",
                error.to_string(),
                false,
                "Correct the task, immutable runner manifest, participants, or economics and retry the same routed-V3 preparation. Do not sign a rejected plan.",
            )
        })?;
    let store = state.store.as_ref().ok_or_else(|| {
        agent_action_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "terms_store_unavailable",
            "DATABASE_URL is required to publish the child terms",
            true,
            "Retry after hosted terms storage is healthy. Do not send the on-chain calls first.",
        )
    })?;
    store
        .upsert_autonomous_bounty_terms(&plan.terms)
        .await
        .map_err(|error| {
            agent_action_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "standing_meta_terms_store_failed",
                error.to_string(),
                true,
                "Retry the identical request. Do not alter or send the returned on-chain terms until hosted publication succeeds.",
            )
    })?;
    plan.hosted_terms_published = true;
    plan.current_state = "hosted_routed_v3_child_terms_published_parent_unclaimed".to_string();
    Ok(Json(plan))
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/creation-plan", responses((status = 200, description = "Unsigned canonical autonomous bounty creation and initial-funding plan")))]
async fn plan_autonomous_bounty_creation(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyCreationRequest>,
) -> Result<Json<AutonomousBountyCreationPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let terms = require_autonomous_creation_terms(&state, network, &request.create).await?;
    let plan = configured_autonomous_planner(network)?
        .plan_creation(network, &request.create)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    record_opportunity_creation_progress(&state, network, &terms, "funding_prepared").await?;
    Ok(Json(plan))
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/authorized-creation-plan", responses((status = 200, description = "Relayer transaction plan after the creator signs Circle USDC EIP-3009 authorization")))]
async fn plan_autonomous_bounty_authorized_creation(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyAuthorizedCreationRequest>,
) -> Result<Json<AutonomousBountyAuthorizedCreationPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let terms = require_autonomous_creation_terms(&state, network, &request.create).await?;
    let plan = configured_autonomous_planner(network)?
        .plan_authorized_creation(
            network,
            &request.create,
            &request.signature,
            request.relayer.as_deref(),
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    record_opportunity_creation_progress(&state, network, &terms, "wallet_signed").await?;
    Ok(Json(plan))
}

async fn require_autonomous_creation_terms(
    state: &SharedState,
    network: &str,
    create: &AutonomousBountyCreate,
) -> Result<AutonomousBountyTermsRecord, StatusCode> {
    let terms = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .get_autonomous_bounty_terms(&create.terms_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    validate_autonomous_creation_for_public_earning(network, create, &terms)
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(terms)
}

async fn record_opportunity_creation_progress(
    state: &SharedState,
    network: &str,
    terms: &AutonomousBountyTermsRecord,
    stage: &str,
) -> Result<(), StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let unfunded_bounty_id =
        terms.document.source_url.as_deref().and_then(|source_url| {
            unfunded_bounty_id_from_source(source_url, &state.public_base_url)
        });
    store
        .record_opportunity_creation_progress(
            &terms.terms_hash,
            unfunded_bounty_id,
            network,
            stage,
            Utc::now(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn unfunded_bounty_id_from_source(source_url: &str, public_base_url: &str) -> Option<Uuid> {
    let prefix = format!(
        "{}/v1/unfunded-bounties/",
        public_base_url.trim_end_matches('/')
    );
    source_url
        .strip_prefix(&prefix)
        .filter(|id| !id.contains(['/', '?', '#']))
        .and_then(|id| Uuid::parse_str(id).ok())
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/contribution-plan", responses((status = 200, description = "Unsigned permissionless pooled USDC contribution plan")))]
async fn plan_autonomous_bounty_contribution(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyContributionRequest>,
) -> Result<Json<AutonomousBountyContributionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.contribution.bounty_contract)
        .await?;
    configured_autonomous_planner(network)?
        .plan_contribution(network, &request.contribution)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/authorized-contribution-plan", responses((status = 200, description = "Single relayer transaction after a funder signs bounded Circle USDC authorization")))]
async fn plan_autonomous_bounty_authorized_contribution(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyAuthorizedContributionRequest>,
) -> Result<Json<AutonomousBountyAuthorizedContributionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.contribution.bounty_contract)
        .await?;
    configured_autonomous_planner(network)?
        .plan_authorized_contribution(
            network,
            &request.contribution,
            &request.signature,
            request.relayer.as_deref(),
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/claim-plan", responses((status = 200, description = "Wallet-batched USDC bond approval and direct claim plan")))]
async fn plan_autonomous_bounty_claim(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyClaimRequest>,
) -> Result<Json<AutonomousBountyClaimPlan>, AgentActionApiError> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    if indexed_open_competition_bounty(&state, network, &request.bounty_contract)
        .await
        .map_err(|status| {
            status_agent_action_error(
                status,
                "Retry after canonical competition inventory is healthy; do not prepare an exclusive claim while mode is unknown.",
            )
        })?
    {
        return Err(wrong_competition_mode_error(
            &state,
            network,
            &request.bounty_contract,
        ));
    }
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract)
        .await
        .map_err(|status| {
            status_agent_action_error(status, "Refresh ready-to-earn inventory and retry.")
        })?;
    require_claimable_autonomous_item(&item).map_err(|status| {
        status_agent_action_error(status, "Refresh canonical state before attempting a claim.")
    })?;
    let claim_bond = item.claim_bond.parse::<u128>().map_err(|_| {
        status_agent_action_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Do not sign; report the malformed indexed bond.",
        )
    })?;
    configured_autonomous_planner(network)
        .map_err(|status| status_agent_action_error(status, "Retry after the planner is healthy."))?
        .plan_claim(
            network,
            &request.bounty_contract,
            &request.solver,
            claim_bond,
            request.authorization_nonce.as_deref(),
            request.authorization_valid_before,
        )
        .map(Json)
        .map_err(|_| {
            status_agent_action_error(
                StatusCode::BAD_REQUEST,
                "Correct the public wallet inputs and retry without signing arbitrary calldata.",
            )
        })
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/authorized-claim-plan", responses((status = 200, description = "Single relayer transaction after the solver signs the exact USDC claim bond authorization")))]
async fn plan_autonomous_bounty_authorized_claim(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyAuthorizedClaimRequest>,
) -> Result<Json<AutonomousBountyAuthorizedClaimPlan>, AgentActionApiError> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    if indexed_open_competition_bounty(&state, network, &request.bounty_contract)
        .await
        .map_err(|status| {
            status_agent_action_error(
                status,
                "Retry after canonical competition inventory is healthy; do not prepare an exclusive claim while mode is unknown.",
            )
        })?
    {
        return Err(wrong_competition_mode_error(
            &state,
            network,
            &request.bounty_contract,
        ));
    }
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract)
        .await
        .map_err(|status| {
            status_agent_action_error(status, "Refresh ready-to-earn inventory and retry.")
        })?;
    require_claimable_autonomous_item(&item).map_err(|status| {
        status_agent_action_error(status, "Refresh canonical state before attempting a claim.")
    })?;
    let claim_bond = item.claim_bond.parse::<u128>().map_err(|_| {
        status_agent_action_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Do not sign; report the malformed indexed bond.",
        )
    })?;
    configured_autonomous_planner(network)
        .map_err(|status| status_agent_action_error(status, "Retry after the planner is healthy."))?
        .plan_authorized_claim(
            network,
            &request.bounty_contract,
            &request.solver,
            claim_bond,
            &request.authorization_nonce,
            request.authorization_valid_before,
            &request.signature,
            request.relayer.as_deref(),
        )
        .map(Json)
        .map_err(|_| {
            status_agent_action_error(
                StatusCode::BAD_REQUEST,
                "Correct the bounded authorization inputs and retry without signing arbitrary calldata.",
            )
        })
}

type AgentClaimProblem = (StatusCode, Json<serde_json::Value>);

#[utoipa::path(
    post,
    path = "/v1/base/autonomous-bounties/claims",
    responses(
        (status = 200, description = "Exclusive claim handoff prepared or canonical claim confirmed"),
        (status = 202, description = "Candidate waitlisted or transaction confirmation pending"),
        (status = 409, description = "Claim transition conflicts with canonical or hosted coordination state"),
        (status = 422, description = "Eligibility, authorization, or bounded relay validation failed"),
        (status = 429, description = "Sponsorship or waitlist cap reached"),
        (status = 503, description = "Canonical index, sponsor, relayer, database, or RPC unavailable")
    )
)]
async fn agent_native_claim(
    State(state): State<SharedState>,
    Json(request): Json<AgentNativeClaimRequest>,
) -> Result<Response, AgentClaimProblem> {
    validate_agent_native_claim_request(&request)?;
    let request_signature = resolve_agent_claim_signature(&request)?;
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let descriptor = base_network_descriptor(network).map_err(|_| {
        agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "request_invalid",
            "validate_request",
            "network must be base-mainnet or base-sepolia",
            "Use the network named by the canonical bounty inventory.",
        )
    })?;
    let network = canonical_base_network_key(descriptor.chain_id).ok_or_else(|| {
        agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "request_invalid",
            "validate_request",
            "network must be Base mainnet or Base Sepolia",
            "Use the network named by the canonical bounty inventory.",
        )
    })?;
    let bounty_contract = normalize_evm_address(&request.bounty_contract).map_err(|_| {
        agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "request_invalid",
            "validate_request",
            "bounty_contract is not a valid EVM address",
            "Use the contract from verified claimable inventory.",
        )
    })?;
    let solver_wallet = normalize_evm_address(&request.solver_wallet).map_err(|_| {
        agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "request_invalid",
            "validate_request",
            "solver_wallet is not a valid EVM address",
            "Provide the public Base payout wallet; never provide its private key.",
        )
    })?;
    if configured_standing_meta_v4_parent(network, &bounty_contract) {
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "standing_meta_v4_atomic_claim_required",
            "route_v4_claim",
            "a Standing Meta V4 parent cannot be claimed directly",
            "Call get_standing_meta_v4_readiness, then prepare_standing_meta_v4_claim. The atomic flow creates and funds the child, snapshots the active solver pool, requests VRF, binds the round, and posts the parent bond in one transaction.",
        ));
    }
    if indexed_open_competition_bounty(&state, network, &bounty_contract)
        .await
        .map_err(|status| {
            agent_claim_problem(
                status,
                "competition_mode_unavailable",
                "select_competition_mode",
                "the canonical competition-mode index is unavailable",
                "Retry after canonical bounty inventory is healthy; do not prepare an exclusive claim while mode is unknown.",
            )
        })?
    {
        let (_, Json(problem)) = wrong_competition_mode_error(&state, network, &bounty_contract);
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "schema_version": "agent-bounties/agent-claim-problem-v1",
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
    let item = indexed_autonomous_bounty(&state, network, &bounty_contract)
        .await
        .map_err(|status| {
            agent_claim_problem(
                status,
                "canonical_inventory_unavailable",
                "load_canonical_bounty",
                "the bounty is not available in verified hosted inventory",
                "Refresh verified claimable inventory and retry only if verification_ready=true.",
            )
        })?;
    let claim_bond = item.claim_bond.parse::<u64>().map_err(|_| {
        agent_claim_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_indexed_bond",
            "load_canonical_bounty",
            "the indexed claim bond cannot be represented safely",
            "Do not sign; report the bounty contract to maintainers.",
        )
    })?;
    if claim_bond == 0 {
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "unsupported_zero_bond",
            "prepare_authorization",
            "autonomous-v1 agent-native claims require a positive indexed solver bond",
            "Use the direct claim planner only if the canonical contract explicitly supports zero bond.",
        ));
    }

    let terms = item.terms.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::CONFLICT,
            "terms_unavailable",
            "evaluate_eligibility",
            "content-addressed bounty terms are unavailable",
            "Do not claim until the terms document is present and terms_valid=true.",
        )
    })?;
    let policy = terms.document.agent_eligibility.clone().unwrap_or_default();
    let coordination = terms
        .document
        .claim_coordination
        .clone()
        .unwrap_or_default();
    let sponsorship_allowed = terms
        .document
        .agent_eligibility
        .as_ref()
        .map(|policy| policy.sponsorship_allowed)
        .unwrap_or(true);
    let sponsorship_available = sponsorship_allowed
        && claim_bond <= state.bond_sponsor.max_bond
        && claim_bond <= policy.maximum_sponsored_bond_base_units
        && state.bond_sponsor.contract_for(network).is_some()
        && state.bond_sponsor.grant_signer().is_some()
        && state.x402_relayer.enabled
        && state.x402_relayer.relayer.is_some();
    let store = state.store.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "coordination_unavailable",
            "reserve_candidate",
            "hosted durable claim coordination is unavailable",
            "Use plan_autonomous_bounty_claim for a direct permissionless wallet claim.",
        )
    })?;
    let existing_candidate = store
        .get_claim_candidate_by_idempotency_key(request.idempotency_key.trim())
        .await
        .map_err(map_agent_claim_db_error)?;
    let existing_reservation = if let Some(candidate) = existing_candidate {
        validate_persisted_claim_candidate_scope(
            &candidate,
            network,
            &bounty_contract,
            &solver_wallet,
            request.agent_id,
        )?;
        let sponsorship = store
            .get_bond_sponsorship_for_candidate(candidate.id)
            .await
            .map_err(map_agent_claim_db_error)?;
        let sponsorship_requested = request.request_bond_sponsorship || sponsorship.is_some();
        if let Some(claim) =
            current_indexed_claim_for_candidate(&item, &terms.policy_hash, &candidate, claim_bond)
        {
            let candidate =
                reconcile_indexed_claim_candidate(&state, candidate, sponsorship.clone(), claim)
                    .await?;
            let recovered_sponsorship = store
                .get_bond_sponsorship_for_candidate(candidate.id)
                .await
                .map_err(map_agent_claim_db_error)?;
            return Ok(agent_claim_response(
                &state,
                StatusCode::OK,
                ClaimCandidateReservation {
                    candidate,
                    waitlist_position: None,
                },
                claim_bond,
                sponsorship_requested,
                sponsorship_available,
                recovered_sponsorship,
                None,
                "The canonical BountyClaimed event is confirmed. Complete the task and prepare the exact submission evidence.",
                None,
            ));
        }
        let reservation = ClaimCandidateReservation {
            candidate: candidate.clone(),
            waitlist_position: None,
        };
        match candidate.status {
            ClaimCandidateStatus::Claimed => {
                return Ok(agent_claim_response(
                    &state,
                    StatusCode::OK,
                    reservation,
                    claim_bond,
                    sponsorship_requested,
                    sponsorship_available,
                    sponsorship,
                    None,
                    "The canonical BountyClaimed event is confirmed. Complete the task and prepare the exact submission evidence.",
                    None,
                ));
            }
            ClaimCandidateStatus::Relaying => {
                let candidate = reconcile_agent_native_claim(&state, candidate, claim_bond).await?;
                let sponsorship = store
                    .get_bond_sponsorship_for_candidate(candidate.id)
                    .await
                    .map_err(map_agent_claim_db_error)?;
                let confirmed = candidate.status == ClaimCandidateStatus::Claimed;
                return Ok(agent_claim_response(
                    &state,
                    if confirmed {
                        StatusCode::OK
                    } else {
                        StatusCode::ACCEPTED
                    },
                    ClaimCandidateReservation {
                        candidate,
                        waitlist_position: None,
                    },
                    claim_bond,
                    sponsorship_requested,
                    sponsorship_available,
                    sponsorship,
                    None,
                    if confirmed {
                        "Canonical BountyClaimed is confirmed. Complete the task and prepare the exact submission evidence."
                    } else {
                        "The exact claim transaction remains pending. Replay this same signed request; do not sign or fund again."
                    },
                    None,
                ));
            }
            ClaimCandidateStatus::Waitlisted => {
                return Ok(agent_claim_response(
                    &state,
                    StatusCode::ACCEPTED,
                    reservation,
                    claim_bond,
                    sponsorship_requested,
                    sponsorship_available,
                    sponsorship,
                    None,
                    "Wait for claim_exclusive notification or poll with the same idempotency_key. Do not sign while waitlisted.",
                    None,
                ));
            }
            ClaimCandidateStatus::Superseded
            | ClaimCandidateStatus::Withdrawn
            | ClaimCandidateStatus::Failed => {
                return Err(agent_claim_problem(
                    StatusCode::CONFLICT,
                    "candidate_terminal",
                    "prepare_authorization",
                    "this hosted claim candidate is terminal",
                    "Retry with a new idempotency_key if the canonical bounty is still claimable.",
                ));
            }
            ClaimCandidateStatus::Exclusive
            | ClaimCandidateStatus::Sponsoring
            | ClaimCandidateStatus::AuthorizationReady => Some(reservation),
        }
    } else {
        None
    };

    if request.request_bond_sponsorship && !sponsorship_available {
        return Err(agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "sponsorship_unavailable",
            "evaluate_sponsorship",
            "atomic bond sponsorship is disabled, unsupported on this network, or exceeds the published cap",
            "Fund the exact indexed bond. Replay the same idempotency_key with request_bond_sponsorship=false.",
        ));
    }
    require_claimable_autonomous_item(&item).map_err(|_| {
        agent_claim_problem(
            StatusCode::CONFLICT,
            "bounty_not_claimable",
            "reserve_candidate",
            "the canonical bounty is not currently funded, claimable, and verification-ready",
            "Choose the next verified claimable bounty. Poll this contract only after canonical state reopens.",
        )
    })?;
    let reservation = if let Some(reservation) = existing_reservation {
        reservation
    } else {
        let (evidence, decision) = build_agent_eligibility(
            &state,
            network,
            &item.creator,
            &solver_wallet,
            request.agent_id,
            &policy,
        )
        .await?;
        if !decision.eligible {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "schema_version": "agent-bounties/claim-problem-v1",
                    "state": "ineligible",
                    "failed_transition": "evaluate_eligibility",
                    "error": "agent_ineligible",
                    "reasons": decision.reasons,
                    "next_action": "Choose a bounty whose published eligibility policy this wallet satisfies."
                })),
            ));
        }
        store
            .promote_waitlisted_claimant_after_canonical_reopen(
                network,
                &bounty_contract,
                coordination.exclusive_claim_seconds,
            )
            .await
            .map_err(map_agent_claim_db_error)?;
        store
            .reserve_claim_candidate(
                &NewClaimCandidate {
                    id: Uuid::new_v4(),
                    idempotency_key: request.idempotency_key.trim().to_string(),
                    network: network.to_string(),
                    bounty_contract: bounty_contract.clone(),
                    solver_wallet: solver_wallet.clone(),
                    agent_id: request.agent_id,
                    eligibility_evidence: evidence,
                    eligibility_decision: decision,
                },
                coordination.exclusive_claim_seconds,
                coordination.waitlist_capacity,
            )
            .await
            .map_err(map_agent_claim_db_error)?
    };

    if reservation.candidate.status == ClaimCandidateStatus::Waitlisted {
        return Ok(agent_claim_response(
            &state,
            StatusCode::ACCEPTED,
            reservation,
            claim_bond,
            request.request_bond_sponsorship,
            sponsorship_available,
            None,
            None,
            "Wait for claim_exclusive notification or poll with the same idempotency_key. Do not sign while waitlisted.",
            None,
        ));
    }
    if reservation.candidate.status == ClaimCandidateStatus::Claimed {
        let sponsorship = store
            .get_bond_sponsorship_for_candidate(reservation.candidate.id)
            .await
            .map_err(map_agent_claim_db_error)?;
        return Ok(agent_claim_response(
            &state,
            StatusCode::OK,
            reservation,
            claim_bond,
            request.request_bond_sponsorship,
            sponsorship_available,
            sponsorship,
            None,
            "The canonical BountyClaimed event is confirmed. Complete the task and prepare the exact submission evidence.",
            None,
        ));
    }
    if reservation.candidate.status.is_terminal() {
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "candidate_terminal",
            "prepare_authorization",
            "this hosted claim candidate is terminal",
            "Retry with a new idempotency_key if the canonical bounty is still claimable.",
        ));
    }

    let mut candidate = reservation.candidate.clone();
    if candidate.authorization_nonce.is_none() {
        let (nonce, valid_before) = claim_authorization_window(&candidate)?;
        candidate = store
            .set_claim_candidate_authorization(candidate.id, &nonce, valid_before)
            .await
            .map_err(map_agent_claim_db_error)?;
    }
    let nonce = candidate.authorization_nonce.clone().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "authorization_missing",
            "prepare_authorization",
            "the reserved candidate has no authorization nonce",
            "Retry with the same idempotency_key.",
        )
    })?;
    let valid_before = candidate.authorization_valid_before.ok_or_else(|| {
        agent_claim_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "authorization_missing",
            "prepare_authorization",
            "the reserved candidate has no authorization deadline",
            "Retry with the same idempotency_key.",
        )
    })?;
    let claim_plan = configured_autonomous_planner(network)
        .and_then(|planner| {
            planner
                .plan_claim(
                    network,
                    &bounty_contract,
                    &solver_wallet,
                    u128::from(claim_bond),
                    Some(&nonce),
                    Some(valid_before),
                )
                .map_err(|_| StatusCode::BAD_REQUEST)
        })
        .map_err(|status| {
            agent_claim_problem(
                status,
                "authorization_plan_failed",
                "prepare_authorization",
                "the exact bounded USDC authorization could not be prepared",
                "Do not sign; retry from fresh canonical inventory.",
            )
        })?;
    let signing_payload = claim_plan.eip3009_authorization.clone().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "authorization_plan_failed",
            "prepare_authorization",
            "the planner omitted the required EIP-3009 payload",
            "Do not sign; report the candidate ID.",
        )
    })?;
    let reservation = ClaimCandidateReservation {
        candidate,
        waitlist_position: reservation.waitlist_position,
    };
    let Some(signature) = request_signature.as_ref() else {
        let next_request = signed_claim_request_template(&request);
        let next_action = "Send wallet_request to the solver wallet, then copy its 65-byte result unchanged into next_request.body.wallet_signature. The platform will relay one atomic claim and, when requested, provide the exact bond inside that same transaction.";
        return Ok(agent_claim_response(
            &state,
            StatusCode::OK,
            reservation,
            claim_bond,
            request.request_bond_sponsorship,
            sponsorship_available,
            None,
            Some(signing_payload),
            next_action,
            Some(next_request),
        ));
    };
    validate_claim_authorization_signature(
        &descriptor,
        &bounty_contract,
        &solver_wallet,
        claim_bond,
        &nonce,
        valid_before,
        signature,
    )?;

    let mut sponsorship = store
        .get_bond_sponsorship_for_candidate(reservation.candidate.id)
        .await
        .map_err(map_agent_claim_db_error)?;
    let use_atomic_sponsorship =
        should_use_atomic_sponsorship(request.request_bond_sponsorship, sponsorship.as_ref());
    let candidate = if use_atomic_sponsorship {
        let reserved = reserve_atomic_bond_sponsorship(
            &state,
            &reservation.candidate,
            claim_bond,
            sponsorship,
        )
        .await?;
        relay_atomic_sponsored_claim(
            &state,
            &reservation.candidate,
            &item,
            claim_bond,
            &nonce,
            valid_before,
            signature,
            &reserved,
        )
        .await?
    } else {
        relay_agent_native_claim(
            &state,
            &reservation.candidate,
            claim_bond,
            &nonce,
            valid_before,
            signature,
        )
        .await?
    };
    sponsorship = store
        .get_bond_sponsorship_for_candidate(reservation.candidate.id)
        .await
        .map_err(map_agent_claim_db_error)?;
    let confirmed = candidate.status == ClaimCandidateStatus::Claimed;
    Ok(agent_claim_response(
        &state,
        if confirmed {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        ClaimCandidateReservation {
            candidate,
            waitlist_position: reservation.waitlist_position,
        },
        claim_bond,
        use_atomic_sponsorship,
        sponsorship_available,
        sponsorship,
        None,
        if confirmed {
            "Canonical BountyClaimed is confirmed. Complete the task and prepare the exact submission evidence."
        } else {
            "The exact claim transaction was broadcast. Replay this signed request until candidate.status=claimed; do not sign or fund again."
        },
        None,
    ))
}

fn configured_standing_meta_v4_parent(network: &str, bounty_contract: &str) -> bool {
    let Ok(prefix) = standing_meta_v4_environment_prefix(network) else {
        return false;
    };
    env::var(format!("{prefix}_PARENT_CONTRACTS"))
        .ok()
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter_map(|value| normalize_evm_address(&value).ok())
        .any(|value| value.eq_ignore_ascii_case(bounty_contract))
}

fn configured_open_competition(network: &str, bounty_contract: &str) -> bool {
    let Ok(prefix) = open_competition_environment_prefix(network) else {
        return false;
    };
    env::var(format!("{prefix}_BOUNTY_CONTRACTS"))
        .ok()
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter_map(|value| normalize_evm_address(&value).ok())
        .any(|value| value.eq_ignore_ascii_case(bounty_contract))
}

async fn indexed_open_competition_bounty(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> Result<bool, StatusCode> {
    let bounty_contract = normalize_evm_address(bounty_contract)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_ascii_lowercase();
    if configured_open_competition(network, &bounty_contract) {
        return Ok(true);
    }

    let release = match open_competition_release_from_environment(network) {
        Ok(release) => release,
        Err(StatusCode::BAD_REQUEST) => return Err(StatusCode::BAD_REQUEST),
        Err(_) => return Ok(false),
    };
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let contracts = store
        .list_canonical_open_competition_contracts(network, &release.factory_contract)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(contracts
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&bounty_contract)))
}

#[utoipa::path(
    get,
    path = "/v1/base/autonomous-bounties/claim-funnel",
    params(("window_hours" = Option<u32>, Query, description = "Bounded lookback window from 1 to 720 hours; defaults to 168")),
    responses(
        (status = 200, description = "Privacy-preserving durable claim and sponsorship funnel counts"),
        (status = 400, description = "Window is outside the supported range"),
        (status = 503, description = "Durable hosted coordination is unavailable")
    )
)]
async fn claim_funnel(
    State(state): State<SharedState>,
    Query(query): Query<ClaimFunnelQuery>,
) -> Result<Json<ClaimFunnelStats>, StatusCode> {
    let window_hours = query.window_hours.unwrap_or(168);
    if !(1..=720).contains(&window_hours) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .claim_funnel_stats(window_hours, &state.recovery_reservations.contracts())
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn validate_agent_native_claim_request(
    request: &AgentNativeClaimRequest,
) -> Result<(), AgentClaimProblem> {
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return Err(agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "request_invalid",
            "validate_request",
            "idempotency_key must contain 1-128 characters",
            "Generate one stable key for this wallet and bounty, then reuse it for every retry.",
        ));
    }
    if request
        .source
        .as_ref()
        .is_some_and(|source| source.len() > 128)
    {
        return Err(agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "request_invalid",
            "validate_request",
            "source must be at most 128 characters",
            "Use a compact source such as github, mcp, curl, python, or cast.",
        ));
    }
    if request.signature.is_some() && request.wallet_signature.is_some() {
        return Err(agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "request_invalid",
            "validate_request",
            "provide one signature form, never both",
            "Remove signature. Send the wallet's unchanged 65-byte result in wallet_signature. Legacy clients must remove wallet_signature and send signature.",
        ));
    }
    Ok(())
}

fn resolve_agent_claim_signature(
    request: &AgentNativeClaimRequest,
) -> Result<Option<AutonomousBountyAuthorizationSignature>, AgentClaimProblem> {
    if let Some(signature) = request.signature.as_ref() {
        return Ok(Some(signature.clone()));
    }
    request
        .wallet_signature
        .as_deref()
        .map(parse_native_wallet_signature)
        .transpose()
}

fn parse_native_wallet_signature(
    signature: &str,
) -> Result<AutonomousBountyAuthorizationSignature, AgentClaimProblem> {
    let Some(encoded) = signature
        .strip_prefix("0x")
        .or_else(|| signature.strip_prefix("0X"))
    else {
        return Err(invalid_native_wallet_signature(
            "wallet_signature must be 0x-prefixed",
        ));
    };
    if encoded.len() != 130 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_native_wallet_signature(
            "wallet_signature must be exactly one 65-byte hex signature",
        ));
    }
    let v = u8::from_str_radix(&encoded[128..], 16).map_err(|_| {
        invalid_native_wallet_signature("wallet_signature has an invalid recovery byte")
    })?;
    let v = match v {
        0 | 27 => 27,
        1 | 28 => 28,
        _ => {
            return Err(invalid_native_wallet_signature(
                "wallet_signature recovery byte must be 0, 1, 27, or 28",
            ))
        }
    };
    Ok(AutonomousBountyAuthorizationSignature {
        v,
        r: format!("0x{}", &encoded[..64]).to_ascii_lowercase(),
        s: format!("0x{}", &encoded[64..128]).to_ascii_lowercase(),
    })
}

fn invalid_native_wallet_signature(message: &str) -> AgentClaimProblem {
    agent_claim_problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "signature_invalid",
        "verify_authorization",
        message,
        "Return the wallet's unchanged 0x-prefixed EIP-712 result in wallet_signature.",
    )
}

fn validate_persisted_claim_candidate_scope(
    candidate: &ClaimCandidate,
    network: &str,
    bounty_contract: &str,
    solver_wallet: &str,
    agent_id: Option<Uuid>,
) -> Result<(), AgentClaimProblem> {
    if !candidate.network.eq_ignore_ascii_case(network)
        || !candidate
            .bounty_contract
            .eq_ignore_ascii_case(bounty_contract)
        || !candidate.solver_wallet.eq_ignore_ascii_case(solver_wallet)
        || candidate.agent_id != agent_id
    {
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "load_candidate",
            "idempotency_key was already used for different claim inputs",
            "Replay the original inputs. Create a new idempotency_key only for a different claim.",
        ));
    }
    Ok(())
}

fn current_indexed_claim_for_candidate<'a>(
    item: &'a AutonomousBountyFeedItem,
    policy_hash: &str,
    candidate: &ClaimCandidate,
    claim_bond: u64,
) -> Option<&'a AutonomousBountyEvent> {
    if !matches!(item.status.as_str(), "claimed" | "submitted" | "paid") {
        return None;
    }
    let current_round = item
        .events
        .iter()
        .filter_map(|event| event.data.get("round").and_then(serde_json::Value::as_u64))
        .max()?;
    item.events
        .iter()
        .filter(|event| {
            event.kind == AutonomousBountyEventKind::BountyClaimed
                && event
                    .contract_address
                    .eq_ignore_ascii_case(&candidate.bounty_contract)
                && event.data["round"].as_u64() == Some(current_round)
                && event.data["solver"]
                    .as_str()
                    .is_some_and(|solver| solver.eq_ignore_ascii_case(&candidate.solver_wallet))
                && json_u128(&event.data["claim_bond"]) == Some(u128::from(claim_bond))
                && event.data["terms_hash"]
                    .as_str()
                    .is_some_and(|hash| hash.eq_ignore_ascii_case(&item.terms_hash))
                && event.data["policy_hash"]
                    .as_str()
                    .is_some_and(|hash| hash.eq_ignore_ascii_case(policy_hash))
        })
        .max_by_key(|event| (event.block_number, event.log_index))
}

fn should_use_atomic_sponsorship(requested: bool, sponsorship: Option<&BondSponsorship>) -> bool {
    requested || sponsorship.is_some()
}

async fn reconcile_indexed_claim_candidate(
    state: &SharedState,
    candidate: ClaimCandidate,
    sponsorship: Option<BondSponsorship>,
    claim: &AutonomousBountyEvent,
) -> Result<ClaimCandidate, AgentClaimProblem> {
    if candidate.status == ClaimCandidateStatus::Claimed {
        return Ok(candidate);
    }
    let store = state.store.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "coordination_unavailable",
            "confirm_claim",
            "durable claim state is unavailable",
            "Retry the same signed request.",
        )
    })?;
    if let Some(sponsorship) = sponsorship {
        if sponsorship.status == BondSponsorshipStatus::Broadcast
            && candidate.status == ClaimCandidateStatus::Relaying
            && sponsorship
                .transaction_hash
                .as_deref()
                .is_some_and(|hash| hash.eq_ignore_ascii_case(&claim.tx_hash))
            && candidate
                .claim_transaction_hash
                .as_deref()
                .is_some_and(|hash| hash.eq_ignore_ascii_case(&claim.tx_hash))
        {
            return store
                .mark_atomic_sponsored_claim_confirmed(
                    candidate.id,
                    sponsorship.id,
                    claim.id,
                    claim.block_number,
                )
                .await
                .map(|(candidate, _)| candidate)
                .map_err(map_agent_claim_db_error);
        }
        if matches!(
            sponsorship.status,
            BondSponsorshipStatus::Reserved | BondSponsorshipStatus::Broadcast
        ) {
            let (code, message) = if sponsorship.status == BondSponsorshipStatus::Reserved {
                (
                    "broadcast_unknown",
                    "Canonical claim evidence exists, but no sponsor transaction hash was durably recorded. The grant remains charged to the rolling cap conservatively.",
                )
            } else {
                (
                    "canonical_claim_tx_mismatch",
                    "The solver owns the canonical round through a different transaction; the recorded sponsor transaction did not create this claim.",
                )
            };
            store
                .mark_bond_sponsorship_failed(sponsorship.id, code, message)
                .await
                .map_err(map_agent_claim_db_error)?;
        }
    }
    store
        .mark_claim_candidate_claimed(candidate.id, claim.id)
        .await
        .map_err(map_agent_claim_db_error)
}

fn canonical_base_network_key(chain_id: u64) -> Option<&'static str> {
    match chain_id {
        8_453 => Some("base-mainnet"),
        84_532 => Some("base-sepolia"),
        _ => None,
    }
}

async fn build_agent_eligibility(
    state: &SharedState,
    network: &str,
    creator_wallet: &str,
    solver_wallet: &str,
    agent_id: Option<Uuid>,
    policy: &AgentEligibilityPolicy,
) -> Result<(AgentEligibilityEvidence, AgentEligibilityDecision), AgentClaimProblem> {
    let store = state.store.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "coordination_unavailable",
            "evaluate_eligibility",
            "durable agent eligibility data is unavailable",
            "Call plan_autonomous_bounty_claim. Submit its exact direct-wallet calls.",
        )
    })?;
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "eligibility_unavailable",
                "evaluate_eligibility",
                "canonical earning history could not be loaded",
                "Retry without signing anything.",
            )
        })?;
    let settlements = events.iter().filter(|event| {
        event.kind == AutonomousBountyEventKind::BountySettled
            && !state
                .recovery_reservations
                .contains(&event.contract_address)
            && event.data["solver"]
                .as_str()
                .is_some_and(|wallet| wallet.eq_ignore_ascii_case(solver_wallet))
    });
    let mut paid_completions = 0u32;
    let mut paid_usdc_base_units = 0u64;
    for event in settlements {
        paid_completions = paid_completions.saturating_add(1);
        let reward = json_u128(&event.data["solver_reward"]).unwrap_or(0);
        let bonus = json_u128(&event.data["timeout_bond_bonus"]).unwrap_or(0);
        paid_usdc_base_units = paid_usdc_base_units
            .saturating_add(u64::try_from(reward.saturating_add(bonus)).unwrap_or(u64::MAX));
    }
    let mut capabilities = Vec::new();
    let mut additional_reasons = Vec::new();
    if let Some(agent_id) = agent_id {
        let agents = store.list_agents().await.map_err(|_| {
            agent_claim_problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "eligibility_unavailable",
                "evaluate_eligibility",
                "registered agent identity could not be loaded",
                "Retry without signing anything.",
            )
        })?;
        match agents.into_iter().find(|agent| agent.id == agent_id) {
            Some(agent) => {
                if agent.status != AgentStatus::Active {
                    additional_reasons.push("registered agent is not active".to_string());
                }
                if agent
                    .payout_wallet
                    .as_deref()
                    .is_none_or(|wallet| !wallet.eq_ignore_ascii_case(solver_wallet))
                {
                    additional_reasons.push(
                        "registered agent payout wallet does not match solver_wallet".to_string(),
                    );
                }
            }
            None => additional_reasons.push("agent_id is not registered".to_string()),
        }
        capabilities = store
            .list_capabilities()
            .await
            .map_err(|_| {
                agent_claim_problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "eligibility_unavailable",
                    "evaluate_eligibility",
                    "registered agent capabilities could not be loaded",
                    "Retry without signing anything.",
                )
            })?
            .into_iter()
            .filter(|capability| capability.agent_id == agent_id)
            .map(|capability| capability.class)
            .collect();
        capabilities.sort_by_key(|class| format!("{class:?}"));
        capabilities.dedup();
    }
    let evidence = AgentEligibilityEvidence {
        agent_id,
        solver_wallet: solver_wallet.to_string(),
        capabilities,
        paid_completions,
        paid_usdc_base_units,
    };
    let mut decision = policy.evaluate(creator_wallet, &evidence);
    decision.reasons.extend(additional_reasons);
    decision.eligible = decision.reasons.is_empty();
    Ok((evidence, decision))
}

fn claim_authorization_window(
    candidate: &ClaimCandidate,
) -> Result<(String, u64), AgentClaimProblem> {
    let now = u64::try_from(Utc::now().timestamp()).unwrap_or_default();
    let exclusive_until = candidate
        .exclusive_until
        .and_then(|value| u64::try_from(value.timestamp()).ok())
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::CONFLICT,
                "exclusive_window_missing",
                "prepare_authorization",
                "candidate does not own a live exclusive window",
                "Poll until promoted from the waitlist, then request a fresh authorization.",
            )
        })?;
    let valid_before = exclusive_until
        .saturating_sub(15)
        .min(now.saturating_add(600));
    if valid_before < now.saturating_add(60) {
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "exclusive_window_expiring",
            "prepare_authorization",
            "fewer than 60 seconds remain in the hosted exclusive window",
            "Retry with a new idempotency_key after the canonical bounty is available again.",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"agent-bounties/claim-authorization-v1");
    hasher.update(candidate.id.as_bytes());
    hasher.update(candidate.network.as_bytes());
    hasher.update(candidate.bounty_contract.as_bytes());
    hasher.update(candidate.solver_wallet.as_bytes());
    Ok((
        format!("0x{}", hex::encode(hasher.finalize())),
        valid_before,
    ))
}

fn signed_claim_request_template(request: &AgentNativeClaimRequest) -> serde_json::Value {
    serde_json::json!({
        "method": "POST",
        "path": "/v1/base/autonomous-bounties/claims",
        "body": {
            "idempotency_key": request.idempotency_key,
            "network": request.network.as_deref().unwrap_or("base-mainnet"),
            "bounty_contract": request.bounty_contract,
            "solver_wallet": request.solver_wallet,
            "agent_id": request.agent_id,
            "request_bond_sponsorship": request.request_bond_sponsorship,
            "wallet_signature": "<replace with the unchanged 0x-prefixed result from wallet_request>",
            "source": request.source
        },
        "insert_signature_at": "body.wallet_signature",
        "wallet_signature_schema": "0x plus 130 hex characters (65 bytes: r || s || v)",
        "legacy_signature_at": "body.signature",
        "legacy_signature_schema": { "v": "integer 0, 1, 27, or 28", "r": "0x plus 64 hex characters", "s": "0x plus 64 hex characters" },
        "signature_source": "the unchanged wallet result from wallet_request"
    })
}

fn eip1193_wallet_request(
    solver_wallet: &str,
    signing_payload: &Eip3009AuthorizationTypedData,
) -> serde_json::Value {
    let payload = serde_json::json!(signing_payload);
    serde_json::json!({
        "method": "eth_signTypedData_v4",
        "params": [solver_wallet, payload.to_string()]
    })
}

#[allow(clippy::too_many_arguments)]
fn agent_claim_response(
    state: &SharedState,
    status: StatusCode,
    reservation: ClaimCandidateReservation,
    claim_bond: u64,
    sponsorship_requested: bool,
    sponsorship_available: bool,
    sponsorship: Option<BondSponsorship>,
    signing_payload: Option<Eip3009AuthorizationTypedData>,
    next_action: &str,
    next_request: Option<serde_json::Value>,
) -> Response {
    let browser_fallback_url = format!(
        "https://agentbounties.app/earn.html?bountyContract={}&solver={}",
        reservation.candidate.bounty_contract, reservation.candidate.solver_wallet
    );
    let sponsor_contract = sponsorship
        .as_ref()
        .map(|grant| grant.sponsor_wallet.clone())
        .or_else(|| {
            sponsorship_requested
                .then(|| {
                    state
                        .bond_sponsor
                        .contract_for(&reservation.candidate.network)
                })
                .flatten()
                .map(str::to_string)
        });
    let sponsorship_protocol = sponsor_contract
        .as_ref()
        .map(|_| "agent-bounties/atomic-claim-sponsor-v1".to_string());
    let wallet_request = signing_payload
        .as_ref()
        .map(|payload| eip1193_wallet_request(&reservation.candidate.solver_wallet, payload));
    let response = AgentNativeClaimResponse {
        schema_version: "agent-bounties/agent-native-claim-v1".to_string(),
        waitlist_position: reservation.waitlist_position,
        claim_bond: claim_bond.to_string(),
        sponsorship_requested,
        sponsorship_available,
        sponsorship_protocol,
        sponsor_contract,
        sponsorship,
        signing_payload,
        wallet_request,
        claim_transaction_hash: reservation.candidate.claim_transaction_hash.clone(),
        canonical_event_id: reservation.candidate.canonical_event_id,
        next_action: next_action.to_string(),
        next_request: next_request.map(|mut value| {
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "url".to_string(),
                    serde_json::json!(format!(
                        "{}/v1/base/autonomous-bounties/claims",
                        state.public_base_url.trim_end_matches('/')
                    )),
                );
            }
            value
        }),
        browser_fallback_url,
        evidence_boundary: "BountyClaimed proves round ownership. BountySettled proves payout. Hosted state, signatures, and transaction hashes prove neither.".to_string(),
        candidate: reservation.candidate,
    };
    (status, Json(response)).into_response()
}

fn agent_claim_problem(
    status: StatusCode,
    code: &str,
    failed_transition: &str,
    message: &str,
    next_action: &str,
) -> AgentClaimProblem {
    (
        status,
        Json(serde_json::json!({
            "schema_version": "agent-bounties/claim-problem-v1",
            "state": "failed",
            "failed_transition": failed_transition,
            "error": code,
            "message": message,
            "next_action": next_action
        })),
    )
}

fn map_agent_claim_db_error(error: DbError) -> AgentClaimProblem {
    match error {
        DbError::ClaimWaitlistFull => agent_claim_problem(
            StatusCode::TOO_MANY_REQUESTS,
            "waitlist_full",
            "reserve_candidate",
            "the bounded waitlist is full",
            "Choose the next claimable bounty. Poll this bounty after its active claim ends.",
        ),
        DbError::ClaimCandidateConflict(message) => agent_claim_problem(
            StatusCode::CONFLICT,
            "candidate_conflict",
            "reserve_candidate",
            &message,
            "Replay the original idempotency_key. Create a new key only after the prior candidate is terminal.",
        ),
        DbError::BondSponsorshipQuotaExceeded(message) => agent_claim_problem(
            StatusCode::TOO_MANY_REQUESTS,
            "sponsorship_cap_reached",
            "sponsor_bond",
            &message,
            "Fund the exact bond from the solver wallet. Request sponsorship again only after the rolling cap clears.",
        ),
        _ => agent_claim_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "coordination_storage_failed",
            "persist_state",
            "durable claim state could not be updated",
            "Retry with the same idempotency_key; do not create a second authorization.",
        ),
    }
}

fn validate_claim_authorization_signature(
    network: &BaseNetworkDescriptor,
    bounty_contract: &str,
    solver_wallet: &str,
    claim_bond: u64,
    nonce: &str,
    valid_before: u64,
    signature: &AutonomousBountyAuthorizationSignature,
) -> Result<(), AgentClaimProblem> {
    let now = u64::try_from(Utc::now().timestamp()).unwrap_or_default();
    let timeout = valid_before.saturating_sub(now);
    if timeout < 6 {
        return Err(agent_claim_problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "authorization_expired",
            "verify_authorization",
            "the bounded USDC authorization expires too soon",
            "Request a fresh candidate after the hosted exclusive window reopens.",
        ));
    }
    let resource = format!(
        "urn:agent-bounties:claim:{}:{}",
        network.chain_id, bounty_contract
    );
    let required = base_usdc_funding_challenge(
        resource,
        format!("eip155:{}", network.chain_id),
        &network.native_usdc_token_address,
        bounty_contract,
        claim_bond,
        timeout,
    )
    .map_err(|_| {
        agent_claim_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "authorization_challenge_failed",
            "verify_authorization",
            "the exact USDC authorization challenge could not be reconstructed",
            "Do not sign or transfer funds; retry with the same idempotency_key.",
        )
    })?;
    let signature_hex = joined_signature(signature)?;
    let payload = PaymentPayload {
        x402_version: X402_VERSION,
        resource: Some(required.resource.clone()),
        accepted: required.accepts[0].clone(),
        payload: serde_json::to_value(Eip3009Payload {
            signature: signature_hex,
            authorization: Eip3009Authorization {
                from: solver_wallet.to_string(),
                to: bounty_contract.to_string(),
                value: claim_bond.to_string(),
                valid_after: "0".to_string(),
                valid_before: valid_before.to_string(),
                nonce: nonce.to_string(),
            },
        })
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authorization_encoding_failed",
                "verify_authorization",
                "the signature payload could not be encoded",
                "Do not retry with a different signature; report the candidate ID.",
            )
        })?,
        extensions: required.extensions.clone(),
    };
    validate_funding_payload(&payload, &required, now).map_err(|_| {
        agent_claim_problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "authorization_invalid",
            "verify_authorization",
            "signature recovery did not match the solver wallet and exact bounded authorization",
            "Sign the returned signing_payload with solver_wallet; do not sign arbitrary calldata.",
        )
    })?;
    Ok(())
}

fn joined_signature(
    signature: &AutonomousBountyAuthorizationSignature,
) -> Result<String, AgentClaimProblem> {
    let r = signature.r.strip_prefix("0x").unwrap_or(&signature.r);
    let s = signature.s.strip_prefix("0x").unwrap_or(&signature.s);
    if r.len() != 64
        || s.len() != 64
        || !r.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !s.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(agent_claim_problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "signature_invalid",
            "verify_authorization",
            "signature r and s must each be one bytes32 hex value",
            "Return the wallet's exact typed-data signature as v, r, and s.",
        ));
    }
    let v = match signature.v {
        0 | 27 => "1b",
        1 | 28 => "1c",
        _ => {
            return Err(agent_claim_problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "signature_invalid",
                "verify_authorization",
                "signature v must be 0, 1, 27, or 28",
                "Return the wallet's exact typed-data signature as v, r, and s.",
            ))
        }
    };
    Ok(format!("0x{r}{s}{v}").to_ascii_lowercase())
}

async fn reserve_atomic_bond_sponsorship(
    state: &SharedState,
    candidate: &ClaimCandidate,
    claim_bond: u64,
    existing: Option<BondSponsorship>,
) -> Result<BondSponsorship, AgentClaimProblem> {
    if let Some(existing) = existing {
        if existing.status == BondSponsorshipStatus::Failed {
            return Err(agent_claim_problem(
                StatusCode::CONFLICT,
                "sponsorship_failed",
                "reserve_sponsorship",
                existing
                    .failure_message
                    .as_deref()
                    .unwrap_or("the atomic sponsorship is terminal"),
                "Start a fresh claim candidate only if the canonical bounty remains claimable.",
            ));
        }
        return Ok(existing);
    }
    let store = state.store.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "sponsorship_unavailable",
            "reserve_sponsorship",
            "durable sponsorship state is unavailable",
            "Fund the exact bond from the solver wallet. Replay the same idempotency_key with request_bond_sponsorship=false.",
        )
    })?;
    let sponsor_contract = state
        .bond_sponsor
        .contract_for(&candidate.network)
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "sponsorship_unavailable",
                "reserve_sponsorship",
                "no atomic sponsor vault is configured for this network",
                "Fund the exact bond directly. Replay on the canonical bounty network.",
            )
        })?;
    store
        .reserve_bond_sponsorship(
            &NewBondSponsorship {
                id: Uuid::new_v4(),
                claim_candidate_id: candidate.id,
                network: candidate.network.clone(),
                bounty_contract: candidate.bounty_contract.clone(),
                solver_wallet: candidate.solver_wallet.clone(),
                sponsor_wallet: sponsor_contract.to_string(),
                amount: claim_bond,
            },
            state.bond_sponsor.max_network_amount_24h,
            state.bond_sponsor.max_solver_amount_24h,
        )
        .await
        .map_err(map_agent_claim_db_error)
}

#[allow(clippy::too_many_arguments)]
async fn relay_atomic_sponsored_claim(
    state: &SharedState,
    candidate: &ClaimCandidate,
    item: &AutonomousBountyFeedItem,
    claim_bond: u64,
    nonce: &str,
    valid_before: u64,
    solver_signature: &AutonomousBountyAuthorizationSignature,
    sponsorship: &BondSponsorship,
) -> Result<ClaimCandidate, AgentClaimProblem> {
    if candidate.status == ClaimCandidateStatus::Relaying
        || sponsorship.status == BondSponsorshipStatus::Broadcast
    {
        return reconcile_agent_native_claim(state, candidate.clone(), claim_bond).await;
    }
    if sponsorship.status != BondSponsorshipStatus::Reserved {
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "sponsorship_state_invalid",
            "relay_atomic_claim",
            "the atomic sponsorship is not reserved for relay",
            "Replay the same request without creating another signature or sponsorship.",
        ));
    }
    let sponsor_contract = state
        .bond_sponsor
        .contract_for(&candidate.network)
        .filter(|contract| contract.eq_ignore_ascii_case(&sponsorship.sponsor_wallet))
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "sponsor_contract_mismatch",
                "relay_atomic_claim",
                "the reserved sponsor vault does not match current network configuration",
                "Do not sign or fund again; report the candidate ID.",
            )
        })?;
    let grant_signer = state.bond_sponsor.grant_signer().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "grant_signer_unavailable",
            "relay_atomic_claim",
            "the atomic sponsor grant signer is unavailable",
            "Retry the same signed request later.",
        )
    })?;
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|_| state.x402_relayer.enabled)
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "claim_relayer_unavailable",
                "relay_atomic_claim",
                "the hosted gas relayer is unavailable",
                "Call plan_autonomous_bounty_claim. Submit its exact direct-wallet calls.",
            )
        })?;
    let planner = configured_autonomous_planner(&candidate.network).map_err(|status| {
        agent_claim_problem(
            status,
            "planner_unavailable",
            "relay_atomic_claim",
            "the canonical claim planner is unavailable",
            "Do not sign arbitrary calldata; retry later.",
        )
    })?;
    let terms = item.terms.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::CONFLICT,
            "terms_unavailable",
            "relay_atomic_claim",
            "the hash-verified terms needed for the sponsor grant are unavailable",
            "Do not relay until terms_valid=true.",
        )
    })?;
    let now = u64::try_from(Utc::now().timestamp()).unwrap_or_default();
    let deadline = valid_before.min(now.saturating_add(300));
    if deadline <= now {
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "authorization_expired",
            "relay_atomic_claim",
            "the solver authorization expired before the atomic grant could be issued",
            "Start a fresh candidate and sign its new bounded payload once.",
        ));
    }
    let grant = AtomicClaimSponsorGrant {
        sponsor_contract: sponsor_contract.to_string(),
        bounty_contract: candidate.bounty_contract.clone(),
        solver: candidate.solver_wallet.clone(),
        round: next_claim_round(item)?,
        bond: u128::from(claim_bond),
        terms_hash: item.terms_hash.clone(),
        policy_hash: terms.policy_hash.clone(),
        authorization_nonce: nonce.to_string(),
        valid_after: 0,
        valid_before,
        grant_nonce: atomic_sponsorship_grant_nonce(sponsorship),
        deadline,
    };
    let grant_digest = planner
        .atomic_sponsor_grant_digest(&candidate.network, &grant)
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "grant_digest_failed",
                "relay_atomic_claim",
                "the exact atomic sponsor grant could not be encoded",
                "Do not sign or broadcast arbitrary data; report the candidate ID.",
            )
        })?;
    let grant_signature = grant_signer.sign_digest(&grant_digest).map_err(|_| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "grant_signing_failed",
            "relay_atomic_claim",
            "the bounded sponsor grant could not be signed",
            "Retry the same solver-signed request later.",
        )
    })?;
    let plan = planner
        .plan_atomic_sponsored_claim(
            &candidate.network,
            &grant,
            &grant_signature,
            solver_signature,
            &relayer.address(),
        )
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "atomic_claim_plan_invalid",
                "relay_atomic_claim",
                "the signed atomic claim could not be converted into exact vault calldata",
                "Do not sign again; report the candidate ID.",
            )
        })?;
    validate_atomic_sponsored_claim_intent(
        &plan.relay_transaction,
        sponsor_contract,
        &relayer.address(),
    )?;
    let descriptor = base_network_descriptor(&candidate.network).map_err(|_| {
        agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "network_invalid",
            "relay_atomic_claim",
            "candidate network is unsupported",
            "Do not sign or broadcast anything.",
        )
    })?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&candidate.network)
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "rpc_unavailable",
                "relay_atomic_claim",
                "Base RPC is unavailable",
                "Retry the same signed request.",
            )
        })?;
    let store = state.store.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "coordination_unavailable",
            "relay_atomic_claim",
            "durable claim state is unavailable",
            "Retry later.",
        )
    })?;
    let lease = store
        .acquire_x402_relayer_lease(&candidate.network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_agent_claim_db_error)?
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "relay_busy",
                "relay_atomic_claim",
                "the bounded gas relay is busy",
                "Retry the same signed request.",
            )
        })?;
    let durable_state: Result<(ClaimCandidate, BondSponsorship), AgentClaimProblem> = async {
        let durable_candidate = store
            .get_claim_candidate(candidate.id)
            .await
            .map_err(map_agent_claim_db_error)?
            .ok_or_else(|| {
                agent_claim_problem(
                    StatusCode::CONFLICT,
                    "candidate_missing",
                    "relay_atomic_claim",
                    "the durable claim candidate disappeared before relay",
                    "Do not sign or fund again; report the candidate ID.",
                )
            })?;
        let durable_sponsorship = store
            .get_bond_sponsorship_for_candidate(candidate.id)
            .await
            .map_err(map_agent_claim_db_error)?
            .ok_or_else(|| {
                agent_claim_problem(
                    StatusCode::CONFLICT,
                    "sponsorship_missing",
                    "relay_atomic_claim",
                    "the durable sponsorship disappeared before relay",
                    "Do not sign or fund again; report the candidate ID.",
                )
            })?;
        Ok((durable_candidate, durable_sponsorship))
    }
    .await;
    let (durable_candidate, durable_sponsorship) = match durable_state {
        Ok(state) => state,
        Err(problem) => {
            let _ = store
                .release_x402_relayer_lease(&candidate.network, lease)
                .await;
            return Err(problem);
        }
    };
    if durable_candidate.status == ClaimCandidateStatus::Claimed {
        store
            .release_x402_relayer_lease(&candidate.network, lease)
            .await
            .map_err(map_agent_claim_db_error)?;
        return Ok(durable_candidate);
    }
    if durable_candidate.status == ClaimCandidateStatus::Relaying
        && durable_sponsorship.status == BondSponsorshipStatus::Broadcast
    {
        store
            .release_x402_relayer_lease(&candidate.network, lease)
            .await
            .map_err(map_agent_claim_db_error)?;
        return reconcile_agent_native_claim(state, durable_candidate, claim_bond).await;
    }
    if durable_sponsorship.status != BondSponsorshipStatus::Reserved
        || !matches!(
            durable_candidate.status,
            ClaimCandidateStatus::Exclusive
                | ClaimCandidateStatus::Sponsoring
                | ClaimCandidateStatus::AuthorizationReady
        )
    {
        store
            .release_x402_relayer_lease(&candidate.network, lease)
            .await
            .map_err(map_agent_claim_db_error)?;
        return Err(agent_claim_problem(
            StatusCode::CONFLICT,
            "sponsorship_state_changed",
            "relay_atomic_claim",
            "the durable candidate or sponsorship changed before relay",
            "Replay the same request; do not sign or fund again.",
        ));
    }
    let relay_result = tokio::time::timeout(
        Duration::from_secs(state.bond_sponsor.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            descriptor.chain_id,
            &plan.relay_transaction,
            state.bond_sponsor.max_gas,
            state.bond_sponsor.max_fee_per_gas_wei,
        ),
    )
    .await;
    let transaction = match relay_result {
        Ok(Ok(transaction)) => transaction,
        Ok(Err(_)) => {
            store
                .release_x402_relayer_lease(&candidate.network, lease)
                .await
                .map_err(map_agent_claim_db_error)?;
            return Err(agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "atomic_claim_broadcast_unknown",
                "relay_atomic_claim",
                "the atomic claim was not returned as a recorded broadcast",
                "Retry the same request and signatures; the grant and USDC nonces prevent duplicate use.",
            ));
        }
        Err(_) => {
            store
                .release_x402_relayer_lease(&candidate.network, lease)
                .await
                .map_err(map_agent_claim_db_error)?;
            return Err(agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "atomic_claim_rpc_timeout",
                "relay_atomic_claim",
                "the relay RPC deadline elapsed without returning a transaction hash",
                "Retry the same request and signatures; do not create another sponsorship.",
            ));
        }
    };
    let mark_result = store
        .mark_atomic_sponsored_claim_broadcast(candidate.id, sponsorship.id, &transaction.tx_hash)
        .await;
    let release_result = store
        .release_x402_relayer_lease(&candidate.network, lease)
        .await;
    let (candidate, _) = match mark_result {
        Ok(value) => value,
        Err(error) => {
            let _ = release_result;
            return Err(map_agent_claim_db_error(error));
        }
    };
    release_result.map_err(map_agent_claim_db_error)?;
    reconcile_agent_native_claim(state, candidate, claim_bond).await
}

fn next_claim_round(item: &AutonomousBountyFeedItem) -> Result<u64, AgentClaimProblem> {
    item.events
        .iter()
        .filter_map(|event| event.data.get("round").and_then(serde_json::Value::as_u64))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "round_overflow",
                "relay_atomic_claim",
                "the canonical bounty round cannot advance safely",
                "Do not sign or broadcast; report the bounty contract.",
            )
        })
}

fn atomic_sponsorship_grant_nonce(sponsorship: &BondSponsorship) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"agent-bounties/atomic-sponsor-grant-v1");
    hasher.update(sponsorship.id.as_bytes());
    hasher.update(sponsorship.claim_candidate_id.as_bytes());
    hasher.update(sponsorship.network.as_bytes());
    hasher.update(sponsorship.bounty_contract.as_bytes());
    hasher.update(sponsorship.solver_wallet.as_bytes());
    format!("0x{}", hex::encode(hasher.finalize()))
}

fn validate_atomic_sponsored_claim_intent(
    intent: &EvmTransactionIntent,
    sponsor_contract: &str,
    relayer: &str,
) -> Result<(), AgentClaimProblem> {
    let calldata = intent.data.strip_prefix("0x").unwrap_or_default();
    if intent.value_wei != 0
        || intent.function
            != "sponsorAndClaim((address,address,uint64,uint256,bytes32,bytes32,bytes32,uint256,uint256,bytes32,uint256),bytes,uint8,bytes32,bytes32)"
        || !intent.to.eq_ignore_ascii_case(sponsor_contract)
        || intent
            .from
            .as_deref()
            .is_none_or(|from| !from.eq_ignore_ascii_case(relayer))
        || !calldata.starts_with("ba3ddedd")
        || calldata.len() != 612 * 2
    {
        return Err(agent_claim_problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "atomic_claim_intent_invalid",
            "relay_atomic_claim",
            "generated transaction exceeded the exact zero-ETH sponsorAndClaim policy",
            "Do not broadcast; report the candidate ID.",
        ));
    }
    Ok(())
}

async fn relay_agent_native_claim(
    state: &SharedState,
    candidate: &ClaimCandidate,
    claim_bond: u64,
    nonce: &str,
    valid_before: u64,
    signature: &AutonomousBountyAuthorizationSignature,
) -> Result<ClaimCandidate, AgentClaimProblem> {
    if candidate.status == ClaimCandidateStatus::Relaying {
        return reconcile_agent_native_claim(state, candidate.clone(), claim_bond).await;
    }
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|_| state.x402_relayer.enabled)
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "claim_relayer_unavailable",
                "relay_claim",
                "the hosted gas relayer is unavailable",
                "Use the direct wallet_calls from plan_autonomous_bounty_claim.",
            )
        })?;
    let planner = configured_autonomous_planner(&candidate.network).map_err(|status| {
        agent_claim_problem(
            status,
            "planner_unavailable",
            "relay_claim",
            "the canonical claim planner is unavailable",
            "Do not sign arbitrary calldata; retry later.",
        )
    })?;
    let plan = planner
        .plan_authorized_claim(
            &candidate.network,
            &candidate.bounty_contract,
            &candidate.solver_wallet,
            u128::from(claim_bond),
            nonce,
            valid_before,
            signature,
            Some(&relayer.address()),
        )
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "claim_plan_invalid",
                "relay_claim",
                "the signed claim could not be converted into the exact relay transaction",
                "Sign only the returned signing_payload and retry with the same idempotency_key.",
            )
        })?;
    validate_agent_claim_relay_intent(
        &plan.relay_transaction,
        &candidate.bounty_contract,
        &relayer.address(),
    )?;
    let descriptor = base_network_descriptor(&candidate.network).map_err(|_| {
        agent_claim_problem(
            StatusCode::BAD_REQUEST,
            "network_invalid",
            "relay_claim",
            "candidate network is unsupported",
            "Do not sign or broadcast anything.",
        )
    })?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&candidate.network)
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "rpc_unavailable",
                "relay_claim",
                "Base RPC is unavailable",
                "Retry the same signed request.",
            )
        })?;
    let store = state.store.as_ref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "coordination_unavailable",
            "relay_claim",
            "durable claim state is unavailable",
            "Call plan_autonomous_bounty_claim. Submit its exact direct-wallet calls.",
        )
    })?;
    let lease = store
        .acquire_x402_relayer_lease(&candidate.network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_agent_claim_db_error)?
        .ok_or_else(|| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "relay_busy",
                "relay_claim",
                "the bounded gas relay is busy",
                "Retry the same signed request.",
            )
        })?;
    let relay_result = tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            descriptor.chain_id,
            &plan.relay_transaction,
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        ),
    )
    .await;
    let release_result = store
        .release_x402_relayer_lease(&candidate.network, lease)
        .await
        .map_err(map_agent_claim_db_error);
    let transaction = match relay_result {
        Ok(Ok(transaction)) => transaction,
        Ok(Err(error)) => {
            release_result?;
            return Err(agent_claim_problem(
                if error.to_string().to_ascii_lowercase().contains("revert") {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                },
                "claim_broadcast_failed",
                "relay_claim",
                "the exact claim transaction failed simulation or broadcast",
                "Check canonical claimability and solver bond balance, then retry the same signed request before it expires.",
            ));
        }
        Err(_) => {
            release_result?;
            return Err(agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "claim_rpc_timeout",
                "relay_claim",
                "the relay RPC deadline elapsed before broadcast was recorded",
                "Retry the same signed request; EIP-3009 nonce reuse cannot double-claim.",
            ));
        }
    };
    release_result?;
    let candidate = store
        .mark_claim_candidate_relaying(candidate.id, &transaction.tx_hash)
        .await
        .map_err(map_agent_claim_db_error)?;
    reconcile_agent_native_claim(state, candidate, claim_bond).await
}

fn validate_agent_claim_relay_intent(
    intent: &EvmTransactionIntent,
    bounty_contract: &str,
    relayer: &str,
) -> Result<(), AgentClaimProblem> {
    let calldata = intent.data.strip_prefix("0x").unwrap_or_default();
    if intent.value_wei != 0
        || intent.function
            != "claimWithAuthorization(address,uint256,uint256,bytes32,uint8,bytes32,bytes32)"
        || !intent.to.eq_ignore_ascii_case(bounty_contract)
        || intent
            .from
            .as_deref()
            .is_none_or(|from| !from.eq_ignore_ascii_case(relayer))
        || calldata.len() != 8 + 7 * 64
    {
        return Err(agent_claim_problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "claim_intent_invalid",
            "relay_claim",
            "generated claim transaction exceeded the exact no-ETH claimWithAuthorization policy",
            "Do not broadcast; report the candidate ID.",
        ));
    }
    Ok(())
}

async fn reconcile_agent_native_claim(
    state: &SharedState,
    candidate: ClaimCandidate,
    claim_bond: u64,
) -> Result<ClaimCandidate, AgentClaimProblem> {
    if candidate.status == ClaimCandidateStatus::Claimed {
        return Ok(candidate);
    }
    let tx_hash = candidate.claim_transaction_hash.as_deref().ok_or_else(|| {
        agent_claim_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "claim_tx_missing",
            "confirm_claim",
            "relaying candidate has no transaction hash",
            "Retry with the same idempotency_key.",
        )
    })?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&candidate.network)
        .map_err(|_| {
            agent_claim_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "rpc_unavailable",
                "confirm_claim",
                "Base RPC is unavailable",
                "Retry the same signed request.",
            )
        })?;
    let deadline = Instant::now() + Duration::from_secs(state.x402_relayer.wait_seconds);
    loop {
        let receipt = fetch_transaction_receipt(&rpc_url, tx_hash, 1)
            .await
            .map_err(|_| {
                agent_claim_problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "rpc_unavailable",
                    "confirm_claim",
                    "the claim receipt could not be fetched",
                    "Retry the same signed request.",
                )
            })?
            .result;
        if let Some(receipt) = receipt {
            if receipt.succeeded().map_err(|_| {
                agent_claim_problem(
                    StatusCode::BAD_GATEWAY,
                    "receipt_invalid",
                    "confirm_claim",
                    "the claim receipt status is invalid",
                    "Retry the same signed request.",
                )
            })? == Some(false)
            {
                let store = state.store.as_ref().ok_or_else(|| {
                    agent_claim_problem(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "coordination_unavailable",
                        "confirm_claim",
                        "durable claim state is unavailable",
                        "Retry later.",
                    )
                })?;
                let sponsorship = store
                    .get_bond_sponsorship_for_candidate(candidate.id)
                    .await
                    .map_err(map_agent_claim_db_error)?;
                if let Some(sponsorship) =
                    sponsorship.filter(|item| item.status == BondSponsorshipStatus::Broadcast)
                {
                    store
                        .mark_atomic_sponsored_claim_failed(
                            candidate.id,
                            sponsorship.id,
                            "transaction_reverted",
                            "The confirmed atomic claim transaction reverted; no bond moved and no canonical claim was created.",
                        )
                        .await
                        .map_err(map_agent_claim_db_error)?;
                } else {
                    store
                        .mark_claim_candidate_failed(
                            candidate.id,
                            "transaction_reverted",
                            "The confirmed claim transaction reverted; no canonical claim was created.",
                        )
                        .await
                        .map_err(map_agent_claim_db_error)?;
                }
                return Err(agent_claim_problem(
                    StatusCode::BAD_GATEWAY,
                    "claim_reverted",
                    "confirm_claim",
                    "the confirmed transaction reverted and emitted no canonical claim",
                    "If the bounty is still claimable, start a fresh candidate with a new idempotency_key.",
                ));
            }
            if let Some(block) = receipt.block_number().map_err(|_| {
                agent_claim_problem(
                    StatusCode::BAD_GATEWAY,
                    "receipt_invalid",
                    "confirm_claim",
                    "the claim receipt block is invalid",
                    "Retry the same signed request.",
                )
            })? {
                let events =
                    decode_autonomous_bounty_logs(receipt.logs_to_evm_logs().map_err(|_| {
                        agent_claim_problem(
                            StatusCode::BAD_GATEWAY,
                            "receipt_invalid",
                            "confirm_claim",
                            "the claim receipt logs are invalid",
                            "Retry the same signed request.",
                        )
                    })?)
                    .map_err(|_| {
                        agent_claim_problem(
                            StatusCode::BAD_GATEWAY,
                            "claim_event_invalid",
                            "confirm_claim",
                            "the claim receipt could not be decoded",
                            "Do not treat the round as claimed; report the transaction hash.",
                        )
                    })?;
                let claim = events.iter().find(|event| {
                    event.kind == AutonomousBountyEventKind::BountyClaimed
                        && event
                            .contract_address
                            .eq_ignore_ascii_case(&candidate.bounty_contract)
                        && event.data["solver"].as_str().is_some_and(|solver| {
                            solver.eq_ignore_ascii_case(&candidate.solver_wallet)
                        })
                        && json_u128(&event.data["claim_bond"]) == Some(u128::from(claim_bond))
                });
                let Some(claim) = claim else {
                    return Err(agent_claim_problem(
                        StatusCode::BAD_GATEWAY,
                        "claim_event_mismatch",
                        "confirm_claim",
                        "the confirmed transaction did not emit the exact canonical BountyClaimed event",
                        "Do not start work; report the transaction hash.",
                    ));
                };
                let latest = fetch_block_number(&rpc_url, 2).await.map_err(|_| {
                    agent_claim_problem(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "rpc_unavailable",
                        "confirm_claim",
                        "latest Base block could not be fetched",
                        "Retry the same signed request.",
                    )
                })?;
                if latest.saturating_sub(block).saturating_add(1)
                    >= state.x402_relayer.confirmations
                {
                    let store = state.store.as_ref().ok_or_else(|| {
                        agent_claim_problem(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "coordination_unavailable",
                            "confirm_claim",
                            "durable claim state is unavailable",
                            "Retry the same signed request.",
                        )
                    })?;
                    for event in events.iter().filter(|event| {
                        event
                            .contract_address
                            .eq_ignore_ascii_case(&candidate.bounty_contract)
                    }) {
                        store
                            .upsert_autonomous_bounty_event(&candidate.network, event)
                            .await
                            .map_err(map_agent_claim_db_error)?;
                    }
                    let sponsorship = store
                        .get_bond_sponsorship_for_candidate(candidate.id)
                        .await
                        .map_err(map_agent_claim_db_error)?;
                    if let Some(sponsorship) =
                        sponsorship.filter(|item| item.status == BondSponsorshipStatus::Broadcast)
                    {
                        return store
                            .mark_atomic_sponsored_claim_confirmed(
                                candidate.id,
                                sponsorship.id,
                                claim.id,
                                block,
                            )
                            .await
                            .map(|(candidate, _)| candidate)
                            .map_err(map_agent_claim_db_error);
                    }
                    return store
                        .mark_claim_candidate_claimed(candidate.id, claim.id)
                        .await
                        .map_err(map_agent_claim_db_error);
                }
            }
        }
        if Instant::now() >= deadline {
            return Ok(candidate);
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn require_claimable_autonomous_item(item: &AutonomousBountyFeedItem) -> Result<(), StatusCode> {
    if !autonomous_bounty_is_earning_ready(item) {
        return Err(StatusCode::CONFLICT);
    }
    Ok(())
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/submission-plan", responses((status = 200, description = "Unsigned submission commitment plan for autonomous verification")))]
async fn plan_autonomous_bounty_submission(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountySubmissionRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.bounty_contract).await?;
    configured_autonomous_planner(network)?
        .plan_submission(
            &request.bounty_contract,
            &request.solver,
            &request.submission_hash,
            &request.evidence_hash,
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/base/autonomous-bounties/submission-preparation",
    responses(
        (status = 200, description = "Canonical active-claim validation, deterministic submission commitments, exact EIP-712 payload, and unsigned relay/evidence templates"),
        (status = 400, description = "Malformed wallet, artifact reference, evidence object, or network"),
        (status = 404, description = "Bounty contract is not an indexed canonical instance"),
        (status = 409, description = "Bounty is not an executable active claim owned by this solver or expires too soon"),
        (status = 503, description = "Canonical indexed state or planner configuration is unavailable")
    )
)]
async fn prepare_autonomous_bounty_submission(
    State(state): State<SharedState>,
    Json(request): Json<PrepareAutonomousBountySubmissionRequest>,
) -> Result<Json<AutonomousBountySubmissionPreparation>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    let observed_at_unix =
        u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    build_autonomous_submission_preparation(
        &configured_autonomous_planner(network)?,
        network,
        &item,
        &request.solver_wallet,
        &request.artifact_reference,
        request.evidence,
        observed_at_unix,
    )
    .map(Json)
    .map_err(|error| match error {
        ChainBaseError::InvalidSubmissionPreparation(_) => StatusCode::CONFLICT,
        ChainBaseError::InvalidSubmissionEvidence(_)
        | ChainBaseError::InvalidAddress(_)
        | ChainBaseError::InvalidCanonicalJson(_)
        | ChainBaseError::UnknownNetwork(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/submission-authorization-plan", responses((status = 200, description = "Exact EIP-712 submission authorization for a gas-sponsored submitWithSignature relay")))]
async fn plan_autonomous_bounty_submission_authorization(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountySubmissionAuthorizationRequest>,
) -> Result<Json<AutonomousBountySubmissionAuthorizationTypedData>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.submission.bounty_contract).await?;
    configured_autonomous_planner(network)?
        .plan_submission_authorization(network, &request.submission)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/verification-attestation-plan", responses((status = 200, description = "Exact EIP-712 payload for one committed verifier to sign")))]
async fn plan_autonomous_verification_attestation(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousVerificationAttestationRequest>,
) -> Result<Json<AutonomousVerificationAttestationTypedData>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item =
        indexed_autonomous_bounty(&state, network, &request.attestation.bounty_contract).await?;
    let observed_at = u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_attestation_request_against_feed(&item, &request.attestation, observed_at)
        .map_err(|_| StatusCode::CONFLICT)?;
    configured_autonomous_planner(network)?
        .plan_verification_attestation(network, &request.attestation)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/module-settlement-plan", responses((status = 200, description = "Permissionless deterministic verifier call that atomically settles on pass")))]
async fn plan_autonomous_module_settlement(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousModuleSettlementRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    require_autonomous_item_mode(&item, "deterministic_module")?;
    configured_autonomous_planner(network)?
        .plan_module_settlement(
            &request.bounty_contract,
            request.caller.as_deref(),
            &request.proof,
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/attestation-settlement-plan", responses((status = 200, description = "Permissionless committed verifier quorum relay that settles or reopens atomically")))]
async fn plan_autonomous_attestation_settlement(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousAttestationSettlementRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if !item.terms_valid {
        return Err(StatusCode::CONFLICT);
    }
    let mode = autonomous_item_mode(&item)?;
    if mode != "signed_quorum" && mode != "ai_judge_quorum" {
        return Err(StatusCode::CONFLICT);
    }
    let policy = item
        .terms
        .as_ref()
        .map(|terms| &terms.document.verification_policy)
        .ok_or(StatusCode::CONFLICT)?;
    let threshold = policy
        .get("threshold")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(StatusCode::CONFLICT)?;
    if request.attestations.len() != threshold {
        return Err(StatusCode::CONFLICT);
    }
    let allowed = policy
        .get("verifiers")
        .and_then(serde_json::Value::as_array)
        .ok_or(StatusCode::CONFLICT)?;
    if request.attestations.iter().any(|attestation| {
        !allowed.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|verifier| verifier.eq_ignore_ascii_case(&attestation.verifier))
        })
    }) {
        return Err(StatusCode::CONFLICT);
    }
    configured_autonomous_planner(network)?
        .plan_attestation_settlement(
            &request.bounty_contract,
            request.caller.as_deref(),
            &request.attestations,
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/expire-claim-plan", responses((status = 200, description = "Permissionless expired-claim release plan")))]
async fn plan_autonomous_expire_claim(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if item.status != "claimed" {
        return Err(StatusCode::CONFLICT);
    }
    configured_autonomous_planner(network)?
        .plan_expire_claim(&request.bounty_contract, request.caller.as_deref())
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/expire-submission-plan", responses((status = 200, description = "Permissionless expired-submission release plan")))]
async fn plan_autonomous_expire_submission(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if item.status != "submitted" {
        return Err(StatusCode::CONFLICT);
    }
    configured_autonomous_planner(network)?
        .plan_expire_submission(&request.bounty_contract, request.caller.as_deref())
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/base/autonomous-bounties/timeout-relay",
    request_body = RelayAutonomousTimeoutRequest,
    responses(
        (status = 200, description = "Canonical timeout event confirmed", body = RelayAutonomousTimeoutResponse),
        (status = 202, description = "Bounded timeout transaction broadcast; confirmation pending", body = RelayAutonomousTimeoutResponse),
        (status = 404, description = "Canonical indexed bounty not found"),
        (status = 409, description = "Requested transition does not match the indexed bounty state or immutable deadline"),
        (status = 422, description = "Generated relay intent violated the bounded timeout policy"),
        (status = 503, description = "Hosted gas relayer, database lease, or Base RPC unavailable")
    )
)]
async fn relay_autonomous_timeout(
    State(state): State<SharedState>,
    Json(request): Json<RelayAutonomousTimeoutRequest>,
) -> Result<Response, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let descriptor = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let network = match descriptor.chain_id {
        8_453 => "base-mainnet",
        84_532 => "base-sepolia",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let bounty_contract =
        normalize_evm_address(&request.bounty_contract).map_err(|_| StatusCode::BAD_REQUEST)?;
    let item = indexed_autonomous_bounty(&state, network, &bounty_contract).await?;
    if !item.terms_valid || item.status != request.action.previous_bounty_state() {
        return Err(StatusCode::CONFLICT);
    }

    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|_| state.x402_relayer.enabled)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let relayer_address = relayer.address();
    let planner = configured_autonomous_planner(network)?;
    let intent = match request.action {
        AutonomousTimeoutAction::ExpireClaim => {
            planner.plan_expire_claim(&bounty_contract, Some(&relayer_address))
        }
        AutonomousTimeoutAction::ExpireSubmission => {
            planner.plan_expire_submission(&bounty_contract, Some(&relayer_address))
        }
    }
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_autonomous_timeout_intent(
        &intent,
        &bounty_contract,
        &relayer_address,
        request.action,
    )?;

    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let lease_token = store
        .acquire_x402_relayer_lease(network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let relay_result = tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            descriptor.chain_id,
            &intent,
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        ),
    )
    .await;
    let release_result = store
        .release_x402_relayer_lease(network, lease_token)
        .await
        .map_err(map_x402_db_error);
    let transaction = match relay_result {
        Ok(Ok(transaction)) => transaction,
        Ok(Err(error)) => return Err(timeout_relay_status(&error)),
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    release_result?;

    let confirmation = wait_for_timeout_confirmation(
        &rpc_url,
        &bounty_contract,
        request.action,
        &transaction,
        state.x402_relayer.confirmations,
        state.x402_relayer.wait_seconds,
    )
    .await?;
    let response = RelayAutonomousTimeoutResponse {
        network: network.to_string(),
        bounty_contract,
        action: request.action,
        previous_bounty_state: request.action.previous_bounty_state().to_string(),
        expected_bounty_state: request.action.expected_bounty_state().to_string(),
        expected_canonical_event: request.action.expected_event_name().to_string(),
        transaction_hash: transaction.tx_hash,
        relayer: transaction.relayer,
        confirmed: confirmation.confirmed,
        confirmed_block: confirmation.confirmed_block,
        canonical_event_id: confirmation.canonical_event_id,
        evidence_boundary: format!(
            "Only a confirmed {} event proves this timeout transition. It is bond/lifecycle evidence, not bounty payout or BountySettled evidence.",
            request.action.expected_event_name()
        ),
    };
    Ok((
        if response.confirmed {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        Json(response),
    )
        .into_response())
}

impl AutonomousTimeoutAction {
    fn previous_bounty_state(self) -> &'static str {
        match self {
            Self::ExpireClaim => "claimed",
            Self::ExpireSubmission => "submitted",
        }
    }

    fn expected_bounty_state(self) -> &'static str {
        "claimable"
    }

    fn function(self) -> &'static str {
        match self {
            Self::ExpireClaim => "expireClaim()",
            Self::ExpireSubmission => "expireSubmission()",
        }
    }

    fn calldata(self) -> &'static str {
        match self {
            Self::ExpireClaim => "0x1257d2c8",
            Self::ExpireSubmission => "0xf9251ec7",
        }
    }

    fn expected_event_name(self) -> &'static str {
        match self {
            Self::ExpireClaim => "ClaimExpired",
            Self::ExpireSubmission => "SubmissionExpired",
        }
    }

    fn expected_event_kind(self) -> AutonomousBountyEventKind {
        match self {
            Self::ExpireClaim => AutonomousBountyEventKind::ClaimExpired,
            Self::ExpireSubmission => AutonomousBountyEventKind::SubmissionExpired,
        }
    }
}

fn validate_autonomous_timeout_intent(
    intent: &EvmTransactionIntent,
    bounty_contract: &str,
    relayer: &str,
    action: AutonomousTimeoutAction,
) -> Result<(), StatusCode> {
    if intent.value_wei != 0
        || intent.function != action.function()
        || !intent.to.eq_ignore_ascii_case(bounty_contract)
        || intent
            .from
            .as_deref()
            .is_none_or(|from| !from.eq_ignore_ascii_case(relayer))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if !intent.data.eq_ignore_ascii_case(action.calldata()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok(())
}

fn timeout_relay_status(error: &ChainBaseError) -> StatusCode {
    match error {
        ChainBaseError::RelayerProvider(message)
            if message.to_ascii_lowercase().contains("revert") =>
        {
            StatusCode::CONFLICT
        }
        ChainBaseError::InvalidRelayIntent(_)
        | ChainBaseError::RelayerChainMismatch { .. }
        | ChainBaseError::RelayerGasLimitExceeded { .. }
        | ChainBaseError::RelayerFeeCapExceeded { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[derive(Debug)]
struct TimeoutRelayConfirmation {
    confirmed: bool,
    confirmed_block: Option<u64>,
    canonical_event_id: Option<String>,
}

async fn wait_for_timeout_confirmation(
    rpc_url: &str,
    bounty_contract: &str,
    action: AutonomousTimeoutAction,
    transaction: &BaseRelayedTransaction,
    required_confirmations: u64,
    wait_seconds: u64,
) -> Result<TimeoutRelayConfirmation, StatusCode> {
    let deadline = Instant::now() + Duration::from_secs(wait_seconds);
    loop {
        let receipt = fetch_transaction_receipt(rpc_url, &transaction.tx_hash, 1)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .result;
        if let Some(receipt) = receipt {
            if receipt
                .succeeded()
                .map_err(|error| base_rpc_fetch_status(&error))?
                == Some(false)
            {
                return Err(StatusCode::BAD_GATEWAY);
            }
            let block_number = receipt
                .block_number()
                .map_err(|error| base_rpc_fetch_status(&error))?
                .ok_or(StatusCode::BAD_GATEWAY)?;
            let events = decode_autonomous_bounty_logs(
                receipt
                    .logs_to_evm_logs()
                    .map_err(|error| base_rpc_fetch_status(&error))?,
            )
            .map_err(|error| base_rpc_fetch_status(&error))?;
            let event = events.into_iter().find(|event| {
                event.kind == action.expected_event_kind()
                    && event.contract_address.eq_ignore_ascii_case(bounty_contract)
            });
            let event = event.ok_or(StatusCode::BAD_GATEWAY)?;
            let latest_block = fetch_block_number(rpc_url, 2)
                .await
                .map_err(|error| base_rpc_fetch_status(&error))?;
            let confirmations = latest_block.saturating_sub(block_number).saturating_add(1);
            if confirmations >= required_confirmations {
                return Ok(TimeoutRelayConfirmation {
                    confirmed: true,
                    confirmed_block: Some(block_number),
                    canonical_event_id: Some(event.log_key),
                });
            }
        }
        if Instant::now() >= deadline {
            return Ok(TimeoutRelayConfirmation {
                confirmed: false,
                confirmed_block: None,
                canonical_event_id: None,
            });
        }
        sleep(Duration::from_secs(1)).await;
    }
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/cancel-plan", responses((status = 200, description = "Creator or post-deadline cancellation plan")))]
async fn plan_autonomous_cancel(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    let caller = request.caller.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
    let funding_deadline = item
        .terms
        .as_ref()
        .and_then(|terms| terms.document.contract_terms["funding_deadline"].as_u64());
    let observed_at =
        u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let caller = validate_autonomous_cancel_authority(
        &item.status,
        &item.creator,
        caller,
        funding_deadline,
        observed_at,
    )
    .map_err(|error| match error {
        ChainBaseError::InvalidAddress(_) => StatusCode::BAD_REQUEST,
        ChainBaseError::InvalidVerificationConfiguration(message)
            if message.contains("cannot be cancelled") =>
        {
            StatusCode::CONFLICT
        }
        _ => StatusCode::FORBIDDEN,
    })?;
    configured_autonomous_planner(network)?
        .plan_cancel(&request.bounty_contract, Some(&caller))
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/refund-withdrawal-plan", responses((status = 200, description = "Contributor pull-refund transaction plan after cancellation")))]
async fn plan_autonomous_refund_withdrawal(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if item.status != "cancelled" {
        return Err(StatusCode::CONFLICT);
    }
    let contributor = request.caller.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
    configured_autonomous_planner(network)?
        .plan_refund_withdrawal(&request.bounty_contract, contributor)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/bounded-wallet-cancel-refund-plan", responses((status = 200, description = "Owner-only atomic bounded-wallet V2 cancellation and creator refund plan")))]
async fn plan_bounded_wallet_cancel_refund(
    State(state): State<SharedState>,
    Json(request): Json<PlanBoundedWalletCancelRefundRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if !item.creator.eq_ignore_ascii_case(&request.bounded_wallet) {
        return Err(StatusCode::FORBIDDEN);
    }
    let planner = configured_autonomous_planner(network)?;
    let plan = match item.status.as_str() {
        "open" | "claimable" => planner.plan_bounded_wallet_cancel_refund(
            &request.bounded_wallet,
            &request.bounty_contract,
            &request.caller,
        ),
        "cancelled" => planner.plan_bounded_wallet_refund(
            &request.bounded_wallet,
            &request.bounty_contract,
            &request.caller,
        ),
        _ => return Err(StatusCode::CONFLICT),
    };
    plan.map(Json).map_err(|_| StatusCode::BAD_REQUEST)
}

fn autonomous_item_mode(item: &AutonomousBountyFeedItem) -> Result<&str, StatusCode> {
    item.terms
        .as_ref()
        .and_then(|terms| terms.document.verification_policy.get("mechanism"))
        .and_then(serde_json::Value::as_str)
        .ok_or(StatusCode::CONFLICT)
}

fn require_autonomous_item_mode(
    item: &AutonomousBountyFeedItem,
    expected: &str,
) -> Result<(), StatusCode> {
    if item.terms_valid && item.status == "submitted" && autonomous_item_mode(item)? == expected {
        Ok(())
    } else {
        Err(StatusCode::CONFLICT)
    }
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/decode-events", responses((status = 200, description = "Decoded autonomous factory, funding, claim, submission, settlement, and refund evidence")))]
async fn decode_autonomous_bounty_events(
    Json(request): Json<DecodeAutonomousBountyEventsRequest>,
) -> Result<Json<Vec<AutonomousBountyEvent>>, StatusCode> {
    decode_autonomous_bounty_logs(request.logs)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/events", responses((status = 200, description = "Persisted confirmed autonomous bounty events")))]
async fn list_autonomous_bounty_events(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousBountyEventsQuery>,
) -> Result<Json<Vec<AutonomousBountyEvent>>, StatusCode> {
    let Some(store) = &state.store else {
        return Ok(Json(Vec::new()));
    };
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let mut events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(bounty_id) = query.bounty_id {
        events.retain(|event| event.bounty_id.eq_ignore_ascii_case(&bounty_id));
    }
    Ok(Json(events))
}

fn autonomous_terms_record(
    request: PublishAutonomousBountyTermsRequest,
) -> Result<AutonomousBountyTermsRecord, AgentActionApiError> {
    build_autonomous_bounty_terms_record(&request.creator_wallet, request.document, Utc::now())
        .map_err(|error| {
            let (status, code) = match &error {
                ChainBaseError::TermsDocumentTooLarge => {
                    (StatusCode::PAYLOAD_TOO_LARGE, "terms_document_too_large")
                }
                _ => (StatusCode::BAD_REQUEST, "invalid_autonomous_bounty_terms"),
            };
            agent_action_error(
                status,
                code,
                error.to_string(),
                false,
                "Correct the terms document and publish it before creating or funding a bounty. The returned terms hash must be committed on-chain unchanged.",
            )
        })
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/terms", responses((status = 200, description = "Content-addressed public bounty terms and contract hash commitments")))]
async fn publish_autonomous_bounty_terms(
    State(state): State<SharedState>,
    Json(request): Json<PublishAutonomousBountyTermsRequest>,
) -> Result<Json<AutonomousBountyTermsRecord>, AgentActionApiError> {
    let record = autonomous_terms_record(request)?;
    let store = state.store.as_ref().ok_or_else(|| {
        agent_action_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "terms_store_unavailable",
            "DATABASE_URL is required to publish public bounty terms",
            true,
            "Retry after the hosted terms store is healthy; do not create the bounty until publication succeeds.",
        )
    })?;
    store
        .upsert_autonomous_bounty_terms(&record)
        .await
        .map_err(|error| {
            agent_action_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "terms_store_write_failed",
                error.to_string(),
                true,
                "Retry publication with the identical document. Do not alter the document or create the bounty until the terms hash can be read back.",
            )
        })?;
    Ok(Json(record))
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/terms/{terms_hash}", params(("terms_hash" = String, Path, description = "0x-prefixed Keccak hash returned by terms publication and committed on-chain")), responses((status = 200, description = "Canonical public bounty terms"), (status = 404, description = "Unknown terms hash")))]
async fn get_autonomous_bounty_terms(
    State(state): State<SharedState>,
    Path(terms_hash): Path<String>,
) -> Result<Json<AutonomousBountyTermsRecord>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .get_autonomous_bounty_terms(&terms_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/submission-evidence", responses((status = 200, description = "Immutable public preimages matching the current canonical SubmissionAdded hashes")))]
async fn publish_autonomous_submission_evidence(
    State(state): State<SharedState>,
    Json(request): Json<PublishAutonomousSubmissionEvidenceRequest>,
) -> Result<Json<AutonomousSubmissionEvidenceRecord>, StatusCode> {
    let network = request
        .network
        .clone()
        .unwrap_or_else(|| "base-mainnet".to_string());
    let item = indexed_autonomous_bounty(&state, &network, &request.bounty_contract).await?;
    let record = autonomous_submission_evidence_record(&network, &item, request)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .upsert_autonomous_submission_evidence(&record)
        .await
        .map(Json)
        .map_err(|error| match error {
            DbError::AutonomousEvidenceConflict(_) => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/submission-evidence/{bounty_contract}/{round}", params(("bounty_contract" = String, Path, description = "Canonical bounty contract"), ("round" = u64, Path, description = "Positive submission round")), responses((status = 200, description = "Hash-checked public submission evidence"), (status = 404, description = "Evidence not published")))]
async fn get_autonomous_submission_evidence(
    State(state): State<SharedState>,
    Path((bounty_contract, round)): Path<(String, u64)>,
    Query(query): Query<AutonomousSubmissionEvidenceQuery>,
) -> Result<Json<AutonomousSubmissionEvidenceRecord>, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    indexed_autonomous_bounty(&state, network, &bounty_contract).await?;
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .get_autonomous_submission_evidence(network, &bounty_contract, round)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

fn autonomous_submission_evidence_record(
    network: &str,
    item: &AutonomousBountyFeedItem,
    request: PublishAutonomousSubmissionEvidenceRequest,
) -> Result<AutonomousSubmissionEvidenceRecord, StatusCode> {
    build_autonomous_submission_evidence_record(
        network,
        item,
        &request.bounty_contract,
        &request.bounty_id,
        request.round,
        &request.solver_wallet,
        &request.artifact_reference,
        request.evidence,
        Utc::now(),
    )
    .map_err(|_| StatusCode::CONFLICT)
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/feed", responses((status = 200, description = "Canonical on-chain bounties joined to content-addressed public terms")))]
async fn autonomous_bounty_feed(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousBountyFeedQuery>,
) -> Result<Json<Vec<AutonomousBountyFeedItem>>, StatusCode> {
    load_autonomous_bounty_feed(
        &state,
        query.network.as_deref().unwrap_or("base-mainnet"),
        query.claimable_only.unwrap_or(false),
    )
    .await
    .map(Json)
}

async fn load_autonomous_bounty_feed(
    state: &SharedState,
    network: &str,
    claimable_only: bool,
) -> Result<Vec<AutonomousBountyFeedItem>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.recovery_reservations.apply(&mut feed, claimable_only);
    Ok(feed)
}

async fn load_verified_autonomous_bounty_feed(
    state: &SharedState,
    network: &str,
    claimable_only: bool,
) -> Result<Vec<AutonomousBountyFeedItem>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let events = store
        .list_verified_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.recovery_reservations.apply(&mut feed, claimable_only);
    Ok(feed)
}

#[utoipa::path(
    get,
    path = "/v1/base/autonomous-bounties/leaderboard",
    params(
        ("network" = Option<String>, Query, description = "Base network; defaults to base-mainnet"),
        ("at" = Option<String>, Query, description = "RFC3339 instant selecting the UTC day and Monday-to-Sunday week")
    ),
    responses((status = 200, body = SolverLeaderboardResponse))
)]
async fn solver_leaderboard(
    State(state): State<SharedState>,
    Query(query): Query<SolverLeaderboardQuery>,
) -> Result<Json<SolverLeaderboardResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let network_descriptor =
        base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let reference_at = query
        .at
        .as_deref()
        .map(DateTime::parse_from_rfc3339)
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let daily_period = leaderboard_period(LeaderboardPeriodKind::Daily, reference_at);
    let weekly_period = leaderboard_period(LeaderboardPeriodKind::Weekly, reference_at);
    let mut completions = store
        .list_canonical_solver_completions(network, weekly_period.starts_at, weekly_period.ends_at)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    completions.retain(|completion| {
        !state
            .recovery_reservations
            .contains(&completion.bounty_contract)
    });
    let daily_ranking = rank_solver_completions(daily_period, completions.clone());
    let weekly_ranking = rank_solver_completions(weekly_period, completions);
    let reward_contract_env = match network_descriptor.chain_id {
        8_453 => "BASE_MAINNET_LEADERBOARD_REWARD_CONTRACT",
        84_532 => "BASE_SEPOLIA_LEADERBOARD_REWARD_CONTRACT",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let reward_contract = env::var(reward_contract_env)
        .ok()
        .and_then(non_empty_secret)
        .and_then(|value| normalize_evm_address(&value).ok());
    let mut reward_pool = match reward_contract.as_deref() {
        None => SolverLeaderboardRewardPoolResponse {
            contract: None,
            settlement_token: network_descriptor.native_usdc_token_address.clone(),
            funding_status: "not_configured".to_string(),
            balance_usdc_base_units: None,
            balance_usdc: None,
            current_daily_and_weekly_required_usdc: "29.00".to_string(),
            maximum_full_weeks_at_current_balance: None,
            observed_safe_block: None,
            observed_safe_block_hash: None,
            observation_error: None,
            evidence_boundary: "No reward contract is configured. Rankings remain informational and no prize is represented as funded.".to_string(),
        },
        Some(contract) => match state.base_rpc_urls.resolve(network) {
            Err(_) => SolverLeaderboardRewardPoolResponse {
                contract: Some(contract.to_string()),
                settlement_token: network_descriptor.native_usdc_token_address.clone(),
                funding_status: "unverified".to_string(),
                balance_usdc_base_units: None,
                balance_usdc: None,
                current_daily_and_weekly_required_usdc: "29.00".to_string(),
                maximum_full_weeks_at_current_balance: None,
                observed_safe_block: None,
                observed_safe_block_hash: None,
                observation_error: Some("Base RPC is not configured.".to_string()),
                evidence_boundary: "The reward address is configured, but its native USDC balance was not verified at a Base safe block.".to_string(),
            },
            Ok((descriptor, rpc_url)) => match observe_erc20_balance_safe(
                &rpc_url,
                &descriptor.native_usdc_token_address,
                contract,
                90_000,
            )
            .await
            {
                Err(_) => SolverLeaderboardRewardPoolResponse {
                    contract: Some(contract.to_string()),
                    settlement_token: descriptor.native_usdc_token_address,
                    funding_status: "unverified".to_string(),
                    balance_usdc_base_units: None,
                    balance_usdc: None,
                    current_daily_and_weekly_required_usdc: "29.00".to_string(),
                    maximum_full_weeks_at_current_balance: None,
                    observed_safe_block: None,
                    observed_safe_block_hash: None,
                    observation_error: Some("Base safe-block balance read failed.".to_string()),
                    evidence_boundary: "The reward address is configured, but its native USDC balance was not verified at a Base safe block.".to_string(),
                },
                Ok(observation) => {
                    let funding_status = if observation.balance >= 29_000_000 {
                        "funded"
                    } else if observation.balance > 0 {
                        "partially_funded"
                    } else {
                        "unfunded"
                    };
                    SolverLeaderboardRewardPoolResponse {
                        contract: Some(observation.account),
                        settlement_token: observation.token,
                        funding_status: funding_status.to_string(),
                        balance_usdc_base_units: Some(observation.balance.to_string()),
                        balance_usdc: Some(format_usdc_base_units(observation.balance)),
                        current_daily_and_weekly_required_usdc: "29.00".to_string(),
                        maximum_full_weeks_at_current_balance: Some(
                            u64::try_from(observation.balance / 47_000_000)
                                .unwrap_or(u64::MAX),
                        ),
                        observed_safe_block: Some(observation.safe_block_number),
                        observed_safe_block_hash: Some(observation.safe_block_hash),
                        observation_error: None,
                        evidence_boundary: "This is the reward contract's native USDC balance at one Base safe block. Funded status means the balance covers the shown 3 USDC daily and 26 USDC weekly prizes if no earlier award consumes it first. Only the paid-winner record at a Base safe block proves prize payment.".to_string(),
                    }
                }
            },
        },
    };
    let (daily_payout_observation, weekly_payout_observation) = match (
        reward_contract.as_deref(),
        state.base_rpc_urls.resolve(network),
    ) {
        (Some(contract), Ok((_, rpc_url))) => (
            observe_leaderboard_payout(&rpc_url, contract, &daily_ranking, 90_100)
                .await
                .ok(),
            observe_leaderboard_payout(&rpc_url, contract, &weekly_ranking, 90_200)
                .await
                .ok(),
        ),
        _ => (None, None),
    };
    if reward_contract.is_some()
        && (daily_payout_observation.is_none() || weekly_payout_observation.is_none())
    {
        reward_pool.funding_status = "unverified".to_string();
        reward_pool.observation_error = Some(
            "The configured address did not return both paid-winner records at a Base safe block."
                .to_string(),
        );
        reward_pool.evidence_boundary = "USDC balance alone does not prove a valid reward contract. Rankings remain informational until the balance and both paid-winner getters are verified at Base safe blocks.".to_string();
    }
    let reward_funding_status = reward_pool.funding_status.clone();
    let generated_at = Utc::now();

    Ok(Json(SolverLeaderboardResponse {
        schema_version: "agent-bounties/solver-leaderboard-v1".to_string(),
        network: network.to_string(),
        generated_at,
        reference_at,
        reward_pool,
        daily: leaderboard_period_response(
            daily_ranking,
            generated_at,
            reward_contract.clone(),
            &reward_funding_status,
            daily_payout_observation,
        ),
        weekly: leaderboard_period_response(
            weekly_ranking,
            generated_at,
            reward_contract,
            &reward_funding_status,
            weekly_payout_observation,
        ),
        next_action: "Claim a funded bounty worth at least 2 USDC, complete it, and confirm BountySettled before the period ends.".to_string(),
        evidence_boundary: "Rankings count indexed canonical settlements with verified Base block time. A configured or funded reward is not payment. Only a confirmed reward transfer proves prize payment.".to_string(),
    }))
}

async fn observe_leaderboard_payout(
    rpc_url: &str,
    contract: &str,
    ranking: &SolverLeaderboardRanking,
    request_id: u64,
) -> Result<SolverLeaderboardAwardSafeObservation, ChainBaseError> {
    let period_kind = match ranking.period.kind {
        LeaderboardPeriodKind::Daily => 0,
        LeaderboardPeriodKind::Weekly => 1,
    };
    let starts_at = u64::try_from(ranking.period.starts_at.timestamp()).map_err(|_| {
        ChainBaseError::InvalidVerificationConfiguration(
            "leaderboard period starts before Unix epoch".to_string(),
        )
    })?;
    let award_id = solver_leaderboard_award_id(period_kind, starts_at)?;
    observe_solver_leaderboard_paid_winner_safe(rpc_url, contract, &award_id, request_id).await
}

fn leaderboard_period_response(
    ranking: SolverLeaderboardRanking,
    now: DateTime<Utc>,
    reward_contract: Option<String>,
    reward_funding_status: &str,
    payout_observation: Option<SolverLeaderboardAwardSafeObservation>,
) -> SolverLeaderboardPeriodResponse {
    let closed = now >= ranking.period.ends_at;
    let has_winner = ranking.leader_wallet.is_some();
    let paid_wallet = payout_observation
        .as_ref()
        .and_then(|observation| observation.paid_winner.clone());
    let payout_status = if !closed {
        "not_due"
    } else if !has_winner {
        "no_winner"
    } else if let Some(paid) = paid_wallet.as_deref() {
        if ranking
            .leader_wallet
            .as_deref()
            .is_some_and(|leader| leader.eq_ignore_ascii_case(paid))
        {
            "paid"
        } else {
            "paid_to_different_wallet"
        }
    } else if reward_contract.is_none() {
        "reward_not_configured"
    } else if payout_observation.is_none() {
        "payout_unverified"
    } else if reward_funding_status != "funded" {
        "awaiting_verified_funding"
    } else {
        "awaiting_finalization"
    };
    SolverLeaderboardPeriodResponse {
        period_status: if closed { "closed" } else { "open" }.to_string(),
        reward_usdc: format_usdc_base_units(ranking.period.kind.reward_usdc_base_units().into()),
        reward_funding_status: reward_funding_status.to_string(),
        reward_payout_status: payout_status.to_string(),
        reward_contract,
        reward_paid_wallet: paid_wallet,
        reward_payout_observed_safe_block: payout_observation
            .as_ref()
            .map(|observation| observation.safe_block_number),
        reward_payout_observed_safe_block_hash: payout_observation
            .map(|observation| observation.safe_block_hash),
        ranking,
    }
}

#[utoipa::path(
    get,
    path = "/v1/base/autonomous-bounties/inventory-summary",
    responses((status = 200, body = AutonomousBountyInventorySummary))
)]
async fn autonomous_bounty_inventory_summary(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousBountyFeedQuery>,
) -> Result<Json<AutonomousBountyInventorySummary>, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let feed =
        load_autonomous_bounty_feed(&state, network, query.claimable_only.unwrap_or(true)).await?;
    build_autonomous_inventory_summary(&state, network, feed).map(Json)
}

#[utoipa::path(
    get,
    path = "/v1/base/autonomous-bounties/inventory-badge.svg",
    responses((status = 200, description = "Live canonical claimable inventory badge"))
)]
async fn autonomous_bounty_inventory_badge(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousBountyFeedQuery>,
) -> Result<Response, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let feed = load_autonomous_bounty_feed(&state, network, true).await?;
    let summary = build_autonomous_inventory_summary(&state, network, feed)?;
    let message = format!(
        "{} claimable | {} USDC",
        summary.claimable_bounty_count, summary.funded_usdc
    );
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="290" height="20" role="img" aria-label="Agent Bounties: {message}"><title>Agent Bounties: {message}</title><linearGradient id="s" x2="0" y2="100%"><stop offset="0" stop-color="#fff" stop-opacity=".16"/><stop offset="1" stop-opacity=".08"/></linearGradient><clipPath id="r"><rect width="290" height="20" rx="3" fill="#fff"/></clipPath><g clip-path="url(#r)"><rect width="110" height="20" fill="#20262e"/><rect x="110" width="180" height="20" fill="#087f5b"/><rect width="290" height="20" fill="url(#s)"/></g><g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11"><text x="55" y="15" fill="#010101" fill-opacity=".3">inventory</text><text x="55" y="14">inventory</text><text x="200" y="15" fill="#010101" fill-opacity=".3">{message}</text><text x="200" y="14">{message}</text></g></svg>"##
    );
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (
                header::CACHE_CONTROL,
                "public, max-age=15, stale-while-revalidate=45",
            ),
        ],
        svg,
    )
        .into_response())
}

fn build_autonomous_inventory_summary(
    state: &SharedState,
    network: &str,
    feed: Vec<AutonomousBountyFeedItem>,
) -> Result<AutonomousBountyInventorySummary, StatusCode> {
    let sum = |field: fn(&AutonomousBountyFeedItem) -> &str| {
        feed.iter().try_fold(0_u128, |total, item| {
            field(item)
                .parse::<u128>()
                .ok()
                .and_then(|amount| total.checked_add(amount))
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
        })
    };
    let funded = sum(|item| &item.funded_amount)?;
    let solver = sum(|item| &item.solver_reward)?;
    let verifier = sum(|item| &item.verifier_reward)?;
    let verification_ready_bounty_count =
        feed.iter().filter(|item| item.verification_ready).count();
    let standing_meta_bounty_count = feed
        .iter()
        .filter(|item| standing_meta_v2_parent_context(item).is_ok())
        .count();
    let items = feed
        .iter()
        .map(|item| AutonomousBountyInventoryItem {
            bounty_id: item.bounty_id.clone(),
            bounty_contract: item.bounty_contract.clone(),
            title: item
                .terms
                .as_ref()
                .map(|terms| terms.document.title.clone()),
            status: item.status.clone(),
            funded_usdc_base_units: item.funded_amount.clone(),
            solver_reward_usdc_base_units: item.solver_reward.clone(),
            verifier_reward_usdc_base_units: item.verifier_reward.clone(),
            verification_ready: item.verification_ready,
            standing_meta_bounty: standing_meta_v2_parent_context(item).is_ok(),
        })
        .collect();
    Ok(AutonomousBountyInventorySummary {
        schema_version: "agent-bounties/inventory-summary-v1".to_string(),
        network: network.to_string(),
        generated_at: Utc::now().to_rfc3339(),
        canonical_source: format!(
            "{}/v1/base/autonomous-bounties/feed?network={network}&claimable_only=true",
            state.public_base_url.trim_end_matches('/')
        ),
        claimable_bounty_count: feed.len(),
        verification_ready_bounty_count,
        standing_meta_bounty_count,
        funded_usdc_base_units: funded.to_string(),
        funded_usdc: format_usdc_base_units(funded),
        solver_reward_usdc_base_units: solver.to_string(),
        solver_reward_usdc: format_usdc_base_units(solver),
        verifier_reward_usdc_base_units: verifier.to_string(),
        verifier_reward_usdc: format_usdc_base_units(verifier),
        items,
        evidence_boundary: "This summary is derived at request time from confirmed canonical events and validated content-addressed terms in the hosted index. It proves current indexed inventory, not a future claim, completion, or payout. Only BountySettled proves payment.".to_string(),
    })
}

async fn build_open_competition_inventory_summary(
    state: &SharedState,
    network: &str,
) -> Result<OpenCompetitionInventorySummary, StatusCode> {
    let (descriptor, _) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let release = open_competition_release_from_environment(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if release.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let prefix = open_competition_environment_prefix(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let public_activation_block = env::var(format!("{prefix}_PUBLIC_ACTIVATION_BLOCK"))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .parse::<u64>()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let catalog = open_competition_verifier_catalog_from_environment(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let [profile] = catalog.profiles.as_slice() else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !profile.public_inventory_eligible
        || profile.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let events = store
        .list_verified_open_competition_events(network, &release.factory_contract)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let generated_at = Utc::now();
    let website_base_url =
        legal_website_base_url(env::var("WEBSITE_BASE_URL").ok(), &state.public_base_url);
    let items = open_competition_discovery_items(
        &events,
        profile,
        network,
        descriptor.chain_id,
        &state.public_base_url,
        &website_base_url,
        public_activation_block,
        generated_at,
    )
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let ready = items.iter().filter(|item| item.ready_to_earn);
    let sum = |field: fn(&github_discovery::GitHubDiscoveryItem) -> &str| {
        ready.clone().try_fold(0_u128, |total, item| {
            field(item)
                .parse::<u128>()
                .ok()
                .and_then(|amount| total.checked_add(amount))
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
        })
    };
    Ok(OpenCompetitionInventorySummary {
        generated_at: generated_at.to_rfc3339(),
        ready_to_earn_count: items.iter().filter(|item| item.ready_to_earn).count(),
        funded_usdc_base_units: sum(|item| &item.funded_usdc_base_units)?.to_string(),
        solver_reward_usdc_base_units: sum(|item| &item.reward_usdc_base_units)?.to_string(),
        verifier_reward_usdc_base_units: sum(|item| &item.verifier_reward_usdc_base_units)?
            .to_string(),
    })
}

async fn build_open_competition_v2_inventory_summary(
    state: &SharedState,
    network: &str,
) -> Result<OpenCompetitionInventorySummary, StatusCode> {
    let generated_at = Utc::now();
    let items = load_public_open_competition_v2_opportunities(
        state,
        network,
        state.public_base_url.trim_end_matches('/'),
        generated_at,
    )
    .await?;
    let ready = items
        .iter()
        .filter(|item| {
            item.work_state == "claimable"
                && item.payment_state == "escrowed"
                && item.payment_committed
                && item.verification_ready
                && item
                    .cash_economics
                    .as_ref()
                    .is_some_and(|economics| economics.gross_cash_margin_positive)
        })
        .collect::<Vec<_>>();
    let sum = |field: fn(&OpportunityItem) -> &str| {
        ready.iter().try_fold(0_u128, |total, item| {
            field(item)
                .parse::<u128>()
                .ok()
                .and_then(|amount| total.checked_add(amount))
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
        })
    };
    Ok(OpenCompetitionInventorySummary {
        generated_at: generated_at.to_rfc3339(),
        ready_to_earn_count: ready.len(),
        funded_usdc_base_units: sum(|item| &item.funded_amount.amount)?.to_string(),
        solver_reward_usdc_base_units: sum(|item| &item.reward.amount)?.to_string(),
        // Open Competition V2 reserves a keeper reward rather than a verifier
        // reward. Keep this existing role-specific public field truthful while
        // the full escrow remains included in available_funding_usdc.
        verifier_reward_usdc_base_units: "0".to_string(),
    })
}

fn format_usdc_base_units(amount: u128) -> String {
    format!(
        "{}.{:02}",
        amount / 1_000_000,
        (amount % 1_000_000) / 10_000
    )
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/verification-jobs", responses((status = 200, description = "Live verifier jobs joined to immutable terms and hash-matched evidence preimages")))]
async fn autonomous_verification_jobs(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousVerificationJobsQuery>,
) -> Result<Json<Vec<AutonomousVerificationJob>>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let evidence = store
        .list_autonomous_submission_evidence(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .recovery_reservations
        .exclude_from_verification_jobs(&mut feed);
    let observed_at = u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut jobs = build_autonomous_verification_jobs(network, feed, evidence, observed_at)
        .map_err(|_| StatusCode::CONFLICT)?;
    if let Some(verifier) = query.verifier {
        let verifier = normalize_evm_address(verifier).map_err(|_| StatusCode::BAD_REQUEST)?;
        jobs.retain(|job| {
            job.verification_mode == "deterministic_module"
                || job
                    .eligible_verifiers
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&verifier))
        });
    }
    Ok(Json(jobs))
}

#[utoipa::path(
    post,
    path = "/v1/stripe/checkout-top-ups",
    request_body = PlanStripeCheckoutTopUpRequest,
    responses(
        (status = 200, description = "Stripe Checkout Session request intent"),
        (status = 400, description = "Invalid top-up request or amount below Stripe minimum")
    )
)]
async fn plan_stripe_checkout_top_up(
    State(state): State<SharedState>,
    Json(request): Json<PlanStripeCheckoutTopUpRequest>,
) -> Result<Json<payments_stripe::StripeRequestIntent>, StatusCode> {
    stripe_checkout_top_up_intent(&state, request).map(Json)
}

fn stripe_checkout_top_up_intent(
    state: &SharedState,
    request: PlanStripeCheckoutTopUpRequest,
) -> Result<StripeRequestIntent, StatusCode> {
    let PlanStripeCheckoutTopUpRequest {
        organization_id,
        amount_minor,
        currency,
        success_url,
        cancel_url,
    } = request;
    let platform_base_url = state.public_base_url.clone();
    let planner = stripe_planner_for_state(state, platform_base_url.clone());
    let amount = Money::new(amount_minor, currency).map_err(|_| StatusCode::BAD_REQUEST)?;
    planner
        .checkout_top_up(&CheckoutTopUpRequest {
            organization_id,
            amount,
            success_url: success_url
                .unwrap_or_else(|| format!("{platform_base_url}/stripe/success")),
            cancel_url: cancel_url.unwrap_or_else(|| format!("{platform_base_url}/stripe/cancel")),
        })
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn stripe_planner_for_state(
    state: &SharedState,
    platform_base_url: impl Into<String>,
) -> StripePlanner {
    let planner = StripePlanner::new(platform_base_url);
    if let Some(payment_method_configuration) = state.stripe_payment_method_configuration.as_deref()
    {
        planner.with_payment_method_configuration(payment_method_configuration)
    } else {
        planner
    }
}

fn apply_state_checkout_payment_method_configuration(
    state: &SharedState,
    intent: &mut StripeRequestIntent,
) -> Result<(), StatusCode> {
    apply_checkout_payment_method_configuration(
        intent,
        state.stripe_payment_method_configuration.as_deref(),
    )
    .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/connect-accounts",
    request_body = PlanStripeConnectAccountRequest,
    responses(
        (status = 200, description = "Stripe Accounts v2 create request intent"),
        (status = 400, description = "Invalid Connect account planning request")
    )
)]
async fn plan_stripe_connect_account(
    Json(request): Json<PlanStripeConnectAccountRequest>,
) -> Result<Json<payments_stripe::ConnectAccountV2CreateIntent>, StatusCode> {
    stripe_connect_account_intent(request)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn stripe_connect_account_intent(
    request: PlanStripeConnectAccountRequest,
) -> Result<payments_stripe::ConnectAccountV2CreateIntent, payments_stripe::StripeIntegrationError>
{
    StripePlanner::new("http://127.0.0.1:8080").connect_account_v2(request.agent_id)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/connect-transfers",
    request_body = PlanStripeConnectTransferRequest,
    responses(
        (status = 200, description = "Stripe Connect transfer request intent for a fiat payout"),
        (status = 400, description = "Invalid payout intent or transfer planning request")
    )
)]
async fn plan_stripe_connect_transfer(
    State(state): State<SharedState>,
    Json(request): Json<PlanStripeConnectTransferRequest>,
) -> Result<Json<StripeTransferPlan>, StatusCode> {
    stripe_connect_transfer_plan(&state, request).map(Json)
}

fn stripe_connect_transfer_plan(
    state: &SharedState,
    request: PlanStripeConnectTransferRequest,
) -> Result<StripeTransferPlan, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    network
        .plan_stripe_transfer(
            AppPlanStripeTransferRequest {
                payout_intent_id: request.payout_intent_id,
                connected_account_id: request.connected_account_id,
            },
            state.public_base_url.clone(),
        )
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/checkout-top-ups",
    request_body = PlanStripeCheckoutTopUpRequest,
    responses(
        (status = 200, description = "Live Stripe Checkout Session execution report"),
        (status = 400, description = "Invalid top-up request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Live Stripe execution is disabled or not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn execute_stripe_checkout_top_up(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanStripeCheckoutTopUpRequest>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let intent = stripe_checkout_top_up_intent(&state, request)?;
    execute_stripe_intent(&state, intent).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/funding-intents/{id}/checkout-session",
    params(("id" = Uuid, Path, description = "Stripe fiat funding intent id")),
    responses(
        (status = 200, description = "Live Stripe Checkout Session execution report for a bounty funding intent"),
        (status = 400, description = "Unknown, non-Stripe, already-applied, or invalid funding intent"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Public Stripe Checkout execution is disabled or live Stripe execution is not configured")
    )
)]
async fn execute_stripe_funding_intent_checkout(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    if !state.stripe_public_checkout_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let intent = {
        let network = state.network.lock().expect("state poisoned");
        network.stripe_checkout_for_funding_intent(id, state.public_base_url.clone())
    }
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut intent = intent;
    apply_state_checkout_payment_method_configuration(&state, &mut intent)?;
    execute_stripe_intent(&state, intent).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/connect-accounts",
    request_body = PlanStripeConnectAccountRequest,
    responses(
        (status = 200, description = "Live Stripe Accounts v2 execution report"),
        (status = 400, description = "Invalid Connect request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Live Stripe execution is disabled or not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn execute_stripe_connect_account(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanStripeConnectAccountRequest>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let intent = stripe_connect_account_intent(request)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .request;
    execute_stripe_intent(&state, intent).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/connect-transfers",
    request_body = PlanStripeConnectTransferRequest,
    responses(
        (status = 200, description = "Live Stripe Connect transfer execution report"),
        (status = 400, description = "Invalid transfer request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Live Stripe execution is disabled or not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn execute_stripe_connect_transfer(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanStripeConnectTransferRequest>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let plan = stripe_connect_transfer_plan(&state, request)?;
    execute_stripe_intent(&state, plan.request).await.map(Json)
}

async fn execute_stripe_intent(
    state: &SharedState,
    intent: StripeRequestIntent,
) -> Result<StripeExecutionReport, StatusCode> {
    if !state.stripe_live_execution_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let secret_key = state
        .stripe_secret_key
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    execute_stripe_request(&intent, secret_key, &state.stripe_api_base_url)
        .await
        .map_err(stripe_execution_status)
}

fn stripe_execution_status(error: payments_stripe::StripeIntegrationError) -> StatusCode {
    match error {
        payments_stripe::StripeIntegrationError::RequestFailed { .. }
        | payments_stripe::StripeIntegrationError::HttpTransport(_) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::BAD_REQUEST,
    }
}

#[utoipa::path(
    post,
    path = "/v1/stripe/connect-snapshots",
    responses(
        (status = 200, description = "Reconciled Stripe Connect payout eligibility snapshot"),
        (status = 400, description = "Invalid Connect snapshot"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn reconcile_stripe_connect_snapshot(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(snapshot): Json<ConnectAccountSnapshot>,
) -> Result<Json<app::StripeConnectPayoutReconciliation>, StatusCode> {
    require_operator(&state, &headers)?;
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .apply_stripe_connect_snapshot(snapshot)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        for bounty in &reconciliation.bounties {
            store
                .upsert_bounty(bounty)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for settlement in &reconciliation.settlements {
            store
                .upsert_settlement(settlement)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &reconciliation.ledger_entries).await?;
    }
    Ok(Json(reconciliation))
}

#[utoipa::path(
    post,
    path = "/v1/stripe/transfer-events",
    responses(
        (status = 200, description = "Reconciled Stripe transfer event as fiat payout evidence"),
        (status = 400, description = "Invalid transfer event payload or signature"),
        (status = 503, description = "Webhook signature verification is not configured")
    )
)]
async fn reconcile_stripe_transfer_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<StripeTransferReconciliation>, StatusCode> {
    match &state.stripe_webhook_secret {
        Some(secret) => {
            let signature = headers
                .get("stripe-signature")
                .and_then(|value| value.to_str().ok())
                .ok_or(StatusCode::BAD_REQUEST)?;
            verify_webhook_signature(&body, signature, secret)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
        }
        None if state.allow_unsigned_stripe_webhooks => {}
        None => return Err(StatusCode::SERVICE_UNAVAILABLE),
    }
    let event: StripeWebhookEvent =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let evidence = StripeEventDeduper::default()
        .apply_connect_transfer(&event)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .apply_stripe_transfer_evidence(evidence)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };

    if let Some(store) = &state.store {
        if !reconciliation.duplicate {
            store
                .upsert_payment_event(&reconciliation.evidence.payment_event)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(settlement) = &reconciliation.settlement {
            store
                .upsert_settlement(settlement)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(bounty) = &reconciliation.bounty {
            store
                .upsert_bounty(bounty)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &reconciliation.ledger_entries).await?;
    }
    Ok(Json(reconciliation))
}

#[utoipa::path(
    post,
    path = "/v1/stripe/checkout-webhooks",
    responses(
        (status = 200, description = "Reconciled paid Stripe Checkout top-up webhook"),
        (status = 400, description = "Invalid webhook payload or signature"),
        (status = 503, description = "Webhook signature verification is not configured")
    )
)]
async fn reconcile_stripe_checkout_webhook(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<app::StripeFundingReconciliation>, StatusCode> {
    match &state.stripe_webhook_secret {
        Some(secret) => {
            let signature = headers
                .get("stripe-signature")
                .and_then(|value| value.to_str().ok())
                .ok_or(StatusCode::BAD_REQUEST)?;
            verify_webhook_signature(&body, signature, secret)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
        }
        None if state.allow_unsigned_stripe_webhooks => {}
        None => return Err(StatusCode::SERVICE_UNAVAILABLE),
    }
    let event: StripeWebhookEvent =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let funding_credit = StripeEventDeduper::default()
        .apply_checkout_top_up(&event)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .apply_stripe_funding_credit(funding_credit)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };

    if let Some(store) = &state.store {
        if !reconciliation.duplicate {
            store
                .upsert_payment_event(&reconciliation.funding_credit.payment_event)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(intent) = &reconciliation.funding_intent {
            store
                .upsert_funding_intent(intent)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(report) = &reconciliation.funding_report {
            store
                .upsert_bounty(&report.bounty)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            store
                .upsert_funding_contribution(&report.contribution)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &reconciliation.ledger_entries).await?;
    }

    Ok(Json(reconciliation))
}

#[utoipa::path(
    post,
    path = "/v1/github/issue-bounty-plan",
    request_body = PlanGitHubIssueBountyRequest,
    responses((status = 200, description = "GitHub issue-form bounty parse and check-run plan"))
)]
async fn plan_github_issue_bounty(
    Json(request): Json<PlanGitHubIssueBountyRequest>,
) -> Json<GitHubIssueBountyPlan> {
    Json(github_issue_bounty_plan(request))
}

fn github_issue_bounty_plan(request: PlanGitHubIssueBountyRequest) -> GitHubIssueBountyPlan {
    let parsed = parse_issue_form_bounty(
        &request.repository,
        &request.issue_url,
        &request.title,
        &request.body,
    );
    match parsed {
        Ok(bounty) => GitHubIssueBountyPlan {
            ready: true,
            check: bounty_check_output(Ok(&bounty)),
            parsed: Some(bounty),
            error: None,
        },
        Err(error) => GitHubIssueBountyPlan {
            ready: false,
            check: bounty_check_output(Err(&error)),
            parsed: None,
            error: Some(error.to_string()),
        },
    }
}

#[utoipa::path(
    post,
    path = "/v1/github/issue-api-sync-plan",
    request_body = PlanGitHubIssueApiSyncRequest,
    responses((status = 200, description = "GitHub issue to hosted API bounty sync plan"))
)]
async fn plan_github_issue_api_sync(
    Json(request): Json<PlanGitHubIssueApiSyncRequest>,
) -> Json<GitHubIssueApiSyncPlan> {
    Json(issue_api_sync_plan(GitHubIssueApiSyncInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        api_base_url: request.api_base_url,
        existing_bounty_ids: request.existing_bounty_ids,
        hosted_api_error: request.hosted_api_error,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/github/issue-api-sync",
    request_body = PlanGitHubIssueApiSyncRequest,
    responses(
        (status = 200, description = "GitHub issue synced into a hosted bounty record"),
        (status = 400, description = "Issue is invalid or hosted API state could not be planned"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn sync_github_issue_api_bounty(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanGitHubIssueApiSyncRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    require_operator(&state, &headers)?;
    let plan = issue_api_sync_plan(GitHubIssueApiSyncInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        api_base_url: request.api_base_url,
        existing_bounty_ids: request.existing_bounty_ids,
        hosted_api_error: request.hosted_api_error,
    });
    if !plan.ready {
        return Err(StatusCode::BAD_REQUEST);
    }
    let parsed = plan.parsed.ok_or(StatusCode::BAD_REQUEST)?;
    let bounty_id = plan.bounty_id.ok_or(StatusCode::BAD_REQUEST)?;
    let idempotency_key = plan.idempotency_key.ok_or(StatusCode::BAD_REQUEST)?;
    let request = OpenPooledBountyRequest {
        bounty_id: None,
        idempotency_key: None,
        title: parsed.request.title,
        template_slug: parsed.template_slug,
        target_amount_minor: parsed.amount.amount,
        currency: parsed.amount.currency,
        funding_mode: parsed.funding_mode,
        privacy: parsed.privacy,
        funding_targets: vec![],
    };

    let bounty = if let Some(store) = &state.store {
        let candidate = {
            let mut network = state.network.lock().expect("state poisoned");
            network.build_github_issue_pooled_bounty(request, bounty_id, idempotency_key)
        };
        let candidate = match candidate {
            Ok(bounty) => bounty,
            Err(_) => {
                persist_all_risk_events(&state).await?;
                return Err(StatusCode::BAD_REQUEST);
            }
        };
        match store
            .upsert_github_issue_sync_bounty(&candidate)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            GitHubIssueSyncBountyUpsert::Upserted(bounty) => {
                state
                    .network
                    .lock()
                    .expect("state poisoned")
                    .bounties
                    .insert(bounty.id, bounty.clone());
                bounty
            }
            GitHubIssueSyncBountyUpsert::BlockedByActivity(existing) => {
                let id = existing.id;
                state
                    .network
                    .lock()
                    .expect("state poisoned")
                    .bounties
                    .insert(id, existing);
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    } else {
        let result = {
            let mut network = state.network.lock().expect("state poisoned");
            network.upsert_github_issue_pooled_bounty(request, bounty_id, idempotency_key)
        };
        let bounty = match result {
            Ok(bounty) => bounty,
            Err(_) => {
                persist_all_risk_events(&state).await?;
                return Err(StatusCode::BAD_REQUEST);
            }
        };
        persist_bounty_and_ledger(&state, &bounty, &[]).await?;
        bounty
    };
    Ok(Json(bounty))
}

#[utoipa::path(
    post,
    path = "/v1/github/create-comment-plan",
    request_body = PlanGitHubCreateCommentRequest,
    responses((status = 200, description = "GitHub issue comment to reviewable canonical bounty handoff"))
)]
async fn plan_github_create_comment(
    Json(request): Json<PlanGitHubCreateCommentRequest>,
) -> Json<GitHubCreateCommentPlan> {
    Json(create_comment_plan(GitHubCreateCommentInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        comment_body: request.comment_body,
        contributor_login: request.contributor_login,
        comment_id: request.comment_id,
        existing_idempotency_keys: request.existing_idempotency_keys,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/social/mention-draft-plan",
    request_body = PlanSocialMentionDraftRequest,
    responses((status = 200, description = "Rollout-gated social mention review draft plan"))
)]
async fn plan_social_mention_draft(
    State(state): State<SharedState>,
    Json(request): Json<PlanSocialMentionDraftRequest>,
) -> Json<SocialMentionDraftPlan> {
    Json(current_social_mention_plan(&state, request).await)
}

async fn current_social_mention_plan(
    state: &SharedState,
    request: PlanSocialMentionDraftRequest,
) -> SocialMentionDraftPlan {
    let operator_enabled = env_flag("AGENT_BOUNTIES_SOCIAL_MENTION_DRAFTS_ENABLED");
    let github_conversion = if operator_enabled {
        match load_autonomous_bounty_feed(state, "base-mainnet", false).await {
            Ok(mut feed) => {
                state
                    .recovery_reservations
                    .exclude_from_reported_outcomes(&mut feed);
                github_issue_conversion_evidence(&feed)
            }
            Err(_) => unavailable_github_conversion_evidence(),
        }
    } else {
        unavailable_github_conversion_evidence()
    };
    social_mention_draft_plan(SocialMentionDraftInput {
        source_network: request.source_network,
        mention_url: request.mention_url,
        mention_id: request.mention_id,
        mention_text: request.mention_text,
        author_handle: request.author_handle,
        operator_enabled,
        github_conversion,
    })
}

#[utoipa::path(
    get,
    path = "/v1/social/mention-ingestion/readiness",
    responses((status = 200, body = SocialMentionIngestionReadiness))
)]
async fn social_mention_ingestion_readiness(
    State(state): State<SharedState>,
) -> Json<SocialMentionIngestionReadiness> {
    let plan = current_social_mention_plan(
        &state,
        PlanSocialMentionDraftRequest {
            source_network: "farcaster".to_string(),
            mention_url:
                "https://farcaster.xyz/readiness/0x0000000000000000000000000000000000000000"
                    .to_string(),
            mention_id: "readiness".to_string(),
            mention_text: "/agent-bounty create 1 USDC readiness probe".to_string(),
            author_handle: Some("readiness".to_string()),
        },
    )
    .await;
    let webhook_configured = state.neynar_social.is_some();
    let reply_configured = state
        .neynar_social
        .as_ref()
        .is_some_and(|config| config.reply_configured());
    let enabled =
        plan.gate.passed && state.store.is_some() && webhook_configured && reply_configured;
    Json(SocialMentionIngestionReadiness {
        schema_version: "agent-bounties/social-mention-ingestion-readiness-v1".to_string(),
        provider: "neynar".to_string(),
        source_network: "farcaster".to_string(),
        enabled,
        operator_enabled: plan.gate.operator_enabled,
        database_configured: state.store.is_some(),
        webhook_configured,
        reply_configured,
        bot_fid: state.neynar_social.as_ref().map(|config| config.bot_fid),
        bot_username: state
            .neynar_social
            .as_ref()
            .map(|config| config.bot_username.clone()),
        webhook_path: "/v1/social/webhooks/neynar".to_string(),
        gate_passed: plan.gate.passed,
        github_originated_canonical_funded: plan.gate.github_originated_canonical_funded,
        github_originated_canonical_settled: plan.gate.github_originated_canonical_settled,
        reason: if enabled {
            "signed mention ingestion, durable drafts, and bot replies are ready".to_string()
        } else if !plan.gate.passed {
            plan.gate.reason
        } else if state.store.is_none() {
            "DATABASE_URL is required for durable replay protection".to_string()
        } else if !webhook_configured {
            "Neynar webhook and bot identity are not configured".to_string()
        } else {
            "Neynar API key and approved signer are required for bot replies".to_string()
        },
        evidence_boundary: "This readiness report does not prove that a provider webhook is registered. A draft is never a published, funded, verified, or settled bounty."
            .to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/v1/social/webhooks/neynar",
    request_body = String,
    responses(
        (status = 200, body = SocialMentionWebhookResponse, description = "Signed mention ingested and reply completed or replayed"),
        (status = 202, body = SocialMentionWebhookResponse, description = "Signed event ignored, blocked, stored for review, or already being processed"),
        (status = 401, body = SocialMentionWebhookResponse, description = "Missing or invalid Neynar signature"),
        (status = 503, body = SocialMentionWebhookResponse, description = "Durable ingestion is not configured")
    )
)]
async fn ingest_neynar_social_mention(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<SocialMentionWebhookResponse>) {
    let Some(config) = state.neynar_social.as_ref() else {
        return social_webhook_response(
            StatusCode::SERVICE_UNAVAILABLE,
            false,
            false,
            "unconfigured",
            None,
            None,
            None,
            "Neynar webhook identity is not configured",
        );
    };
    let Some(store) = state.store.as_ref() else {
        return social_webhook_response(
            StatusCode::SERVICE_UNAVAILABLE,
            false,
            false,
            "unconfigured",
            None,
            None,
            None,
            "DATABASE_URL is required for durable ingestion",
        );
    };
    let Some(signature) = headers
        .get("x-neynar-signature")
        .and_then(|value| value.to_str().ok())
    else {
        return social_webhook_response(
            StatusCode::UNAUTHORIZED,
            false,
            false,
            "rejected",
            None,
            None,
            None,
            "missing Neynar signature",
        );
    };
    if !verify_neynar_signature(&body, signature, &config.webhook_secret) {
        return social_webhook_response(
            StatusCode::UNAUTHORIZED,
            false,
            false,
            "rejected",
            None,
            None,
            None,
            "invalid Neynar signature",
        );
    }
    let event: NeynarWebhookEvent = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(_) => {
            return social_webhook_response(
                StatusCode::BAD_REQUEST,
                false,
                false,
                "rejected",
                None,
                None,
                None,
                "invalid Neynar event JSON",
            )
        }
    };
    if event.event_type != "cast.created" || event.data.object != "cast" {
        return social_webhook_response(
            StatusCode::ACCEPTED,
            false,
            false,
            "ignored",
            None,
            None,
            None,
            "signed event is not a created cast",
        );
    }
    let cast = event.data;
    if !valid_farcaster_hash(&cast.hash)
        || cast.author.fid <= 0
        || cast.author.username.is_empty()
        || cast.author.username.len() > 64
        || !cast.author.username.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
        || cast.text.is_empty()
        || cast.text.len() > 8_000
    {
        return social_webhook_response(
            StatusCode::BAD_REQUEST,
            false,
            false,
            "rejected",
            None,
            None,
            None,
            "created cast fields are invalid",
        );
    }
    let directly_mentions_bot = cast
        .mentioned_profiles
        .iter()
        .any(|profile| profile.fid == config.bot_fid)
        || text_mentions_bot(&cast.text, &config.bot_username);
    let mention_id = cast.hash.to_ascii_lowercase();
    let mention_url = format!(
        "https://farcaster.xyz/{}/{}",
        cast.author.username, mention_id
    );
    let mut plan = current_social_mention_plan(
        &state,
        PlanSocialMentionDraftRequest {
            source_network: "farcaster".to_string(),
            mention_url: mention_url.clone(),
            mention_id: mention_id.clone(),
            mention_text: cast.text.clone(),
            author_handle: Some(cast.author.username.clone()),
        },
    )
    .await;
    let id = Uuid::new_v4();
    let draft_handoff_url = plan
        .ready
        .then(|| config.draft_handoff_url(id))
        .filter(|_| directly_mentions_bot);
    if let (Some(draft), Some(handoff_url)) = (&mut plan.draft, &draft_handoff_url) {
        draft.draft_handoff_url = handoff_url.clone();
    }
    let status = if !directly_mentions_bot || (plan.gate.passed && !plan.ready) {
        "ignored"
    } else if !plan.gate.passed {
        "blocked"
    } else if config.reply_configured() {
        "reply_pending"
    } else {
        "draft_ready"
    };
    let draft = if directly_mentions_bot && plan.ready {
        plan.draft
            .as_ref()
            .and_then(|draft| serde_json::to_value(draft).ok())
    } else {
        None
    };
    let reservation = match store
        .reserve_social_mention_ingestion(&NewSocialMentionIngestion {
            id,
            provider: "neynar".to_string(),
            provider_event_id: format!("cast.created:{mention_id}"),
            source_network: "farcaster".to_string(),
            mention_id,
            mention_url,
            author_fid: cast.author.fid,
            author_handle: Some(cast.author.username),
            mention_text: cast.text,
            status: status.to_string(),
            draft,
            idempotency_key: plan.idempotency_key.clone(),
            received_at: Utc::now(),
        })
        .await
    {
        Ok(reservation) => reservation,
        Err(_) => {
            return social_webhook_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                false,
                false,
                "failed",
                None,
                None,
                None,
                "could not durably reserve the mention",
            )
        }
    };
    let record = reservation.record;
    let persisted_handoff_url = record
        .draft
        .as_ref()
        .and_then(|draft| draft.get("draft_handoff_url"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if matches!(
        record.status.as_str(),
        "ignored" | "blocked" | "draft_ready"
    ) {
        let message = match record.status.as_str() {
            "ignored" => "signed cast did not contain a valid direct bot command",
            "blocked" => "canonical GitHub conversion gate is not currently satisfied",
            _ => "review draft stored; bot reply credentials are not configured",
        };
        return social_webhook_response(
            StatusCode::ACCEPTED,
            record.status == "draft_ready",
            !reservation.inserted,
            &record.status,
            Some(record.id),
            persisted_handoff_url,
            record.reply_cast_hash,
            message,
        );
    }
    if record.status == "replied" {
        return social_webhook_response(
            StatusCode::OK,
            true,
            true,
            "replied",
            Some(record.id),
            persisted_handoff_url,
            record.reply_cast_hash,
            "mention was already converted and replied to",
        );
    }
    let lease_token = Uuid::new_v4();
    let claimed = match store
        .claim_social_mention_reply(record.id, lease_token, 45)
        .await
    {
        Ok(claimed) => claimed,
        Err(_) => {
            return social_webhook_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                false,
                !reservation.inserted,
                "failed",
                Some(record.id),
                persisted_handoff_url,
                None,
                "could not acquire the reply lease",
            )
        }
    };
    let Some(claimed) = claimed else {
        return social_webhook_response(
            StatusCode::ACCEPTED,
            true,
            !reservation.inserted,
            &record.status,
            Some(record.id),
            persisted_handoff_url,
            record.reply_cast_hash,
            "another worker owns the active reply lease",
        );
    };
    let handoff_url = persisted_handoff_url
        .as_deref()
        .unwrap_or("https://agentbounties.app/post.html");
    match publish_neynar_draft_reply(config, &claimed, handoff_url).await {
        Ok(reply_cast_hash) => {
            let completed = store
                .complete_social_mention_reply(
                    claimed.id,
                    lease_token,
                    Some(&reply_cast_hash),
                    None,
                )
                .await
                .ok()
                .flatten();
            if completed.is_none() {
                return social_webhook_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    false,
                    !reservation.inserted,
                    "failed",
                    Some(claimed.id),
                    persisted_handoff_url,
                    Some(reply_cast_hash),
                    "reply was published but durable completion could not be recorded",
                );
            }
            social_webhook_response(
                StatusCode::OK,
                true,
                !reservation.inserted,
                "replied",
                Some(claimed.id),
                persisted_handoff_url,
                Some(reply_cast_hash),
                "review draft stored and bot reply published",
            )
        }
        Err(error) => {
            let _ = store
                .complete_social_mention_reply(claimed.id, lease_token, None, Some(&error))
                .await;
            social_webhook_response(
                StatusCode::BAD_GATEWAY,
                false,
                !reservation.inserted,
                "reply_failed",
                Some(claimed.id),
                persisted_handoff_url,
                None,
                "draft was stored but the provider reply failed; retry is safe",
            )
        }
    }
}

// Keeping the evidence-bearing response fields explicit makes security-sensitive
// early returns reviewable at each call site.
#[allow(clippy::too_many_arguments)]
fn social_webhook_response(
    status_code: StatusCode,
    accepted: bool,
    duplicate: bool,
    status: &str,
    ingestion_id: Option<Uuid>,
    draft_handoff_url: Option<String>,
    reply_cast_hash: Option<String>,
    message: &str,
) -> (StatusCode, Json<SocialMentionWebhookResponse>) {
    (
        status_code,
        Json(SocialMentionWebhookResponse {
            schema_version: "agent-bounties/social-mention-webhook-v1".to_string(),
            accepted,
            duplicate,
            status: status.to_string(),
            ingestion_id,
            draft_handoff_url,
            reply_cast_hash,
            message: message.to_string(),
            evidence_boundary: "This records a review draft and optional social reply only. It does not publish, fund, verify, settle, or prove payment for a bounty."
                .to_string(),
        }),
    )
}

fn verify_neynar_signature(body: &[u8], signature: &str, secret: &[u8]) -> bool {
    let Ok(signature) = hex::decode(signature.trim()) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha512>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

fn valid_farcaster_hash(hash: &str) -> bool {
    hash.len() == 42
        && hash.starts_with("0x")
        && hash[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn text_mentions_bot(text: &str, bot_username: &str) -> bool {
    let target = format!("@{}", bot_username.to_ascii_lowercase());
    text.split_whitespace().any(|token| {
        token
            .trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && !matches!(character, '@' | '_' | '-')
            })
            .eq_ignore_ascii_case(&target)
    })
}

async fn publish_neynar_draft_reply(
    config: &NeynarSocialIngestionConfig,
    ingestion: &SocialMentionIngestion,
    handoff_url: &str,
) -> Result<String, String> {
    let api_key = config
        .api_key
        .as_deref()
        .ok_or_else(|| "NEYNAR_API_KEY is not configured".to_string())?;
    let signer_uuid = config
        .signer_uuid
        .as_deref()
        .ok_or_else(|| "NEYNAR_SIGNER_UUID is not configured".to_string())?;
    let reply_text = format!("Draft ready for review (not published or funded): {handoff_url}");
    let idem_hash = hex::encode(Sha256::digest(
        format!("neynar-reply:{}", ingestion.id).as_bytes(),
    ));
    let idem = &idem_hash[..16];
    let response = config
        .client
        .post(format!("{}/v2/farcaster/cast/", config.api_base_url))
        .header("x-api-key", api_key)
        .json(&NeynarPublishCastRequest {
            signer_uuid,
            text: &reply_text,
            parent: &ingestion.mention_id,
            parent_author_fid: ingestion.author_fid,
            idem,
        })
        .send()
        .await
        .map_err(|error| format!("Neynar request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Neynar returned HTTP {status}"));
    }
    let published = response
        .json::<NeynarPublishCastResponse>()
        .await
        .map_err(|error| format!("Neynar response was invalid: {error}"))?;
    if !valid_farcaster_hash(&published.cast.hash) {
        return Err("Neynar response contained an invalid cast hash".to_string());
    }
    Ok(published.cast.hash)
}

#[utoipa::path(
    get,
    path = "/v1/social/mention-drafts/{id}",
    params(("id" = Uuid, Path, description = "Persisted social mention ingestion ID")),
    responses(
        (status = 200, body = SocialMentionDraftResponse),
        (status = 404, description = "Draft not found")
    )
)]
async fn get_social_mention_draft(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SocialMentionDraftResponse>, StatusCode> {
    let store = state.store.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let record = store
        .get_social_mention_ingestion(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let draft = record.draft.ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(SocialMentionDraftResponse {
        schema_version: "agent-bounties/social-mention-draft-v1".to_string(),
        ingestion_id: record.id,
        status: record.status,
        source_network: record.source_network,
        mention_url: record.mention_url,
        author_handle: record.author_handle,
        draft,
        evidence_boundary:
            "Review required: this draft has not been published, funded, verified, or settled."
                .to_string(),
    }))
}

fn unavailable_github_conversion_evidence() -> GitHubCanonicalConversionEvidence {
    GitHubCanonicalConversionEvidence {
        evidence_available: false,
        github_originated_canonical_funded: 0,
        github_originated_canonical_settled: 0,
        evidence_source: "indexed confirmed Base events joined to public GitHub-attributed terms"
            .to_string(),
    }
}

fn github_issue_conversion_evidence(
    feed: &[AutonomousBountyFeedItem],
) -> GitHubCanonicalConversionEvidence {
    let github_items = feed.iter().filter(|item| {
        item.terms.as_ref().is_some_and(|terms| {
            terms.document.source_url.as_deref().is_some_and(|url| {
                url.starts_with("https://github.com/") && url.contains("/issues/")
            })
        })
    });
    let mut funded = 0_u32;
    let mut settled = 0_u32;
    for item in github_items {
        if item
            .events
            .iter()
            .any(|event| event.kind == AutonomousBountyEventKind::BountyBecameClaimable)
        {
            funded = funded.saturating_add(1);
        }
        if item
            .events
            .iter()
            .any(|event| event.kind == AutonomousBountyEventKind::BountySettled)
        {
            settled = settled.saturating_add(1);
        }
    }
    GitHubCanonicalConversionEvidence {
        evidence_available: true,
        github_originated_canonical_funded: funded,
        github_originated_canonical_settled: settled,
        evidence_source: "indexed confirmed Base events joined to public GitHub-attributed terms"
            .to_string(),
    }
}

#[utoipa::path(
    post,
    path = "/v1/github/funding-comment-plan",
    request_body = PlanGitHubFundingCommentRequest,
    responses((status = 200, description = "GitHub public funding-comment signal plan"))
)]
async fn plan_github_funding_comment(
    Json(request): Json<PlanGitHubFundingCommentRequest>,
) -> Json<GitHubFundingCommentPlan> {
    Json(funding_comment_plan(GitHubFundingCommentInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        comment_body: request.comment_body,
        contributor_login: request.contributor_login,
        comment_id: request.comment_id,
        funding_api_base_url: request.funding_api_base_url,
        existing_idempotency_keys: request.existing_idempotency_keys,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/github/claim-comment-plan",
    request_body = PlanGitHubClaimCommentRequest,
    responses((status = 200, description = "GitHub public claim-comment reservation plan"))
)]
async fn plan_github_claim_comment(
    Json(request): Json<PlanGitHubClaimCommentRequest>,
) -> Json<GitHubClaimCommentPlan> {
    Json(claim_comment_plan(GitHubClaimCommentInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        comment_body: request.comment_body,
        contributor_login: request.contributor_login,
        comment_id: request.comment_id,
        claim_age_minutes: request.claim_age_minutes,
        progress_signal_count: request.progress_signal_count,
        active_claim_login: request.active_claim_login,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/github/proof-comment-plan",
    request_body = PlanGitHubProofCommentRequest,
    responses((status = 200, description = "GitHub proof comment markdown and check-run plan"))
)]
async fn plan_github_proof_comment(
    Json(request): Json<PlanGitHubProofCommentRequest>,
) -> Json<GitHubProofCommentPlan> {
    let comment = GitHubProofComment {
        bounty_id: request.bounty_id,
        proof_url: request.proof_url,
        verifier_summary: request.verifier_summary,
        settlement_url: request.settlement_url,
    };
    Json(proof_comment_plan(comment))
}

#[utoipa::path(
    post,
    path = "/v1/github/proof-comment-plan-from-proof",
    request_body = PlanGitHubProofCommentFromProofRequest,
    responses(
        (status = 200, description = "GitHub proof comment plan derived from a stored public proof"),
        (status = 404, description = "Proof not found, private, or missing verifier result")
    )
)]
async fn plan_github_proof_comment_from_proof(
    State(state): State<SharedState>,
    Json(request): Json<PlanGitHubProofCommentFromProofRequest>,
) -> Result<Json<GitHubProofCommentPlan>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    github_proof_comment_plan_from_proof(
        &network,
        &state.public_base_url,
        request.proof_id,
        request.settlement_url,
    )
    .map(Json)
}

fn github_proof_comment_plan_from_proof(
    network: &BountyNetwork,
    public_base_url: &str,
    proof_id: Uuid,
    settlement_url: Option<String>,
) -> Result<GitHubProofCommentPlan, StatusCode> {
    let proof = network.proofs.get(&proof_id).ok_or(StatusCode::NOT_FOUND)?;
    if proof.privacy == PrivacyLevel::Private {
        return Err(StatusCode::NOT_FOUND);
    }
    let verifier = network
        .verifier_results
        .get(&proof.verifier_result_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let verifier_summary = if verifier.summary.trim().is_empty() {
        format!("{:?} verifier accepted", verifier.kind)
    } else {
        format!("{:?}: {}", verifier.kind, verifier.summary.trim())
    };
    let comment = GitHubProofComment {
        bounty_id: proof.bounty_id,
        proof_url: format!(
            "{}/public/proofs/{}",
            public_base_url.trim_end_matches('/'),
            proof.id
        ),
        verifier_summary,
        settlement_url,
    };
    Ok(proof_comment_plan(comment))
}

#[utoipa::path(get, path = "/v1/bounties/{id}")]
async fn bounty_status(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<BountyStatusResponse>, StatusCode> {
    bounty_status_snapshot(&state, id).await.map(Json)
}

async fn bounty_status_snapshot(
    state: &SharedState,
    id: Uuid,
) -> Result<BountyStatusResponse, StatusCode> {
    service_runtime::bounty_status(state.store.as_ref(), &state.network, id)
        .await
        .map_err(|error| {
            if error.retryable() {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::NOT_FOUND
            }
        })
}

async fn public_proof_page(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    let proof = network
        .proofs
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();
    if proof.privacy == PrivacyLevel::Private {
        return Err(StatusCode::NOT_FOUND);
    }
    let verifier = network
        .verifier_results
        .get(&proof.verifier_result_id)
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();

    Ok(Html(web_public::render_proof_page(&proof, &verifier)))
}

async fn public_agent_profile(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    let agent = network
        .agents
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();
    let reputation_score = network
        .reputation_events
        .values()
        .filter(|event| event.agent_id == id)
        .map(|event| event.delta)
        .sum();
    let accepted_count = network
        .reputation_events
        .values()
        .filter(|event| event.agent_id == id && event.delta > 0)
        .count();
    let real_paid_intents = network
        .settlements
        .values()
        .flat_map(|settlement| &settlement.payout_intents)
        .filter(|intent| {
            intent.recipient_agent_id == id
                && intent.status == PayoutStatus::Paid
                && intent.rail != PaymentRail::Simulated
        })
        .collect::<Vec<_>>();
    let paid_currency = real_paid_intents
        .iter()
        .find(|intent| intent.amount.currency == "usdc")
        .or_else(|| real_paid_intents.first())
        .map(|intent| intent.amount.currency.clone())
        .unwrap_or_else(|| "usdc".to_string());
    let paid_minor = real_paid_intents
        .iter()
        .filter(|intent| intent.amount.currency == paid_currency.as_str())
        .map(|intent| intent.amount.amount)
        .sum();

    Ok(Html(web_public::render_agent_profile(
        &agent,
        accepted_count,
        reputation_score,
        paid_minor,
        &paid_currency,
    )))
}

async fn public_verifier_profile(
    State(state): State<SharedState>,
    Path(kind): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let verifier_kind = parse_verifier_kind(&kind).ok_or(StatusCode::NOT_FOUND)?;
    let stats = {
        let network = state.network.lock().expect("state poisoned");
        let results = network
            .verifier_results
            .values()
            .filter(|result| result.kind == verifier_kind)
            .collect::<Vec<_>>();
        let total_checks = results.len();
        let accepted_count = results
            .iter()
            .filter(|result| result.decision == VerificationDecision::Accepted)
            .count();
        let rejected_count = results
            .iter()
            .filter(|result| result.decision == VerificationDecision::Rejected)
            .count();
        let needs_review_count = results
            .iter()
            .filter(|result| result.decision == VerificationDecision::NeedsReview)
            .count();
        let average_confidence = if total_checks == 0 {
            0.0
        } else {
            results.iter().map(|result| result.confidence).sum::<f32>() / total_checks as f32
        };
        web_public::VerifierProfileStats {
            total_checks,
            accepted_count,
            rejected_count,
            needs_review_count,
            average_confidence,
        }
    };
    Ok(Html(web_public::render_verifier_profile(
        &format!("{verifier_kind:?}"),
        &stats,
    )))
}

async fn public_bounty_feed_page(State(state): State<SharedState>) -> Html<String> {
    let bounties = {
        let network = state.network.lock().expect("state poisoned");
        network.list_claimable_bounties()
    };
    let items = web_public::public_bounty_feed(&bounties, &state.public_base_url);
    Html(web_public::render_bounty_feed_page(&items))
}

async fn public_funding_feed_page(State(state): State<SharedState>) -> Html<String> {
    let items = {
        let network = state.network.lock().expect("state poisoned");
        public_funding_feed_items(&network, &state.public_base_url)
    };
    Html(web_public::render_funding_feed_page(&items))
}

async fn public_bounty_page(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let status = {
        let network = state.network.lock().expect("state poisoned");
        network.status(id).map_err(|_| StatusCode::NOT_FOUND)?
    };
    if status.bounty.privacy == PrivacyLevel::Private {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Html(web_public::render_public_bounty_page(
        &public_bounty_page_model(&status, &state.public_base_url),
    )))
}

fn public_funding_feed_items(
    network: &BountyNetwork,
    public_base_url: &str,
) -> Vec<web_public::PublicFundingFeedItem> {
    let mut items = network
        .bounties
        .values()
        .filter_map(|bounty| network.status(bounty.id).ok())
        .filter(public_status_accepts_funding)
        .map(|status| public_funding_feed_item(&status, public_base_url))
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        public_remaining_partition_count(right)
            .cmp(&public_remaining_partition_count(left))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.bounty_id.cmp(&right.bounty_id))
    });
    items
}

fn public_status_accepts_funding(status: &BountyStatusResponse) -> bool {
    let public_non_terminal = status.bounty.privacy != PrivacyLevel::Private
        && !matches!(
            status.bounty.status,
            BountyStatus::Paid
                | BountyStatus::Refunded
                | BountyStatus::Disputed
                | BountyStatus::Expired
        );
    let partition_remaining = status
        .funding_summary
        .partitions
        .iter()
        .any(|partition| partition.remaining.amount > 0);
    public_non_terminal && (partition_remaining || status.funding_summary.remaining.amount > 0)
}

fn public_remaining_partition_count(item: &web_public::PublicFundingFeedItem) -> usize {
    item.funding_partitions
        .iter()
        .filter(|partition| partition.remaining_minor > 0)
        .count()
}

fn public_funding_feed_item(
    status: &BountyStatusResponse,
    public_base_url: &str,
) -> web_public::PublicFundingFeedItem {
    let api = public_base_url.trim_end_matches('/');
    let bounty = &status.bounty;
    let funding_intent_url = format!("{api}/v1/bounties/{}/funding-intents", bounty.id);
    let public_url = format!("{api}/public/bounties/{}", bounty.id);
    let funding_partitions = status
        .funding_summary
        .partitions
        .iter()
        .map(|partition| web_public::PublicFundingPartition {
            rail: format!("{:?}", partition.rail),
            target_minor: partition.target.amount,
            confirmed_minor: partition.confirmed.amount,
            remaining_minor: partition.remaining.amount,
            currency: partition.target.currency.clone(),
            contribution_count: partition.contribution_count,
            escrow_count: partition.escrow_count,
            claimable: partition.claimable,
        })
        .collect::<Vec<_>>();
    let funding_intent_examples = web_public::public_funding_intent_examples(
        &bounty.id.to_string(),
        &funding_intent_url,
        &public_url,
        &format!("{:?}", bounty.funding_mode),
        status.funding_summary.remaining.amount,
        &status.funding_summary.remaining.currency,
        &funding_partitions,
    );
    web_public::PublicFundingFeedItem {
        bounty_id: bounty.id.to_string(),
        title: bounty.title.clone(),
        template_slug: bounty.template_slug.clone(),
        amount_minor: bounty.amount.amount,
        currency: bounty.amount.currency.clone(),
        funding_mode: format!("{:?}", bounty.funding_mode),
        status: format!("{:?}", bounty.status),
        privacy: format!("{:?}", bounty.privacy),
        terms_hash: bounty.terms_hash.clone(),
        created_at: bounty.created_at.to_rfc3339(),
        claimable: status.funding_summary.claimable,
        funding_target_minor: status.funding_summary.target.amount,
        funding_applied_minor: status.funding_summary.applied.amount,
        funding_remaining_minor: status.funding_summary.remaining.amount,
        contribution_count: status.funding_summary.contribution_count,
        public_url,
        status_url: format!("{api}/v1/bounties/{}", bounty.id),
        template_url: format!("{api}/public/templates/{}", bounty.template_slug),
        funding_intent_url,
        funding_contribution_url: format!("{api}/v1/bounties/{}/funding-contributions", bounty.id),
        funding_partitions,
        funding_intent_examples,
    }
}

fn public_bounty_page_model(
    status: &BountyStatusResponse,
    public_base_url: &str,
) -> web_public::PublicBountyPage {
    let api = public_base_url.trim_end_matches('/');
    let bounty = &status.bounty;
    let verification_type = status
        .verifier_results
        .iter()
        .max_by_key(|result| result.created_at)
        .map(|result| format!("{:?}", result.kind))
        .or_else(|| {
            web_public::bounty_templates()
                .into_iter()
                .find(|template| template.slug == bounty.template_slug)
                .map(|template| template.verifier.to_string())
        })
        .unwrap_or_else(|| "Unknown".to_string());
    let proof_urls = status
        .proofs
        .iter()
        .filter(|proof| proof.privacy != PrivacyLevel::Private)
        .map(|proof| format!("{api}/public/proofs/{}", proof.id))
        .collect();
    let public_url = format!("{api}/public/bounties/{}", bounty.id);
    let funding_partitions = status
        .funding_summary
        .partitions
        .iter()
        .map(|partition| web_public::PublicFundingPartition {
            rail: format!("{:?}", partition.rail),
            target_minor: partition.target.amount,
            confirmed_minor: partition.confirmed.amount,
            remaining_minor: partition.remaining.amount,
            currency: partition.target.currency.clone(),
            contribution_count: partition.contribution_count,
            escrow_count: partition.escrow_count,
            claimable: partition.claimable,
        })
        .collect::<Vec<_>>();
    let funding_intent_url = format!("{api}/v1/bounties/{}/funding-intents", bounty.id);
    let funding_intent_examples = web_public::public_funding_intent_examples(
        &bounty.id.to_string(),
        &funding_intent_url,
        &public_url,
        &format!("{:?}", bounty.funding_mode),
        status.funding_summary.remaining.amount,
        &status.funding_summary.remaining.currency,
        &funding_partitions,
    );
    let verifier_result_links = status
        .verifier_results
        .iter()
        .map(|result| web_public::PublicBountyRecordLink {
            label: format!(
                "{:?} {:?} verifier result {}",
                result.kind, result.decision, result.id
            ),
            url: format!("{public_url}#verifier-results"),
        })
        .collect();
    let settlement_links = status
        .settlements
        .iter()
        .map(|settlement| {
            let paid_payouts = settlement
                .payout_intents
                .iter()
                .filter(|intent| intent.status == PayoutStatus::Paid)
                .count();
            let total_payouts = settlement.payout_intents.len();
            web_public::PublicBountyRecordLink {
                label: format!(
                    "{:?} settlement {} ({paid_payouts}/{total_payouts} payouts paid)",
                    settlement.rail, settlement.id
                ),
                url: format!("{public_url}#settlements"),
            }
        })
        .collect();
    let template_signal_links = status
        .template_signals
        .iter()
        .map(|signal| web_public::PublicBountyRecordLink {
            label: format!("{} template signal {}", signal.template_slug, signal.id),
            url: format!("{api}/public/templates/{}", signal.template_slug),
        })
        .collect();
    web_public::PublicBountyPage {
        bounty_id: bounty.id.to_string(),
        title: bounty.title.clone(),
        template_slug: bounty.template_slug.clone(),
        amount_minor: bounty.amount.amount,
        currency: bounty.amount.currency.clone(),
        funding_mode: format!("{:?}", bounty.funding_mode),
        privacy: format!("{:?}", bounty.privacy),
        status: format!("{:?}", bounty.status),
        terms_hash: bounty.terms_hash.clone(),
        created_at: bounty.created_at.to_rfc3339(),
        verification_type,
        claimable: status.funding_summary.claimable,
        funding_target_minor: status.funding_summary.target.amount,
        funding_applied_minor: status.funding_summary.applied.amount,
        funding_remaining_minor: status.funding_summary.remaining.amount,
        contribution_count: status.funding_summary.contribution_count,
        public_url,
        claim_url: format!("{api}/v1/bounties/{}/claim", bounty.id),
        status_url: format!("{api}/v1/bounties/{}", bounty.id),
        template_url: format!("{api}/public/templates/{}", bounty.template_slug),
        funding_intent_url,
        funding_contribution_url: format!("{api}/v1/bounties/{}/funding-contributions", bounty.id),
        proof_urls,
        funding_partitions,
        funding_intent_examples,
        verifier_result_links,
        settlement_links,
        template_signal_links,
    }
}

async fn public_capability_feed_page(State(state): State<SharedState>) -> Html<String> {
    let (capabilities, agents, reputation_events, settlements) = {
        let network = state.network.lock().expect("state poisoned");
        (
            network.capabilities.values().cloned().collect::<Vec<_>>(),
            network.agents.values().cloned().collect::<Vec<_>>(),
            network
                .reputation_events
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            network.settlements.values().cloned().collect::<Vec<_>>(),
        )
    };
    let items = web_public::public_capability_feed(
        &capabilities,
        &agents,
        &reputation_events,
        &settlements,
        &state.public_base_url,
    );
    Html(web_public::render_capability_feed_page(&items))
}

async fn public_template_index() -> Html<String> {
    Html(web_public::render_template_index(
        &web_public::bounty_templates(),
    ))
}

async fn public_template_page(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let template = web_public::bounty_templates()
        .into_iter()
        .find(|template| template.slug == slug)
        .ok_or(StatusCode::NOT_FOUND)?;
    let stats = {
        let network = state.network.lock().expect("state poisoned");
        let matching = network
            .template_signals
            .values()
            .filter(|signal| signal.template_slug == slug && signal.success)
            .collect::<Vec<_>>();
        let currency = matching
            .first()
            .map(|signal| signal.amount.currency.clone())
            .unwrap_or_else(|| "usdc".to_string());
        let accepted_value_minor = matching
            .iter()
            .filter(|signal| signal.amount.currency == currency)
            .map(|signal| signal.amount.amount)
            .sum();
        web_public::TemplateStats {
            accepted_count: matching.len(),
            accepted_value_minor,
            currency,
        }
    };
    Ok(Html(web_public::render_template_page(
        &template,
        Some(&stats),
    )))
}

fn parse_verifier_kind(kind: &str) -> Option<VerifierKind> {
    match kind.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "manual" => Some(VerifierKind::Manual),
        "jsonschema" => Some(VerifierKind::JsonSchema),
        "dockercommand" => Some(VerifierKind::DockerCommand),
        "githubci" => Some(VerifierKind::GitHubCi),
        "httpcallback" => Some(VerifierKind::HttpCallback),
        "aijudgefilter" => Some(VerifierKind::AiJudgeFilter),
        _ => None,
    }
}

async fn record_eval_run(state: &SharedState, run: EvalRun) -> Result<(), StatusCode> {
    service_runtime::record_eval_run(state.store.as_ref(), &state.eval_runs, run)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn mutation_status(error: service_runtime::MutationError) -> StatusCode {
    if error.is_invalid() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

async fn hydrate_network(store: &PostgresStore) -> anyhow::Result<BountyNetwork> {
    service_runtime::hydrate_bounty_network(store).await
}

fn map_objective_error(error: ObjectiveError) -> StatusCode {
    match error {
        ObjectiveError::ProposalNotFound(_)
        | ObjectiveError::ContributionNeedNotFound(_)
        | ObjectiveError::ContributionOfferNotFound(_)
        | ObjectiveError::UnknownParticipant(_) => StatusCode::NOT_FOUND,
        ObjectiveError::StaleAction
        | ObjectiveError::ProposalExpired
        | ObjectiveError::ProposalAlreadyAccepted
        | ObjectiveError::InvalidAction(_, _)
        | ObjectiveError::NotReady(_)
        | ObjectiveError::AmendmentsUnavailable => StatusCode::CONFLICT,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn map_objective_db_error(error: DbError) -> StatusCode {
    match error {
        DbError::ObjectiveAlreadyExists(_) | DbError::ObjectiveRevisionConflict { .. } => {
            StatusCode::CONFLICT
        }
        DbError::ObjectiveNotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn load_objective(state: &SharedState, id: Id) -> Result<Objective, StatusCode> {
    if let Some(store) = &state.store {
        return store
            .get_objective(id)
            .await
            .map_err(map_objective_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    }
    state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .get(&id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)
}

async fn load_objectives(state: &SharedState) -> Result<Vec<Objective>, StatusCode> {
    if let Some(store) = &state.store {
        return store
            .list_objectives()
            .await
            .map_err(map_objective_db_error);
    }
    let mut objectives = state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .values()
        .cloned()
        .collect::<Vec<_>>();
    objectives.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(objectives)
}

async fn persist_new_objective(
    state: &SharedState,
    objective: &Objective,
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .create_objective(objective)
            .await
            .map_err(map_objective_db_error)?;
    } else {
        let mut network = state.network.lock().expect("state poisoned");
        if network.objectives.contains_key(&objective.id) {
            return Err(StatusCode::CONFLICT);
        }
        network.objectives.insert(objective.id, objective.clone());
        return Ok(());
    }
    state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .insert(objective.id, objective.clone());
    Ok(())
}

async fn persist_objective_replacement(
    state: &SharedState,
    objective: &Objective,
    expected_revision: u64,
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .replace_objective(objective, expected_revision)
            .await
            .map_err(map_objective_db_error)?;
    } else {
        let mut network = state.network.lock().expect("state poisoned");
        let current_revision = network
            .objectives
            .get(&objective.id)
            .map(|current| current.revision)
            .ok_or(StatusCode::NOT_FOUND)?;
        if current_revision != expected_revision {
            return Err(StatusCode::CONFLICT);
        }
        network.objectives.insert(objective.id, objective.clone());
        return Ok(());
    }
    state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .insert(objective.id, objective.clone());
    Ok(())
}

async fn load_objective_canonical_evidence(
    state: &SharedState,
    objectives: &[Objective],
) -> Result<ObjectiveCanonicalEvidence, StatusCode> {
    let Some(store) = &state.store else {
        return Ok(ObjectiveCanonicalEvidence::default());
    };
    let mut networks = BTreeSet::new();
    for objective in objectives {
        let Some(bundle) = objective.accepted_value_bundle.as_ref() else {
            continue;
        };
        if let Some(payment) = &bundle.monetary_payment {
            networks.insert(payment.bounty.network.clone());
        }
        for need in &bundle.contribution_needs {
            if let domain::ContributionCompensation::Paid { payment } = &need.compensation {
                networks.insert(payment.bounty.network.clone());
            }
        }
    }
    if networks.is_empty() {
        return Ok(ObjectiveCanonicalEvidence::default());
    }
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(map_objective_db_error)?;
    let mut evidence = ObjectiveCanonicalEvidence::default();
    for network in networks {
        let events = store
            .list_autonomous_bounty_events(&network)
            .await
            .map_err(map_objective_db_error)?;
        let mut feed = build_autonomous_bounty_feed(events, terms.clone(), false)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.recovery_reservations.apply(&mut feed, false);
        state
            .recovery_reservations
            .exclude_from_reported_outcomes(&mut feed);
        let mut network_evidence = build_objective_canonical_evidence(&network, &feed);
        evidence.funding.append(&mut network_evidence.funding);
        evidence
            .settlements
            .append(&mut network_evidence.settlements);
    }
    Ok(evidence)
}

async fn persist_bounty_and_ledger(
    state: &SharedState,
    bounty: &domain::Bounty,
    ledger_entries: &[ledger::LedgerEntry],
) -> Result<(), StatusCode> {
    service_runtime::persist_bounty_and_ledger(
        state.store.as_ref(),
        &state.network,
        bounty,
        ledger_entries,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn persist_ledger_entries(
    store: &PostgresStore,
    entries: &[ledger::LedgerEntry],
) -> Result<(), StatusCode> {
    service_runtime::persist_ledger_entries(store, entries)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn persist_all_risk_events(state: &SharedState) -> Result<(), StatusCode> {
    service_runtime::persist_all_risk_events(state.store.as_ref(), &state.network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[allow(dead_code)]
fn expected_digest_for_body(body: &str) -> String {
    hash_artifact(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::B256,
        signers::{local::PrivateKeySigner, SignerSync},
    };
    use app::{
        AddFundingContributionRequest, ClaimBountyRequest, CreateFundingIntentRequest,
        OpenPooledBountyRequest, PostBountyRequest, RegisterAgentRequest,
        RegisterCapabilityRequest, SubmitResultRequest, VerifySubmissionRequest,
    };
    use chrono::TimeZone;
    use db::{
        PlatformClaimCohortStats, PlatformDailyStats, PlatformIdentityStats,
        PlatformMetricsCoverageStats, PlatformPayoutStats,
    };
    use domain::{
        AffectedPartyDeclaration, Bounty, BountyStatus, CapabilityClass, DeliverableAccessPolicy,
        ExpectedEffect, FundingIntentStatus, FundingMode, IdentityDisclosure, ObjectiveAuthority,
        ObjectiveAuthorityKind, ObjectiveParticipant, ObjectivePrivacyDeclaration, ObjectiveStatus,
        ObjectiveVerificationMechanism, ObjectiveVerificationPolicy, ParticipantKind,
        PaymentEventStatus, PaymentRail, PayoutStatus, ProofRecord, PublicEvidencePolicy,
        RightsPolicy, VerifierKind,
    };
    use github_app::GitHubCheckConclusion;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        str::FromStr,
        thread,
    };

    type TestHmacSha256 = Hmac<Sha256>;

    fn open_competition_heartbeat(
        now: DateTime<Utc>,
        status: &str,
        cursor: Option<u64>,
        age_seconds: i64,
    ) -> BaseIndexerHeartbeat {
        BaseIndexerHeartbeat {
            network: "base-mainnet".to_string(),
            escrow_contract: "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5".to_string(),
            status: status.to_string(),
            started_at: now - ChronoDuration::seconds(age_seconds + 1),
            completed_at: Some(now - ChronoDuration::seconds(age_seconds)),
            latest_block: cursor.map(|block| block.saturating_add(2)),
            confirmed_to_block: cursor,
            from_block: cursor,
            to_block: cursor,
            fetched_logs: 0,
            persisted_cursor_block: cursor,
            skipped_reason: None,
            error_message: None,
            updated_at: now - ChronoDuration::seconds(age_seconds),
        }
    }

    #[test]
    fn open_competition_monitoring_requires_fresh_healthy_cursor_evidence() {
        let now = Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).unwrap();
        let safe_block = 50_000_000;
        let healthy = open_competition_heartbeat(now, "success", Some(safe_block), 15);
        assert!(open_competition_monitoring_is_fresh(
            &healthy, safe_block, now
        ));

        let stale = open_competition_heartbeat(now, "success", Some(safe_block), 91);
        assert!(!open_competition_monitoring_is_fresh(
            &stale, safe_block, now
        ));

        let lagging = open_competition_heartbeat(now, "success", Some(safe_block - 21), 15);
        assert!(!open_competition_monitoring_is_fresh(
            &lagging, safe_block, now
        ));

        let failed = open_competition_heartbeat(now, "failed", Some(safe_block), 15);
        assert!(!open_competition_monitoring_is_fresh(
            &failed, safe_block, now
        ));
    }

    #[test]
    fn open_competition_monitoring_accepts_only_the_caught_up_skip_reason() {
        let now = Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).unwrap();
        let safe_block = 50_000_000;
        let mut caught_up = open_competition_heartbeat(now, "skipped", Some(safe_block), 15);
        caught_up.skipped_reason = Some("no confirmed blocks are ready to scan".to_string());
        assert!(open_competition_monitoring_is_fresh(
            &caught_up, safe_block, now
        ));

        caught_up.skipped_reason =
            Some("latest block is below configured confirmations".to_string());
        assert!(!open_competition_monitoring_is_fresh(
            &caught_up, safe_block, now
        ));

        caught_up.skipped_reason = Some("no confirmed blocks are ready to scan".to_string());
        caught_up.error_message = Some("redacted failure".to_string());
        assert!(!open_competition_monitoring_is_fresh(
            &caught_up, safe_block, now
        ));
    }

    #[test]
    fn public_metrics_marks_indexer_heartbeats_delayed_after_five_minutes() {
        let now = Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).unwrap();
        let fresh = open_competition_heartbeat(now, "success", Some(50_000_000), 300);
        assert!(public_metrics_indexer_heartbeat_fresh(&fresh, now));

        let stale = open_competition_heartbeat(now, "success", Some(50_000_000), 301);
        assert!(!public_metrics_indexer_heartbeat_fresh(&stale, now));

        let mut lagging = open_competition_heartbeat(now, "success", Some(50_000_000), 1);
        lagging.latest_block = Some(50_000_021);
        assert!(!public_metrics_indexer_heartbeat_fresh(&lagging, now));

        let mut failed = open_competition_heartbeat(now, "success", Some(50_000_000), 1);
        failed.error_message = Some("redacted".to_string());
        assert!(!public_metrics_indexer_heartbeat_fresh(&failed, now));
    }

    fn entrant_relay_fixture(action: u8) -> OpenCompetitionEntrantRelay {
        let now = Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).unwrap();
        OpenCompetitionEntrantRelay {
            id: Uuid::new_v4(),
            idempotency_key: format!("entrant-receipt-{action}"),
            network: "base-mainnet".to_string(),
            wallet: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_contract: "0x2222222222222222222222222222222222222222".to_string(),
            delegate: "0x3333333333333333333333333333333333333333".to_string(),
            action,
            wallet_nonce: 7,
            deadline: 1_786_118_700,
            payload_hash: format!("0x{}", "aa".repeat(32)),
            request_fingerprint: "fixture".to_string(),
            relayer_address: "0x4444444444444444444444444444444444444444".to_string(),
            status: OpenCompetitionEntrantRelayStatus::Broadcast,
            retryable: true,
            attempt_count: 1,
            tx_hash: Some(format!("0x{}", "55".repeat(32))),
            estimated_gas: Some(90_000),
            gas_limit: Some(120_000),
            error_code: None,
            error_message: None,
            receipt_block: None,
            receipt_block_hash: None,
            canonical_safe_block: None,
            canonical_safe_block_hash: None,
            canonical_event: None,
            payment_proven: false,
            created_at: now,
            updated_at: now,
        }
    }

    fn address_topic(address: &str) -> String {
        format!("0x{}{}", "00".repeat(12), &address[2..])
    }

    fn entrant_relay_receipt(
        relay: &OpenCompetitionEntrantRelay,
        bounty_signature: &str,
        bounty_topics_after_signature: Vec<String>,
    ) -> RpcTransactionReceipt {
        let transaction_hash = relay.tx_hash.clone().unwrap();
        let action_log = chain_base::RpcEvmLog {
            address: relay.wallet.clone(),
            topics: vec![
                event_topic("EntrantActionExecuted(uint8,address,address,uint256,bytes32)"),
                format!("0x{:064x}", relay.action),
                address_topic(&relay.delegate),
                address_topic(&relay.relayer_address),
            ],
            data: format!("0x{:064x}{}", relay.wallet_nonce, &relay.payload_hash[2..]),
            transaction_hash: transaction_hash.clone(),
            block_number: "0x64".to_string(),
            log_index: "0x0".to_string(),
        };
        let mut topics = vec![event_topic(bounty_signature)];
        topics.extend(bounty_topics_after_signature);
        let bounty_log = chain_base::RpcEvmLog {
            address: relay.bounty_contract.clone(),
            topics,
            data: "0x".to_string(),
            transaction_hash: transaction_hash.clone(),
            block_number: "0x64".to_string(),
            log_index: "0x1".to_string(),
        };
        RpcTransactionReceipt {
            transaction_hash,
            block_number: Some("0x64".to_string()),
            block_hash: Some(format!("0x{}", "66".repeat(32))),
            status: Some("0x1".to_string()),
            logs: vec![action_log, bounty_log],
        }
    }

    #[test]
    fn entrant_relay_receipts_require_exact_action_and_solver_topics() {
        let bounty_id = format!("0x{}", "77".repeat(32));
        let sequence = format!("0x{:064x}", 1);
        let entry_number = format!("0x{:064x}", 1);
        let cases = [
            (
                0,
                "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)",
                vec![
                    bounty_id.clone(),
                    address_topic("0x1111111111111111111111111111111111111111"),
                    entry_number,
                ],
                "SolutionCommitted",
                false,
            ),
            (
                1,
                "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
                vec![
                    bounty_id.clone(),
                    sequence.clone(),
                    address_topic("0x1111111111111111111111111111111111111111"),
                ],
                "BountySettled",
                true,
            ),
            (
                1,
                "CompetitionSubmissionRejected(bytes32,uint64,address,uint256,bytes32)",
                vec![
                    bounty_id.clone(),
                    sequence,
                    address_topic("0x1111111111111111111111111111111111111111"),
                ],
                "CompetitionSubmissionRejected",
                false,
            ),
            (
                2,
                "EntryBondWithdrawn(bytes32,address,uint256)",
                vec![
                    bounty_id.clone(),
                    address_topic("0x1111111111111111111111111111111111111111"),
                ],
                "EntryBondWithdrawn",
                false,
            ),
        ];
        for (action, signature, topics, expected_event, expected_payment) in cases {
            let relay = entrant_relay_fixture(action);
            let receipt = entrant_relay_receipt(&relay, signature, topics);
            assert_eq!(
                validate_open_competition_entrant_relay_receipt(&relay, &receipt).unwrap(),
                (expected_event, expected_payment)
            );
        }

        let relay = entrant_relay_fixture(0);
        let misplaced_wallet = entrant_relay_receipt(
            &relay,
            "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)",
            vec![
                address_topic(&relay.wallet),
                address_topic("0x9999999999999999999999999999999999999999"),
                format!("0x{:064x}", 1),
            ],
        );
        assert_eq!(
            validate_open_competition_entrant_relay_receipt(&relay, &misplaced_wallet),
            Err(StatusCode::BAD_GATEWAY)
        );

        let mut high_nonce = entrant_relay_receipt(
            &relay,
            "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)",
            vec![
                bounty_id,
                address_topic(&relay.wallet),
                format!("0x{:064x}", 1),
            ],
        );
        high_nonce.logs[0].data = format!(
            "0x01{}{:016x}{}",
            "00".repeat(23),
            relay.wallet_nonce,
            &relay.payload_hash[2..]
        );
        assert_eq!(
            validate_open_competition_entrant_relay_receipt(&relay, &high_nonce),
            Err(StatusCode::BAD_GATEWAY)
        );
    }

    #[test]
    fn relayer_gas_cap_rejection_requires_configuration_change() {
        assert!(!x402_relay_error_is_retryable(
            &ChainBaseError::RelayerGasLimitExceeded {
                estimated: 565_940,
                maximum: 300_000,
            }
        ));
        assert!(x402_relay_error_is_retryable(
            &ChainBaseError::RelayerProvider("temporary RPC failure".to_string())
        ));
    }

    #[tokio::test]
    async fn objective_api_requires_signed_creation_and_preserves_role_boundaries() {
        let signer: PrivateKeySigner =
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                .parse()
                .unwrap();
        let participant_id = Uuid::new_v4();
        let draft = ObjectiveCreationDraft {
            id: Uuid::new_v4(),
            title: "Publish a verified public report".to_string(),
            desired_outcome: "A source-linked report passes its committed review.".to_string(),
            human_purpose: "Enable an informed decision by the named beneficiary.".to_string(),
            participants: vec![ObjectiveParticipant {
                id: participant_id,
                kind: ParticipantKind::Organization,
                display_name: "Requesting organization".to_string(),
                wallet: format!("{:#x}", signer.address()),
                identity_disclosure: IdentityDisclosure::Pseudonymous,
                public_identity_reference: None,
            }],
            requesting_party_id: participant_id,
            beneficiary_ids: vec![participant_id],
            affected_parties: vec![AffectedPartyDeclaration {
                participant_id,
                expected_effect: ExpectedEffect::Mixed,
                description: "Receives the result and bears the decision risk.".to_string(),
            }],
            authority: ObjectiveAuthority {
                kind: ObjectiveAuthorityKind::OrganizationWallet,
                member_ids: vec![participant_id],
                threshold: 1,
                public_statement:
                    "One declared organization wallet controls binding objective decisions."
                        .to_string(),
            },
            available_resources: Vec::new(),
            expected_final_deliverable: "Public report and evidence package".to_string(),
            requested_access_policy: DeliverableAccessPolicy::Public,
            requested_rights_policy: RightsPolicy {
                owner_ids: vec![participant_id],
                license_or_terms: "CC-BY-4.0".to_string(),
                restrictions: Vec::new(),
            },
            requested_final_verification: ObjectiveVerificationPolicy {
                mechanism: ObjectiveVerificationMechanism::CommittedVerifier {
                    verifier_id: participant_id,
                },
                acceptance_criteria: vec!["Every claim links to inspectable evidence.".to_string()],
                evidence_schema: "https://example.test/report-evidence.schema.json".to_string(),
                evidence_schema_hash: format!("0x{}", "1".repeat(64)),
                trust_assumptions: vec![
                    "The named verifier wallet follows the public criteria.".to_string()
                ],
            },
            privacy: ObjectivePrivacyDeclaration {
                blockchain_information_is_public: true,
                evidence_policy: PublicEvidencePolicy::Public,
                redaction_limits: "No private data is accepted by this public objective."
                    .to_string(),
            },
        };
        let plan = plan_objective_creation(Json(draft)).await.unwrap().0;
        let commitment = B256::from_str(&plan.commitment_hash).unwrap();
        let signature = signer.sign_message_sync(commitment.as_slice()).unwrap();
        let signed = SignedObjectiveCreation {
            plan,
            approvals: vec![domain::WalletApproval {
                participant_id,
                signature: signature.to_string(),
            }],
        };
        let state = test_state(BountyNetwork::default());
        let created = create_objective(State(state.clone()), Json(signed.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(created.objective.status, ObjectiveStatus::OpenForProposals);
        assert_eq!(created.objective.requesting_party_id, participant_id);
        assert_eq!(created.objective.authority.member_ids, vec![participant_id]);
        assert!(!created.readiness.ready);
        assert!(created
            .readiness
            .blockers
            .iter()
            .any(|blocker| blocker.contains("provider proposal")));

        let listed = list_objectives(State(state.clone())).await.unwrap().0;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].objective.id, created.objective.id);
        assert_eq!(
            create_objective(State(state), Json(signed))
                .await
                .unwrap_err(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn health_identifies_protocol_and_deployed_revision() {
        let response = health_response("0123456789abcdef0123456789abcdef01234567").into_response();

        assert_eq!(
            response.headers()["x-agent-bounties-revision"],
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(
            response.headers()["x-agent-bounties-protocol"],
            "agent-bounties/autonomous-v1"
        );
    }

    fn test_chatgpt_action_intent(action: &str) -> DbChatgptActionIntent {
        DbChatgptActionIntent {
            id: Uuid::new_v4(),
            idempotency_key: format!("chatgpt-test-{}", Uuid::new_v4()),
            action: action.to_string(),
            network: "base-mainnet".to_string(),
            opportunity_id: Some(
                "canonical_base:base-mainnet:0x1111111111111111111111111111111111111111"
                    .to_string(),
            ),
            bounty_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
            bounty_id: Some(format!("0x{}", "22".repeat(32))),
            actor_wallet: Some("0x3333333333333333333333333333333333333333".to_string()),
            amount_base_units: (action == "fund").then_some(1_000_000),
            details: serde_json::json!({}),
            request_fingerprint: "44".repeat(32),
            status: "pending_confirmation".to_string(),
            transaction_hash: Some(format!("0x{}", "55".repeat(32))),
            canonical_event_id: None,
            canonical_event_kind: None,
            confirmed_block: None,
            expires_at: Utc::now() + ChronoDuration::hours(1),
            created_at: Utc::now() - ChronoDuration::seconds(10),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn chatgpt_action_request_requires_action_specific_fields() {
        let valid_fund = normalize_chatgpt_action_request(CreateChatgptActionIntentRequest {
            idempotency_key: "chatgpt-fund-123".to_string(),
            action: ChatgptActionKind::Fund,
            network: Some("base-mainnet".to_string()),
            opportunity_id: Some("canonical_base:base-mainnet:0xabc".to_string()),
            bounty_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
            bounty_id: None,
            actor_wallet: None,
            amount_base_units: Some(1_000_000),
            details: serde_json::json!({"title": "Fund test"}),
        })
        .unwrap();
        assert_eq!(valid_fund.network.as_deref(), Some("base-mainnet"));
        assert_eq!(
            valid_fund.bounty_contract.as_deref(),
            Some("0x1111111111111111111111111111111111111111")
        );

        let missing_amount = normalize_chatgpt_action_request(CreateChatgptActionIntentRequest {
            amount_base_units: None,
            ..valid_fund.clone()
        });
        assert_eq!(missing_amount.unwrap_err(), StatusCode::BAD_REQUEST);

        let amount_on_completion =
            normalize_chatgpt_action_request(CreateChatgptActionIntentRequest {
                action: ChatgptActionKind::Complete,
                amount_base_units: Some(1),
                ..valid_fund
            });
        assert_eq!(amount_on_completion.unwrap_err(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn chatgpt_action_details_are_bounded_and_reject_sensitive_fields() {
        validate_chatgpt_action_details(
            ChatgptActionKind::Post,
            &serde_json::json!({
                "draft": {
                    "title": "Publish a public test",
                    "goal": "Produce a bounded artifact.",
                    "acceptance_criteria": ["The public check passes."],
                    "solver_reward_usdc": "2.00",
                    "verifier_reward_usdc": "0.10",
                    "source_url": null,
                    "crowdfund": true,
                    "discovery_source": "ChatGPT"
                }
            }),
        )
        .unwrap();
        validate_chatgpt_action_details(
            ChatgptActionKind::Complete,
            &serde_json::json!({
                "artifact_reference": "https://github.com/example/repo/commit/abc",
                "evidence": {
                    "transaction_hash": format!("0x{}", "11".repeat(32)),
                    "source_snapshot_digest": "sha256:public"
                }
            }),
        )
        .unwrap();

        for sensitive in [
            "private_key",
            "seed-phrase",
            "wallet signature",
            "payment_authorization",
            "card_number",
            "access_token",
        ] {
            let details = serde_json::json!({
                "artifact_reference": "https://example.com/artifact",
                "evidence": {(sensitive): "must-not-be-stored"}
            });
            assert_eq!(
                validate_chatgpt_action_details(ChatgptActionKind::Complete, &details).unwrap_err(),
                StatusCode::BAD_REQUEST,
                "{sensitive} must be rejected"
            );
        }

        assert_eq!(
            validate_chatgpt_action_details(
                ChatgptActionKind::Fund,
                &serde_json::json!({"artifact_reference": "wrong action field"})
            )
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            validate_chatgpt_action_details(
                ChatgptActionKind::Complete,
                &serde_json::json!({
                    "artifact_reference": "https://example.com/artifact",
                    "evidence": {"items": vec!["x"; 51]}
                })
            )
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn chatgpt_action_details_reject_excessive_nesting() {
        let mut nested = serde_json::json!("leaf");
        for _ in 0..8 {
            nested = serde_json::json!({"level": nested});
        }
        let details = serde_json::json!({
            "artifact_reference": "https://example.com/artifact",
            "evidence": nested
        });
        assert_eq!(
            validate_chatgpt_action_details(ChatgptActionKind::Complete, &details).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn chatgpt_action_confirmation_requires_exact_transaction_actor_amount_and_time() {
        let intent = test_chatgpt_action_intent("fund");
        let matching = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: "base-mainnet:101:0".to_string(),
            tx_hash: intent.transaction_hash.clone().unwrap(),
            block_number: 101,
            log_index: 0,
            contract_address: intent.bounty_contract.clone().unwrap(),
            bounty_id: intent.bounty_id.clone().unwrap(),
            kind: AutonomousBountyEventKind::FundingAdded,
            data: serde_json::json!({
                "contributor": intent.actor_wallet.clone().unwrap(),
                "amount": 1_000_000
            }),
            occurred_at: Utc::now(),
        };
        assert!(canonical_event_matches_chatgpt_intent(&intent, &matching));

        let mut wrong_transaction = matching.clone();
        wrong_transaction.tx_hash = format!("0x{}", "66".repeat(32));
        assert!(!canonical_event_matches_chatgpt_intent(
            &intent,
            &wrong_transaction
        ));

        let mut wrong_actor = matching.clone();
        wrong_actor.data["contributor"] =
            serde_json::json!("0x7777777777777777777777777777777777777777");
        assert!(!canonical_event_matches_chatgpt_intent(
            &intent,
            &wrong_actor
        ));

        let mut wrong_amount = matching.clone();
        wrong_amount.data["amount"] = serde_json::json!(999_999);
        assert!(!canonical_event_matches_chatgpt_intent(
            &intent,
            &wrong_amount
        ));

        let mut historical_replay = matching;
        historical_replay.occurred_at = intent.created_at - ChronoDuration::seconds(1);
        assert!(!canonical_event_matches_chatgpt_intent(
            &intent,
            &historical_replay
        ));
    }

    #[test]
    fn verify_action_accepts_only_canonical_settlement_or_rejection() {
        let mut intent = test_chatgpt_action_intent("verify");
        intent.amount_base_units = None;
        let event = |kind| AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: format!("base-mainnet:{}:0", Uuid::new_v4()),
            tx_hash: intent.transaction_hash.clone().unwrap(),
            block_number: 102,
            log_index: 0,
            contract_address: intent.bounty_contract.clone().unwrap(),
            bounty_id: intent.bounty_id.clone().unwrap(),
            kind,
            data: serde_json::json!({}),
            occurred_at: Utc::now(),
        };
        assert!(canonical_event_matches_chatgpt_intent(
            &intent,
            &event(AutonomousBountyEventKind::BountySettled)
        ));
        assert!(canonical_event_matches_chatgpt_intent(
            &intent,
            &event(AutonomousBountyEventKind::SubmissionRejected)
        ));
        assert!(!canonical_event_matches_chatgpt_intent(
            &intent,
            &event(AutonomousBountyEventKind::SubmissionAdded)
        ));
    }

    #[test]
    fn cloud_agent_errors_are_machine_readable_and_provider_safe() {
        let (status, Json(error)) = cloud_agent_api_error(CloudAgentError::InvalidResponse(
            "objective task dependencies contain a cycle".to_string(),
        ));
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(error.error_code, "cloud_agent_invalid_model_output");
        assert!(error.retryable);
        assert!(error.message.contains("cycle"));

        let (status, Json(error)) = cloud_agent_api_error(CloudAgentError::Provider(
            "provider returned HTTP 401 with private diagnostics".to_string(),
        ));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.error_code, "cloud_agent_provider_unavailable");
        assert!(error.retryable);
        assert!(!error.message.contains("private diagnostics"));
    }

    #[test]
    fn site_analytics_accepts_only_first_party_origins_and_minimized_fields() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://agentbounties.app"),
        );
        assert!(site_analytics_origin_allowed(&headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://bountyboard.global"),
        );
        assert!(site_analytics_origin_allowed(&headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://agentbounties.app.evil.example"),
        );
        assert!(!site_analytics_origin_allowed(&headers));

        let now = Utc.with_ymd_and_hms(2026, 7, 19, 18, 0, 0).unwrap();
        let event = validated_site_analytics_event(
            SiteAnalyticsEventRequest {
                event_id: Uuid::new_v4(),
                visitor_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                event_name: "competition_entry_confirmed".to_string(),
                page_path: "/competition.html".to_string(),
                source: Some("GitHub".to_string()),
                campaign: Some("launch-2026".to_string()),
                referrer_host: Some("GitHub.com".to_string()),
                opportunity_id: Some("eip155:8453:agent-bounties/open-competition-v1:0x1111111111111111111111111111111111111111".to_string()),
                bounty_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
                occurred_at: now,
            },
            now,
        )
        .unwrap();
        assert_eq!(event.source.as_deref(), Some("github"));
        assert_eq!(event.referrer_host.as_deref(), Some("github.com"));
        assert_eq!(event.page_path, "/competition.html");
    }

    #[test]
    fn interface_attribution_accepts_only_explicit_api_or_cli_values() {
        let mut headers = HeaderMap::new();
        assert_eq!(attributed_api_interface(&headers), None);

        headers.insert(
            INTERFACE_ATTRIBUTION_HEADER,
            HeaderValue::from_static("api"),
        );
        assert_eq!(
            attributed_api_interface(&headers),
            Some(ObservedInterface::Api)
        );

        headers.insert(
            INTERFACE_ATTRIBUTION_HEADER,
            HeaderValue::from_static("CLI"),
        );
        assert_eq!(
            attributed_api_interface(&headers),
            Some(ObservedInterface::Cli)
        );

        headers.insert(
            INTERFACE_ATTRIBUTION_HEADER,
            HeaderValue::from_static("website"),
        );
        assert_eq!(attributed_api_interface(&headers), None);
    }

    #[test]
    fn interface_analytics_exclusion_requires_a_verified_scoped_or_operator_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            ANALYTICS_EXCLUSION_HEADER,
            HeaderValue::from_static("wrong-token"),
        );
        assert!(!analytics_exclusion_is_authorized(
            Some("analytics-secret"),
            Some("operator-secret"),
            &headers,
        ));

        headers.insert(
            ANALYTICS_EXCLUSION_HEADER,
            HeaderValue::from_static("analytics-secret"),
        );
        assert!(analytics_exclusion_is_authorized(
            Some("analytics-secret"),
            Some("operator-secret"),
            &headers,
        ));

        headers.remove(ANALYTICS_EXCLUSION_HEADER);
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer operator-secret"),
        );
        assert!(analytics_exclusion_is_authorized(
            Some("analytics-secret"),
            Some("operator-secret"),
            &headers,
        ));
        assert!(!analytics_exclusion_is_authorized(None, None, &headers,));

        let mut response = StatusCode::OK.into_response();
        response
            .headers_mut()
            .insert(ANALYTICS_EXCLUDED_HEADER, HeaderValue::from_static("true"));
        assert_eq!(
            response
                .headers()
                .get(ANALYTICS_EXCLUDED_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
    }

    #[test]
    fn marketing_domains_redirect_once_and_preserve_deep_paths_and_queries() {
        assert_eq!(
            marketing_domain_destination("agentbounties.work", &"/".parse().unwrap()),
            Some("https://agentbounties.app/tasks/".to_string())
        );
        assert_eq!(
            marketing_domain_destination(
                "WWW.AGENTBOUNTIES.DEV:443",
                &"/sdk/rust?version=1".parse().unwrap()
            ),
            Some("https://agentbounties.app/sdk/rust?version=1".to_string())
        );
        assert_eq!(
            marketing_domain_destination(
                "bountyboard.global",
                &"/earn.html?from=legacy".parse().unwrap()
            ),
            Some("https://agentbounties.app/earn.html?from=legacy".to_string())
        );
        assert_eq!(
            marketing_domain_destination("api.agentbounties.app", &"/health".parse().unwrap()),
            None
        );
        assert_eq!(
            marketing_domain_destination("status.agentbounties.app", &"/".parse().unwrap()),
            Some("https://api.agentbounties.app/health".to_string())
        );
    }

    #[test]
    fn site_analytics_rejects_query_strings_unknown_events_and_stale_timestamps() {
        let now = Utc.with_ymd_and_hms(2026, 7, 19, 18, 0, 0).unwrap();
        let request = |event_name: &str, page_path: &str, occurred_at| SiteAnalyticsEventRequest {
            event_id: Uuid::new_v4(),
            visitor_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            event_name: event_name.to_string(),
            page_path: page_path.to_string(),
            source: None,
            campaign: None,
            referrer_host: None,
            opportunity_id: None,
            bounty_contract: None,
            occurred_at,
        };
        assert!(
            validated_site_analytics_event(request("page_view", "/?secret=1", now), now).is_err()
        );
        assert!(validated_site_analytics_event(request("arbitrary", "/", now), now).is_err());
        assert!(validated_site_analytics_event(
            request("page_view", "/", now - ChronoDuration::days(8)),
            now,
        )
        .is_err());
    }

    #[test]
    fn legal_policy_is_hash_bound_and_acceptance_is_action_wallet_and_time_bound() {
        let policy = build_legal_policy("https://agentbounties.app/");
        assert_eq!(policy.terms_version, LEGAL_TERMS_VERSION);
        assert_eq!(policy.statement_hash, legal_statement_hash());
        assert_eq!(policy.terms_url, "https://agentbounties.app/terms.html");
        assert!(policy
            .supported_actions
            .iter()
            .any(|action| action == "post_bounty"));
        assert!(policy
            .supported_actions
            .iter()
            .any(|action| action == "cancel_bounty"));
        assert_eq!(
            legal_website_base_url(None, "https://api.agentbounties.app/"),
            "https://agentbounties.app"
        );
        assert_eq!(
            legal_website_base_url(
                Some(" https://preview.example ".to_string()),
                "https://api.agentbounties.app"
            ),
            "https://preview.example"
        );

        let now = Utc.with_ymd_and_hms(2026, 7, 18, 18, 0, 0).unwrap();
        let mut request = RecordLegalAcceptanceRequest {
            terms_version: LEGAL_TERMS_VERSION.to_string(),
            privacy_version: LEGAL_PRIVACY_VERSION.to_string(),
            action: "post_bounty".to_string(),
            wallet_address: "0x1111111111111111111111111111111111111111".to_string(),
            statement_hash: legal_statement_hash(),
            acceptance_method: "api_explicit".to_string(),
            accepted_at: now,
        };
        assert_eq!(
            validate_legal_acceptance_request(&request, now).unwrap(),
            request.wallet_address
        );

        request.action = "settle_without_verification".to_string();
        assert_eq!(
            validate_legal_acceptance_request(&request, now),
            Err(StatusCode::BAD_REQUEST)
        );
        request.action = "post_bounty".to_string();
        request.accepted_at = now - ChronoDuration::minutes(16);
        assert_eq!(
            validate_legal_acceptance_request(&request, now),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn timeout_relay_accepts_only_exact_permissionless_transition_intent() {
        let bounty = "0x1111111111111111111111111111111111111111";
        let relayer = "0x2222222222222222222222222222222222222222";
        let mut intent = EvmTransactionIntent {
            from: Some(relayer.to_string()),
            to: bounty.to_string(),
            value_wei: 0,
            data: AutonomousTimeoutAction::ExpireSubmission
                .calldata()
                .to_string(),
            function: AutonomousTimeoutAction::ExpireSubmission
                .function()
                .to_string(),
        };

        assert!(validate_autonomous_timeout_intent(
            &intent,
            bounty,
            relayer,
            AutonomousTimeoutAction::ExpireSubmission,
        )
        .is_ok());

        intent.data = AutonomousTimeoutAction::ExpireClaim.calldata().to_string();
        assert_eq!(
            validate_autonomous_timeout_intent(
                &intent,
                bounty,
                relayer,
                AutonomousTimeoutAction::ExpireSubmission,
            ),
            Err(StatusCode::UNPROCESSABLE_ENTITY)
        );
        intent.data = AutonomousTimeoutAction::ExpireSubmission
            .calldata()
            .to_string();
        intent.value_wei = 1;
        assert_eq!(
            validate_autonomous_timeout_intent(
                &intent,
                bounty,
                relayer,
                AutonomousTimeoutAction::ExpireSubmission,
            ),
            Err(StatusCode::UNPROCESSABLE_ENTITY)
        );
    }

    #[test]
    fn atomic_sponsorship_accepts_only_one_exact_vault_call() {
        let sponsor = "0x2222222222222222222222222222222222222222";
        let relayer = "0x3333333333333333333333333333333333333333";
        let mut intent = EvmTransactionIntent {
            from: Some(relayer.to_string()),
            to: sponsor.to_string(),
            value_wei: 0,
            data: format!("0xba3ddedd{}", "00".repeat(608)),
            function: "sponsorAndClaim((address,address,uint64,uint256,bytes32,bytes32,bytes32,uint256,uint256,bytes32,uint256),bytes,uint8,bytes32,bytes32)".to_string(),
        };

        assert!(validate_atomic_sponsored_claim_intent(&intent, sponsor, relayer).is_ok());
        intent.value_wei = 1;
        assert!(validate_atomic_sponsored_claim_intent(&intent, sponsor, relayer).is_err());
        intent.value_wei = 0;
        intent.data.replace_range(2..10, "deadbeef");
        assert!(validate_atomic_sponsored_claim_intent(&intent, sponsor, relayer).is_err());
    }

    #[test]
    fn atomic_sponsorship_nonce_is_stable_and_candidate_bound() {
        let now = Utc::now();
        let mut sponsorship = BondSponsorship {
            id: Uuid::new_v4(),
            claim_candidate_id: Uuid::new_v4(),
            network: "base-mainnet".to_string(),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
            solver_wallet: "0x2222222222222222222222222222222222222222".to_string(),
            sponsor_wallet: "0x3333333333333333333333333333333333333333".to_string(),
            amount: 10_000,
            status: BondSponsorshipStatus::Reserved,
            transaction_hash: None,
            confirmed_block: None,
            failure_code: None,
            failure_message: None,
            created_at: now,
            updated_at: now,
        };
        let first = atomic_sponsorship_grant_nonce(&sponsorship);
        assert_eq!(first, atomic_sponsorship_grant_nonce(&sponsorship));
        assert_eq!(first.len(), 66);
        assert!(should_use_atomic_sponsorship(false, Some(&sponsorship)));
        assert!(!should_use_atomic_sponsorship(false, None));
        sponsorship.claim_candidate_id = Uuid::new_v4();
        assert_ne!(first, atomic_sponsorship_grant_nonce(&sponsorship));
    }

    #[test]
    fn indexed_claim_recovery_requires_current_round_and_exact_scope() {
        let now = Utc::now();
        let bounty_contract = "0x1111111111111111111111111111111111111111";
        let solver_wallet = "0x2222222222222222222222222222222222222222";
        let terms_hash = format!("0x{}", "33".repeat(32));
        let policy_hash = format!("0x{}", "44".repeat(32));
        let candidate = ClaimCandidate {
            id: Uuid::new_v4(),
            idempotency_key: "recover-indexed-claim".to_string(),
            network: "base-mainnet".to_string(),
            bounty_contract: bounty_contract.to_string(),
            solver_wallet: solver_wallet.to_string(),
            agent_id: None,
            eligibility_evidence: AgentEligibilityEvidence {
                agent_id: None,
                solver_wallet: solver_wallet.to_string(),
                capabilities: Vec::new(),
                paid_completions: 0,
                paid_usdc_base_units: 0,
            },
            eligibility_decision: AgentEligibilityDecision {
                eligible: true,
                reasons: Vec::new(),
            },
            status: ClaimCandidateStatus::AuthorizationReady,
            exclusive_until: Some(now),
            authorization_nonce: Some(format!("0x{}", "55".repeat(32))),
            authorization_valid_before: Some(1_800_000_000),
            claim_transaction_hash: None,
            canonical_event_id: None,
            failure_code: None,
            failure_message: None,
            created_at: now,
            updated_at: now,
        };
        let matching_claim = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: "base-mainnet:claim:1".to_string(),
            tx_hash: format!("0x{}", "66".repeat(32)),
            block_number: 100,
            log_index: 1,
            contract_address: bounty_contract.to_string(),
            bounty_id: format!("0x{}", "77".repeat(32)),
            kind: AutonomousBountyEventKind::BountyClaimed,
            data: serde_json::json!({
                "round": 1,
                "solver": solver_wallet,
                "terms_hash": terms_hash,
                "policy_hash": policy_hash,
                "claim_bond": 100_000u64,
                "claim_expires_at": 1_800_000_000u64
            }),
            occurred_at: now,
        };
        let mut item = AutonomousBountyFeedItem {
            bounty_id: matching_claim.bounty_id.clone(),
            bounty_contract: bounty_contract.to_string(),
            creator: "0x9999999999999999999999999999999999999999".to_string(),
            status: "claimable".to_string(),
            solver_reward: "2000000".to_string(),
            verifier_reward: "100000".to_string(),
            claim_bond: "100000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "2100000".to_string(),
            funded_amount: "2100000".to_string(),
            required_external_spend: "0".to_string(),
            gross_cash_margin: "2000000".to_string(),
            terms_hash: terms_hash.clone(),
            terms: None,
            terms_valid: true,
            verification_mode: "deterministic".to_string(),
            verifier_module: None,
            verifier_set_hash: None,
            verifier_threshold: Some(1),
            runner_identifier: Some("test_fixture".to_string()),
            verification_ready: true,
            verification_readiness_reason: "ready".to_string(),
            validation_errors: Vec::new(),
            events: vec![matching_claim.clone()],
        };

        assert!(
            current_indexed_claim_for_candidate(&item, &policy_hash, &candidate, 100_000).is_none()
        );
        item.status = "claimed".to_string();
        assert_eq!(
            current_indexed_claim_for_candidate(&item, &policy_hash, &candidate, 100_000)
                .map(|event| event.id),
            Some(matching_claim.id)
        );
        assert!(current_indexed_claim_for_candidate(
            &item,
            &format!("0x{}", "88".repeat(32)),
            &candidate,
            100_000
        )
        .is_none());

        let mut newer_other_solver = matching_claim;
        newer_other_solver.id = Uuid::new_v4();
        newer_other_solver.block_number = 101;
        newer_other_solver.data["round"] = serde_json::json!(2);
        newer_other_solver.data["solver"] =
            serde_json::json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        item.events.push(newer_other_solver);
        assert!(
            current_indexed_claim_for_candidate(&item, &policy_hash, &candidate, 100_000).is_none()
        );
    }

    #[test]
    fn persisted_claim_idempotency_is_bound_to_original_scope() {
        let now = Utc::now();
        let agent_id = Uuid::new_v4();
        let candidate = ClaimCandidate {
            id: Uuid::new_v4(),
            idempotency_key: "claim-once".to_string(),
            network: "base-mainnet".to_string(),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
            solver_wallet: "0x2222222222222222222222222222222222222222".to_string(),
            agent_id: Some(agent_id),
            eligibility_evidence: AgentEligibilityEvidence {
                agent_id: Some(agent_id),
                solver_wallet: "0x2222222222222222222222222222222222222222".to_string(),
                capabilities: Vec::new(),
                paid_completions: 0,
                paid_usdc_base_units: 0,
            },
            eligibility_decision: AgentEligibilityDecision {
                eligible: true,
                reasons: Vec::new(),
            },
            status: ClaimCandidateStatus::Relaying,
            exclusive_until: Some(now),
            authorization_nonce: Some(format!("0x{}", "11".repeat(32))),
            authorization_valid_before: Some(1_800_000_000),
            claim_transaction_hash: Some(format!("0x{}", "22".repeat(32))),
            canonical_event_id: None,
            failure_code: None,
            failure_message: None,
            created_at: now,
            updated_at: now,
        };

        assert!(validate_persisted_claim_candidate_scope(
            &candidate,
            "base-mainnet",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            Some(agent_id),
        )
        .is_ok());
        assert!(validate_persisted_claim_candidate_scope(
            &candidate,
            "base-sepolia",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            Some(agent_id),
        )
        .is_err());
        assert!(validate_persisted_claim_candidate_scope(
            &candidate,
            "base-mainnet",
            "0x1111111111111111111111111111111111111111",
            "0x3333333333333333333333333333333333333333",
            Some(agent_id),
        )
        .is_err());
        assert!(validate_persisted_claim_candidate_scope(
            &candidate,
            "base-mainnet",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            None,
        )
        .is_err());
    }

    #[test]
    fn agent_claim_relay_rejects_value_or_wrong_shape() {
        let bounty = "0x1111111111111111111111111111111111111111";
        let relayer = "0x2222222222222222222222222222222222222222";
        let mut intent = EvmTransactionIntent {
            from: Some(relayer.to_string()),
            to: bounty.to_string(),
            value_wei: 0,
            data: format!("0x{:0>456}", "abcd1234"),
            function:
                "claimWithAuthorization(address,uint256,uint256,bytes32,uint8,bytes32,bytes32)"
                    .to_string(),
        };
        assert!(validate_agent_claim_relay_intent(&intent, bounty, relayer).is_ok());
        intent.value_wei = 1;
        assert!(validate_agent_claim_relay_intent(&intent, bounty, relayer).is_err());
    }

    #[test]
    fn malformed_solver_signature_is_rejected_before_sponsorship() {
        let signature = AutonomousBountyAuthorizationSignature {
            v: 27,
            r: "0x01".to_string(),
            s: format!("0x{:064x}", 2),
        };
        let error = joined_signature(&signature).unwrap_err();
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn native_wallet_signature_is_split_and_normalized() {
        for (input_v, expected_v) in [(0_u8, 27_u8), (1, 28), (27, 27), (28, 28)] {
            let encoded = format!("0x{}{}{:02x}", "11".repeat(32), "22".repeat(32), input_v);

            let parsed = parse_native_wallet_signature(&encoded).unwrap();

            assert_eq!(parsed.v, expected_v);
            assert_eq!(parsed.r, format!("0x{}", "11".repeat(32)));
            assert_eq!(parsed.s, format!("0x{}", "22".repeat(32)));
            assert_eq!(
                joined_signature(&parsed).unwrap(),
                format!("0x{}{}{:02x}", "11".repeat(32), "22".repeat(32), expected_v)
            );
        }
    }

    #[test]
    fn malformed_native_wallet_signatures_are_rejected() {
        let invalid = [
            "0x01".to_string(),
            format!("{}{}1b", "11".repeat(32), "22".repeat(32)),
            format!("0x{}zz", "11".repeat(64)),
            format!("0x{}{}02", "11".repeat(32), "22".repeat(32)),
        ];

        for signature in invalid {
            let error = parse_native_wallet_signature(&signature).unwrap_err();
            assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        }
    }

    #[test]
    fn agent_claim_rejects_ambiguous_signature_forms_and_preserves_legacy_form() {
        let legacy = AutonomousBountyAuthorizationSignature {
            v: 27,
            r: format!("0x{}", "11".repeat(32)),
            s: format!("0x{}", "22".repeat(32)),
        };
        let mut request = AgentNativeClaimRequest {
            idempotency_key: "native-signature-test".to_string(),
            network: Some("base-mainnet".to_string()),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
            solver_wallet: "0x2222222222222222222222222222222222222222".to_string(),
            agent_id: None,
            request_bond_sponsorship: true,
            signature: Some(legacy.clone()),
            wallet_signature: None,
            source: Some("test".to_string()),
        };

        let resolved = resolve_agent_claim_signature(&request).unwrap().unwrap();
        assert_eq!(resolved.v, legacy.v);
        assert_eq!(resolved.r, legacy.r);
        assert_eq!(resolved.s, legacy.s);

        request.wallet_signature = Some(format!("0x{}{}1b", "11".repeat(32), "22".repeat(32)));
        let error = validate_agent_native_claim_request(&request).unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn agent_claim_returns_an_exact_eip1193_wallet_request_and_replay_path() {
        let payload: Eip3009AuthorizationTypedData = serde_json::from_value(serde_json::json!({
            "types": {},
            "domain": {
                "name": "USD Coin",
                "version": "2",
                "chainId": 8453,
                "verifyingContract": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
            },
            "primaryType": "ReceiveWithAuthorization",
            "message": {
                "from": "0x2222222222222222222222222222222222222222",
                "to": "0x1111111111111111111111111111111111111111",
                "value": "10000",
                "validAfter": "0",
                "validBefore": "1800000000",
                "nonce": format!("0x{}", "33".repeat(32))
            }
        }))
        .unwrap();
        let solver = "0x2222222222222222222222222222222222222222";

        let wallet_request = eip1193_wallet_request(solver, &payload);

        assert_eq!(wallet_request["method"], "eth_signTypedData_v4");
        assert_eq!(wallet_request["params"][0], solver);
        let encoded_payload = wallet_request["params"][1].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(encoded_payload).unwrap(),
            serde_json::json!(payload)
        );

        let request = AgentNativeClaimRequest {
            idempotency_key: "wallet-request-test".to_string(),
            network: Some("base-mainnet".to_string()),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
            solver_wallet: solver.to_string(),
            agent_id: None,
            request_bond_sponsorship: true,
            signature: None,
            wallet_signature: None,
            source: Some("test".to_string()),
        };
        let replay = signed_claim_request_template(&request);
        assert_eq!(replay["insert_signature_at"], "body.wallet_signature");
        assert!(replay["body"]["wallet_signature"]
            .as_str()
            .unwrap()
            .contains("wallet_request"));
    }

    #[test]
    fn x402_payment_required_response_uses_v2_wire_header_and_no_store() {
        let challenge = base_usdc_funding_challenge(
            "https://api.example/v1/x402/base/bounties/0x1111111111111111111111111111111111111111/funding?network=base-mainnet&amount=150000",
            "eip155:8453",
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "0x1111111111111111111111111111111111111111",
            150_000,
            300,
        )
        .unwrap();

        let response = x402_payment_required_response(challenge.clone()).unwrap();

        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store, private"
        );
        let decoded = payments_x402::decode_payment_required_header(
            response.headers()[PAYMENT_REQUIRED_HEADER]
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, challenge);
        assert_eq!(decoded.accepts[0].scheme, AGENT_BOUNTY_FUND_SCHEME);
    }

    #[test]
    fn x402_funding_amount_defaults_to_gap_and_rejects_overfunding_or_wrong_state() {
        assert_eq!(
            resolve_x402_funding_amount("open", "2000000", "150000", None).unwrap(),
            1_850_000
        );
        assert_eq!(
            resolve_x402_funding_amount("open", "2000000", "150000", Some(250000)).unwrap(),
            250_000
        );
        assert_eq!(
            resolve_x402_funding_amount("open", "2000000", "150000", Some(1_850_001)).unwrap_err(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            resolve_x402_funding_amount("claimable", "2000000", "2000000", None).unwrap_err(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn hosted_x402_intent_allows_only_exact_zero_value_funding_call() {
        let relayer = "0x2222222222222222222222222222222222222222";
        let bounty = "0x1111111111111111111111111111111111111111";
        let mut intent = EvmTransactionIntent {
            from: Some(relayer.to_string()),
            to: bounty.to_string(),
            value_wei: 0,
            data: format!(
                "0x{}{}",
                AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR,
                "00".repeat(8 * 32)
            ),
            function: AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION.to_string(),
        };
        assert!(validate_hosted_x402_intent(&intent, relayer, bounty).is_ok());

        intent.value_wei = 1;
        assert_eq!(
            validate_hosted_x402_intent(&intent, relayer, bounty).unwrap_err(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        intent.value_wei = 0;
        intent.data = "0xdeadbeef".to_string();
        assert_eq!(
            validate_hosted_x402_intent(&intent, relayer, bounty).unwrap_err(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn confirmed_x402_relay_emits_payment_response_only_after_canonical_event() {
        let state = test_state(BountyNetwork::default());
        let now = Utc::now();
        let attempt = X402RelayAttempt {
            id: Uuid::new_v4(),
            idempotency_key: "x402:test".to_string(),
            network: "base-mainnet".to_string(),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
            contributor: "0x3333333333333333333333333333333333333333".to_string(),
            amount: 150_000,
            authorization_nonce: format!("0x{}", "44".repeat(32)),
            authorization_valid_before: 2_000_000_000,
            request_fingerprint: "fingerprint".to_string(),
            relayer_address: "0x2222222222222222222222222222222222222222".to_string(),
            status: X402RelayStatus::Confirmed,
            retryable: false,
            attempt_count: 1,
            tx_hash: Some(format!("0x{}", "55".repeat(32))),
            estimated_gas: Some(100_000),
            gas_limit: Some(120_000),
            error_code: None,
            error_message: None,
            canonical_event_id: Some(Uuid::new_v4()),
            confirmed_block: Some(123),
            created_at: now,
            updated_at: now,
        };

        let public_relay = x402_public_relay(&attempt).to_string();
        assert!(!public_relay.contains(&attempt.idempotency_key));
        assert!(!public_relay.contains(&attempt.authorization_nonce));
        assert!(!public_relay.contains(&attempt.request_fingerprint));

        let response = x402_relay_response(&state, &attempt).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let settlement = payments_x402::decode_payment_response_header(
            response.headers()[PAYMENT_RESPONSE_HEADER]
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert!(settlement.success);
        assert_eq!(settlement.amount.as_deref(), Some("150000"));
        assert_eq!(settlement.network, "eip155:8453");
    }

    #[tokio::test]
    async fn x402_discovery_is_explicit_about_custom_funding_and_mpp_boundary() {
        let state = test_state(BountyNetwork::default());
        let document = x402_discovery(State(state)).await.0;

        assert_eq!(document["x402Version"], X402_VERSION);
        assert_eq!(document["resources"][0]["scheme"], AGENT_BOUNTY_FUND_SCHEME);
        assert_eq!(document["resources"][0]["genericExactCompatible"], false);
        assert_eq!(document["hostedRelay"]["enabled"], false);
        assert_eq!(document["hostedRelay"]["minUsdcBaseUnits"], "100000");
        assert_eq!(document["hostedRelay"]["maxDailyAttempts"], 100);
        assert_eq!(
            document["hostedRelay"]["maxDailyAttemptsPerContributor"],
            10
        );
        assert!(document["hostedRelay"]["statusUrlTemplate"]
            .as_str()
            .unwrap()
            .contains("/v1/x402/base/relays/{relay_id}"));
        assert_eq!(document["mpp"]["status"], "planned");
        assert!(document["safety"]["standardExactToBountyContract"]
            .as_str()
            .unwrap()
            .contains("FundingAdded"));
    }

    #[tokio::test]
    async fn agent_paid_status_endpoint_summarizes_solver_receivables() {
        let (network, _bounty, _proof) = completed_simulated_bounty().await;
        let solver_id = network
            .settlements
            .values()
            .flat_map(|settlement| &settlement.payout_intents)
            .find(|intent| intent.amount.currency == "usdc")
            .expect("solver payout intent exists")
            .recipient_agent_id;
        let state = test_state(network);

        let response = agent_paid_status(State(state), Path(solver_id))
            .await
            .unwrap()
            .0;
        let post_value = &response["post_value_loop"];
        assert_eq!(post_value["trigger"], "verified_completion");
        assert!(post_value["self_interest"]
            .as_str()
            .unwrap()
            .contains("more and higher-value funded bounties"));
        assert!(post_value["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["kind"] == "tell_your_human"));
        let response: app::AgentPayoutStatusResponse = serde_json::from_value(response).unwrap();

        assert_eq!(response.agent.id, solver_id);
        assert_eq!(response.payouts.len(), 1);
        assert_eq!(response.payouts[0].status, PayoutStatus::Paid);
        assert_eq!(response.totals[0].currency, "usdc");
        assert_eq!(response.totals[0].pending_minor, 0);
        assert_eq!(response.totals[0].paid_minor, 1_000_000);
    }

    #[tokio::test]
    async fn simulated_paid_status_uses_verified_completion_copy() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "simulated-solver".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Verify simulated distribution copy".to_string(),
                template_slug: "small-code-change".to_string(),
                amount_minor: 1_000,
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
        let artifact = "{\"simulated\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://tests/simulated.json".to_string(),
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
        let state = test_state(network);

        let response = agent_paid_status(State(state), Path(solver.id))
            .await
            .unwrap()
            .0;

        assert_eq!(
            response["post_value_loop"]["trigger"],
            "verified_completion"
        );
        assert!(!response["post_value_loop"]["value_statement"]
            .as_str()
            .unwrap()
            .contains("received a reconciled payout"));
    }

    #[tokio::test]
    async fn stripe_checkout_webhook_credits_platform_balance_once() {
        let organization_id = Uuid::new_v4();
        let state = test_state_with_unsigned_stripe_webhooks(BountyNetwork::default());
        let body = stripe_checkout_event_body("evt_paid", "cs_paid", organization_id);

        let first = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(body.clone()),
        )
        .await
        .unwrap()
        .0;

        assert!(!first.duplicate);
        assert_eq!(first.ledger_entries.len(), 1);
        assert_eq!(
            first.funding_credit.payment_event.status,
            PaymentEventStatus::Applied
        );

        let replay = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(body),
        )
        .await
        .unwrap()
        .0;

        assert!(replay.duplicate);
        assert!(replay.ledger_entries.is_empty());
        assert_eq!(
            replay.funding_credit.payment_event.status,
            PaymentEventStatus::IgnoredDuplicate
        );
        let network = state.network.lock().expect("state poisoned");
        assert_eq!(network.payment_events.len(), 1);
        assert_eq!(network.ledger.entries().len(), 1);
    }

    #[tokio::test]
    async fn stripe_checkout_webhook_rejects_unsigned_when_not_explicitly_allowed() {
        let organization_id = Uuid::new_v4();
        let state = test_state(BountyNetwork::default());
        let body =
            stripe_checkout_event_body("evt_unsigned_paid", "cs_unsigned_paid", organization_id);

        assert_eq!(
            reconcile_stripe_checkout_webhook(State(state), HeaderMap::new(), Bytes::from(body),)
                .await
                .unwrap_err(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn stripe_checkout_webhook_requires_valid_signature_when_secret_configured() {
        let organization_id = Uuid::new_v4();
        let secret = b"whsec_test";
        let state = test_state_with_stripe_webhook_secret(BountyNetwork::default(), secret);
        let body = stripe_checkout_event_body("evt_signed_paid", "cs_signed_paid", organization_id);

        assert_eq!(
            reconcile_stripe_checkout_webhook(
                State(state.clone()),
                HeaderMap::new(),
                Bytes::from(body.clone()),
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );

        let mut bad_headers = HeaderMap::new();
        bad_headers.insert("stripe-signature", "t=1700000000,v1=00".parse().unwrap());
        assert_eq!(
            reconcile_stripe_checkout_webhook(
                State(state.clone()),
                bad_headers,
                Bytes::from(body.clone()),
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );

        let mut signed_headers = HeaderMap::new();
        signed_headers.insert(
            "stripe-signature",
            stripe_signature_header(&body, secret).parse().unwrap(),
        );
        let signed = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            signed_headers,
            Bytes::from(body),
        )
        .await
        .unwrap()
        .0;

        assert!(!signed.duplicate);
        assert_eq!(signed.ledger_entries.len(), 1);
        assert_eq!(
            signed.funding_credit.payment_event.status,
            PaymentEventStatus::Applied
        );
    }

    #[tokio::test]
    async fn stripe_checkout_top_up_endpoint_plans_checkout_session() {
        let organization_id = Uuid::new_v4();
        let state = test_state(BountyNetwork::default());

        let intent = plan_stripe_checkout_top_up(
            State(state),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id,
                amount_minor: 5_000,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(intent.endpoint, "/v1/checkout/sessions");
        assert!(intent.idempotency_key.contains("checkout_top_up"));
        assert_eq!(intent.body["mode"], "payment");
        assert_eq!(
            intent.body["client_reference_id"],
            organization_id.to_string()
        );
        assert_eq!(
            intent.body["success_url"],
            "http://127.0.0.1:8080/stripe/success"
        );
        assert_eq!(
            intent.body["cancel_url"],
            "http://127.0.0.1:8080/stripe/cancel"
        );
        assert!(intent.body.get("payment_method_types").is_none());
        assert!(intent.body.get("payment_method_configuration").is_none());
    }

    #[tokio::test]
    async fn stripe_checkout_top_up_endpoint_applies_payment_method_configuration() {
        let organization_id = Uuid::new_v4();
        let state = test_state_with_stripe_payment_method_configuration(
            BountyNetwork::default(),
            "pmc_paypal_enabled",
        );

        let intent = plan_stripe_checkout_top_up(
            State(state),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id,
                amount_minor: 5_000,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(
            intent.body["payment_method_configuration"],
            "pmc_paypal_enabled"
        );
        assert!(intent.body.get("payment_method_types").is_none());
    }

    #[tokio::test]
    async fn stripe_checkout_top_up_endpoint_rejects_below_minimum() {
        let state = test_state(BountyNetwork::default());

        let error = plan_stripe_checkout_top_up(
            State(state),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id: Uuid::new_v4(),
                amount_minor: 49,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stripe_connect_account_endpoint_uses_accounts_v2() {
        let agent_id = Uuid::new_v4();

        let intent =
            plan_stripe_connect_account(Json(PlanStripeConnectAccountRequest { agent_id }))
                .await
                .unwrap()
                .0;

        assert_eq!(intent.request.endpoint, "/v2/core/accounts");
        assert_eq!(
            intent.request.body["metadata"]["agent_id"],
            agent_id.to_string()
        );
    }

    #[tokio::test]
    async fn live_stripe_checkout_endpoint_returns_execution_report() {
        let organization_id = Uuid::new_v4();
        let stripe_api_base_url = spawn_rpc_response(serde_json::json!({
            "id": "cs_test_live",
            "object": "checkout.session",
            "url": "https://checkout.stripe.com/c/pay/cs_test_live",
            "livemode": false
        }));
        let state = test_state_with_stripe_live(BountyNetwork::default(), stripe_api_base_url);

        let report = execute_stripe_checkout_top_up(
            State(state),
            HeaderMap::new(),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id,
                amount_minor: 5_000,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.status, 200);
        assert_eq!(report.stripe_id.as_deref(), Some("cs_test_live"));
        assert_eq!(
            report.url.as_deref(),
            Some("https://checkout.stripe.com/c/pay/cs_test_live")
        );
        assert_eq!(report.request.endpoint, "/v1/checkout/sessions");
    }

    #[tokio::test]
    async fn public_stripe_funding_intent_checkout_executes_bounty_checkout() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Debit card funded bounty".to_string(),
                template_slug: "small-code-change".to_string(),
                target_amount_minor: 5_000,
                currency: "usd".to_string(),
                funding_mode: domain::FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            })
            .unwrap();
        let funding_intent = network
            .create_funding_intent(
                CreateFundingIntentRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: Some(organization_id),
                    amount_minor: 5_000,
                    currency: "usd".to_string(),
                    rail: domain::PaymentRail::StripeFiat,
                    external_reference: Some("card-funding-test".to_string()),
                    stripe_success_url: Some("https://agentbounties.app/success.html".to_string()),
                    stripe_cancel_url: Some("https://agentbounties.app/cancel.html".to_string()),
                },
                "http://127.0.0.1:8080",
            )
            .unwrap()
            .intent;
        let stripe_api_base_url = spawn_rpc_response(serde_json::json!({
            "id": "cs_test_bounty",
            "object": "checkout.session",
            "url": "https://checkout.stripe.com/c/pay/cs_test_bounty",
            "livemode": false
        }));
        let state = test_state_with_stripe_public_checkout_and_payment_method_configuration(
            network,
            stripe_api_base_url,
            "pmc_paypal_enabled",
        );

        let report = execute_stripe_funding_intent_checkout(State(state), Path(funding_intent.id))
            .await
            .unwrap()
            .0;

        assert_eq!(report.status, 200);
        assert_eq!(report.request.endpoint, "/v1/checkout/sessions");
        assert_eq!(
            report.request.idempotency_key,
            format!("bounty_funding_intent:{}", funding_intent.id)
        );
        assert_eq!(
            report.request.body["success_url"],
            "https://agentbounties.app/success.html"
        );
        assert_eq!(
            report.request.body["cancel_url"],
            "https://agentbounties.app/cancel.html"
        );
        assert_eq!(
            report.request.body["metadata"]["bounty_id"],
            bounty.id.to_string()
        );
        assert_eq!(
            report.request.body["metadata"]["funding_intent_id"],
            funding_intent.id.to_string()
        );
        assert_eq!(
            report.request.body["payment_method_configuration"],
            "pmc_paypal_enabled"
        );
        assert!(report.request.body.get("payment_method_types").is_none());
        assert_eq!(
            report.url.as_deref(),
            Some("https://checkout.stripe.com/c/pay/cs_test_bounty")
        );
    }

    #[tokio::test]
    async fn public_stripe_funding_intent_checkout_is_disabled_by_default() {
        let state =
            test_state_with_stripe_live(BountyNetwork::default(), "http://127.0.0.1:9".to_string());

        let error = execute_stripe_funding_intent_checkout(State(state), Path(Uuid::new_v4()))
            .await
            .unwrap_err();

        assert_eq!(error, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn live_stripe_execution_is_disabled_by_default() {
        let state = test_state(BountyNetwork::default());

        let error = execute_stripe_connect_account(
            State(state),
            HeaderMap::new(),
            Json(PlanStripeConnectAccountRequest {
                agent_id: Uuid::new_v4(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn github_issue_bounty_plan_parses_valid_issue_form() {
        let plan = plan_github_issue_bounty(Json(PlanGitHubIssueBountyRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/7".to_string(),
            title: "[bounty]: Fix CI".to_string(),
            body: valid_github_issue_body(),
        }))
        .await
        .0;

        assert!(plan.ready);
        assert!(plan.error.is_none());
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
        let parsed = plan.parsed.expect("parsed bounty");
        assert_eq!(parsed.template_slug, "fix-ci-failure");
        assert_eq!(parsed.amount.amount, 10_000_000);
        assert_eq!(parsed.amount.currency, "usdc");
    }

    #[tokio::test]
    async fn github_issue_bounty_plan_returns_action_required_for_bad_issue_form() {
        let plan = plan_github_issue_bounty(Json(PlanGitHubIssueBountyRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/8".to_string(),
            title: "[bounty]: Missing fields".to_string(),
            body: "### Goal\nFix CI".to_string(),
        }))
        .await
        .0;

        assert!(!plan.ready);
        assert!(plan.parsed.is_none());
        assert!(plan.error.expect("error").contains("missing required"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[tokio::test]
    async fn public_pooled_bounty_cannot_overwrite_existing_unfunded_bounty() {
        let state = test_state(BountyNetwork::default());
        let existing = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Original public bounty".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;

        let error = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: Some(existing.id),
                idempotency_key: Some(format!(
                    "github-issue-sync:agent-bounties/example:{}",
                    existing.id
                )),
                title: "Overwrite public bounty".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 2_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::Simulated,
                privacy: PrivacyLevel::Private,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::BAD_REQUEST);
        let status = bounty_status(State(state), Path(existing.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status.bounty.title, "Original public bounty");
        assert_eq!(status.bounty.privacy, PrivacyLevel::Public);
        assert_eq!(status.bounty.amount.amount, 1_000);
    }

    #[tokio::test]
    async fn github_issue_api_sync_is_operator_gated_and_title_edit_stable() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");
        let denied = sync_github_issue_api_bounty(
            State(state.clone()),
            HeaderMap::new(),
            Json(github_issue_api_sync_request(
                115,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap_err();
        assert_eq!(denied, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let first = sync_github_issue_api_bounty(
            State(state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                115,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap()
        .0;

        let edited = sync_github_issue_api_bounty(
            State(state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                115,
                "[bounty]: Sync hosted GitHub issue bounty records",
                valid_github_issue_body_with_goal(
                    "Keep the same hosted bounty after an issue title edit.",
                ),
            )),
        )
        .await
        .unwrap()
        .0;

        let other_issue = sync_github_issue_api_bounty(
            State(state),
            headers,
            Json(github_issue_api_sync_request(
                116,
                "[bounty]: Sync hosted GitHub issue bounty records",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(edited.id, first.id);
        assert_ne!(other_issue.id, first.id);
        assert_eq!(
            edited.title,
            "[bounty]: Sync hosted GitHub issue bounty records"
        );
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn audience_audit_persists_idempotently_across_processes() {
        let database_url = postgres_test_database_url();
        let first_store = PostgresStore::connect(&database_url).await.unwrap();
        first_store.migrate().await.unwrap();
        let first_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            first_store,
        );
        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let unique = Uuid::new_v4();

        let member = upsert_audience_member(
            State(first_state.clone()),
            headers.clone(),
            Json(UpsertAudienceMemberRequest {
                provider: domain::AudienceProvider::Github,
                external_id: format!("github-user-{unique}"),
                handle: format!("audience-{unique}"),
                public_profile_url: Some(format!("https://github.com/audience-{unique}")),
                roles: vec![],
                observed_at: None,
            }),
        )
        .await
        .unwrap()
        .0;
        let stale_store = PostgresStore::connect(&database_url).await.unwrap();
        let stale_network = hydrate_network(&stale_store).await.unwrap();
        let stale_state =
            test_state_with_operator_token_and_store(stale_network, "secret-token", stale_store);
        let event_id = format!("pull-request:{unique}");
        let interaction = record_audience_interaction(
            State(first_state),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: event_id.clone(),
                kind: domain::AudienceInteractionKind::PullRequestOpened,
                public_url: Some(format!(
                    "https://github.com/NSPG13/agent-bounties/pull/{unique}"
                )),
                occurred_at: None,
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        let star = record_audience_interaction(
            State(stale_state.clone()),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: format!("star:{unique}"),
                kind: domain::AudienceInteractionKind::RepoStarred,
                public_url: Some("https://github.com/NSPG13/agent-bounties/stargazers".to_string()),
                occurred_at: None,
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_ne!(star.id, interaction.id);
        let conflicting_replay = record_audience_interaction(
            State(stale_state),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: event_id.clone(),
                kind: domain::AudienceInteractionKind::FundingSignaled,
                public_url: interaction.public_url.clone(),
                occurred_at: Some(interaction.occurred_at),
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflicting_replay, StatusCode::CONFLICT);

        let second_store = PostgresStore::connect(&database_url).await.unwrap();
        let second_network = hydrate_network(&second_store).await.unwrap();
        let second_state =
            test_state_with_operator_token_and_store(second_network, "secret-token", second_store);
        let replay = record_audience_interaction(
            State(second_state.clone()),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: event_id,
                kind: domain::AudienceInteractionKind::PullRequestOpened,
                public_url: interaction.public_url.clone(),
                occurred_at: Some(interaction.occurred_at),
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(replay.id, interaction.id);

        let persisted = list_audience_interactions(State(second_state.clone()), headers.clone())
            .await
            .unwrap()
            .0
            .into_iter()
            .filter(|candidate| candidate.audience_member_id == member.id)
            .collect::<Vec<_>>();
        assert_eq!(persisted.len(), 2);
        let persisted_member = list_audience_members(State(second_state.clone()), headers.clone())
            .await
            .unwrap()
            .0
            .into_iter()
            .find(|candidate| candidate.id == member.id)
            .unwrap();
        assert!(persisted_member
            .roles
            .contains(&domain::AudienceRole::Contributor));
        assert!(persisted_member
            .roles
            .contains(&domain::AudienceRole::Promoter));
        assert_eq!(
            persisted_member.lifecycle_stage,
            domain::AudienceLifecycleStage::Retained
        );
        let report = audience_report(State(second_state), headers)
            .await
            .unwrap()
            .0;
        assert!(report.total_members >= 1);
        assert!(report.total_interactions >= 1);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn github_issue_api_sync_postgres_rejects_stale_cross_process_activity() {
        let database_url = postgres_test_database_url();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let sync_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            store.clone(),
        );
        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let issue_number = (Uuid::new_v4().as_u128() % 1_000_000_000_000) as u64 + 1;

        let first = sync_github_issue_api_bounty(
            State(sync_state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap()
        .0;

        let edited = sync_github_issue_api_bounty(
            State(sync_state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync hosted GitHub issue bounty records",
                valid_github_issue_body_with_goal(
                    "Keep the same hosted bounty after an issue title edit.",
                ),
            )),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(edited.id, first.id);
        assert_eq!(
            edited.title,
            "[bounty]: Sync hosted GitHub issue bounty records"
        );

        let mcp_store = PostgresStore::connect(&database_url).await.unwrap();
        let mcp_network = hydrate_network(&mcp_store).await.unwrap();
        assert!(mcp_network.bounties.contains_key(&first.id));
        let mcp_state =
            test_state_with_operator_token_and_store(mcp_network, "secret-token", mcp_store);
        let funding_report = create_funding_intent(
            State(mcp_state),
            Path(first.id),
            Json(CreateFundingIntentRequest {
                bounty_id: first.id,
                contributor_agent_id: None,
                source_organization_id: Some(Uuid::new_v4()),
                amount_minor: first.amount.amount,
                currency: first.amount.currency.clone(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some(format!("stale-sync-{issue_number}")),
                stripe_success_url: None,
                stripe_cancel_url: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(funding_report.0.intent.bounty_id, first.id);
        assert_eq!(
            funding_report.0.intent.status,
            FundingIntentStatus::AwaitingEvidence
        );

        let rejected = sync_github_issue_api_bounty(
            State(sync_state.clone()),
            headers,
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Unsafe edit after funding activity",
                valid_github_issue_body_with_goal(
                    "This stale API process must not overwrite a funded row.",
                ),
            )),
        )
        .await
        .unwrap_err();
        assert_eq!(rejected, StatusCode::BAD_REQUEST);

        let persisted = store
            .list_bounties()
            .await
            .unwrap()
            .into_iter()
            .find(|bounty| bounty.id == first.id)
            .unwrap();
        assert_eq!(persisted.title, edited.title);
        assert_eq!(persisted.amount.amount, edited.amount.amount);
        assert_eq!(persisted.status, BountyStatus::Unfunded);

        let funding_intents = store
            .list_funding_intents()
            .await
            .unwrap()
            .into_iter()
            .filter(|intent| intent.bounty_id == first.id)
            .collect::<Vec<_>>();
        assert_eq!(funding_intents.len(), 1);

        let stale_status = bounty_status(State(sync_state), Path(first.id))
            .await
            .unwrap()
            .0;
        assert_eq!(stale_status.bounty.title, edited.title);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn github_issue_api_sync_postgres_serializes_concurrent_initial_sync() {
        let database_url = postgres_test_database_url();
        let first_store = PostgresStore::connect(&database_url).await.unwrap();
        first_store.migrate().await.unwrap();
        let second_store = PostgresStore::connect(&database_url).await.unwrap();
        let first_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            first_store.clone(),
        );
        let second_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            second_store,
        );
        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let issue_number = (Uuid::new_v4().as_u128() % 1_000_000_000_000) as u64 + 1;

        let first_sync = sync_github_issue_api_bounty(
            State(first_state),
            headers.clone(),
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        );
        let second_sync = sync_github_issue_api_bounty(
            State(second_state),
            headers,
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        );
        let (first_result, second_result) = tokio::join!(first_sync, second_sync);
        let first = first_result.unwrap().0;
        let second = second_result.unwrap().0;

        assert_eq!(first.id, second.id);
        assert_eq!(first.title, "[bounty]: Sync GitHub issue into API");
        assert_eq!(second.title, "[bounty]: Sync GitHub issue into API");

        let matching_bounties = first_store
            .list_bounties()
            .await
            .unwrap()
            .into_iter()
            .filter(|bounty| bounty.id == first.id)
            .collect::<Vec<_>>();
        assert_eq!(matching_bounties.len(), 1);
        assert_eq!(matching_bounties[0].status, BountyStatus::Unfunded);
        assert_eq!(matching_bounties[0].amount.amount, first.amount.amount);
    }

    #[tokio::test]
    async fn github_create_comment_plan_returns_review_only_wallet_handoff() {
        let plan = plan_github_create_comment(Json(PlanGitHubCreateCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/501".to_string(),
            title: "Fix canonical receipt reconciliation".to_string(),
            body: "The receipt worker drops a confirmed log after restart.".to_string(),
            comment_body: "/agent-bounty create 25 USDC".to_string(),
            contributor_login: Some("maintainer".to_string()),
            comment_id: Some("9001".to_string()),
            existing_idempotency_keys: vec![],
        }))
        .await
        .0;

        assert!(plan.ready);
        let signal = plan.signal.expect("create signal");
        assert_eq!(signal.draft.state, "review_required_not_published");
        assert!(signal.draft.acceptance_criteria.is_empty());
        assert!(signal.draft.draft_handoff_url.contains("from=github-issue"));
        assert!(!signal.draft.bounty_created);
        assert!(!signal.draft.canonical_funding_confirmed);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[test]
    fn social_rollout_counts_canonical_events_with_github_issue_provenance() {
        let now = Utc::now();
        let mut document: AutonomousBountyTermsDocument =
            serde_json::from_str(include_str!("../../../bounties/autonomous-v1/244.json")).unwrap();
        document.source_url =
            Some("https://github.com/agent-bounties/agent-bounties/issues/501".to_string());
        document.discovery_source = Some("GitHub /agent-bounty create".to_string());
        let terms = AutonomousBountyTermsRecord {
            terms_hash: format!("0x{}", "44".repeat(32)),
            policy_hash: format!("0x{}", "45".repeat(32)),
            acceptance_criteria_hash: format!("0x{}", "46".repeat(32)),
            benchmark_hash: format!("0x{}", "47".repeat(32)),
            evidence_schema_hash: format!("0x{}", "48".repeat(32)),
            creator_wallet: format!("0x{}", "33".repeat(20)),
            document,
            created_at: now,
        };
        let event = |kind, log_index| AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: format!("base-mainnet:501:{log_index}"),
            tx_hash: format!("0x{}", "55".repeat(32)),
            block_number: 501,
            log_index,
            contract_address: format!("0x{}", "22".repeat(20)),
            bounty_id: format!("0x{}", "11".repeat(32)),
            kind,
            data: serde_json::json!({}),
            occurred_at: now,
        };
        let item = AutonomousBountyFeedItem {
            bounty_id: format!("0x{}", "11".repeat(32)),
            bounty_contract: format!("0x{}", "22".repeat(20)),
            creator: format!("0x{}", "33".repeat(20)),
            status: "settled".to_string(),
            solver_reward: "2000000".to_string(),
            verifier_reward: "10000".to_string(),
            claim_bond: "10000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "2010000".to_string(),
            funded_amount: "2010000".to_string(),
            required_external_spend: "0".to_string(),
            gross_cash_margin: "2000000".to_string(),
            terms_hash: terms.terms_hash.clone(),
            terms: Some(terms),
            terms_valid: true,
            verification_mode: "signed_quorum".to_string(),
            verifier_module: None,
            verifier_set_hash: None,
            verifier_threshold: Some(2),
            runner_identifier: Some("sandboxed_regression_v1".to_string()),
            verification_ready: true,
            verification_readiness_reason: "ready".to_string(),
            validation_errors: vec![],
            events: vec![
                event(AutonomousBountyEventKind::BountyBecameClaimable, 1),
                event(AutonomousBountyEventKind::BountySettled, 2),
            ],
        };

        let mut legacy_github_item = item.clone();
        legacy_github_item
            .terms
            .as_mut()
            .unwrap()
            .document
            .discovery_source = Some("manual GitHub link".to_string());

        let mut ignored_non_github_item = item.clone();
        ignored_non_github_item
            .terms
            .as_mut()
            .unwrap()
            .document
            .source_url = Some("https://example.com/tasks/501".to_string());

        let evidence =
            github_issue_conversion_evidence(&[item, legacy_github_item, ignored_non_github_item]);
        assert!(evidence.evidence_available);
        assert_eq!(evidence.github_originated_canonical_funded, 2);
        assert_eq!(evidence.github_originated_canonical_settled, 2);
    }

    #[test]
    fn neynar_signature_uses_raw_body_hmac_sha512_and_rejects_tampering() {
        let secret = b"neynar-webhook-secret";
        let body = br#"{"type":"cast.created","data":{"hash":"0x4242"}}"#;
        let mut mac = Hmac::<sha2::Sha512>::new_from_slice(secret).unwrap();
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());

        assert!(verify_neynar_signature(body, &signature, secret));
        assert!(!verify_neynar_signature(
            br#"{"type":"cast.created","data":{"hash":"0x2424"}}"#,
            &signature,
            secret
        ));
        assert!(!verify_neynar_signature(body, "not-hex", secret));
    }

    #[test]
    fn farcaster_bot_mention_matching_respects_token_boundaries() {
        assert!(text_mentions_bot(
            "@bountyboard /agent-bounty create 25 USDC ship it",
            "bountyboard"
        ));
        assert!(text_mentions_bot(
            "Please ask (@bountyboard), then create the draft",
            "bountyboard"
        ));
        assert!(!text_mentions_bot(
            "@bountyboard-scam /agent-bounty create 25 USDC",
            "bountyboard"
        ));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn neynar_webhook_persists_one_short_draft_and_one_reply_across_retries() {
        let database_url = postgres_test_database_url();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        seed_social_rollout_evidence(&store).await;

        let reply_hash = format!("0x{}", "24".repeat(20));
        let neynar_api_base = spawn_rpc_response(serde_json::json!({
            "cast": {"hash": reply_hash}
        }));
        let webhook_secret = b"neynar-webhook-secret";
        let mut state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "unused-operator-token",
            store,
        );
        Arc::get_mut(&mut state).unwrap().neynar_social =
            Some(Arc::new(NeynarSocialIngestionConfig {
                webhook_secret: webhook_secret.to_vec(),
                bot_fid: 12_345,
                bot_username: "bountyboard".to_string(),
                api_key: Some("test-neynar-key".to_string()),
                signer_uuid: Some("123e4567-e89b-42d3-a456-426614174000".to_string()),
                api_base_url: neynar_api_base,
                website_base_url: "https://agentbounties.app".to_string(),
                client: reqwest::Client::new(),
            }));

        let cast_hash = format!("0x{:040x}", Uuid::new_v4().as_u128());
        let body = serde_json::json!({
            "created_at": Utc::now().timestamp(),
            "type": "cast.created",
            "data": {
                "object": "cast",
                "hash": cast_hash,
                "author": {"fid": 42, "username": "requester"},
                "text": "@bountyboard\n/agent-bounty create 25 USDC\nimplement deterministic retries",
                "mentioned_profiles": [{"fid": 12345}]
            }
        })
        .to_string();
        let mut mac = Hmac::<sha2::Sha512>::new_from_slice(webhook_secret).unwrap();
        mac.update(body.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        let mut headers = HeaderMap::new();
        headers.insert("x-neynar-signature", signature.parse().unwrap());
        let previous_gate = std::env::var("AGENT_BOUNTIES_SOCIAL_MENTION_DRAFTS_ENABLED").ok();
        std::env::set_var("AGENT_BOUNTIES_SOCIAL_MENTION_DRAFTS_ENABLED", "true");

        let (first_status, Json(first)) = ingest_neynar_social_mention(
            State(state.clone()),
            headers.clone(),
            Bytes::from(body.clone()),
        )
        .await;
        assert_eq!(
            first_status,
            StatusCode::OK,
            "unexpected webhook result: {}",
            serde_json::to_string(&first).unwrap()
        );
        assert!(first.accepted);
        assert!(!first.duplicate);
        assert_eq!(first.status, "replied");
        assert_eq!(first.reply_cast_hash.as_deref(), Some(reply_hash.as_str()));
        let ingestion_id = first.ingestion_id.unwrap();
        let handoff = first.draft_handoff_url.unwrap();
        assert_eq!(
            handoff,
            format!(
                "https://agentbounties.app/post.html?from=social-mention&socialDraft={ingestion_id}"
            )
        );

        let (replay_status, Json(replay)) =
            ingest_neynar_social_mention(State(state.clone()), headers, Bytes::from(body)).await;
        assert_eq!(replay_status, StatusCode::OK);
        assert!(replay.accepted);
        assert!(replay.duplicate);
        assert_eq!(replay.ingestion_id, Some(ingestion_id));
        assert_eq!(replay.reply_cast_hash.as_deref(), Some(reply_hash.as_str()));

        let draft = get_social_mention_draft(State(state), Path(ingestion_id))
            .await
            .unwrap()
            .0;
        assert_eq!(draft.status, "replied");
        assert_eq!(draft.draft["state"], "review_required_not_published");
        assert_eq!(draft.draft["draft_handoff_url"], handoff);
        assert_eq!(draft.draft["bounty_created"], false);
        assert_eq!(draft.draft["canonical_funding_confirmed"], false);

        if let Some(value) = previous_gate {
            std::env::set_var("AGENT_BOUNTIES_SOCIAL_MENTION_DRAFTS_ENABLED", value);
        } else {
            std::env::remove_var("AGENT_BOUNTIES_SOCIAL_MENTION_DRAFTS_ENABLED");
        }
    }

    async fn seed_social_rollout_evidence(store: &PostgresStore) {
        let now = Utc::now();
        for index in 0_u64..3 {
            let mut document: AutonomousBountyTermsDocument =
                serde_json::from_str(include_str!("../../../bounties/autonomous-v1/244.json"))
                    .unwrap();
            document.source_url = Some(format!(
                "https://github.com/agent-bounties/agent-bounties/issues/{}",
                Uuid::new_v4().as_u128()
            ));
            let record = build_autonomous_bounty_terms_record(
                "0x884834E884d6E93462655A2820140aD03E6747bC",
                document,
                now,
            )
            .unwrap();
            store.upsert_autonomous_bounty_terms(&record).await.unwrap();
            let bounty_id = format!("0x{:064x}", Uuid::new_v4().as_u128());
            let bounty_contract = format!("0x{:040x}", Uuid::new_v4().as_u128());
            let contract_terms = &record.document.contract_terms;
            let events = [
                (
                    AutonomousBountyEventKind::CanonicalBountyCreated,
                    serde_json::json!({
                        "bounty_contract": bounty_contract,
                        "creator": record.creator_wallet,
                        "terms_hash": record.terms_hash,
                        "policy_hash": record.policy_hash,
                        "creation_nonce": contract_terms["creation_nonce"]
                    }),
                ),
                (
                    AutonomousBountyEventKind::CanonicalBountyTermsCommitted,
                    serde_json::json!({
                        "acceptance_criteria_hash": record.acceptance_criteria_hash,
                        "benchmark_hash": record.benchmark_hash,
                        "evidence_schema_hash": record.evidence_schema_hash
                    }),
                ),
                (
                    AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured,
                    serde_json::json!({
                        "solver_reward": contract_terms["solver_reward"]["amount"],
                        "verifier_reward": contract_terms["verifier_reward"]["amount"],
                        "claim_bond": contract_terms["claim_bond"]["amount"],
                        "target_amount": contract_terms["initial_funding"]["amount"],
                        "initial_funding": contract_terms["initial_funding"]["amount"],
                        "funding_deadline": contract_terms["funding_deadline"],
                        "claim_window_seconds": contract_terms["claim_window_seconds"],
                        "verification_window_seconds": contract_terms["verification_window_seconds"]
                    }),
                ),
                (
                    AutonomousBountyEventKind::CanonicalBountyVerificationConfigured,
                    serde_json::json!({
                        "verification_mode": 0,
                        "verifier_module": record.document.verification_policy["verifier_module"],
                        "verifier_reward_recipient": record.document.verification_policy["verifier_reward_recipient"],
                        "threshold": 1,
                        "verifier_set_hash": format!("0x{}", "00".repeat(32))
                    }),
                ),
                (
                    AutonomousBountyEventKind::BountyBecameClaimable,
                    serde_json::json!({"funded_amount": contract_terms["initial_funding"]["amount"]}),
                ),
                (
                    AutonomousBountyEventKind::BountySettled,
                    serde_json::json!({}),
                ),
            ];
            for (event_index, (kind, data)) in events.into_iter().enumerate() {
                if index == 2 && kind == AutonomousBountyEventKind::BountySettled {
                    continue;
                }
                store
                    .upsert_autonomous_bounty_event(
                        "base-mainnet",
                        &AutonomousBountyEvent {
                            id: Uuid::new_v4(),
                            log_key: format!("social-test:{}:{event_index}", Uuid::new_v4()),
                            tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
                            block_number: 1_000_000 + index,
                            log_index: u64::try_from(event_index).unwrap(),
                            contract_address: bounty_contract.clone(),
                            bounty_id: bounty_id.clone(),
                            kind,
                            data,
                            occurred_at: now,
                        },
                    )
                    .await
                    .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn github_funding_comment_plan_flags_operator_reconciliation() {
        let plan = plan_github_funding_comment(Json(PlanGitHubFundingCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Co-funding".to_string(),
            body: valid_github_issue_body(),
            comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow".to_string(),
            contributor_login: Some("solver-agent".to_string()),
            comment_id: Some("123".to_string()),
            funding_api_base_url: None,
            existing_idempotency_keys: vec![],
        }))
        .await
        .0;

        assert!(plan.ready);
        let signal = plan.signal.expect("funding signal");
        assert!(signal.requires_operator_reconciliation);
        assert_eq!(signal.amount.currency, "usdc");
        assert!(signal.funding_handoff_url.is_none());
        assert!(signal.idempotency_key.ends_with(":comment:123"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_funding_comment_plan_returns_stripe_handoff_url() {
        let plan = plan_github_funding_comment(Json(PlanGitHubFundingCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Co-funding".to_string(),
            body: valid_github_issue_body_with_funding_mode("StripeFiatLedger"),
            comment_body: "/agent-bounty fund 5 USD via StripeFiatLedger".to_string(),
            contributor_login: Some("human-funder".to_string()),
            comment_id: Some("124".to_string()),
            funding_api_base_url: Some("https://api.agentbounties.example".to_string()),
            existing_idempotency_keys: vec![],
        }))
        .await
        .0;

        assert!(plan.ready);
        let signal = plan.signal.expect("funding signal");
        let handoff = signal.funding_handoff_url.expect("handoff url");
        assert!(handoff.contains("https://agentbounties.app/funding.html"));
        assert!(handoff.contains("apiBaseUrl=https%3A%2F%2Fapi.agentbounties.example"));
        assert!(handoff.contains("rail=StripeFiat"));
        assert!(handoff.contains("externalReference=github-funding-comment%3A"));
        assert!(plan.check.text.contains("Stripe Checkout funding handoff"));
        assert!(plan
            .check
            .text
            .contains("verified Stripe webhook reconciliation"));
    }

    #[tokio::test]
    async fn github_funding_comment_plan_rejects_duplicate_signal() {
        let existing_key =
            "github-funding-comment:agent-bounties/agent-bounties:https://github.com/agent-bounties/agent-bounties/issues/20:comment:123";
        let plan = plan_github_funding_comment(Json(PlanGitHubFundingCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Co-funding".to_string(),
            body: valid_github_issue_body(),
            comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow".to_string(),
            contributor_login: None,
            comment_id: Some("123".to_string()),
            funding_api_base_url: None,
            existing_idempotency_keys: vec![existing_key.to_string()],
        }))
        .await
        .0;

        assert!(!plan.ready);
        assert!(plan.error.unwrap().contains("duplicate funding signal"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[tokio::test]
    async fn github_claim_comment_plan_reserves_progress_backed_claim() {
        let plan = plan_github_claim_comment(Json(PlanGitHubClaimCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_github_issue_body(),
            comment_body: "/agent-bounty claim\nPlan: add deterministic stale claim tests."
                .to_string(),
            contributor_login: Some("solver-agent".to_string()),
            comment_id: Some("456".to_string()),
            claim_age_minutes: Some(5),
            progress_signal_count: 0,
            active_claim_login: None,
        }))
        .await
        .0;

        assert!(plan.ready);
        let signal = plan.signal.expect("claim signal");
        assert!(!signal.settlement_authority);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_claim_comment_plan_rejects_templated_claim_without_progress() {
        let plan = plan_github_claim_comment(Json(PlanGitHubClaimCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_github_issue_body(),
            comment_body:
                "/agent-bounty claim\nI'm reviewing the codebase and will open a PR shortly."
                    .to_string(),
            contributor_login: Some("claim-bot".to_string()),
            comment_id: Some("457".to_string()),
            claim_age_minutes: Some(1),
            progress_signal_count: 0,
            active_claim_login: None,
        }))
        .await
        .0;

        assert!(!plan.ready);
        assert!(plan.signal.is_none());
        assert!(plan.error.unwrap().contains("concrete progress signal"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[tokio::test]
    async fn github_proof_comment_plan_returns_markdown_and_fingerprint() {
        let bounty_id = Uuid::new_v4();
        let plan = plan_github_proof_comment(Json(PlanGitHubProofCommentRequest {
            bounty_id,
            proof_url: "https://agentbounties.local/public/proofs/abc".to_string(),
            verifier_summary: "GitHub CI passed".to_string(),
            settlement_url: Some("https://basescan.org/tx/0xabc".to_string()),
        }))
        .await
        .0;

        assert_eq!(plan.comment.bounty_id, bounty_id);
        assert!(plan.markdown.contains("Proof:"));
        assert!(plan.markdown.contains("Settlement:"));
        assert_eq!(plan.fingerprint.len(), 64);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_proof_comment_plan_from_proof_uses_stored_public_proof() {
        let (network, bounty, proof) = completed_simulated_bounty().await;
        let state = test_state(network);
        let plan = plan_github_proof_comment_from_proof(
            State(state),
            Json(PlanGitHubProofCommentFromProofRequest {
                proof_id: proof.id,
                settlement_url: Some("https://basescan.org/tx/0xabc".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(plan.comment.bounty_id, bounty.id);
        assert_eq!(
            plan.comment.proof_url,
            format!("http://127.0.0.1:8080/public/proofs/{}", proof.id)
        );
        assert!(plan.comment.verifier_summary.contains("JsonSchema"));
        assert!(plan.markdown.contains("Settlement:"));
        assert_eq!(plan.fingerprint.len(), 64);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_proof_comment_plan_from_proof_rejects_private_proofs() {
        let (mut network, _bounty, mut proof) = completed_simulated_bounty().await;
        proof.privacy = PrivacyLevel::Private;
        network.proofs.insert(proof.id, proof.clone());
        let state = test_state(network);
        let error = plan_github_proof_comment_from_proof(
            State(state.clone()),
            Json(PlanGitHubProofCommentFromProofRequest {
                proof_id: proof.id,
                settlement_url: None,
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::NOT_FOUND);
        let public_error = public_proof_page(State(state), Path(proof.id))
            .await
            .unwrap_err();
        assert_eq!(public_error, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn discovery_endpoint_advertises_autonomous_protocol_only() {
        let state = test_state(BountyNetwork::default());
        let manifest = agent_bounties_discovery(State(state)).await.0;

        assert_eq!(
            manifest.schema,
            "https://agentbounties.org/schemas/discovery-manifest.v2.json"
        );
        assert_eq!(manifest.protocol["version"], "agent-bounties/autonomous-v1");
        assert_eq!(manifest.protocol["operator_settlement_signer"], false);
        assert_eq!(
            manifest.endpoints.autonomous_canonical_child_terms_plan,
            "http://127.0.0.1:8080/v1/base/autonomous-bounties/canonical-child-terms-plan"
        );
        assert_eq!(
            manifest
                .endpoints
                .autonomous_standing_meta_v2_child_preparation,
            "http://127.0.0.1:8080/v1/base/autonomous-bounties/standing-meta-v2-child-preparation"
        );
        assert_eq!(
            manifest.endpoints.autonomous_creation_plan,
            "http://127.0.0.1:8080/v1/base/autonomous-bounties/creation-plan"
        );
        assert_eq!(
            manifest.endpoints.autonomous_bounty_feed,
            "http://127.0.0.1:8080/v1/base/autonomous-bounties/feed"
        );
        assert!(manifest
            .agent_tools
            .iter()
            .any(|tool| tool == "plan_autonomous_canonical_child_terms"));
        assert!(manifest
            .agent_tools
            .iter()
            .any(|tool| tool == "prepare_standing_meta_v2_child"));
        assert!(manifest
            .agent_tools
            .iter()
            .any(|tool| tool == "plan_autonomous_bounty_submission"));
        assert!(manifest
            .agent_tools
            .iter()
            .all(|tool| !tool.starts_with("plan_base_")));
    }

    #[tokio::test]
    async fn canonical_child_terms_endpoint_matches_contract_vectors() {
        let plan = plan_autonomous_canonical_child_terms(Json(CanonicalChildBountyTermsRequest {
            parent_bounty_id: format!("0x{}", "ab".repeat(32)),
            parent_round: 1,
            parent_solver: "0x3333333333333333333333333333333333333333".to_string(),
            parent_solver_reward: Money::new(900_000, "usdc").unwrap(),
            child_acceptance_criteria: vec!["Produce the committed digital artifact.".to_string()],
            verifier_module: "0x4444444444444444444444444444444444444444".to_string(),
        }))
        .await
        .unwrap()
        .0;

        assert_eq!(
            plan.acceptance_criteria_hash,
            chain_base::keccak256_canonical_json(&serde_json::json!(plan.acceptance_criteria))
                .unwrap()
        );
        assert_eq!(plan.required_child_status, "settled");
        assert_eq!(plan.minimum_child_target.amount, 900_000);
    }

    #[tokio::test]
    async fn canonical_child_terms_endpoint_explains_rejected_canary_verifier() {
        let error = plan_autonomous_canonical_child_terms(Json(CanonicalChildBountyTermsRequest {
            parent_bounty_id: format!("0x{}", "ab".repeat(32)),
            parent_round: 1,
            parent_solver: "0x3333333333333333333333333333333333333333".to_string(),
            parent_solver_reward: Money::new(900_000, "usdc").unwrap(),
            child_acceptance_criteria: vec!["Produce the committed digital artifact.".to_string()],
            verifier_module: chain_base::BASE_MAINNET_LEADING_ZERO_WORK_VERIFIER.to_string(),
        }))
        .await
        .unwrap_err();

        let (status, Json(body)) = error;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error_code, "invalid_canonical_child_terms_plan");
        assert!(body
            .message
            .contains("leading-zero work canary cannot verify a canonical child task"));
        assert!(!body.retryable);
        assert!(body.next_action.contains("Do not create or fund"));
    }

    #[test]
    fn terms_publication_returns_actionable_semantic_error() {
        let mut document: AutonomousBountyTermsDocument =
            serde_json::from_str(include_str!("../../../bounties/autonomous-v1/244.json")).unwrap();
        document.benchmark = serde_json::json!({
            "engine": "github_ci",
            "required_checks": ["ci"]
        });

        let error = autonomous_terms_record(PublishAutonomousBountyTermsRequest {
            creator_wallet: "0x884834E884d6E93462655A2820140aD03E6747bC".to_string(),
            document,
        })
        .unwrap_err();

        let (status, Json(body)) = error;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error_code, "invalid_autonomous_bounty_terms");
        assert!(body
            .message
            .contains("known leading-zero verifier must use its exact 16-bit"));
        assert!(body.next_action.contains("before creating or funding"));
    }

    #[tokio::test]
    async fn live_money_readiness_endpoint_reports_non_secret_defaults() {
        let state = test_state(BountyNetwork::default());
        let report = live_money_readiness(
            State(state),
            Query(LiveMoneyReadinessQuery {
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.network, "Base");
        assert_eq!(report.network_chain_id, 8_453);
        assert_eq!(report.stripe_secret_key_mode, "unset");
        assert!(!report.stripe_payment_method_configuration_configured);
        assert_eq!(report.supplied_usdc_token_matches_native, Some(true));
        assert!(!report.live_money_ready);
        assert!(report
            .checks
            .iter()
            .any(|check| { check.name == "Autonomous bounty factory" && check.configured }));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "Stripe live-money execution gate"));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "Stripe Checkout payment-method configuration"));
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("pmc_paypal_enabled"));
    }

    #[test]
    fn agent_wallet_readiness_rate_limit_problem_is_retryable_and_redacted() {
        let (status, Json(problem)) =
            map_agent_wallet_readiness_error(ChainBaseError::RpcProviderError {
                code: -32_016,
                message: "over rate limit at https://credential.example".to_string(),
            });

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(problem["error"], "base_rpc_rate_limited");
        assert_eq!(problem["retryable"], true);
        assert!(!problem.to_string().contains("credential.example"));
        assert!(problem["next_action"]
            .as_str()
            .unwrap()
            .contains("do not create parallel retries"));
    }

    #[test]
    fn agent_wallet_readiness_invalid_bounty_problem_is_not_retryable() {
        let (status, Json(problem)) = map_agent_wallet_readiness_error(
            ChainBaseError::InvalidAddress("not canonical".to_string()),
        );

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(problem["error"], "invalid_readiness_request");
        assert_eq!(problem["retryable"], false);
        assert_eq!(problem["failed_transition"], "validate_request_or_bounty");
    }

    #[tokio::test]
    async fn live_money_readiness_endpoint_reports_payment_method_configuration_without_id() {
        let state = test_state_with_stripe_payment_method_configuration(
            BountyNetwork::default(),
            "pmc_paypal_enabled",
        );
        let report = live_money_readiness(
            State(state),
            Query(LiveMoneyReadinessQuery {
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        let json = serde_json::to_string(&report).unwrap();

        assert!(report.stripe_payment_method_configuration_configured);
        assert!(report.checks.iter().any(|check| {
            check.name == "Stripe Checkout payment-method configuration"
                && check.configured
                && check.env_vars == vec!["STRIPE_PAYMENT_METHOD_CONFIGURATION".to_string()]
        }));
        assert!(!json.contains("pmc_paypal_enabled"));
    }

    #[tokio::test]
    async fn live_money_readiness_endpoint_rejects_unknown_network() {
        let state = test_state(BountyNetwork::default());
        let error = live_money_readiness(
            State(state),
            Query(LiveMoneyReadinessQuery {
                network: Some("optimism".to_string()),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn api_docs_endpoint_points_to_openapi_json() {
        let html = api_docs().await.0;

        assert!(html.contains("/api-docs/openapi.json"));
        assert!(html.contains("/llms.txt"));
        assert!(html.contains("/schemas/discovery-manifest.v2.json"));
        assert!(html.contains("/.well-known/agent-bounties.json"));
    }

    #[test]
    fn mainnet_planner_uses_only_the_canonical_attested_deployment() {
        let expected = autonomous_planner_addresses(8_453, None, None).unwrap();
        assert_eq!(expected.0, CANONICAL_BASE_MAINNET_BOUNTY_FACTORY);
        assert_eq!(expected.1, CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION);

        let matching = autonomous_planner_addresses(
            8_453,
            Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_uppercase()),
            Some(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_string()),
        )
        .unwrap();
        assert_eq!(matching, expected);

        assert_eq!(
            autonomous_planner_addresses(
                8_453,
                Some("0x1111111111111111111111111111111111111111".to_string()),
                None,
            ),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
    }

    #[test]
    fn agent_claim_network_keys_match_indexer_storage_keys() {
        assert_eq!(canonical_base_network_key(8_453), Some("base-mainnet"));
        assert_eq!(canonical_base_network_key(84_532), Some("base-sepolia"));
        assert_eq!(canonical_base_network_key(1), None);
    }

    #[test]
    fn mainnet_readiness_uses_canonical_factory_and_rejects_drift() {
        assert_eq!(
            canonical_mainnet_factory(None, None).as_deref(),
            Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
        );
        assert_eq!(
            canonical_mainnet_factory(
                Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_uppercase()),
                Some(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_uppercase()),
            )
            .as_deref(),
            Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
        );
        assert_eq!(
            canonical_mainnet_factory(
                Some("0x1111111111111111111111111111111111111111".to_string()),
                None,
            ),
            None
        );
        assert_eq!(
            canonical_mainnet_factory(
                None,
                Some("0x2222222222222222222222222222222222222222".to_string()),
            ),
            None
        );
    }

    #[test]
    fn sepolia_planner_still_requires_explicit_addresses() {
        assert_eq!(
            autonomous_planner_addresses(84_532, None, None),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert!(autonomous_planner_addresses(
            84_532,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("0x2222222222222222222222222222222222222222".to_string()),
        )
        .is_ok());
    }

    #[test]
    fn opportunity_comment_identifier_validation_is_bounded() {
        assert_eq!(
            normalize_opportunity_comment_id("canonical:base-mainnet:0xabc").unwrap(),
            "canonical:base-mainnet:0xabc"
        );
        assert_eq!(
            normalize_opportunity_comment_id("bad/id"),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            normalize_opportunity_comment_id(""),
            Err(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            bounded_public_text("  useful context  ", 60).unwrap(),
            "useful context"
        );
        assert_eq!(
            bounded_public_text(&"x".repeat(501), 500),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn opportunity_feedback_is_bounded_and_wallet_material_is_redacted() {
        let request = OpportunityFeedbackRequest {
            stage: " Funding ".to_string(),
            discovery_source: Some("MCP scanner".to_string()),
            participation_reason: None,
            friction: Some("The canonical activation condition was unclear.".to_string()),
            recommendation: Some("Show a safe-block activation receipt.".to_string()),
            evidence_reference: Some("canonical:base-mainnet:0xabc".to_string()),
            wallet: Some("0x1111111111111111111111111111111111111111".to_string()),
            wallet_signature: Some(format!("0x{}", "22".repeat(65))),
        };
        let normalized = normalize_opportunity_feedback(&request).unwrap();
        assert_eq!(normalized.stage, "funding");
        let public =
            public_opportunity_feedback(Some(serde_json::to_value(normalized).unwrap())).unwrap();
        assert_eq!(public.evidence_status, "self_reported");
        assert_eq!(
            public.wallet_evidence_status.as_deref(),
            Some("supplied_unverified")
        );
        let serialized = serde_json::to_string(&public).unwrap();
        assert!(!serialized.contains("0x1111111111111111111111111111111111111111"));
        assert!(!serialized.contains(&"22".repeat(65)));
    }

    #[test]
    fn opportunity_feedback_rejects_invalid_stage_empty_direction_and_partial_wallet_proof() {
        let base = OpportunityFeedbackRequest {
            stage: "posting".to_string(),
            discovery_source: None,
            participation_reason: None,
            friction: Some("Funding sequence took too many steps.".to_string()),
            recommendation: None,
            evidence_reference: None,
            wallet: None,
            wallet_signature: None,
        };
        let mut invalid_stage = base.clone();
        invalid_stage.stage = "traffic".to_string();
        assert_eq!(
            normalize_opportunity_feedback(&invalid_stage),
            Err(StatusCode::BAD_REQUEST)
        );
        let mut empty = base.clone();
        empty.friction = None;
        assert_eq!(
            normalize_opportunity_feedback(&empty),
            Err(StatusCode::BAD_REQUEST)
        );
        let mut partial_wallet = base;
        partial_wallet.wallet = Some("0x1111111111111111111111111111111111111111".to_string());
        assert_eq!(
            normalize_opportunity_feedback(&partial_wallet),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn platform_metric_windows_use_exact_launch_and_first_month_boundaries() {
        let ended_at = parse_public_metrics_timestamp("2026-08-12T20:22:19Z").unwrap();
        let seven_days = platform_metric_window(Some("7d"), ended_at).unwrap();
        assert_eq!(
            seven_days.started_at,
            parse_public_metrics_timestamp("2026-08-05T20:22:19Z").unwrap()
        );
        assert_eq!(
            seven_days.previous_started_at,
            parse_public_metrics_timestamp("2026-07-29T20:22:19Z").unwrap()
        );
        assert_eq!(
            seven_days.launch_at.to_rfc3339(),
            "2026-07-08T20:22:19+00:00"
        );
        assert_eq!(
            seven_days.first_month_ended_at.to_rfc3339(),
            "2026-08-08T20:22:19+00:00"
        );

        let lifetime = platform_metric_window(Some("lifetime"), ended_at).unwrap();
        assert_eq!(lifetime.started_at, lifetime.launch_at);
        assert_eq!(lifetime.previous_started_at, lifetime.launch_at);
        assert_eq!(
            platform_metric_window(Some("30d"), ended_at),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    fn test_platform_demand_growth_stats() -> PlatformDemandGrowthStats {
        PlatformDemandGrowthStats {
            gmv_7d_base_units: "1000000".to_string(),
            gmv_28d_base_units: "1200000".to_string(),
            lifetime_gmv_base_units: "1200000".to_string(),
            new_poster_funder_wallets_28d: 2,
            active_poster_funder_wallets_28d: 4,
            repeat_poster_funder_wallets_28d: 1,
            non_operator_attributed_gmv_28d_base_units: "600000".to_string(),
            attributed_gmv_28d_base_units: "1200000".to_string(),
        }
    }

    #[test]
    fn public_metrics_policy_excludes_the_declared_operator_wallets() {
        let policy = public_metrics_policy().unwrap();
        assert_eq!(
            policy.maintainer_wallets,
            vec![
                "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
                "0x884834e884d6e93462655a2820140ad03e6747bc",
                "0xfb58949365e3a30fd62e86edb0daffccf4ef7477",
                "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
            ]
        );
    }

    #[test]
    fn platform_metric_history_uses_policy_exclusions_not_recovery_reservations() {
        let recovery_contract = "0x9999999999999999999999999999999999999999";
        let reservations =
            AutonomousBountyRecoveryReservations::parse_csv(Some(recovery_contract)).unwrap();
        assert!(reservations.contains(recovery_contract));

        let exclusions = historical_platform_metric_exclusions(&public_metrics_policy().unwrap());
        assert!(!exclusions
            .iter()
            .any(|contract| contract == recovery_contract));
        assert_eq!(exclusions.len(), 4);
    }

    #[test]
    fn platform_metrics_response_is_aggregate_only_and_preserves_payment_math() {
        let generated_at = parse_public_metrics_timestamp("2026-08-12T20:22:19Z").unwrap();
        let stats = PlatformMetricsStats {
            generated_at,
            identities: PlatformIdentityStats {
                selected: 4,
                previous: 2,
                latest_week: 4,
                previous_week: 2,
                first_month: 3,
                lifetime: 5,
                posters: 1,
                funders: 1,
                solvers: 2,
                verifiers: 1,
                commenters: 1,
                marketplace_wallets: 3,
                opportunity_comment_authors: 1,
            },
            payouts: PlatformPayoutStats {
                selected_total_base_units: "1250000".to_string(),
                previous_total_base_units: "100000".to_string(),
                first_month_total_base_units: "900000".to_string(),
                lifetime_total_base_units: "1350000".to_string(),
                selected_solver_base_units: "1000000".to_string(),
                selected_verifier_base_units: "200000".to_string(),
                selected_keeper_base_units: "0".to_string(),
                selected_bonus_base_units: "50000".to_string(),
                selected_settled_rounds: 1,
                previous_settled_rounds: 0,
                first_month_settled_rounds: 1,
                lifetime_settled_rounds: 1,
            },
            claim_cohort: PlatformClaimCohortStats {
                settled: 1,
                mature: 2,
                immature: 1,
            },
            daily: vec![PlatformDailyStats {
                day: "2026-08-12".to_string(),
                active_identities: 4,
                payout_base_units: "1250000".to_string(),
                settled_rounds: 1,
            }],
            coverage: PlatformMetricsCoverageStats {
                verified_canonical_events: 9,
                awaiting_block_time_events: 1,
                opportunity_comments: 2,
                latest_verified_event_at: Some(generated_at),
                latest_comment_at: Some(generated_at),
            },
        };
        let mut policy = public_metrics_policy().unwrap();
        policy.maintainer_github_logins = vec!["private-maintainer-login".to_string()];
        policy.maintainer_wallets = vec!["0x1111111111111111111111111111111111111111".to_string()];
        let response = platform_metrics_response(
            stats,
            test_platform_demand_growth_stats(),
            platform_metric_window(Some("7d"), generated_at).unwrap(),
            policy,
            None,
            None,
            None,
            PlatformCanonicalSourceFreshness {
                autonomous: true,
                open_competition: true,
                open_competition_v2: true,
            },
        )
        .unwrap();
        let json = serde_json::to_string(&response).unwrap();

        assert_eq!(
            response.schema_version,
            "agent-bounties/platform-metrics-v3"
        );
        assert_eq!(response.marketplace_payout_volume.selected.usdc, "1.250000");
        assert_eq!(
            response.marketplace_payout_volume.selected_keeper_pay.usdc,
            "0.000000"
        );
        assert_eq!(
            response.mature_claim_to_settlement.settlement_rate,
            Some(0.5)
        );
        assert_eq!(response.platform_active_identities.namespaces.len(), 2);
        assert_eq!(
            response.platform_active_identities.namespaces[0].active_identities,
            3
        );
        assert_eq!(
            response.platform_active_identities.namespaces[1].active_identities,
            1
        );
        assert_eq!(response.current_inventory.status, "unavailable");
        assert_eq!(response.demand_growth.gmv_usdc_7d.usdc, "1.000000");
        assert_eq!(
            response.demand_growth.repeat_poster_funder_rate_28d,
            Some(0.25)
        );
        assert_eq!(
            response.demand_growth.non_operator_funded_gmv_share_28d,
            Some(0.5)
        );
        assert_eq!(response.coverage.status, "partial");
        assert!(response.coverage.marketplace_indexers_fresh);
        assert!(!response.coverage.github_included);
        assert!(!json.contains("private-maintainer-login"));
        assert!(!json.contains("0x1111111111111111111111111111111111111111"));
        assert!(!json.contains("open_competition_v2_indexer_fresh"));
        assert!(!json.contains("open_competition_indexer_fresh"));
        assert!(!json.contains("autonomous_indexer_fresh"));
        assert!(!json.contains("standing_meta"));
        assert!(!json.contains("verified_open_competition_v2"));
    }

    #[test]
    fn platform_inventory_combines_all_fresh_sources_without_type_split() {
        let autonomous = AutonomousBountyInventorySummary {
            schema_version: "test".to_string(),
            network: "base-mainnet".to_string(),
            generated_at: "2026-08-12T20:00:00+00:00".to_string(),
            canonical_source: "test".to_string(),
            claimable_bounty_count: 2,
            verification_ready_bounty_count: 1,
            standing_meta_bounty_count: 1,
            funded_usdc_base_units: "3000000".to_string(),
            funded_usdc: "3.00".to_string(),
            solver_reward_usdc_base_units: "2400000".to_string(),
            solver_reward_usdc: "2.40".to_string(),
            verifier_reward_usdc_base_units: "600000".to_string(),
            verifier_reward_usdc: "0.60".to_string(),
            items: Vec::new(),
            evidence_boundary: "test".to_string(),
        };
        let competition = OpenCompetitionInventorySummary {
            generated_at: "2026-08-12T20:01:00+00:00".to_string(),
            ready_to_earn_count: 1,
            funded_usdc_base_units: "1200000".to_string(),
            solver_reward_usdc_base_units: "1000000".to_string(),
            verifier_reward_usdc_base_units: "200000".to_string(),
        };
        let competition_v2 = OpenCompetitionInventorySummary {
            generated_at: "2026-08-12T20:02:00+00:00".to_string(),
            ready_to_earn_count: 10,
            funded_usdc_base_units: "30400000".to_string(),
            solver_reward_usdc_base_units: "30000000".to_string(),
            verifier_reward_usdc_base_units: "0".to_string(),
        };
        let combined = platform_inventory_response(
            Some(autonomous.clone()),
            Some(competition),
            Some(competition_v2),
        )
        .unwrap();

        assert_eq!(combined.status, "ready");
        assert_eq!(combined.active_funded_opportunities, Some(13));
        assert_eq!(
            combined.available_funding_usdc.as_deref(),
            Some("34.600000")
        );
        assert_eq!(
            combined.available_solver_rewards_usdc.as_deref(),
            Some("33.400000")
        );
        let combined_json = serde_json::to_string(&combined).unwrap();
        assert!(!combined_json.contains("autonomous_claimable_bounties"));
        assert!(!combined_json.contains("open_competitions_ready_to_earn"));
        assert!(!combined_json.contains("open_competition_v2"));
        assert!(!combined_json.contains("standing_meta_bounties"));

        let partial = platform_inventory_response(Some(autonomous), None, None).unwrap();
        assert_eq!(partial.status, "partial");
        assert_eq!(partial.active_funded_opportunities, None);
        assert!(partial.available_funding_usdc.is_none());
    }

    #[test]
    fn platform_metrics_zero_mature_denominator_has_no_rate() {
        assert_eq!(
            platform_metrics_response(
                PlatformMetricsStats {
                    generated_at: parse_public_metrics_timestamp("2026-08-12T20:22:19Z").unwrap(),
                    identities: PlatformIdentityStats {
                        selected: 0,
                        previous: 0,
                        latest_week: 0,
                        previous_week: 0,
                        first_month: 0,
                        lifetime: 0,
                        posters: 0,
                        funders: 0,
                        solvers: 0,
                        verifiers: 0,
                        commenters: 0,
                        marketplace_wallets: 0,
                        opportunity_comment_authors: 0,
                    },
                    payouts: PlatformPayoutStats {
                        selected_total_base_units: "0".to_string(),
                        previous_total_base_units: "0".to_string(),
                        first_month_total_base_units: "0".to_string(),
                        lifetime_total_base_units: "0".to_string(),
                        selected_solver_base_units: "0".to_string(),
                        selected_verifier_base_units: "0".to_string(),
                        selected_keeper_base_units: "0".to_string(),
                        selected_bonus_base_units: "0".to_string(),
                        selected_settled_rounds: 0,
                        previous_settled_rounds: 0,
                        first_month_settled_rounds: 0,
                        lifetime_settled_rounds: 0,
                    },
                    claim_cohort: PlatformClaimCohortStats {
                        settled: 0,
                        mature: 0,
                        immature: 2,
                    },
                    daily: Vec::new(),
                    coverage: PlatformMetricsCoverageStats {
                        verified_canonical_events: 0,
                        awaiting_block_time_events: 0,
                        opportunity_comments: 0,
                        latest_verified_event_at: None,
                        latest_comment_at: None,
                    },
                },
                PlatformDemandGrowthStats {
                    gmv_7d_base_units: "0".to_string(),
                    gmv_28d_base_units: "0".to_string(),
                    lifetime_gmv_base_units: "0".to_string(),
                    new_poster_funder_wallets_28d: 0,
                    active_poster_funder_wallets_28d: 0,
                    repeat_poster_funder_wallets_28d: 0,
                    non_operator_attributed_gmv_28d_base_units: "0".to_string(),
                    attributed_gmv_28d_base_units: "0".to_string(),
                },
                platform_metric_window(
                    Some("7d"),
                    parse_public_metrics_timestamp("2026-08-12T20:22:19Z").unwrap(),
                )
                .unwrap(),
                public_metrics_policy().unwrap(),
                None,
                None,
                None,
                PlatformCanonicalSourceFreshness {
                    autonomous: true,
                    open_competition: true,
                    open_competition_v2: true,
                },
            )
            .unwrap()
            .mature_claim_to_settlement
            .settlement_rate,
            None
        );
    }

    #[tokio::test]
    async fn openapi_json_endpoint_contains_agent_router_path() {
        let document = openapi_json().await.0;
        let value = serde_json::to_value(document).unwrap();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/openapi-contract.json")).unwrap();
        assert_eq!(
            hash_artifact(&serde_json::to_string(&value).unwrap()),
            fixture["normalized_sha256"]
        );
        let paths = value["paths"].as_object().unwrap();

        assert!(paths.contains_key("/v1/route-blocked-goal"));
        assert!(paths.contains_key("/llms.txt"));
        assert!(paths.contains_key("/schemas/discovery-manifest.v2.json"));
        assert!(paths.contains_key("/v1/risk/policy"));
        assert!(paths.contains_key("/v1/readiness/live-money"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/verifiers"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/events"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/creation-preparation"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/authorized-creation-preparation"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/state"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/readiness"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/commit-preparation"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/reveal-preparation"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/status"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/bond-withdrawal-preparation"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/entrant-action-preparation"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/entrant-action-relays"));
        assert!(paths.contains_key("/v1/base/open-competition-v1/entrant-action-relays/{relay_id}"));
        assert!(
            paths.contains_key("/v1/base/autonomous-bounties/bounded-wallet-cancel-refund-plan")
        );
        assert!(paths.contains_key("/v1/cloud-agent/objective-plans"));
        assert!(
            value["paths"]["/v1/cloud-agent/objective-plans"]["post"]["responses"]
                .get("502")
                .is_some()
        );
        assert!(paths.contains_key("/v1/objectives/creation-plans"));
        assert!(paths.contains_key("/v1/objectives"));
        assert!(paths.contains_key("/v1/objectives/{id}"));
        assert!(paths.contains_key("/v1/objectives/{id}/action-plans"));
        assert!(paths.contains_key("/v1/objectives/{id}/actions"));
        assert!(paths.contains_key("/v1/objectives/{id}/reconcile"));
        assert!(paths.contains_key("/v1/opportunities"));
        assert!(paths.contains_key("/v1/github/bounty-discovery-v1"));
        assert!(paths.contains_key("/v1/opportunities/stream"));
        assert!(paths.contains_key("/v1/opportunities/{opportunity_id}/comments"));
        assert!(value["components"]["schemas"]
            .get("OpportunityFeedbackRequest")
            .is_some());
        assert!(value["components"]["schemas"]
            .get("PlatformDemandGrowthResponse")
            .is_some());
        assert!(paths.contains_key("/v1/opportunities/feed.rss"));
        assert!(paths.contains_key("/v1/opportunities/feed.atom"));
        assert!(paths.contains_key("/v1/opportunities/feed.json"));
        assert!(paths.contains_key("/v1/opportunities/conversion-funnel"));
        assert!(paths.contains_key("/v1/chatgpt/action-intents"));
        assert!(paths.contains_key("/v1/chatgpt/action-intents/{intent_id}"));
        assert!(paths.contains_key("/v1/chatgpt/action-intents/{intent_id}/observations"));
        assert!(paths.contains_key("/v1/analytics/events"));
        assert!(paths.contains_key("/v1/analytics/site"));
        assert!(paths.contains_key("/v1/metrics/platform"));
        assert!(paths.contains_key("/v1/discovery/subscriptions"));
        assert!(paths.contains_key("/v1/discovery/subscriptions/{id}"));
        assert!(paths.contains_key("/public/opportunities/{opportunity_id}/embed"));
        assert!(paths.contains_key("/public/opportunities/{opportunity_id}/embed.svg"));
        assert!(paths.contains_key("/public/opportunities/{opportunity_id}/embed.md"));
        assert!(paths.contains_key("/v1/base/autonomous-bounties/{bounty_contract}/analysis"));
        assert!(paths.contains_key("/v1/unfunded-bounties"));
        assert!(paths.contains_key("/v1/unfunded-bounties/{id}"));
        assert!(paths.contains_key("/v1/unfunded-bounties/{id}/solutions"));
        assert!(paths.contains_key("/v1/base/agent-wallet/readiness"));
        assert!(paths.contains_key("/v1/risk/events"));
        assert!(paths.contains_key("/v1/risk/reviews"));
        assert!(paths.contains_key("/v1/risk/bounty-approvals"));
        assert!(paths.contains_key("/v1/risk/payout-approvals"));
        assert!(paths.contains_key("/v1/risk/events/{id}/reject"));
        assert!(paths.contains_key("/v1/agents/{id}/paid-status"));
        assert!(paths.contains_key("/v1/contributor-contacts"));
        assert!(paths.contains_key("/v1/audience/members"));
        assert!(paths.contains_key("/v1/audience/interactions"));
        assert!(paths.contains_key("/v1/audience/discovery-responses"));
        assert!(paths.contains_key("/v1/audience/outreach-attempts"));
        assert!(paths.contains_key("/v1/audience/report"));
        assert!(paths.contains_key("/v1/capabilities/search"));
        assert!(paths.contains_key("/.well-known/x402.json"));
        assert!(paths.contains_key("/v1/x402/base/bounties/{bounty_contract}/funding"));
        assert!(paths.contains_key("/v1/x402/base/relays/{relay_id}"));
        for v2 in [
            "/v1/base/open-competition-v2-beta3/release",
            "/v1/base/open-competition-v2-beta3/profiles",
            "/v1/base/open-competition-v2-beta3/structured-artifact-profile",
            "/v1/base/open-competition-v2-beta3/validate",
            "/v1/base/open-competition-v2-beta3/creation-preparation",
            "/v1/base/open-competition-v2-beta3/funding-preparation",
            "/v1/base/open-competition-v2-beta3/inventory",
            "/v1/base/open-competition-v2-beta3/events",
            "/v1/base/open-competition-v2-beta3/proof-quotes",
            "/v1/base/open-competition-v2-beta3/proof-preparation",
            "/v1/base/open-competition-v2-beta3/proof-attribution",
            "/v1/base/open-competition-v2-beta3/action-preparation",
            "/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}",
            "/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/payment",
            "/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/relay-authorization",
        ] {
            assert!(paths.contains_key(v2), "missing {v2}");
        }
        assert!(paths.contains_key("/v1/base/broadcast-signed-transaction"));
        assert!(paths.contains_key("/v1/base/transaction-receipt"));
        for autonomous in [
            "/v1/base/autonomous-bounties/canonical-child-terms-plan",
            "/v1/base/autonomous-bounties/creation-plan",
            "/v1/base/autonomous-bounties/authorized-creation-plan",
            "/v1/base/autonomous-bounties/contribution-plan",
            "/v1/base/autonomous-bounties/authorized-contribution-plan",
            "/v1/base/autonomous-bounties/claims",
            "/v1/base/autonomous-bounties/claim-funnel",
            "/v1/base/autonomous-bounties/claim-plan",
            "/v1/base/autonomous-bounties/authorized-claim-plan",
            "/v1/base/autonomous-bounties/submission-plan",
            "/v1/base/autonomous-bounties/submission-preparation",
            "/v1/base/autonomous-bounties/submission-authorization-plan",
            "/v1/base/autonomous-bounties/verification-jobs",
            "/v1/base/autonomous-bounties/decode-events",
            "/v1/base/autonomous-bounties/events",
            "/v1/base/autonomous-bounties/terms",
            "/v1/base/autonomous-bounties/terms/{terms_hash}",
            "/v1/base/autonomous-bounties/feed",
        ] {
            assert!(paths.contains_key(autonomous), "missing {autonomous}");
        }
        for objective in [
            "/v1/objectives/creation-plans",
            "/v1/objectives",
            "/v1/objectives/{id}",
            "/v1/objectives/{id}/action-plans",
            "/v1/objectives/{id}/actions",
            "/v1/objectives/{id}/reconcile",
        ] {
            assert!(paths.contains_key(objective), "missing {objective}");
        }
        for retired in [
            "/v1/base/indexer-status",
            "/v1/base/escrow-events",
            "/v1/base/evm-logs",
            "/v1/base/log-query",
            "/v1/base/rpc-logs",
            "/v1/base/fetch-rpc-logs",
            "/v1/base/funding-plan",
            "/v1/base/release-queue",
            "/v1/base/release-plan",
            "/v1/base/refund-plan",
            "/v1/base/dispute-plan",
        ] {
            assert!(
                !paths.contains_key(retired),
                "retired path leaked: {retired}"
            );
        }
        assert!(paths.contains_key("/v1/stripe/live/checkout-top-ups"));
        assert!(paths.contains_key("/v1/stripe/live/funding-intents/{id}/checkout-session"));
        assert!(paths.contains_key("/v1/stripe/live/connect-accounts"));
        assert!(paths.contains_key("/v1/stripe/connect-snapshots"));
        assert!(paths.contains_key("/v1/stripe/checkout-webhooks"));
        assert!(paths.contains_key("/v1/bounties/{id}/funding-intents"));
        assert!(paths.contains_key("/v1/github/issue-bounty-plan"));
        assert!(paths.contains_key("/v1/github/issue-api-sync-plan"));
        assert!(paths.contains_key("/v1/github/issue-api-sync"));
        assert!(paths.contains_key("/v1/github/create-comment-plan"));
        assert!(paths.contains_key("/v1/github/funding-comment-plan"));
        assert!(paths.contains_key("/v1/github/claim-comment-plan"));
        assert!(paths.contains_key("/v1/social/mention-draft-plan"));
        assert!(paths.contains_key("/v1/social/mention-ingestion/readiness"));
        assert!(paths.contains_key("/v1/social/webhooks/neynar"));
        assert!(paths.contains_key("/v1/social/mention-drafts/{id}"));
        assert!(paths.contains_key("/v1/github/proof-comment-plan"));
        assert!(paths.contains_key("/v1/github/proof-comment-plan-from-proof"));
        assert!(paths.contains_key("/v1/evals/loops"));
        assert!(paths.contains_key("/v1/evals/runs"));

        let security_schemes = value["components"]["securitySchemes"]
            .as_object()
            .expect("security schemes");
        assert_eq!(
            security_schemes["operator_api_token"]["name"],
            OPERATOR_TOKEN_HEADER
        );
        assert_eq!(security_schemes["operator_api_token"]["in"], "header");
        assert_eq!(security_schemes["operator_bearer"]["scheme"], "bearer");

        for path in [
            "/v1/risk/bounty-approvals",
            "/v1/risk/payout-approvals",
            "/v1/risk/events/{id}/reject",
            "/v1/contributor-contacts",
            "/v1/base/broadcast-signed-transaction",
            "/v1/stripe/live/checkout-top-ups",
            "/v1/stripe/live/connect-accounts",
            "/v1/stripe/connect-snapshots",
            "/v1/github/issue-api-sync",
        ] {
            let security = paths[path]["post"]["security"].as_array().unwrap();
            assert!(
                security
                    .iter()
                    .any(|requirement| requirement.get("operator_api_token").is_some()),
                "{path} missing operator_api_token security"
            );
            assert!(
                security
                    .iter()
                    .any(|requirement| requirement.get("operator_bearer").is_some()),
                "{path} missing operator_bearer security"
            );
            assert!(paths[path]["post"]["responses"]["401"].is_object());
        }

        assert!(paths["/v1/base/transaction-receipt"]["post"]
            .get("security")
            .is_none());
        assert!(paths["/v1/base/transaction-receipt"]["post"]["responses"]
            .get("401")
            .is_none());

        assert!(
            paths["/v1/stripe/live/funding-intents/{id}/checkout-session"]["post"]
                .get("security")
                .is_none(),
            "Public funder Checkout must not require operator auth"
        );
        assert!(
            paths["/v1/stripe/live/funding-intents/{id}/checkout-session"]["post"]["responses"]
                ["503"]
                .is_object()
        );
        assert!(
            paths["/v1/stripe/checkout-webhooks"]["post"]
                .get("security")
                .is_none(),
            "Stripe checkout webhook must remain callable by Stripe without operator auth"
        );
        assert!(paths["/v1/stripe/checkout-webhooks"]["post"]["responses"]["503"].is_object());
    }

    #[tokio::test]
    async fn contributor_contacts_are_operator_gated() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");
        let denied = upsert_contributor_contact(
            State(state.clone()),
            HeaderMap::new(),
            Json(UpsertContributorContactRequest {
                github_login: "qilu13".to_string(),
                email: None,
                payout_wallet: Some("0x1111111111111111111111111111111111111111".to_string()),
                associated_prs: vec!["#24".to_string()],
                contact_consent: false,
                wallet_consent: true,
                outreach_allowed: false,
                source: Some("github-comment-opt-in".to_string()),
                notes: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(denied, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let contact = upsert_contributor_contact(
            State(state.clone()),
            headers.clone(),
            Json(UpsertContributorContactRequest {
                github_login: "qilu13".to_string(),
                email: None,
                payout_wallet: Some("0x1111111111111111111111111111111111111111".to_string()),
                associated_prs: vec!["#24".to_string()],
                contact_consent: false,
                wallet_consent: true,
                outreach_allowed: false,
                source: Some("github-comment-opt-in".to_string()),
                notes: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(contact.github_login, "qilu13");
        assert!(contact.wallet_consent);
        let contacts = list_contributor_contacts(State(state), headers)
            .await
            .unwrap()
            .0;
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].associated_prs, vec!["#24".to_string()]);
    }

    #[tokio::test]
    async fn audience_audit_is_operator_gated_and_reports_public_attribution() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");
        let request = UpsertAudienceMemberRequest {
            provider: domain::AudienceProvider::Github,
            external_id: "U_123".to_string(),
            handle: "nexicturbo".to_string(),
            public_profile_url: Some("https://github.com/nexicturbo".to_string()),
            roles: vec![],
            observed_at: None,
        };
        let denied = upsert_audience_member(
            State(state.clone()),
            HeaderMap::new(),
            Json(request.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(denied, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let member = upsert_audience_member(State(state.clone()), headers.clone(), Json(request))
            .await
            .unwrap()
            .0;
        let _ = record_audience_interaction(
            State(state.clone()),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: "pull-request:138".to_string(),
                kind: domain::AudienceInteractionKind::PullRequestOpened,
                public_url: Some("https://github.com/NSPG13/agent-bounties/pull/138".to_string()),
                occurred_at: None,
                referrer_url: None,
                campaign: Some("github-bounty-label".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap();
        let _ = record_outreach_attempt(
            State(state.clone()),
            headers.clone(),
            Json(RecordOutreachAttemptRequest {
                audience_member_id: member.id,
                provider_event_id: "issue-comment:feedback:138".to_string(),
                channel: domain::OutreachChannel::GithubPublic,
                public_url: Some(
                    "https://github.com/NSPG13/agent-bounties/pull/138#issuecomment-1".to_string(),
                ),
                prompt_version: "distribution-v1".to_string(),
                status: domain::OutreachStatus::Responded,
                sent_at: None,
            }),
        )
        .await
        .unwrap();
        let _ = record_discovery_response(
            State(state.clone()),
            headers.clone(),
            Json(RecordDiscoveryResponseRequest {
                audience_member_id: member.id,
                interaction_id: None,
                provider_response_id: "pr-body:138".to_string(),
                public_source_url: Some(
                    "https://github.com/NSPG13/agent-bounties/pull/138".to_string(),
                ),
                found_via: "GitHub issue list".to_string(),
                motivation: "clear payout-integrity scope".to_string(),
                improvement_suggestion: "show durable payment evidence".to_string(),
                agent_or_tool: Some("coding agent".to_string()),
                private_storage_consent: false,
                captured_at: None,
            }),
        )
        .await
        .unwrap();

        let report = audience_report(State(state), headers).await.unwrap().0;
        assert_eq!(report.total_members, 1);
        assert_eq!(report.total_interactions, 1);
        assert_eq!(report.members_asked_for_discovery_feedback, 1);
        assert_eq!(report.members_with_discovery_responses, 1);
    }

    #[tokio::test]
    async fn eval_endpoints_record_local_run_history() {
        let state = test_state(BountyNetwork::default());

        let result = run_bountybench(State(state.clone())).await.unwrap().0;
        assert!(result.passed);

        let runs = list_eval_runs(State(state)).await.0;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].suite, "BountyBench/router-v0");
        assert!(runs[0].passed);
    }

    #[tokio::test]
    async fn risk_policy_endpoint_exposes_settlement_limits() {
        let policy = risk_policy().await.0;

        assert_eq!(policy.low_value_usdc_cap_minor, 10_000_000);
        assert_eq!(policy.low_value_usdc_cap_currency, "usdc");
        assert!(!policy.ai_judges_can_authorize_payment);
        assert!(policy
            .settlement_invariants
            .iter()
            .any(|rule| rule.contains("Stripe ledger credits")));
    }

    #[tokio::test]
    async fn risk_events_endpoint_lists_review_queue() {
        let mut network = BountyNetwork::default();
        let result = network.post_funded_bounty(PostBountyRequest {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 25_000_000,
            currency: "usdc".to_string(),
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let state = test_state(network);

        let events = list_risk_events(
            State(state),
            Query(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(10),
                ..RiskEventFilter::default()
            }),
        )
        .await
        .0;

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, domain::RiskAction::NeedsReview);
        assert!(events[0].reasons[0].contains("low-value cap"));
    }

    #[tokio::test]
    async fn risk_bounty_approval_endpoint_creates_funding_ready_bounty() {
        let mut network = BountyNetwork::default();
        let result = network.post_funded_bounty(PostBountyRequest {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 25_000_000,
            currency: "usdc".to_string(),
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let risk_event_id = network
            .list_risk_events(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .first()
            .unwrap()
            .id;
        let state = test_state(network);

        let approval = approve_risk_bounty(
            State(state.clone()),
            HeaderMap::new(),
            Json(ApproveRiskBountyRequest {
                risk_event_id,
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                operator_id: "operator-1".to_string(),
                note: "Approved after manual scope review".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(approval.bounty.status, BountyStatus::Unfunded);
        assert!(approval.bounty.terms_hash.is_some());
        assert_eq!(approval.review.outcome, domain::RiskReviewOutcome::Approved);
        let reviews = list_risk_reviews(State(state)).await.0;
        assert_eq!(reviews.len(), 1);
    }

    #[tokio::test]
    async fn risk_rejection_endpoint_records_review_without_bounty() {
        let mut network = BountyNetwork::default();
        let result = network.post_funded_bounty(PostBountyRequest {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 25_000_000,
            currency: "usdc".to_string(),
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let risk_event_id = network
            .list_risk_events(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .first()
            .unwrap()
            .id;
        let state = test_state(network);

        let review = reject_risk_event(
            State(state.clone()),
            HeaderMap::new(),
            Path(risk_event_id),
            Json(RejectRiskEventRequest {
                risk_event_id: Uuid::nil(),
                operator_id: "operator-1".to_string(),
                note: "Rejected until payer completes manual onboarding".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(review.outcome, domain::RiskReviewOutcome::Rejected);
        let network = state.network.lock().unwrap();
        assert!(network.bounties.is_empty());
    }

    #[tokio::test]
    async fn llms_txt_endpoint_points_agents_to_discovery_and_mcp() {
        let state = test_state(BountyNetwork::default());

        let text = llms_txt(State(state)).await;

        assert!(text.contains("# Agent Bounties"));
        assert!(text.contains("/.well-known/agent-bounties.json"));
        assert!(text.contains("http://127.0.0.1:8090/tools"));
        assert!(text.contains("Do not skip steps"));
        assert!(text.contains("get_solver_leaderboard"));
        assert!(text.contains("agent_native_claim"));
        assert!(text.contains("BountySettled"));
        assert!(!text.contains("createEscrow"));
    }

    #[tokio::test]
    async fn agent_card_is_served_with_cache_headers_and_etag() {
        let state = test_state(BountyNetwork::default());
        let response = agent_card(State(state)).await;
        let (parts, _body) = response.into_response().into_parts();
        let headers = parts.headers;
        assert!(headers.contains_key(axum::http::header::ETAG));
        assert!(headers.contains_key(axum::http::header::CACHE_CONTROL));
        let ct = headers.get(axum::http::header::CONTENT_TYPE);
        assert_eq!(
            ct.and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn leaderboard_reports_paid_only_for_the_ranked_winner() {
        let reference = Utc
            .with_ymd_and_hms(2026, 7, 17, 12, 0, 0)
            .single()
            .unwrap();
        let period = leaderboard_period(LeaderboardPeriodKind::Daily, reference);
        let ends_at = period.ends_at;
        let ranking = rank_solver_completions(
            period,
            [domain::CanonicalSolverCompletion {
                bounty_id: "bounty-1".to_string(),
                bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
                solver_wallet: "0x2222222222222222222222222222222222222222".to_string(),
                creator_wallet: "0x3333333333333333333333333333333333333333".to_string(),
                solver_reward_usdc_base_units: 2_000_000,
                occurred_at: reference,
                block_number: 42,
                log_index: 1,
                standing_meta_bounty: false,
            }],
        );
        let unconfigured = leaderboard_period_response(
            ranking.clone(),
            ends_at + chrono::Duration::hours(2),
            None,
            "not_configured",
            None,
        );
        assert_eq!(unconfigured.reward_payout_status, "reward_not_configured");

        let paid = leaderboard_period_response(
            ranking,
            ends_at + chrono::Duration::hours(2),
            Some("0x4444444444444444444444444444444444444444".to_string()),
            "funded",
            Some(SolverLeaderboardAwardSafeObservation {
                contract: "0x4444444444444444444444444444444444444444".to_string(),
                award_id: format!("0x{}", "55".repeat(32)),
                paid_winner: Some("0x2222222222222222222222222222222222222222".to_string()),
                safe_block_number: 50,
                safe_block_hash: format!("0x{}", "66".repeat(32)),
                safe_block_timestamp: u64::try_from(ends_at.timestamp()).unwrap(),
            }),
        );
        assert_eq!(paid.reward_payout_status, "paid");
        assert_eq!(
            paid.reward_paid_wallet.as_deref(),
            Some("0x2222222222222222222222222222222222222222")
        );
        assert_eq!(paid.reward_payout_observed_safe_block, Some(50));
    }

    #[tokio::test]
    async fn public_bounty_feed_excludes_private_bounties() {
        let mut network = BountyNetwork::default();
        let public = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix public CI".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let private = network
            .post_funded_bounty(PostBountyRequest {
                title: "Private ledger work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                amount_minor: 2_000_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Private,
            })
            .unwrap();
        let state = test_state(network);

        let feed = public_bounty_feed(State(state)).await.0;

        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].bounty_id, public.id.to_string());
        assert_ne!(feed[0].bounty_id, private.id.to_string());
        assert_eq!(
            feed[0].claim_url,
            format!("http://127.0.0.1:8080/v1/bounties/{}/claim", public.id)
        );
        assert_eq!(
            feed[0].public_url,
            format!("http://127.0.0.1:8080/public/bounties/{}", public.id)
        );
    }

    #[tokio::test]
    async fn opportunity_projection_keeps_payment_and_work_state_separate() {
        let mut network = BountyNetwork::default();
        let claimable = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix public API tests".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let private = network
            .post_funded_bounty(PostBountyRequest {
                title: "Private work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                amount_minor: 2_000_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Private,
            })
            .unwrap();
        let state = test_state(network);

        let response = list_opportunities(
            State(state),
            Query(OpportunityQuery {
                view: Some("ready_to_earn".to_string()),
                ..OpportunityQuery::default()
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(response.degraded);
        assert_eq!(response.applied_view.as_deref(), Some("ready_to_earn"));
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].source_id, claimable.id.to_string());
        assert_ne!(response.items[0].source_id, private.id.to_string());
        assert_eq!(response.items[0].work_state, "claimable");
        assert_eq!(response.items[0].payment_state, "escrowed");
        assert!(response.items[0].payment_committed);
        assert!(response.items[0]
            .discovery_factors
            .iter()
            .any(|factor| factor.contains("claimable+escrowed+verification_ready")));
    }

    #[tokio::test]
    async fn opportunity_projection_rejects_unknown_views() {
        let state = test_state(BountyNetwork::default());
        let error = list_opportunities(
            State(state),
            Query(OpportunityQuery {
                view: Some("agent_persona".to_string()),
                ..OpportunityQuery::default()
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn opportunity_embed_amounts_and_links_are_evidence_bound() {
        assert_eq!(decimal_amount("1000000", 6), "1");
        assert_eq!(decimal_amount("1250000", 6), "1.25");
        assert_eq!(decimal_amount("1", 6), "0.000001");
        assert!(safe_external_url("javascript:alert(1)").is_none());
        assert_eq!(percent_encode_path_segment("legacy:a/b"), "legacy%3Aa%2Fb");
    }

    #[tokio::test]
    async fn opportunity_embed_reuses_live_projection_state() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Build <safe> API docs".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                amount_minor: 1_250_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let state = test_state(network);
        let id = format!("legacy:{}", bounty.id);

        let html = opportunity_embed_page(
            State(state.clone()),
            Path(id.clone()),
            Query(OpportunityEmbedQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(html.status(), StatusCode::OK);
        assert!(html.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors *"));
        let html = axum::body::to_bytes(html.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(html.to_vec()).unwrap();
        assert!(html.contains("Work: claimable"));
        assert!(html.contains("Payment: escrowed"));
        assert!(html.contains("1.25 USDC"));
        assert!(html.contains("Build &lt;safe&gt; API docs"));

        let svg = opportunity_embed_svg(
            State(state.clone()),
            Path(id.clone()),
            Query(OpportunityEmbedQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(
            svg.headers()[header::CONTENT_TYPE],
            "image/svg+xml; charset=utf-8"
        );

        let markdown = opportunity_embed_markdown(
            State(state),
            Path(id),
            Query(OpportunityEmbedQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(
            markdown.headers()[header::CONTENT_TYPE],
            "text/markdown; charset=utf-8"
        );
    }

    #[test]
    fn discovery_subscription_filters_are_bounded_and_normalized() {
        let mut filters = DiscoverySubscriptionFilters {
            skills: vec![" Rust ".to_string(), "rust".to_string()],
            categories: vec!["engineering".to_string()],
            minimum_committed_reward: Some(domain::DiscoveryRewardFilter {
                amount: "1000000".to_string(),
                currency: " usdc ".to_string(),
                unit: " BASE_UNITS ".to_string(),
                decimals: 6,
            }),
            work_states: vec!["claimable".to_string()],
            payment_states: vec!["escrowed".to_string()],
            verification_methods: vec!["deterministic_module".to_string()],
            source_types: vec!["canonical_base".to_string()],
            deadline_within_hours: Some(72),
        };
        normalize_discovery_filters(&mut filters).unwrap();
        assert_eq!(filters.skills, vec!["Rust"]);
        let minimum = filters.minimum_committed_reward.unwrap();
        assert_eq!(minimum.currency, "USDC");
        assert_eq!(minimum.unit, "base_units");

        filters = DiscoverySubscriptionFilters {
            payment_states: vec!["funded-ish".to_string()],
            ..DiscoverySubscriptionFilters::default()
        };
        assert_eq!(
            normalize_discovery_filters(&mut filters),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn discovery_management_token_comparison_does_not_accept_prefixes() {
        assert!(constant_time_text_eq("abc123", "abc123"));
        assert!(!constant_time_text_eq("abc123", "abc124"));
        assert!(!constant_time_text_eq("abc123", "abc"));
    }

    #[test]
    fn conversion_correlation_accepts_only_exact_hosted_unfunded_urls() {
        let id = Uuid::new_v4();
        let base = "https://api.agentbounties.app";
        assert_eq!(
            unfunded_bounty_id_from_source(&format!("{base}/v1/unfunded-bounties/{id}"), base),
            Some(id)
        );
        assert_eq!(
            unfunded_bounty_id_from_source(
                &format!("https://evil.example/v1/unfunded-bounties/{id}"),
                base
            ),
            None
        );
        assert_eq!(
            unfunded_bounty_id_from_source(
                &format!("{base}/v1/unfunded-bounties/{id}?spoof=true"),
                base
            ),
            None
        );
    }

    #[test]
    fn conversion_response_never_infers_independent_active_agents() {
        let response = opportunity_conversion_response(
            OpportunityLifecycleStats {
                published: 10,
                solution_received: 6,
                funding_prepared: 4,
                wallet_signed_observed: 3,
                canonical_created: 3,
                funded: 2,
                claimed: 2,
                submitted: 1,
                settled: 1,
                average_seconds_to_first_solution: Some(120.0),
                median_seconds_to_first_solution: Some(90.0),
                average_seconds_creation_to_settlement: Some(3_600.0),
                canonical_created_in_window: 5,
                canonical_claimed_in_window: 4,
                canonical_settled_in_window: 3,
                unique_canonical_poster_wallets: 4,
                repeat_canonical_poster_wallets: 1,
                unique_paid_solver_wallets: 3,
                repeat_paid_solver_wallets: 1,
            },
            720,
            Utc::now() - ChronoDuration::hours(720),
            Utc::now(),
        );
        assert_eq!(response.stages.len(), 9);
        assert_eq!(response.rates[1].metric, "unfunded_to_funded_conversion");
        assert_eq!(response.rates[1].value, Some(0.2));
        assert_eq!(response.actors.independent_active_agents, None);
        assert!(!response.actors.independence_measurement_available);
        assert!(response
            .actors
            .evidence_boundary
            .contains("wallet is not proof"));
    }

    #[tokio::test]
    async fn public_funding_feed_lists_only_public_bounties_with_remaining_funding() {
        let state = test_state(BountyNetwork::default());
        let partial = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund public docs".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let _partial_funding = add_funding_contribution(
            State(state.clone()),
            Path(partial.id),
            Json(AddFundingContributionRequest {
                bounty_id: partial.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 400,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("public-funding-feed-partial".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        let stripe = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund public Stripe work".to_string(),
                template_slug: "payment-state-machine".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let private = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund private work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Private,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let funded = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Funded public work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let _funded_contribution = add_funding_contribution(
            State(state.clone()),
            Path(funded.id),
            Json(AddFundingContributionRequest {
                bounty_id: funded.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 1_000,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("public-funding-feed-funded".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        let feed = public_funding_feed(State(state.clone())).await.0;
        let ids = feed
            .iter()
            .map(|item| item.bounty_id.clone())
            .collect::<Vec<_>>();

        assert!(ids.contains(&partial.id.to_string()));
        assert!(ids.contains(&stripe.id.to_string()));
        assert!(!ids.contains(&private.id.to_string()));
        assert!(!ids.contains(&funded.id.to_string()));
        let partial_item = feed
            .iter()
            .find(|item| item.bounty_id == partial.id.to_string())
            .expect("partial public bounty should be in funding feed");
        assert_eq!(partial_item.funding_remaining_minor, 600);
        assert!(partial_item
            .funding_partitions
            .iter()
            .any(|partition| partition.remaining_minor == 600));

        let html = public_funding_feed_page(State(state)).await.0;
        assert!(html.contains("Fundable Agent Bounties"));
        assert!(html.contains("agent-bounty-funding-feed"));
        assert!(html.contains(&format!("/public/bounties/{}", partial.id)));
        assert!(html.contains(&format!("/v1/bounties/{}/funding-intents", partial.id)));
        assert!(!html.contains(&format!("/public/bounties/{}", private.id)));
        assert!(!html.contains(&format!("/public/bounties/{}", funded.id)));
    }

    #[tokio::test]
    async fn public_bounty_detail_exposes_agent_actions() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix public <CI>".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let state = test_state(network);

        let html = public_bounty_page(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;

        assert!(html.contains("Fix public &lt;CI&gt;"));
        assert!(html.contains("Funding State"));
        assert!(html.contains("Funding partitions"));
        assert!(html.contains("application/ld+json"));
        assert!(html.contains("agent-bounty-public-status"));
        assert!(html.contains("Machine status"));
        assert!(html.contains(r#"data-agent-action="claim""#));
        assert!(!html.contains("Add funding"));
        assert!(!html.contains(r#"rel="payment""#));
        assert!(html.contains(&format!("/public/bounties/{}", bounty.id)));
        assert!(html.contains(&format!("/v1/bounties/{}/claim", bounty.id)));
        assert!(!html.contains(&format!("/v1/bounties/{}/funding-contributions", bounty.id)));
    }

    #[tokio::test]
    async fn public_bounty_detail_exposes_cofunding_while_target_remains() {
        let state = test_state(BountyNetwork::default());
        let bounty = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund shared public work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let partial = add_funding_contribution(
            State(state.clone()),
            Path(bounty.id),
            Json(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 400_000,
                currency: "USDC".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("partial-public-page".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(partial.bounty.status, BountyStatus::Unfunded);
        assert_eq!(partial.funding_summary.remaining.amount, 600_000);

        let html = public_bounty_page(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;

        assert!(html.contains("partially funded"));
        assert!(html.contains("Co-funding command:"));
        assert!(html.contains(&format!(
            "/agent-bounty fund {} 0.6 USDC via Simulated",
            bounty.id
        )));
        assert!(html.contains(r#"rel="payment""#));
        assert!(html.contains(r#"data-agent-action="add_funding_evidence""#));
        assert!(!html.contains(r#"data-agent-action="create_funding_intent""#));
        assert!(html.contains(&format!("/v1/bounties/{}/funding-contributions", bounty.id)));
        assert!(!html.contains(r#"data-agent-action="claim""#));
    }

    #[tokio::test]
    async fn public_bounty_detail_hides_private_bounties() {
        let mut network = BountyNetwork::default();
        let private = network
            .post_funded_bounty(PostBountyRequest {
                title: "Private ledger work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                amount_minor: 2_000_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Private,
            })
            .unwrap();
        let state = test_state(network);

        let error = public_bounty_page(State(state), Path(private.id))
            .await
            .unwrap_err();

        assert_eq!(error, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn base_broadcast_signed_transaction_endpoint_returns_tx_hash() {
        let rpc_url = spawn_rpc_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 13,
            "result": format!("0x{}", "cc".repeat(32))
        }));
        let state = test_state_with_base_rpc(BountyNetwork::default(), rpc_url);

        let report = broadcast_base_signed_transaction(
            State(state),
            HeaderMap::new(),
            Json(BroadcastBaseSignedTransactionRequest {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.network.chain_id, 84_532);
        assert_eq!(report.request.method, "eth_sendRawTransaction");
        assert_eq!(report.request.params[0], "0x010203");
        assert_eq!(report.tx_hash, format!("0x{}", "cc".repeat(32)));
        assert!(report.next_step.contains("transaction-receipt"));
    }

    #[tokio::test]
    async fn operator_token_blocks_protected_api_calls_when_configured() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");

        let error = broadcast_base_signed_transaction(
            State(state.clone()),
            HeaderMap::new(),
            Json(BroadcastBaseSignedTransactionRequest {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret-token".parse().unwrap());
        let error = broadcast_base_signed_transaction(
            State(state),
            headers,
            Json(BroadcastBaseSignedTransactionRequest {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn legacy_verification_requires_configured_operator_authorization() {
        let bounty_id = Uuid::new_v4();
        let request = VerifySubmissionRequest {
            bounty_id: Uuid::nil(),
            submission_id: Uuid::new_v4(),
            expected_artifact_digest: "0xdeadbeef".to_string(),
            verifier_kind: Some(VerifierKind::JsonSchema),
            rubric: None,
            evidence: None,
            approved_risk_event_id: None,
        };

        let error = verify_submission(
            State(test_state(BountyNetwork::default())),
            Path(bounty_id),
            HeaderMap::new(),
            Json(request.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(error, StatusCode::SERVICE_UNAVAILABLE);

        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");
        let error = verify_submission(
            State(state.clone()),
            Path(bounty_id),
            HeaderMap::new(),
            Json(request.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(error, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret-token".parse().unwrap());
        let error = verify_submission(State(state), Path(bounty_id), headers, Json(request))
            .await
            .unwrap_err();
        assert_eq!(error, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn public_capability_search_finds_registered_solvers() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "capability-solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        network
            .register_capability(RegisterCapabilityRequest {
                agent_id: solver.id,
                class: CapabilityClass::Coding,
                template_slugs: vec!["small-code-change".to_string()],
                min_price_minor: 500_000,
                max_price_minor: 1_000_000,
                currency: "usdc".to_string(),
                latency_seconds: 600,
                supported_verifiers: vec![VerifierKind::JsonSchema],
            })
            .unwrap();
        let state = test_state(network);

        let feed = public_capability_feed(State(state.clone())).await.0;
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].agent_id, solver.id.to_string());
        assert_eq!(
            feed[0].agent_profile_url,
            format!("http://127.0.0.1:8080/public/agents/{}", solver.id)
        );

        let search = search_capabilities(
            State(state),
            Json(SearchCapabilitiesRequest {
                class: Some(CapabilityClass::Coding),
                template_slug: Some("small-code-change".to_string()),
                currency: Some("USDC".to_string()),
                max_price_minor: Some(600_000),
            }),
        )
        .await
        .0;

        assert_eq!(search.len(), 1);
        assert_eq!(search[0].agent_handle, "capability-solver");
    }

    #[tokio::test]
    async fn pooled_funding_endpoints_make_bounty_claimable_at_target() {
        let state = test_state(BountyNetwork::default());

        let bounty = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund shared docs work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;

        let partial = add_funding_contribution(
            State(state.clone()),
            Path(bounty.id),
            Json(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 400,
                currency: "USDC".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("first".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(partial.bounty.status, BountyStatus::Unfunded);
        assert_eq!(partial.funding_summary.remaining.amount, 600);

        let funded = add_funding_contribution(
            State(state.clone()),
            Path(bounty.id),
            Json(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 600,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("second".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(funded.bounty.status, BountyStatus::Claimable);
        assert!(funded.funding_summary.claimable);
        assert_eq!(funded.funding_summary.contribution_count, 2);

        let status = bounty_status(State(state.clone()), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status.funding_contributions.len(), 2);
        assert_eq!(status.funding_summary.applied.amount, 1_000);
        let feed = list_claimable_bounties(State(state)).await.0;
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].id, bounty.id);
    }

    #[tokio::test]
    async fn funding_intent_endpoint_waits_for_verified_stripe_webhook() {
        let state = test_state_with_unsigned_stripe_webhooks(BountyNetwork::default());
        let organization_id = Uuid::new_v4();
        let bounty = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund Stripe API intent".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;

        let intent = create_funding_intent(
            State(state.clone()),
            Path(bounty.id),
            Json(CreateFundingIntentRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(organization_id),
                amount_minor: 500,
                currency: "usd".to_string(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some("api-stripe-intent".to_string()),
                stripe_success_url: None,
                stripe_cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(intent.requires_reconciliation);
        assert_eq!(intent.funding_summary.applied.amount, 0);

        let status_before = bounty_status(State(state.clone()), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status_before.funding_intents.len(), 1);
        assert!(status_before.funding_contributions.is_empty());

        let event = serde_json::json!({
            "id": "evt_api_intent",
            "type": "checkout.session.completed",
            "payload": {
                "id": "cs_api_intent",
                "client_reference_id": organization_id.to_string(),
                "amount_total": 500,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_api_intent",
                "metadata": {
                    "bounty_id": bounty.id.to_string(),
                    "funding_intent_id": intent.intent.id.to_string()
                }
            }
        });
        let reconciliation = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(serde_json::to_vec(&event).unwrap()),
        )
        .await
        .unwrap()
        .0;
        assert!(reconciliation.funding_report.is_some());
        assert_eq!(reconciliation.ledger_entries.len(), 2);

        let status_after = bounty_status(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status_after.funding_contributions.len(), 1);
        assert_eq!(status_after.funding_summary.applied.amount, 500);
        assert!(status_after.funding_summary.claimable);
    }

    #[tokio::test]
    async fn public_verifier_profile_summarizes_verifier_results() {
        let (network, _bounty, _proof) = completed_simulated_bounty().await;
        let state = test_state(network);

        let html = public_verifier_profile(State(state), Path("JsonSchema".to_string()))
            .await
            .unwrap()
            .0;

        assert!(html.contains("JsonSchema Verifier"));
        assert!(html.contains("Total checks"));
        assert!(html.contains("<dt>Accepted</dt><dd>1</dd>"));
    }

    fn test_state(network: BountyNetwork) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_cloud_agent() -> Arc<CloudAgentService> {
        Arc::new(CloudAgentService::from_env().expect("disabled test cloud agent is valid"))
    }

    fn postgres_test_database_url() -> String {
        std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
            .expect("AGENT_BOUNTIES_TEST_DATABASE_URL must be set for ignored Postgres sync tests")
    }

    fn test_state_with_unsigned_stripe_webhooks(network: BountyNetwork) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: true,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_state_with_stripe_webhook_secret(network: BountyNetwork, secret: &[u8]) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: Some(secret.to_vec()),
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_state_with_operator_token(network: BountyNetwork, token: &str) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: Some(token.to_string()),
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_state_with_operator_token_and_store(
        network: BountyNetwork,
        token: &str,
        store: PostgresStore,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: Some(store),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: Some(token.to_string()),
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_state_with_base_rpc(
        network: BountyNetwork,
        base_sepolia_rpc_url: String,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig {
                base_sepolia: Some(base_sepolia_rpc_url),
                base_mainnet: None,
            },
            base_broadcast_enabled: true,
            operator_api_token: None,
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_state_with_stripe_live(
        network: BountyNetwork,
        stripe_api_base_url: String,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: Some("sk_test_mock".to_string()),
            stripe_live_execution_enabled: true,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url,
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_state_with_stripe_payment_method_configuration(
        network: BountyNetwork,
        payment_method_configuration: &str,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: Some(payment_method_configuration.to_string()),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn test_state_with_stripe_public_checkout_and_payment_method_configuration(
        network: BountyNetwork,
        stripe_api_base_url: String,
        payment_method_configuration: &str,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: Some("sk_test_mock".to_string()),
            stripe_live_execution_enabled: true,
            stripe_public_checkout_enabled: true,
            stripe_api_base_url,
            stripe_payment_method_configuration: Some(payment_method_configuration.to_string()),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            analytics_exclusion_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            bond_sponsor: BondSponsorConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
            cloud_agent: test_cloud_agent(),
            discovery_webhooks: None,
            neynar_social: None,
        })
    }

    fn spawn_rpc_response(response: serde_json::Value) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer).unwrap();
            let body = response.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}")
    }

    #[test]
    fn inventory_summary_uses_canonical_feed_amounts_without_static_counts() {
        let state = test_state(BountyNetwork::default());
        let summary = build_autonomous_inventory_summary(
            &state,
            "base-mainnet",
            vec![AutonomousBountyFeedItem {
                bounty_id: format!("0x{}", "11".repeat(32)),
                bounty_contract: format!("0x{}", "22".repeat(20)),
                creator: format!("0x{}", "33".repeat(20)),
                status: "claimable".to_string(),
                solver_reward: "900000".to_string(),
                verifier_reward: "100000".to_string(),
                claim_bond: "100000".to_string(),
                timeout_bond_pool: "0".to_string(),
                target_amount: "1000000".to_string(),
                funded_amount: "1000000".to_string(),
                required_external_spend: "0".to_string(),
                gross_cash_margin: "900000".to_string(),
                terms_hash: format!("0x{}", "44".repeat(32)),
                terms: None,
                terms_valid: true,
                verification_mode: "deterministic_module".to_string(),
                verifier_module: None,
                verifier_set_hash: None,
                verifier_threshold: Some(1),
                runner_identifier: Some("test_fixture".to_string()),
                verification_ready: true,
                verification_readiness_reason: "test fixture".to_string(),
                validation_errors: Vec::new(),
                events: Vec::new(),
            }],
        )
        .unwrap();

        assert_eq!(summary.claimable_bounty_count, 1);
        assert_eq!(summary.verification_ready_bounty_count, 1);
        assert_eq!(summary.funded_usdc, "1.00");
        assert_eq!(summary.solver_reward_usdc, "0.90");
        assert_eq!(summary.verifier_reward_usdc, "0.10");
        assert!(summary.canonical_source.contains("claimable_only=true"));
    }

    #[test]
    fn api_bind_addr_prefers_explicit_config_then_host_port() {
        assert_eq!(
            service_bind_addr(Some("0.0.0.0:9000"), Some("10000"), "127.0.0.1:8080"),
            "0.0.0.0:9000"
        );
        assert_eq!(
            service_bind_addr(Some(""), Some("10000"), "127.0.0.1:8080"),
            "0.0.0.0:10000"
        );
        assert_eq!(
            service_bind_addr(None, Some(" 10001 "), "127.0.0.1:8080"),
            "0.0.0.0:10001"
        );
        assert_eq!(
            service_bind_addr(None, None, "127.0.0.1:8080"),
            "127.0.0.1:8080"
        );
    }

    async fn completed_simulated_bounty() -> (BountyNetwork, Bounty, ProofRecord) {
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
                artifact_uri: "s3://api/artifact.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        let proof = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: expected_digest_for_body(artifact),
                verifier_kind: Some(domain::VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();
        (network, bounty, proof)
    }

    fn stripe_checkout_event_body(
        event_id: &str,
        session_id: &str,
        organization_id: Uuid,
    ) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": event_id,
            "type": "checkout.session.completed",
            "payload": {
                "id": session_id,
                "client_reference_id": organization_id.to_string(),
                "amount_total": 5_000,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_paid"
            }
        }))
        .unwrap()
    }

    fn stripe_signature_header(payload: &[u8], secret: &[u8]) -> String {
        let timestamp = Utc::now().timestamp();
        let mut signed_payload = timestamp.to_string().into_bytes();
        signed_payload.push(b'.');
        signed_payload.extend_from_slice(payload);
        let mut mac = TestHmacSha256::new_from_slice(secret).unwrap();
        mac.update(&signed_payload);
        format!(
            "t={},v1={}",
            timestamp,
            hex::encode(mac.finalize().into_bytes())
        )
    }

    fn valid_github_issue_body() -> String {
        valid_github_issue_body_with_funding_mode("StripeFiatLedger")
    }

    fn github_issue_api_sync_request(
        issue_number: u64,
        title: &str,
        body: String,
    ) -> PlanGitHubIssueApiSyncRequest {
        PlanGitHubIssueApiSyncRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: format!(
                "https://github.com/agent-bounties/agent-bounties/issues/{issue_number}"
            ),
            title: title.to_string(),
            body,
            api_base_url: Some("https://api.agentbounties.example".to_string()),
            existing_bounty_ids: vec![],
            hosted_api_error: None,
        }
    }

    fn valid_github_issue_body_with_goal(goal: &str) -> String {
        format!(
            r#"### Goal
{goal}

### Acceptance criteria
The test job is green and the patch explains the failure.

### Template
fix-ci-failure

### Suggested amount
10 USDC

### Funding mode
StripeFiatLedger
"#
        )
    }

    fn valid_github_issue_body_with_funding_mode(funding_mode: &str) -> String {
        format!(
            r#"### Goal
Fix the failing CI check.

### Acceptance criteria
The test job is green and the patch explains the failure.

### Template
fix-ci-failure

### Suggested amount
10 USDC

### Funding mode
{funding_mode}
"#
        )
    }
}
