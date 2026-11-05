-- Additive evidence records. None of these tables is a settlement oracle.
CREATE TABLE IF NOT EXISTS verification_artifacts (
    id UUID PRIMARY KEY,
    owner_key TEXT NOT NULL,
    request_key TEXT NOT NULL,
    manifest_hash TEXT NOT NULL,
    manifest JSONB NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes BETWEEN 0 AND 104857600),
    status TEXT NOT NULL DEFAULT 'uploading' CHECK (status IN ('uploading','ready','deleting','deleted')),
    canonical_scope JSONB,
    winning BOOLEAN NOT NULL DEFAULT FALSE,
    dispute_hold BOOLEAN NOT NULL DEFAULT FALSE,
    retain_until TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '7 days',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(owner_key, request_key),
    CHECK (manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (octet_length(manifest::TEXT) <= 1048576),
    CHECK (NOT winning OR canonical_scope IS NOT NULL)
);
CREATE INDEX IF NOT EXISTS verification_artifacts_retention_idx ON verification_artifacts(retain_until)
    WHERE NOT winning AND NOT dispute_hold AND status <> 'deleted';

CREATE TABLE IF NOT EXISTS verification_plans (
    plan_hash TEXT NOT NULL,
    owner_key TEXT NOT NULL,
    document JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_key, plan_hash),
    CHECK (plan_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (octet_length(document::TEXT) <= 1048576)
);
CREATE TABLE IF NOT EXISTS verification_runs (
    id UUID PRIMARY KEY,
    owner_key TEXT NOT NULL,
    artifact_id UUID NOT NULL REFERENCES verification_artifacts(id),
    plan_hash TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','completed','unavailable')),
    report JSONB,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(owner_key, artifact_id, plan_hash),
    FOREIGN KEY (owner_key, plan_hash) REFERENCES verification_plans(owner_key, plan_hash),
    CHECK (report IS NULL OR octet_length(report::TEXT) <= 1048576)
);
CREATE TABLE IF NOT EXISTS verification_review_reports (
    id UUID PRIMARY KEY,
    owner_key TEXT NOT NULL,
    network TEXT NOT NULL,
    bounty_contract TEXT NOT NULL,
    round BIGINT NOT NULL CHECK (round >= 0),
    reviewer TEXT NOT NULL,
    assessment_hash TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    assessment JSONB NOT NULL,
    signed_attestation JSONB,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','signed','submitted','confirmed')),
    transaction_hash TEXT,
    canonical_event_id TEXT,
    previous_report_id UUID REFERENCES verification_review_reports(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(network,bounty_contract,round,reviewer,assessment_hash,request_hash),
    CHECK (octet_length(assessment::TEXT) <= 262144),
    CHECK ((status='confirmed') = (canonical_event_id IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS verifier_review_deliveries (
    notification_id UUID NOT NULL REFERENCES verifier_review_notifications(id),
    stage TEXT NOT NULL CHECK (stage IN ('initial','remaining_24h','remaining_6h','remaining_1h','overdue')),
    due_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    provider_id TEXT,
    provider_payload JSONB,
    recipient_email TEXT,
    first_attempt_at TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(notification_id,stage)
);

CREATE TABLE IF NOT EXISTS verification_report_events (
    id UUID PRIMARY KEY,
    report_id UUID NOT NULL REFERENCES verification_review_reports(id),
    phase TEXT NOT NULL CHECK (phase IN ('draft','signed','submitted','confirmed')),
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(report_id,phase)
);
ALTER TABLE verification_review_reports ADD COLUMN IF NOT EXISTS request JSONB;
ALTER TABLE verification_artifacts ADD COLUMN IF NOT EXISTS closed_at TIMESTAMPTZ;
ALTER TABLE verification_artifacts ADD COLUMN IF NOT EXISTS pending_scope JSONB;

CREATE TABLE IF NOT EXISTS verification_review_assignments (
    notification_id UUID PRIMARY KEY REFERENCES verifier_review_notifications(id),
    assigned_by TEXT NOT NULL,
    assignee_key TEXT NOT NULL,
    accepted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (length(assignee_key) BETWEEN 1 AND 200)
);

ALTER TABLE verification_artifacts ADD COLUMN IF NOT EXISTS source JSONB;
