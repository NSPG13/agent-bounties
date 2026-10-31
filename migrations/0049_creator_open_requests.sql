CREATE TABLE IF NOT EXISTS creator_open_requests (
    network TEXT NOT NULL CHECK(network IN ('base-mainnet','base-sepolia')),
    request_hash TEXT NOT NULL,
    bounty_contract TEXT NOT NULL,
    action_id UUID NOT NULL UNIQUE REFERENCES sponsored_actions(id),
    unsigned_request JSONB NOT NULL CHECK(jsonb_typeof(unsigned_request)='object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(network, request_hash)
);
CREATE INDEX IF NOT EXISTS creator_open_request_contract ON creator_open_requests(network,bounty_contract,created_at);
