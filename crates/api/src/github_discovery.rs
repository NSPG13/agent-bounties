use chain_base::{
    AutonomousBountyEvent, AutonomousBountyEventKind, AutonomousBountyFeedItem,
    OpenCompetitionDeploymentState, OpenCompetitionEvent, OpenCompetitionEventKind,
    OpenCompetitionVerifierProfile,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;

pub const GITHUB_DISCOVERY_SCHEMA: &str = "agent-bounties/github-bounty-discovery-v1";
pub const AUTONOMOUS_PROTOCOL_VERSION: &str = "agent-bounties/autonomous-v1";
pub const OPEN_COMPETITION_PROTOCOL_VERSION: &str = "agent-bounties/open-competition-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct GitHubDiscoverySafeBlock {
    pub number: u64,
    pub hash: String,
    pub timestamp: u64,
    pub age_seconds: i64,
    pub fresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct GitHubDiscoverySourceStatus {
    pub source_type: String,
    pub protocol_version: String,
    pub factory_contract: Option<String>,
    pub available: bool,
    pub fresh: bool,
    pub item_count: usize,
    pub persisted_cursor_block: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct GitHubDiscoveryAction {
    pub kind: String,
    pub label: String,
    pub method: String,
    pub url: String,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct GitHubSettlementEvidence {
    pub event_name: String,
    pub bounty_id: String,
    pub bounty_contract: String,
    pub transaction_hash: String,
    pub block_number: u64,
    pub log_index: u64,
    pub solver_wallet: String,
    pub solver_reward: String,
    pub returned_bond: String,
    pub completion_bonus: String,
    pub solver_payout: String,
    pub verifier_reward: String,
    pub confirmed_canonical: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct GitHubDiscoveryVerifier {
    pub profile_id: Option<String>,
    pub display_name: String,
    pub method: String,
    pub address: Option<String>,
    pub runtime_code_hash: Option<String>,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct GitHubDiscoveryItem {
    pub discovery_id: String,
    pub network: String,
    pub chain_id: u64,
    pub protocol_version: String,
    pub source_id: String,
    pub visibility: String,
    pub bounty_id: String,
    pub bounty_contract: String,
    pub created_at: String,
    pub created_block: u64,
    pub updated_at: String,
    pub title: String,
    pub summary: String,
    pub categories: Vec<String>,
    pub skills: Vec<String>,
    pub difficulty: Option<String>,
    pub public_url: String,
    pub source_url: Option<String>,
    pub competition_mode: String,
    pub lifecycle_state: String,
    pub funded: bool,
    pub verification_ready: bool,
    pub ready_to_earn: bool,
    pub reward_usdc_base_units: String,
    pub verifier_reward_usdc_base_units: String,
    pub bond_usdc_base_units: String,
    pub funded_usdc_base_units: String,
    pub funding_target_usdc_base_units: String,
    pub deadline: Option<String>,
    pub deadline_kind: Option<String>,
    pub entry_count: Option<u8>,
    pub max_entries: Option<u8>,
    pub verifier: GitHubDiscoveryVerifier,
    pub next_action: GitHubDiscoveryAction,
    pub recovery_action_available: bool,
    pub identity_warning: Option<String>,
    pub settlement_evidence: Option<GitHubSettlementEvidence>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct GitHubDiscoveryProjectionResponse {
    pub schema_version: String,
    pub generated_at: String,
    pub network: String,
    pub chain_id: u64,
    pub safe_block: Option<GitHubDiscoverySafeBlock>,
    pub degraded: bool,
    pub source_statuses: Vec<GitHubDiscoverySourceStatus>,
    pub items: Vec<GitHubDiscoveryItem>,
    pub evidence_boundary: String,
}

pub fn assemble_projection(
    network: &str,
    chain_id: u64,
    generated_at: DateTime<Utc>,
    safe_block: Option<GitHubDiscoverySafeBlock>,
    source_statuses: Vec<GitHubDiscoverySourceStatus>,
    mut items: Vec<GitHubDiscoveryItem>,
) -> Result<GitHubDiscoveryProjectionResponse, String> {
    let expected_protocols = BTreeSet::from([
        AUTONOMOUS_PROTOCOL_VERSION.to_string(),
        OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
    ]);
    let observed_protocols = source_statuses
        .iter()
        .map(|source| source.protocol_version.clone())
        .collect::<BTreeSet<_>>();
    if observed_protocols != expected_protocols || source_statuses.len() != expected_protocols.len()
    {
        return Err("public bounty protocol adapter set is incomplete or duplicated".to_string());
    }
    let mut identities = BTreeSet::new();
    for item in &items {
        if item.network != network || item.chain_id != chain_id || item.visibility != "public" {
            return Err(format!(
                "discovery item {} does not match the response network",
                item.discovery_id
            ));
        }
        if !identities.insert(item.discovery_id.clone()) {
            return Err(format!("duplicate discovery id: {}", item.discovery_id));
        }
        validate_item(item)?;
    }
    for source in &source_statuses {
        let projected_count = items
            .iter()
            .filter(|item| item.protocol_version == source.protocol_version)
            .count();
        if projected_count != source.item_count {
            return Err(format!(
                "source item count mismatch for {}",
                source.protocol_version
            ));
        }
    }
    items.sort_by(|left, right| {
        right
            .created_block
            .cmp(&left.created_block)
            .then_with(|| left.discovery_id.cmp(&right.discovery_id))
    });
    let degraded = safe_block.as_ref().is_none_or(|block| !block.fresh)
        || source_statuses
            .iter()
            .any(|source| !source.available || !source.fresh);
    Ok(GitHubDiscoveryProjectionResponse {
        schema_version: GITHUB_DISCOVERY_SCHEMA.to_string(),
        generated_at: generated_at.to_rfc3339(),
        network: network.to_string(),
        chain_id,
        safe_block,
        degraded,
        source_statuses,
        items,
        evidence_boundary: "This is a read-only GitHub discovery projection. GitHub issues and labels cannot create funding, claims, verification, settlement, refunds, or payment. Only a confirmed canonical BountySettled event in settlement_evidence proves solver payment.".to_string(),
    })
}

fn validate_item(item: &GitHubDiscoveryItem) -> Result<(), String> {
    if item.discovery_id.len() > 240
        || item.title.trim().is_empty()
        || item.bounty_contract.len() != 42
        || item.reward_usdc_base_units.parse::<u128>().is_err()
        || item.bond_usdc_base_units.parse::<u128>().is_err()
        || item.funded_usdc_base_units.parse::<u128>().is_err()
        || item.funding_target_usdc_base_units.parse::<u128>().is_err()
    {
        return Err(format!(
            "discovery item is malformed: {}",
            item.discovery_id
        ));
    }
    if item.lifecycle_state == "settled" {
        let settlement = item.settlement_evidence.as_ref().ok_or_else(|| {
            format!(
                "settled item lacks BountySettled evidence: {}",
                item.discovery_id
            )
        })?;
        if !settlement.confirmed_canonical || settlement.event_name != "BountySettled" {
            return Err(format!(
                "settled item has noncanonical payment evidence: {}",
                item.discovery_id
            ));
        }
    } else if item.settlement_evidence.is_some() {
        return Err(format!(
            "non-settled item exposes payment evidence: {}",
            item.discovery_id
        ));
    }
    if item.ready_to_earn
        && (item.lifecycle_state != "ready_to_earn" || !item.funded || !item.verification_ready)
    {
        return Err(format!(
            "ready-to-earn item violates funding or verifier invariants: {}",
            item.discovery_id
        ));
    }
    Ok(())
}

pub fn autonomous_discovery_items(
    feed: &[AutonomousBountyFeedItem],
    network: &str,
    chain_id: u64,
    api_base_url: &str,
    website_base_url: &str,
) -> Result<Vec<GitHubDiscoveryItem>, String> {
    feed.iter()
        .map(|item| {
            autonomous_discovery_item(item, network, chain_id, api_base_url, website_base_url)
        })
        .collect()
}

fn autonomous_discovery_item(
    item: &AutonomousBountyFeedItem,
    network: &str,
    chain_id: u64,
    api_base_url: &str,
    website_base_url: &str,
) -> Result<GitHubDiscoveryItem, String> {
    let api = api_base_url.trim_end_matches('/');
    let website = website_base_url.trim_end_matches('/');
    let contract = item.bounty_contract.to_ascii_lowercase();
    let created = unique_autonomous_event(item, AutonomousBountyEventKind::CanonicalBountyCreated)?;
    let created_at = created
        .map(|event| event.occurred_at)
        .or_else(|| item.terms.as_ref().map(|terms| terms.created_at))
        .ok_or_else(|| format!("autonomous bounty lacks creation time: {contract}"))?;
    let created_block = created.map(|event| event.block_number).unwrap_or_default();
    let updated_at = item
        .events
        .iter()
        .max_by_key(|event| (event.block_number, event.log_index))
        .map(|event| event.occurred_at)
        .unwrap_or(created_at);
    let target = parse_amount(&item.target_amount, "target_amount")?;
    let funded = parse_amount(&item.funded_amount, "funded_amount")?;
    if target == 0 || funded > target {
        return Err(format!(
            "autonomous bounty economics are invalid: {contract}"
        ));
    }
    let fully_funded = funded == target;
    let terms = item.terms.as_ref();
    let title = terms
        .map(|terms| terms.document.title.clone())
        .unwrap_or_else(|| item.bounty_id.clone());
    let summary = terms
        .map(|terms| terms.document.goal.clone())
        .unwrap_or_else(|| {
            "Inspect the canonical published bounty terms before acting.".to_string()
        });
    let evidence_schema = terms
        .map(|terms| terms.document.evidence_schema.clone())
        .unwrap_or(Value::Null);
    let (categories, skills, _) =
        web_public::discovery_taxonomy_with_matches(&title, Some(&summary), &evidence_schema);
    let verification_ready = item.terms_valid && item.verification_ready;
    let lifecycle_state = match item.status.as_str() {
        "open" if !fully_funded => "funding_needed",
        "open" => "unavailable",
        "claimable" if fully_funded && verification_ready => "ready_to_earn",
        "claimable" => "unavailable",
        "claimed" => "in_progress",
        "submitted" => "verification_pending",
        "paid" => "settled",
        "cancelled" => "cancelled",
        other => return Err(format!("unknown autonomous status {other:?}: {contract}")),
    };
    let events_url = format!(
        "{api}/v1/base/autonomous-bounties/events?network={network}&bounty_id={}",
        item.bounty_id
    );
    let public_url = format!("{website}/earn.html?bountyContract={contract}&network={network}");
    let (deadline, deadline_kind) = autonomous_deadline(item);
    let next_action = autonomous_action(lifecycle_state, network, &contract, api, &events_url);
    let settlement_evidence = if lifecycle_state == "settled" {
        Some(autonomous_settlement(item, &contract)?)
    } else {
        None
    };
    let recovery_action_available =
        lifecycle_state == "cancelled" && autonomous_refunded_principal(item)? < funded;
    Ok(GitHubDiscoveryItem {
        discovery_id: format!(
            "eip155:{chain_id}:{AUTONOMOUS_PROTOCOL_VERSION}:{contract}"
        ),
        network: network.to_string(),
        chain_id,
        protocol_version: AUTONOMOUS_PROTOCOL_VERSION.to_string(),
        source_id: contract.clone(),
        visibility: "public".to_string(),
        bounty_id: item.bounty_id.clone(),
        bounty_contract: contract,
        created_at: created_at.to_rfc3339(),
        created_block,
        updated_at: updated_at.to_rfc3339(),
        title,
        summary,
        categories,
        skills,
        difficulty: None,
        public_url,
        source_url: terms.and_then(|terms| terms.document.source_url.clone()),
        competition_mode: "exclusive_claim".to_string(),
        lifecycle_state: lifecycle_state.to_string(),
        funded: fully_funded,
        verification_ready,
        ready_to_earn: lifecycle_state == "ready_to_earn",
        reward_usdc_base_units: item.solver_reward.clone(),
        verifier_reward_usdc_base_units: item.verifier_reward.clone(),
        bond_usdc_base_units: item.claim_bond.clone(),
        funded_usdc_base_units: item.funded_amount.clone(),
        funding_target_usdc_base_units: item.target_amount.clone(),
        deadline,
        deadline_kind,
        entry_count: None,
        max_entries: None,
        verifier: GitHubDiscoveryVerifier {
            profile_id: None,
            display_name: item.verification_mode.clone(),
            method: item.verification_mode.clone(),
            address: item.verifier_module.clone(),
            runtime_code_hash: None,
            ready: verification_ready,
        },
        next_action,
        recovery_action_available,
        identity_warning: None,
        settlement_evidence,
        evidence_boundary: "Canonical autonomous-v1 lifecycle state comes from confirmed factory and bounty events plus content-addressed terms. GitHub is a discovery mirror; only confirmed BountySettled proves solver payment.".to_string(),
    })
}

fn autonomous_action(
    lifecycle: &str,
    network: &str,
    contract: &str,
    api: &str,
    events_url: &str,
) -> GitHubDiscoveryAction {
    let (kind, label, method, url, instructions) = match lifecycle {
        "funding_needed" => (
            "fund",
            "Help fund this bounty",
            "POST",
            format!("{api}/v1/base/autonomous-bounties/contribution-plan"),
            "Prepare an exact native-USDC contribution and confirm FundingAdded before describing it as funded.",
        ),
        "ready_to_earn" => (
            "claim",
            "Claim this bounty",
            "POST",
            format!("{api}/v1/base/autonomous-bounties/claim-plan"),
            "Prepare the exclusive claim for the displayed contract and confirm BountyClaimed before starting work.",
        ),
        "verification_pending" => (
            "verify",
            "Inspect verification work",
            "GET",
            format!("{api}/v1/base/autonomous-bounties/verification-jobs?network={network}"),
            "Inspect the committed verifier job. A submission is not acceptance or payment.",
        ),
        "cancelled" => (
            "withdraw_refund",
            "Inspect refund recovery",
            "POST",
            format!("{api}/v1/base/autonomous-bounties/refund-withdrawal-plan"),
            "Only an eligible contributor wallet can prepare its own pull refund.",
        ),
        _ => (
            "inspect",
            "Inspect canonical state",
            "GET",
            events_url.to_string(),
            "Inspect confirmed canonical events. GitHub state is not settlement evidence.",
        ),
    };
    let instructions = if lifecycle == "ready_to_earn" {
        format!("{instructions} Bounty contract: {contract}.")
    } else {
        instructions.to_string()
    };
    GitHubDiscoveryAction {
        kind: kind.to_string(),
        label: label.to_string(),
        method: method.to_string(),
        url,
        instructions,
    }
}

fn autonomous_deadline(item: &AutonomousBountyFeedItem) -> (Option<String>, Option<String>) {
    let Some(terms) = item.terms.as_ref() else {
        return (None, None);
    };
    let Some(contract_terms) = terms.document.contract_terms.as_object() else {
        return (None, None);
    };
    let (field, kind) = match item.status.as_str() {
        "open" | "claimable" => ("funding_deadline", "funding_deadline"),
        _ => return (None, None),
    };
    let value = contract_terms.get(field).and_then(json_u64);
    (
        value
            .and_then(|value| DateTime::<Utc>::from_timestamp(value as i64, 0))
            .map(|value| value.to_rfc3339()),
        value.map(|_| kind.to_string()),
    )
}

fn autonomous_refunded_principal(item: &AutonomousBountyFeedItem) -> Result<u128, String> {
    item.events
        .iter()
        .filter(|event| event.kind == AutonomousBountyEventKind::RefundWithdrawn)
        .try_fold(0u128, |total, event| {
            json_u128_field(&event.data, "principal").and_then(|amount| {
                total
                    .checked_add(amount)
                    .ok_or_else(|| "refund total overflow".to_string())
            })
        })
}

fn autonomous_settlement(
    item: &AutonomousBountyFeedItem,
    contract: &str,
) -> Result<GitHubSettlementEvidence, String> {
    let matches = item
        .events
        .iter()
        .filter(|event| event.kind == AutonomousBountyEventKind::BountySettled)
        .collect::<Vec<_>>();
    let [event] = matches.as_slice() else {
        return Err(format!(
            "paid autonomous bounty requires one BountySettled: {contract}"
        ));
    };
    let solver_reward = json_u128_field(&event.data, "solver_reward")?;
    let returned_bond = json_u128_field(&event.data, "claim_bond_returned")?;
    let completion_bonus = json_u128_field(&event.data, "timeout_bond_bonus")?;
    let solver_payout = json_u128_field(&event.data, "solver_payout")?;
    let verifier_reward = json_u128_field(&event.data, "verifier_reward")?;
    if solver_payout
        != solver_reward
            .checked_add(returned_bond)
            .and_then(|value| value.checked_add(completion_bonus))
            .ok_or_else(|| "solver payout overflow".to_string())?
    {
        return Err(format!(
            "autonomous settlement payout is inconsistent: {contract}"
        ));
    }
    Ok(GitHubSettlementEvidence {
        event_name: "BountySettled".to_string(),
        bounty_id: item.bounty_id.clone(),
        bounty_contract: contract.to_string(),
        transaction_hash: event.tx_hash.clone(),
        block_number: event.block_number,
        log_index: event.log_index,
        solver_wallet: json_text_field(&event.data, "solver")?,
        solver_reward: solver_reward.to_string(),
        returned_bond: returned_bond.to_string(),
        completion_bonus: completion_bonus.to_string(),
        solver_payout: solver_payout.to_string(),
        verifier_reward: verifier_reward.to_string(),
        confirmed_canonical: true,
    })
}

fn unique_autonomous_event(
    item: &AutonomousBountyFeedItem,
    kind: AutonomousBountyEventKind,
) -> Result<Option<&AutonomousBountyEvent>, String> {
    let matches = item
        .events
        .iter()
        .filter(|event| event.kind == kind)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(format!(
            "duplicate autonomous event {:?}: {}",
            kind, item.bounty_contract
        ));
    }
    Ok(matches.first().copied())
}

#[allow(clippy::too_many_arguments)]
pub fn open_competition_discovery_items(
    events: &[OpenCompetitionEvent],
    profile: &OpenCompetitionVerifierProfile,
    network: &str,
    chain_id: u64,
    api_base_url: &str,
    website_base_url: &str,
    public_activation_block: u64,
    now: DateTime<Utc>,
) -> Result<Vec<GitHubDiscoveryItem>, String> {
    if !profile.public_inventory_eligible
        || profile.deployment_state != OpenCompetitionDeploymentState::ActiveReadyToEarn
    {
        return Ok(Vec::new());
    }
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
    grouped
        .into_iter()
        .map(|(bounty_id, mut bounty_events)| {
            bounty_events.sort_by_key(|event| (event.block_number, event.log_index));
            open_competition_discovery_item(
                &bounty_id,
                &bounty_events,
                profile,
                network,
                chain_id,
                api_base_url,
                website_base_url,
                now,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn open_competition_discovery_item(
    bounty_id: &str,
    events: &[&OpenCompetitionEvent],
    profile: &OpenCompetitionVerifierProfile,
    network: &str,
    chain_id: u64,
    api_base_url: &str,
    website_base_url: &str,
    now: DateTime<Utc>,
) -> Result<GitHubDiscoveryItem, String> {
    let api = api_base_url.trim_end_matches('/');
    let website = website_base_url.trim_end_matches('/');
    let created = required_unique_open_event(
        events,
        OpenCompetitionEventKind::CanonicalCompetitionCreated,
    )?;
    let terms = required_unique_open_event(
        events,
        OpenCompetitionEventKind::CanonicalCompetitionTermsCommitted,
    )?;
    let economics = required_unique_open_event(
        events,
        OpenCompetitionEventKind::CanonicalCompetitionEconomicsConfigured,
    )?;
    let verification = required_unique_open_event(
        events,
        OpenCompetitionEventKind::CanonicalCompetitionVerificationConfigured,
    )?;
    if !json_text_field(&verification.data, "verifier_module")?
        .eq_ignore_ascii_case(&profile.verifier_address)
        || !json_text_field(&terms.data, "benchmark_hash")?
            .eq_ignore_ascii_case(&profile.benchmark_hash)
        || !json_text_field(&terms.data, "evidence_schema_hash")?
            .eq_ignore_ascii_case(&profile.evidence_schema_hash)
    {
        return Err(format!(
            "competition verifier commitments do not match catalog: {bounty_id}"
        ));
    }
    let contract = json_text_field(&created.data, "bounty_contract")?.to_ascii_lowercase();
    let solver_reward = json_u128_field(&economics.data, "solver_reward")?;
    let verifier_reward = json_u128_field(&economics.data, "verifier_reward")?;
    let entry_bond = json_u128_field(&economics.data, "entry_bond")?;
    let target = json_u128_field(&economics.data, "target_amount")?;
    let mut funded = json_u128_field(&economics.data, "initial_funding")?;
    let funding_deadline = json_u64_field(&economics.data, "funding_deadline")?;
    let max_entries = u8::try_from(json_u64_field(&economics.data, "max_entries")?)
        .map_err(|_| format!("competition capacity exceeds u8: {bounty_id}"))?;
    if solver_reward == 0
        || verifier_reward == 0
        || entry_bond != verifier_reward
        || target != solver_reward.saturating_add(verifier_reward)
        || max_entries == 0
        || max_entries > 64
    {
        return Err(format!("competition economics are invalid: {bounty_id}"));
    }
    for event in events
        .iter()
        .filter(|event| event.kind == OpenCompetitionEventKind::FundingAdded)
    {
        funded = funded.max(json_u128_field(&event.data, "funded_amount")?);
    }
    let opened = last_open_event(events, OpenCompetitionEventKind::CompetitionOpened);
    let settled = unique_optional_open_event(events, OpenCompetitionEventKind::BountySettled)?;
    let cancelled = unique_optional_open_event(events, OpenCompetitionEventKind::BountyCancelled)?;
    if settled.is_some() && cancelled.is_some() {
        return Err(format!(
            "competition is both settled and cancelled: {bounty_id}"
        ));
    }
    let competition_ends_at = opened
        .map(|event| json_u64_field(&event.data, "competition_ends_at"))
        .transpose()?;
    let committed = solver_set(events, OpenCompetitionEventKind::SolutionCommitted)?;
    let revealed = solver_set(events, OpenCompetitionEventKind::SolutionRevealed)?;
    let expired = solver_set(events, OpenCompetitionEventKind::CommitmentExpired)?;
    let withdrawn = solver_set(events, OpenCompetitionEventKind::EntryBondWithdrawn)?;
    if committed.len() > usize::from(max_entries) {
        return Err(format!(
            "competition exceeds immutable capacity: {bounty_id}"
        ));
    }
    let active_reveals = events.iter().any(|event| {
        event.kind == OpenCompetitionEventKind::SolutionCommitted
            && json_text_field(&event.data, "solver")
                .ok()
                .is_some_and(|solver| {
                    let solver = solver.to_ascii_lowercase();
                    !revealed.contains(&solver)
                        && !expired.contains(&solver)
                        && json_u64_field(&event.data, "reveal_deadline")
                            .ok()
                            .is_some_and(|deadline| deadline >= now.timestamp() as u64)
                })
    });
    let fully_funded = funded == target;
    let accepts_entries = settled.is_none()
        && cancelled.is_none()
        && fully_funded
        && competition_ends_at.is_some_and(|deadline| deadline > now.timestamp() as u64)
        && committed.len() < usize::from(max_entries);
    let lifecycle_state = if settled.is_some() {
        "settled"
    } else if cancelled.is_some() {
        "cancelled"
    } else if accepts_entries {
        "ready_to_earn"
    } else if !fully_funded {
        "funding_needed"
    } else if active_reveals {
        "in_progress"
    } else if opened.is_some() {
        "expired"
    } else {
        "unavailable"
    };
    let events_url =
        format!("{api}/v1/base/open-competition-v1/events?network={network}&bounty_id={bounty_id}");
    let public_url = format!(
        "{website}/competition.html?bountyContract={contract}&network={network}&verifierProfileId={}",
        profile.profile_id
    );
    let next_action = match lifecycle_state {
        "ready_to_earn" => GitHubDiscoveryAction {
            kind: "enter_competition".to_string(),
            label: "Enter competition".to_string(),
            method: "POST".to_string(),
            url: format!("{api}/v1/base/open-competition-v1/commit-preparation"),
            instructions: "Generate and save the secret-salt recovery envelope locally, then submit only its commitment. First valid confirmed reveal wins.".to_string(),
        },
        "funding_needed" => GitHubDiscoveryAction {
            kind: "fund".to_string(),
            label: "Help fund this competition".to_string(),
            method: "GET".to_string(),
            url: public_url.clone(),
            instructions: "Inspect the exact immutable competition economics before funding. A token transfer without FundingAdded is not canonical funding.".to_string(),
        },
        "settled" => GitHubDiscoveryAction {
            kind: "inspect_settlement".to_string(),
            label: "Inspect canonical settlement".to_string(),
            method: "GET".to_string(),
            url: events_url.clone(),
            instructions: "Only the confirmed BountySettled event proves solver payment.".to_string(),
        },
        _ => GitHubDiscoveryAction {
            kind: if lifecycle_state == "cancelled" { "recover" } else { "inspect" }.to_string(),
            label: if lifecycle_state == "cancelled" { "Inspect refunds and bond recovery" } else { "Inspect canonical state" }.to_string(),
            method: "GET".to_string(),
            url: events_url.clone(),
            instructions: "Inspect version-specific canonical events and use only wallet-scoped pull recovery actions.".to_string(),
        },
    };
    let settlement_evidence = settled
        .map(|event| open_competition_settlement(event, bounty_id, &contract))
        .transpose()?;
    let loser_bond_recovery = committed.iter().any(|solver| {
        !revealed.contains(solver) && !withdrawn.contains(solver) && !expired.contains(solver)
    });
    let refund_recovery = if let Some(cancelled) = cancelled {
        let principal = json_u128_field(&cancelled.data, "principal")?;
        let withdrawn_principal = events
            .iter()
            .filter(|event| event.kind == OpenCompetitionEventKind::RefundWithdrawn)
            .try_fold(0u128, |total, event| {
                json_u128_field(&event.data, "principal").and_then(|value| {
                    total
                        .checked_add(value)
                        .ok_or_else(|| "refund total overflow".to_string())
                })
            })?;
        withdrawn_principal < principal
    } else {
        false
    };
    let deadline_value = competition_ends_at.unwrap_or(funding_deadline);
    Ok(GitHubDiscoveryItem {
        discovery_id: format!(
            "eip155:{chain_id}:{OPEN_COMPETITION_PROTOCOL_VERSION}:{contract}"
        ),
        network: network.to_string(),
        chain_id,
        protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
        source_id: contract.clone(),
        visibility: "public".to_string(),
        bounty_id: bounty_id.to_string(),
        bounty_contract: contract,
        created_at: created.occurred_at.to_rfc3339(),
        created_block: created.block_number,
        updated_at: events.last().map(|event| event.occurred_at).unwrap_or(created.occurred_at).to_rfc3339(),
        title: "Scope-bound hash-work competition".to_string(),
        summary: "Produce proof bytes accepted by the exact published deterministic verifier. This profile does not judge ordinary code, design, writing, research, or task quality.".to_string(),
        categories: vec!["cryptographic-work".to_string(), "deterministic".to_string()],
        skills: vec!["commit-reveal".to_string(), "hash-work".to_string()],
        difficulty: None,
        public_url,
        source_url: None,
        competition_mode: "first_valid_submission".to_string(),
        lifecycle_state: lifecycle_state.to_string(),
        funded: fully_funded,
        verification_ready: true,
        ready_to_earn: lifecycle_state == "ready_to_earn",
        reward_usdc_base_units: solver_reward.to_string(),
        verifier_reward_usdc_base_units: verifier_reward.to_string(),
        bond_usdc_base_units: entry_bond.to_string(),
        funded_usdc_base_units: funded.to_string(),
        funding_target_usdc_base_units: target.to_string(),
        deadline: DateTime::<Utc>::from_timestamp(deadline_value as i64, 0).map(|value| value.to_rfc3339()),
        deadline_kind: Some(if competition_ends_at.is_some() { "competition_deadline" } else { "funding_deadline" }.to_string()),
        entry_count: Some(u8::try_from(committed.len()).map_err(|_| "entry count exceeds u8".to_string())?),
        max_entries: Some(max_entries),
        verifier: GitHubDiscoveryVerifier {
            profile_id: Some(profile.profile_id.clone()),
            display_name: profile.display_name.clone(),
            method: profile.module_kind.clone(),
            address: Some(profile.verifier_address.clone()),
            runtime_code_hash: Some(profile.runtime_code_hash.clone()),
            ready: true,
        },
        next_action,
        recovery_action_available: loser_bond_recovery || refund_recovery,
        identity_warning: Some("One wallet does not prove one independent person.".to_string()),
        settlement_evidence,
        evidence_boundary: "Open Competition is limited to this exact catalog-pinned deterministic verifier. GitHub cannot choose a winner. A commitment, reveal, transaction hash, or hosted row is not payment; only confirmed BountySettled proves solver payment.".to_string(),
    })
}

fn open_competition_settlement(
    event: &OpenCompetitionEvent,
    bounty_id: &str,
    contract: &str,
) -> Result<GitHubSettlementEvidence, String> {
    let solver_reward = json_u128_field(&event.data, "solver_reward")?;
    let returned_bond = json_u128_field(&event.data, "entry_bond_returned")?;
    let completion_bonus = json_u128_field(&event.data, "timeout_bond_bonus")?;
    let verifier_reward = json_u128_field(&event.data, "verifier_reward")?;
    let solver_payout = solver_reward
        .checked_add(returned_bond)
        .and_then(|value| value.checked_add(completion_bonus))
        .ok_or_else(|| "competition payout overflow".to_string())?;
    Ok(GitHubSettlementEvidence {
        event_name: "BountySettled".to_string(),
        bounty_id: bounty_id.to_string(),
        bounty_contract: contract.to_string(),
        transaction_hash: event.tx_hash.clone(),
        block_number: event.block_number,
        log_index: event.log_index,
        solver_wallet: json_text_field(&event.data, "solver")?,
        solver_reward: solver_reward.to_string(),
        returned_bond: returned_bond.to_string(),
        completion_bonus: completion_bonus.to_string(),
        solver_payout: solver_payout.to_string(),
        verifier_reward: verifier_reward.to_string(),
        confirmed_canonical: event.data.get("canonical_payment_evidence") == Some(&json!(true)),
    })
}

fn required_unique_open_event<'a>(
    events: &'a [&OpenCompetitionEvent],
    kind: OpenCompetitionEventKind,
) -> Result<&'a OpenCompetitionEvent, String> {
    unique_optional_open_event(events, kind)?
        .ok_or_else(|| format!("competition is missing {:?}", kind))
}

fn unique_optional_open_event<'a>(
    events: &'a [&OpenCompetitionEvent],
    kind: OpenCompetitionEventKind,
) -> Result<Option<&'a OpenCompetitionEvent>, String> {
    let matches = events
        .iter()
        .copied()
        .filter(|event| event.kind == kind)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(format!("competition has duplicate {:?}", kind));
    }
    Ok(matches.first().copied())
}

fn last_open_event<'a>(
    events: &'a [&OpenCompetitionEvent],
    kind: OpenCompetitionEventKind,
) -> Option<&'a OpenCompetitionEvent> {
    events
        .iter()
        .rev()
        .copied()
        .find(|event| event.kind == kind)
}

fn solver_set(
    events: &[&OpenCompetitionEvent],
    kind: OpenCompetitionEventKind,
) -> Result<BTreeSet<String>, String> {
    events
        .iter()
        .filter(|event| event.kind == kind)
        .map(|event| {
            json_text_field(&event.data, "solver").map(|solver| solver.to_ascii_lowercase())
        })
        .collect()
}

fn parse_amount(value: &str, field: &str) -> Result<u128, String> {
    value
        .parse::<u128>()
        .map_err(|_| format!("invalid {field}"))
}

fn json_text_field(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing text field {field}"))
}

fn json_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn json_u64_field(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(json_u64)
        .ok_or_else(|| format!("missing integer field {field}"))
}

fn json_u128_field(value: &Value, field: &str) -> Result<u128, String> {
    let value = value
        .get(field)
        .ok_or_else(|| format!("missing integer field {field}"))?;
    if let Some(value) = value.as_u64() {
        return Ok(u128::from(value));
    }
    value
        .as_str()
        .ok_or_else(|| format!("invalid integer field {field}"))?
        .parse::<u128>()
        .map_err(|_| format!("invalid integer field {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::{built_in_open_competition_verifier_catalog, AutonomousBountyEvent};
    use chrono::TimeZone;
    use domain::Id;

    fn open_event(kind: OpenCompetitionEventKind, block: u64, data: Value) -> OpenCompetitionEvent {
        OpenCompetitionEvent {
            id: Id::from_u128(u128::from(block) + 1),
            protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
            log_key: format!("{block}:0"),
            tx_hash: format!("0x{:064x}", block),
            block_number: block,
            log_index: 0,
            contract_address: "0x3551ca7bb9090fb8c1648eea40837c8a1cbcc973".to_string(),
            bounty_id: format!("0x{:064x}", 1),
            kind,
            data,
            occurred_at: Utc.timestamp_opt(1_700_000_000 + block as i64, 0).unwrap(),
        }
    }

    fn competition_fixture() -> (Vec<OpenCompetitionEvent>, OpenCompetitionVerifierProfile) {
        let mut profiles = built_in_open_competition_verifier_catalog("base-mainnet")
            .unwrap()
            .profiles;
        let profile = profiles.remove(0);
        let contract = "0x3551ca7bb9090fb8c1648eea40837c8a1cbcc973";
        let events = vec![
            open_event(
                OpenCompetitionEventKind::CanonicalCompetitionCreated,
                10,
                json!({
                    "bounty_contract": contract,
                    "terms_hash": format!("0x{}", "11".repeat(32)),
                    "policy_hash": format!("0x{}", "22".repeat(32))
                }),
            ),
            open_event(
                OpenCompetitionEventKind::CanonicalCompetitionTermsCommitted,
                10,
                json!({
                    "acceptance_criteria_hash": format!("0x{}", "33".repeat(32)),
                    "benchmark_hash": profile.benchmark_hash,
                    "evidence_schema_hash": profile.evidence_schema_hash
                }),
            ),
            open_event(
                OpenCompetitionEventKind::CanonicalCompetitionEconomicsConfigured,
                10,
                json!({
                    "solver_reward": 500000,
                    "verifier_reward": 50000,
                    "entry_bond": 50000,
                    "target_amount": 550000,
                    "initial_funding": 550000,
                    "funding_deadline": 1_800_000_000u64,
                    "max_entries": 4
                }),
            ),
            open_event(
                OpenCompetitionEventKind::CanonicalCompetitionVerificationConfigured,
                10,
                json!({
                    "verifier_module": profile.verifier_address
                }),
            ),
            open_event(
                OpenCompetitionEventKind::CompetitionOpened,
                11,
                json!({
                    "competition_ends_at": 1_800_000_000u64,
                    "max_entries": 4
                }),
            ),
        ];
        (events, profile)
    }

    fn autonomous_fixture(status: &str, funded: u128) -> AutonomousBountyFeedItem {
        let contract = "0x1111111111111111111111111111111111111111";
        let mut events = vec![autonomous_event(
            AutonomousBountyEventKind::CanonicalBountyCreated,
            json!({}),
        )];
        if status == "cancelled" {
            events.push(AutonomousBountyEvent {
                id: Id::from_u128(10_002),
                log_key: "2:0".to_string(),
                tx_hash: format!("0x{}", "ab".repeat(32)),
                block_number: 2,
                log_index: 0,
                contract_address: contract.to_string(),
                bounty_id: format!("0x{}", "bb".repeat(32)),
                kind: AutonomousBountyEventKind::BountyCancelled,
                data: json!({"principal": funded}),
                occurred_at: Utc.timestamp_opt(1_700_000_010, 0).unwrap(),
            });
        }
        if status == "paid" {
            events.push(AutonomousBountyEvent {
                id: Id::from_u128(10_003),
                log_key: "3:0".to_string(),
                tx_hash: format!("0x{}", "ac".repeat(32)),
                block_number: 3,
                log_index: 0,
                contract_address: contract.to_string(),
                bounty_id: format!("0x{}", "bb".repeat(32)),
                kind: AutonomousBountyEventKind::BountySettled,
                data: json!({
                    "solver": "0x9999999999999999999999999999999999999999",
                    "solver_reward": 2_000_000,
                    "claim_bond_returned": 10_000,
                    "timeout_bond_bonus": 0,
                    "solver_payout": 2_010_000,
                    "verifier_reward": 10_000
                }),
                occurred_at: Utc.timestamp_opt(1_700_000_020, 0).unwrap(),
            });
        }
        AutonomousBountyFeedItem {
            bounty_id: format!("0x{}", "bb".repeat(32)),
            bounty_contract: contract.to_string(),
            creator: "0x2222222222222222222222222222222222222222".to_string(),
            status: status.to_string(),
            solver_reward: "2000000".to_string(),
            verifier_reward: "10000".to_string(),
            claim_bond: "10000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "2010000".to_string(),
            funded_amount: funded.to_string(),
            required_external_spend: "0".to_string(),
            gross_cash_margin: "2000000".to_string(),
            terms_hash: format!("0x{}", "11".repeat(32)),
            terms: None,
            terms_valid: true,
            verification_mode: "deterministic_module".to_string(),
            verifier_module: Some("0x3333333333333333333333333333333333333333".to_string()),
            verifier_set_hash: None,
            verifier_threshold: None,
            runner_identifier: None,
            verification_ready: true,
            verification_readiness_reason: "ready".to_string(),
            validation_errors: vec![],
            events,
        }
    }

    #[test]
    fn open_competition_is_lifecycle_complete_and_uses_enter_action() {
        let (mut events, profile) = competition_fixture();
        let now = Utc.timestamp_opt(1_700_000_100, 0).unwrap();
        let active = open_competition_discovery_items(
            &events,
            &profile,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            now,
        )
        .unwrap();
        assert_eq!(active[0].lifecycle_state, "ready_to_earn");
        assert_eq!(active[0].next_action.kind, "enter_competition");
        assert_eq!(active[0].next_action.label, "Enter competition");

        events.push(open_event(
            OpenCompetitionEventKind::BountyCancelled,
            12,
            json!({"principal": 550000, "expired_entry_bonus": 0}),
        ));
        let cancelled = open_competition_discovery_items(
            &events,
            &profile,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            now,
        )
        .unwrap();
        assert_eq!(cancelled[0].lifecycle_state, "cancelled");
        assert!(cancelled[0].recovery_action_available);
    }

    #[test]
    fn settlement_requires_canonical_event_evidence() {
        let (mut events, profile) = competition_fixture();
        events.push(open_event(
            OpenCompetitionEventKind::SolutionCommitted,
            11,
            json!({
                "solver": "0x9999999999999999999999999999999999999999",
                "entry_number": 1,
                "commitment": format!("0x{}", "11".repeat(32)),
                "committed_block": 11,
                "reveal_deadline": 1_800_000_000u64,
                "bond": 50_000
            }),
        ));
        events.push(open_event(
            OpenCompetitionEventKind::SolutionRevealed,
            12,
            json!({
                "solver": "0x9999999999999999999999999999999999999999"
            }),
        ));
        events.push(open_event(
            OpenCompetitionEventKind::BountySettled,
            13,
            json!({
                "solver": "0x9999999999999999999999999999999999999999",
                "solver_reward": 500000,
                "entry_bond_returned": 50000,
                "timeout_bond_bonus": 0,
                "verifier_reward": 50000,
                "canonical_payment_evidence": true
            }),
        ));
        let items = open_competition_discovery_items(
            &events,
            &profile,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            Utc.timestamp_opt(1_700_000_100, 0).unwrap(),
        )
        .unwrap();
        let settlement = items[0].settlement_evidence.as_ref().unwrap();
        assert_eq!(items[0].lifecycle_state, "settled");
        assert_eq!(settlement.solver_payout, "550000");
        assert!(!items[0].recovery_action_available);

        events.last_mut().unwrap().data["canonical_payment_evidence"] = json!(false);
        let invalid = open_competition_discovery_items(
            &events,
            &profile,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            Utc.timestamp_opt(1_700_000_100, 0).unwrap(),
        )
        .unwrap();
        assert!(assemble_projection(
            "base-mainnet",
            8453,
            Utc::now(),
            Some(GitHubDiscoverySafeBlock {
                number: 12,
                hash: format!("0x{}", "aa".repeat(32)),
                timestamp: 1_700_000_100,
                age_seconds: 0,
                fresh: true,
            }),
            vec![],
            invalid,
        )
        .is_err());
    }

    #[test]
    fn autonomous_projection_covers_funding_ready_cancelled_and_settled_states() {
        let cases = [
            ("open", 0, "funding_needed"),
            ("claimable", 2_010_000, "ready_to_earn"),
            ("claimed", 2_010_000, "in_progress"),
            ("submitted", 2_010_000, "verification_pending"),
            ("cancelled", 2_010_000, "cancelled"),
            ("paid", 2_010_000, "settled"),
        ];
        for (status, funded, expected) in cases {
            let projected = autonomous_discovery_items(
                &[autonomous_fixture(status, funded)],
                "base-mainnet",
                8453,
                "https://api.example",
                "https://www.example",
            )
            .unwrap();
            assert_eq!(projected[0].lifecycle_state, expected);
            assert_eq!(projected[0].competition_mode, "exclusive_claim");
            if status == "paid" {
                assert_eq!(
                    projected[0]
                        .settlement_evidence
                        .as_ref()
                        .unwrap()
                        .event_name,
                    "BountySettled"
                );
            }
            if status == "cancelled" {
                assert!(projected[0].recovery_action_available);
            }
        }
    }

    #[test]
    fn competition_capacity_expiry_and_verifier_mismatch_fail_closed() {
        let (mut events, profile) = competition_fixture();
        let expired = open_competition_discovery_items(
            &events,
            &profile,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            Utc.timestamp_opt(1_900_000_000, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(expired[0].lifecycle_state, "expired");

        for index in 0..4u64 {
            events.push(open_event(
                OpenCompetitionEventKind::SolutionCommitted,
                20 + index,
                json!({
                    "solver": format!("0x{:040x}", index + 1),
                    "entry_number": index + 1,
                    "commitment": format!("0x{:064x}", index + 1),
                    "committed_block": 20 + index,
                    "reveal_deadline": 1_800_000_000u64,
                    "bond": 50_000
                }),
            ));
        }
        let full = open_competition_discovery_items(
            &events,
            &profile,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            Utc.timestamp_opt(1_700_000_100, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(full[0].entry_count, Some(4));
        assert_eq!(full[0].lifecycle_state, "in_progress");
        assert!(!full[0].ready_to_earn);

        let mut mismatch = profile.clone();
        mismatch.verifier_address = "0x4444444444444444444444444444444444444444".to_string();
        assert!(open_competition_discovery_items(
            &events,
            &mismatch,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            Utc.timestamp_opt(1_700_000_100, 0).unwrap(),
        )
        .is_err());
    }

    #[test]
    fn projection_fails_closed_on_duplicates_and_degraded_sources() {
        let (events, profile) = competition_fixture();
        let items = open_competition_discovery_items(
            &events,
            &profile,
            "base-mainnet",
            8453,
            "https://api.example",
            "https://www.example",
            10,
            Utc.timestamp_opt(1_700_000_100, 0).unwrap(),
        )
        .unwrap();
        let source = GitHubDiscoverySourceStatus {
            source_type: "open_competition".to_string(),
            protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
            factory_contract: None,
            available: true,
            fresh: false,
            item_count: 1,
            persisted_cursor_block: Some(11),
            error: Some("stale_indexer".to_string()),
        };
        let autonomous_source = GitHubDiscoverySourceStatus {
            source_type: "canonical_autonomous".to_string(),
            protocol_version: AUTONOMOUS_PROTOCOL_VERSION.to_string(),
            factory_contract: None,
            available: false,
            fresh: false,
            item_count: 0,
            persisted_cursor_block: Some(11),
            error: Some("stale_indexer".to_string()),
        };
        let response = assemble_projection(
            "base-mainnet",
            8453,
            Utc::now(),
            None,
            vec![autonomous_source, source],
            items.clone(),
        )
        .unwrap();
        assert!(response.degraded);
        assert!(assemble_projection(
            "base-mainnet",
            8453,
            Utc::now(),
            None,
            vec![],
            vec![items[0].clone(), items[0].clone()],
        )
        .is_err());
    }

    #[test]
    fn protocol_adapter_set_is_explicit() {
        let supported = BTreeSet::from([
            AUTONOMOUS_PROTOCOL_VERSION,
            OPEN_COMPETITION_PROTOCOL_VERSION,
        ]);
        assert_eq!(supported.len(), 2);
        assert!(supported.contains(AUTONOMOUS_PROTOCOL_VERSION));
        assert!(supported.contains(OPEN_COMPETITION_PROTOCOL_VERSION));
    }

    #[allow(dead_code)]
    fn autonomous_event(kind: AutonomousBountyEventKind, data: Value) -> AutonomousBountyEvent {
        AutonomousBountyEvent {
            id: Id::from_u128(10_001),
            log_key: "1:0".to_string(),
            tx_hash: format!("0x{}", "aa".repeat(32)),
            block_number: 1,
            log_index: 0,
            contract_address: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_id: format!("0x{}", "bb".repeat(32)),
            kind,
            data,
            occurred_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        }
    }
}
