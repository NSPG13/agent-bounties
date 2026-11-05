use crate::{review_indexer_is_fresh, ReviewEmailOutcome, ReviewEmailSender};
use db::PostgresStore;
use serde::Serialize;

pub(crate) enum ReminderSource<'a> {
    LegacyFactory(&'a str),
    CreatorOpenRpc(&'a str),
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct ReminderDeliveryReport {
    pub provider_accepted: u64,
    pub retry_pending: u64,
    pub failed: u64,
    pub source_unavailable: u64,
}

pub(crate) async fn deliver_review_reminders(
    store: &PostgresStore,
    sender: &ReviewEmailSender,
    network: &str,
    source: ReminderSource<'_>,
) -> anyhow::Result<ReminderDeliveryReport> {
    let mut report = ReminderDeliveryReport::default();
    store.seed_review_reminders(network).await?;
    for _ in 0..4 {
        let Some(reminder) = store
            .claim_review_reminder_for_protocol(
                network,
                matches!(source, ReminderSource::CreatorOpenRpc(_)),
            )
            .await?
        else {
            break;
        };
        let current = match source {
            ReminderSource::LegacyFactory(factory) => store
                .get_base_indexer_heartbeat(network, factory)
                .await?
                .as_ref()
                .is_some_and(|h| review_indexer_is_fresh(h, chrono::Utc::now())),
            ReminderSource::CreatorOpenRpc(rpc) => {
                crate::creator_open_notifications::source_current(
                    store,
                    network,
                    &reminder.contract,
                    rpc,
                )
                .await?
            }
        };
        if !current {
            report.source_unavailable += 1;
            continue;
        }
        let payload = match reminder.payload.clone().map(Ok).unwrap_or_else(|| {
            sender.prepare_reminder(
                &reminder.email,
                &reminder.contract,
                (reminder.kind == "creator_open_submission").then_some(reminder.entry),
                reminder.deadline,
                &reminder.stage,
            )
        }) {
            Ok(payload) => payload,
            Err(_) => {
                store.finish_review_reminder(&reminder, None, false).await?;
                report.failed += 1;
                continue;
            }
        };
        let Some(payload) = store.begin_review_reminder(&reminder, &payload).await? else {
            continue;
        };
        let key = format!("review-reminder:{}:{}", reminder.id, reminder.stage);
        match sender.send(&key, &payload).await {
            ReviewEmailOutcome::Accepted { provider_id } => {
                store
                    .finish_review_reminder(&reminder, Some(&provider_id), false)
                    .await?;
                report.provider_accepted += 1;
            }
            ReviewEmailOutcome::Retryable { .. } => {
                store.finish_review_reminder(&reminder, None, true).await?;
                report.retry_pending += 1;
            }
            ReviewEmailOutcome::PermanentFailure { .. } => {
                store.finish_review_reminder(&reminder, None, false).await?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}
