use app::BountyStatusResponse;
use chain_base::{standing_meta_v2_parent_context, AutonomousBountyFeedItem};
use chrono::{DateTime, Utc};
use db::{TrialBounty, UnfundedBountySolution};
use domain::{BountyStatus, DiscoveryOpportunitySnapshot, DiscoveryRewardFilter, PrivacyLevel};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use utoipa::ToSchema;
use web_public::escape_html;

pub const OPPORTUNITY_PROJECTION_SCHEMA: &str = "agent-bounties/opportunity-projection-v1";

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
    pub standing_meta_bounty: bool,
    pub reward: OpportunityAmount,
    pub completion_bonus: Option<OpportunityAmount>,
    pub funded_amount: OpportunityAmount,
    pub funding_target: OpportunityAmount,
    pub bond: OpportunityAmount,
    pub deadline: Option<String>,
    pub deadline_kind: Option<String>,
    pub verification_method: String,
    pub verification_ready: bool,
    pub evidence_requirements: Value,
    pub terms_hash: Option<String>,
    pub proof_urls: Vec<String>,
    pub next_action: OpportunityNextAction,
    pub embeds: OpportunityEmbedLinks,
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
                "verification_method": item.verification_method,
                "verification_ready": item.verification_ready,
                "terms_hash": item.terms_hash,
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
    truncate_feed_text(
        &format!(
            "{goal}\n\nWork state: {}. Payment state: {}. {reward} Verification: {}. Next action: {}",
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
        standing_meta_bounty: false,
        reward: OpportunityAmount::usdc_base_units("0"),
        completion_bonus: None,
        funded_amount: OpportunityAmount::usdc_base_units("0"),
        funding_target: OpportunityAmount::usdc_base_units("0"),
        bond: OpportunityAmount::usdc_base_units("0"),
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
        embeds: opportunity_embed_links(api, &opportunity_id, None),
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
        standing_meta_bounty: false,
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
        embeds: opportunity_embed_links(api, &opportunity_id, None),
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
    let opportunity_id = format!("canonical:{network}:{}", item.bounty_contract);
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
        standing_meta_bounty: standing_meta_v2_parent_context(item).is_ok(),
        reward: OpportunityAmount::usdc_base_units(item.solver_reward.clone()),
        completion_bonus: Some(OpportunityAmount::usdc_base_units(
            item.timeout_bond_pool.clone(),
        )),
        funded_amount: OpportunityAmount::usdc_base_units(item.funded_amount.clone()),
        funding_target: OpportunityAmount::usdc_base_units(item.target_amount.clone()),
        bond: OpportunityAmount::usdc_base_units(item.claim_bond.clone()),
        deadline,
        deadline_kind,
        verification_method: item.verification_mode.clone(),
        verification_ready: state.verification_ready,
        evidence_requirements,
        terms_hash: Some(item.terms_hash.clone()),
        proof_urls,
        next_action,
        embeds: opportunity_embed_links(api, &opportunity_id, Some(network)),
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
            let matches = item.work_state == "claimable"
                && item.payment_state == "escrowed"
                && item.payment_committed
                && item.verification_ready;
            if matches {
                item.discovery_factors.push(
                    "view:ready_to_earn;factors=claimable+escrowed+verification_ready".to_string(),
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
    let left_ready =
        left.work_state == "claimable" && left.payment_committed && left.verification_ready;
    let right_ready =
        right.work_state == "claimable" && right.payment_committed && right.verification_ready;
    right_ready
        .cmp(&left_ready)
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
    use chain_base::{AutonomousBountyEvent, AutonomousBountyEventKind};
    use domain::{AutonomousBountyTermsDocument, AutonomousBountyTermsRecord};
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
            terms_hash: terms.terms_hash.clone(),
            terms: Some(terms),
            terms_valid: true,
            verification_mode: "signed_quorum".to_string(),
            verifier_module: None,
            verification_ready,
            verification_readiness_reason: "ready".to_string(),
            validation_errors: Vec::new(),
            events: vec![event],
        }
    }

    #[test]
    fn unfunded_projection_is_real_open_work_without_payment_commitment() {
        let item = unfunded_opportunity(&trial(), &[], "https://api.example");
        assert_eq!(item.work_state, "open");
        assert_eq!(item.payment_state, "none");
        assert!(!item.payment_committed);
        assert_eq!(item.reward.amount, "0");
        assert_eq!(item.next_action.action, "submit_unfunded_bounty_solution");
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
}
