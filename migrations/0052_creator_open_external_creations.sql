CREATE TABLE IF NOT EXISTS creator_open_external_creations (
    id UUID PRIMARY KEY,
    network TEXT NOT NULL CHECK(network IN ('base-mainnet','base-sepolia')),
    bounty_contract TEXT NOT NULL,
    transaction_hash TEXT NOT NULL,
    unsigned_request JSONB NOT NULL CHECK(jsonb_typeof(unsigned_request)='object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(network,bounty_contract)
);
ALTER TABLE creator_open_indexes ALTER COLUMN request_hash DROP NOT NULL;
ALTER TABLE creator_open_indexes ADD COLUMN IF NOT EXISTS external_creation_id UUID REFERENCES creator_open_external_creations(id);
ALTER TABLE creator_open_indexes DROP CONSTRAINT IF EXISTS creator_open_index_source;
ALTER TABLE creator_open_indexes ADD CONSTRAINT creator_open_index_source CHECK ((request_hash IS NULL) <> (external_creation_id IS NULL));
