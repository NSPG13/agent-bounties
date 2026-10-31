-- Funding reserves a held paid order exactly once. The bank action keeps state 'held' and records
-- the funding operation. A row-level check makes refund reservation (a state change) impossible
-- once funding is reserved, and funding cannot reserve a refund-reserved order. Replays are
-- idempotent and leave the 0063 state constraint valid. Only canonical FundingAdded marks funding.
-- This runner splits files on semicolons, so comments here must not contain one.
CREATE TABLE IF NOT EXISTS managed_bounty_funding_operations (
    order_id UUID PRIMARY KEY REFERENCES managed_stripe_paid_receipts(order_id),
    operation_id UUID NOT NULL UNIQUE,
    plan JSONB NOT NULL CHECK (jsonb_typeof(plan) = 'object'),
    plan_digest TEXT NOT NULL CHECK (plan_digest ~ '^[0-9a-f]{64}$'),
    reserved_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    tx_hash TEXT CHECK (tx_hash IS NULL OR tx_hash ~ '^0x[0-9a-f]{64}$'),
    outcome TEXT NOT NULL DEFAULT 'reserved'
        CHECK (outcome IN ('reserved','broadcast','funded','reconciliation_required')),
    funded_tx_hash TEXT CHECK (funded_tx_hash IS NULL OR funded_tx_hash ~ '^0x[0-9a-f]{64}$'),
    funded_block BIGINT CHECK (funded_block IS NULL OR funded_block >= 0),
    CHECK ((lease_token IS NULL) = (lease_until IS NULL)),
    CHECK ((outcome = 'funded') = (funded_tx_hash IS NOT NULL AND funded_block IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS managed_bounty_funding_open
    ON managed_bounty_funding_operations(outcome) WHERE outcome IN ('reserved','broadcast');

ALTER TABLE managed_stripe_bank_actions
    ADD COLUMN IF NOT EXISTS funding_operation_id UUID UNIQUE
    REFERENCES managed_bounty_funding_operations(operation_id);
ALTER TABLE managed_stripe_bank_actions DROP CONSTRAINT IF EXISTS managed_bank_action_funding_check;
ALTER TABLE managed_stripe_bank_actions ADD CONSTRAINT managed_bank_action_funding_check
    CHECK (funding_operation_id IS NULL OR state = 'held');
