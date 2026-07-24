-- Public, evidence-bound competitive intelligence. This is deliberately
-- separate from product analytics and never stores credentials, cookies,
-- authenticated content, or personal profiles.
CREATE TABLE IF NOT EXISTS competitors (
    slug TEXT PRIMARY KEY CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,62}$'),
    brand_name TEXT NOT NULL CHECK (char_length(brand_name) <= 160),
    canonical_url TEXT NOT NULL CHECK (canonical_url ~ '^https://'),
    market_type TEXT NOT NULL CHECK (market_type IN ('agent_bounty', 'github_bounty', 'social_bounty', 'general_bounty')),
    status TEXT NOT NULL CHECK (status IN ('active', 'monitoring', 'uncertain', 'inactive')),
    direct_competitor_reason TEXT NOT NULL CHECK (char_length(direct_competitor_reason) <= 1000),
    inclusion_evidence_url TEXT NOT NULL CHECK (inclusion_evidence_url ~ '^https://'),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS competitor_links (
    competitor_slug TEXT NOT NULL REFERENCES competitors(slug) ON DELETE CASCADE,
    link_kind TEXT NOT NULL CHECK (link_kind IN ('website', 'repository', 'x', 'farcaster', 'discord', 'linkedin', 'other')),
    url TEXT NOT NULL CHECK (url ~ '^https://'),
    PRIMARY KEY (competitor_slug, link_kind, url)
);

CREATE TABLE IF NOT EXISTS competitor_capabilities (
    competitor_slug TEXT NOT NULL REFERENCES competitors(slug) ON DELETE CASCADE,
    capability_key TEXT NOT NULL CHECK (capability_key ~ '^[a-z0-9][a-z0-9_-]{1,62}$'),
    description TEXT NOT NULL CHECK (char_length(description) <= 1000),
    evidence_url TEXT NOT NULL CHECK (evidence_url ~ '^https://'),
    observed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (competitor_slug, capability_key)
);

CREATE TABLE IF NOT EXISTS competitor_intelligence_runs (
    id UUID PRIMARY KEY,
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'completed_with_failures', 'failed')),
    registry_sha256 TEXT NOT NULL CHECK (registry_sha256 ~ '^[0-9a-f]{64}$'),
    report_json JSONB,
    report_markdown TEXT,
    CHECK ((status = 'running' AND completed_at IS NULL) OR (status <> 'running' AND completed_at IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS competitor_intelligence_runs_completed_idx
    ON competitor_intelligence_runs (completed_at DESC) WHERE status IN ('completed', 'completed_with_failures');

CREATE TABLE IF NOT EXISTS competitor_source_observations (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES competitor_intelligence_runs(id) ON DELETE CASCADE,
    competitor_slug TEXT NOT NULL REFERENCES competitors(slug) ON DELETE CASCADE,
    source_key TEXT NOT NULL CHECK (source_key ~ '^[a-z0-9][a-z0-9_-]{1,62}$'),
    source_url TEXT NOT NULL CHECK (source_url ~ '^https://'),
    observed_at TIMESTAMPTZ NOT NULL,
    http_status INTEGER CHECK (http_status BETWEEN 100 AND 599),
    content_sha256 TEXT CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    extracted JSONB NOT NULL DEFAULT '{}'::jsonb,
    error_kind TEXT CHECK (error_kind IN ('network', 'timeout', 'invalid_response', 'metric_ambiguous', 'database')),
    error_message TEXT CHECK (error_message IS NULL OR char_length(error_message) <= 500),
    CHECK ((error_kind IS NULL AND http_status BETWEEN 200 AND 299 AND content_sha256 IS NOT NULL) OR error_kind IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS competitor_source_observations_run_source_idx
    ON competitor_source_observations (run_id, competitor_slug, source_key);
CREATE INDEX IF NOT EXISTS competitor_source_observations_history_idx
    ON competitor_source_observations (competitor_slug, source_key, observed_at DESC);

CREATE TABLE IF NOT EXISTS competitor_metric_observations (
    source_observation_id UUID NOT NULL REFERENCES competitor_source_observations(id) ON DELETE CASCADE,
    metric_key TEXT NOT NULL CHECK (metric_key ~ '^[a-z0-9][a-z0-9_-]{1,62}$'),
    value_numeric NUMERIC,
    value_text TEXT,
    unit TEXT NOT NULL CHECK (char_length(unit) <= 80),
    evidence_url TEXT NOT NULL CHECK (evidence_url ~ '^https://'),
    PRIMARY KEY (source_observation_id, metric_key),
    CHECK ((value_numeric IS NOT NULL) <> (value_text IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS competitor_metric_observations_metric_idx
    ON competitor_metric_observations (metric_key, source_observation_id);

CREATE TABLE IF NOT EXISTS competitor_intelligence_changes (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES competitor_intelligence_runs(id) ON DELETE CASCADE,
    competitor_slug TEXT NOT NULL REFERENCES competitors(slug) ON DELETE CASCADE,
    change_kind TEXT NOT NULL CHECK (change_kind IN ('source_changed', 'metric_changed', 'source_recovered', 'source_failed')),
    field_path TEXT NOT NULL CHECK (char_length(field_path) <= 240),
    previous_value JSONB,
    current_value JSONB,
    evidence_url TEXT NOT NULL CHECK (evidence_url ~ '^https://'),
    detected_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS competitor_intelligence_changes_run_idx
    ON competitor_intelligence_changes (run_id, competitor_slug, change_kind);
