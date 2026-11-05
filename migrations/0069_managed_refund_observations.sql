-- The latest read of each card refund's status from Stripe, beside the status its signed events
-- prove. A read never changes refund state. A mismatch tells the operator to resend the missing
-- Stripe event, which then arrives through the signed webhook. One row per refund kind and order,
-- replaced by each read. This runner splits files on semicolons, so comments here must not
-- contain one.
CREATE TABLE IF NOT EXISTS managed_refund_observations (
    kind TEXT NOT NULL CHECK (kind IN ('full','review_deposit')),
    order_id UUID NOT NULL REFERENCES managed_stripe_paid_receipts(order_id),
    refund_id TEXT NOT NULL CHECK (refund_id ~ '^re_[A-Za-z0-9_]+$'),
    stripe_status TEXT NOT NULL
        CHECK (stripe_status IN ('pending','requires_action','succeeded','failed','canceled')),
    signed_status TEXT
        CHECK (signed_status IS NULL OR signed_status IN ('pending','requires_action','succeeded','failed','canceled')),
    refund_created_at BIGINT NOT NULL CHECK (refund_created_at > 0),
    observed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (kind, order_id)
);
