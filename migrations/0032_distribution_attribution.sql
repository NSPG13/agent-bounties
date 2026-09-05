CREATE TABLE IF NOT EXISTS distribution_acquisitions (
  id UUID PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  first_touch_rail TEXT NOT NULL,
  measurement_eligible BOOLEAN NOT NULL DEFAULT TRUE,
  canary_kind TEXT,
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  observation_count BIGINT NOT NULL DEFAULT 1,
  CONSTRAINT distribution_acquisition_token_hash_check CHECK (
    token_hash ~ '^[a-f0-9]{64}$'
  ),
  CONSTRAINT distribution_acquisition_rail_check CHECK (
    first_touch_rail IN (
      'bankr', 'github', 'linear', 'vscode', 'cursor', 'cline',
      'openclaw', 'claude-custom', 'chatgpt-dev', 'glama',
      'mcp-so', 'mcpservers'
    )
  ),
  CONSTRAINT distribution_acquisition_canary_check CHECK (
    canary_kind IS NULL OR canary_kind IN ('dry-run-v1', 'mainnet-v1')
  ),
  CONSTRAINT distribution_acquisition_measurement_check CHECK (
    (measurement_eligible = TRUE AND canary_kind IS NULL)
    OR (measurement_eligible = FALSE AND canary_kind IS NOT NULL)
  ),
  CONSTRAINT distribution_acquisition_observation_check CHECK (
    first_observed_at <= last_observed_at AND observation_count > 0
  )
);

CREATE INDEX IF NOT EXISTS distribution_acquisitions_rail_observed_idx
  ON distribution_acquisitions (first_touch_rail, first_observed_at DESC);

CREATE TABLE IF NOT EXISTS distribution_acquisition_assists (
  acquisition_id UUID NOT NULL REFERENCES distribution_acquisitions(id) ON DELETE CASCADE,
  rail TEXT NOT NULL,
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  observation_count BIGINT NOT NULL DEFAULT 1,
  PRIMARY KEY (acquisition_id, rail),
  CONSTRAINT distribution_assist_rail_check CHECK (
    rail IN (
      'bankr', 'github', 'linear', 'vscode', 'cursor', 'cline',
      'openclaw', 'claude-custom', 'chatgpt-dev', 'glama',
      'mcp-so', 'mcpservers'
    )
  ),
  CONSTRAINT distribution_assist_observation_check CHECK (
    first_observed_at <= last_observed_at AND observation_count > 0
  )
);

CREATE INDEX IF NOT EXISTS distribution_assists_rail_observed_idx
  ON distribution_acquisition_assists (rail, first_observed_at DESC);

CREATE TABLE IF NOT EXISTS distribution_acquisition_handoffs (
  id UUID PRIMARY KEY,
  acquisition_id UUID NOT NULL REFERENCES distribution_acquisitions(id) ON DELETE CASCADE,
  request_fingerprint TEXT NOT NULL,
  terms_hash TEXT,
  creator_wallet TEXT,
  prepared_at TIMESTAMPTZ NOT NULL,
  wallet_reviewed_at TIMESTAMPTZ,
  terms_bound_at TIMESTAMPTZ,
  CONSTRAINT distribution_handoff_fingerprint_check CHECK (
    request_fingerprint ~ '^[a-f0-9]{64}$'
  ),
  CONSTRAINT distribution_handoff_terms_hash_check CHECK (
    terms_hash IS NULL OR terms_hash ~ '^0x[a-f0-9]{64}$'
  ),
  CONSTRAINT distribution_handoff_creator_check CHECK (
    creator_wallet IS NULL OR creator_wallet ~ '^0x[a-f0-9]{40}$'
  ),
  CONSTRAINT distribution_handoff_binding_check CHECK (
    (terms_hash IS NULL AND creator_wallet IS NULL AND terms_bound_at IS NULL)
    OR (terms_hash IS NOT NULL AND creator_wallet IS NOT NULL AND terms_bound_at IS NOT NULL)
  ),
  UNIQUE (acquisition_id, request_fingerprint)
);

CREATE UNIQUE INDEX IF NOT EXISTS distribution_handoffs_terms_hash_idx
  ON distribution_acquisition_handoffs (terms_hash)
  WHERE terms_hash IS NOT NULL;

