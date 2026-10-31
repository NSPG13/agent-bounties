CREATE TABLE IF NOT EXISTS autonomous_canonical_checkpoints (
    network TEXT NOT NULL,
    factory_contract TEXT NOT NULL,
    block_number BIGINT NOT NULL CHECK (block_number >= 0),
    block_hash TEXT NOT NULL CHECK (block_hash ~ '^0x[0-9a-f]{64}$'),
    invalidated BOOLEAN NOT NULL DEFAULT false,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (network, factory_contract)
)
