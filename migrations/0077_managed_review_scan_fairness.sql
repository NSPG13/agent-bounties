-- Durable scheduling only. Attempt timestamps do not prove payment or observation.
-- Numbers 0070-0076 belong to the private main/collaboration migration lineage.
ALTER TABLE managed_bounty_funding_operations
    ADD COLUMN IF NOT EXISTS last_review_attempt_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS managed_funding_review_rotation
    ON managed_bounty_funding_operations ((plan->>'network'), last_review_attempt_at NULLS FIRST, order_id)
    WHERE outcome='funded';
ALTER TABLE managed_refund_operations
    ADD COLUMN IF NOT EXISTS last_observation_attempt_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS managed_refund_observation_rotation
    ON managed_refund_operations (last_observation_attempt_at NULLS FIRST, order_id)
    WHERE refund_id IS NOT NULL;
ALTER TABLE managed_deposit_refund_operations
    ADD COLUMN IF NOT EXISTS last_observation_attempt_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS managed_deposit_observation_rotation
    ON managed_deposit_refund_operations (last_observation_attempt_at NULLS FIRST, order_id)
    WHERE refund_id IS NOT NULL;
