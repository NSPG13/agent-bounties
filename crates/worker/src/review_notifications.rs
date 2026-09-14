//! Transactional review requests derived from confirmed canonical submissions.
//! This projection never accepts an email address, artifact, or recipient from a solver.

use crate::{ReviewEmailConfig, ReviewEmailOutcome, ReviewEmailSender};
use chain_base::{
    build_autonomous_bounty_feed, AutonomousBountyEvent, AutonomousBountyEventKind,
    AutonomousBountyFeedItem,
};
use chrono::{DateTime, Utc};
use db::BaseIndexerHeartbeat;
use db::{
    NewReviewNotification, PostgresStore, ReviewNotificationOutcome, ReviewNotificationRound,
};
use domain::AutonomousBountyTermsRecord;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const NETWORK: &str = "base-mainnet";

pub struct VerifierEmailRuntime {
    sender: ReviewEmailSender,
    factory: String,
}

impl VerifierEmailRuntime {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(config) = ReviewEmailConfig::from_env()? else {
            return Ok(None);
        };
        let factory = std::env::var("BASE_MAINNET_BOUNTY_FACTORY")
            .ok()
            .and_then(|value| wallet(&value))
            .ok_or_else(|| {
                anyhow::anyhow!("VERIFIER_EMAIL_ENABLED requires BASE_MAINNET_BOUNTY_FACTORY")
            })?;
        Ok(Some(Self {
            sender: ReviewEmailSender::new(config)?,
            factory,
        }))
    }

    /// Runs independently of HTTP submissions so direct contract submissions,
    /// process restarts, and late verified contact linking are also covered.
    pub async fn run(self, store: PostgresStore) {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(30));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            match self.poll_once(&store).await {
                Ok(report) => println!(
                    "{}",
                    serde_json::json!({"event":"verifier_review_notifications", "report":report})
                ),
                // DB/transport errors can include private values. Log only a
                // fixed failure class; the durable outbox retains retry state.
                Err(_) => eprintln!(
                    "{}",
                    serde_json::json!({"event":"verifier_review_notifications", "status":"failed", "code":"notification_poll_failed"})
                ),
            }
        }
    }

    async fn poll_once(&self, store: &PostgresStore) -> anyhow::Result<ReviewDeliveryReport> {
        let mut report = ReviewDeliveryReport::default();
        let now = Utc::now();
        let heartbeat = store
            .get_base_indexer_heartbeat(NETWORK, &self.factory)
            .await?;
        if !heartbeat
            .as_ref()
            .is_some_and(|heartbeat| review_indexer_is_fresh(heartbeat, now))
        {
            report.blocked_reason = Some("indexer_not_fresh");
            return Ok(report);
        }
        let events = store
            .list_verified_autonomous_bounty_events(NETWORK)
            .await?;
        let terms = store.list_autonomous_bounty_terms().await?;
        let (pending, retained, projection) =
            project_verifier_reviews(&self.factory, events, terms, now);
        let jobs = pending
            .into_iter()
            .map(|review| {
                Ok(NewReviewNotification {
                    network: NETWORK.to_string(),
                    bounty_contract: review.bounty_contract,
                    bounty_id: review.bounty_id,
                    round: i64::try_from(review.round)?,
                    verifier_wallet: review.verifier_wallet,
                    submission_log_key: review.submission_log_key,
                    submitted_at: review.submitted_at,
                    deadline: Some(review.review_deadline),
                })
            })
            .collect::<Result<Vec<_>, std::num::TryFromIntError>>()?;
        let retained = retained
            .into_iter()
            .map(|(bounty_contract, round)| {
                Ok(ReviewNotificationRound {
                    bounty_contract,
                    round: i64::try_from(round)?,
                })
            })
            .collect::<Result<Vec<_>, std::num::TryFromIntError>>()?;
        report.outbox = store
            .reconcile_review_notifications(NETWORK, &jobs, &retained)
            .await?;
        report.projection = projection;
        let started = std::time::Instant::now();
        for _ in 0..10 {
            if started.elapsed().as_secs() >= 20 {
                break;
            }
            let heartbeat = store
                .get_base_indexer_heartbeat(NETWORK, &self.factory)
                .await?;
            if !heartbeat
                .as_ref()
                .is_some_and(|heartbeat| review_indexer_is_fresh(heartbeat, Utc::now()))
            {
                report.blocked_reason = Some("indexer_not_fresh");
                break;
            }
            let Some(lease) = store
                .claim_review_notifications(NETWORK, 1, 60)
                .await?
                .into_iter()
                .next()
            else {
                break;
            };
            let payload = match lease.provider_payload.clone().map(Ok).unwrap_or_else(|| {
                self.sender.prepare_payload(
                    &lease.recipient_email,
                    &lease.notification.bounty_contract,
                    lease.notification.deadline,
                )
            }) {
                Ok(payload) => payload,
                Err(_) => {
                    store
                        .finish_review_notification_attempt(
                            lease.id,
                            lease.lease_token,
                            ReviewNotificationOutcome::Failed {
                                code: "invalid_email_payload".into(),
                            },
                        )
                        .await?;
                    report.failed += 1;
                    continue;
                }
            };
            let Some(attempt) = store
                .begin_review_notification_attempt(&lease, &payload)
                .await?
            else {
                continue;
            };
            let outcome = match self
                .sender
                .send(&attempt.idempotency_key, &attempt.provider_payload)
                .await
            {
                ReviewEmailOutcome::Accepted { provider_id } => {
                    report.provider_accepted += 1;
                    ReviewNotificationOutcome::Accepted { provider_id }
                }
                ReviewEmailOutcome::Retryable {
                    code,
                    retry_after_seconds,
                } => {
                    report.retry_pending += 1;
                    ReviewNotificationOutcome::Retry {
                        code: code.as_str().into(),
                        retry_after_seconds: retry_after_seconds.unwrap_or(60).clamp(30, 3600)
                            as u32,
                    }
                }
                ReviewEmailOutcome::PermanentFailure { code } => {
                    report.failed += 1;
                    ReviewNotificationOutcome::Failed {
                        code: code.as_str().into(),
                    }
                }
            };
            store
                .finish_review_notification_attempt(attempt.id, attempt.lease_token, outcome)
                .await?;
        }
        Ok(report)
    }
}

