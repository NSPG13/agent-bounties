-- Immutable recovery evidence. Old signed operations remain inspectable.
CREATE TABLE IF NOT EXISTS posting_authorization_retirements (
    account_id TEXT NOT NULL,
    operation_id UUID NOT NULL,
    replacement_operation_id UUID NOT NULL,
    source_revision BIGINT NOT NULL CHECK (source_revision > 0),
    continuation_hash TEXT NOT NULL,
    chain_proof JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, operation_id),
    UNIQUE (account_id, replacement_operation_id),
    CHECK (operation_id <> replacement_operation_id),
    CHECK (continuation_hash ~ '^0x[0-9a-f]{64}$'),
    CHECK (jsonb_typeof(chain_proof) = 'object')
);
