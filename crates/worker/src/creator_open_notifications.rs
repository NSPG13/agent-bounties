//! Canonical creator-open notices. The read-only indexer owns this runtime;
//! provider acceptance is not delivery, verdict authorization or payment.
use crate::{CreatorPushSender, ReviewEmailConfig, ReviewEmailOutcome, ReviewEmailSender};
use db::{PostgresStore, ReviewNotificationOutcome};

pub(crate) async fn source_current(
    store: &PostgresStore,
    network: &str,
    bounty: &str,
    rpc: &str,
) -> anyhow::Result<bool> {
    let Some(index) = store
        .creator_open_index_batch(network, Some(bounty))
        .await?
        .pop()
    else {
        return Ok(false);
    };
    let Some(block) = index.checkpoint.as_ref() else {
        return Ok(false);
    };
    if !index.available || index.checked_at < chrono::Utc::now() - chrono::Duration::seconds(120) {
        return Ok(false);
    }
    let actual = chain_base::fetch_exact_block_identity(rpc, block.number, 71).await;
    if actual.as_ref().is_ok_and(|b| b.hash == block.hash) {
        return Ok(true);
    }
    store
        .invalidate_creator_open_index(&index, actual.is_ok())
        .await?;
    Ok(false)
}

pub struct CreatorOpenNotificationRuntime {
    email: Option<ReviewEmailSender>,
    push: Option<CreatorPushSender>,
}
impl CreatorOpenNotificationRuntime {
    pub fn from_env(network: &str) -> anyhow::Result<Option<Self>> {
        // Account wallet ownership is currently verified for Base mainnet only.
        if network != "base-mainnet" {
            return Ok(None);
        }
        let email = ReviewEmailConfig::from_env()?
            .map(ReviewEmailSender::new)
            .transpose()?;
        let push = CreatorPushSender::from_env()?;
        Ok((email.is_some() || push.is_some()).then_some(Self { email, push }))
    }
    pub async fn poll(&self, store: &PostgresStore, rpc: &str) -> anyhow::Result<()> {
        if let Some(push) = &self.push {
            push.poll_creator_open(store, "base-mainnet", rpc).await?;
        }
        let Some(sender) = &self.email else {
            return Ok(());
        };
        let reminders = crate::review_deadline_notifications::deliver_review_reminders(
            store,
            sender,
            "base-mainnet",
            crate::review_deadline_notifications::ReminderSource::CreatorOpenRpc(rpc),
        )
        .await?;
        println!(
            "{}",
            serde_json::json!({"event":"creator_open_review_reminders", "report":reminders})
        );
        let started = std::time::Instant::now();
        for _ in 0..5 {
            if started.elapsed().as_secs() >= 20 {
                break;
            }
            let Some(lease) = store
                .claim_review_notifications_for_protocol("base-mainnet", 1, 60, true)
                .await?
                .pop()
            else {
                break;
            };
            let payload = match lease.provider_payload.clone().map(Ok).unwrap_or_else(|| {
                sender.prepare_creator_open_notice(
                    &lease.recipient_email,
                    &lease.notification.bounty_contract,
                    lease.notification.round,
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
                    continue;
                }
            };
            if !source_current(
                store,
                "base-mainnet",
                &lease.notification.bounty_contract,
                rpc,
            )
            .await?
            {
                continue;
            }
            let Some(attempt) = store
                .begin_review_notification_attempt(&lease, &payload)
                .await?
            else {
                continue;
            };
            let outcome = match sender
                .send(&attempt.idempotency_key, &attempt.provider_payload)
                .await
            {
                ReviewEmailOutcome::Accepted { provider_id } => {
                    ReviewNotificationOutcome::Accepted { provider_id }
                }
                ReviewEmailOutcome::Retryable {
                    code,
                    retry_after_seconds,
                } => ReviewNotificationOutcome::Retry {
                    code: code.as_str().into(),
                    retry_after_seconds: retry_after_seconds.unwrap_or(60).clamp(30, 3600) as u32,
                },
                ReviewEmailOutcome::PermanentFailure { code } => {
                    ReviewNotificationOutcome::Failed {
                        code: code.as_str().into(),
                    }
                }
            };
            store
                .finish_review_notification_attempt(attempt.id, attempt.lease_token, outcome)
                .await?;
        }
        Ok(())
    }
}
