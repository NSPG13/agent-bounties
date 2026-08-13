CREATE TABLE IF NOT EXISTS external_interface_usage_hourly (
  bucket_started_at TIMESTAMPTZ NOT NULL,
  interface TEXT NOT NULL,
  protocol_era TEXT NOT NULL,
  request_count BIGINT NOT NULL DEFAULT 0,
  successful_request_count BIGINT NOT NULL DEFAULT 0,
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (bucket_started_at, interface, protocol_era),
  CONSTRAINT external_interface_usage_interface_check CHECK (
    interface IN ('api', 'cli', 'mcp')
  ),
  CONSTRAINT external_interface_usage_protocol_era_check CHECK (
    protocol_era IN ('not_applicable', 'legacy', 'modern', 'http_adapter')
  ),
  CONSTRAINT external_interface_usage_pair_check CHECK (
    (interface IN ('api', 'cli') AND protocol_era = 'not_applicable')
    OR (interface = 'mcp' AND protocol_era <> 'not_applicable')
  ),
  CONSTRAINT external_interface_usage_counts_check CHECK (
    request_count >= 0
    AND successful_request_count >= 0
    AND successful_request_count <= request_count
  ),
  CONSTRAINT external_interface_usage_observation_order_check CHECK (
    first_observed_at <= last_observed_at
  )
);

CREATE INDEX IF NOT EXISTS external_interface_usage_hourly_recent_idx
  ON external_interface_usage_hourly (bucket_started_at DESC, interface, protocol_era);

COMMENT ON TABLE external_interface_usage_hourly IS
  'Privacy-minimized hourly external request counters. Requests bearing a verified analytics-exclusion credential are never inserted.';
