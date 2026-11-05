-- Customer acceptance is neither payment nor canonical bounty funding.
-- Additive tables intentionally survive application rollback.
CREATE TABLE IF NOT EXISTS managed_service_quotes (
    quote_id UUID PRIMARY KEY,
    buyer_principal TEXT NOT NULL CHECK (length(buyer_principal) BETWEEN 1 AND 200),
    quote_digest TEXT NOT NULL CHECK (quote_digest ~ '^[0-9a-f]{64}$'),
    quote JSONB NOT NULL CHECK (jsonb_typeof(quote) = 'object'),
    issued_at BIGINT NOT NULL CHECK (issued_at > 0),
    expires_at BIGINT NOT NULL CHECK (expires_at > issued_at),
    UNIQUE (quote_id, buyer_principal, quote_digest)
);

CREATE TABLE IF NOT EXISTS managed_service_orders (
    order_id UUID PRIMARY KEY,
    quote_id UUID NOT NULL UNIQUE,
    buyer_principal TEXT NOT NULL,
    quote_digest TEXT NOT NULL,
    accepted_at BIGINT NOT NULL CHECK (accepted_at > 0),
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 200),
    UNIQUE (buyer_principal, idempotency_key),
    FOREIGN KEY (quote_id, buyer_principal, quote_digest)
        REFERENCES managed_service_quotes (quote_id, buyer_principal, quote_digest)
);

-- No worker can execute this action in the current slice.
CREATE TABLE IF NOT EXISTS managed_service_actions (
    order_id UUID PRIMARY KEY REFERENCES managed_service_orders (order_id),
    quote_digest TEXT NOT NULL CHECK (quote_digest ~ '^[0-9a-f]{64}$'),
    action TEXT NOT NULL CHECK (action = 'prepare_checkout'),
    state TEXT NOT NULL CHECK (state = 'held')
);