CREATE TABLE IF NOT EXISTS distribution_handoff_failures (
  acquisition_id UUID NOT NULL REFERENCES distribution_acquisitions(id) ON DELETE CASCADE,
  request_fingerprint TEXT NOT NULL,
  failure_code TEXT NOT NULL,
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  observation_count BIGINT NOT NULL DEFAULT 1,
  PRIMARY KEY (acquisition_id, request_fingerprint, failure_code),
  CONSTRAINT distribution_handoff_failure_fingerprint_check CHECK (
    request_fingerprint ~ '^[a-f0-9]{64}$'
  ),
  CONSTRAINT distribution_handoff_failure_code_check CHECK (
    failure_code IN ('invalid_arguments', 'preparation_failed')
  ),
  CONSTRAINT distribution_handoff_failure_observation_check CHECK (
    first_observed_at <= last_observed_at AND observation_count > 0
  )
);

CREATE TABLE IF NOT EXISTS distribution_rail_usage_hourly (
  bucket_started_at TIMESTAMPTZ NOT NULL,
  rail TEXT NOT NULL,
  request_count BIGINT NOT NULL DEFAULT 0,
  successful_request_count BIGINT NOT NULL DEFAULT 0,
  excluded_request_count BIGINT NOT NULL DEFAULT 0,
  excluded_successful_request_count BIGINT NOT NULL DEFAULT 0,
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (bucket_started_at, rail),
  CONSTRAINT distribution_rail_usage_rail_check CHECK (
    rail IN (
      'bankr', 'github', 'linear', 'vscode', 'cursor', 'cline',
      'openclaw', 'claude-custom', 'chatgpt-dev', 'glama',
      'mcp-so', 'mcpservers'
    )
  ),
  CONSTRAINT distribution_rail_usage_counts_check CHECK (
    request_count >= 0
    AND successful_request_count >= 0
    AND successful_request_count <= request_count
    AND excluded_request_count >= 0
    AND excluded_request_count <= request_count
    AND excluded_successful_request_count >= 0
    AND excluded_successful_request_count <= successful_request_count
    AND excluded_successful_request_count <= excluded_request_count
  ),
  CONSTRAINT distribution_rail_usage_observation_check CHECK (
    first_observed_at <= last_observed_at
  )
);

CREATE TABLE IF NOT EXISTS distribution_wallet_exclusions (
  wallet_address TEXT NOT NULL,
  exclusion_class TEXT NOT NULL,
  reason TEXT,
  active BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (wallet_address, exclusion_class),
  CONSTRAINT distribution_wallet_exclusion_address_check CHECK (
    wallet_address ~ '^0x[a-f0-9]{40}$'
  ),
  CONSTRAINT distribution_wallet_exclusion_class_check CHECK (
    exclusion_class IN (
      'maintainer', 'operator', 'test', 'synthetic_canary', 'sponsored',
      'circular_funding', 'related_party', 'operator_funded_development'
    )
  ),
  CONSTRAINT distribution_wallet_exclusion_reason_check CHECK (
    reason IS NULL OR length(reason) <= 500
  )
);

CREATE INDEX IF NOT EXISTS distribution_wallet_exclusions_active_idx
  ON distribution_wallet_exclusions (exclusion_class, wallet_address)
  WHERE active = TRUE;

COMMENT ON TABLE distribution_acquisitions IS
  'Operator-only opaque acquisition records. token_hash is analytics-only and grants no wallet, payment, verification, or settlement authority. Explicit bounded canary markers are retained and excluded from marketing metrics.';

COMMENT ON TABLE distribution_acquisition_handoffs IS
  'Retry-safe attribution joins from an MCP-prepared review handoff through durable wallet-review acknowledgement to immutable published terms. Canonical events remain the only funding and settlement evidence.';

COMMENT ON TABLE distribution_handoff_failures IS
  'Idempotent attributed prepare-failure signals. Stores only an opaque acquisition key, request digest, bounded failure code, timestamps, and count, never prompt or task content.';

COMMENT ON TABLE distribution_rail_usage_hourly IS
  'Privacy-minimized rail request totals. No IP address, user agent, prompt, wallet, task body, or raw acquisition token is stored.';

COMMENT ON TABLE distribution_wallet_exclusions IS
  'Operator-managed wallet classifications excluded from external acquisition outcomes. Classification is analytics policy, never identity or payment evidence.';
