CREATE TABLE IF NOT EXISTS managed_stripe_checkout_operations (
    order_id UUID PRIMARY KEY REFERENCES managed_service_orders(order_id),
    operation_id UUID NOT NULL UNIQUE,
    stripe_account_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    plan JSONB NOT NULL,
    plan_digest TEXT NOT NULL CHECK (plan_digest ~ '^[0-9a-f]{64}$'),
    session_id TEXT,
    payment_intent_id TEXT,
    checkout_url TEXT,
    UNIQUE(stripe_account_id, livemode, session_id)
);

CREATE TABLE IF NOT EXISTS managed_stripe_event_inbox (
    stripe_account_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    event_id TEXT NOT NULL,
    order_id UUID NOT NULL REFERENCES managed_service_orders(order_id),
    operation_id UUID NOT NULL REFERENCES managed_stripe_checkout_operations(operation_id),
    body_sha256 TEXT NOT NULL CHECK (body_sha256 ~ '^[0-9a-f]{64}$'),
    event_fingerprint TEXT NOT NULL CHECK (event_fingerprint ~ '^[0-9a-f]{64}$'),
    event JSONB NOT NULL,
    event_created_at BIGINT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('pending','paid','failed','expired')),
    disposition TEXT NOT NULL CHECK (disposition IN ('pending_binding','processed','quarantined')),
    PRIMARY KEY(stripe_account_id, livemode, event_id)
);
CREATE INDEX IF NOT EXISTS managed_stripe_pending_events ON managed_stripe_event_inbox(order_id, disposition, event_created_at);

CREATE TABLE IF NOT EXISTS managed_stripe_paid_receipts (
    order_id UUID PRIMARY KEY REFERENCES managed_service_orders(order_id),
    stripe_account_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    payment_intent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    quote_digest TEXT NOT NULL,
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    currency TEXT NOT NULL CHECK (currency IN ('usd','mxn')),
    UNIQUE(stripe_account_id,livemode,payment_intent_id),
    UNIQUE(stripe_account_id,livemode,session_id)
);
CREATE TABLE IF NOT EXISTS managed_stripe_bank_actions (
    order_id UUID PRIMARY KEY REFERENCES managed_stripe_paid_receipts(order_id),
    action TEXT NOT NULL CHECK (action='reconcile_bank_cash'),
    state TEXT NOT NULL CHECK (state='held')
);

ALTER TABLE managed_service_actions DROP CONSTRAINT IF EXISTS managed_service_actions_state_check;
ALTER TABLE managed_service_actions ADD CONSTRAINT managed_service_actions_state_check CHECK (state IN ('held','completed'));
