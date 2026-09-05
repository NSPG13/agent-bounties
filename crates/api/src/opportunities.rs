use app::BountyStatusResponse;
use chain_base::{
    standing_meta_v2_parent_context, AutonomousBountyFeedItem, OpenCompetitionDeploymentState,
    OpenCompetitionEvent, OpenCompetitionEventKind, OpenCompetitionV2Event,
    OpenCompetitionV2EventKind, OpenCompetitionV2MetricProgramRelease,
    OpenCompetitionV2ProgramClassification, OpenCompetitionV2ProjectedState,
    OpenCompetitionV2Release, OpenCompetitionVerifierProfile,
};
use chrono::{DateTime, Utc};
use db::{OpenCompetitionV2StoredProjection, TrialBounty, UnfundedBountySolution};
use domain::{BountyStatus, DiscoveryOpportunitySnapshot, DiscoveryRewardFilter, PrivacyLevel};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;
use web_public::escape_html;

pub const OPPORTUNITY_PROJECTION_SCHEMA: &str = "agent-bounties/opportunity-projection-v1";
const OPEN_COMPETITION_V2_METADATA_JSON: &str =
    include_str!("../../../ops/open-competition-v2-public-metadata-v1.json");

#[derive(Debug, Clone, Copy)]
pub struct OpenCompetitionV2HostedCosts {
    pub proof_fee: u128,
    pub relay_fee: u128,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenCompetitionV2PublicMetadataRegistry {
    schema_version: String,
    network: String,
    factory_contract: String,
    competitions: Vec<OpenCompetitionV2PublicMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenCompetitionV2PublicMetadata {
    seed_id: String,
    bounty_id: String,
    competition: String,
    title: String,
    summary: String,
    source_url: String,
    epoch_starts_at: Option<String>,
    epoch_ends_at: Option<String>,
    minimum_score_base_units: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpportunityQuery {
    pub network: Option<String>,
    pub view: Option<String>,
    pub source_type: Option<String>,
    pub work_state: Option<String>,
    pub payment_state: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityAmount {
    pub amount: String,
    pub currency: String,
    pub unit: String,
    pub decimals: u8,
}

impl OpportunityAmount {
    fn usdc_base_units(amount: impl Into<String>) -> Self {
        Self {
            amount: amount.into(),
            currency: "USDC".to_string(),
            unit: "base_units".to_string(),
            decimals: 6,
        }
    }

    fn minor_units(amount: i64, currency: &str) -> Self {
        Self {
            amount: amount.to_string(),
            currency: currency.to_ascii_uppercase(),
            unit: "minor_units".to_string(),
            decimals: if currency.eq_ignore_ascii_case("usdc") {
                6
            } else {
                2
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct OpportunityNextAction {
    pub action: String,
    pub method: String,
    pub url: String,
    pub body_template: Option<Value>,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityEmbedLinks {
    pub html: String,
    pub svg: String,
    pub markdown: String,
    pub iframe: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityImage {
    pub source: String,
    pub prompt: Option<String>,
    pub alt_text: String,
    pub asset_url: String,
    pub sha256: Option<String>,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityCashEconomics {
    pub solver_reward: OpportunityAmount,
    pub refundable_claim_bond: OpportunityAmount,
    pub required_external_spend: OpportunityAmount,
    pub gross_cash_margin: OpportunityAmount,
    pub gross_cash_margin_positive: bool,
    pub scope_disclaimer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityStandingMetaV4Economics {
    pub parent_solver_reward: OpportunityAmount,
    pub parent_verifier_reward: OpportunityAmount,
    pub maximum_required_child_outlay: OpportunityAmount,
    pub successful_settlement_margin: OpportunityAmount,
    pub gas_sponsorship_available: bool,
    pub scope_disclaimer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityAnonymousSeparation {
    pub stake_per_wallet_role: OpportunityAmount,
    pub sortition_mechanism: String,
    pub candidate_count: u16,
    pub selection_proof: Option<String>,
    pub selection_status: String,
    pub identity_required: bool,
    pub unrelated_owner_proven: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityVerifierGovernance {
    pub governance: String,
    pub vrf_request_id: Option<String>,
    pub vrf_proof: Option<String>,
    pub primary_policy: String,
    pub appellate_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityAppealPolicy {
    pub eligible_appellants: Vec<String>,
    pub bond: OpportunityAmount,
    pub appeal_deadline_seconds: u64,
    pub jury_size: u8,
    pub threshold: u8,
    pub current_state: String,
    pub immediate_waiver_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityStandingMetaV4Coordination {
    pub competition_mode: String,
    pub atomic_claim_required: bool,
    pub child_lifecycle: String,
    pub per_bounty_enrollment_seconds: u64,
    pub selected_solver_response_seconds: u64,
    pub timing_safety: String,
    pub why_not_first_valid: String,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunityStandingMetaV4 {
    pub economics: OpportunityStandingMetaV4Economics,
    pub anonymous_separation: OpportunityAnonymousSeparation,
    pub verifier_governance: OpportunityVerifierGovernance,
    pub appeal_policy: OpportunityAppealPolicy,
    pub coordination: OpportunityStandingMetaV4Coordination,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct OpportunityItem {
    pub opportunity_id: String,
    pub source_type: String,
    pub source_id: String,
    pub source_status: String,
    pub title: String,
    pub goal: Option<String>,
    pub categories: Vec<String>,
    pub skills: Vec<String>,
    pub public_url: String,
    pub source_url: Option<String>,
    pub work_state: String,
    pub payment_state: String,
    pub payment_committed: bool,
    pub competition_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verifier_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verifier_profile_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_count: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_entries: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub competition_ends_at: Option<u64>,
    pub standing_meta_bounty: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cash_economics: Option<OpportunityCashEconomics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standing_meta_v4: Option<OpportunityStandingMetaV4>,
    pub decision_authority: String,
    pub payment_authority: String,
    pub reward: OpportunityAmount,
    pub completion_bonus: Option<OpportunityAmount>,
    pub funded_amount: OpportunityAmount,
    pub funding_target: OpportunityAmount,
    pub bond: OpportunityAmount,
    pub refundable_bond: OpportunityAmount,
    pub external_spend: OpportunityAmount,
    pub gross_cash_margin: OpportunityAmount,
    pub deadline: Option<String>,
    pub deadline_kind: Option<String>,
    pub verification_method: String,
    pub verification_ready: bool,
    pub evidence_requirements: Value,
    pub terms_hash: Option<String>,
    pub proof_urls: Vec<String>,
    pub next_action: OpportunityNextAction,
    pub embeds: OpportunityEmbedLinks,
    pub image: OpportunityImage,
    pub discovery_factors: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct OpportunitySourceStatus {
    pub source_type: String,
    pub available: bool,
    pub authoritative_urls: Vec<String>,
    pub item_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct OpportunityProjectionResponse {
    pub schema_version: String,
    pub generated_at: String,
    pub network: String,
    pub applied_view: Option<String>,
    pub degraded: bool,
    pub source_statuses: Vec<OpportunitySourceStatus>,
    pub items: Vec<OpportunityItem>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpportunityFeedDocuments {
    pub rss: String,
    pub atom: String,
    pub json: String,
    pub updated_at: String,
}

pub fn render_opportunity_feeds(
    projection: &OpportunityProjectionResponse,
    public_base_url: &str,
) -> OpportunityFeedDocuments {
    let public_base_url = public_base_url.trim_end_matches('/');
    let feed_root = format!("{public_base_url}/v1/opportunities");
    let updated_at = projection
        .items
        .iter()
        .filter_map(|item| DateTime::parse_from_rfc3339(&item.updated_at).ok())
        .max()
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|| DateTime::UNIX_EPOCH.with_timezone(&Utc));
    let rss_date = updated_at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let atom_date = updated_at.to_rfc3339();

    let mut rss_items = String::new();
    let mut atom_entries = String::new();
    let mut json_items = Vec::with_capacity(projection.items.len());
    for item in &projection.items {
        let summary = feed_summary(item);
        let title = escape_html(&item.title);
        let public_url = escape_html(&item.public_url);
        let opportunity_id = escape_html(&item.opportunity_id);
        let published = normalized_feed_date(&item.created_at);
        let modified = normalized_feed_date(&item.updated_at);
        let rss_modified = DateTime::parse_from_rfc3339(&modified)
            .map(|value| {
                value
                    .with_timezone(&Utc)
                    .format("%a, %d %b %Y %H:%M:%S GMT")
                    .to_string()
            })
            .unwrap_or_else(|_| rss_date.clone());

        rss_items.push_str(&format!(
            "<item><title>{title}</title><link>{public_url}</link><guid isPermaLink=\"false\">{opportunity_id}</guid><description>{}</description><pubDate>{rss_modified}</pubDate><category>{}</category><category>{}</category><category>{}</category></item>",
            escape_html(&summary),
            escape_html(&item.work_state),
            escape_html(&item.payment_state),
            escape_html(&item.source_type),
        ));

        let category_elements = item
            .categories
            .iter()
            .chain(item.skills.iter())
            .map(|value| format!("<category term=\"{}\"/>", escape_html(value)))
            .collect::<String>();
        atom_entries.push_str(&format!(
            "<entry><id>urn:bountyboard:{opportunity_id}</id><title>{title}</title><link href=\"{public_url}\"/><published>{published}</published><updated>{modified}</updated><summary type=\"text\">{}</summary><category term=\"{}\"/><category term=\"{}\"/><category term=\"{}\"/>{category_elements}</entry>",
            escape_html(&summary),
            escape_html(&item.work_state),
            escape_html(&item.payment_state),
            escape_html(&item.source_type),
        ));

        json_items.push(json!({
            "id": item.opportunity_id,
            "url": item.public_url,
            "title": item.title,
            "image": item.image.asset_url,
            "content_text": summary,
            "date_published": published,
            "date_modified": modified,
            "tags": feed_tags(item),
            "_bountyboard": {
                "source_type": item.source_type,
                "work_state": item.work_state,
                "payment_state": item.payment_state,
                "payment_committed": item.payment_committed,
                "reward": item.reward,
                "cash_economics": item.cash_economics,
                "verification_method": item.verification_method,
                "verification_ready": item.verification_ready,
                "terms_hash": item.terms_hash,
                "image": item.image,
                "next_action": item.next_action,
                "evidence_boundary": item.evidence_boundary,
            }
        }));
    }

    let rss_url = format!("{feed_root}/feed.rss");
    let atom_url = format!("{feed_root}/feed.atom");
    let json_url = format!("{feed_root}/feed.json");
    let rss = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><rss version=\"2.0\"><channel><title>Agent Bounties opportunities</title><link>{}</link><description>Public funded and unfunded work discoverable by agents. Payment state is explicit.</description><lastBuildDate>{rss_date}</lastBuildDate><atom:link xmlns:atom=\"http://www.w3.org/2005/Atom\" href=\"{}\" rel=\"self\" type=\"application/rss+xml\"/>{rss_items}</channel></rss>",
        escape_html(public_base_url),
        escape_html(&rss_url),
    );
    let atom = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><id>{}</id><title>Agent Bounties opportunities</title><updated>{atom_date}</updated><link href=\"{}\"/><link href=\"{}\" rel=\"self\" type=\"application/atom+xml\"/><subtitle>Public funded and unfunded work discoverable by agents. Payment state is explicit.</subtitle>{atom_entries}</feed>",
        escape_html(&atom_url),
        escape_html(public_base_url),
        escape_html(&atom_url),
    );
    let json = serde_json::to_string_pretty(&json!({
        "version": "https://jsonfeed.org/version/1.1",
        "title": "Agent Bounties opportunities",
        "home_page_url": public_base_url,
        "feed_url": json_url,
        "description": "Public funded and unfunded work discoverable by agents. Payment state and commitment are explicit; only canonical settlement proves payment.",
        "items": json_items,
    }))
    .unwrap_or_else(|_| "{\"version\":\"https://jsonfeed.org/version/1.1\",\"items\":[]}".to_string());

    OpportunityFeedDocuments {
        rss,
        atom,
        json,
        updated_at: rss_date,
    }
}

fn feed_summary(item: &OpportunityItem) -> String {
    let reward = if item.payment_committed {
        format!(
            "Committed reward: {} {} ({}; {} decimals).",
            item.reward.amount, item.reward.currency, item.reward.unit, item.reward.decimals
        )
    } else if item.reward.amount != "0" {
        format!(
            "Proposed reward: {} {} ({}; {} decimals); payment is not committed.",
            item.reward.amount, item.reward.currency, item.reward.unit, item.reward.decimals
        )
    } else {
        "No payment is committed.".to_string()
    };
    let goal = item
        .goal
        .as_deref()
        .unwrap_or("No additional goal text was supplied.");
    let cash_economics = item
        .cash_economics
        .as_ref()
        .map_or_else(String::new, |economics| {
            format!(
                " Solver reward: {} {} base units. Refundable claim bond: {} {} base units. Required external spend: {} {} base units. Gross cash margin (not net profit): {} {} base units. {}",
                economics.solver_reward.amount,
                economics.solver_reward.currency,
                economics.refundable_claim_bond.amount,
                economics.refundable_claim_bond.currency,
                economics.required_external_spend.amount,
                economics.required_external_spend.currency,
                economics.gross_cash_margin.amount,
                economics.gross_cash_margin.currency,
                economics.scope_disclaimer,
            )
        });
    truncate_feed_text(
        &format!(
            "{goal}\n\nWork state: {}. Payment state: {}. {reward}{cash_economics} Verification: {}. Next action: {}",
            item.work_state,
            item.payment_state,
            item.verification_method,
            item.next_action.instructions
        ),
        2_000,
    )
}

fn feed_tags(item: &OpportunityItem) -> Vec<String> {
    let mut tags = vec![
        item.source_type.clone(),
        item.work_state.clone(),
        item.payment_state.clone(),
    ];
    tags.extend(item.categories.iter().cloned());
    tags.extend(item.skills.iter().cloned());
    tags.sort();
    tags.dedup();
    tags
}

fn normalized_feed_date(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_else(|_| DateTime::UNIX_EPOCH.with_timezone(&Utc).to_rfc3339())
}

fn truncate_feed_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

impl OpportunityItem {
    pub fn discovery_snapshot(&self) -> DiscoveryOpportunitySnapshot {
        DiscoveryOpportunitySnapshot {
            opportunity_id: self.opportunity_id.clone(),
            source_type: self.source_type.clone(),
            categories: self.categories.clone(),
            skills: self.skills.clone(),
            work_state: self.work_state.clone(),
            payment_state: self.payment_state.clone(),
            payment_committed: self.payment_committed,
            reward: DiscoveryRewardFilter {
                amount: self.reward.amount.clone(),
                currency: self.reward.currency.clone(),
                unit: self.reward.unit.clone(),
                decimals: self.reward.decimals,
            },
            deadline: self.deadline.as_deref().and_then(|deadline| {
                DateTime::parse_from_rfc3339(deadline)
                    .ok()
                    .map(|deadline| deadline.with_timezone(&Utc))
            }),
            verification_method: self.verification_method.clone(),
            public_url: self.public_url.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpportunityView {
    Recent,
    Engineering,
    Creative,
    Urgent,
    SeekingFunding,
    ReadyToEarn,
}

impl OpportunityView {
    pub fn parse(value: Option<&str>) -> Result<Option<Self>, ()> {
        value
            .map(|value| match value.trim().to_ascii_lowercase().as_str() {
                "recent" => Ok(Self::Recent),
                "engineering" => Ok(Self::Engineering),
                "creative" => Ok(Self::Creative),
                "urgent" => Ok(Self::Urgent),
                "seeking_funding" => Ok(Self::SeekingFunding),
                "ready_to_earn" => Ok(Self::ReadyToEarn),
                _ => Err(()),
            })
            .transpose()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Engineering => "engineering",
            Self::Creative => "creative",
            Self::Urgent => "urgent",
            Self::SeekingFunding => "seeking_funding",
            Self::ReadyToEarn => "ready_to_earn",
        }
    }
}

pub fn unfunded_opportunity(
    trial: &TrialBounty,
    solutions: &[UnfundedBountySolution],
    api_base_url: &str,
) -> OpportunityItem {
    let api = api_base_url.trim_end_matches('/');
    let work_state = if solutions.is_empty() {
        "open"
    } else {
        "submitted"
    };
    let public_url = format!("{api}/v1/unfunded-bounties/{}", trial.id);
    let opportunity_id = format!("unfunded:{}", trial.id);
    let evidence_requirements = json!({
        "acceptance_criteria": trial.acceptance_criteria,
        "solution_fields": ["summary", "deliverable_markdown", "evidence"]
    });
    let (categories, skills, keyword_matches) = web_public::discovery_taxonomy_with_matches(
        &trial.title,
        Some(&trial.goal),
        &evidence_requirements,
    );
    let embeds = opportunity_embed_links(api, &opportunity_id, None);
    let image = fallback_opportunity_image(&embeds, &trial.title);
    OpportunityItem {
        opportunity_id: opportunity_id.clone(),
        source_type: "unfunded_offchain".to_string(),
        source_id: trial.id.to_string(),
        source_status: trial.status.clone(),
        title: trial.title.clone(),
        goal: Some(trial.goal.clone()),
        categories,
        skills,
        public_url,
        source_url: trial.source_url.clone(),
        work_state: work_state.to_string(),
        payment_state: "none".to_string(),
        payment_committed: false,
        competition_mode: "open_unfunded_submission".to_string(),
        network: None,
        verifier_profile_id: None,
        verifier_profile_name: None,
        entry_count: None,
        max_entries: None,
        competition_ends_at: None,
        standing_meta_bounty: false,
        cash_economics: None,
        standing_meta_v4: None,
        decision_authority: "The poster reviews this offchain submission; no canonical verifier is configured.".to_string(),
        payment_authority: "None. This opportunity is unfunded and creates no payment promise.".to_string(),
        reward: OpportunityAmount::usdc_base_units("0"),
        completion_bonus: None,
        funded_amount: OpportunityAmount::usdc_base_units("0"),
        funding_target: OpportunityAmount::usdc_base_units("0"),
        bond: OpportunityAmount::usdc_base_units("0"),
        refundable_bond: OpportunityAmount::usdc_base_units("0"),
        external_spend: OpportunityAmount::usdc_base_units("0"),
        gross_cash_margin: OpportunityAmount::usdc_base_units("0"),
        deadline: Some(trial.expires_at.to_rfc3339()),
        deadline_kind: Some("publication_expires_at".to_string()),
        verification_method: "poster_review_or_unspecified".to_string(),
        verification_ready: false,
        evidence_requirements,
        terms_hash: None,
        proof_urls: Vec::new(),
        next_action: OpportunityNextAction {
            action: "submit_unfunded_bounty_solution".to_string(),
            method: "POST".to_string(),
            url: format!("{api}/v1/unfunded-bounties/{}/solutions", trial.id),
            body_template: Some(json!({
                "agent_id": "<registered agent UUID>",
                "summary": "<bounded public summary>",
                "deliverable_markdown": "<complete deliverable>",
                "evidence": {}
            })),
            instructions: "A registered agent may submit public work. No payment claim or promise is created.".to_string(),
        },
        embeds,
        image,
        discovery_factors: base_factors(
            "unfunded_offchain",
            work_state,
            "none",
            &keyword_matches,
        ),
        created_at: trial.created_at.to_rfc3339(),
        updated_at: solutions
            .iter()
            .map(|solution| solution.updated_at)
            .max()
            .unwrap_or(trial.created_at)
            .to_rfc3339(),
        evidence_boundary: "This is a public off-chain opportunity with no committed payment. Agent solutions are public submissions, not canonical claims, verification, settlement, or payment evidence.".to_string(),
    }
}

pub fn legacy_opportunity(
    status: &BountyStatusResponse,
    api_base_url: &str,
) -> Option<OpportunityItem> {
    let bounty = &status.bounty;
    if bounty.privacy == PrivacyLevel::Private
        || matches!(
            bounty.status,
            BountyStatus::Refunding
                | BountyStatus::Refunded
                | BountyStatus::Disputed
                | BountyStatus::Expired
        )
    {
        return None;
    }
    let api = api_base_url.trim_end_matches('/');
    let (work_state, payment_state, payment_committed) = legacy_states(status);
    let source_status = legacy_status_name(&bounty.status);
    let public_url = format!("{api}/public/bounties/{}", bounty.id);
    let opportunity_id = format!("legacy:{}", bounty.id);
    let next_action = legacy_next_action(status, api, work_state, payment_state);
    let updated_at = status
        .settlements
        .iter()
        .map(|record| record.created_at)
        .chain(status.proofs.iter().map(|record| record.created_at))
        .chain(
            status
                .verifier_results
                .iter()
                .map(|record| record.created_at),
        )
        .chain(status.submissions.iter().map(|record| record.submitted_at))
        .chain(status.claims.iter().map(|record| record.claimed_at))
        .max()
        .unwrap_or(bounty.created_at);
    let proof_urls = status
        .proofs
        .iter()
        .map(|proof| format!("{api}/public/proofs/{}", proof.id))
        .collect();
    let verification_method = status
        .verifier_results
        .last()
        .map(|result| format!("legacy_{:?}", result.kind).to_ascii_lowercase())
        .unwrap_or_else(|| format!("template:{}", bounty.template_slug));
    let evidence_requirements = json!({
        "template_slug": bounty.template_slug,
        "terms_hash": bounty.terms_hash,
        "status_url": format!("{api}/v1/bounties/{}", bounty.id)
    });
    let (categories, skills, keyword_matches) =
        web_public::discovery_taxonomy_with_matches(&bounty.title, None, &evidence_requirements);
    let embeds = opportunity_embed_links(api, &opportunity_id, None);
    let image = fallback_opportunity_image(&embeds, &bounty.title);
    Some(OpportunityItem {
        opportunity_id: opportunity_id.clone(),
        source_type: "legacy_bounty".to_string(),
        source_id: bounty.id.to_string(),
        source_status: source_status.to_string(),
        title: bounty.title.clone(),
        goal: None,
        categories,
        skills,
        public_url,
        source_url: None,
        work_state: work_state.to_string(),
        payment_state: payment_state.to_string(),
        payment_committed,
        competition_mode: "exclusive_claim".to_string(),
        network: None,
        verifier_profile_id: None,
        verifier_profile_name: None,
        entry_count: None,
        max_entries: None,
        competition_ends_at: None,
        standing_meta_bounty: false,
        cash_economics: None,
        standing_meta_v4: None,
        decision_authority: format!("Legacy configured verification path: {verification_method}."),
        payment_authority: "The configured legacy reconciled rail; this is not canonical Base BountySettled evidence.".to_string(),
        reward: OpportunityAmount::minor_units(bounty.amount.amount, &bounty.amount.currency),
        completion_bonus: None,
        funded_amount: OpportunityAmount::minor_units(
            status.funding_summary.applied.amount,
            &status.funding_summary.applied.currency,
        ),
        funding_target: OpportunityAmount::minor_units(
            status.funding_summary.target.amount,
            &status.funding_summary.target.currency,
        ),
        bond: OpportunityAmount::minor_units(0, &bounty.amount.currency),
        refundable_bond: OpportunityAmount::minor_units(0, &bounty.amount.currency),
        external_spend: OpportunityAmount::minor_units(0, &bounty.amount.currency),
        gross_cash_margin: OpportunityAmount::minor_units(bounty.amount.amount, &bounty.amount.currency),
        deadline: None,
        deadline_kind: None,
        verification_method,
        verification_ready: matches!(
            bounty.status,
            BountyStatus::Claimable
                | BountyStatus::Claimed
                | BountyStatus::Submitted
                | BountyStatus::Verifying
                | BountyStatus::Accepted
                | BountyStatus::Payable
                | BountyStatus::Paid
        ),
        evidence_requirements,
        terms_hash: bounty.terms_hash.clone(),
        proof_urls,
        next_action,
        embeds,
        image,
        discovery_factors: base_factors(
            "legacy_bounty",
            work_state,
            payment_state,
            &keyword_matches,
        ),
        created_at: bounty.created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
        evidence_boundary: "This legacy platform record is not canonical Base autonomous-v1 evidence. Its payment state follows the configured reconciled rail; only canonical BountySettled proves payment for autonomous-v1 bounties.".to_string(),
    })
}

pub fn canonical_opportunity(
    item: &AutonomousBountyFeedItem,
    network: &str,
    api_base_url: &str,
) -> Option<OpportunityItem> {
    if item.status == "cancelled" {
        return None;
    }
    let api = api_base_url.trim_end_matches('/');
    let funded = item.funded_amount.parse::<u128>().unwrap_or_default();
    let target = item.target_amount.parse::<u128>().unwrap_or_default();
    let state = web_public::canonical_opportunity_state(item);
    let work_state = state.work_state.as_str();
    let payment_state = state.payment_state.as_str();
    let payment_committed = state.payment_committed;
    let terms = item.terms.as_ref();
    let deadline = state.deadline;
    let deadline_kind = state.deadline_kind;
    let evidence_requirements = terms
        .map(|record| record.document.evidence_schema.clone())
        .unwrap_or(Value::Null);
    let title = terms
        .map(|record| record.document.title.clone())
        .unwrap_or_else(|| item.bounty_id.clone());
    let goal = terms.map(|record| record.document.goal.clone());
    let (categories, skills, keyword_matches) = web_public::discovery_taxonomy_with_matches(
        &title,
        goal.as_deref(),
        &evidence_requirements,
    );
    let public_url = terms
        .and_then(|record| record.document.source_url.clone())
        .unwrap_or_else(|| {
            format!(
                "{api}/v1/base/autonomous-bounties/events?network={network}&bounty_id={}",
                item.bounty_id
            )
        });
    let next_action = canonical_next_action(
        item,
        network,
        api,
        work_state,
        payment_state,
        funded,
        target,
    );
    let updated_at = item
        .events
        .last()
        .map(|event| event.occurred_at)
        .or_else(|| terms.map(|record| record.created_at))
        .unwrap_or_else(Utc::now);
    let proof_urls = (item.status == "paid")
        .then(|| {
            format!(
                "{api}/v1/base/autonomous-bounties/events?network={network}&bounty_id={}",
                item.bounty_id
            )
        })
        .into_iter()
        .collect();
    let (external_spend, gross_cash_margin) = if let Ok(ctx) = standing_meta_v2_parent_context(item) {
        let external_amount = ctx.child_target.amount;
        let solver_amount = ctx.solver_reward.amount;
        let margin = solver_amount - external_amount;
        (
            OpportunityAmount::usdc_base_units(external_amount.to_string()),
            OpportunityAmount::usdc_base_units(margin.to_string()),
        )
    } else {
        (
            OpportunityAmount::usdc_base_units("0"),
            OpportunityAmount::usdc_base_units(item.solver_reward.clone()),
        )
    };
    let opportunity_id = format!("canonical:{network}:{}", item.bounty_contract);
    let embeds = opportunity_embed_links(api, &opportunity_id, Some(network));
    let image = terms
        .and_then(|record| record.document.image.as_ref())
        .map(|image| OpportunityImage {
            source: image.source.clone(),
            prompt: Some(image.prompt.clone()),
            alt_text: image.alt_text.clone(),
            asset_url: image.asset_url.clone(),
            sha256: Some(image.sha256.clone()),
            mime_type: image.mime_type.clone(),
        })
        .unwrap_or_else(|| fallback_opportunity_image(&embeds, &title));
    let gross_cash_margin = item.gross_cash_margin.parse::<i128>().ok()?;
    let cash_economics = OpportunityCashEconomics {
        solver_reward: OpportunityAmount::usdc_base_units(item.solver_reward.clone()),
        refundable_claim_bond: OpportunityAmount::usdc_base_units(item.claim_bond.clone()),
        required_external_spend: OpportunityAmount::usdc_base_units(
            item.required_external_spend.clone(),
        ),
        gross_cash_margin: OpportunityAmount::usdc_base_units(item.gross_cash_margin.clone()),
        gross_cash_margin_positive: gross_cash_margin > 0,
        scope_disclaimer: "Gross cash margin is solver reward minus required external spend. It excludes gas, taxes, execution costs, failure risk, and other costs; the claim bond is refundable only under the committed lifecycle rules. It is not guaranteed net profit.".to_string(),
    };
    let verification_method =
        if item.verification_mode == "signed_quorum" && item.verifier_threshold == Some(1) {
            "single_verifier".to_string()
        } else {
            item.verification_mode.clone()
        };
    Some(OpportunityItem {
        opportunity_id: opportunity_id.clone(),
        source_type: "canonical_base".to_string(),
        source_id: item.bounty_contract.clone(),
        source_status: item.status.clone(),
        title,
        goal,
        categories,
        skills,
        public_url,
        source_url: terms.and_then(|record| record.document.source_url.clone()),
        work_state: work_state.to_string(),
        payment_state: payment_state.to_string(),
        payment_committed,
        competition_mode: "exclusive_claim".to_string(),
        network: Some(network.to_string()),
        verifier_profile_id: None,
        verifier_profile_name: None,
        entry_count: None,
        max_entries: None,
        competition_ends_at: None,
        standing_meta_bounty: standing_meta_v2_parent_context(item).is_ok(),
        cash_economics: Some(cash_economics),
        standing_meta_v4: None,
        decision_authority: format!(
            "The immutable canonical verification mode/module configured on {} decides the submission result.",
            item.bounty_contract
        ),
        payment_authority: format!(
            "The exact canonical bounty contract {} controls escrow; only its confirmed BountySettled event proves payment.",
            item.bounty_contract
        ),
        reward: OpportunityAmount::usdc_base_units(item.solver_reward.clone()),
        completion_bonus: Some(OpportunityAmount::usdc_base_units(
            item.timeout_bond_pool.clone(),
        )),
        funded_amount: OpportunityAmount::usdc_base_units(item.funded_amount.clone()),
        funding_target: OpportunityAmount::usdc_base_units(item.target_amount.clone()),
        bond: OpportunityAmount::usdc_base_units(item.claim_bond.clone()),
        refundable_bond: OpportunityAmount::usdc_base_units(item.claim_bond.clone()),
        external_spend,
        gross_cash_margin,
        deadline,
        deadline_kind,
        verification_method,
        verification_ready: state.verification_ready,
        evidence_requirements,
        terms_hash: Some(item.terms_hash.clone()),
        proof_urls,
        next_action,
        embeds,
        image,
        discovery_factors: base_factors(
            "canonical_base",
            work_state,
            payment_state,
            &keyword_matches,
        ),
        created_at: item
            .events
            .first()
            .map(|event| event.occurred_at)
            .or_else(|| terms.map(|record| record.created_at))
            .unwrap_or(updated_at)
            .to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
        evidence_boundary: "Canonical lifecycle and payment language require confirmed factory/bounty events. Payment is `paid` only after confirmed BountySettled; a plan, signature, transaction hash, hosted row, or AI analysis is not payment evidence.".to_string(),
    })
}

fn reviewed_v2_profile<'a>(
    release: &'a OpenCompetitionV2Release,
    record: &OpenCompetitionV2StoredProjection,
) -> Option<&'a OpenCompetitionV2MetricProgramRelease> {
    let projection = &record.projection;
    let matches = |actual: &Option<String>, expected: &str| {
        actual
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(expected))
    };
    release.metric_programs.iter().find(|profile| {
        profile.classification == OpenCompetitionV2ProgramClassification::Reviewed
            && matches(&projection.program_vkey, &profile.program_vkey)
            && matches(&projection.source_hash, &profile.source_hash)
            && matches(&projection.elf_hash, &profile.elf_hash)
            && matches(
                &projection.journal_schema_hash,
                &profile.journal_schema_hash,
            )
            && matches(
                &projection.metric_program_hash,
                &profile.metric_program_hash,
            )
            && profile.review_evidence_hash.starts_with("0x")
            && profile.review_evidence_hash.len() == 66
            && profile
                .review_evidence_hash
                .as_bytes()
                .iter()
                .skip(2)
                .any(|byte| *byte != b'0')
    })
}

fn v2_public_metadata(
    release: &OpenCompetitionV2Release,
) -> Result<BTreeMap<String, OpenCompetitionV2PublicMetadata>, String> {
    let registry: OpenCompetitionV2PublicMetadataRegistry =
        serde_json::from_str(OPEN_COMPETITION_V2_METADATA_JSON)
            .map_err(|error| format!("invalid Open Competition V2 metadata: {error}"))?;
    if registry.schema_version != "agent-bounties/open-competition-v2-public-metadata-v1" {
        return Err("Open Competition V2 public metadata schema mismatch".to_string());
    }
    // Display metadata is not canonical inventory evidence. It remains optional
    // for ordinary contracts, but forward-GMV competitions are not participation
    // ready without their exact scoring window and public terms.
    if registry.network != release.network
        || !registry
            .factory_contract
            .eq_ignore_ascii_case(&release.factory_contract)
    {
        return Ok(BTreeMap::new());
    }
    let mut indexed = BTreeMap::new();
    for item in registry.competitions {
        if item.seed_id.trim().is_empty()
            || item.title.trim().is_empty()
            || item.summary.trim().is_empty()
            || !reviewed_v2_public_source_url(&item.source_url)
            || item.bounty_id.len() != 66
            || item.competition.len() != 42
        {
            return Err("Open Competition V2 public metadata entry is malformed".to_string());
        }
        match (&item.epoch_starts_at, &item.epoch_ends_at) {
            (Some(starts_at), Some(ends_at)) => {
                let starts_at = DateTime::parse_from_rfc3339(starts_at)
                    .map_err(|_| "Open Competition V2 metadata start is malformed")?;
                let ends_at = DateTime::parse_from_rfc3339(ends_at)
                    .map_err(|_| "Open Competition V2 metadata end is malformed")?;
                if starts_at >= ends_at
                    || item
                        .minimum_score_base_units
                        .as_deref()
                        .is_none_or(|value| {
                            value.parse::<u128>().ok().is_none_or(|value| value == 0)
                        })
                {
                    return Err(
                        "Open Competition V2 metadata scoring window is malformed".to_string()
                    );
                }
            }
            (None, None) if item.minimum_score_base_units.is_none() => {}
            _ => {
                return Err("Open Competition V2 metadata scoring window is incomplete".to_string())
            }
        }
        let key = item.competition.to_ascii_lowercase();
        if indexed.insert(key, item).is_some() {
            return Err("Open Competition V2 public metadata contains a duplicate".to_string());
        }
    }
    Ok(indexed)
}

fn reviewed_v2_public_source_url(source_url: &str) -> bool {
    if source_url.starts_with("https://github.com/NSPG13/agent-bounties/issues/") {
        return true;
    }
    let Some(remainder) = source_url.strip_prefix("https://github.com/NSPG13/agent-bounties/blob/")
    else {
        return false;
    };
    let Some((revision, path)) = remainder.split_once('/') else {
        return false;
    };
    revision.len() == 40
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && matches!(
            path,
            "ops/open-competition-v2-forward-gmv-candidate-pool-v2.json"
                | "ops/open-competition-v2-forward-gmv-reward-cohort-v1.json"
        )
}

fn v2_scoring_phase(
    metadata: Option<&OpenCompetitionV2PublicMetadata>,
    now: DateTime<Utc>,
) -> Option<&'static str> {
    let item = metadata?;
    let starts_at = DateTime::parse_from_rfc3339(item.epoch_starts_at.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    let ends_at = DateTime::parse_from_rfc3339(item.epoch_ends_at.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    Some(if now < starts_at {
        "upcoming"
    } else if now < ends_at {
        "scoring"
    } else {
        "proof"
    })
}

pub fn open_competition_v2_opportunities(
    records: &[OpenCompetitionV2StoredProjection],
    events: &[OpenCompetitionV2Event],
    release: &OpenCompetitionV2Release,
    network: &str,
    api_base_url: &str,
    website_base_url: &str,
    hosted_costs: OpenCompetitionV2HostedCosts,
    now: DateTime<Utc>,
) -> Result<Vec<OpportunityItem>, String> {
    if release.protocol_version != "agent-bounties/open-competition-v2-beta3"
        || release.network != network
        || !release.public_creation_enabled
    {
        return Ok(Vec::new());
    }
    let api = api_base_url.trim_end_matches('/');
    let website = website_base_url.trim_end_matches('/');
    let metadata = v2_public_metadata(release)?;
    let external_spend = hosted_costs
        .proof_fee
        .checked_add(hosted_costs.relay_fee)
        .ok_or_else(|| "Open Competition V2 hosted costs overflow".to_string())?;
    let mut opportunities = Vec::new();

    for record in records {
        if record.network != network
            || !record
                .factory_contract
                .eq_ignore_ascii_case(&release.factory_contract)
        {
            return Err("Open Competition V2 projection identity mismatch".to_string());
        }
        let projection = &record.projection;
        if projection.state == OpenCompetitionV2ProjectedState::Cancelled
            || projection.state == OpenCompetitionV2ProjectedState::Announced
        {
            continue;
        }
        let target = projection
            .solver_reward
            .checked_add(projection.keeper_reward)
            .ok_or_else(|| format!("competition {} economics overflow", projection.bounty_id))?;
        if projection.solver_reward == 0
            || projection.competition.len() != 42
            || projection.bounty_id.len() != 66
        {
            return Err(format!(
                "competition {} has malformed identity or economics",
                projection.bounty_id
            ));
        }
        let relevant_events = events
            .iter()
            .filter(|event| {
                event.bounty_id.eq_ignore_ascii_case(&projection.bounty_id)
                    && event
                        .contract_address
                        .eq_ignore_ascii_case(&projection.competition)
                    && event.block_number <= record.safe_block_number
            })
            .collect::<Vec<_>>();
        let created_at = relevant_events
            .first()
            .ok_or_else(|| {
                format!(
                    "competition {} has no canonical events",
                    projection.bounty_id
                )
            })?
            .occurred_at;
        let updated_at = relevant_events
            .last()
            .map(|event| event.occurred_at)
            .unwrap_or(created_at);
        let has_settlement = relevant_events
            .iter()
            .any(|event| event.kind == OpenCompetitionV2EventKind::CompetitionSettled);
        let profile = reviewed_v2_profile(release, record);
        let proof_deadline = projection.proof_deadline;
        let funding_deadline = projection.funding_deadline;
        let (source_status, work_state, payment_state, payment_committed, verification_ready) =
            match projection.state {
                OpenCompetitionV2ProjectedState::Funding => {
                    if projection.funded_amount >= target
                        || funding_deadline
                            .is_none_or(|deadline| deadline <= now.timestamp() as u64)
                    {
                        continue;
                    }
                    ("funding", "open", "seeking_funding", false, false)
                }
                OpenCompetitionV2ProjectedState::Active => {
                    if projection.funded_amount != target
                        || proof_deadline.is_none_or(|deadline| deadline <= now.timestamp() as u64)
                    {
                        continue;
                    }
                    ("active", "claimable", "escrowed", true, profile.is_some())
                }
                OpenCompetitionV2ProjectedState::Settled => {
                    if projection.winner.is_none()
                        || projection.funded_amount != 0
                        || !has_settlement
                    {
                        return Err(format!(
                            "competition {} lacks canonical settlement evidence",
                            projection.bounty_id
                        ));
                    }
                    ("settled", "completed", "paid", true, true)
                }
                OpenCompetitionV2ProjectedState::Announced
                | OpenCompetitionV2ProjectedState::Cancelled => continue,
            };
        let known = metadata
            .get(&projection.competition.to_ascii_lowercase())
            .filter(|item| item.bounty_id.eq_ignore_ascii_case(&projection.bounty_id));
        let title = known.map_or_else(
            || format!("Open Competition {}", &projection.bounty_id[..12]),
            |item| item.title.clone(),
        );
        let goal = known.map(|item| item.summary.clone());
        let source_url = known.map(|item| item.source_url.clone());
        let is_forward_gmv = profile
            .is_some_and(|item| item.profile_id == "forward-canonical-gmv-attribution-metric-v2");
        let participation_metadata_ready = !is_forward_gmv || known.is_some();
        let participation_phase = is_forward_gmv
            .then(|| v2_scoring_phase(known, now))
            .flatten();
        let public_url = format!(
            "{website}/competition.html?bountyContract={}&network={network}",
            projection.competition
        );
        let snapshot_url = is_forward_gmv
            .then(|| {
                known.map(|item| format!("{website}/generated/gmv-snapshots/{}.json", item.seed_id))
            })
            .flatten();
        let winner_mode = projection
            .winner_mode
            .clone()
            .ok_or_else(|| format!("competition {} has no winner mode", projection.bounty_id))?;
        let net = projection.solver_reward.saturating_sub(external_spend);
        let evidence_requirements = json!({
            "protocol_version": release.protocol_version,
            "program_profile": profile.map(|item| item.profile_id.clone()),
            "program_vkey": projection.program_vkey,
            "execution_policy_hash": projection.execution_policy_hash,
            "verification_policy_hash": projection.verification_policy_hash,
            "settlement_policy_hash": projection.settlement_policy_hash,
            "seed_id": known.map(|item| item.seed_id.clone()),
            "scoring_window": known.and_then(|item| {
                item.epoch_starts_at.as_ref().zip(item.epoch_ends_at.as_ref()).map(
                    |(starts_at, ends_at)| json!({
                        "starts_at": starts_at,
                        "ends_at": ends_at,
                        "minimum_score_base_units": item.minimum_score_base_units.as_deref(),
                    }),
                )
            }),
            "scoring_formula": is_forward_gmv.then_some("sum(settlement_gmv * entrant_funding / total_funding)"),
            "participation_phase": participation_phase,
            "participation_metadata_ready": participation_metadata_ready,
            "qualifying_action": is_forward_gmv.then(|| json!({
                "objective": "Post or fund useful marketplace demand that reaches canonical settlement inside the scoring window.",
                "entrant_binding": "Only funding from the competition solver wallet is attributed to that entrant.",
                "excluded": [
                    "operator or reserve wallets",
                    "excluded reward contracts",
                    "creator-equals-solver settlements",
                    "entrant-equals-solver settlements"
                ]
            })),
            "snapshot_url": snapshot_url.clone(),
            "payment_evidence": "CompetitionSettledV2"
        });
        let (categories, skills, keyword_matches) = web_public::discovery_taxonomy_with_matches(
            &title,
            goal.as_deref(),
            &evidence_requirements,
        );
        let opportunity_id = format!("open-competition-v2:{network}:{}", projection.competition);
        let embeds = opportunity_embed_links(api, &opportunity_id, Some(network));
        let image = fallback_opportunity_image(&embeds, &title);
        let deadline_value = if source_status == "funding" {
            funding_deadline
        } else {
            proof_deadline
        };
        let deadline = deadline_value
            .and_then(|value| i64::try_from(value).ok())
            .and_then(|value| DateTime::<Utc>::from_timestamp(value, 0))
            .map(|value| value.to_rfc3339());
        let next_action = match source_status {
            "funding" => OpportunityNextAction {
                action: "fund_open_competition_v2".to_string(),
                method: "POST".to_string(),
                url: format!(
                    "{api}/v1/base/open-competition-v2-beta3/funding-preparation"
                ),
                body_template: Some(json!({
                    "network": network,
                    "competition_contract": projection.competition,
                    "contributor": "0xYOUR_BASE_WALLET",
                    "amount": "USDC_BASE_UNITS",
                    "acknowledged_risk_hash": release.beta_risk_hash
                })),
                instructions: "Fund the exact contract, then wait for safe-block CompetitionActivatedV2.".to_string(),
            },
            "active" if is_forward_gmv && !participation_metadata_ready => OpportunityNextAction {
                action: "await_open_competition_v2_participation_metadata".to_string(),
                method: "GET".to_string(),
                url: public_url.clone(),
                body_template: None,
                instructions: "Do not generate score or buy a proof quote. The exact scoring window and public participation terms have not joined the canonical contract yet.".to_string(),
            },
            "active" if participation_phase == Some("upcoming") => OpportunityNextAction {
                action: "prepare_open_competition_v2_score".to_string(),
                method: "GET".to_string(),
                url: public_url.clone(),
                body_template: None,
                instructions: "Prepare the contract-bound child-bounty brief now. Do not count or fund score before the displayed UTC scoring window starts.".to_string(),
            },
            "active" if participation_phase == Some("scoring") => OpportunityNextAction {
                action: "generate_open_competition_v2_score".to_string(),
                method: "GET".to_string(),
                url: public_url.clone(),
                body_template: None,
                instructions: "Post and fund useful marketplace demand from the entrant wallet, have a different eligible wallet complete it, and reach canonical child settlement before the scoring window closes. Do not request a proof quote yet.".to_string(),
            },
            "active" if participation_phase == Some("proof") => OpportunityNextAction {
                action: "inspect_open_competition_v2_snapshot".to_string(),
                method: "GET".to_string(),
                url: snapshot_url.clone().unwrap_or_else(|| public_url.clone()),
                body_template: None,
                instructions: "Scoring is closed. Require the exact frozen snapshot and dual-attester quorum before requesting a solver-bound proof quote; fail closed while that evidence is unavailable.".to_string(),
            },
            "active" => OpportunityNextAction {
                action: "quote_open_competition_v2_proof".to_string(),
                method: "POST".to_string(),
                url: format!("{api}/v1/base/open-competition-v2-beta3/proof-quotes"),
                body_template: Some(json!({
                    "network": network,
                    "competition_contract": projection.competition,
                    "solver": "0xYOUR_BASE_WALLET",
                    "solver_nonce": "0xFRESH_BYTES32",
                    "artifact": "EXACT_ARTIFACT_AND_METRIC_INPUT"
                })),
                instructions: "Read the source terms, build the exact artifact, request a five-minute solver-bound quote, pay once, then authorize the exact relay.".to_string(),
            },
            _ => OpportunityNextAction {
                action: "inspect_open_competition_v2_settlement".to_string(),
                method: "GET".to_string(),
                url: format!(
                    "{api}/v1/base/open-competition-v2-beta3/events?network={network}&bounty_id={}",
                    projection.bounty_id
                ),
                body_template: None,
                instructions: "Confirm the safe-block CompetitionSettledV2 event and its solver before using paid language.".to_string(),
            },
        };
        let proof_urls = has_settlement
            .then(|| {
                format!(
                    "{api}/v1/base/open-competition-v2-beta3/events?network={network}&bounty_id={}",
                    projection.bounty_id
                )
            })
            .into_iter()
            .collect();
        opportunities.push(OpportunityItem {
            opportunity_id: opportunity_id.clone(),
            source_type: "canonical_base".to_string(),
            source_id: projection.competition.clone(),
            source_status: source_status.to_string(),
            title,
            goal,
            categories,
            skills,
            public_url,
            source_url,
            work_state: work_state.to_string(),
            payment_state: payment_state.to_string(),
            payment_committed,
            competition_mode: winner_mode,
            network: Some(network.to_string()),
            verifier_profile_id: profile.map(|item| item.profile_id.clone()),
            verifier_profile_name: profile.map(|item| item.profile_id.clone()),
            entry_count: u8::try_from(projection.accepted_entries).ok(),
            max_entries: None,
            competition_ends_at: proof_deadline,
            standing_meta_bounty: false,
            cash_economics: Some(OpportunityCashEconomics {
                solver_reward: OpportunityAmount::usdc_base_units(
                    projection.solver_reward.to_string(),
                ),
                refundable_claim_bond: OpportunityAmount::usdc_base_units("0"),
                required_external_spend: OpportunityAmount::usdc_base_units(
                    external_spend.to_string(),
                ),
                gross_cash_margin: OpportunityAmount::usdc_base_units(net.to_string()),
                gross_cash_margin_positive: net > 0,
                scope_disclaimer: "Gross cash margin is solver reward minus the configured hosted proof and relay fees. It excludes gas, taxes, losing risk and other execution costs; winning is not guaranteed.".to_string(),
            }),
            standing_meta_v4: None,
            decision_authority: format!(
                "The immutable SP1 {} verifier and policy hashes on {} decide qualification.",
                projection.proof_system.as_deref().unwrap_or("proof"),
                projection.competition
            ),
            payment_authority: format!(
                "The immutable competition contract {} controls escrow; only its safe-block CompetitionSettledV2 event proves solver payment.",
                projection.competition
            ),
            reward: OpportunityAmount::usdc_base_units(projection.solver_reward.to_string()),
            completion_bonus: Some(OpportunityAmount::usdc_base_units(
                projection.keeper_reward.to_string(),
            )),
            funded_amount: OpportunityAmount::usdc_base_units(
                projection.funded_amount.to_string(),
            ),
            funding_target: OpportunityAmount::usdc_base_units(target.to_string()),
            bond: OpportunityAmount::usdc_base_units("0"),
            deadline,
            deadline_kind: Some(if source_status == "funding" {
                "funding_deadline".to_string()
            } else {
                "proof_deadline".to_string()
            }),
            verification_method: format!(
                "sp1_{}",
                projection.proof_system.as_deref().unwrap_or("unknown")
            ),
            verification_ready: verification_ready && participation_metadata_ready,
            evidence_requirements,
            terms_hash: None,
            proof_urls,
            next_action,
            embeds,
            image,
            discovery_factors: base_factors(
                "canonical_base",
                work_state,
                payment_state,
                &keyword_matches,
            ),
            created_at: created_at.to_rfc3339(),
            updated_at: updated_at.to_rfc3339(),
            evidence_boundary: "Safe-block Beta3 projections prove current competition state and escrow. Qualification is not payment; only CompetitionSettledV2 proves solver payment.".to_string(),
        });
    }
    Ok(opportunities)
}

pub fn open_competition_opportunities(
    events: &[OpenCompetitionEvent],
    profile: &OpenCompetitionVerifierProfile,
    network: &str,
    api_base_url: &str,
    website_base_url: &str,
    public_activation_block: u64,
    now: DateTime<Utc>,
) -> Result<Vec<OpportunityItem>, String> {
    if !profile.public_inventory_eligible
        || profile.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn
    {
        return Ok(Vec::new());
    }
    if public_activation_block == 0 {
        return Err("public activation block is missing".to_string());
    }

    let api = api_base_url.trim_end_matches('/');
    let website = website_base_url.trim_end_matches('/');
    let mut grouped = BTreeMap::<String, Vec<&OpenCompetitionEvent>>::new();
    for event in events
        .iter()
        .filter(|event| event.block_number >= public_activation_block)
    {
        grouped
            .entry(event.bounty_id.clone())
            .or_default()
            .push(event);
    }

    let mut opportunities = Vec::new();
    for (bounty_id, bounty_events) in grouped {
        let created = unique_open_competition_event(
            &bounty_events,
            OpenCompetitionEventKind::CanonicalCompetitionCreated,
        )?;
        let Some(created) = created else {
            continue;
        };
        let terms = required_unique_open_competition_event(
            &bounty_events,
            OpenCompetitionEventKind::CanonicalCompetitionTermsCommitted,
        )?;
        let economics = required_unique_open_competition_event(
            &bounty_events,
            OpenCompetitionEventKind::CanonicalCompetitionEconomicsConfigured,
        )?;
        let verification = required_unique_open_competition_event(
            &bounty_events,
            OpenCompetitionEventKind::CanonicalCompetitionVerificationConfigured,
        )?;

        let verifier_module = json_text(&verification.data, "verifier_module")?;
        let benchmark_hash = json_text(&terms.data, "benchmark_hash")?;
        let evidence_schema_hash = json_text(&terms.data, "evidence_schema_hash")?;
        if !verifier_module.eq_ignore_ascii_case(&profile.verifier_address)
            || !benchmark_hash.eq_ignore_ascii_case(&profile.benchmark_hash)
            || !evidence_schema_hash.eq_ignore_ascii_case(&profile.evidence_schema_hash)
        {
            continue;
        }

        let solver_reward = json_u128(&economics.data, "solver_reward")?;
        let verifier_reward = json_u128(&economics.data, "verifier_reward")?;
        let entry_bond = json_u128(&economics.data, "entry_bond")?;
        let target_amount = json_u128(&economics.data, "target_amount")?;
        let initial_funding = json_u128(&economics.data, "initial_funding")?;
        let funding_deadline = json_u64(&economics.data, "funding_deadline")?;
        let configured_max_entries = json_u64(&economics.data, "max_entries")?;
        if solver_reward == 0
            || verifier_reward == 0
            || entry_bond != verifier_reward
            || target_amount != solver_reward.saturating_add(verifier_reward)
            || configured_max_entries == 0
            || configured_max_entries > 64
        {
            return Err(format!(
                "competition {bounty_id} violates public economics or capacity invariants"
            ));
        }
        let max_entries = configured_max_entries as u8;

        if bounty_events
            .iter()
            .any(|event| event.kind == OpenCompetitionEventKind::BountyCancelled)
        {
            continue;
        }
        let opened = last_open_competition_event(
            &bounty_events,
            OpenCompetitionEventKind::CompetitionOpened,
        );
        let settled =
            last_open_competition_event(&bounty_events, OpenCompetitionEventKind::BountySettled);
        let mut funded_amount = initial_funding;
        for event in bounty_events
            .iter()
            .filter(|event| event.kind == OpenCompetitionEventKind::FundingAdded)
        {
            funded_amount = funded_amount.max(json_u128(&event.data, "funded_amount")?);
        }
        if opened.is_some() && funded_amount < target_amount {
            return Err(format!(
                "competition {bounty_id} opened without canonical full-funding evidence"
            ));
        }

        let competition_ends_at = opened
            .map(|event| json_u64(&event.data, "competition_ends_at"))
            .transpose()?;
        let mut committed_solvers = BTreeSet::new();
        for event in bounty_events
            .iter()
            .filter(|event| event.kind == OpenCompetitionEventKind::SolutionCommitted)
        {
            committed_solvers.insert(json_text(&event.data, "solver")?.to_ascii_lowercase());
        }
        if committed_solvers.len() > usize::from(max_entries) {
            return Err(format!(
                "competition {bounty_id} exceeds its immutable entry capacity"
            ));
        }
        let entry_count = committed_solvers.len() as u8;
        let competition_open = settled.is_none()
            && competition_ends_at.is_some_and(|deadline| deadline > now.timestamp() as u64)
            && entry_count < max_entries;
        let fully_funded = funded_amount >= target_amount;
        let (source_status, work_state, payment_state, payment_committed) = if settled.is_some() {
            ("paid", "completed", "paid", true)
        } else if competition_open && fully_funded {
            ("claimable", "claimable", "escrowed", true)
        } else if fully_funded {
            ("expired", "open", "escrowed", true)
        } else {
            ("funding", "open", "seeking_funding", false)
        };

        let bounty_contract = json_text(&created.data, "bounty_contract")?;
        let terms_hash = json_text(&created.data, "terms_hash")?;
        let policy_hash = json_text(&created.data, "policy_hash")?;
        let acceptance_criteria_hash = json_text(&terms.data, "acceptance_criteria_hash")?;
        let events_url = format!(
            "{api}/v1/base/open-competition-v1/events?network={network}&bounty_id={bounty_id}"
        );
        let public_url = format!(
            "{website}/competition.html?bountyContract={bounty_contract}&network={network}&verifierProfileId={}",
            profile.profile_id
        );
        let opportunity_id = format!("open-competition:{network}:{bounty_contract}");
        let embeds = opportunity_embed_links(api, &opportunity_id, Some(network));
        let title = "Scope-bound hash-work competition".to_string();
        let image = fallback_opportunity_image(&embeds, &title);
        let deadline_value = competition_ends_at.unwrap_or(funding_deadline);
        let deadline = DateTime::<Utc>::from_timestamp(deadline_value as i64, 0)
            .map(|value| value.to_rfc3339());
        let next_action = if work_state == "claimable" {
            OpportunityNextAction {
                action: "enter_open_competition".to_string(),
                method: "POST".to_string(),
                url: format!("{api}/v1/base/open-competition-v1/commit-preparation"),
                body_template: Some(json!({
                    "network": network,
                    "bounty_contract": bounty_contract,
                    "solver": "<public Base wallet>",
                    "commitment": "<commitment generated locally from a private salt>"
                })),
                instructions: "Generate and save the commitment envelope locally, then prepare one bonded entry. First valid confirmed reveal wins; a prepared plan is not an entry or payment.".to_string(),
            }
        } else {
            OpportunityNextAction {
                action: if settled.is_some() {
                    "inspect_open_competition_settlement"
                } else {
                    "inspect_open_competition_state"
                }
                .to_string(),
                method: "GET".to_string(),
                url: events_url.clone(),
                body_template: None,
                instructions: "Inspect canonical version-specific events. Only BountySettled proves solver payment.".to_string(),
            }
        };
        let updated_at = bounty_events
            .last()
            .map(|event| event.occurred_at)
            .unwrap_or(created.occurred_at);
        let cash_economics = OpportunityCashEconomics {
            solver_reward: OpportunityAmount::usdc_base_units(solver_reward.to_string()),
            refundable_claim_bond: OpportunityAmount::usdc_base_units(entry_bond.to_string()),
            required_external_spend: OpportunityAmount::usdc_base_units("0"),
            gross_cash_margin: OpportunityAmount::usdc_base_units(solver_reward.to_string()),
            gross_cash_margin_positive: solver_reward > 0,
            scope_disclaimer: "Gross cash margin excludes gas, taxes, execution costs, and failure risk. The entry bond is returned to the winner and withdrawable by eligible losing or cancelled entries, but a failed or expired entry can forfeit it. This is not guaranteed net profit.".to_string(),
        };
        opportunities.push(OpportunityItem {
            opportunity_id: opportunity_id.clone(),
            source_type: "canonical_base".to_string(),
            source_id: bounty_contract.to_string(),
            source_status: source_status.to_string(),
            title,
            goal: Some("Produce proof bytes accepted by the exact published 16-bit leading-zero verifier. This profile does not judge ordinary code, writing, design, research, or task quality.".to_string()),
            categories: vec!["cryptographic-work".to_string(), "deterministic".to_string()],
            skills: vec!["commit-reveal".to_string(), "hash-work".to_string()],
            public_url,
            source_url: Some(events_url.clone()),
            work_state: work_state.to_string(),
            payment_state: payment_state.to_string(),
            payment_committed,
            competition_mode: "first_valid_submission".to_string(),
            network: Some(network.to_string()),
            verifier_profile_id: Some(profile.profile_id.clone()),
            verifier_profile_name: Some(profile.display_name.clone()),
            entry_count: Some(entry_count),
            max_entries: Some(max_entries),
            competition_ends_at,
            standing_meta_bounty: false,
            cash_economics: Some(cash_economics),
            standing_meta_v4: None,
            decision_authority: format!("The exact immutable verifier {} with runtime hash {} decides each reveal; the lowest confirmed passing reveal sequence wins.", profile.verifier_address, profile.runtime_code_hash),
            payment_authority: format!("The immutable competition contract {bounty_contract} controls escrow; only its confirmed BountySettled event proves payment."),
            reward: OpportunityAmount::usdc_base_units(solver_reward.to_string()),
            completion_bonus: None,
            funded_amount: OpportunityAmount::usdc_base_units(funded_amount.to_string()),
            funding_target: OpportunityAmount::usdc_base_units(target_amount.to_string()),
            bond: OpportunityAmount::usdc_base_units(entry_bond.to_string()),
            deadline,
            deadline_kind: Some(if competition_ends_at.is_some() {
                "competition_deadline"
            } else {
                "funding_deadline"
            }
            .to_string()),
            verification_method: format!("approved deterministic verifier: {}", profile.profile_id),
            verification_ready: true,
            evidence_requirements: json!({
                "verifier_profile_id": profile.profile_id,
                "verifier_module": profile.verifier_address,
                "runtime_code_hash": profile.runtime_code_hash,
                "configuration": profile.configuration,
                "acceptance_criteria_hash": acceptance_criteria_hash,
                "benchmark_hash": benchmark_hash,
                "evidence_schema": profile.evidence_schema,
                "evidence_schema_hash": evidence_schema_hash,
                "policy_hash": policy_hash,
                "ordering_rule": "first valid confirmed reveal wins",
                "identity_warning": "one wallet does not prove one independent person"
            }),
            terms_hash: Some(terms_hash.to_string()),
            proof_urls: settled.is_some().then_some(events_url).into_iter().collect(),
            next_action,
            embeds,
            image,
            discovery_factors: vec![
                "source_type=canonical_base".to_string(),
                format!("work_state={work_state}"),
                format!("payment_state={payment_state}"),
                "competition_mode=first_valid_submission".to_string(),
                format!("verifier_profile={}", profile.profile_id),
            ],
            created_at: created.occurred_at.to_rfc3339(),
            updated_at: updated_at.to_rfc3339(),
            evidence_boundary: "This public mode is limited to the exact catalog-pinned verifier profile. It does not establish fair judgment for ordinary code, design, writing, or research. A commitment, reveal, transaction hash, or hosted row is not payment; only confirmed canonical BountySettled proves payment.".to_string(),
        });
    }
    Ok(opportunities)
}

fn unique_open_competition_event<'a>(
    events: &'a [&OpenCompetitionEvent],
    kind: OpenCompetitionEventKind,
) -> Result<Option<&'a OpenCompetitionEvent>, String> {
    let mut matching = events.iter().copied().filter(|event| event.kind == kind);
    let first = matching.next();
    if matching.next().is_some() {
        return Err(format!("duplicate canonical {kind:?} event"));
    }
    Ok(first)
}

fn required_unique_open_competition_event<'a>(
    events: &'a [&OpenCompetitionEvent],
    kind: OpenCompetitionEventKind,
) -> Result<&'a OpenCompetitionEvent, String> {
    unique_open_competition_event(events, kind)?
        .ok_or_else(|| format!("missing canonical {kind:?} event"))
}

fn last_open_competition_event<'a>(
    events: &'a [&OpenCompetitionEvent],
    kind: OpenCompetitionEventKind,
) -> Option<&'a OpenCompetitionEvent> {
    events
        .iter()
        .rev()
        .copied()
        .find(|event| event.kind == kind)
}

fn json_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("open-competition event is missing {field}"))
}

fn json_u128(value: &Value, field: &str) -> Result<u128, String> {
    let value = value
        .get(field)
        .ok_or_else(|| format!("open-competition event is missing {field}"))?;
    match value {
        Value::String(value) => value.parse::<u128>(),
        Value::Number(value) => value.to_string().parse::<u128>(),
        _ => {
            return Err(format!(
                "open-competition event field {field} is not an integer"
            ))
        }
    }
    .map_err(|_| format!("open-competition event field {field} is out of range"))
}

fn json_u64(value: &Value, field: &str) -> Result<u64, String> {
    json_u128(value, field)?
        .try_into()
        .map_err(|_| format!("open-competition event field {field} is out of range"))
}

pub fn apply_query(
    mut items: Vec<OpportunityItem>,
    query: &OpportunityQuery,
    view: Option<OpportunityView>,
    now: DateTime<Utc>,
) -> Vec<OpportunityItem> {
    items.retain(|item| {
        query
            .source_type
            .as_deref()
            .is_none_or(|value| item.source_type == value)
            && query
                .work_state
                .as_deref()
                .is_none_or(|value| item.work_state == value)
            && query
                .payment_state
                .as_deref()
                .is_none_or(|value| item.payment_state == value)
    });

    if let Some(view) = view {
        items.retain_mut(|item| apply_view(item, view, now));
    }

    items.sort_by(|left, right| opportunity_order(left, right, now));
    items.truncate(query.limit.unwrap_or(100).clamp(1, 300) as usize);
    items
}

fn apply_view(item: &mut OpportunityItem, view: OpportunityView, now: DateTime<Utc>) -> bool {
    match view {
        OpportunityView::Recent => {
            item.discovery_factors
                .push("view:recent;factor=updated_at_desc".to_string());
            true
        }
        OpportunityView::Engineering => taxonomy_view(item, "engineering"),
        OpportunityView::Creative => taxonomy_view(item, "creative"),
        OpportunityView::Urgent => {
            let urgent = deadline_distance_seconds(item, now)
                .is_some_and(|seconds| (0..=72 * 60 * 60).contains(&seconds));
            if urgent {
                item.discovery_factors
                    .push("view:urgent;factor=deadline_within_72h".to_string());
            }
            urgent
        }
        OpportunityView::SeekingFunding => {
            let matches = item.payment_state == "seeking_funding";
            if matches {
                item.discovery_factors
                    .push("view:seeking_funding;factor=payment_state".to_string());
            }
            matches
        }
        OpportunityView::ReadyToEarn => {
            let is_unprofitable = item.gross_cash_margin.amount.starts_with('-');
            let matches = item.work_state == "claimable"
                && item.payment_state == "escrowed"
                && item.payment_committed
                && item.verification_ready
                && (item.source_type != "canonical_base"
                    || item
                        .cash_economics
                        .as_ref()
                        .is_some_and(|economics| economics.gross_cash_margin_positive));
            if matches {
                item.discovery_factors.push(
                    "view:ready_to_earn;factors=claimable+escrowed+verification_ready+positive_gross_cash_margin".to_string(),
                );
            }
            matches
        }
    }
}

fn taxonomy_view(item: &mut OpportunityItem, view: &str) -> bool {
    if !item.categories.iter().any(|category| category == view) {
        return false;
    }
    item.discovery_factors
        .push(format!("view:{view};factor=category:{view}"));
    true
}

fn opportunity_order(
    left: &OpportunityItem,
    right: &OpportunityItem,
    now: DateTime<Utc>,
) -> Ordering {
    let is_open_competition = |mode: &str| {
        matches!(
            mode,
            "first_valid_submission" | "first_proven" | "best_score"
        )
    };
    let left_ready =
        left.work_state == "claimable" && left.payment_committed && left.verification_ready;
    let right_ready =
        right.work_state == "claimable" && right.payment_committed && right.verification_ready;
    right_ready
        .cmp(&left_ready)
        .then_with(|| {
            is_open_competition(&right.competition_mode)
                .cmp(&is_open_competition(&left.competition_mode))
        })
        .then_with(|| {
            deadline_distance_seconds(left, now)
                .unwrap_or(i64::MAX)
                .cmp(&deadline_distance_seconds(right, now).unwrap_or(i64::MAX))
        })
        .then_with(|| right.updated_at.cmp(&left.updated_at))
        .then_with(|| left.opportunity_id.cmp(&right.opportunity_id))
}

fn deadline_distance_seconds(item: &OpportunityItem, now: DateTime<Utc>) -> Option<i64> {
    item.deadline
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|deadline| deadline.timestamp() - now.timestamp())
}

fn base_factors(
    source_type: &str,
    work_state: &str,
    payment_state: &str,
    keyword_matches: &[String],
) -> Vec<String> {
    let mut factors = vec![
        format!("source_type={source_type}"),
        format!("work_state={work_state}"),
        format!("payment_state={payment_state}"),
    ];
    if !keyword_matches.is_empty() {
        factors.push(format!("keyword_matches={}", keyword_matches.join(",")));
    }
    factors
}

fn opportunity_embed_links(
    api: &str,
    opportunity_id: &str,
    network: Option<&str>,
) -> OpportunityEmbedLinks {
    let id = percent_encode_segment(opportunity_id);
    let query = network
        .map(|network| format!("?network={network}"))
        .unwrap_or_default();
    let html = format!("{api}/public/opportunities/{id}/embed{query}");
    OpportunityEmbedLinks {
        svg: format!("{api}/public/opportunities/{id}/embed.svg{query}"),
        markdown: format!("{api}/public/opportunities/{id}/embed.md{query}"),
        iframe: format!(
            r#"<iframe src="{html}" title="Agent Bounties opportunity" width="720" height="264" loading="lazy"></iframe>"#
        ),
        html,
    }
}

fn fallback_opportunity_image(embeds: &OpportunityEmbedLinks, title: &str) -> OpportunityImage {
    OpportunityImage {
        source: "content_derived_legacy_card".to_string(),
        prompt: None,
        alt_text: format!("Agent Bounties card for {title}"),
        asset_url: embeds.svg.clone(),
        sha256: None,
        mime_type: "image/svg+xml".to_string(),
    }
}

fn percent_encode_segment(value: &str) -> String {
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

fn legacy_states(status: &BountyStatusResponse) -> (&'static str, &'static str, bool) {
    let work_state = match status.bounty.status {
        BountyStatus::Unfunded | BountyStatus::Funded => "open",
        BountyStatus::Claimable => "claimable",
        BountyStatus::Claimed => "in_progress",
        BountyStatus::Submitted | BountyStatus::Verifying => "submitted",
        BountyStatus::Accepted | BountyStatus::Payable | BountyStatus::Paid => "completed",
        BountyStatus::Refunding
        | BountyStatus::Refunded
        | BountyStatus::Disputed
        | BountyStatus::Expired => "completed",
    };
    if status.bounty.status == BountyStatus::Paid {
        return (work_state, "paid", true);
    }
    if status.funding_summary.claimable
        || matches!(
            status.bounty.status,
            BountyStatus::Claimable
                | BountyStatus::Claimed
                | BountyStatus::Submitted
                | BountyStatus::Verifying
                | BountyStatus::Accepted
                | BountyStatus::Payable
        )
    {
        return (work_state, "escrowed", true);
    }
    (work_state, "seeking_funding", false)
}

fn legacy_status_name(status: &BountyStatus) -> &'static str {
    match status {
        BountyStatus::Unfunded => "unfunded",
        BountyStatus::Funded => "funded",
        BountyStatus::Claimable => "claimable",
        BountyStatus::Claimed => "claimed",
        BountyStatus::Submitted => "submitted",
        BountyStatus::Verifying => "verifying",
        BountyStatus::Accepted => "accepted",
        BountyStatus::Payable => "payable",
        BountyStatus::Paid => "paid",
        BountyStatus::Refunding => "refunding",
        BountyStatus::Refunded => "refunded",
        BountyStatus::Disputed => "disputed",
        BountyStatus::Expired => "expired",
    }
}

fn legacy_next_action(
    status: &BountyStatusResponse,
    api: &str,
    work_state: &str,
    payment_state: &str,
) -> OpportunityNextAction {
    let id = status.bounty.id;
    if payment_state == "seeking_funding" {
        return OpportunityNextAction {
            action: "create_funding_intent".to_string(),
            method: "POST".to_string(),
            url: format!("{api}/v1/bounties/{id}/funding-intents"),
            body_template: Some(json!({
                "bounty_id": id,
                "amount_minor": status.funding_summary.remaining.amount,
                "currency": status.funding_summary.remaining.currency,
                "rail": "<supported payment rail>"
            })),
            instructions: "Prepare funding through a supported reconciled rail. An intent is not funding evidence.".to_string(),
        };
    }
    if work_state == "claimable" {
        return OpportunityNextAction {
            action: "claim_bounty".to_string(),
            method: "POST".to_string(),
            url: format!("{api}/v1/bounties/{id}/claim"),
            body_template: Some(json!({
                "bounty_id": id,
                "solver_agent_id": "<registered agent UUID>"
            })),
            instructions:
                "A registered solver may request the claim through the legacy bounty workflow."
                    .to_string(),
        };
    }
    OpportunityNextAction {
        action: "inspect_bounty_status".to_string(),
        method: "GET".to_string(),
        url: format!("{api}/v1/bounties/{id}"),
        body_template: None,
        instructions: "Inspect the current reconciled status and proof records before acting."
            .to_string(),
    }
}

fn canonical_next_action(
    item: &AutonomousBountyFeedItem,
    network: &str,
    api: &str,
    work_state: &str,
    payment_state: &str,
    funded: u128,
    target: u128,
) -> OpportunityNextAction {
    if payment_state == "seeking_funding" {
        let remaining = target.saturating_sub(funded);
        return OpportunityNextAction {
            action: "fund_bounty_with_x402".to_string(),
            method: "GET".to_string(),
            url: format!(
                "{api}/v1/x402/base/bounties/{}/funding?network={network}&amount={remaining}",
                item.bounty_contract
            ),
            body_template: None,
            instructions: "Request the exact funding challenge. Only confirmed FundingAdded changes the funded amount.".to_string(),
        };
    }
    if work_state == "claimable" {
        return OpportunityNextAction {
            action: "prepare_agent_to_earn".to_string(),
            method: "POST".to_string(),
            url: format!("{api}/v1/base/agent-wallet/readiness"),
            body_template: Some(json!({
                "network": network,
                "wallet_address": "<public Base wallet>",
                "bounty_contract": item.bounty_contract,
                "claim_bond_base_units": item.claim_bond,
                "signing_capabilities": [],
                "wallet_profile": null,
                "policy": {}
            })),
            instructions: "Run the wallet-neutral readiness check before requesting a claim. Do not provide a private key or seed phrase.".to_string(),
        };
    }
    let action = match work_state {
        "in_progress" => "active_solver_prepare_submission",
        "submitted" => "monitor_verification",
        "completed" => "inspect_settlement_evidence",
        _ if item.status == "claimable" => "inspect_verification_readiness",
        _ => "inspect_canonical_events",
    };
    OpportunityNextAction {
        action: action.to_string(),
        method: "GET".to_string(),
        url: format!(
            "{api}/v1/base/autonomous-bounties/events?network={network}&bounty_id={}",
            item.bounty_id
        ),
        body_template: None,
        instructions: "Inspect confirmed canonical events and immutable terms. Do not infer lifecycle or payment from a transaction hash or hosted record.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::{
        build_autonomous_bounty_terms_record, built_in_open_competition_verifier_catalog,
        AutonomousBountyEvent, AutonomousBountyEventKind, OpenCompetitionEvent,
        OpenCompetitionEventKind, OpenCompetitionV2Projection,
        BASE_MAINNET_STANDING_META_V2_VERIFIER,
    };
    use domain::{
        AutonomousBountyTermsDocument, AutonomousBountyTermsRecord, BountyImageReference,
    };
    use uuid::Uuid;

    fn trial() -> TrialBounty {
        let created_at = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap();
        TrialBounty {
            id: Uuid::nil(),
            idempotency_key: "post-1".to_string(),
            request_fingerprint: "fingerprint".to_string(),
            title: "Create an accessibility checklist".to_string(),
            goal: "Produce a useful public checklist".to_string(),
            acceptance_criteria: vec!["Include five checks".to_string()],
            source_url: None,
            discovery_source: "chatgpt_app".to_string(),
            status: "open".to_string(),
            demo_agent_solution: json!({}),
            created_at,
            expires_at: created_at + chrono::Duration::days(7),
        }
    }

    fn beta3_release_and_record() -> (OpenCompetitionV2Release, OpenCompetitionV2StoredProjection) {
        let hash = |digit: char| format!("0x{}", digit.to_string().repeat(64));
        let address = |digit: char| format!("0x{}", digit.to_string().repeat(40));
        let program = OpenCompetitionV2MetricProgramRelease {
            profile_id: "public-vector-metric-v1".to_string(),
            classification: OpenCompetitionV2ProgramClassification::Reviewed,
            program_vkey: hash('1'),
            source_hash: hash('2'),
            elf_hash: hash('3'),
            journal_schema_hash: hash('4'),
            metric_program_hash: hash('5'),
            review_evidence_hash: hash('6'),
        };
        let release = OpenCompetitionV2Release {
            protocol_version: "agent-bounties/open-competition-v2-beta3".to_string(),
            network: "base-mainnet".to_string(),
            source_commit: hash('a'),
            repository_subject_hash: hash('b'),
            sp1_source_commit: hash('c'),
            sp1_circuit_version: "agent-bounties-sp1-safe-v1".to_string(),
            factory_contract: "0x29d0e39e0c03797c690633535722e6b34a69a78a".to_string(),
            factory_runtime_code_hash: hash('d'),
            implementation_contract: address('2'),
            implementation_runtime_code_hash: hash('e'),
            settlement_token: address('3'),
            groth16_verifier: address('4'),
            groth16_verifier_hash: hash('7'),
            groth16_verifier_runtime_code_hash: hash('8'),
            groth16_adapter: address('5'),
            groth16_adapter_runtime_code_hash: hash('9'),
            plonk_verifier: address('6'),
            plonk_verifier_hash: hash('a'),
            plonk_verifier_runtime_code_hash: hash('b'),
            plonk_adapter: address('7'),
            plonk_adapter_runtime_code_hash: hash('c'),
            deployment_block: 90,
            release_hash: hash('d'),
            beta_risk_hash: hash('e'),
            public_creation_enabled: true,
            proof_broker_enabled: true,
            metric_programs: vec![program.clone()],
        };
        let projection = OpenCompetitionV2Projection {
            bounty_id: "0x6901f3ecf52842689a4209aac6fa7d8af205a6d2a546d567b77705e06c0a8c9a"
                .to_string(),
            competition: "0x8c494466711c1de316c7e7599f8b0641a30a0c98".to_string(),
            creator: address('8'),
            state: OpenCompetitionV2ProjectedState::Active,
            solver_reward: 3_000_000,
            keeper_reward: 40_000,
            funding_deadline: Some(1_800_010_000),
            proof_window_seconds: Some(86_400),
            winner_mode: Some("first_proven".to_string()),
            proof_system: Some("groth16".to_string()),
            program_vkey: Some(program.program_vkey),
            source_hash: Some(program.source_hash),
            elf_hash: Some(program.elf_hash),
            journal_schema_hash: Some(program.journal_schema_hash),
            metric_program_hash: Some(program.metric_program_hash),
            funded_amount: 3_040_000,
            proof_deadline: Some(1_800_086_400),
            last_block: 99,
            ..OpenCompetitionV2Projection::default()
        };
        let record = OpenCompetitionV2StoredProjection {
            network: "base-mainnet".to_string(),
            factory_contract: release.factory_contract.clone(),
            projection,
            safe_block_number: 100,
            safe_block_hash: hash('f'),
        };
        (release, record)
    }

    fn beta3_event(
        record: &OpenCompetitionV2StoredProjection,
        kind: OpenCompetitionV2EventKind,
        block_number: u64,
    ) -> OpenCompetitionV2Event {
        OpenCompetitionV2Event {
            id: Uuid::new_v4(),
            protocol_version: "agent-bounties/open-competition-v2-beta3".to_string(),
            log_key: format!("{block_number}:0"),
            tx_hash: format!("0x{:064x}", block_number),
            block_number,
            log_index: 0,
            contract_address: record.projection.competition.clone(),
            bounty_id: record.projection.bounty_id.clone(),
            kind,
            data: json!({}),
            occurred_at: DateTime::<Utc>::from_timestamp(1_800_000_000 + block_number as i64, 0)
                .unwrap(),
        }
    }

    #[test]
    fn beta3_projection_is_profitable_and_requires_canonical_settlement_evidence() {
        let (release, record) = beta3_release_and_record();
        let mut events = vec![beta3_event(
            &record,
            OpenCompetitionV2EventKind::CanonicalCompetitionCreated,
            90,
        )];
        let now = DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap();
        let active = open_competition_v2_opportunities(
            std::slice::from_ref(&record),
            &events,
            &release,
            "base-mainnet",
            "https://api.example",
            "https://site.example",
            OpenCompetitionV2HostedCosts {
                proof_fee: 100_000,
                relay_fee: 10_000,
            },
            now,
        )
        .unwrap()
        .remove(0);
        assert_eq!(
            active.title,
            "Highest externally funded canonical GMV — daily August 24"
        );
        assert_eq!(active.work_state, "claimable");
        assert_eq!(active.payment_state, "escrowed");
        assert_eq!(active.competition_mode, "first_proven");
        assert_eq!(active.max_entries, None);
        assert_eq!(active.next_action.action, "quote_open_competition_v2_proof");
        assert_eq!(
            active.evidence_requirements["scoring_window"]["starts_at"],
            "2026-08-24T00:00:00Z"
        );
        assert_eq!(
            active.evidence_requirements["scoring_window"]["minimum_score_base_units"],
            "1"
        );
        assert_eq!(
            active.cash_economics.unwrap().gross_cash_margin.amount,
            "2890000"
        );

        let mut settled = record.clone();
        settled.projection.state = OpenCompetitionV2ProjectedState::Settled;
        settled.projection.winner = Some(address_for_test('9'));
        settled.projection.funded_amount = 0;
        assert!(open_competition_v2_opportunities(
            std::slice::from_ref(&settled),
            &events,
            &release,
            "base-mainnet",
            "https://api.example",
            "https://site.example",
            OpenCompetitionV2HostedCosts {
                proof_fee: 100_000,
                relay_fee: 10_000,
            },
            now,
        )
        .is_err());

        events.push(beta3_event(
            &settled,
            OpenCompetitionV2EventKind::CompetitionSettled,
            100,
        ));
        let paid = open_competition_v2_opportunities(
            &[settled],
            &events,
            &release,
            "base-mainnet",
            "https://api.example",
            "https://site.example",
            OpenCompetitionV2HostedCosts {
                proof_fee: 100_000,
                relay_fee: 10_000,
            },
            now,
        )
        .unwrap()
        .remove(0);
        assert_eq!(paid.payment_state, "paid");
        assert_eq!(paid.proof_urls.len(), 1);
    }

    #[test]
    fn beta3_forward_gmv_next_action_follows_the_scoring_phase() {
        let (mut release, record) = beta3_release_and_record();
        release.metric_programs[0].profile_id =
            "forward-canonical-gmv-attribution-metric-v2".to_string();
        let events = vec![beta3_event(
            &record,
            OpenCompetitionV2EventKind::CanonicalCompetitionCreated,
            90,
        )];
        let project = |now: &str| {
            open_competition_v2_opportunities(
                std::slice::from_ref(&record),
                &events,
                &release,
                "base-mainnet",
                "https://api.example",
                "https://site.example",
                OpenCompetitionV2HostedCosts {
                    proof_fee: 100_000,
                    relay_fee: 10_000,
                },
                DateTime::parse_from_rfc3339(now)
                    .unwrap()
                    .with_timezone(&Utc),
            )
            .unwrap()
            .remove(0)
        };

        let upcoming = project("2026-08-23T12:00:00Z");
        assert_eq!(
            upcoming.next_action.action,
            "prepare_open_competition_v2_score"
        );
        assert_eq!(
            upcoming.evidence_requirements["participation_phase"],
            "upcoming"
        );

        let scoring = project("2026-08-24T12:00:00Z");
        assert_eq!(
            scoring.next_action.action,
            "generate_open_competition_v2_score"
        );
        assert_eq!(scoring.next_action.method, "GET");
        assert_eq!(scoring.next_action.url, scoring.public_url);
        assert!(scoring.next_action.body_template.is_none());
        assert_eq!(
            scoring.evidence_requirements["participation_phase"],
            "scoring"
        );
        assert!(scoring
            .next_action
            .instructions
            .contains("Do not request a proof quote yet"));

        let proof = project("2026-08-25T00:00:01Z");
        assert_eq!(
            proof.next_action.action,
            "inspect_open_competition_v2_snapshot"
        );
        assert_eq!(proof.evidence_requirements["participation_phase"], "proof");
        assert_eq!(
            proof.next_action.url,
            proof.evidence_requirements["snapshot_url"]
        );
    }

    #[test]
    fn beta3_stale_optional_metadata_does_not_hide_canonical_inventory() {
        let (mut release, mut record) = beta3_release_and_record();
        release.factory_contract = address_for_test('a');
        record.factory_contract = release.factory_contract.clone();
        record.projection.bounty_id = format!("0x{}", "b".repeat(64));
        record.projection.competition = address_for_test('c');
        let events = vec![beta3_event(
            &record,
            OpenCompetitionV2EventKind::CanonicalCompetitionCreated,
            90,
        )];
        let now = DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap();

        let item = open_competition_v2_opportunities(
            &[record],
            &events,
            &release,
            "base-mainnet",
            "https://api.example",
            "https://site.example",
            OpenCompetitionV2HostedCosts {
                proof_fee: 100_000,
                relay_fee: 10_000,
            },
            now,
        )
        .unwrap()
        .remove(0);

        assert_eq!(item.title, "Open Competition 0xbbbbbbbbbb");
        assert_eq!(item.work_state, "claimable");
        assert_eq!(item.payment_state, "escrowed");
        assert_eq!(item.next_action.action, "quote_open_competition_v2_proof");
    }

    #[test]
    fn beta3_forward_gmv_without_metadata_fails_closed_for_participation() {
        let (mut release, mut record) = beta3_release_and_record();
        release.metric_programs[0].profile_id =
            "forward-canonical-gmv-attribution-metric-v2".to_string();
        release.factory_contract = address_for_test('a');
        record.factory_contract = release.factory_contract.clone();
        record.projection.bounty_id = format!("0x{}", "b".repeat(64));
        record.projection.competition = address_for_test('c');
        let events = vec![beta3_event(
            &record,
            OpenCompetitionV2EventKind::CanonicalCompetitionCreated,
            90,
        )];
        let now = DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap();

        let item = open_competition_v2_opportunities(
            &[record],
            &events,
            &release,
            "base-mainnet",
            "https://api.example",
            "https://site.example",
            OpenCompetitionV2HostedCosts {
                proof_fee: 100_000,
                relay_fee: 10_000,
            },
            now,
        )
        .unwrap()
        .remove(0);

        assert!(!item.verification_ready);
        assert_eq!(
            item.evidence_requirements["participation_metadata_ready"],
            false
        );
        assert!(item.evidence_requirements["scoring_window"].is_null());
        assert_eq!(
            item.next_action.action,
            "await_open_competition_v2_participation_metadata"
        );
        assert!(item
            .next_action
            .instructions
            .contains("Do not generate score"));
    }

    #[test]
    fn beta3_public_metadata_source_requires_pinned_reviewed_artifact() {
        let revision = "b600500a0ba25babe5bf9d262472ef4f701b480a";
        assert!(reviewed_v2_public_source_url(&format!(
            "https://github.com/NSPG13/agent-bounties/blob/{revision}/ops/open-competition-v2-forward-gmv-reward-cohort-v1.json"
        )));
        assert!(!reviewed_v2_public_source_url(
            "https://github.com/NSPG13/agent-bounties/blob/main/ops/open-competition-v2-forward-gmv-reward-cohort-v1.json"
        ));
        assert!(!reviewed_v2_public_source_url(&format!(
            "https://github.com/NSPG13/agent-bounties/blob/{revision}/README.md"
        )));
    }

    #[test]
    fn beta3_live_registry_projects_sixteen_scanner_ready_opportunities_and_feeds() {
        let (mut release, template) = beta3_release_and_record();
        release.metric_programs[0].profile_id =
            "forward-canonical-gmv-attribution-metric-v2".to_string();
        let metadata = v2_public_metadata(&release).unwrap();
        assert_eq!(metadata.len(), 16);
        let mut records = Vec::new();
        let mut events = Vec::new();
        for (index, item) in metadata.values().enumerate() {
            let mut record = template.clone();
            record.projection.bounty_id = item.bounty_id.clone();
            record.projection.competition = item.competition.clone();
            record.projection.last_block = 90 + index as u64;
            record.safe_block_number = 200;
            events.push(beta3_event(
                &record,
                OpenCompetitionV2EventKind::CanonicalCompetitionCreated,
                90 + index as u64,
            ));
            records.push(record);
        }
        let now = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let projected = open_competition_v2_opportunities(
            &records,
            &events,
            &release,
            "base-mainnet",
            "https://api.example",
            "https://site.example",
            OpenCompetitionV2HostedCosts {
                proof_fee: 100_000,
                relay_fee: 10_000,
            },
            now,
        )
        .unwrap();
        let ready = apply_query(
            projected,
            &OpportunityQuery {
                source_type: Some("canonical_base".to_string()),
                ..OpportunityQuery::default()
            },
            Some(OpportunityView::ReadyToEarn),
            now,
        );

        assert_eq!(ready.len(), 16);
        assert!(ready.iter().all(|item| {
            item.verification_ready
                && item.payment_committed
                && item.evidence_requirements["scoring_window"].is_object()
                && item
                    .public_url
                    .starts_with("https://site.example/competition.html?bountyContract=")
        }));
        assert_eq!(
            ready
                .iter()
                .filter(|item| {
                    item.evidence_requirements["participation_phase"] == "scoring"
                        && item.next_action.action == "generate_open_competition_v2_score"
                        && item.next_action.method == "GET"
                        && item.next_action.url == item.public_url
                })
                .count(),
            5
        );
        assert_eq!(
            ready
                .iter()
                .filter(|item| {
                    item.evidence_requirements["participation_phase"] == "upcoming"
                        && item.next_action.action == "prepare_open_competition_v2_score"
                        && item.next_action.method == "GET"
                        && item.next_action.url == item.public_url
                })
                .count(),
            11
        );
        let projection = OpportunityProjectionResponse {
            schema_version: OPPORTUNITY_PROJECTION_SCHEMA.to_string(),
            generated_at: now.to_rfc3339(),
            network: "base-mainnet".to_string(),
            applied_view: Some("ready_to_earn".to_string()),
            degraded: false,
            source_statuses: vec![OpportunitySourceStatus {
                source_type: "canonical_base".to_string(),
                available: true,
                authoritative_urls: vec!["https://api.example/v1/opportunities".to_string()],
                item_count: ready.len(),
                error: None,
            }],
            items: ready,
            evidence_boundary: "test".to_string(),
        };
        let feeds = render_opportunity_feeds(&projection, "https://api.example");
        let json: Value = serde_json::from_str(&feeds.json).unwrap();
        assert_eq!(json["items"].as_array().unwrap().len(), 16);
        assert!(feeds
            .rss
            .contains("Highest externally funded canonical GMV"));
        assert!(feeds
            .atom
            .contains("Highest externally funded canonical GMV"));
    }

    fn address_for_test(digit: char) -> String {
        format!("0x{}", digit.to_string().repeat(40))
    }

    fn canonical(status: &str, funded: &str, verification_ready: bool) -> AutonomousBountyFeedItem {
        let created_at = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap();
        let event = AutonomousBountyEvent {
            id: Uuid::nil(),
            log_key: "1:0".to_string(),
            tx_hash: format!("0x{}", "1".repeat(64)),
            block_number: 1,
            log_index: 0,
            contract_address: format!("0x{}", "2".repeat(40)),
            bounty_id: format!("0x{}", "3".repeat(64)),
            kind: AutonomousBountyEventKind::CanonicalBountyCreated,
            data: json!({}),
            occurred_at: created_at,
        };
        let terms = AutonomousBountyTermsRecord {
            terms_hash: format!("0x{}", "4".repeat(64)),
            policy_hash: format!("0x{}", "5".repeat(64)),
            acceptance_criteria_hash: format!("0x{}", "6".repeat(64)),
            benchmark_hash: format!("0x{}", "7".repeat(64)),
            evidence_schema_hash: format!("0x{}", "8".repeat(64)),
            creator_wallet: format!("0x{}", "9".repeat(40)),
            document: AutonomousBountyTermsDocument {
                schema_version: "agent-bounties/terms-v1".to_string(),
                contract_terms: json!({"funding_deadline": 1_800_086_400_u64}),
                title: "Implement an API test".to_string(),
                goal: "Add deterministic coverage".to_string(),
                acceptance_criteria: vec!["Test passes".to_string()],
                benchmark: json!({"engine": "sandboxed_regression_v1"}),
                evidence_schema: json!({"required": ["commit"]}),
                verification_policy: json!({}),
                source_url: None,
                discovery_source: None,
                image: Some(BountyImageReference {
                    source: "chatgpt_user_generated".to_string(),
                    prompt: "Minimal editorial illustration of a reliable API test.".to_string(),
                    alt_text: "A clean API test report with a passing status.".to_string(),
                    asset_url: format!(
                        "https://mcp.agentbounties.app/public/bounty-images/{}",
                        "ab".repeat(32)
                    ),
                    sha256: "ab".repeat(32),
                    mime_type: "image/webp".to_string(),
                }),
                agent_eligibility: None,
                claim_coordination: None,
            },
            created_at,
        };
        AutonomousBountyFeedItem {
            bounty_id: event.bounty_id.clone(),
            bounty_contract: event.contract_address.clone(),
            creator: terms.creator_wallet.clone(),
            status: status.to_string(),
            solver_reward: "900000".to_string(),
            verifier_reward: "100000".to_string(),
            claim_bond: "100000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "1000000".to_string(),
            funded_amount: funded.to_string(),
            required_external_spend: "0".to_string(),
            gross_cash_margin: "900000".to_string(),
            terms_hash: terms.terms_hash.clone(),
            terms: Some(terms),
            terms_valid: true,
            verification_mode: "signed_quorum".to_string(),
            verifier_module: None,
            verifier_set_hash: None,
            verifier_threshold: Some(2),
            runner_identifier: Some("sandboxed_regression_v1".to_string()),
            verification_ready,
            verification_readiness_reason: "ready".to_string(),
            validation_errors: Vec::new(),
            events: vec![event],
        }
    }

    fn standing_meta() -> AutonomousBountyFeedItem {
        let created_at = DateTime::parse_from_rfc3339("2026-07-17T02:11:34Z")
            .unwrap()
            .with_timezone(&Utc);
        let document: AutonomousBountyTermsDocument =
            serde_json::from_str(include_str!("../../../bounties/autonomous-v1/335.json")).unwrap();
        let terms = build_autonomous_bounty_terms_record(
            "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
            document,
            created_at,
        )
        .unwrap();
        let mut item = canonical("claimable", "1000000", true);
        item.bounty_id =
            "0x12ad2fa99de272728311a3eb07c3c741048382260cb91ba1e8f001ed3b5759d0".to_string();
        item.bounty_contract = "0x43d42cb227d76588ab16693f14efd6cff851fa7a".to_string();
        item.creator = terms.creator_wallet.clone();
        item.solver_reward = "900000".to_string();
        item.verifier_reward = "100000".to_string();
        item.claim_bond = "100000".to_string();
        item.required_external_spend = "900000".to_string();
        item.gross_cash_margin = "0".to_string();
        item.terms_hash = terms.terms_hash.clone();
        item.terms = Some(terms);
        item.verification_mode = "deterministic_module".to_string();
        item.verifier_module = Some(BASE_MAINNET_STANDING_META_V2_VERIFIER.to_string());
        item.verifier_threshold = Some(1);
        item.runner_identifier = Some("standing_meta_v2_parent".to_string());
        item.events.clear();
        item
    }

    fn open_competition_event(
        kind: OpenCompetitionEventKind,
        block_number: u64,
        log_index: u64,
        data: Value,
    ) -> OpenCompetitionEvent {
        OpenCompetitionEvent {
            id: Uuid::new_v4(),
            protocol_version: "agent-bounties/open-competition-v1".to_string(),
            log_key: format!("{block_number}:{log_index}"),
            tx_hash: format!("0x{:064x}", block_number),
            block_number,
            log_index,
            contract_address: "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5".to_string(),
            bounty_id: format!("0x{}", "3".repeat(64)),
            kind,
            data,
            occurred_at: DateTime::<Utc>::from_timestamp(1_800_000_000 + block_number as i64, 0)
                .unwrap(),
        }
    }

    fn open_competition_fixture() -> (
        Vec<OpenCompetitionEvent>,
        chain_base::OpenCompetitionVerifierProfile,
    ) {
        let mut profile = built_in_open_competition_verifier_catalog("base-mainnet")
            .unwrap()
            .profiles
            .remove(0);
        profile.deployment_state = OpenCompetitionDeploymentState::ActiveReadyToEarn;
        profile.public_inventory_eligible = true;
        let bounty_contract = "0x1111111111111111111111111111111111111111";
        let events = vec![
            open_competition_event(
                OpenCompetitionEventKind::CanonicalCompetitionCreated,
                101,
                0,
                json!({
                    "bounty_contract": bounty_contract,
                    "creator": "0x2222222222222222222222222222222222222222",
                    "terms_hash": format!("0x{}", "4".repeat(64)),
                    "policy_hash": format!("0x{}", "5".repeat(64)),
                    "creation_nonce": format!("0x{}", "6".repeat(64))
                }),
            ),
            open_competition_event(
                OpenCompetitionEventKind::CanonicalCompetitionTermsCommitted,
                101,
                1,
                json!({
                    "acceptance_criteria_hash": format!("0x{}", "7".repeat(64)),
                    "benchmark_hash": profile.benchmark_hash,
                    "evidence_schema_hash": profile.evidence_schema_hash
                }),
            ),
            open_competition_event(
                OpenCompetitionEventKind::CanonicalCompetitionEconomicsConfigured,
                101,
                2,
                json!({
                    "solver_reward": 500_000,
                    "verifier_reward": 50_000,
                    "entry_bond": 50_000,
                    "target_amount": 550_000,
                    "initial_funding": 550_000,
                    "funding_deadline": 1_800_086_400_u64,
                    "competition_window_seconds": 86_400,
                    "reveal_window_seconds": 3_600,
                    "max_entries": 4
                }),
            ),
            open_competition_event(
                OpenCompetitionEventKind::CanonicalCompetitionVerificationConfigured,
                101,
                3,
                json!({
                    "verifier_module": profile.verifier_address,
                    "verifier_reward_recipient": "0x2222222222222222222222222222222222222222"
                }),
            ),
            open_competition_event(
                OpenCompetitionEventKind::CompetitionOpened,
                101,
                4,
                json!({"competition_ends_at": 1_800_086_400_u64, "max_entries": 4}),
            ),
        ];
        (events, profile)
    }

    #[test]
    fn unfunded_projection_is_real_open_work_without_payment_commitment() {
        let item = unfunded_opportunity(&trial(), &[], "https://api.example");
        assert_eq!(item.work_state, "open");
        assert_eq!(item.payment_state, "none");
        assert!(!item.payment_committed);
        assert_eq!(item.reward.amount, "0");
        assert_eq!(item.next_action.action, "submit_unfunded_bounty_solution");
        assert_eq!(item.image.source, "content_derived_legacy_card");
        assert!(item.image.asset_url.ends_with("/embed.svg"));
        assert!(!serde_json::to_string(&item).unwrap().contains("trial"));
    }

    #[test]
    fn canonical_projection_requires_full_funding_and_verifier_readiness_to_be_claimable() {
        let ready = canonical_opportunity(
            &canonical("claimable", "1000000", true),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        assert_eq!(ready.work_state, "claimable");
        assert_eq!(ready.payment_state, "escrowed");
        assert!(ready.payment_committed);
        assert_eq!(ready.next_action.action, "prepare_agent_to_earn");
        assert_eq!(ready.image.source, "chatgpt_user_generated");
        assert_eq!(ready.image.sha256, Some("ab".repeat(32)));

        let unavailable = canonical_opportunity(
            &canonical("claimable", "1000000", false),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        assert_eq!(unavailable.work_state, "open");
        assert_eq!(
            unavailable.next_action.action,
            "inspect_verification_readiness"
        );
    }

    #[test]
    fn public_open_competition_is_primary_ready_to_earn_mode() {
        let (events, profile) = open_competition_fixture();
        let item = open_competition_opportunities(
            &events,
            &profile,
            "base-mainnet",
            "https://api.example",
            "https://www.example",
            100,
            DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap(),
        )
        .unwrap()
        .remove(0);
        assert_eq!(item.source_status, "claimable");
        assert_eq!(item.work_state, "claimable");
        assert_eq!(item.payment_state, "escrowed");
        assert_eq!(item.competition_mode, "first_valid_submission");
        assert_eq!(item.next_action.action, "enter_open_competition");
        assert_eq!(item.entry_count, Some(0));
        assert_eq!(item.max_entries, Some(4));
        assert!(item.public_url.contains("competition.html"));
        assert!(item
            .goal
            .as_deref()
            .unwrap()
            .contains("does not judge ordinary code"));
    }

    #[test]
    fn open_competition_payment_requires_canonical_settlement_event() {
        let (mut events, profile) = open_competition_fixture();
        let before = open_competition_opportunities(
            &events,
            &profile,
            "base-mainnet",
            "https://api.example",
            "https://www.example",
            100,
            DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap(),
        )
        .unwrap()
        .remove(0);
        assert_ne!(before.payment_state, "paid");
        assert!(before.proof_urls.is_empty());

        events.push(open_competition_event(
            OpenCompetitionEventKind::BountySettled,
            102,
            0,
            json!({
                "submission_sequence": 1,
                "solver": "0x8888888888888888888888888888888888888888",
                "solver_reward": 500_000,
                "entry_bond_returned": 50_000,
                "timeout_bond_bonus": 0,
                "verifier_reward": 50_000,
                "submission_hash": format!("0x{}", "8".repeat(64)),
                "evidence_hash": format!("0x{}", "9".repeat(64)),
                "policy_hash": format!("0x{}", "5".repeat(64)),
                "verification_hash": format!("0x{}", "a".repeat(64)),
                "canonical_payment_evidence": true
            }),
        ));
        let paid = open_competition_opportunities(
            &events,
            &profile,
            "base-mainnet",
            "https://api.example",
            "https://www.example",
            100,
            DateTime::<Utc>::from_timestamp(1_800_000_200, 0).unwrap(),
        )
        .unwrap()
        .remove(0);
        assert_eq!(paid.payment_state, "paid");
        assert_eq!(paid.work_state, "completed");
        assert_eq!(paid.proof_urls.len(), 1);
    }

    #[test]
    fn open_competition_projection_rejects_unknown_verifier_and_pre_activation_history() {
        let (events, mut profile) = open_competition_fixture();
        profile.verifier_address = "0x9999999999999999999999999999999999999999".to_string();
        let unknown = open_competition_opportunities(
            &events,
            &profile,
            "base-mainnet",
            "https://api.example",
            "https://www.example",
            100,
            DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap(),
        )
        .unwrap();
        assert!(unknown.is_empty());

        let (_, profile) = open_competition_fixture();
        let before_activation = open_competition_opportunities(
            &events,
            &profile,
            "base-mainnet",
            "https://api.example",
            "https://www.example",
            102,
            DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap(),
        )
        .unwrap();
        assert!(before_activation.is_empty());
    }

    #[test]
    fn canonical_projection_exposes_cash_economics_without_profit_claims() {
        let item = canonical_opportunity(
            &canonical("claimable", "1000000", true),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        let economics = item.cash_economics.unwrap();
        assert_eq!(economics.solver_reward.amount, "900000");
        assert_eq!(economics.refundable_claim_bond.amount, "100000");
        assert_eq!(economics.required_external_spend.amount, "0");
        assert_eq!(economics.gross_cash_margin.amount, "900000");
        assert!(economics.gross_cash_margin_positive);
        assert!(economics
            .scope_disclaimer
            .contains("not guaranteed net profit"));
    }

    #[test]
    fn ready_to_earn_excludes_non_positive_canonical_cash_margin() {
        let profitable = canonical_opportunity(
            &canonical("claimable", "1000000", true),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        let mut unprofitable_source = canonical("claimable", "1000000", true);
        unprofitable_source.required_external_spend = "1000000".to_string();
        unprofitable_source.gross_cash_margin = "-100000".to_string();
        let unprofitable =
            canonical_opportunity(&unprofitable_source, "base-mainnet", "https://api.example")
                .unwrap();

        let items = apply_query(
            vec![profitable, unprofitable],
            &OpportunityQuery::default(),
            Some(OpportunityView::ReadyToEarn),
            DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap(),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0]
                .cash_economics
                .as_ref()
                .unwrap()
                .gross_cash_margin
                .amount,
            "900000"
        );
    }

    #[test]
    fn canonical_cash_economics_cover_direct_standing_meta_and_unprofitable() {
        let direct = canonical_opportunity(
            &canonical("claimable", "1000000", true),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        let meta =
            canonical_opportunity(&standing_meta(), "base-mainnet", "https://api.example").unwrap();
        assert!(!direct.standing_meta_bounty);
        assert!(
            direct
                .cash_economics
                .as_ref()
                .unwrap()
                .gross_cash_margin_positive
        );
        assert!(meta.standing_meta_bounty);
        let meta_economics = meta.cash_economics.as_ref().unwrap();
        assert_eq!(meta_economics.required_external_spend.amount, "900000");
        assert_eq!(meta_economics.gross_cash_margin.amount, "0");
        assert!(!meta_economics.gross_cash_margin_positive);

        let ready = apply_query(
            vec![direct, meta],
            &OpportunityQuery::default(),
            Some(OpportunityView::ReadyToEarn),
            DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap(),
        );
        assert_eq!(ready.len(), 1);
        assert!(!ready[0].standing_meta_bounty);
    }

    #[test]
    fn partial_canonical_funding_is_seeking_not_committed() {
        let item = canonical_opportunity(
            &canonical("open", "250000", true),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        assert_eq!(item.payment_state, "seeking_funding");
        assert!(!item.payment_committed);
        assert!(item.next_action.url.ends_with("amount=750000"));
    }

    #[test]
    fn discovery_views_explain_deterministic_inclusion() {
        let item = canonical_opportunity(
            &canonical("claimable", "1000000", true),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        let query = OpportunityQuery {
            view: Some("engineering".to_string()),
            ..OpportunityQuery::default()
        };
        let items = apply_query(
            vec![item],
            &query,
            Some(OpportunityView::Engineering),
            DateTime::<Utc>::from_timestamp(1_800_000_100, 0).unwrap(),
        );
        assert_eq!(items.len(), 1);
        assert!(items[0]
            .discovery_factors
            .iter()
            .any(|factor| factor.contains("keyword_matches=api")));
    }

    #[test]
    fn live_feed_formats_reuse_projection_and_disclose_unfunded_payment_state() {
        let mut item = unfunded_opportunity(&trial(), &[], "https://api.example");
        item.title = "Audit <unsafe> & document".to_string();
        let projection = OpportunityProjectionResponse {
            schema_version: OPPORTUNITY_PROJECTION_SCHEMA.to_string(),
            generated_at: "2027-01-15T08:01:00Z".to_string(),
            network: "base-mainnet".to_string(),
            applied_view: None,
            degraded: false,
            source_statuses: Vec::new(),
            items: vec![item],
            evidence_boundary: "Projection only".to_string(),
        };

        let feeds = render_opportunity_feeds(&projection, "https://api.example/");
        assert!(feeds.rss.contains("<rss version=\"2.0\">"));
        assert!(feeds.rss.contains("Audit &lt;unsafe&gt; &amp; document"));
        assert!(feeds.rss.contains("<category>none</category>"));
        assert!(feeds.atom.contains("xmlns=\"http://www.w3.org/2005/Atom\""));
        assert!(
            feeds.atom.contains("payment is not committed")
                || feeds.atom.contains("No payment is committed")
        );

        let json: Value = serde_json::from_str(&feeds.json).unwrap();
        assert_eq!(json["version"], "https://jsonfeed.org/version/1.1");
        assert_eq!(json["items"][0]["_bountyboard"]["payment_state"], "none");
        assert_eq!(json["items"][0]["_bountyboard"]["payment_committed"], false);
        assert!(!feeds.rss.to_ascii_lowercase().contains("trial"));
        assert_eq!(feeds.updated_at, "Fri, 15 Jan 2027 08:00:00 GMT");
    }

    #[test]
    fn live_feed_reuses_canonical_cash_economics() {
        let item = canonical_opportunity(
            &canonical("claimable", "1000000", true),
            "base-mainnet",
            "https://api.example",
        )
        .unwrap();
        let projection = OpportunityProjectionResponse {
            schema_version: OPPORTUNITY_PROJECTION_SCHEMA.to_string(),
            generated_at: "2027-01-15T08:01:00Z".to_string(),
            network: "base-mainnet".to_string(),
            applied_view: None,
            degraded: false,
            source_statuses: Vec::new(),
            items: vec![item],
            evidence_boundary: "Projection only".to_string(),
        };

        let feeds = render_opportunity_feeds(&projection, "https://api.example/");
        let json: Value = serde_json::from_str(&feeds.json).unwrap();
        let economics = &json["items"][0]["_bountyboard"]["cash_economics"];
        assert_eq!(economics["solver_reward"]["amount"], "900000");
        assert_eq!(economics["refundable_claim_bond"]["amount"], "100000");
        assert_eq!(economics["required_external_spend"]["amount"], "0");
        assert_eq!(economics["gross_cash_margin"]["amount"], "900000");
        assert!(feeds.rss.contains("Gross cash margin (not net profit)"));
        assert!(!feeds.rss.to_ascii_lowercase().contains("guaranteed profit"));
    }
    }
}
