-- Operator-only recovery audit. Original relay rows and first-send age are unchanged.
CREATE TABLE IF NOT EXISTS timeout_revalidation_gas_receipts (
    operation_id UUID NOT NULL REFERENCES relay_transaction_outbox(operation_id),
    tx_hash TEXT NOT NULL CHECK(tx_hash ~ '^0x[0-9a-f]{64}$'),
    budget_day DATE NOT NULL,
    reserved_micro_usdc BIGINT NOT NULL CHECK(reserved_micro_usdc > 0 AND reserved_micro_usdc <= 1000000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(operation_id,tx_hash,budget_day)
);
CREATE TABLE IF NOT EXISTS timeout_revalidation_authorizations (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES relay_transaction_outbox(operation_id),
    network TEXT NOT NULL CHECK(network IN ('base-mainnet','base-sepolia')),
    tx_hash TEXT NOT NULL CHECK(tx_hash ~ '^0x[0-9a-f]{64}$'),
    relayer_address TEXT NOT NULL,
    nonce BIGINT NOT NULL CHECK(nonce >= 0),
    timeout_scope JSONB NOT NULL CHECK(jsonb_typeof(timeout_scope)='object'),
    lease_token UUID NOT NULL,
    operator_identity TEXT NOT NULL CHECK(length(operator_identity) BETWEEN 1 AND 128),
    observations JSONB NOT NULL CHECK(jsonb_typeof(observations)='object' AND octet_length(observations::text) <= 16384),
    observed_at TIMESTAMPTZ NOT NULL,
    budget_day DATE NOT NULL,
    required_budget_micro_usdc BIGINT NOT NULL CHECK(required_budget_micro_usdc > 0 AND required_budget_micro_usdc <= 1000000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    CHECK(expires_at > created_at AND expires_at <= created_at + interval '120 seconds'),
    FOREIGN KEY(operation_id,tx_hash,budget_day) REFERENCES timeout_revalidation_gas_receipts(operation_id,tx_hash,budget_day)
);
CREATE INDEX IF NOT EXISTS timeout_revalidation_operation ON timeout_revalidation_authorizations(operation_id,created_at);
CREATE TABLE IF NOT EXISTS timeout_revalidation_events (
    authorization_id UUID PRIMARY KEY REFERENCES timeout_revalidation_authorizations(id),
    status TEXT NOT NULL CHECK(status IN ('consumed','expired','superseded')),
    observations JSONB CHECK(observations IS NULL OR (jsonb_typeof(observations)='object' AND octet_length(observations::text) <= 16384)),
    observed_at TIMESTAMPTZ,
    required_budget_micro_usdc BIGINT CHECK(required_budget_micro_usdc > 0 AND required_budget_micro_usdc <= 1000000),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK((status='consumed' AND observations IS NOT NULL AND observed_at IS NOT NULL AND required_budget_micro_usdc IS NOT NULL) OR (status<>'consumed' AND observations IS NULL AND observed_at IS NULL AND required_budget_micro_usdc IS NULL))
);
CREATE OR REPLACE RULE timeout_revalidation_authorizations_no_update AS ON UPDATE TO timeout_revalidation_authorizations DO INSTEAD NOTHING;
CREATE OR REPLACE RULE timeout_revalidation_authorizations_no_delete AS ON DELETE TO timeout_revalidation_authorizations DO INSTEAD NOTHING;
CREATE OR REPLACE RULE timeout_revalidation_events_no_update AS ON UPDATE TO timeout_revalidation_events DO INSTEAD NOTHING;
CREATE OR REPLACE RULE timeout_revalidation_events_no_delete AS ON DELETE TO timeout_revalidation_events DO INSTEAD NOTHING;
