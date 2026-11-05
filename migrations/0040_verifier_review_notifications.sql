CREATE TABLE IF NOT EXISTS verifier_review_contacts (
    account_id TEXT PRIMARY KEY REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    verified_email TEXT,
    verified_at TIMESTAMPTZ,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (char_length(provider) BETWEEN 1 AND 32),
    CHECK (verified_email IS NULL OR char_length(verified_email) BETWEEN 3 AND 320),
    CHECK ((verified_email IS NULL) = (verified_at IS NULL))
);

CREATE TABLE IF NOT EXISTS verifier_review_notifications (
    id UUID PRIMARY KEY,
    network TEXT NOT NULL,
    bounty_contract TEXT NOT NULL,
    bounty_id TEXT NOT NULL,
    round BIGINT NOT NULL,
    verifier_wallet TEXT NOT NULL,
    submission_log_key TEXT NOT NULL,
    submitted_at TIMESTAMPTZ NOT NULL,
    review_deadline TIMESTAMPTZ,
    source_active BOOLEAN NOT NULL DEFAULT TRUE,
    last_seen_sync_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    observed_account_id TEXT,
    recipient_account_id TEXT,
    recipient_email TEXT,
    provider_payload JSONB,
    first_attempt_at TIMESTAMPTZ,
    retry_until TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    attempt_started BOOLEAN NOT NULL DEFAULT FALSE,
    provider_id TEXT,
    accepted_at TIMESTAMPTZ,
    last_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (network, bounty_contract, round, verifier_wallet),
    CHECK (network ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
    CHECK (bounty_contract ~ '^0x[0-9a-f]{40}$'),
    CHECK (bounty_id ~ '^0x[0-9a-f]{64}$'),
    CHECK (round >= 0),
    CHECK (verifier_wallet ~ '^0x[0-9a-f]{40}$'),
    CHECK (char_length(submission_log_key) BETWEEN 1 AND 256),
    CHECK (status IN ('pending', 'leased', 'accepted', 'cancelled', 'failed')),
    CHECK ((status = 'leased') = (lease_token IS NOT NULL AND lease_expires_at IS NOT NULL)),
    CHECK (NOT attempt_started OR status = 'leased'),
    CHECK ((first_attempt_at IS NULL AND retry_until IS NULL AND recipient_account_id IS NULL
            AND recipient_email IS NULL AND provider_payload IS NULL)
        OR (first_attempt_at IS NOT NULL AND retry_until > first_attempt_at
            AND retry_until < first_attempt_at + INTERVAL '24 hours'
            AND recipient_account_id IS NOT NULL AND recipient_email IS NOT NULL
            AND provider_payload IS NOT NULL)),
    CHECK (provider_payload IS NULL OR (jsonb_typeof(provider_payload) = 'object'
        AND octet_length(provider_payload::TEXT) <= 65536)),
    CHECK (attempt_count BETWEEN 0 AND 100),
    CHECK (last_code IS NULL OR last_code ~ '^[a-z0-9_]{1,64}$'),
    CHECK (provider_id IS NULL OR char_length(provider_id) BETWEEN 1 AND 256),
    CHECK ((status = 'accepted') = (accepted_at IS NOT NULL AND provider_id IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS verifier_review_notifications_claim_idx
    ON verifier_review_notifications (network, next_attempt_at, created_at)
    WHERE source_active AND status IN ('pending', 'leased');

CREATE INDEX IF NOT EXISTS verifier_review_notifications_recipient_idx
    ON verifier_review_notifications (recipient_account_id)
    WHERE status IN ('pending', 'leased');
