CREATE TABLE IF NOT EXISTS wallet_topup_attempts (
    id UUID PRIMARY KEY,
    account_id TEXT NOT NULL,
    operation_id UUID NOT NULL,
    wallet TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('coinbase', 'moonpay', 'metamask')),
    request_fingerprint TEXT NOT NULL,
    context JSONB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('preparing', 'prepared', 'pending', 'unknown', 'completed', 'cancelled', 'failed')),
    quote JSONB,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS wallet_topup_one_unresolved_operation
    ON wallet_topup_attempts (account_id, operation_id)
    WHERE status IN ('preparing', 'prepared', 'pending', 'unknown');
CREATE UNIQUE INDEX IF NOT EXISTS wallet_topup_one_unresolved_wallet
    ON wallet_topup_attempts (account_id, wallet)
    WHERE status IN ('preparing', 'prepared', 'pending', 'unknown');
CREATE TABLE IF NOT EXISTS posting_creation_relays (
    account_id TEXT NOT NULL,
    operation_id UUID NOT NULL,
    relay_id UUID NOT NULL UNIQUE,
    draft_hash TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    create_request JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, operation_id)
);
-- A signed transaction is retained before its first submission. Recovery may
-- reconcile this hash, but cannot allocate another nonce or transaction.
CREATE TABLE IF NOT EXISTS posting_relay_transactions (
    relay_id UUID PRIMARY KEY REFERENCES x402_relay_attempts(id),
    network TEXT NOT NULL,
    relayer_address TEXT NOT NULL,
    nonce BIGINT NOT NULL,
    tx_hash TEXT NOT NULL UNIQUE,
    signed_transaction TEXT NOT NULL,
    transaction_metadata JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (network, relayer_address, nonce)
);