#[derive(Debug, Default, Serialize)]
struct ReviewDeliveryReport {
    projection: ReviewProjectionReport,
    outbox: db::ReviewNotificationSync,
    provider_accepted: u64,
    retry_pending: u64,
    failed: u64,
    blocked_reason: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingVerifierReview {
    pub bounty_contract: String,
    pub bounty_id: String,
    pub round: u64,
    pub verifier_wallet: String,
    pub submission_log_key: String,
    pub submitted_at: DateTime<Utc>,
    pub review_deadline: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize)]
pub struct ReviewProjectionReport {
    pub active_reviews: usize,
    pub invalid_bounties: usize,
    pub unavailable_terms: usize,
}

/// Isolate malformed bounties, while requiring the configured factory and child
/// emitter for every record used to derive a recipient. Caller supplies only
/// events whose block timestamps have been verified by the confirmed indexer.
pub fn project_verifier_reviews(
    factory: &str,
    events: Vec<AutonomousBountyEvent>,
    terms: Vec<AutonomousBountyTermsRecord>,
    now: DateTime<Utc>,
) -> (
    Vec<PendingVerifierReview>,
    Vec<(String, u64)>,
    ReviewProjectionReport,
) {
    let mut groups: BTreeMap<String, Vec<AutonomousBountyEvent>> = BTreeMap::new();
    let mut report = ReviewProjectionReport::default();
    for event in events {
        groups
            .entry(event.bounty_id.to_ascii_lowercase())
            .or_default()
            .push(event);
    }
    let mut reviews = Vec::new();
    let mut retained_rounds = Vec::new();
    for group in groups.into_values() {
        let created: Vec<_> = group
            .iter()
            .filter(|event| {
                event.kind == AutonomousBountyEventKind::CanonicalBountyCreated
                    && event.contract_address.eq_ignore_ascii_case(factory)
            })
            .collect();
        if created.len() != 1 {
            report.invalid_bounties += 1;
            continue;
        }
        let Some(contract) = created[0].data["bounty_contract"].as_str().and_then(wallet) else {
            report.invalid_bounties += 1;
            continue;
        };
        let emitters_valid = group.iter().all(|event| {
            let expected = match event.kind {
                AutonomousBountyEventKind::CanonicalBountyCreated
                | AutonomousBountyEventKind::CanonicalBountyTermsCommitted
                | AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured
                | AutonomousBountyEventKind::CanonicalBountyVerificationConfigured
                | AutonomousBountyEventKind::ExternalBountySubmitted => factory,
                _ => &contract,
            };
            event.contract_address.eq_ignore_ascii_case(expected)
        });
        if !emitters_valid {
            report.invalid_bounties += 1;
            continue;
        }
        let current = group
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    AutonomousBountyEventKind::SubmissionAdded
                        | AutonomousBountyEventKind::BountyClaimed
                        | AutonomousBountyEventKind::SubmissionExpired
                        | AutonomousBountyEventKind::SubmissionRejected
                        | AutonomousBountyEventKind::ClaimExpired
                        | AutonomousBountyEventKind::BountySettled
                        | AutonomousBountyEventKind::BountyCancelled
                )
            })
            .max_by_key(|event| (event.block_number, event.log_index));
        let retained = current
            .filter(|event| event.kind == AutonomousBountyEventKind::SubmissionAdded)
            .and_then(|event| event.data["round"].as_u64().filter(|round| *round > 0))
            .map(|round| (contract.clone(), round));
        let relevant_terms: Vec<_> = group
            .iter()
            .filter_map(|event| event.data["terms_hash"].as_str())
            .flat_map(|hash| {
                terms
                    .iter()
                    .filter(move |term| term.terms_hash.eq_ignore_ascii_case(hash))
            })
            .cloned()
            .collect();
        let Ok(feed) = build_autonomous_bounty_feed(group, relevant_terms, false) else {
            report.invalid_bounties += 1;
            retained_rounds.extend(retained);
            continue;
        };
        for item in feed {
            if !item.terms_valid || item.terms.is_none() {
                report.unavailable_terms += 1;
                retained_rounds.extend(retained.clone());
                continue;
            }
            match review_recipients(&item, now) {
                Ok(mut pending) => reviews.append(&mut pending),
                Err(()) => {
                    report.invalid_bounties += 1;
                    retained_rounds.extend(retained.clone());
                }
            }
        }
    }
    reviews.sort_by(|a, b| {
        (
            a.review_deadline,
            &a.bounty_contract,
            a.round,
            &a.verifier_wallet,
        )
            .cmp(&(
                b.review_deadline,
                &b.bounty_contract,
                b.round,
                &b.verifier_wallet,
            ))
    });
    report.active_reviews = reviews.len();
    (reviews, retained_rounds, report)
}

