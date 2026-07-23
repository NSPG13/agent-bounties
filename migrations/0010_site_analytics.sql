CREATE TABLE IF NOT EXISTS site_analytics_events (
  event_id UUID PRIMARY KEY,
  visitor_id UUID NOT NULL,
  session_id UUID NOT NULL,
  event_name TEXT NOT NULL,
  page_path TEXT NOT NULL,
  source TEXT,
  campaign TEXT,
  referrer_host TEXT,
  opportunity_id TEXT,
  bounty_contract TEXT,
  occurred_at TIMESTAMPTZ NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT site_analytics_event_name_check CHECK (event_name IN (
    'page_view',
    'market_view',
    'funded_bounty_click',
    'unfunded_post_started',
    'unfunded_post_completed',
    'funding_started',
    'claim_started',
    'claim_confirmed',
    'canonical_post_started',
    'canonical_post_confirmed'
  )),
  CONSTRAINT site_analytics_page_path_check CHECK (
    length(page_path) BETWEEN 1 AND 160
    AND left(page_path, 1) = '/'
    AND position('?' IN page_path) = 0
    AND position('#' IN page_path) = 0
  ),
  CONSTRAINT site_analytics_source_check CHECK (
    source IS NULL OR (length(source) BETWEEN 1 AND 64 AND source ~ '^[a-z0-9][a-z0-9._-]*$')
  ),
  CONSTRAINT site_analytics_campaign_check CHECK (
    campaign IS NULL OR (length(campaign) BETWEEN 1 AND 64 AND campaign ~ '^[a-z0-9][a-z0-9._-]*$')
  ),
  CONSTRAINT site_analytics_referrer_host_check CHECK (
    referrer_host IS NULL OR (length(referrer_host) BETWEEN 1 AND 253 AND referrer_host ~ '^[a-z0-9.-]+$')
  ),
  CONSTRAINT site_analytics_opportunity_check CHECK (
    opportunity_id IS NULL OR (length(opportunity_id) BETWEEN 1 AND 200 AND opportunity_id ~ '^[A-Za-z0-9:._-]+$')
  ),
  CONSTRAINT site_analytics_bounty_contract_check CHECK (
    bounty_contract IS NULL OR bounty_contract ~ '^0x[0-9a-f]{40}$'
  ),
  CONSTRAINT site_analytics_event_time_check CHECK (
    occurred_at >= received_at - INTERVAL '7 days'
    AND occurred_at <= received_at + INTERVAL '5 minutes'
  )
);

CREATE INDEX IF NOT EXISTS site_analytics_events_occurred_idx
  ON site_analytics_events (occurred_at DESC);

CREATE INDEX IF NOT EXISTS site_analytics_events_visitor_idx
  ON site_analytics_events (visitor_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS site_analytics_events_session_idx
  ON site_analytics_events (session_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS site_analytics_events_name_idx
  ON site_analytics_events (event_name, occurred_at DESC);

CREATE INDEX IF NOT EXISTS site_analytics_events_source_idx
  ON site_analytics_events (source, occurred_at DESC);
