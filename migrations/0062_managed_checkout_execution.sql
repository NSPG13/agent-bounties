CREATE TABLE IF NOT EXISTS managed_stripe_checkout_execution (
    order_id UUID PRIMARY KEY REFERENCES managed_stripe_checkout_operations(order_id),
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    create_attempts BIGINT NOT NULL DEFAULT 0 CHECK (create_attempts >= 0),
    last_create_at TIMESTAMPTZ,
    last_result TEXT CHECK (last_result IN ('bound','retry','reconciliation_required','configuration_error')),
    CHECK ((lease_token IS NULL) = (lease_until IS NULL)),
    CHECK ((create_attempts = 0) = (last_create_at IS NULL))
);
