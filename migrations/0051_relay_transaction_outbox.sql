CREATE TABLE IF NOT EXISTS relay_transaction_outbox (
    operation_id UUID PRIMARY KEY,
    network TEXT NOT NULL CHECK(network IN ('base-mainnet','base-sepolia')),
    relayer_address TEXT NOT NULL,
    nonce BIGINT NOT NULL CHECK(nonce>=0),
    tx_hash TEXT NOT NULL UNIQUE,
    signed_transaction TEXT NOT NULL,
    transaction_metadata JSONB NOT NULL,
    send_attempts INTEGER NOT NULL DEFAULT 0 CHECK(send_attempts BETWEEN 0 AND 3),
    next_send_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(network,relayer_address,nonce)
);
