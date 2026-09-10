CREATE TABLE IF NOT EXISTS site_posting_drafts (
    account_id TEXT NOT NULL,
    operation_id UUID NOT NULL,
    draft JSONB NOT NULL,
    recovery_state JSONB NOT NULL DEFAULT '{}'::jsonb,
    draft_hash TEXT NOT NULL CHECK (draft_hash ~ '^[0-9a-f]{64}$'),
    approved_draft_hash TEXT CHECK (approved_draft_hash = draft_hash),
    revision BIGINT NOT NULL CHECK (revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, operation_id),
    CHECK (jsonb_typeof(draft) = 'object' AND octet_length(draft::text) <= 131072),
    CHECK (jsonb_typeof(recovery_state) = 'object' AND octet_length(recovery_state::text) <= 32768)
);
CREATE INDEX IF NOT EXISTS site_posting_drafts_expiry_idx ON site_posting_drafts (expires_at);