fn review_recipients(
    item: &AutonomousBountyFeedItem,
    now: DateTime<Utc>,
) -> Result<Vec<PendingVerifierReview>, ()> {
    // A runner readiness failure is a reason to alert its designated people,
    // not a reason to hide work from them. Module-only policies have no inbox.
    if item.status != "submitted"
        || !item.terms_valid
        || !matches!(
            item.verification_mode.as_str(),
            "signed_quorum" | "ai_judge_quorum"
        )
    {
        return Ok(Vec::new());
    }
    let terms = item.terms.as_ref().ok_or(())?;
    let policy = &terms.document.verification_policy;
    if policy["mechanism"].as_str() != Some(item.verification_mode.as_str()) {
        return Err(());
    }
    let Some(recipients) = policy["verifiers"].as_array() else {
        return Err(());
    };
    if recipients.len() > 64 {
        return Err(());
    }
    let mut wallets = BTreeSet::new();
    for recipient in recipients {
        wallets.insert(recipient.as_str().and_then(wallet).ok_or(())?);
    }
    if wallets.is_empty() {
        return Ok(Vec::new());
    }
    let event = item
        .events
        .iter()
        .filter(|event| event.kind == AutonomousBountyEventKind::SubmissionAdded)
        .max_by_key(|event| (event.block_number, event.log_index))
        .ok_or(())?;
    let round = event.data["round"]
        .as_u64()
        .filter(|round| *round > 0)
        .ok_or(())?;
    let seconds = event.data["verification_expires_at"]
        .as_i64()
        .filter(|value| *value > 0)
        .ok_or(())?;
    let deadline = DateTime::from_timestamp(seconds, 0).ok_or(())?;
    // Never depend on benchmark.delivery_deadline or contract funding_deadline.
    // Autonomous-v1 always emits its separate relative review deadline.
    if deadline <= now || event.occurred_at > now {
        return Ok(Vec::new());
    }
    if deadline <= event.occurred_at || event.log_key.is_empty() {
        return Err(());
    }
    let contract = wallet(&item.bounty_contract).ok_or(())?;
    if !event.contract_address.eq_ignore_ascii_case(&contract)
        || !event.bounty_id.eq_ignore_ascii_case(&item.bounty_id)
    {
        return Err(());
    }
    Ok(wallets
        .into_iter()
        .map(|verifier_wallet| PendingVerifierReview {
            bounty_contract: contract.clone(),
            bounty_id: item.bounty_id.to_ascii_lowercase(),
            round,
            verifier_wallet,
            submission_log_key: event.log_key.clone(),
            submitted_at: event.occurred_at,
            review_deadline: deadline,
        })
        .collect())
}

