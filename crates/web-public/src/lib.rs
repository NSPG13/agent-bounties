use domain::{
    Agent, AgentStatus, Bounty, BountyStatus, Capability, PrivacyLevel, ProofRecord,
    ReputationEvent, Settlement, VerifierResult,
};
use risk::{RiskPolicy, RiskPolicyDescriptor};
use serde::{Deserialize, Serialize};

const DISCOVERY_SCHEMA: &str = "https://agentbounties.org/schemas/discovery-manifest.v1.json";
const GITHUB_ISSUE_TEMPLATE_URL: &str =
    "https://github.com/NSPG13/agent-bounties/issues/new?template=paid-bounty.yml";
const AGENT_QUICKSTART_URL: &str =
    "https://github.com/NSPG13/agent-bounties/blob/main/docs/agent-quickstart.md";

#[derive(Debug, Clone)]
pub struct BountyTemplate {
    pub slug: &'static str,
    pub title: &'static str,
    pub verifier: &'static str,
    pub input: &'static str,
    pub output: &'static str,
}

#[derive(Debug, Clone)]
pub struct TemplateStats {
    pub accepted_count: usize,
    pub accepted_value_minor: i64,
    pub currency: String,
}

#[derive(Debug, Clone)]
pub struct VerifierProfileStats {
    pub total_checks: usize,
    pub accepted_count: usize,
    pub rejected_count: usize,
    pub needs_review_count: usize,
    pub average_confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryManifest {
    pub schema: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub open_source: bool,
    pub endpoints: DiscoveryEndpoints,
    pub agent_entrypoints: Vec<AgentEntrypoint>,
    pub payment_rails: Vec<PaymentRailDescriptor>,
    pub trust_tiers: Vec<TrustTierDescriptor>,
    pub templates: Vec<DiscoveryTemplate>,
    pub proof_surfaces: Vec<String>,
    pub risk_controls: Vec<String>,
    pub risk_policy: RiskPolicyDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryEndpoints {
    pub api_base: String,
    pub openapi_json: String,
    pub swagger_ui: String,
    pub mcp_tools: String,
    pub discovery: String,
    pub discovery_schema: String,
    pub llms_txt: String,
    pub agent_quickstart: String,
    pub public_bounties: String,
    pub public_bounty: String,
    pub public_funding: String,
    pub templates: String,
    pub pooled_bounties: String,
    pub bounty_funding_intents: String,
    pub bounty_funding_contributions: String,
    pub bounty_feed: String,
    pub funding_feed: String,
    pub capability_feed: String,
    pub eval_runs: String,
    pub risk_policy: String,
    pub risk_events: String,
    pub risk_reviews: String,
    pub risk_bounty_approvals: String,
    pub risk_payout_approvals: String,
    pub risk_event_rejections: String,
    pub agent_paid_status: String,
    pub base_log_query: String,
    pub base_escrow_events: String,
    pub base_rpc_logs: String,
    pub base_fetch_rpc_logs: String,
    pub base_broadcast_signed_transaction: String,
    pub base_transaction_receipt: String,
    pub base_funding_plan: String,
    pub base_release_queue: String,
    pub base_refund_plan: String,
    pub base_dispute_plan: String,
    pub stripe_checkout_top_ups: String,
    pub stripe_connect_accounts: String,
    pub stripe_connect_transfers: String,
    pub stripe_connect_snapshots: String,
    pub stripe_live_checkout_top_ups: String,
    pub stripe_live_connect_accounts: String,
    pub stripe_live_connect_transfers: String,
    pub stripe_transfer_events: String,
    pub github_issue_bounty_plan: String,
    pub github_funding_comment_plan: String,
    pub github_proof_comment_plan: String,
    pub github_proof_comment_from_proof_plan: String,
    pub github_issue_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEntrypoint {
    pub name: String,
    pub transport: String,
    pub endpoint: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentRailDescriptor {
    pub name: String,
    pub currency: String,
    pub status: String,
    pub settlement: String,
    pub funding_required_before_claim: bool,
    pub automatic_release_limit_minor: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustTierDescriptor {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryTemplate {
    pub slug: String,
    pub title: String,
    pub verifier: String,
    pub input: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicBountyFeedItem {
    pub bounty_id: String,
    pub title: String,
    pub template_slug: String,
    pub amount_minor: i64,
    pub currency: String,
    pub funding_mode: String,
    pub status: String,
    pub privacy: String,
    pub terms_hash: Option<String>,
    pub claim_url: String,
    pub status_url: String,
    pub public_url: String,
    pub template_url: String,
    pub funding_contribution_url: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicFundingFeedItem {
    pub bounty_id: String,
    pub title: String,
    pub template_slug: String,
    pub amount_minor: i64,
    pub currency: String,
    pub funding_mode: String,
    pub status: String,
    pub privacy: String,
    pub terms_hash: Option<String>,
    pub created_at: String,
    pub claimable: bool,
    pub funding_target_minor: i64,
    pub funding_applied_minor: i64,
    pub funding_remaining_minor: i64,
    pub contribution_count: usize,
    pub public_url: String,
    pub status_url: String,
    pub template_url: String,
    pub funding_intent_url: String,
    pub funding_contribution_url: String,
    pub funding_partitions: Vec<PublicFundingPartition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicBountyPage {
    pub bounty_id: String,
    pub title: String,
    pub template_slug: String,
    pub amount_minor: i64,
    pub currency: String,
    pub funding_mode: String,
    pub privacy: String,
    pub status: String,
    pub terms_hash: Option<String>,
    pub created_at: String,
    pub verification_type: String,
    pub claimable: bool,
    pub funding_target_minor: i64,
    pub funding_applied_minor: i64,
    pub funding_remaining_minor: i64,
    pub contribution_count: usize,
    pub public_url: String,
    pub claim_url: String,
    pub status_url: String,
    pub template_url: String,
    pub funding_contribution_url: String,
    pub proof_urls: Vec<String>,
    pub funding_partitions: Vec<PublicFundingPartition>,
    pub verifier_result_links: Vec<PublicBountyRecordLink>,
    pub settlement_links: Vec<PublicBountyRecordLink>,
    pub template_signal_links: Vec<PublicBountyRecordLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicFundingPartition {
    pub rail: String,
    pub target_minor: i64,
    pub confirmed_minor: i64,
    pub remaining_minor: i64,
    pub currency: String,
    pub contribution_count: usize,
    pub escrow_count: usize,
    pub claimable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicBountyRecordLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicBountyNextAction {
    pub kind: String,
    pub label: String,
    pub href: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicCapabilityFeedItem {
    pub capability_id: String,
    pub agent_id: String,
    pub agent_handle: String,
    pub class: String,
    pub template_slugs: Vec<String>,
    pub min_price_minor: i64,
    pub max_price_minor: i64,
    pub currency: String,
    pub latency_seconds: u64,
    pub supported_verifiers: Vec<String>,
    pub reputation_score: i32,
    pub accepted_bounties: usize,
    pub paid_minor: i64,
    pub agent_profile_url: String,
    pub request_quotes_url: String,
}

pub fn discovery_manifest(api_base_url: &str, mcp_base_url: &str) -> DiscoveryManifest {
    let api = normalize_base_url(api_base_url);
    let mcp = normalize_base_url(mcp_base_url);
    let risk_policy = RiskPolicy::default().descriptor();
    let low_value_usdc_cap_minor = risk_policy.low_value_usdc_cap_minor;
    DiscoveryManifest {
        schema: DISCOVERY_SCHEMA.to_string(),
        name: "Agent Bounties".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description:
            "Open-source payment-first network where AI agents request help, complete verified digital work, and get paid."
                .to_string(),
        open_source: true,
        endpoints: DiscoveryEndpoints {
            api_base: api.clone(),
            openapi_json: format!("{api}/api-docs/openapi.json"),
            swagger_ui: format!("{api}/docs"),
            mcp_tools: format!("{mcp}/tools"),
            discovery: format!("{api}/.well-known/agent-bounties.json"),
            discovery_schema: format!("{api}/schemas/discovery-manifest.v1.json"),
            llms_txt: format!("{api}/llms.txt"),
            agent_quickstart: AGENT_QUICKSTART_URL.to_string(),
            public_bounties: format!("{api}/public/bounties"),
            public_bounty: format!("{api}/public/bounties/{{bounty_id}}"),
            public_funding: format!("{api}/public/funding"),
            templates: format!("{api}/public/templates"),
            pooled_bounties: format!("{api}/v1/bounties/pooled"),
            bounty_funding_intents: format!("{api}/v1/bounties/{{bounty_id}}/funding-intents"),
            bounty_funding_contributions: format!("{api}/v1/bounties/{{bounty_id}}/funding-contributions"),
            bounty_feed: format!("{api}/v1/bounties/feed"),
            funding_feed: format!("{api}/v1/bounties/funding-feed"),
            capability_feed: format!("{api}/v1/capabilities/feed"),
            eval_runs: format!("{api}/v1/evals/runs"),
            risk_policy: format!("{api}/v1/risk/policy"),
            risk_events: format!("{api}/v1/risk/events"),
            risk_reviews: format!("{api}/v1/risk/reviews"),
            risk_bounty_approvals: format!("{api}/v1/risk/bounty-approvals"),
            risk_payout_approvals: format!("{api}/v1/risk/payout-approvals"),
            risk_event_rejections: format!("{api}/v1/risk/events/{{risk_event_id}}/reject"),
            agent_paid_status: format!("{api}/v1/agents/{{agent_id}}/paid-status"),
            base_log_query: format!("{api}/v1/base/log-query"),
            base_escrow_events: format!("{api}/v1/base/escrow-events"),
            base_rpc_logs: format!("{api}/v1/base/rpc-logs"),
            base_fetch_rpc_logs: format!("{api}/v1/base/fetch-rpc-logs"),
            base_broadcast_signed_transaction: format!(
                "{api}/v1/base/broadcast-signed-transaction"
            ),
            base_transaction_receipt: format!("{api}/v1/base/transaction-receipt"),
            base_funding_plan: format!("{api}/v1/base/funding-plan"),
            base_release_queue: format!("{api}/v1/base/release-queue"),
            base_refund_plan: format!("{api}/v1/base/refund-plan"),
            base_dispute_plan: format!("{api}/v1/base/dispute-plan"),
            stripe_checkout_top_ups: format!("{api}/v1/stripe/checkout-top-ups"),
            stripe_connect_accounts: format!("{api}/v1/stripe/connect-accounts"),
            stripe_connect_transfers: format!("{api}/v1/stripe/connect-transfers"),
            stripe_connect_snapshots: format!("{api}/v1/stripe/connect-snapshots"),
            stripe_live_checkout_top_ups: format!("{api}/v1/stripe/live/checkout-top-ups"),
            stripe_live_connect_accounts: format!("{api}/v1/stripe/live/connect-accounts"),
            stripe_live_connect_transfers: format!("{api}/v1/stripe/live/connect-transfers"),
            stripe_transfer_events: format!("{api}/v1/stripe/transfer-events"),
            github_issue_bounty_plan: format!("{api}/v1/github/issue-bounty-plan"),
            github_funding_comment_plan: format!("{api}/v1/github/funding-comment-plan"),
            github_proof_comment_plan: format!("{api}/v1/github/proof-comment-plan"),
            github_proof_comment_from_proof_plan: format!(
                "{api}/v1/github/proof-comment-plan-from-proof"
            ),
            github_issue_template: GITHUB_ISSUE_TEMPLATE_URL.to_string(),
        },
        agent_entrypoints: vec![
            AgentEntrypoint {
                name: "route_blocked_goal".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/route_blocked_goal"),
                description:
                    "First call for stuck agents; returns whether to solve directly, use a template, request quotes, post a bounty, or request verification."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "list_claimable_bounties".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/list_claimable_bounties"),
                description:
                    "List funded public bounty work that agents can claim immediately."
                    .to_string(),
            },
            AgentEntrypoint {
                name: "open_pooled_bounty".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/open_pooled_bounty"),
                description:
                    "Open an unfunded bounty target that multiple contributors can fund before claim."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "discover_fundable_bounties".to_string(),
                transport: "HTTP JSON plus public HTML".to_string(),
                endpoint: format!("{api}/v1/bounties/funding-feed"),
                description:
                    "Find public bounties that still need pooled, Stripe, Base, or mixed-rail funding before they become claimable."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "add_bounty_funding".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/add_bounty_funding"),
                description:
                    "Add an applied funding contribution to a pooled bounty and read the updated funding summary."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "search_capabilities".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/search_capabilities"),
                description:
                    "Search public solver capabilities by class, template, currency, or maximum price before requesting quotes."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "claim_bounty".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/claim_bounty"),
                description: "Claim funded work that is already eligible for a solver.".to_string(),
            },
            AgentEntrypoint {
                name: "get_paid_status".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/get_paid_status"),
                description: "Check whether an accepted bounty has reached a paid settlement state."
                    .to_string(),
            },
            AgentEntrypoint {
                name: "plan_base_funding".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/plan_base_funding"),
                description:
                    "Build unsigned Base USDC approval and escrow creation transactions for a posted bounty."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "reconcile_base_escrow_event".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/reconcile_base_escrow_event"),
                description:
                    "Operator/indexer entrypoint for applying a normalized Base escrow event; EscrowCreated is required before Base work becomes claimable."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "list_base_release_queue".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/list_base_release_queue"),
                description:
                    "List pending Base USDC releases and readiness errors before settlement signing."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "plan_stripe_connect_transfer".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/plan_stripe_connect_transfer"),
                description:
                    "Build a Stripe Connect transfer request for a specific fiat payout intent after deterministic verification."
                        .to_string(),
            },
            AgentEntrypoint {
                name: "reconcile_stripe_transfer_event".to_string(),
                transport: "MCP-compatible HTTP JSON".to_string(),
                endpoint: format!("{mcp}/tools/reconcile_stripe_transfer_event"),
                description:
                    "Operator entrypoint for applying transfer.created evidence so Stripe fiat payout intents can become paid."
                        .to_string(),
            },
        ],
        payment_rails: vec![
            PaymentRailDescriptor {
                name: "Base Sepolia USDC escrow".to_string(),
                currency: "usdc".to_string(),
                status: "open testnet".to_string(),
                settlement: "On-chain escrow create/release/refund events reconcile into the platform ledger.".to_string(),
                funding_required_before_claim: true,
                automatic_release_limit_minor: Some(low_value_usdc_cap_minor),
            },
            PaymentRailDescriptor {
                name: "Hosted low-value Base USDC".to_string(),
                currency: "usdc".to_string(),
                status: "gated mainnet beta".to_string(),
                settlement: "Low-value automatic release is capped; higher-value work requires review.".to_string(),
                funding_required_before_claim: true,
                automatic_release_limit_minor: Some(low_value_usdc_cap_minor),
            },
            PaymentRailDescriptor {
                name: "Stripe fiat ledger".to_string(),
                currency: "usd".to_string(),
                status: "onboarding and compliance gated".to_string(),
                settlement: "Checkout funds internal balances; Connect snapshots control eligibility; transfer.created evidence marks fiat payout intents paid.".to_string(),
                funding_required_before_claim: true,
                automatic_release_limit_minor: None,
            },
            PaymentRailDescriptor {
                name: "Mixed Stripe fiat plus Base USDC partitions".to_string(),
                currency: "multi".to_string(),
                status: "open source deterministic harness".to_string(),
                settlement: "One verified proof creates separate per-rail settlements; fiat and USDC are never netted into one balance.".to_string(),
                funding_required_before_claim: true,
                automatic_release_limit_minor: Some(low_value_usdc_cap_minor),
            },
        ],
        trust_tiers: vec![
            TrustTierDescriptor {
                name: "sandbox".to_string(),
                description: "Local simulated credits, deterministic verifiers, and no external money movement.".to_string(),
            },
            TrustTierDescriptor {
                name: "testnet".to_string(),
                description: "Base Sepolia escrow and payout rehearsal with public proof records.".to_string(),
            },
            TrustTierDescriptor {
                name: "low-value-usdc".to_string(),
                description: "Hosted low-value Base USDC payouts within risk limits.".to_string(),
            },
            TrustTierDescriptor {
                name: "fiat".to_string(),
                description: "Stripe-funded balances and Connect payout states behind eligibility gates.".to_string(),
            },
        ],
        templates: bounty_templates()
            .into_iter()
            .map(|template| DiscoveryTemplate {
                slug: template.slug.to_string(),
                title: template.title.to_string(),
                verifier: template.verifier.to_string(),
                input: template.input.to_string(),
                output: template.output.to_string(),
            })
            .collect(),
        proof_surfaces: vec![
            format!("{api}/public/proofs/{{proof_id}}"),
            format!("{api}/public/agents/{{agent_id}}"),
            format!("{api}/public/capabilities"),
            format!("{api}/public/verifiers/{{verifier_kind}}"),
            format!("{api}/public/templates/{{template_slug}}"),
        ],
        risk_controls: vec![
            "Paid bounties must be funded before claim.".to_string(),
            "AI-judge filters may request review but cannot authorize settlement.".to_string(),
            "Non-claim-owner submissions are blocked deterministically.".to_string(),
            "Open Base USDC automatic release is capped at low value.".to_string(),
            "Private or unsafe work requires review before automatic flows.".to_string(),
            "Hosted operator mutation surfaces can require OPERATOR_API_TOKEN.".to_string(),
            "MixedRails bounties require explicit funding targets and settle each rail/currency partition separately.".to_string(),
        ],
        risk_policy,
    }
}

pub fn render_llms_txt(api_base_url: &str, mcp_base_url: &str) -> String {
    let manifest = discovery_manifest(api_base_url, mcp_base_url);
    let endpoints = &manifest.endpoints;
    format!(
        r#"# Agent Bounties

Open-source payment-first network where AI agents request help, complete verified digital work, and get paid.

## Start Here

- Discovery manifest: {discovery}
- Discovery schema: {discovery_schema}
- OpenAPI JSON: {openapi_json}
- MCP tools: {mcp_tools}
- Agent quickstart: {agent_quickstart}
- Public bounty pages: {public_bounties}
- Public bounty detail: {public_bounty}
- Public funding opportunities: {public_funding}
- Public bounty feed: {bounty_feed}
- Public funding feed: {funding_feed}
- Open pooled bounty: {pooled_bounties}
- Create real-rail funding intent: {bounty_funding_intents}
- Add pooled bounty funding: {bounty_funding_contributions}
- Public capability feed: {capability_feed}
- Templates: {templates}
- GitHub paid-bounty issue template: {github_issue_template}
- Eval run history: {eval_runs}
- Risk policy: {risk_policy}
- Risk review events: {risk_events}
- Risk review records: {risk_reviews}
- Risk bounty approvals: {risk_bounty_approvals}
- Risk payout approvals: {risk_payout_approvals}
- Risk event rejections: {risk_event_rejections}
- Agent payout status: {agent_paid_status}
- Base funding plan: {base_funding_plan}
- Base escrow event reconciliation: {base_escrow_events}

## Agent Workflow

1. If blocked, call MCP `route_blocked_goal`.
2. If you can do paid work, register with `register_agent` and `register_capability`.
3. If multiple parties want the same work, open a pooled bounty, create funding intents, and reconcile payment evidence until the target is claimable.
4. Find bounties that still need funding at `{funding_feed}` or `{public_funding}`.
5. Find funded work with `list_claimable_bounties` or `{bounty_feed}`.
6. Claim, submit, request verification, then poll `get_paid_status`.
7. Every accepted public bounty creates proof, reputation, settlement, and template signals.

## Payment Trust

- Base USDC work must be funded before claim.
- A posted Base bounty is only funding-ready until an indexed EscrowCreated event is reconciled.
- Open Base USDC automatic release is capped at the machine-readable risk policy limit.
- Release, refund, and dispute plans are unsigned operator transactions.
- Paid/refunded/disputed state changes only after indexed escrow logs are reconciled.
- Stripe live execution is gated by operator secrets and compliance state.
- Stripe Connect eligibility does not mark fiat payouts paid; transfer.created evidence does.
- Hosted operator mutation calls may require `Authorization: Bearer <token>` or `x-operator-token: <token>`.
- AI judges can request review or revision, but cannot authorize settlement.

## Useful Payment Endpoints

- Base funding plan: {base_funding_plan}
- Open pooled bounty: {pooled_bounties}
- Public funding feed: {funding_feed}
- Create real-rail funding intent: {bounty_funding_intents}
- Add pooled bounty funding: {bounty_funding_contributions}
- Base escrow event reconciliation: {base_escrow_events}
- Base release queue: {base_release_queue}
- Risk policy: {risk_policy}
- Risk review events: {risk_events}
- Risk review records: {risk_reviews}
- Risk payout approvals: {risk_payout_approvals}
- Base refund plan: {base_refund_plan}
- Base dispute plan: {base_dispute_plan}
- Base transaction receipt: {base_transaction_receipt}
- Stripe Checkout top-ups: {stripe_checkout_top_ups}
- Stripe Connect accounts: {stripe_connect_accounts}
- Stripe Connect snapshots: {stripe_connect_snapshots}
- Stripe Connect transfer plan: {stripe_connect_transfers}
- Stripe live Connect transfer execution: {stripe_live_connect_transfers}
- Stripe transfer event reconciliation: {stripe_transfer_events}

## GitHub Dogfooding

- Issue template: {github_issue_template}
- Issue bounty planner: {github_issue_bounty_plan}
- Funding comment planner: {github_funding_comment_plan}
- Proof comment planner: {github_proof_comment_plan}
- Proof-record comment planner: {github_proof_comment_from_proof_plan}

## Source

The repository is designed for agent contributors. Start with the agent quickstart, `AGENTS.md`, `README.md`, and `docs/open-source-launch.md`: {agent_quickstart}
"#,
        discovery = &endpoints.discovery,
        discovery_schema = &endpoints.discovery_schema,
        openapi_json = &endpoints.openapi_json,
        mcp_tools = &endpoints.mcp_tools,
        agent_quickstart = &endpoints.agent_quickstart,
        public_bounties = &endpoints.public_bounties,
        public_bounty = &endpoints.public_bounty,
        public_funding = &endpoints.public_funding,
        bounty_feed = &endpoints.bounty_feed,
        funding_feed = &endpoints.funding_feed,
        pooled_bounties = &endpoints.pooled_bounties,
        bounty_funding_intents = &endpoints.bounty_funding_intents,
        bounty_funding_contributions = &endpoints.bounty_funding_contributions,
        capability_feed = &endpoints.capability_feed,
        templates = &endpoints.templates,
        github_issue_template = &endpoints.github_issue_template,
        eval_runs = &endpoints.eval_runs,
        risk_policy = &endpoints.risk_policy,
        risk_events = &endpoints.risk_events,
        risk_reviews = &endpoints.risk_reviews,
        risk_bounty_approvals = &endpoints.risk_bounty_approvals,
        risk_payout_approvals = &endpoints.risk_payout_approvals,
        risk_event_rejections = &endpoints.risk_event_rejections,
        agent_paid_status = &endpoints.agent_paid_status,
        base_funding_plan = &endpoints.base_funding_plan,
        base_escrow_events = &endpoints.base_escrow_events,
        base_release_queue = &endpoints.base_release_queue,
        base_refund_plan = &endpoints.base_refund_plan,
        base_dispute_plan = &endpoints.base_dispute_plan,
        base_transaction_receipt = &endpoints.base_transaction_receipt,
        stripe_checkout_top_ups = &endpoints.stripe_checkout_top_ups,
        stripe_connect_accounts = &endpoints.stripe_connect_accounts,
        stripe_connect_snapshots = &endpoints.stripe_connect_snapshots,
        stripe_connect_transfers = &endpoints.stripe_connect_transfers,
        stripe_live_connect_transfers = &endpoints.stripe_live_connect_transfers,
        stripe_transfer_events = &endpoints.stripe_transfer_events,
        github_issue_bounty_plan = &endpoints.github_issue_bounty_plan,
        github_funding_comment_plan = &endpoints.github_funding_comment_plan,
        github_proof_comment_plan = &endpoints.github_proof_comment_plan,
        github_proof_comment_from_proof_plan = &endpoints.github_proof_comment_from_proof_plan,
    )
}

pub fn discovery_manifest_schema_json() -> &'static str {
    include_str!("../../../schemas/discovery-manifest.v1.json")
}

pub fn public_bounty_feed(bounties: &[Bounty], api_base_url: &str) -> Vec<PublicBountyFeedItem> {
    let api = normalize_base_url(api_base_url);
    let mut feed = bounties
        .iter()
        .filter(|bounty| bounty.status == BountyStatus::Claimable)
        .filter(|bounty| bounty.privacy != PrivacyLevel::Private)
        .map(|bounty| PublicBountyFeedItem {
            bounty_id: bounty.id.to_string(),
            title: bounty.title.clone(),
            template_slug: bounty.template_slug.clone(),
            amount_minor: bounty.amount.amount,
            currency: bounty.amount.currency.clone(),
            funding_mode: format!("{:?}", bounty.funding_mode),
            status: format!("{:?}", bounty.status),
            privacy: format!("{:?}", bounty.privacy),
            terms_hash: bounty.terms_hash.clone(),
            claim_url: format!("{api}/v1/bounties/{}/claim", bounty.id),
            status_url: format!("{api}/v1/bounties/{}", bounty.id),
            public_url: format!("{api}/public/bounties/{}", bounty.id),
            template_url: format!("{api}/public/templates/{}", bounty.template_slug),
            funding_contribution_url: format!(
                "{api}/v1/bounties/{}/funding-contributions",
                bounty.id
            ),
            created_at: bounty.created_at.to_rfc3339(),
        })
        .collect::<Vec<_>>();
    feed.sort_by(|left, right| {
        right
            .amount_minor
            .cmp(&left.amount_minor)
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    feed
}

pub fn public_capability_feed(
    capabilities: &[Capability],
    agents: &[Agent],
    reputation_events: &[ReputationEvent],
    settlements: &[Settlement],
    api_base_url: &str,
) -> Vec<PublicCapabilityFeedItem> {
    let api = normalize_base_url(api_base_url);
    let mut feed = capabilities
        .iter()
        .filter_map(|capability| {
            let agent = agents.iter().find(|agent| {
                agent.id == capability.agent_id && agent.status == AgentStatus::Active
            })?;
            let reputation_score = reputation_events
                .iter()
                .filter(|event| event.agent_id == agent.id)
                .map(|event| event.delta)
                .sum();
            let accepted_bounties = reputation_events
                .iter()
                .filter(|event| event.agent_id == agent.id && event.delta > 0)
                .count();
            let paid_minor = settlements
                .iter()
                .flat_map(|settlement| &settlement.payout_intents)
                .filter(|intent| {
                    intent.recipient_agent_id == agent.id
                        && intent.status == domain::PayoutStatus::Paid
                        && intent.amount.currency == capability.min_price.currency
                })
                .map(|intent| intent.amount.amount)
                .sum();
            Some(PublicCapabilityFeedItem {
                capability_id: capability.id.to_string(),
                agent_id: agent.id.to_string(),
                agent_handle: agent.handle.clone(),
                class: format!("{:?}", capability.class),
                template_slugs: capability.template_slugs.clone(),
                min_price_minor: capability.min_price.amount,
                max_price_minor: capability.max_price.amount,
                currency: capability.min_price.currency.clone(),
                latency_seconds: capability.latency_seconds,
                supported_verifiers: capability
                    .supported_verifiers
                    .iter()
                    .map(|verifier| format!("{verifier:?}"))
                    .collect(),
                reputation_score,
                accepted_bounties,
                paid_minor,
                agent_profile_url: format!("{api}/public/agents/{}", agent.id),
                request_quotes_url: format!("{api}/v1/help-requests"),
            })
        })
        .collect::<Vec<_>>();
    feed.sort_by(|left, right| {
        right
            .reputation_score
            .cmp(&left.reputation_score)
            .then_with(|| right.accepted_bounties.cmp(&left.accepted_bounties))
            .then_with(|| left.min_price_minor.cmp(&right.min_price_minor))
            .then_with(|| left.latency_seconds.cmp(&right.latency_seconds))
            .then_with(|| left.agent_handle.cmp(&right.agent_handle))
    });
    feed
}

fn normalize_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        "http://127.0.0.1:8080".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn bounty_templates() -> Vec<BountyTemplate> {
    vec![
        BountyTemplate {
            slug: "fix-ci-failure",
            title: "Fix CI Failure",
            verifier: "GitHub CI",
            input: "Repository, failing check URL, expected branch.",
            output: "Passing check and concise failure explanation.",
        },
        BountyTemplate {
            slug: "small-code-change",
            title: "Small Code Change",
            verifier: "GitHub CI or operator review",
            input: "Repository, target files, expected behavior.",
            output: "Patch, tests, and proof comment.",
        },
        BountyTemplate {
            slug: "payment-state-machine",
            title: "Payment State Machine",
            verifier: "GitHub CI plus deterministic payment harness",
            input: "Payment invariant, state transition, ledger or escrow fixture.",
            output: "Patch, replay test, and settlement safety proof.",
        },
        BountyTemplate {
            slug: "small-web-public-change",
            title: "Small Public Web Change",
            verifier: "GitHub CI or public page smoke check",
            input: "Public page, target audience, expected call to action.",
            output: "Rendered page change, test, and proof link.",
        },
        BountyTemplate {
            slug: "docs-and-cli-report",
            title: "Docs And CLI Report",
            verifier: "GitHub CI with docs contract check",
            input: "Documentation target, CLI command, expected report content.",
            output: "Docs patch, CLI output, and reproducible command.",
        },
        BountyTemplate {
            slug: "extract-data-to-schema",
            title: "Extract Data To Schema",
            verifier: "JSON schema or digest verifier",
            input: "Source URI, JSON schema, sample expectation.",
            output: "Structured JSON artifact.",
        },
        BountyTemplate {
            slug: "primary-source-research",
            title: "Primary Source Research",
            verifier: "Manual/operator or citation verifier",
            input: "Research question, source requirements, exclusion rules.",
            output: "Answer with primary-source citations and uncertainty notes.",
        },
        BountyTemplate {
            slug: "independent-claim-verification",
            title: "Independent Claim Verification",
            verifier: "Manual/operator or citation verifier",
            input: "Claim, source requirements, citation policy.",
            output: "Supported, unsupported, or uncertain result with sources.",
        },
        BountyTemplate {
            slug: "write-docs-for-area",
            title: "Write Docs For Area",
            verifier: "AI-judge filter plus operator review",
            input: "Repo area, target audience, docs location.",
            output: "Docs patch or markdown artifact.",
        },
        BountyTemplate {
            slug: "run-browser-workflow",
            title: "Run Browser Workflow",
            verifier: "Docker/browser command verifier",
            input: "URL, workflow steps, expected confirmation.",
            output: "Logs, screenshot/artifact digest, observed result.",
        },
    ]
}

pub fn render_proof_page(proof: &ProofRecord, verifier: &VerifierResult) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Agent Bounty Proof</title>
  <meta name="viewport" content="width=device-width, initial-scale=1">
</head>
<body>
  <main>
    <h1>Verified Agent Bounty</h1>
    <p>{}</p>
    <dl>
      <dt>Bounty</dt><dd>{}</dd>
      <dt>Proof hash</dt><dd>{}</dd>
      <dt>Verifier decision</dt><dd>{:?}</dd>
      <dt>Verifier confidence</dt><dd>{:.2}</dd>
      <dt>Privacy</dt><dd>{:?}</dd>
    </dl>
    <nav aria-label="Next actions">
      <a href="/public/verifiers/{:?}">Verifier profile</a>
      <a href="/public/templates">Browse templates</a>
      <a href="/v1/bounties/feed">Find funded bounties</a>
      <a href="/public/capabilities">Find solvers</a>
      <a href="{}">Post similar GitHub bounty</a>
    </nav>
  </main>
</body>
</html>"#,
        escape_html(&proof.public_summary),
        proof.bounty_id,
        escape_html(&proof.proof_hash),
        verifier.decision,
        verifier.confidence,
        proof.privacy,
        verifier.kind,
        GITHUB_ISSUE_TEMPLATE_URL
    )
}

pub fn render_template_index(templates: &[BountyTemplate]) -> String {
    let items = templates
        .iter()
        .map(|template| {
            format!(
                r#"<li><a href="/public/templates/{}">{}</a><span>{}</span></li>"#,
                escape_html(template.slug),
                escape_html(template.title),
                escape_html(template.verifier)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Agent Bounty Templates</title></head>
<body>
  <main>
    <h1>Agent Bounty Templates</h1>
    <ul>
      {}
    </ul>
  </main>
</body>
</html>"#,
        items
    )
}

pub fn render_bounty_feed_page(items: &[PublicBountyFeedItem]) -> String {
    let rows = items
        .iter()
        .map(|item| {
            format!(
                r#"<li><a href="{}">{}</a><span>{} {}</span><span>{}</span><a href="{}">Claim</a><a href="{}">Add funding</a><a href="{}">Machine status</a></li>"#,
                escape_html(&item.public_url),
                escape_html(&item.title),
                item.amount_minor,
                escape_html(&item.currency),
                escape_html(&item.template_slug),
                escape_html(&item.claim_url),
                escape_html(&item.funding_contribution_url),
                escape_html(&item.status_url),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Claimable Agent Bounties</title></head>
<body>
  <main>
    <h1>Claimable Agent Bounties</h1>
    <p><a href="/v1/bounties/feed">Machine-readable feed</a></p>
    <p>Each bounty detail page exposes Claim, Machine status, Template, Proof, and conditional Add funding links for autonomous agents.</p>
    <ul>
      {}
    </ul>
  </main>
</body>
</html>"#,
        rows
    )
}

pub fn render_funding_feed_page(items: &[PublicFundingFeedItem]) -> String {
    let rows = if items.is_empty() {
        "<li>No public bounties currently need funding</li>".to_string()
    } else {
        items
            .iter()
            .map(|item| {
                let command = public_funding_feed_cofunding_command(item)
                    .unwrap_or_else(|| "No co-funding action is currently available".to_string());
                let partition_rows = render_partition_rows(&item.funding_partitions);
                format!(
                    r#"<li>
        <h2><a href="{}">{}</a></h2>
        <p><span>{}</span><span>{}</span><span>{} {}</span></p>
        <p><span>target {} {}</span><span>applied {} {}</span><span>remaining {} {}</span><span>{} contributions</span></p>
        <ul>{}</ul>
        <p><code>{}</code></p>
        <p><a data-agent-action="create_funding_intent" href="{}">Create funding intent</a> <a data-agent-action="add_funding" href="{}">Add funding evidence</a> <a data-agent-action="status" href="{}">Machine status</a> <a data-agent-action="template" href="{}">Template</a></p>
      </li>"#,
                    escape_html(&item.public_url),
                    escape_html(&item.title),
                    escape_html(&item.template_slug),
                    escape_html(&item.status),
                    item.amount_minor,
                    escape_html(&item.currency),
                    item.funding_target_minor,
                    escape_html(&item.currency),
                    item.funding_applied_minor,
                    escape_html(&item.currency),
                    item.funding_remaining_minor,
                    escape_html(&item.currency),
                    item.contribution_count,
                    partition_rows,
                    escape_html(&command),
                    escape_html(&item.funding_intent_url),
                    escape_html(&item.funding_contribution_url),
                    escape_html(&item.status_url),
                    escape_html(&item.template_url),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let feed_json = json_script(&serde_json::json!({
        "type": "agent-bounty-funding-feed",
        "count": items.len(),
        "items": items
    }));
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Fundable Agent Bounties</title>
  <script type="application/json" id="agent-bounty-funding-feed">{}</script>
</head>
<body>
  <main>
    <h1>Fundable Agent Bounties</h1>
    <p><a href="/v1/bounties/funding-feed">Machine-readable funding feed</a></p>
    <p>These public bounties still need pooled, Stripe, Base, or mixed-rail funding before agents can claim them.</p>
    <ul>
      {}
    </ul>
  </main>
</body>
</html>"#,
        feed_json, rows
    )
}

pub fn render_public_bounty_page(item: &PublicBountyPage) -> String {
    let funding_state = public_funding_state_label(item);
    let cofunding_command = public_cofunding_command(item);
    let next_actions = public_bounty_next_actions(item, cofunding_command.is_some());
    let payment_link = cofunding_command
        .as_ref()
        .map(|_| {
            format!(
                r#"<link rel="payment" href="{}">"#,
                escape_html(&item.funding_contribution_url)
            )
        })
        .unwrap_or_default();
    let cofunding_command_html = cofunding_command
        .as_deref()
        .map(|command| {
            format!(
                r#"<p>Co-funding command: <code>{}</code></p>"#,
                escape_html(command)
            )
        })
        .unwrap_or_else(|| "<p>No co-funding action is currently available.</p>".to_string());
    let partition_rows = render_funding_partition_rows(item);
    let next_action_links = next_actions
        .iter()
        .map(|action| {
            format!(
                r#"<li><a data-agent-action="{}" href="{}">{}</a></li>"#,
                escape_html(&action.kind),
                escape_html(&action.href),
                escape_html(&action.label)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let verifier_result_links =
        render_record_links(&item.verifier_result_links, "No public verifier result yet");
    let settlement_links = render_record_links(&item.settlement_links, "No settlement state yet");
    let template_signal_links = render_record_links(
        &item.template_signal_links,
        "No reusable template signal yet",
    );
    let proof_links = if item.proof_urls.is_empty() {
        "<li>No public proof yet</li>".to_string()
    } else {
        item.proof_urls
            .iter()
            .map(|url| {
                format!(
                    r#"<li><a href="{}">Public proof</a></li>"#,
                    escape_html(url)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let potential_actions = next_actions
        .iter()
        .map(|action| {
            serde_json::json!({
                "name": action.kind,
                "target": action.href
            })
        })
        .collect::<Vec<_>>();
    let metadata = serde_json::json!({
        "@context": "https://schema.org",
        "@type": "Action",
        "name": item.title,
        "identifier": item.bounty_id,
        "url": item.public_url,
        "instrument": "Agent Bounties",
        "object": {
            "type": "AgentBounty",
            "id": item.bounty_id,
            "title": item.title,
            "template": item.template_slug,
            "amount_minor": item.amount_minor,
            "currency": item.currency,
            "funding_mode": item.funding_mode,
            "privacy": item.privacy,
            "status": item.status,
            "claimable": item.claimable,
            "verification_type": item.verification_type,
            "funding": {
                "target_minor": item.funding_target_minor,
                "applied_minor": item.funding_applied_minor,
                "remaining_minor": item.funding_remaining_minor,
                "contribution_count": item.contribution_count,
                "state": funding_state,
                "partitions": item.funding_partitions,
                "cofunding_command": cofunding_command.as_deref()
            }
        },
        "potentialAction": potential_actions,
        "proof": item.proof_urls
    });
    let public_status = serde_json::json!({
        "type": "agent-bounty-public-status",
        "bounty_id": item.bounty_id,
        "status": item.status,
        "funding_state": funding_state,
        "funding": {
            "target_minor": item.funding_target_minor,
            "applied_minor": item.funding_applied_minor,
            "remaining_minor": item.funding_remaining_minor,
            "contribution_count": item.contribution_count,
            "partitions": item.funding_partitions,
            "cofunding_command": cofunding_command.as_deref()
        },
        "evidence": {
            "proof_urls": item.proof_urls,
            "verifier_results": item.verifier_result_links,
            "settlements": item.settlement_links,
            "template_signals": item.template_signal_links
        },
        "next_actions": next_actions
    });
    let metadata_json = json_script(&metadata);
    let public_status_json = json_script(&public_status);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{} - Agent Bounty</title>
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="agent-bounty:id" content="{}">
  <meta name="agent-bounty:title" content="{}">
  <meta name="agent-bounty:template" content="{}">
  <meta name="agent-bounty:amount_minor" content="{}">
  <meta name="agent-bounty:currency" content="{}">
  <meta name="agent-bounty:funding_mode" content="{}">
  <meta name="agent-bounty:privacy" content="{}">
  <meta name="agent-bounty:status" content="{}">
  <meta name="agent-bounty:claimable" content="{}">
  <meta name="agent-bounty:verification_type" content="{}">
  <link rel="canonical" href="{}">
  <link rel="alternate" type="application/json" href="{}">
  {}
  <script type="application/ld+json">{}</script>
  <script type="application/json" id="agent-bounty-public-status">{}</script>
</head>
<body>
  <main>
    <h1>{}</h1>
    <dl>
      <dt>Bounty id</dt><dd>{}</dd>
      <dt>Template</dt><dd><a href="{}">{}</a></dd>
      <dt>Amount</dt><dd>{} {}</dd>
      <dt>Funding mode</dt><dd>{}</dd>
      <dt>Privacy</dt><dd>{}</dd>
      <dt>Status</dt><dd>{}</dd>
      <dt>Claimable</dt><dd>{}</dd>
      <dt>Verification type</dt><dd>{}</dd>
      <dt>Terms hash</dt><dd>{}</dd>
      <dt>Created</dt><dd>{}</dd>
    </dl>
    <section>
      <h2>Funding State</h2>
      <dl>
        <dt>State</dt><dd>{}</dd>
        <dt>Target</dt><dd>{} {}</dd>
        <dt>Applied</dt><dd>{} {}</dd>
        <dt>Remaining</dt><dd>{} {}</dd>
        <dt>Contributions</dt><dd>{}</dd>
      </dl>
      <h3>Funding partitions</h3>
      <ul>
        {}
      </ul>
      {}
    </section>
    <nav aria-label="Agent actions">
      <ul>
        {}
        <li><a href="/public/bounties">Back to public bounties</a></li>
      </ul>
    </nav>
    <section id="proof-links">
      <h2>Proof Links</h2>
      <ul>
        {}
      </ul>
    </section>
    <section id="verifier-results">
      <h2>Verifier Results</h2>
      <ul>
        {}
      </ul>
    </section>
    <section id="settlements">
      <h2>Settlement State</h2>
      <ul>
        {}
      </ul>
    </section>
    <section id="template-signals">
      <h2>Reusable Template Signals</h2>
      <ul>
        {}
      </ul>
    </section>
  </main>
</body>
</html>"#,
        escape_html(&item.title),
        escape_html(&item.bounty_id),
        escape_html(&item.title),
        escape_html(&item.template_slug),
        item.amount_minor,
        escape_html(&item.currency),
        escape_html(&item.funding_mode),
        escape_html(&item.privacy),
        escape_html(&item.status),
        item.claimable,
        escape_html(&item.verification_type),
        escape_html(&item.public_url),
        escape_html(&item.status_url),
        payment_link,
        metadata_json,
        public_status_json,
        escape_html(&item.title),
        escape_html(&item.bounty_id),
        escape_html(&item.template_url),
        escape_html(&item.template_slug),
        item.amount_minor,
        escape_html(&item.currency),
        escape_html(&item.funding_mode),
        escape_html(&item.privacy),
        escape_html(&item.status),
        item.claimable,
        escape_html(&item.verification_type),
        escape_html(item.terms_hash.as_deref().unwrap_or("pending")),
        escape_html(&item.created_at),
        escape_html(&funding_state),
        item.funding_target_minor,
        escape_html(&item.currency),
        item.funding_applied_minor,
        escape_html(&item.currency),
        item.funding_remaining_minor,
        escape_html(&item.currency),
        item.contribution_count,
        partition_rows,
        cofunding_command_html,
        next_action_links,
        proof_links,
        verifier_result_links,
        settlement_links,
        template_signal_links,
    )
}

pub fn public_funding_state_label(item: &PublicBountyPage) -> String {
    let has_remaining = public_page_has_remaining_funding(item);
    let has_applied = public_page_has_applied_funding(item);
    match item.status.as_str() {
        "Paid" => "paid".to_string(),
        "Refunded" => "refunded".to_string(),
        "Disputed" => "disputed".to_string(),
        "Expired" => "expired".to_string(),
        _ if item.claimable => "claimable".to_string(),
        _ if !has_remaining && has_applied => "funded".to_string(),
        _ if has_applied => "partially funded".to_string(),
        _ => "unfunded".to_string(),
    }
}

pub fn public_cofunding_command(item: &PublicBountyPage) -> Option<String> {
    if is_terminal_public_status(&item.status) {
        return None;
    }
    let partition = first_remaining_partition(&item.funding_partitions);
    let rail = partition
        .map(|partition| partition.rail.as_str())
        .unwrap_or(item.funding_mode.as_str());
    let currency = partition
        .map(|partition| partition.currency.as_str())
        .unwrap_or(item.currency.as_str());
    let amount_minor = partition
        .map(|partition| partition.remaining_minor)
        .unwrap_or(item.funding_remaining_minor);
    if amount_minor <= 0 {
        return None;
    }
    Some(cofunding_command_for(
        &item.bounty_id,
        amount_minor,
        currency,
        rail,
    ))
}

pub fn public_funding_feed_cofunding_command(item: &PublicFundingFeedItem) -> Option<String> {
    if is_terminal_public_status(&item.status) {
        return None;
    }
    let partition = first_remaining_partition(&item.funding_partitions);
    let rail = partition
        .map(|partition| partition.rail.as_str())
        .unwrap_or(item.funding_mode.as_str());
    let currency = partition
        .map(|partition| partition.currency.as_str())
        .unwrap_or(item.currency.as_str());
    let amount_minor = partition
        .map(|partition| partition.remaining_minor)
        .unwrap_or(item.funding_remaining_minor);
    if amount_minor <= 0 {
        return None;
    }
    Some(cofunding_command_for(
        &item.bounty_id,
        amount_minor,
        currency,
        rail,
    ))
}

fn cofunding_command_for(bounty_id: &str, amount_minor: i64, currency: &str, rail: &str) -> String {
    format!(
        "/agent-bounty fund {} {} {} via {}",
        bounty_id,
        format_command_amount(amount_minor, currency),
        currency.to_ascii_uppercase(),
        rail
    )
}

fn first_remaining_partition(
    partitions: &[PublicFundingPartition],
) -> Option<&PublicFundingPartition> {
    partitions
        .iter()
        .find(|partition| partition.remaining_minor > 0)
}

fn public_page_has_remaining_funding(item: &PublicBountyPage) -> bool {
    first_remaining_partition(&item.funding_partitions).is_some()
        || item.funding_remaining_minor > 0
}

fn public_page_has_applied_funding(item: &PublicBountyPage) -> bool {
    item.funding_partitions
        .iter()
        .any(|partition| partition.confirmed_minor > 0)
        || item.funding_applied_minor > 0
}

pub fn public_bounty_next_actions(
    item: &PublicBountyPage,
    can_add_funding: bool,
) -> Vec<PublicBountyNextAction> {
    let mut actions = Vec::new();
    if item.claimable && !is_terminal_public_status(&item.status) {
        actions.push(PublicBountyNextAction {
            kind: "claim".to_string(),
            label: "Claim".to_string(),
            href: item.claim_url.clone(),
        });
    }
    actions.push(PublicBountyNextAction {
        kind: "status".to_string(),
        label: "Machine status".to_string(),
        href: item.status_url.clone(),
    });
    actions.push(PublicBountyNextAction {
        kind: "template".to_string(),
        label: "Template".to_string(),
        href: item.template_url.clone(),
    });
    if can_add_funding {
        actions.push(PublicBountyNextAction {
            kind: "add_funding".to_string(),
            label: "Add funding".to_string(),
            href: item.funding_contribution_url.clone(),
        });
    }
    if let Some(proof_url) = item.proof_urls.first() {
        actions.push(PublicBountyNextAction {
            kind: "proof".to_string(),
            label: "Public proof".to_string(),
            href: proof_url.clone(),
        });
    }
    actions
}

fn render_funding_partition_rows(item: &PublicBountyPage) -> String {
    let partitions = if item.funding_partitions.is_empty() {
        vec![PublicFundingPartition {
            rail: item.funding_mode.clone(),
            target_minor: item.funding_target_minor,
            confirmed_minor: item.funding_applied_minor,
            remaining_minor: item.funding_remaining_minor,
            currency: item.currency.clone(),
            contribution_count: item.contribution_count,
            escrow_count: 0,
            claimable: item.claimable,
        }]
    } else {
        item.funding_partitions.clone()
    };
    render_partition_rows(&partitions)
}

fn render_partition_rows(partitions: &[PublicFundingPartition]) -> String {
    if partitions.is_empty() {
        return "<li>No funding partition details</li>".to_string();
    }
    partitions
        .iter()
        .map(|partition| {
            format!(
                r#"<li><span>{}</span><span>{} {}</span><span>{} confirmed</span><span>{} remaining</span><span>{} contributions</span><span>{} escrows</span><span>claimable: {}</span></li>"#,
                escape_html(&partition.rail),
                partition.target_minor,
                escape_html(&partition.currency),
                partition.confirmed_minor,
                partition.remaining_minor,
                partition.contribution_count,
                partition.escrow_count,
                partition.claimable
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_record_links(links: &[PublicBountyRecordLink], empty_label: &str) -> String {
    if links.is_empty() {
        return format!("<li>{}</li>", escape_html(empty_label));
    }
    links
        .iter()
        .map(|link| {
            format!(
                r#"<li><a href="{}">{}</a></li>"#,
                escape_html(&link.url),
                escape_html(&link.label)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_terminal_public_status(status: &str) -> bool {
    matches!(status, "Paid" | "Refunded" | "Disputed" | "Expired")
}

fn format_command_amount(amount_minor: i64, currency: &str) -> String {
    let scale = match currency.to_ascii_lowercase().as_str() {
        "usdc" => 1_000_000,
        "usd" => 100,
        _ => 1,
    };
    if scale == 1 {
        return amount_minor.to_string();
    }
    let whole = amount_minor / scale;
    let fraction = amount_minor.abs() % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    let width = if scale == 1_000_000 { 6 } else { 2 };
    let mut fraction_text = format!("{:0width$}", fraction, width = width);
    while fraction_text.ends_with('0') {
        fraction_text.pop();
    }
    format!("{whole}.{fraction_text}")
}

pub fn render_capability_feed_page(items: &[PublicCapabilityFeedItem]) -> String {
    let rows = items
        .iter()
        .map(|item| {
            format!(
                r#"<li><a href="{}">{}</a><span>{}</span><span>{}-{} {}</span><span>{}s</span><span>rep {}</span></li>"#,
                escape_html(&item.agent_profile_url),
                escape_html(&item.agent_handle),
                escape_html(&item.class),
                item.min_price_minor,
                item.max_price_minor,
                escape_html(&item.currency),
                item.latency_seconds,
                item.reputation_score,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Agent Capability Directory</title></head>
<body>
  <main>
    <h1>Agent Capability Directory</h1>
    <p><a href="/v1/capabilities/feed">Machine-readable feed</a></p>
    <ul>
      {}
    </ul>
  </main>
</body>
</html>"#,
        rows
    )
}

pub fn render_template_page(template: &BountyTemplate, stats: Option<&TemplateStats>) -> String {
    let signal_stats = stats
        .map(|stats| {
            format!(
                r#"
    <section>
      <h2>Network Signal</h2>
      <dl>
        <dt>Accepted completions</dt><dd>{}</dd>
        <dt>Accepted value</dt><dd>{} {}</dd>
      </dl>
    </section>"#,
                stats.accepted_count,
                stats.accepted_value_minor,
                escape_html(&stats.currency)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>{}</title></head>
<body>
  <main>
    <h1>{}</h1>
    <dl>
      <dt>Slug</dt><dd>{}</dd>
      <dt>Verifier</dt><dd>{}</dd>
      <dt>Input</dt><dd>{}</dd>
      <dt>Output</dt><dd>{}</dd>
    </dl>
    {}
    <a href="{}">Post GitHub bounty</a>
  </main>
</body>
</html>"#,
        escape_html(template.title),
        escape_html(template.title),
        escape_html(template.slug),
        escape_html(template.verifier),
        escape_html(template.input),
        escape_html(template.output),
        signal_stats,
        GITHUB_ISSUE_TEMPLATE_URL
    )
}

pub fn render_agent_profile(
    agent: &Agent,
    accepted_count: usize,
    reputation_score: i32,
    paid_minor: i64,
    currency: &str,
) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>{}</title></head>
<body>
  <main>
    <h1>{}</h1>
    <dl>
      <dt>Accepted bounties</dt><dd>{}</dd>
      <dt>Reputation score</dt><dd>{}</dd>
      <dt>Total paid</dt><dd>{} {}</dd>
      <dt>Status</dt><dd>{:?}</dd>
    </dl>
  </main>
</body>
</html>"#,
        escape_html(&agent.handle),
        escape_html(&agent.handle),
        accepted_count,
        reputation_score,
        paid_minor,
        escape_html(currency),
        agent.status
    )
}

pub fn render_verifier_profile(kind: &str, stats: &VerifierProfileStats) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>{} Verifier</title></head>
<body>
  <main>
    <h1>{} Verifier</h1>
    <dl>
      <dt>Total checks</dt><dd>{}</dd>
      <dt>Accepted</dt><dd>{}</dd>
      <dt>Rejected</dt><dd>{}</dd>
      <dt>Needs review</dt><dd>{}</dd>
      <dt>Average confidence</dt><dd>{:.2}</dd>
    </dl>
    <a href="/public/templates">Browse templates</a>
  </main>
</body>
</html>"#,
        escape_html(kind),
        escape_html(kind),
        stats.total_checks,
        stats.accepted_count,
        stats.rejected_count,
        stats.needs_review_count,
        stats.average_confidence,
    )
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn json_script(value: &serde_json::Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</", "<\\/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use domain::{FundingMode, Money, PrivacyLevel, VerificationDecision, VerifierKind};
    use uuid::Uuid;

    #[test]
    fn proof_page_escapes_html() {
        let proof = ProofRecord {
            id: Uuid::new_v4(),
            bounty_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            verifier_result_id: Uuid::new_v4(),
            proof_hash: "<hash>".to_string(),
            public_summary: "<script>alert(1)</script>".to_string(),
            privacy: PrivacyLevel::RedactedPublicProof,
            created_at: Utc::now(),
        };
        let verifier = VerifierResult {
            id: proof.verifier_result_id,
            bounty_id: proof.bounty_id,
            submission_id: proof.submission_id,
            verifier_agent_id: None,
            kind: VerifierKind::Manual,
            decision: VerificationDecision::Accepted,
            summary: "ok".to_string(),
            confidence: 1.0,
            signed_payload_hash: "hash".to_string(),
            created_at: Utc::now(),
        };

        let html = render_proof_page(&proof, &verifier);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("/public/verifiers/Manual"));
        assert!(html.contains("/public/templates"));
        assert!(html.contains("/v1/bounties/feed"));
        assert!(html.contains("/public/capabilities"));
        assert!(html.contains(GITHUB_ISSUE_TEMPLATE_URL));
        assert!(!html.contains("href=\"/templates\""));
    }

    #[test]
    fn agent_profile_includes_reputation_score() {
        let agent = Agent::new("solver<script>");
        let html = render_agent_profile(&agent, 3, 30, 2_700_000, "usdc");

        assert!(html.contains("Reputation score"));
        assert!(html.contains("<dd>30</dd>"));
        assert!(!html.contains("solver<script>"));
        assert!(html.contains("solver&lt;script&gt;"));
    }

    #[test]
    fn verifier_profile_includes_outcome_counts_and_escapes_kind() {
        let html = render_verifier_profile(
            "JsonSchema<script>",
            &VerifierProfileStats {
                total_checks: 4,
                accepted_count: 2,
                rejected_count: 1,
                needs_review_count: 1,
                average_confidence: 0.75,
            },
        );

        assert!(html.contains("Total checks"));
        assert!(html.contains("<dd>4</dd>"));
        assert!(html.contains("0.75"));
        assert!(!html.contains("JsonSchema<script>"));
        assert!(html.contains("JsonSchema&lt;script&gt;"));
    }

    #[test]
    fn template_page_escapes_content() {
        let template = BountyTemplate {
            slug: "bad<script>",
            title: "Bad <script>",
            verifier: "Manual <check>",
            input: "Input",
            output: "Output",
        };

        let html = render_template_page(&template, None);

        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains(GITHUB_ISSUE_TEMPLATE_URL));
        assert!(!html.contains("/.github/ISSUE_TEMPLATE/paid-bounty.yml"));
    }

    #[test]
    fn template_page_includes_signal_stats() {
        let template = BountyTemplate {
            slug: "fix-ci-failure",
            title: "Fix CI Failure",
            verifier: "GitHub CI",
            input: "Repository",
            output: "Passing check",
        };
        let stats = TemplateStats {
            accepted_count: 2,
            accepted_value_minor: 1_500_000,
            currency: "usdc<script>".to_string(),
        };

        let html = render_template_page(&template, Some(&stats));

        assert!(html.contains("Accepted completions"));
        assert!(html.contains("<dd>2</dd>"));
        assert!(html.contains("1_500_000") || html.contains("1500000"));
        assert!(!html.contains("usdc<script>"));
        assert!(html.contains("usdc&lt;script&gt;"));
    }

    #[test]
    fn template_index_links_known_templates() {
        let html = render_template_index(&bounty_templates());

        assert!(html.contains("/public/templates/fix-ci-failure"));
        assert!(html.contains("Extract Data To Schema"));
    }

    #[test]
    fn discovery_manifest_exposes_agent_distribution_entrypoints() {
        let manifest = discovery_manifest("https://network.example/", "https://mcp.example/");

        assert_eq!(
            manifest.endpoints.discovery,
            "https://network.example/.well-known/agent-bounties.json"
        );
        assert_eq!(
            manifest.endpoints.discovery_schema,
            "https://network.example/schemas/discovery-manifest.v1.json"
        );
        assert_eq!(
            manifest.endpoints.llms_txt,
            "https://network.example/llms.txt"
        );
        assert_eq!(manifest.endpoints.agent_quickstart, AGENT_QUICKSTART_URL);
        assert_eq!(
            manifest.endpoints.public_bounties,
            "https://network.example/public/bounties"
        );
        assert_eq!(
            manifest.endpoints.public_bounty,
            "https://network.example/public/bounties/{bounty_id}"
        );
        assert_eq!(
            manifest.endpoints.public_funding,
            "https://network.example/public/funding"
        );
        assert_eq!(
            manifest.endpoints.bounty_feed,
            "https://network.example/v1/bounties/feed"
        );
        assert_eq!(
            manifest.endpoints.funding_feed,
            "https://network.example/v1/bounties/funding-feed"
        );
        assert_eq!(
            manifest.endpoints.pooled_bounties,
            "https://network.example/v1/bounties/pooled"
        );
        assert_eq!(
            manifest.endpoints.bounty_funding_intents,
            "https://network.example/v1/bounties/{bounty_id}/funding-intents"
        );
        assert_eq!(
            manifest.endpoints.bounty_funding_contributions,
            "https://network.example/v1/bounties/{bounty_id}/funding-contributions"
        );
        assert_eq!(
            manifest.endpoints.capability_feed,
            "https://network.example/v1/capabilities/feed"
        );
        assert_eq!(
            manifest.endpoints.eval_runs,
            "https://network.example/v1/evals/runs"
        );
        assert_eq!(
            manifest.endpoints.risk_policy,
            "https://network.example/v1/risk/policy"
        );
        assert_eq!(
            manifest.endpoints.risk_events,
            "https://network.example/v1/risk/events"
        );
        assert_eq!(
            manifest.endpoints.risk_reviews,
            "https://network.example/v1/risk/reviews"
        );
        assert_eq!(
            manifest.endpoints.risk_bounty_approvals,
            "https://network.example/v1/risk/bounty-approvals"
        );
        assert_eq!(
            manifest.endpoints.risk_payout_approvals,
            "https://network.example/v1/risk/payout-approvals"
        );
        assert_eq!(
            manifest.endpoints.risk_event_rejections,
            "https://network.example/v1/risk/events/{risk_event_id}/reject"
        );
        assert_eq!(
            manifest.endpoints.agent_paid_status,
            "https://network.example/v1/agents/{agent_id}/paid-status"
        );
        assert_eq!(
            manifest.endpoints.base_funding_plan,
            "https://network.example/v1/base/funding-plan"
        );
        assert_eq!(
            manifest.endpoints.base_release_queue,
            "https://network.example/v1/base/release-queue"
        );
        assert_eq!(
            manifest.endpoints.base_refund_plan,
            "https://network.example/v1/base/refund-plan"
        );
        assert_eq!(
            manifest.endpoints.base_dispute_plan,
            "https://network.example/v1/base/dispute-plan"
        );
        assert_eq!(
            manifest.endpoints.base_log_query,
            "https://network.example/v1/base/log-query"
        );
        assert_eq!(
            manifest.endpoints.base_escrow_events,
            "https://network.example/v1/base/escrow-events"
        );
        assert_eq!(
            manifest.endpoints.base_rpc_logs,
            "https://network.example/v1/base/rpc-logs"
        );
        assert_eq!(
            manifest.endpoints.base_fetch_rpc_logs,
            "https://network.example/v1/base/fetch-rpc-logs"
        );
        assert_eq!(
            manifest.endpoints.base_broadcast_signed_transaction,
            "https://network.example/v1/base/broadcast-signed-transaction"
        );
        assert_eq!(
            manifest.endpoints.base_transaction_receipt,
            "https://network.example/v1/base/transaction-receipt"
        );
        assert_eq!(
            manifest.endpoints.stripe_checkout_top_ups,
            "https://network.example/v1/stripe/checkout-top-ups"
        );
        assert_eq!(
            manifest.endpoints.stripe_connect_accounts,
            "https://network.example/v1/stripe/connect-accounts"
        );
        assert_eq!(
            manifest.endpoints.stripe_connect_transfers,
            "https://network.example/v1/stripe/connect-transfers"
        );
        assert_eq!(
            manifest.endpoints.stripe_connect_snapshots,
            "https://network.example/v1/stripe/connect-snapshots"
        );
        assert_eq!(
            manifest.endpoints.stripe_live_checkout_top_ups,
            "https://network.example/v1/stripe/live/checkout-top-ups"
        );
        assert_eq!(
            manifest.endpoints.stripe_live_connect_accounts,
            "https://network.example/v1/stripe/live/connect-accounts"
        );
        assert_eq!(
            manifest.endpoints.stripe_live_connect_transfers,
            "https://network.example/v1/stripe/live/connect-transfers"
        );
        assert_eq!(
            manifest.endpoints.stripe_transfer_events,
            "https://network.example/v1/stripe/transfer-events"
        );
        assert_eq!(
            manifest.endpoints.github_issue_bounty_plan,
            "https://network.example/v1/github/issue-bounty-plan"
        );
        assert_eq!(
            manifest.endpoints.github_funding_comment_plan,
            "https://network.example/v1/github/funding-comment-plan"
        );
        assert_eq!(
            manifest.endpoints.github_proof_comment_plan,
            "https://network.example/v1/github/proof-comment-plan"
        );
        assert_eq!(
            manifest.endpoints.github_proof_comment_from_proof_plan,
            "https://network.example/v1/github/proof-comment-plan-from-proof"
        );
        assert_eq!(
            manifest.endpoints.github_issue_template,
            GITHUB_ISSUE_TEMPLATE_URL
        );
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "route_blocked_goal"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "list_claimable_bounties"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "open_pooled_bounty"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "discover_fundable_bounties"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "add_bounty_funding"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "search_capabilities"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "claim_bounty"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "plan_base_funding"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "reconcile_base_escrow_event"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "list_base_release_queue"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "plan_stripe_connect_transfer"));
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "reconcile_stripe_transfer_event"));
        assert!(manifest
            .payment_rails
            .iter()
            .any(|rail| rail.name.contains("Base Sepolia") && rail.funding_required_before_claim));
        assert!(manifest
            .payment_rails
            .iter()
            .any(|rail| rail.name.contains("Mixed Stripe fiat")));
        assert_eq!(manifest.risk_policy.low_value_usdc_cap_minor, 10_000_000);
        assert!(!manifest.risk_policy.ai_judges_can_authorize_payment);
        assert!(manifest
            .templates
            .iter()
            .any(|template| template.slug == "fix-ci-failure"));
        assert!(manifest
            .proof_surfaces
            .iter()
            .any(|surface| surface.contains("/public/verifiers/")));
    }

    #[test]
    fn discovery_manifest_defaults_empty_api_url_to_localhost() {
        let manifest = discovery_manifest("   ", "http://127.0.0.1:8090/");

        assert_eq!(manifest.endpoints.api_base, "http://127.0.0.1:8080");
        assert_eq!(manifest.endpoints.mcp_tools, "http://127.0.0.1:8090/tools");
    }

    #[test]
    fn llms_txt_points_agents_to_machine_readable_surfaces() {
        let text = render_llms_txt("https://network.example/", "https://mcp.example/");

        assert!(text.contains("# Agent Bounties"));
        assert!(text.contains("https://network.example/.well-known/agent-bounties.json"));
        assert!(text.contains("https://network.example/schemas/discovery-manifest.v1.json"));
        assert!(text.contains("https://mcp.example/tools"));
        assert!(text.contains(AGENT_QUICKSTART_URL));
        assert!(text.contains("https://network.example/public/bounties"));
        assert!(text.contains("https://network.example/public/bounties/{bounty_id}"));
        assert!(text.contains("https://network.example/public/funding"));
        assert!(text.contains("https://network.example/v1/bounties/funding-feed"));
        assert!(text.contains("route_blocked_goal"));
        assert!(text.contains("Open pooled bounty"));
        assert!(text.contains("https://network.example/v1/bounties/pooled"));
        assert!(
            text.contains("https://network.example/v1/bounties/{bounty_id}/funding-contributions")
        );
        assert!(text.contains(GITHUB_ISSUE_TEMPLATE_URL));
        assert!(text.contains("AI judges"));
        assert!(text.contains("Risk policy"));
        assert!(text.contains("https://network.example/v1/risk/policy"));
        assert!(text.contains("Risk review events"));
        assert!(text.contains("Base escrow event reconciliation"));
        assert!(text.contains("EscrowCreated"));
        assert!(text.contains("https://network.example/v1/risk/events"));
        assert!(text.contains("Risk review records"));
        assert!(text.contains("https://network.example/v1/risk/reviews"));
        assert!(text.contains("Risk bounty approvals"));
        assert!(text.contains("https://network.example/v1/risk/bounty-approvals"));
        assert!(text.contains("Agent payout status"));
        assert!(text.contains("https://network.example/v1/agents/{agent_id}/paid-status"));
        assert!(text.contains("Base refund plan"));
        assert!(text.contains("Stripe Connect transfer plan"));
        assert!(text.contains("https://network.example/v1/stripe/connect-transfers"));
        assert!(text.contains("https://network.example/v1/stripe/transfer-events"));
        assert!(text.contains("https://network.example/v1/github/funding-comment-plan"));
        assert!(text.contains("https://network.example/v1/github/proof-comment-plan-from-proof"));
        assert!(discovery_manifest_schema_json().contains("\"$id\""));
        assert!(discovery_manifest_schema_json().contains("\"agent_entrypoints\""));
        assert!(discovery_manifest_schema_json().contains("\"github_funding_comment_plan\""));
        assert!(
            discovery_manifest_schema_json().contains("\"github_proof_comment_from_proof_plan\"")
        );
        assert!(discovery_manifest_schema_json().contains("\"pooled_bounties\""));
        assert!(discovery_manifest_schema_json().contains("\"bounty_funding_intents\""));
        assert!(discovery_manifest_schema_json().contains("\"bounty_funding_contributions\""));
        assert!(discovery_manifest_schema_json().contains("\"funding_feed\""));
        assert!(discovery_manifest_schema_json().contains("\"public_funding\""));
        assert!(discovery_manifest_schema_json().contains("\"public_bounty\""));
    }

    #[test]
    fn public_bounty_feed_excludes_private_or_unclaimable_work() {
        let public_bounty = claimable_bounty("Public fix", 5_000, PrivacyLevel::Public);
        let private_bounty = claimable_bounty("Private fix", 9_000, PrivacyLevel::Private);
        let mut claimed_bounty = claimable_bounty("Claimed fix", 7_000, PrivacyLevel::Public);
        claimed_bounty.claim().unwrap();

        let feed = public_bounty_feed(
            &[public_bounty.clone(), private_bounty, claimed_bounty],
            "https://network.example/",
        );

        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].bounty_id, public_bounty.id.to_string());
        assert_eq!(
            feed[0].claim_url,
            format!(
                "https://network.example/v1/bounties/{}/claim",
                public_bounty.id
            )
        );
        assert_eq!(
            feed[0].template_url,
            "https://network.example/public/templates/fix-ci-failure"
        );
        assert_eq!(
            feed[0].public_url,
            format!(
                "https://network.example/public/bounties/{}",
                public_bounty.id
            )
        );
        assert_eq!(
            feed[0].funding_contribution_url,
            format!(
                "https://network.example/v1/bounties/{}/funding-contributions",
                public_bounty.id
            )
        );
    }

    #[test]
    fn public_bounty_feed_sorts_highest_reward_first() {
        let low = claimable_bounty("Low", 1_000, PrivacyLevel::Public);
        let high = claimable_bounty("High", 3_000, PrivacyLevel::RedactedPublicProof);

        let feed = public_bounty_feed(&[low, high], "https://network.example");

        assert_eq!(feed[0].title, "High");
        assert_eq!(feed[1].title, "Low");
    }

    #[test]
    fn public_capability_feed_includes_active_agents_and_reputation() {
        let mut agent = Agent::new("solver<script>");
        let capability = Capability {
            id: Uuid::new_v4(),
            agent_id: agent.id,
            class: domain::CapabilityClass::Coding,
            template_slugs: vec!["small-code-change".to_string()],
            min_price: Money::new(500_000, "usdc").unwrap(),
            max_price: Money::new(1_000_000, "usdc").unwrap(),
            latency_seconds: 600,
            supported_verifiers: vec![VerifierKind::JsonSchema],
        };
        let reputation = ReputationEvent {
            id: Uuid::new_v4(),
            agent_id: agent.id,
            bounty_id: Uuid::new_v4(),
            capability_class: domain::CapabilityClass::Coding,
            template_slug: "small-code-change".to_string(),
            delta: 10,
            reason: "accepted".to_string(),
            created_at: Utc::now(),
        };

        let feed = public_capability_feed(
            &[capability.clone()],
            &[agent.clone()],
            &[reputation],
            &[],
            "https://network.example/",
        );

        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].agent_id, agent.id.to_string());
        assert_eq!(feed[0].agent_handle, "solver<script>");
        assert_eq!(feed[0].reputation_score, 10);
        assert_eq!(feed[0].accepted_bounties, 1);
        assert_eq!(
            feed[0].agent_profile_url,
            format!("https://network.example/public/agents/{}", agent.id)
        );

        agent.status = AgentStatus::Suspended;
        assert!(public_capability_feed(
            &[capability],
            &[agent],
            &[],
            &[],
            "https://network.example"
        )
        .is_empty());
    }

    #[test]
    fn capability_feed_page_escapes_agent_handles() {
        let item = PublicCapabilityFeedItem {
            capability_id: Uuid::new_v4().to_string(),
            agent_id: Uuid::new_v4().to_string(),
            agent_handle: "<script>alert(1)</script>".to_string(),
            class: "Coding".to_string(),
            template_slugs: vec!["small-code-change".to_string()],
            min_price_minor: 500_000,
            max_price_minor: 1_000_000,
            currency: "usdc".to_string(),
            latency_seconds: 600,
            supported_verifiers: vec!["JsonSchema".to_string()],
            reputation_score: 10,
            accepted_bounties: 1,
            paid_minor: 0,
            agent_profile_url: "/public/agents/1".to_string(),
            request_quotes_url: "/v1/help-requests".to_string(),
        };

        let html = render_capability_feed_page(&[item]);

        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("/v1/capabilities/feed"));
    }

    #[test]
    fn bounty_feed_page_escapes_titles() {
        let item = PublicBountyFeedItem {
            bounty_id: Uuid::new_v4().to_string(),
            title: "<script>alert(1)</script>".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 1_000,
            currency: "usdc".to_string(),
            funding_mode: "BaseUsdcEscrow".to_string(),
            status: "Claimable".to_string(),
            privacy: "Public".to_string(),
            terms_hash: None,
            claim_url: "/claim".to_string(),
            status_url: "/status".to_string(),
            public_url: "/public/bounties/1".to_string(),
            template_url: "/template".to_string(),
            funding_contribution_url: "/fund".to_string(),
            created_at: Utc::now().to_rfc3339(),
        };

        let html = render_bounty_feed_page(&[item]);

        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("/v1/bounties/feed"));
        assert!(html.contains("/public/bounties/1"));
        assert!(html.contains("/fund"));
        assert!(html.contains("Add funding"));
    }

    #[test]
    fn empty_bounty_feed_page_still_points_agents_to_funding_action() {
        let html = render_bounty_feed_page(&[]);

        assert!(html.contains("/v1/bounties/feed"));
        assert!(html.contains("Add funding"));
    }

    #[test]
    fn funding_feed_page_exposes_machine_readable_funding_actions() {
        let item = public_funding_feed_item_fixture(500_000, 500_000, "BaseUsdc");

        let html = render_funding_feed_page(&[item.clone()]);

        assert!(html.contains("Fundable Agent Bounties"));
        assert!(html.contains("agent-bounty-funding-feed"));
        assert!(html.contains(r#"data-agent-action="create_funding_intent""#));
        assert!(html.contains(r#"data-agent-action="add_funding""#));
        assert!(html.contains(&item.funding_intent_url));
        assert!(html.contains(&item.funding_contribution_url));
        assert!(html.contains(&format!(
            "/agent-bounty fund {} 0.5 USDC via BaseUsdc",
            item.bounty_id
        )));
    }

    #[test]
    fn empty_funding_feed_page_still_points_agents_to_machine_feed() {
        let html = render_funding_feed_page(&[]);

        assert!(html.contains("/v1/bounties/funding-feed"));
        assert!(html.contains("No public bounties currently need funding"));
    }

    #[test]
    fn public_bounty_page_exposes_agent_links_and_escapes_metadata() {
        let item = PublicBountyPage {
            bounty_id: Uuid::new_v4().to_string(),
            title: "</script><script>alert(1)</script>".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 1_000,
            currency: "usdc".to_string(),
            funding_mode: "BaseUsdcEscrow".to_string(),
            privacy: "Public".to_string(),
            status: "Claimable".to_string(),
            terms_hash: Some("terms<script>".to_string()),
            created_at: Utc::now().to_rfc3339(),
            verification_type: "GitHubCi".to_string(),
            claimable: true,
            funding_target_minor: 1_000,
            funding_applied_minor: 1_000,
            funding_remaining_minor: 0,
            contribution_count: 1,
            public_url: "https://network.example/public/bounties/1".to_string(),
            claim_url: "https://network.example/v1/bounties/1/claim".to_string(),
            status_url: "https://network.example/v1/bounties/1".to_string(),
            template_url: "https://network.example/public/templates/fix-ci-failure".to_string(),
            funding_contribution_url: "https://network.example/v1/bounties/1/funding-contributions"
                .to_string(),
            proof_urls: vec!["https://network.example/public/proofs/1".to_string()],
            funding_partitions: vec![PublicFundingPartition {
                rail: "BaseUsdc".to_string(),
                target_minor: 1_000,
                confirmed_minor: 1_000,
                remaining_minor: 0,
                currency: "usdc".to_string(),
                contribution_count: 1,
                escrow_count: 1,
                claimable: true,
            }],
            verifier_result_links: vec![PublicBountyRecordLink {
                label: "GitHubCi Accepted verifier result".to_string(),
                url: "https://network.example/public/bounties/1#verifier-results".to_string(),
            }],
            settlement_links: vec![PublicBountyRecordLink {
                label: "BaseUsdc settlement".to_string(),
                url: "https://network.example/public/bounties/1#settlements".to_string(),
            }],
            template_signal_links: vec![PublicBountyRecordLink {
                label: "fix-ci-failure template signal".to_string(),
                url: "https://network.example/public/templates/fix-ci-failure".to_string(),
            }],
        };

        let html = render_public_bounty_page(&item);

        assert!(html.contains("application/ld+json"));
        assert!(html.contains("agent-bounty-public-status"));
        assert!(html.contains("agent-bounty:title"));
        assert!(html.contains("agent-bounty:verification_type"));
        assert!(html.contains("Funding State"));
        assert!(html.contains("Funding partitions"));
        assert!(html.contains("Machine status"));
        assert!(html.contains(r#"data-agent-action="claim""#));
        assert!(!html.contains("Add funding"));
        assert!(!html.contains(r#"rel="payment""#));
        assert!(html.contains("No co-funding action is currently available"));
        assert!(html.contains("BaseUsdc"));
        assert!(html.contains("Verifier Results"));
        assert!(html.contains("Settlement State"));
        assert!(html.contains("Reusable Template Signals"));
        assert!(html.contains("https://network.example/public/proofs/1"));
        assert!(html.contains("https://network.example/public/bounties/1#verifier-results"));
        assert!(!html.contains("https://network.example/v1/bounties/1/funding-contributions"));
        assert!(!html.contains("</script><script>"));
        assert!(html.contains("&lt;/script&gt;&lt;script&gt;"));
    }

    #[test]
    fn public_bounty_page_exposes_cofunding_only_when_funding_remains() {
        let item = public_bounty_page_fixture("Unfunded", 500_000, 500_000, false);

        let html = render_public_bounty_page(&item);

        assert!(html.contains("partially funded"));
        assert!(html.contains("Co-funding command:"));
        assert!(html.contains(&format!(
            "/agent-bounty fund {} 0.5 USDC via BaseUsdc",
            item.bounty_id
        )));
        assert!(html.contains(r#"rel="payment""#));
        assert!(html.contains(r#"data-agent-action="add_funding""#));
        assert!(html.contains("https://network.example/v1/bounties/1/funding-contributions"));
        assert!(!html.contains(r#"data-agent-action="claim""#));
    }

    #[test]
    fn public_bounty_page_uses_remaining_partition_for_mixed_funding() {
        let mut item = public_bounty_page_fixture("Unfunded", 500, 0, false);
        item.currency = "usd".to_string();
        item.funding_mode = "MixedRails".to_string();
        item.funding_target_minor = 500;
        item.funding_applied_minor = 500;
        item.funding_remaining_minor = 0;
        item.funding_partitions = vec![
            PublicFundingPartition {
                rail: "StripeFiat".to_string(),
                target_minor: 500,
                confirmed_minor: 500,
                remaining_minor: 0,
                currency: "usd".to_string(),
                contribution_count: 1,
                escrow_count: 0,
                claimable: true,
            },
            PublicFundingPartition {
                rail: "BaseUsdc".to_string(),
                target_minor: 1_000_000,
                confirmed_minor: 0,
                remaining_minor: 1_000_000,
                currency: "usdc".to_string(),
                contribution_count: 0,
                escrow_count: 0,
                claimable: false,
            },
        ];

        let html = render_public_bounty_page(&item);

        assert!(html.contains("partially funded"));
        assert!(html.contains(&format!(
            "/agent-bounty fund {} 1 USDC via BaseUsdc",
            item.bounty_id
        )));
        assert!(html.contains(r#"data-agent-action="add_funding""#));
    }

    #[test]
    fn public_bounty_page_suppresses_unsafe_payment_actions_for_terminal_states() {
        for status in ["Paid", "Refunded", "Disputed", "Expired"] {
            let item = public_bounty_page_fixture(status, 500_000, 500_000, false);

            let html = render_public_bounty_page(&item);

            assert!(!html.contains("Co-funding command:"), "{status}");
            assert!(!html.contains(r#"rel="payment""#), "{status}");
            assert!(
                !html.contains(r#"data-agent-action="add_funding""#),
                "{status}"
            );
            assert!(!html.contains(r#"data-agent-action="claim""#), "{status}");
        }
    }

    fn public_bounty_page_fixture(
        status: &str,
        applied_minor: i64,
        remaining_minor: i64,
        claimable: bool,
    ) -> PublicBountyPage {
        PublicBountyPage {
            bounty_id: Uuid::new_v4().to_string(),
            title: "Public fix".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: applied_minor + remaining_minor,
            currency: "usdc".to_string(),
            funding_mode: "BaseUsdcEscrow".to_string(),
            privacy: "Public".to_string(),
            status: status.to_string(),
            terms_hash: Some("terms".to_string()),
            created_at: Utc::now().to_rfc3339(),
            verification_type: "GitHubCi".to_string(),
            claimable,
            funding_target_minor: applied_minor + remaining_minor,
            funding_applied_minor: applied_minor,
            funding_remaining_minor: remaining_minor,
            contribution_count: usize::from(applied_minor > 0),
            public_url: "https://network.example/public/bounties/1".to_string(),
            claim_url: "https://network.example/v1/bounties/1/claim".to_string(),
            status_url: "https://network.example/v1/bounties/1".to_string(),
            template_url: "https://network.example/public/templates/fix-ci-failure".to_string(),
            funding_contribution_url: "https://network.example/v1/bounties/1/funding-contributions"
                .to_string(),
            proof_urls: vec![],
            funding_partitions: vec![PublicFundingPartition {
                rail: "BaseUsdc".to_string(),
                target_minor: applied_minor + remaining_minor,
                confirmed_minor: applied_minor,
                remaining_minor,
                currency: "usdc".to_string(),
                contribution_count: usize::from(applied_minor > 0),
                escrow_count: usize::from(applied_minor > 0),
                claimable,
            }],
            verifier_result_links: vec![],
            settlement_links: vec![],
            template_signal_links: vec![],
        }
    }

    fn public_funding_feed_item_fixture(
        applied_minor: i64,
        remaining_minor: i64,
        rail: &str,
    ) -> PublicFundingFeedItem {
        PublicFundingFeedItem {
            bounty_id: Uuid::new_v4().to_string(),
            title: "Fund shared public work".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: applied_minor + remaining_minor,
            currency: "usdc".to_string(),
            funding_mode: "BaseUsdcEscrow".to_string(),
            status: "Unfunded".to_string(),
            privacy: "Public".to_string(),
            terms_hash: Some("terms".to_string()),
            created_at: Utc::now().to_rfc3339(),
            claimable: false,
            funding_target_minor: applied_minor + remaining_minor,
            funding_applied_minor: applied_minor,
            funding_remaining_minor: remaining_minor,
            contribution_count: usize::from(applied_minor > 0),
            public_url: "https://network.example/public/bounties/1".to_string(),
            status_url: "https://network.example/v1/bounties/1".to_string(),
            template_url: "https://network.example/public/templates/fix-ci-failure".to_string(),
            funding_intent_url: "https://network.example/v1/bounties/1/funding-intents".to_string(),
            funding_contribution_url: "https://network.example/v1/bounties/1/funding-contributions"
                .to_string(),
            funding_partitions: vec![PublicFundingPartition {
                rail: rail.to_string(),
                target_minor: applied_minor + remaining_minor,
                confirmed_minor: applied_minor,
                remaining_minor,
                currency: "usdc".to_string(),
                contribution_count: usize::from(applied_minor > 0),
                escrow_count: usize::from(applied_minor > 0),
                claimable: false,
            }],
        }
    }

    fn claimable_bounty(title: &str, amount_minor: i64, privacy: PrivacyLevel) -> Bounty {
        let mut bounty = Bounty::new(
            title,
            "fix-ci-failure",
            Money::new(amount_minor, "usdc").unwrap(),
            FundingMode::BaseUsdcEscrow,
            privacy,
        );
        bounty.mark_funded("terms").unwrap();
        bounty.make_claimable().unwrap();
        bounty
    }
}
