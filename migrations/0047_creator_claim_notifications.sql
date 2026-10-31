ALTER TABLE verifier_review_notifications ADD COLUMN IF NOT EXISTS notification_kind TEXT NOT NULL DEFAULT 'submission' CHECK (notification_kind IN ('submission','claim'));
ALTER TABLE verifier_review_notifications DROP CONSTRAINT IF EXISTS verifier_review_notifications_network_bounty_contract_round_key;
CREATE UNIQUE INDEX IF NOT EXISTS verifier_review_notifications_kind_identity ON verifier_review_notifications (network,bounty_contract,round,verifier_wallet,notification_kind);
