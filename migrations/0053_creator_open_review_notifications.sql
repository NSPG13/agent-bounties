ALTER TABLE verifier_review_notifications DROP CONSTRAINT IF EXISTS verifier_review_notifications_notification_kind_check;
ALTER TABLE verifier_review_notifications ADD CONSTRAINT verifier_review_notifications_notification_kind_check CHECK(notification_kind IN ('submission','claim','creator_open_submission'));
DROP INDEX IF EXISTS verifier_review_notifications_kind_identity;
CREATE UNIQUE INDEX verifier_review_notifications_kind_identity ON verifier_review_notifications(network,bounty_contract,round,verifier_wallet,notification_kind) WHERE notification_kind IN ('submission','claim');
CREATE UNIQUE INDEX IF NOT EXISTS creator_open_review_identity ON verifier_review_notifications(network,bounty_contract,round,verifier_wallet,notification_kind,submission_log_key) WHERE notification_kind='creator_open_submission';
CREATE OR REPLACE VIEW creator_open_review_sources AS
SELECT i.network,i.bounty_contract,i.projection->>'bounty_id' AS bounty_id,
       e.key::BIGINT AS round,i.projection->>'creator' AS verifier_wallet,
       'creator-open:'||(e.value->>'submission_hash')||':'||(e.value->>'evidence_hash')||':'||(e.value->>'review_deadline') AS submission_log_key,
       to_timestamp((e.value->>'review_deadline')::BIGINT-(i.projection->>'review_window')::BIGINT) AS submitted_at,
       to_timestamp((e.value->>'review_deadline')::BIGINT) AS review_deadline
FROM creator_open_indexes i CROSS JOIN LATERAL jsonb_each(i.projection->'entries') e
WHERE i.available AND i.checked_at>statement_timestamp()-interval '120 seconds'
  AND i.checkpoint IS NOT NULL AND i.projection->>'protocol'='agent-bounties/creator-open-v1'
  AND i.projection->>'status'='open' AND e.value->>'status'='pending'
  AND to_timestamp((e.value->>'review_deadline')::BIGINT)>statement_timestamp();
