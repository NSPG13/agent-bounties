CREATE TABLE IF NOT EXISTS interface_usage_hourly (
  bucket_started_at TIMESTAMPTZ NOT NULL,
  interface TEXT NOT NULL,
  protocol_era TEXT NOT NULL,
  request_count BIGINT NOT NULL DEFAULT 0 CHECK (request_count >= 0),
  successful_request_count BIGINT NOT NULL DEFAULT 0 CHECK (
    successful_request_count >= 0
    AND successful_request_count <= request_count
  ),
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (bucket_started_at, interface, protocol_era),
  CONSTRAINT interface_usage_interface_check CHECK (
    interface IN ('api', 'cli', 'mcp')
  ),
  CONSTRAINT interface_usage_protocol_era_check CHECK (
    protocol_era IN ('not_applicable', 'legacy', 'modern', 'http_adapter')
  ),
  CONSTRAINT interface_usage_interface_era_check CHECK (
    (interface IN ('api', 'cli') AND protocol_era = 'not_applicable')
    OR (interface = 'mcp' AND protocol_era <> 'not_applicable')
  ),
  CONSTRAINT interface_usage_bucket_check CHECK (
    bucket_started_at = (
      date_trunc('hour', bucket_started_at AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'
    )
  ),
  CONSTRAINT interface_usage_observation_order_check CHECK (
    first_observed_at <= last_observed_at
  )
);

CREATE INDEX IF NOT EXISTS interface_usage_hourly_recent_idx
  ON interface_usage_hourly (bucket_started_at DESC, interface, protocol_era);
