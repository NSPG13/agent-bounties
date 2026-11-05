-- The refundable review deposit of a card bounty. After a canonical BountySettled with no
-- reversed rejection, a partial refund of exactly the deposit line goes to the original payment
-- through the same fenced execution and signed-event reconciliation as full refunds, in tables
-- of its own. A reversed rejection instead records a payout obligation to the solver in the
-- ruling's transaction. It is paid only by a verified USDC transfer whose hash can settle one
-- obligation. This runner splits files on semicolons, so comments here must not contain one.
CREATE TABLE IF NOT EXISTS managed_deposit_refund_operations (
    order_id UUID PRIMARY KEY REFERENCES managed_bounty_funding_operations(order_id),
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
CREATE TABLE IF NOT EXISTS managed_deposit_refund_events (
    stripe_account_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    event_id TEXT NOT NULL,
    order_id UUID NOT NULL REFERENCES managed_deposit_refund_operations(order_id),
    operation_id UUID NOT NULL REFERENCES managed_deposit_refund_operations(operation_id),
    event JSONB NOT NULL,
    event_fingerprint TEXT NOT NULL CHECK (event_fingerprint ~ '^[0-9a-f]{64}$'),
    event_created_at BIGINT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('pending','requires_action','succeeded','failed','canceled')),
    disposition TEXT NOT NULL CHECK (disposition IN ('pending_binding','processed','quarantined')),
    PRIMARY KEY(stripe_account_id,livemode,event_id)
);
CREATE INDEX IF NOT EXISTS managed_deposit_refund_pending_events
    ON managed_deposit_refund_events(order_id,disposition);
CREATE TABLE IF NOT EXISTS managed_deposit_refund_execution (
    order_id UUID PRIMARY KEY REFERENCES managed_deposit_refund_operations(order_id),
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    create_attempts BIGINT NOT NULL DEFAULT 0 CHECK (create_attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_result TEXT CHECK(last_result IN ('bound','retry','reconciliation_required','configuration_error')),
    CHECK ((lease_token IS NULL) = (lease_until IS NULL))
);
CREATE TABLE IF NOT EXISTS managed_deposit_payouts (
    order_id UUID PRIMARY KEY REFERENCES managed_bounty_funding_operations(order_id),
    round BIGINT NOT NULL CHECK (round > 0),
    network TEXT NOT NULL CHECK (network IN ('base-mainnet','base-sepolia')),
    solver TEXT NOT NULL CHECK (solver ~ '^0x[0-9a-f]{40}$'),
    amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc > 0),
    tx_hash TEXT UNIQUE CHECK (tx_hash IS NULL OR tx_hash ~ '^0x[0-9a-f]{64}$'),
    block_number BIGINT CHECK (block_number IS NULL OR block_number >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    paid_at TIMESTAMPTZ,
    FOREIGN KEY (order_id, round) REFERENCES managed_customer_reviews(order_id, round),
    CHECK ((tx_hash IS NULL) = (block_number IS NULL)),
    CHECK ((tx_hash IS NULL) = (paid_at IS NULL))
);
