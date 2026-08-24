CREATE TABLE IF NOT EXISTS discoverability_snapshots (
  provider TEXT NOT NULL,
  observed_at TIMESTAMPTZ NOT NULL,
  window_started_at TIMESTAMPTZ NOT NULL,
  window_ended_at TIMESTAMPTZ NOT NULL,
  data_through TIMESTAMPTZ NOT NULL,
  payload_checksum TEXT NOT NULL,
  payload JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (
    provider,
    observed_at,
    window_started_at,
    window_ended_at,
    payload_checksum
  ),
  CONSTRAINT discoverability_snapshot_provider_check CHECK (
    provider IN ('search_console', 'github', 'first_party', 'external_interfaces')
  ),
  CONSTRAINT discoverability_snapshot_window_check CHECK (
    window_started_at <= window_ended_at
    AND data_through >= window_started_at
    AND data_through <= window_ended_at
    AND window_ended_at <= observed_at
  ),
  CONSTRAINT discoverability_snapshot_checksum_check CHECK (
    payload_checksum ~ '^[a-f0-9]{64}$'
  ),
  CONSTRAINT discoverability_snapshot_payload_check CHECK (
    jsonb_typeof(payload) = 'object'
  )
);

CREATE INDEX IF NOT EXISTS discoverability_snapshots_recent_idx
  ON discoverability_snapshots (provider, observed_at DESC, data_through DESC);

COMMENT ON TABLE discoverability_snapshots IS
  'Operator-only provider snapshots retained for discoverability trend reporting. Raw queries, paths, and referrers must never be projected by the public summary.';

CREATE TABLE IF NOT EXISTS discovery_route_usage_hourly (
  bucket_started_at TIMESTAMPTZ NOT NULL,
  interface TEXT NOT NULL,
  route_family TEXT NOT NULL,
  attribution_reliability TEXT NOT NULL,
  interaction_count BIGINT NOT NULL DEFAULT 0,
  successful_interaction_count BIGINT NOT NULL DEFAULT 0,
  first_observed_at TIMESTAMPTZ NOT NULL,
  last_observed_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (bucket_started_at, interface, route_family, attribution_reliability),
  CONSTRAINT discovery_route_usage_interface_check CHECK (
    interface IN ('a2a', 'mcp', 'api', 'cli', 'feed')
  ),
  CONSTRAINT discovery_route_usage_family_check CHECK (
    route_family IN (
      'agent_card',
      'opportunity_list',
      'opportunity_detail',
      'alerts',
      'protocol_orientation'
    )
  ),
  CONSTRAINT discovery_route_usage_reliability_check CHECK (
    attribution_reliability IN ('observed', 'declared')
  ),
  CONSTRAINT discovery_route_usage_counts_check CHECK (
    interaction_count >= 0
    AND successful_interaction_count >= 0
    AND successful_interaction_count <= interaction_count
  ),
  CONSTRAINT discovery_route_usage_observation_order_check CHECK (
    first_observed_at <= last_observed_at
  )
);

CREATE INDEX IF NOT EXISTS discovery_route_usage_hourly_recent_idx
  ON discovery_route_usage_hourly (
    bucket_started_at DESC,
    interface,
    route_family,
    attribution_reliability
  );

COMMENT ON TABLE discovery_route_usage_hourly IS
  'Privacy-minimized aggregate discovery interactions. No IP address, user agent, prompt, wallet, task content, request body, client identifier, or unique-agent claim is stored.';

ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_event_name_check;

ALTER TABLE site_analytics_events
  ADD CONSTRAINT site_analytics_event_name_check CHECK (event_name IN (
    'page_view',
    'market_view',
    'funded_bounty_click',
    'opportunity_feed_click',
    'unfunded_post_started',
    'unfunded_post_completed',
    'funding_started',
    'claim_started',
    'claim_confirmed',
    'competition_entry_started',
    'competition_entry_confirmed',
    'competition_reveal_started',
    'competition_reveal_confirmed',
    'competition_view',
    'competition_instructions_copied',
    'competition_template_copied',
    'competition_child_post_started',
    'competition_feedback_started',
    'competition_feedback_submitted',
    'canonical_post_started',
    'canonical_post_confirmed',
    'auth_completed',
    'wallet_link_started',
    'wallet_link_confirmed',
    'wallet_missing_detected',
    'wallet_connected',
    'wallet_unfunded_detected',
    'wallet_funded_observed',
    'canonical_post_handoff_viewed',
    'onramp_viewed',
    'onramp_moonpay_started',
    'onramp_metamask_started',
    'onramp_coinbase_started',
    'onramp_returned'
  ));
