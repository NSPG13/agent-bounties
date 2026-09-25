//! Read-only continuation records from the account's confirmed event endpoints.
//! Dates may require attention; only terminal events establish an outcome.
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn hex(value: &str, bytes: usize) -> bool {
    value.len() == 2 + bytes * 2
        && value.starts_with("0x")
        && value[2..].bytes().all(|b| b.is_ascii_hexdigit())
}
fn subject_contract(event: &Value) -> &str {
    if matches!(
        text(event, "kind"),
        "canonical_bounty_created" | "canonical_competition_created"
    ) {
        let data = &event["data"];
        let address = text(data, "bounty_contract");
        if address.is_empty() {
            text(data, "competition")
        } else {
            address
        }
    } else {
        text(event, "contract_address")
    }
}
fn integer(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn position(event: &Value) -> (u64, u64) {
    (
        integer(&event["block_number"]).unwrap_or(0),
        integer(&event["log_index"]).unwrap_or(0),
    )
}
fn owned(data: &Value, field: &str, wallets: &BTreeSet<String>) -> bool {
    wallets.contains(&text(data, field).to_ascii_lowercase())
}
fn terminal(kind: &str) -> bool {
    matches!(
        kind,
        "bounty_settled"
            | "competition_settled"
            | "submission_expired"
            | "submission_rejected"
            | "claim_expired"
    )
}

pub fn inbox(evidence: &Value, wallets: &BTreeSet<String>, now: DateTime<Utc>) -> Value {
    let mut items = Vec::new();
    for source in ["autonomous", "competition_v1", "competition_v2"] {
        let stream = &evidence[source];
        let Some(events) = stream.as_array().or_else(|| stream["events"].as_array()) else {
            continue;
        };
        // These endpoints return persisted confirmed events. Reject explicit
        // unconfirmed data, malformed identities and conflicting duplicate logs.
        let mut logs = BTreeMap::new();
        for event in events {
            if event["confirmed"] == false
                || !hex(subject_contract(event), 20)
                || !hex(text(event, "bounty_id"), 32)
                || !hex(text(event, "tx_hash"), 32)
                || position(event).0 == 0
            {
                continue;
            }
            let key = (
                text(event, "tx_hash").to_ascii_lowercase(),
                position(event).1,
            );
            if logs.get(&key).is_some_and(|previous| *previous != event) {
                return json!({"status":"unavailable", "reason":"conflicting_canonical_events", "items":[]});
            }
            logs.insert(key, event);
        }
        let mut contracts: BTreeMap<(String, String), Vec<&Value>> = BTreeMap::new();
        for event in logs.into_values() {
            contracts
                .entry((
                    subject_contract(event).to_ascii_lowercase(),
                    text(event, "bounty_id").to_ascii_lowercase(),
                ))
                .or_default()
                .push(event);
        }
        for ((contract, bounty), mut events) in contracts {
            events.sort_by_key(|event| position(event));
            let creator = events.iter().any(|event| {
                matches!(
                    text(event, "kind"),
                    "canonical_bounty_created" | "canonical_competition_created"
                ) && owned(&event["data"], "creator", wallets)
            });
            let contributors = events
                .iter()
                .filter(|event| {
                    text(event, "kind") == "funding_added"
                        && owned(&event["data"], "contributor", wallets)
                })
                .map(|event| text(&event["data"], "contributor").to_ascii_lowercase())
                .collect::<BTreeSet<_>>();
            let funding_confirmed = events.iter().any(|event| {
                matches!(
                    text(event, "kind"),
                    "bounty_became_claimable"
                        | "bounty_claimed"
                        | "submission_added"
                        | "bounty_settled"
                        | "competition_settled"
                        | "entry_qualified"
                )
            });
            let page = if source == "autonomous" {
                "participate"
            } else {
                "competition"
            };
            let url = format!("https://agentbounties.app/{page}.html?bountyContract={contract}&network=base-mainnet");
            let title = format!("Bounty {}…{}", &contract[..8], &contract[38..]);
            let mut rounds: BTreeMap<(u64, String), Vec<&Value>> = BTreeMap::new();
            for event in &events {
                let kind = text(event, "kind");
                let data = &event["data"];
                if !matches!(
                    kind,
                    "bounty_claimed"
                        | "submission_added"
                        | "solution_committed"
                        | "solution_revealed"
                        | "entry_qualified"
                ) && !terminal(kind)
                {
                    continue;
                }
                let solver = text(data, "solver").to_ascii_lowercase();
                if !hex(&solver, 20) || (!creator && !wallets.contains(&solver)) {
                    continue;
                }
                let round = integer(&data["round"]).unwrap_or(0);
                if source == "autonomous" && round == 0 {
                    continue;
                }
                rounds.entry((round, solver)).or_default().push(event);
            }
            let latest_state = events.iter().rev().find(|event| {
                matches!(
                    text(event, "kind"),
                    "canonical_bounty_created"
                        | "canonical_competition_created"
                        | "bounty_became_claimable"
                        | "bounty_claimed"
                        | "submission_added"
                        | "bounty_cancelled"
                ) || terminal(text(event, "kind"))
            });
            if creator
                && latest_state.is_some_and(|event| {
                    matches!(
                        text(event, "kind"),
                        "canonical_bounty_created"
                            | "canonical_competition_created"
                            | "bounty_became_claimable"
                            | "submission_expired"
                            | "submission_rejected"
                            | "claim_expired"
                    )
                })
            {
                items.push(json!({"id":format!("posted:{source}:{contract}"), "title":title, "group":"working", "status":"Posted bounty · check next step", "next_actor":"poster or solver", "next_action":"Check current funding and solver availability for this posted bounty.", "continuation_url":url, "updated_at":latest_state.unwrap()["occurred_at"], "payment_state":"unpaid", "source":source, "bounty_contract":contract, "bounty_id":bounty, "network":"base-mainnet", "account_role":"poster", "funding_confirmed":funding_confirmed}));
            }
            for ((round, solver), mut history) in rounds {
                let other_winner = if source != "autonomous" {
                    events
                        .iter()
                        .find(|event| {
                            matches!(
                                text(event, "kind"),
                                "bounty_settled" | "competition_settled"
                            ) && text(&event["data"], "solver").to_ascii_lowercase() != solver
                        })
                        .copied()
                } else {
                    None
                };
                if let Some(settlement) = other_winner {
                    history.push(settlement);
                }
                let latest = history.last().unwrap();
                let kind = text(latest, "kind");
                let data = &latest["data"];
                let terminal_count = history
                    .iter()
                    .filter(|event| terminal(text(event, "kind")))
                    .count();
                let deadline = if kind == "bounty_claimed" {
                    integer(&data["claim_expires_at"])
                } else if kind == "submission_added" {
                    integer(&data["verification_expires_at"])
                } else {
                    None
                };
                let overdue =
                    deadline.is_some_and(|deadline| deadline <= now.timestamp().max(0) as u64);
                let (group, status, actor, next_action) = if terminal_count > 1 {
                    ("needs_action", "Result needs review", "operator", "Reconcile conflicting round outcomes before retrying or reporting payment.")
                } else if other_winner.is_some() {
                    ("completed", "Competition settled · another entry won", "none", "View the confirmed competition result. This entry received no solver reward.")
                } else {
                    match kind {
                    "bounty_settled" | "competition_settled" => ("paid", "Paid", "none", "View the confirmed settlement."),
                    "submission_expired" => ("completed", "Review expired · bond returned", "none", "View the confirmed bond return. This round earned no solver reward; the bounty reopened."),
                    "submission_rejected" => ("completed", "Did not pass", "none", "View the confirmed rejection. The bond covers review and the bounty reopened."),
                    "claim_expired" => ("completed", "Claim expired · bond forfeited", "none", "View the confirmed claim timeout. The bounty reopened."),
                    "submission_added" if overdue => ("needs_action", "Review window elapsed", "any caller", "Check the current round and available review-timeout recovery. A passed deadline alone does not prove a bond return."),
                    "submission_added" => ("awaiting_review", "Awaiting review", "verifier", "Check the committed review and its deadline."),
                    "bounty_claimed" if overdue => ("needs_action", "Work window elapsed", "any caller", "Check the current round and claim-timeout recovery before taking another action."),
                    "bounty_claimed" => ("working", "Work in progress", "solver", "Continue the claimed task and submit before the deadline."),
                    "solution_committed" => ("needs_action", "Entry committed", "solver", "Check the reveal window and continue this entry."),
                    _ => ("awaiting_review", "Entry awaiting settlement", "verifier", "Check this competition's proof, review and settlement state."),
                }
                };
                let timeline = history.iter().map(|event| json!({
                    "event": text(event, "kind"), "occurred_at": event["occurred_at"],
                    "transaction_url": format!("https://basescan.org/tx/{}", text(event, "tx_hash")),
                })).collect::<Vec<_>>();
                items.push(json!({
                    "id":format!("{source}:{contract}:{round}:{solver}"), "title":title,
                    "group":group, "status":status, "round":round, "solver":solver,
                    "bounty_contract":contract, "bounty_id":bounty, "network":"base-mainnet",
                    "next_actor":actor, "next_action":next_action, "continuation_url":url,
                    "updated_at":latest["occurred_at"], "deadline":deadline, "timeline":timeline,
                    "payment_state":if group == "paid" {"paid"} else {"unpaid"},
                    "source":source, "account_role":if creator {"poster"} else {"solver"}, "funding_confirmed":funding_confirmed,
                }));
            }
            if events
                .iter()
                .any(|event| text(event, "kind") == "bounty_cancelled")
            {
                for contributor in contributors {
                    let refunded = events.iter().any(|event| {
                        text(event, "kind") == "refund_withdrawn"
                            && text(&event["data"], "contributor")
                                .eq_ignore_ascii_case(&contributor)
                    });
                    if !refunded {
                        items.push(json!({"id":format!("refund:{source}:{contract}:{contributor}"), "title":title, "group":"recover_funds", "status":"Check contribution refund", "next_actor":"contributor", "next_action":"Check your remaining refundable contribution. Cancellation is not a completed refund.", "continuation_url":url, "updated_at":events.last().unwrap()["occurred_at"], "payment_state":"unverified", "source":source}));
                    }
                }
            }
        }
    }
    items.sort_by(|a, b| {
        text(b, "updated_at")
            .cmp(text(a, "updated_at"))
            .then_with(|| text(a, "id").cmp(text(b, "id")))
    });
    json!({"schema_version":"agent-bounties/account-activity-v1", "status":"available", "items":items, "generated_at":now,
        "evidence_boundary":"Round outcomes come from confirmed canonical event endpoints. A passed deadline is an action reminder. Only the matching settlement event proves solver payment; drafts, review and cancellation do not."})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: &str, block: u64, round: u64) -> Value {
        json!({"kind":kind,"block_number":block,"log_index":0,"tx_hash":format!("0x{block:064x}"),"contract_address":format!("0x{}","11".repeat(20)),"bounty_id":format!("0x{}","ab".repeat(32)),"occurred_at":"2026-09-25T10:00:00Z","data":{"round":round,"solver":format!("0x{}","22".repeat(20)),"claim_expires_at":2000000000,"verification_expires_at":1000000000}})
    }
    fn read(events: Vec<Value>) -> Value {
        inbox(
            &json!({"autonomous":events}),
            &BTreeSet::from([format!("0x{}", "22".repeat(20))]),
            DateTime::parse_from_rfc3339("2026-09-25T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }
    #[test]
    fn review_expiry_is_an_unpaid_past_round_and_does_not_change_the_next_claim() {
        let submitted = event("submission_added", 2, 1);
        let overdue = read(vec![event("bounty_claimed", 1, 1), submitted.clone()]);
        assert_eq!(overdue["items"][0]["group"], "needs_action");
        assert_eq!(overdue["items"][0]["payment_state"], "unpaid");
        let expired = event("submission_expired", 3, 1);
        let mut unconfirmed = event("bounty_settled", 5, 1);
        unconfirmed["confirmed"] = json!(false);
        let result = read(vec![
            event("bounty_claimed", 1, 1),
            submitted,
            expired.clone(),
            expired,
            event("bounty_claimed", 4, 2),
            unconfirmed,
        ]);
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
        let first = result["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["round"] == 1)
            .unwrap();
        assert_eq!(first["status"], "Review expired · bond returned");
        assert_eq!(first["payment_state"], "unpaid");
        assert_eq!(first["timeline"].as_array().unwrap().len(), 3);
        let next = result["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["round"] == 2)
            .unwrap();
        assert_eq!(next["group"], "working");
    }
    #[test]
    fn contradictory_terminal_events_fail_closed_and_foreign_wallets_stay_hidden() {
        let result = read(vec![
            event("submission_expired", 3, 1),
            event("bounty_settled", 4, 1),
        ]);
        assert_eq!(result["items"][0]["status"], "Result needs review");
        assert_ne!(result["items"][0]["payment_state"], "paid");
        let first = event("submission_expired", 3, 1);
        let mut changed = first.clone();
        changed["kind"] = json!("bounty_settled");
        assert_eq!(read(vec![first, changed])["status"], "unavailable");
        let mut foreign = event("bounty_claimed", 1, 1);
        foreign["data"]["solver"] = json!(format!("0x{}", "33".repeat(20)));
        assert!(read(vec![foreign])["items"].as_array().unwrap().is_empty());
    }
    #[test]
    fn creator_factory_events_link_to_the_bounty_and_each_funders_recovery_is_independent() {
        let wallet = format!("0x{}", "22".repeat(20));
        let mut created = event("canonical_bounty_created", 1, 0);
        created["data"]["creator"] = json!(wallet);
        created["data"]["bounty_contract"] = created["contract_address"].clone();
        created["contract_address"] = json!(format!("0x{}", "ff".repeat(20)));
        let result = read(vec![created.clone()]);
        assert!(result["items"][0]["continuation_url"]
            .as_str()
            .unwrap()
            .contains(&"11".repeat(20)));
        let mut funded = event("funding_added", 2, 0);
        funded["data"]["contributor"] = json!(wallet);
        let cancelled = event("bounty_cancelled", 3, 0);
        let recovery = read(vec![created.clone(), funded.clone(), cancelled.clone()]);
        assert_eq!(recovery["items"][0]["group"], "recover_funds");
        let mut withdrawn = event("refund_withdrawn", 4, 0);
        withdrawn["data"]["contributor"] = json!(wallet);
        assert!(read(vec![created, funded, cancelled, withdrawn])["items"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn a_losing_competition_entry_is_completed_without_being_marked_paid() {
        let entry = event("entry_qualified", 1, 0);
        let mut settled = event("competition_settled", 2, 0);
        settled["data"]["solver"] = json!(format!("0x{}", "33".repeat(20)));
        let result = inbox(
            &json!({"competition_v2":[entry,settled]}),
            &BTreeSet::from([format!("0x{}", "22".repeat(20))]),
            Utc::now(),
        );
        assert_eq!(result["items"][0]["group"], "completed");
        assert_eq!(result["items"][0]["payment_state"], "unpaid");
    }
}
