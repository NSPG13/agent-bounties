CREATE TABLE IF NOT EXISTS creator_open_indexes (
    network TEXT NOT NULL,
    bounty_contract TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    version BIGINT NOT NULL DEFAULT 0,
    checkpoint JSONB,
    logs JSONB NOT NULL DEFAULT '[]'::jsonb,
    projection JSONB,
    available BOOLEAN NOT NULL DEFAULT false,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(network,bounty_contract),
    FOREIGN KEY(network,request_hash) REFERENCES creator_open_requests(network,request_hash)
);
CREATE INDEX IF NOT EXISTS creator_open_index_poll ON creator_open_indexes(network,checked_at);
