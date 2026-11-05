-- A refund reservation permanently excludes the original payment from new funding.
ALTER TABLE managed_stripe_bank_actions DROP CONSTRAINT IF EXISTS managed_stripe_bank_actions_state_check;
ALTER TABLE managed_stripe_bank_actions ADD CONSTRAINT managed_stripe_bank_actions_state_check
    CHECK (state IN ('held','refund_reserved'));

CREATE TABLE IF NOT EXISTS managed_refund_operations (
    order_id UUID PRIMARY KEY REFERENCES managed_stripe_paid_receipts(order_id),
    operation_id UUID NOT NULL UNIQUE,
    stripe_account_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    plan JSONB NOT NULL,
    plan_digest TEXT NOT NULL CHECK (plan_digest ~ '^[0-9a-f]{64}$'),
    refund_id TEXT,
    charge_id TEXT,
    CHECK ((refund_id IS NULL) = (charge_id IS NULL)),
    UNIQUE(stripe_account_id,livemode,refund_id)
);
CREATE TABLE IF NOT EXISTS managed_refund_events (
    stripe_account_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    event_id TEXT NOT NULL,
    order_id UUID NOT NULL REFERENCES managed_refund_operations(order_id),
    operation_id UUID NOT NULL REFERENCES managed_refund_operations(operation_id),
    event JSONB NOT NULL,
    event_fingerprint TEXT NOT NULL CHECK (event_fingerprint ~ '^[0-9a-f]{64}$'),
    event_created_at BIGINT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('pending','requires_action','succeeded','failed','canceled')),
    disposition TEXT NOT NULL CHECK (disposition IN ('pending_binding','processed','quarantined')),
    PRIMARY KEY(stripe_account_id,livemode,event_id)
);
CREATE INDEX IF NOT EXISTS managed_refund_pending_events ON managed_refund_events(order_id,disposition);
CREATE TABLE IF NOT EXISTS managed_refund_execution (
    order_id UUID PRIMARY KEY REFERENCES managed_refund_operations(order_id),
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    create_attempts BIGINT NOT NULL DEFAULT 0 CHECK (create_attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_result TEXT CHECK(last_result IN ('bound','retry','reconciliation_required','configuration_error')),
    CHECK ((lease_token IS NULL) = (lease_until IS NULL))
);
