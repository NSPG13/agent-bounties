//! Durable Stripe transitions. Processor evidence is never applied to the API
//! cache until the database transaction commits; restart/retry uses DB receipts.
use app::{
    BountyNetwork, StripeConnectPayoutReconciliation, StripeFundingReconciliation,
    StripeTransferReconciliation,
};
use db::{PostgresStore, StripeReconciliationSnapshot};
use domain::Id;
use ledger::Ledger;
use payments_stripe::{ConnectAccountSnapshot, ConnectTransferEvidence, StripeFundingCredit};
use serde_json::json;
use sha2::{Digest, Sha256};

fn snapshot_network(snapshot: StripeReconciliationSnapshot) -> anyhow::Result<BountyNetwork> {
    Ok(BountyNetwork {
        agents: snapshot.agents.into_iter().map(|v| (v.id, v)).collect(),
        bounties: snapshot.bounties.into_iter().map(|v| (v.id, v)).collect(),
        funding_intents: snapshot
            .funding_intents
            .into_iter()
            .map(|v| (v.id, v))
            .collect(),
        funding_contributions: snapshot
            .funding_contributions
            .into_iter()
            .map(|v| (v.id, v))
            .collect(),
        settlements: snapshot
            .settlements
            .into_iter()
            .map(|v| (v.id, v))
            .collect(),
        payment_events: snapshot
            .payment_events
            .into_iter()
            .map(|v| (v.id, v))
            .collect(),
        ledger: Ledger::from_entries(snapshot.ledger_entries)?,
        ..BountyNetwork::default()
    })
}

fn binding_hash(value: serde_json::Value) -> anyhow::Result<String> {
    Ok(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(serde_json::to_vec(&value)?))
    ))
}

/// The outcome is historical evidence; snapshot is current committed state.
/// Callers serialize the whole reconciliation-and-cache-install interval.
pub struct DurableStripeOutcome<T> {
    pub outcome: T,
    snapshot: StripeReconciliationSnapshot,
    bounty_ids: Vec<Id>,
}

impl<T> DurableStripeOutcome<T> {
    pub fn update_cache(&self, network: &mut BountyNetwork) -> anyhow::Result<()> {
        let mut candidate = network.clone();
        // Immutable ledger entries can be merged without replacing unrelated
        // cached work. Detect disagreements rather than silently losing them.
        for entry in &self.snapshot.ledger_entries {
            if let Some(existing) = candidate.ledger.entries().iter().find(|e| {
                e.id == entry.id
                    || (e.external_event_id.is_some()
                        && e.external_event_id == entry.external_event_id)
            }) {
                anyhow::ensure!(
                    serde_json::to_value(existing)? == serde_json::to_value(entry)?,
                    "financial cache disagrees with durable ledger"
                );
            } else {
                candidate.ledger.append(entry.clone())?;
            }
        }
        for event in &self.snapshot.payment_events {
            candidate
                .payment_events
                .retain(|_, old| old.external_id != event.external_id);
            candidate.payment_events.insert(event.id, event.clone());
        }
        for &id in &self.bounty_ids {
            let bounty = self
                .snapshot
                .bounties
                .iter()
                .find(|v| v.id == id)
                .ok_or_else(|| anyhow::anyhow!("committed payment bounty missing"))?;
            candidate.bounties.insert(id, bounty.clone());
            for v in self
                .snapshot
                .funding_intents
                .iter()
                .filter(|v| v.bounty_id == id)
            {
                candidate.funding_intents.insert(v.id, v.clone());
            }
            for v in self
                .snapshot
                .funding_contributions
                .iter()
                .filter(|v| v.bounty_id == id)
            {
                candidate.funding_contributions.insert(v.id, v.clone());
            }
            for v in self
                .snapshot
                .settlements
                .iter()
                .filter(|v| v.bounty_id == id)
            {
                candidate.settlements.insert(v.id, v.clone());
            }
        }
        *network = candidate;
        Ok(())
    }
}