fn wallet(value: &str) -> Option<String> {
    (value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
        && value[2..].bytes().any(|byte| byte != b'0'))
    .then(|| value.to_ascii_lowercase())
}

pub fn review_indexer_is_fresh(heartbeat: &BaseIndexerHeartbeat, now: DateTime<Utc>) -> bool {
    let healthy = heartbeat.status == "success"
        || (heartbeat.status == "skipped"
            && heartbeat.skipped_reason.as_deref()
                == Some("no confirmed blocks are ready to scan"));
    let Some(completed_at) = heartbeat.completed_at else {
        return false;
    };
    let age = now.signed_duration_since(completed_at).num_seconds();
    healthy
        && heartbeat.error_message.is_none()
        && (0..=300).contains(&age)
        && heartbeat
            .latest_block
            .zip(heartbeat.persisted_cursor_block)
            .is_some_and(|(latest, cursor)| cursor <= latest && cursor.saturating_add(20) >= latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::{
        build_autonomous_bounty_terms_record, evm_address_word, evm_event_topic, evm_uint256_word,
        evm_words_data, AutonomousBountyLogDecoder, EvmLog, BASE_MAINNET_USDC_TOKEN_ADDRESS,
    };
    use chrono::Duration;
    use serde_json::json;
    use sha3::{Digest, Keccak256};
    use uuid::Uuid;

    fn item(now: DateTime<Utc>) -> AutonomousBountyFeedItem {
        let contract = format!("0x{}", "1".repeat(40));
        let verifier = format!("0x{}", "2".repeat(40));
        serde_json::from_value(json!({
            "bounty_id": format!("0x{}", "a".repeat(64)), "bounty_contract": contract,
            "creator": format!("0x{}", "3".repeat(40)), "status":"submitted",
            "solver_reward":"15000000", "verifier_reward":"2000000", "claim_bond":"2000000",
            "timeout_bond_pool":"0", "target_amount":"17000000", "funded_amount":"17000000",
            "terms_hash":"terms", "terms_valid":true, "verification_mode":"signed_quorum",
            "verifier_module":null, "verification_ready":false,
            "verification_readiness_reason":"runner unavailable", "validation_errors":[],
            "terms": {"terms_hash":"terms", "policy_hash":"policy", "acceptance_criteria_hash":"criteria",
                "benchmark_hash":"benchmark", "evidence_schema_hash":"schema", "creator_wallet":"creator",
                "created_at":now, "document": {"schema_version":"agent-bounties/terms-v1", "contract_terms":{},
                    "title":"PRIVATE TITLE", "goal":"PRIVATE CONTENT", "acceptance_criteria":[], "benchmark":{},
                    "evidence_schema":{}, "verification_policy":{"mechanism":"signed_quorum","verifiers":[verifier]},
                    "source_url":null,"discovery_source":null}},
            "events":[{"id":Uuid::new_v4(),"log_key":"tx:1","tx_hash":"tx","block_number":1,"log_index":1,
                "contract_address":contract,"bounty_id":format!("0x{}", "a".repeat(64)),"kind":"submission_added",
                "occurred_at":now-Duration::seconds(10),"data":{"round":3,"verification_expires_at":(now+Duration::hours(2)).timestamp()}}]
        })).unwrap()
    }

    #[test]
    fn no_bounty_deadline_still_notifies_actual_verifier_when_runner_unready() {
        let now = Utc::now();
        let item = item(now);
        let pending = review_recipients(&item, now).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].verifier_wallet, format!("0x{}", "2".repeat(40)));
        assert_ne!(pending[0].verifier_wallet, item.creator);
        assert_eq!(
            pending[0].review_deadline,
            now.with_nanosecond(0).unwrap() + Duration::hours(2)
        );
        assert!(!format!("{pending:?}").contains("PRIVATE"));
    }

    #[test]
    fn duplicate_verifiers_collapse_and_no_designation_has_no_fallback() {
        let now = Utc::now();
        let mut item = item(now);
        let policy = &mut item.terms.as_mut().unwrap().document.verification_policy;
        let address = policy["verifiers"][0].clone();
        policy["verifiers"] = json!([address, address]);
        assert_eq!(review_recipients(&item, now).unwrap().len(), 1);
        item.terms.as_mut().unwrap().document.verification_policy["verifiers"] = json!([]);
        assert!(review_recipients(&item, now).unwrap().is_empty());
    }

    #[test]
    fn only_live_rounds_with_valid_terms_and_emitted_deadlines_are_eligible() {
        let now = Utc::now();
        for status in ["paid", "cancelled", "claimable", "claimed"] {
            let mut item = item(now);
            item.status = status.into();
            assert!(review_recipients(&item, now).unwrap().is_empty());
        }
        let mut invalid = item(now);
        invalid.terms_valid = false;
        assert!(review_recipients(&invalid, now).unwrap().is_empty());
        let mut expired = item(now);
        expired.events[0].data["verification_expires_at"] = json!(now.timestamp());
        assert!(review_recipients(&expired, now).unwrap().is_empty());
        let mut missing = item(now);
        missing.events[0].data["verification_expires_at"] = json!(null);
        assert!(review_recipients(&missing, now).is_err());
    }

    #[test]
    fn unrelated_emitters_and_malformed_wallets_never_become_recipients() {
        let now = Utc::now();
        let mut bad = item(now);
        bad.events[0].contract_address = format!("0x{}", "9".repeat(40));
        assert!(review_recipients(&bad, now).is_err());
        let mut bad = item(now);
        bad.terms.as_mut().unwrap().document.verification_policy["verifiers"] =
            json!(["attacker@example.com"]);
        assert!(review_recipients(&bad, now).is_err());
        assert!(wallet("0x0000000000000000000000000000000000000000").is_none());
    }

    #[test]
    fn delivery_requires_a_recent_successful_indexer_near_the_chain_tip() {
        let now = Utc::now();
        let heartbeat = BaseIndexerHeartbeat {
            network: NETWORK.into(),
            escrow_contract: format!("0x{}", "1".repeat(40)),
            status: "success".into(),
            started_at: now,
            completed_at: Some(now),
            latest_block: Some(100),
            confirmed_to_block: Some(98),
            from_block: Some(98),
            to_block: Some(98),
            fetched_logs: 0,
            persisted_cursor_block: Some(98),
            skipped_reason: None,
            error_message: None,
            updated_at: now,
        };
        assert!(review_indexer_is_fresh(&heartbeat, now));
        for seconds in [-1, 301] {
            let mut stale = heartbeat.clone();
            stale.completed_at = Some(now - Duration::seconds(seconds));
            assert!(!review_indexer_is_fresh(&stale, now));
        }
        for cursor in [79, 101] {
            let mut lagging = heartbeat.clone();
            lagging.persisted_cursor_block = Some(cursor);
            assert!(!review_indexer_is_fresh(&lagging, now));
        }
        let mut failed = heartbeat.clone();
        failed.error_message = Some("RPC unavailable".into());
        assert!(!review_indexer_is_fresh(&failed, now));
        let mut idle = heartbeat;
        idle.status = "skipped".into();
        idle.skipped_reason = Some("no confirmed blocks are ready to scan".into());
        assert!(review_indexer_is_fresh(&idle, now));
        idle.skipped_reason = Some("indexing disabled".into());
        assert!(!review_indexer_is_fresh(&idle, now));
    }

    fn canonical_fixture(
        now: DateTime<Utc>,
    ) -> (
        String,
        Vec<AutonomousBountyEvent>,
        AutonomousBountyTermsRecord,
    ) {
        let factory = format!("0x{}", "1".repeat(40));
        let contract = format!("0x{}", "2".repeat(40));
        let creator = format!("0x{}", "3".repeat(40));
        let verifiers = [
            format!("0x{}", "4".repeat(40)),
            format!("0x{}", "5".repeat(40)),
        ];
        let solver = format!("0x{}", "6".repeat(40));
        let bounty_id = format!("0x{}", "a".repeat(64));
        let nonce = format!("0x{}", "b".repeat(64));
        let created_at = now - Duration::hours(1);
        let submitted_at = now - Duration::minutes(1);
        let deadline = submitted_at + Duration::hours(1);
        let terms = build_autonomous_bounty_terms_record(&creator, serde_json::from_value(json!({
            "schema_version": "agent-bounties/terms-v1",
            "contract_terms": {
                "protocol_version": "agent-bounties/autonomous-v1",
                "creator_wallet": creator,
                "network": NETWORK,
                "settlement_token": BASE_MAINNET_USDC_TOKEN_ADDRESS,
                "solver_reward": {"amount": 900000, "currency": "usdc"},
                "verifier_reward": {"amount": 100000, "currency": "usdc"},
                "claim_bond": {"amount": 100000, "currency": "usdc"},
                "initial_funding": {"amount": 1000000, "currency": "usdc"},
                "funding_deadline": (now + Duration::days(1)).timestamp(),
                "claim_window_seconds": 3600,
                "verification_window_seconds": 3600,
                "creation_nonce": nonce,
            },
            "title": "Synthetic private review fixture",
            "goal": "Inspect the submitted fixture against the agreed criteria.",
            "acceptance_criteria": ["The submitted fixture matches the expected result."],
            "benchmark": {"engine": "manual_fixture_review"},
            "evidence_schema": {"type": "object", "required": ["artifact_digest"]},
            "verification_policy": {"mechanism": "signed_quorum", "threshold": 1, "verifiers": verifiers},
        })).unwrap(), created_at).unwrap();
        // ABI encoding of address[] binds both designated wallets to the
        // contract event, independently of the terms' JSON representation.
        let verifier_words = vec![
            evm_uint256_word(32),
            evm_uint256_word(2),
            evm_address_word(&verifiers[0]).unwrap(),
            evm_address_word(&verifiers[1]).unwrap(),
        ];
        let encoded = evm_words_data(&verifier_words).unwrap();
        let set_hash = format!(
            "0x{}",
            hex::encode(Keccak256::digest(hex::decode(&encoded[2..]).unwrap()))
        );
        let zero = format!("0x{}", "0".repeat(40));
        let decode = |signature: &str,
                      address: &str,
                      extra_topics: Vec<String>,
                      words: Vec<String>,
                      block,
                      index,
                      occurred_at| {
            let mut topics = vec![evm_event_topic(signature), bounty_id.clone()];
            topics.extend(extra_topics);
            AutonomousBountyLogDecoder
                .decode(EvmLog {
                    address: address.to_owned(),
                    topics,
                    data: evm_words_data(&words).unwrap(),
                    tx_hash: format!("0x{block:064x}"),
                    block_number: block,
                    log_index: index,
                    occurred_at: Some(occurred_at),
                })
                .unwrap()
        };
        let events = vec![
            decode("CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)", &factory,
                vec![evm_address_word(&contract).unwrap(), evm_address_word(&creator).unwrap()],
                vec![terms.terms_hash.clone(), terms.policy_hash.clone(), nonce], 10, 0, created_at),
            decode("CanonicalBountyTermsCommitted(bytes32,bytes32,bytes32,bytes32)", &factory, vec![],
                vec![terms.acceptance_criteria_hash.clone(), terms.benchmark_hash.clone(), terms.evidence_schema_hash.clone()], 10, 1, created_at),
            decode("CanonicalBountyEconomicsConfigured(bytes32,uint256,uint256,uint256,uint256,uint64,uint64,uint64)", &factory, vec![],
                vec![evm_uint256_word(900000), evm_uint256_word(100000), evm_uint256_word(1000000), evm_uint256_word(1000000),
                    evm_uint256_word((now + Duration::days(1)).timestamp() as u128), evm_uint256_word(3600), evm_uint256_word(3600)], 10, 2, created_at),
            decode("CanonicalBountyVerificationConfigured(bytes32,uint8,address,address,uint8,bytes32)", &factory, vec![],
                vec![evm_uint256_word(1), evm_address_word(&zero).unwrap(), evm_address_word(&zero).unwrap(), evm_uint256_word(1), set_hash], 10, 3, created_at),
            decode("BountyClaimed(bytes32,uint64,address,bytes32,bytes32,uint256,uint64)", &contract,
                vec![evm_uint256_word(1), evm_address_word(&solver).unwrap()],
                vec![terms.terms_hash.clone(), terms.policy_hash.clone(), evm_uint256_word(100000), evm_uint256_word(deadline.timestamp() as u128)],
                11, 0, now - Duration::minutes(2)),
            decode("SubmissionAdded(bytes32,uint64,address,bytes32,bytes32,uint64)", &contract,
                vec![evm_uint256_word(1), evm_address_word(&solver).unwrap()],
                vec![format!("0x{}", "c".repeat(64)), format!("0x{}", "d".repeat(64)), evm_uint256_word(deadline.timestamp() as u128)],
                12, 0, submitted_at),
        ];
        (factory, events, terms)
    }

    #[test]
    fn decoded_canonical_submission_notifies_all_designated_verifiers_without_a_bounty_deadline() {
        let now = DateTime::from_timestamp(1800000000, 0).unwrap();
        let (factory, events, terms) = canonical_fixture(now);
        assert!(terms.document.benchmark.get("delivery_deadline").is_none());
        let feed =
            build_autonomous_bounty_feed(events.clone(), vec![terms.clone()], false).unwrap();
        assert!(feed[0].terms_valid, "{:?}", feed[0].validation_errors);
        assert_eq!(feed[0].status, "submitted");
        assert!(
            !feed[0].verification_ready,
            "unregistered fixture verifiers have no hosted runner"
        );
        let (pending, retained, report) =
            project_verifier_reviews(&factory, events, vec![terms], now);
        assert_eq!(pending.len(), 2);
        assert_eq!(
            pending
                .iter()
                .map(|review| review.verifier_wallet.clone())
                .collect::<Vec<_>>(),
            vec![
                format!("0x{}", "4".repeat(40)),
                format!("0x{}", "5".repeat(40))
            ]
        );
        assert!(pending.iter().all(|review| review.round == 1
            && review.review_deadline == now + Duration::minutes(59)
            && review.submission_log_key.ends_with(":0")));
        assert!(retained.is_empty());
        assert_eq!(report.active_reviews, 2);
        assert_eq!(report.invalid_bounties, 0);
        assert_eq!(report.unavailable_terms, 0);
    }

    #[test]
    fn canonical_submission_with_missing_or_mismatched_terms_is_suspended() {
        let now = DateTime::from_timestamp(1800000000, 0).unwrap();
        let (factory, events, terms) = canonical_fixture(now);
        let mut wrong_policy = terms.clone();
        wrong_policy.document.verification_policy["verifiers"][0] =
            json!(format!("0x{}", "9".repeat(40)));
        let mut wrong_hash = terms.clone();
        wrong_hash.acceptance_criteria_hash = format!("0x{}", "f".repeat(64));
        for available in [vec![], vec![wrong_policy], vec![wrong_hash]] {
            let (pending, retained, report) =
                project_verifier_reviews(&factory, events.clone(), available, now);
            assert!(pending.is_empty());
            assert_eq!(retained, vec![(format!("0x{}", "2".repeat(40)), 1)]);
            assert_eq!(report.unavailable_terms, 1);
        }
        // Repairing availability resumes the same canonical occurrence.
        let (pending, _, _) = project_verifier_reviews(&factory, events, vec![terms], now);
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn malformed_unrelated_canonical_bounty_does_not_block_valid_review_requests() {
        let now = DateTime::from_timestamp(1800000000, 0).unwrap();
        let (factory, mut events, terms) = canonical_fixture(now);
        let mut malformed = events.clone();
        for event in &mut malformed {
            event.bounty_id = format!("0x{}", "e".repeat(64));
            if event.contract_address != factory {
                event.contract_address = format!("0x{}", "7".repeat(40));
            }
            if event.kind == AutonomousBountyEventKind::CanonicalBountyCreated {
                event.data["bounty_contract"] = json!(format!("0x{}", "7".repeat(40)));
            }
        }
        malformed.retain(|event| {
            event.kind != AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured
        });
        events.extend(malformed);
        events.reverse();
        let (pending, retained, report) =
            project_verifier_reviews(&factory, events, vec![terms], now);
        assert_eq!(pending.len(), 2);
        assert!(pending
            .iter()
            .all(|review| review.bounty_contract == format!("0x{}", "2".repeat(40))));
        assert_eq!(retained, vec![(format!("0x{}", "7".repeat(40)), 1)]);
        assert_eq!(report.invalid_bounties, 1);
        assert_eq!(report.active_reviews, 2);
    }

    use chrono::Timelike;
}
