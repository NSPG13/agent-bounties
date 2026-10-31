-- Card disputes on managed payments, bound by PaymentIntent to one paid order. Any dispute
-- stops automatic funding of that order. Later events update status only when newer.
-- This runner splits files on semicolons, so comments here must not contain one.
CREATE TABLE IF NOT EXISTS managed_stripe_disputes (
    dispute_id TEXT PRIMARY KEY CHECK (dispute_id ~ '^d[pu]_[A-Za-z0-9_]+$'),
    order_id UUID NOT NULL REFERENCES managed_stripe_paid_receipts(order_id),
    payment_intent_id TEXT NOT NULL CHECK (payment_intent_id ~ '^pi_[A-Za-z0-9_]+$'),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency TEXT NOT NULL CHECK (currency ~ '^[a-z]{3}$'),
    status TEXT NOT NULL,
    reason TEXT NOT NULL,
    last_event_id TEXT NOT NULL,
    last_event_created BIGINT NOT NULL CHECK (last_event_created > 0),
    first_recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS managed_stripe_disputes_order ON managed_stripe_disputes(order_id);