/// Callers serialize this operation through cache installation with their local
/// Stripe gate. The database lock, not that local gate, excludes other services.
pub async fn reconcile_stripe_connect_snapshot(
    store: Option<&PostgresStore>,
    network: &std::sync::Mutex<BountyNetwork>,
    observation: ConnectAccountSnapshot,
) -> anyhow::Result<StripeConnectPayoutReconciliation> {
    let Some(store) = store else {
        let mut guard = network.lock().expect("state poisoned");
        let mut candidate = guard.clone();
        let outcome = candidate.apply_stripe_connect_snapshot(observation)?;
        *guard = candidate;
        return Ok(outcome);
    };
    let mut tx = store.begin_stripe_connect_snapshot().await?;
    let mut candidate = snapshot_network(tx.snapshot().await?)?;
    // Refresh even already-paid settlements omitted from the eligibility result:
    // the caller's cache may predate their confirmed transfer.
    let bounty_ids = candidate
        .settlements
        .values()
        .filter(|s| s.rail == domain::PaymentRail::StripeFiat)
        .filter(|s| {
            s.payout_intents
                .iter()
                .any(|p| p.recipient_agent_id == observation.agent_id)
        })
        .map(|s| s.bounty_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let outcome = candidate.apply_stripe_connect_snapshot(observation)?;
    anyhow::ensure!(
        outcome.bounties.is_empty() && outcome.ledger_entries.is_empty(),
        "Connect eligibility must not move money or change bounty status"
    );
    tx.persist_settlements(&outcome.settlements).await?;
    let durable = DurableStripeOutcome {
        outcome,
        snapshot: tx.commit_with_snapshot().await?,
        bounty_ids,
    };
    durable.update_cache(&mut network.lock().expect("state poisoned"))?;
    Ok(durable.outcome)
}

pub async fn reconcile_stripe_funding(
    store: &PostgresStore,
    credit: StripeFundingCredit,
) -> anyhow::Result<DurableStripeOutcome<StripeFundingReconciliation>> {
    let bounty_id = credit.bounty_id;
    let binding = binding_hash(json!({
        "organization_id": credit.organization_id,
        "amount": credit.amount,
        "payment_intent_id": credit.payment_intent_id,
        "bounty_id": credit.bounty_id,
        "funding_intent_id": credit.funding_intent_id,
    }))?;
    let mut tx = store
        .begin_stripe_reconciliation(
            &credit.payment_event,
            "checkout.session",
            &credit.checkout_session_id,
            &binding,
        )
        .await?;
    if let Some(value) = tx.existing_outcome().cloned() {
        let mut result: StripeFundingReconciliation = serde_json::from_value(value.clone())?;
        let snapshot = tx.commit_with_snapshot(&value).await?;
        result.duplicate = true;
        return Ok(DurableStripeOutcome {
            outcome: result,
            snapshot,
            bounty_ids: bounty_id.into_iter().collect(),
        });
    }
    let mut candidate = snapshot_network(tx.snapshot().await?)?;
    let result = candidate.apply_stripe_funding_credit(credit)?;
    // Old nontransactional receipts cannot prove all writes succeeded. Require
    // reconciliation/migration rather than cementing a possibly partial credit.
    anyhow::ensure!(
        !result.duplicate,
        "legacy Stripe credit needs reconciliation"
    );
    tx.persist_funding(
        result.funding_intent.as_ref(),
        result.funding_report.as_ref().map(|r| &r.bounty),
        result.funding_report.as_ref().map(|r| &r.contribution),
        &result.ledger_entries,
    )
    .await?;
    let snapshot = tx
        .commit_with_snapshot(&serde_json::to_value(&result)?)
        .await?;
    Ok(DurableStripeOutcome {
        outcome: result,
        snapshot,
        bounty_ids: bounty_id.into_iter().collect(),
    })
}

pub async fn reconcile_stripe_transfer(
    store: &PostgresStore,
    evidence: ConnectTransferEvidence,
) -> anyhow::Result<DurableStripeOutcome<StripeTransferReconciliation>> {
    let bounty_id = Some(evidence.bounty_id);
    let binding = binding_hash(json!({
        "connected_account_id": evidence.connected_account_id,
        "bounty_id": evidence.bounty_id,
        "proof_record_id": evidence.proof_record_id,
        "settlement_id": evidence.settlement_id,
        "payout_intent_id": evidence.payout_intent_id,
        "agent_id": evidence.agent_id,
        "amount": evidence.amount,
    }))?;
    let mut tx = store
        .begin_stripe_reconciliation(
            &evidence.payment_event,
            "transfer",
            &evidence.transfer_id,
            &binding,
        )
        .await?;
    if let Some(value) = tx.existing_outcome().cloned() {
        let mut result: StripeTransferReconciliation = serde_json::from_value(value.clone())?;
        let snapshot = tx.commit_with_snapshot(&value).await?;
        result.duplicate = true;
        return Ok(DurableStripeOutcome {
            outcome: result,
            snapshot,
            bounty_ids: bounty_id.into_iter().collect(),
        });
    }
    let mut candidate = snapshot_network(tx.snapshot().await?)?;
    let result = candidate.apply_stripe_transfer_evidence(evidence)?;
    anyhow::ensure!(
        !result.duplicate,
        "legacy Stripe transfer needs reconciliation"
    );
    tx.persist_transfer(
        result.settlement.as_ref(),
        result.bounty.as_ref(),
        &result.ledger_entries,
    )
    .await?;
    let snapshot = tx
        .commit_with_snapshot(&serde_json::to_value(&result)?)
        .await?;
    Ok(DurableStripeOutcome {
        outcome: result,
        snapshot,
        bounty_ids: bounty_id.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::OpenPooledBountyRequest;
    use chrono::Utc;
    use domain::{
        FundingIntent, FundingIntentStatus, FundingMode, Money, PaymentEvent, PaymentEventStatus,
        PaymentRail, PrivacyLevel,
    };
    use uuid::Uuid;

    // Synthetic records only; this fixture neither contacts Stripe nor pays an account.
    async fn connect_fixture(store: &PostgresStore) -> (BountyNetwork, domain::Settlement) {
        use domain::*;
        let mut network = BountyNetwork::default();
        let mut bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Connect concurrency fixture".into(),
                template_slug: "write-docs-for-area".into(),
                target_amount_minor: 1000,
                currency: "usd".into(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Private,
                funding_targets: vec![],
            })
            .unwrap();
        bounty.status = BountyStatus::Payable;
        store.upsert_bounty(&bounty).await.unwrap();
        let agent = Agent::new(format!("connect-fixture-{}", Uuid::new_v4()));
        store.upsert_agent(&agent).await.unwrap();
        let submission = Submission {
            id: Uuid::new_v4(),
            bounty_id: bounty.id,
            solver_agent_id: agent.id,
            artifact_digest: "fixture".into(),
            artifact_uri: "private://fixture".into(),
            submitted_at: Utc::now(),
        };
        store.upsert_submission(&submission).await.unwrap();
        let result = VerifierResult {
            id: Uuid::new_v4(),
            bounty_id: bounty.id,
            submission_id: submission.id,
            verifier_agent_id: None,
            kind: VerifierKind::Manual,
            decision: VerificationDecision::Accepted,
            summary: "synthetic".into(),
            confidence: 1.0,
            signed_payload_hash: "fixture".into(),
            created_at: Utc::now(),
        };
        store.upsert_verifier_result(&result).await.unwrap();
        let proof = ProofRecord {
            id: Uuid::new_v4(),
            bounty_id: bounty.id,
            submission_id: submission.id,
            verifier_result_id: result.id,
            proof_hash: "fixture".into(),
            public_summary: "".into(),
            privacy: PrivacyLevel::Private,
            created_at: Utc::now(),
        };
        store.upsert_proof_record(&proof).await.unwrap();
        let settlement = Settlement {
            id: Uuid::new_v4(),
            bounty_id: bounty.id,
            proof_record_id: proof.id,
            rail: PaymentRail::StripeFiat,
            platform_fee: Money::zero("usd"),
            created_at: Utc::now(),
            payout_intents: vec![PayoutIntent {
                id: Uuid::new_v4(),
                bounty_id: bounty.id,
                recipient_agent_id: agent.id,
                rail: PaymentRail::StripeFiat,
                amount: Money::new(1000, "usd").unwrap(),
                status: PayoutStatus::Blocked,
            }],
        };
        store.upsert_settlement(&settlement).await.unwrap();
        (
            crate::hydrate_bounty_network(store).await.unwrap(),
            settlement,
        )
    }

    fn connect_observation(
        settlement: &domain::Settlement,
        eligible: bool,
    ) -> ConnectAccountSnapshot {
        ConnectAccountSnapshot {
            agent_id: settlement.payout_intents[0].recipient_agent_id,
            connected_account_id: Some("acct_synthetic".into()),
            payouts_enabled: eligible,
            disabled_reason: None,
            currently_due: vec![],
        }
    }

    fn transfer_evidence(settlement: &domain::Settlement) -> ConnectTransferEvidence {
        ConnectTransferEvidence {
            connected_account_id: "acct_synthetic".into(),
            transfer_id: format!("tr_synthetic_{}", Uuid::new_v4().simple()),
            bounty_id: settlement.bounty_id,
            proof_record_id: settlement.proof_record_id,
            settlement_id: settlement.id,
            payout_intent_id: settlement.payout_intents[0].id,
            agent_id: settlement.payout_intents[0].recipient_agent_id,
            amount: settlement.payout_intents[0].amount.clone(),
            payment_event: PaymentEvent {
                id: Uuid::new_v4(),
                rail: PaymentRail::StripeFiat,
                external_id: format!("evt_synthetic_{}", Uuid::new_v4().simple()),
                status: PaymentEventStatus::Applied,
                payload_hash: "sha256:connect-fixture".into(),
                received_at: Utc::now(),
            },
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn connect_snapshot_reads_committed_transfer_instead_of_stale_cache() {
        use domain::{BountyStatus, PayoutStatus};
        use std::{sync::Mutex, time::Duration};
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let (network, settlement) = connect_fixture(&store).await;
        let stale = Mutex::new(network);
        // A separately connected process applies a transfer while this process
        // still sees a blocked payout. Keep its transaction open to prove the
        // eligibility reader waits for the committed state.
        let evidence = transfer_evidence(&settlement);
        let mut payment_tx = store
            .begin_stripe_reconciliation(
                &evidence.payment_event,
                "transfer",
                &evidence.transfer_id,
                &binding_hash(json!({"fixture": settlement.id})).unwrap(),
            )
            .await
            .unwrap();
        let mut candidate = snapshot_network(payment_tx.snapshot().await.unwrap()).unwrap();
        let payment = candidate.apply_stripe_transfer_evidence(evidence).unwrap();
        payment_tx
            .persist_transfer(
                payment.settlement.as_ref(),
                payment.bounty.as_ref(),
                &payment.ledger_entries,
            )
            .await
            .unwrap();
        let other = PostgresStore::connect(&url).await.unwrap();
        let operation = reconcile_stripe_connect_snapshot(
            Some(&other),
            &stale,
            connect_observation(&settlement, true),
        );
        tokio::pin!(operation);
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut operation)
                .await
                .is_err()
        );
        payment_tx
            .commit(&serde_json::to_value(&payment).unwrap())
            .await
            .unwrap();
        let observation = operation.await.unwrap();
        let committed = store.list_settlements().await.unwrap();
        assert_eq!(
            committed
                .iter()
                .find(|s| s.id == settlement.id)
                .unwrap()
                .payout_intents[0]
                .status,
            PayoutStatus::Paid,
            "eligibility must not overwrite the payment committed by another process"
        );
        assert!(observation.settlements.is_empty());
        assert!(observation.ledger_entries.is_empty());
        {
            let network = stale.lock().unwrap();
            assert_eq!(
                network.settlements[&settlement.id].payout_intents[0].status,
                PayoutStatus::Paid
            );
            assert_eq!(
                network.bounties[&settlement.bounty_id].status,
                BountyStatus::Paid
            );
        }
        // A later ineligible observation cannot undo payment either. Discarding
        // responses and reconnecting must not invent a second ledger movement.
        reconcile_stripe_connect_snapshot(
            Some(&other),
            &stale,
            connect_observation(&settlement, false),
        )
        .await
        .unwrap();
        let restarted = PostgresStore::connect(&url).await.unwrap();
        let hydrated = crate::hydrate_bounty_network(&restarted).await.unwrap();
        assert_eq!(
            hydrated.settlements[&settlement.id].payout_intents[0].status,
            PayoutStatus::Paid
        );
        for entry in &payment.ledger_entries {
            assert_eq!(
                hydrated
                    .ledger
                    .entries()
                    .iter()
                    .filter(|v| v.id == entry.id)
                    .count(),
                1
            );
        }
        // Exercise the opposite order: eligibility commits first, then a payment
        // waiting in another service reads that state and pays exactly once.
        let (_, second) = connect_fixture(&store).await;
        let mut eligibility_tx = store.begin_stripe_connect_snapshot().await.unwrap();
        let mut candidate = snapshot_network(eligibility_tx.snapshot().await.unwrap()).unwrap();
        let eligible = candidate
            .apply_stripe_connect_snapshot(connect_observation(&second, true))
            .unwrap();
        eligibility_tx
            .persist_settlements(&eligible.settlements)
            .await
            .unwrap();
        let transfer = reconcile_stripe_transfer(&other, transfer_evidence(&second));
        tokio::pin!(transfer);
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut transfer)
                .await
                .is_err()
        );
        eligibility_tx.commit_with_snapshot().await.unwrap();
        let paid = transfer.await.unwrap();
        assert!(!paid.outcome.duplicate);
        assert_eq!(
            paid.outcome.settlement.unwrap().payout_intents[0].status,
            PayoutStatus::Paid
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn connect_snapshot_rolls_back_all_settlements_and_keeps_cache_on_failure() {
        use domain::PayoutStatus;
        use std::sync::Mutex;
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let (network, settlement) = connect_fixture(&store).await;
        let cache = Mutex::new(network);
        let mut changed = settlement.clone();
        changed.payout_intents[0].status = PayoutStatus::Pending;
        let mut invalid = changed.clone();
        invalid.id = Uuid::new_v4();
        invalid.proof_record_id = Uuid::new_v4(); // Foreign-key failure after first write.
        let mut tx = store.begin_stripe_connect_snapshot().await.unwrap();
        assert!(tx.persist_settlements(&[changed, invalid]).await.is_err());
        drop(tx);
        let restored = crate::hydrate_bounty_network(&store).await.unwrap();
        assert_eq!(
            restored.settlements[&settlement.id].payout_intents[0].status,
            PayoutStatus::Blocked
        );
        // A rejected database write must also leave the caller cache untouched.
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        let constraint = format!("connect_fixture_{}", settlement.id.simple());
        sqlx::query(&format!("ALTER TABLE settlements ADD CONSTRAINT {constraint} CHECK (id <> '{}' OR payout_intents #>> '{{0,status}}' <> 'Pending')", settlement.id))
            .execute(&pool).await.unwrap();
        let failed = reconcile_stripe_connect_snapshot(
            Some(&store),
            &cache,
            connect_observation(&settlement, true),
        )
        .await;
        sqlx::query(&format!(
            "ALTER TABLE settlements DROP CONSTRAINT {constraint}"
        ))
        .execute(&pool)
        .await
        .unwrap();
        assert!(failed.is_err());
        assert_eq!(
            cache.lock().unwrap().settlements[&settlement.id].payout_intents[0].status,
            PayoutStatus::Blocked
        );
        assert_eq!(
            crate::hydrate_bounty_network(&store)
                .await
                .unwrap()
                .settlements[&settlement.id]
                .payout_intents[0]
                .status,
            PayoutStatus::Blocked
        );
        // Retry the same observation after recovery, then reverse eligibility.
        // This must remain reversible for unpaid payouts; it is not a blanket
        // monotonic-status rule that would hide genuine eligibility changes.
        for (eligible, expected) in [
            (true, PayoutStatus::Pending),
            (false, PayoutStatus::Blocked),
        ] {
            let result = reconcile_stripe_connect_snapshot(
                Some(&store),
                &cache,
                connect_observation(&settlement, eligible),
            )
            .await
            .unwrap();
            assert!(result.ledger_entries.is_empty());
            assert_eq!(
                cache.lock().unwrap().settlements[&settlement.id].payout_intents[0].status,
                expected
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn stripe_durable_funding_survives_lost_response_restart_and_conflicting_replays() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let mut network = BountyNetwork::default();
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Stripe durability fixture".into(),
                template_slug: "write-docs-for-area".into(),
                target_amount_minor: 1000,
                currency: "usd".into(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            })
            .unwrap();
        let org = Uuid::new_v4();
        let intent = FundingIntent {
            id: Uuid::new_v4(),
            bounty_id: bounty.id,
            contributor_agent_id: None,
            source_organization_id: Some(org),
            rail: PaymentRail::StripeFiat,
            amount: Money::new(1000, "usd").unwrap(),
            status: FundingIntentStatus::AwaitingEvidence,
            external_reference: None,
            stripe_success_url: None,
            stripe_cancel_url: None,
            created_at: Utc::now(),
        };
        store.upsert_bounty(&bounty).await.unwrap();
        store.upsert_funding_intent(&intent).await.unwrap();
        let credit = StripeFundingCredit {
            organization_id: org,
            amount: intent.amount.clone(),
            checkout_session_id: format!("cs_test_{}", Uuid::new_v4().simple()),
            payment_intent_id: Some(format!("pi_test_{}", Uuid::new_v4().simple())),
            bounty_id: Some(bounty.id),
            funding_intent_id: Some(intent.id),
            payment_event: PaymentEvent {
                id: Uuid::new_v4(),
                rail: PaymentRail::StripeFiat,
                external_id: format!("evt_{}", Uuid::new_v4().simple()),
                status: PaymentEventStatus::Applied,
                payload_hash: "sha256:initial-fixture".into(),
                received_at: Utc::now(),
            },
        };
        // Failed validation must not leave even the preceding top-up durable.
        let mut invalid = credit.clone();
        invalid.amount = Money::new(1001, "usd").unwrap();
        assert!(reconcile_stripe_funding(&store, invalid).await.is_err());
        let before = crate::hydrate_bounty_network(&store).await.unwrap();
        assert!(!before
            .payment_events
            .values()
            .any(|e| e.external_id == credit.payment_event.external_id));
        assert_eq!(
            before.funding_intents[&intent.id].status,
            FundingIntentStatus::AwaitingEvidence
        );

        // Discard the response/cache update exactly as a process dying after commit.
        let first = reconcile_stripe_funding(&store, credit.clone())
            .await
            .unwrap();
        assert!(!first.outcome.duplicate);
        assert_eq!(first.outcome.ledger_entries.len(), 2);
        drop(store);
        let store = PostgresStore::connect(&url).await.unwrap();
        let retry = reconcile_stripe_funding(&store, credit.clone())
            .await
            .unwrap();
        assert!(retry.outcome.duplicate);
        retry.update_cache(&mut network).unwrap();
        assert_eq!(
            network.funding_intents[&intent.id].status,
            FundingIntentStatus::Applied
        );
        assert_eq!(
            network
                .status(bounty.id)
                .unwrap()
                .funding_summary
                .applied
                .amount,
            1000
        );
        assert_eq!(
            retry.outcome.ledger_entries[0].id,
            first.outcome.ledger_entries[0].id
        );
        // A different Stripe event for the same session also has only one effect.
        let mut second = credit.clone();
        second.payment_event.id = Uuid::new_v4();
        second.payment_event.external_id = format!("evt_{}", Uuid::new_v4().simple());
        second.payment_event.payload_hash = "sha256:async-success-fixture".into();
        assert!(
            reconcile_stripe_funding(&store, second.clone())
                .await
                .unwrap()
                .outcome
                .duplicate
        );
        let alias_id = second.payment_event.external_id.clone();
        // Replaying the alias must keep IgnoredDuplicate in the durable audit log.
        assert!(
            reconcile_stripe_funding(&store, second.clone())
                .await
                .unwrap()
                .outcome
                .duplicate
        );
        second.organization_id = Uuid::new_v4();
        assert!(reconcile_stripe_funding(&store, second).await.is_err());
        let mut changed_event = credit.clone();
        changed_event.payment_event.payload_hash = "sha256:tampered-fixture".into();
        assert!(reconcile_stripe_funding(&store, changed_event)
            .await
            .is_err());

        let restored = crate::hydrate_bounty_network(&store).await.unwrap();
        assert_eq!(
            restored
                .payment_events
                .values()
                .find(|e| e.external_id == credit.payment_event.external_id)
                .unwrap()
                .status,
            PaymentEventStatus::Applied
        );
        assert_eq!(
            restored
                .payment_events
                .values()
                .find(|e| e.external_id == alias_id)
                .unwrap()
                .status,
            PaymentEventStatus::IgnoredDuplicate
        );

        assert_eq!(
            restored.funding_intents[&intent.id].status,
            FundingIntentStatus::Applied
        );
        assert_eq!(
            restored
                .status(bounty.id)
                .unwrap()
                .funding_summary
                .applied
                .amount,
            1000
        );
        for entry in &first.outcome.ledger_entries {
            assert_eq!(
                restored
                    .ledger
                    .entries()
                    .iter()
                    .filter(|e| e.id == entry.id)
                    .count(),
                1
            );
        }
        // Upgrade from event-keyed ledger storage: no session receipt can be inferred
        // from a later event ID, even for a generic top-up with no funding intent.
        let legacy_org = Uuid::new_v4();
        let mut legacy = credit.clone();
        legacy.organization_id = legacy_org;
        legacy.bounty_id = None;
        legacy.funding_intent_id = None;
        legacy.checkout_session_id = format!("cs_test_{}", Uuid::new_v4().simple());
        legacy.payment_event.id = Uuid::new_v4();
        legacy.payment_event.external_id = format!("evt_{}", Uuid::new_v4().simple());
        let entry = ledger::LedgerEntry::new(
            "stripe checkout top-up",
            Some(format!(
                "stripe-checkout-top-up:{}",
                legacy.payment_event.external_id
            )),
            vec![
                ledger::debit(format!("stripe_cash:{legacy_org}"), legacy.amount.clone()),
                ledger::credit(
                    format!("platform_balance:{legacy_org}"),
                    legacy.amount.clone(),
                ),
            ],
        )
        .unwrap();
        store.insert_ledger_entry(&entry).await.unwrap();
        store
            .upsert_payment_event(&legacy.payment_event)
            .await
            .unwrap();
        let mut alias = legacy.clone();
        alias.payment_event.id = Uuid::new_v4();
        alias.payment_event.external_id = format!("evt_{}", Uuid::new_v4().simple());
        alias.payment_event.payload_hash = "sha256:legacy-alias".into();
        let restarted = PostgresStore::connect(&url).await.unwrap();
        assert!(reconcile_stripe_funding(&restarted, alias.clone())
            .await
            .is_err());
        let hydrated = crate::hydrate_bounty_network(&restarted).await.unwrap();
        assert_eq!(
            hydrated.ledger.balance(
                &ledger::AccountCode::new(format!("platform_balance:{legacy_org}")),
                "usd"
            ),
            -legacy.amount.amount
        );
        assert!(!hydrated
            .payment_events
            .values()
            .any(|event| event.external_id == alias.payment_event.external_id));
        assert_eq!(
            hydrated
                .ledger
                .entries()
                .iter()
                .filter(|value| value.id == entry.id)
                .count(),
            1
        );
    }
}
