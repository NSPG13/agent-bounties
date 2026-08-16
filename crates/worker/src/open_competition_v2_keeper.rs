use anyhow::Context;
use chain_base::{
    fetch_contract_bool_at, fetch_safe_block_identity, plan_open_competition_v2_action,
    OpenCompetitionV2EventKind, OpenCompetitionV2ProjectedState,
};
use db::{OpenCompetitionV2StoredProjection, PostgresStore};
use serde::Serialize;

use crate::OpenCompetitionV2BrokerChainConfig;

pub const OPEN_COMPETITION_V2_KEEPER_NETWORK_ENV: &str = "OPEN_COMPETITION_V2_KEEPER_NETWORK";
pub const OPEN_COMPETITION_V2_KEEPER_FACTORY_ENV: &str = "OPEN_COMPETITION_V2_KEEPER_FACTORY";

#[derive(Debug, Clone)]
pub struct OpenCompetitionV2KeeperConfig {
    pub network: String,
    pub factory_contract: String,
}

impl OpenCompetitionV2KeeperConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let network = required_env(OPEN_COMPETITION_V2_KEEPER_NETWORK_ENV)?;
        let factory_contract = required_env(OPEN_COMPETITION_V2_KEEPER_FACTORY_ENV)?;
        Ok(Self {
            network,
            factory_contract,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KeeperAction {
    competition: String,
    action: &'static str,
    contributor: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OpenCompetitionV2KeeperReport {
    pub network: String,
    pub safe_block_number: u64,
    pub action: String,
    pub competition_contract: Option<String>,
    pub contributor: Option<String>,
    pub transaction_hash: Option<String>,
    pub evidence_boundary: String,
}

pub async fn poll_open_competition_v2_keeper_once(
    store: &PostgresStore,
    config: &OpenCompetitionV2KeeperConfig,
    chain: &OpenCompetitionV2BrokerChainConfig,
) -> anyhow::Result<OpenCompetitionV2KeeperReport> {
    let (_, rpc_url, _) = chain.rpc(&config.network)?;
    let safe = fetch_safe_block_identity(&rpc_url, 91).await?;
    let projections = store
        .list_open_competition_v2_projections(&config.network, &config.factory_contract)
        .await?;
    let events = store
        .list_open_competition_v2_events(&config.network, &config.factory_contract)
        .await?;
    let action = choose_keeper_action(&projections, &events, safe.timestamp);
    let action = match action {
        Some(action) => Some(action),
        None => find_unavailable_verifier_action(&projections, &rpc_url, safe.number).await?,
    };
    let Some(action) = action else {
        return Ok(OpenCompetitionV2KeeperReport {
            network: config.network.clone(),
            safe_block_number: safe.number,
            action: "idle".to_string(),
            competition_contract: None,
            contributor: None,
            transaction_hash: None,
            evidence_boundary: evidence_boundary().to_string(),
        });
    };
    let plan = plan_open_competition_v2_action(
        &config.network,
        &action.competition,
        Some(&chain.relayer.address()),
        action.action,
        action.contributor.as_deref(),
    )?;
    let transaction = chain
        .relayer
        .simulate_and_broadcast(
            &rpc_url,
            chain_base::base_network_descriptor(&config.network)?.chain_id,
            &plan.wallet_call,
            chain.max_gas,
            chain.max_fee_per_gas_wei,
        )
        .await?;
    Ok(OpenCompetitionV2KeeperReport {
        network: config.network.clone(),
        safe_block_number: safe.number,
        action: action.action.to_string(),
        competition_contract: Some(action.competition),
        contributor: action.contributor,
        transaction_hash: Some(transaction.tx_hash),
        evidence_boundary: evidence_boundary().to_string(),
    })
}

fn choose_keeper_action(
    projections: &[OpenCompetitionV2StoredProjection],
    events: &[chain_base::OpenCompetitionV2Event],
    safe_timestamp: u64,
) -> Option<KeeperAction> {
    let mut ordered = projections.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|stored| {
        (
            state_priority(stored.projection.state),
            stored.projection.proof_deadline.unwrap_or(u64::MAX),
            stored.projection.funding_deadline.unwrap_or(u64::MAX),
            stored.projection.bounty_id.clone(),
        )
    });
    for stored in ordered {
        let projection = &stored.projection;
        match projection.state {
            OpenCompetitionV2ProjectedState::Funding
                if projection
                    .funding_deadline
                    .is_some_and(|deadline| safe_timestamp > deadline) =>
            {
                return Some(KeeperAction {
                    competition: projection.competition.clone(),
                    action: "cancel_funding",
                    contributor: None,
                });
            }
            OpenCompetitionV2ProjectedState::Active
                if projection
                    .proof_deadline
                    .is_some_and(|deadline| safe_timestamp > deadline) =>
            {
                let action = if projection.winner_mode.as_deref() == Some("best_score")
                    && projection.leader.is_some()
                {
                    "finalize_best_score"
                } else {
                    "expire_competition"
                };
                return Some(KeeperAction {
                    competition: projection.competition.clone(),
                    action,
                    contributor: None,
                });
            }
            OpenCompetitionV2ProjectedState::Cancelled if projection.refund_pool_remaining > 0 => {
                if let Some(contributor) = next_refund_contributor(&projection.bounty_id, events) {
                    return Some(KeeperAction {
                        competition: projection.competition.clone(),
                        action: "withdraw_refund_for",
                        contributor: Some(contributor),
                    });
                }
            }
            _ => {}
        }
    }
    None
}

async fn find_unavailable_verifier_action(
    projections: &[OpenCompetitionV2StoredProjection],
    rpc_url: &str,
    safe_block: u64,
) -> anyhow::Result<Option<KeeperAction>> {
    for stored in projections {
        let projection = &stored.projection;
        if !matches!(
            projection.state,
            OpenCompetitionV2ProjectedState::Funding | OpenCompetitionV2ProjectedState::Active
        ) {
            continue;
        }
        let Some(adapter) = projection.verifier_adapter.as_deref() else {
            continue;
        };
        if !fetch_contract_bool_at(rpc_url, adapter, "0x5d7a55da", safe_block, 92).await? {
            return Ok(Some(KeeperAction {
                competition: projection.competition.clone(),
                action: "cancel_unavailable_verifier",
                contributor: None,
            }));
        }
    }
    Ok(None)
}

fn next_refund_contributor(
    bounty_id: &str,
    events: &[chain_base::OpenCompetitionV2Event],
) -> Option<String> {
    let mut contributors = events
        .iter()
        .filter(|event| {
            event.bounty_id.eq_ignore_ascii_case(bounty_id)
                && event.kind == OpenCompetitionV2EventKind::FundingAdded
        })
        .filter_map(|event| {
            event
                .data
                .get("contributor")
                .and_then(|value| value.as_str())
        })
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    contributors.sort();
    contributors.dedup();
    let withdrawn = events
        .iter()
        .filter(|event| {
            event.bounty_id.eq_ignore_ascii_case(bounty_id)
                && event.kind == OpenCompetitionV2EventKind::RefundWithdrawn
        })
        .filter_map(|event| {
            event
                .data
                .get("contributor")
                .and_then(|value| value.as_str())
        })
        .map(str::to_ascii_lowercase)
        .collect::<std::collections::HashSet<_>>();
    contributors
        .into_iter()
        .find(|contributor| !withdrawn.contains(contributor))
}

fn state_priority(state: OpenCompetitionV2ProjectedState) -> u8 {
    match state {
        OpenCompetitionV2ProjectedState::Cancelled => 0,
        OpenCompetitionV2ProjectedState::Active => 1,
        OpenCompetitionV2ProjectedState::Funding => 2,
        OpenCompetitionV2ProjectedState::Settled => 3,
        OpenCompetitionV2ProjectedState::Announced => 4,
    }
}

fn required_env(key: &str) -> anyhow::Result<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{key} is required"))
}

fn evidence_boundary() -> &'static str {
    "A keeper broadcast is not settlement or refund evidence. Reconcile the safe canonical V2 event before changing hosted state."
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::{OpenCompetitionV2Event, OpenCompetitionV2Projection};
    use chrono::Utc;
    use uuid::Uuid;

    fn stored(state: OpenCompetitionV2ProjectedState) -> OpenCompetitionV2StoredProjection {
        OpenCompetitionV2StoredProjection {
            network: "base-sepolia".to_string(),
            factory_contract: format!("0x{}", "01".repeat(20)),
            projection: OpenCompetitionV2Projection {
                bounty_id: format!("0x{}", "02".repeat(32)),
                competition: format!("0x{}", "03".repeat(20)),
                state,
                ..Default::default()
            },
            safe_block_number: 10,
            safe_block_hash: format!("0x{}", "04".repeat(32)),
        }
    }

    fn event(kind: OpenCompetitionV2EventKind, contributor: &str) -> OpenCompetitionV2Event {
        OpenCompetitionV2Event {
            id: Uuid::new_v4(),
            protocol_version: "agent-bounties/open-competition-v2-beta3".to_string(),
            log_key: Uuid::new_v4().to_string(),
            tx_hash: format!("0x{}", "05".repeat(32)),
            block_number: 9,
            log_index: 0,
            contract_address: format!("0x{}", "03".repeat(20)),
            bounty_id: format!("0x{}", "02".repeat(32)),
            kind,
            data: serde_json::json!({"contributor": contributor}),
            occurred_at: Utc::now(),
        }
    }

    #[test]
    fn keeper_selects_expiry_and_best_score_finalization() {
        let mut funding = stored(OpenCompetitionV2ProjectedState::Funding);
        funding.projection.funding_deadline = Some(99);
        assert_eq!(
            choose_keeper_action(&[funding], &[], 100).unwrap().action,
            "cancel_funding"
        );

        let mut active = stored(OpenCompetitionV2ProjectedState::Active);
        active.projection.proof_deadline = Some(99);
        active.projection.winner_mode = Some("best_score".to_string());
        active.projection.leader = Some(format!("0x{}", "06".repeat(20)));
        assert_eq!(
            choose_keeper_action(&[active], &[], 100).unwrap().action,
            "finalize_best_score"
        );
    }

    #[test]
    fn refund_queue_skips_withdrawn_contributors() {
        let mut cancelled = stored(OpenCompetitionV2ProjectedState::Cancelled);
        cancelled.projection.refund_pool_remaining = 100;
        let first = format!("0x{}", "11".repeat(20));
        let second = format!("0x{}", "22".repeat(20));
        let events = vec![
            event(OpenCompetitionV2EventKind::FundingAdded, &first),
            event(OpenCompetitionV2EventKind::FundingAdded, &second),
            event(OpenCompetitionV2EventKind::RefundWithdrawn, &first),
        ];
        assert_eq!(
            choose_keeper_action(&[cancelled], &events, 100)
                .unwrap()
                .contributor,
            Some(second)
        );
    }
}
