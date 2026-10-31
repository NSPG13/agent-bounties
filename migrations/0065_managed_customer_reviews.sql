-- The paying customer's verdict per canonical round of a funded card bounty, stored with the
-- platform review signature over it. One verdict per order and round, never replaced. Only an
-- order with a funding operation can hold one. This runner splits files on semicolons, so
-- comments here must not contain one.
CREATE TABLE IF NOT EXISTS managed_customer_reviews (
    order_id UUID NOT NULL REFERENCES managed_bounty_funding_operations(order_id),
    round BIGINT NOT NULL CHECK (round > 0),
    verdict JSONB NOT NULL CHECK (jsonb_typeof(verdict) = 'object'),
    response_hash TEXT NOT NULL CHECK (response_hash ~ '^0x[0-9a-f]{64}$'),
    verifier TEXT NOT NULL CHECK (verifier ~ '^0x[0-9a-f]{40}$'),
    deadline BIGINT NOT NULL CHECK (deadline > 0),
    signature TEXT NOT NULL CHECK (signature ~ '^0x[0-9a-f]{130}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (order_id, round)
);
