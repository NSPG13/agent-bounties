CREATE TABLE IF NOT EXISTS github_origin_installations (
    installation_id BIGINT PRIMARY KEY CHECK(installation_id>0),
    disabled BOOLEAN NOT NULL DEFAULT FALSE,
    retired BOOLEAN NOT NULL DEFAULT FALSE,
    source_updated_at TIMESTAMPTZ NOT NULL DEFAULT '-infinity',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS github_origin_drafts (
    id UUID PRIMARY KEY,
    installation_id BIGINT NOT NULL REFERENCES github_origin_installations(installation_id),
    requester_github_id TEXT NOT NULL CHECK(requester_github_id ~ '^[1-9][0-9]{0,19}$'),
    delivery_id TEXT NOT NULL UNIQUE,
    event_key TEXT NOT NULL UNIQUE,
    body_sha256 TEXT NOT NULL CHECK(body_sha256 ~ '^[0-9a-f]{64}$'),
    plan JSONB NOT NULL CHECK(jsonb_typeof(plan)='object' AND octet_length(plan::TEXT)<=131072),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '7 days'
);
CREATE INDEX IF NOT EXISTS github_origin_drafts_owner ON github_origin_drafts(requester_github_id,received_at DESC);
