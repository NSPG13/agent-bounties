ALTER TABLE site_analytics_events
  ADD COLUMN IF NOT EXISTS placement TEXT,
  ADD COLUMN IF NOT EXISTS variant TEXT,
  ADD COLUMN IF NOT EXISTS opportunity_class TEXT,
  ADD COLUMN IF NOT EXISTS current_source TEXT,
  ADD COLUMN IF NOT EXISTS current_campaign TEXT,
  ADD COLUMN IF NOT EXISTS current_referrer_host TEXT,
  ADD COLUMN IF NOT EXISTS site_host TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_event_name_check;

ALTER TABLE site_analytics_events
  ADD CONSTRAINT site_analytics_event_name_check CHECK (event_name IN (
    'page_view',
    'market_view',
    'opportunity_exposed',
    'funded_bounty_click',
    'unfunded_post_started',
    'unfunded_post_completed',
    'funding_started',
    'claim_started',
    'claim_confirmed',
    'canonical_post_started',
    'canonical_post_confirmed'
  ));

ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_context_token_check;

ALTER TABLE site_analytics_events
  ADD CONSTRAINT site_analytics_context_token_check CHECK (
    (placement IS NULL OR (length(placement) BETWEEN 1 AND 64 AND placement ~ '^[a-z0-9][a-z0-9._-]*$'))
    AND (variant IS NULL OR (length(variant) BETWEEN 1 AND 64 AND variant ~ '^[a-z0-9][a-z0-9._-]*$'))
    AND (opportunity_class IS NULL OR (length(opportunity_class) BETWEEN 1 AND 64 AND opportunity_class ~ '^[a-z0-9][a-z0-9._-]*$'))
    AND (current_source IS NULL OR (length(current_source) BETWEEN 1 AND 64 AND current_source ~ '^[a-z0-9][a-z0-9._-]*$'))
    AND (current_campaign IS NULL OR (length(current_campaign) BETWEEN 1 AND 64 AND current_campaign ~ '^[a-z0-9][a-z0-9._-]*$'))
  );

ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_current_referrer_host_check;

ALTER TABLE site_analytics_events
  ADD CONSTRAINT site_analytics_current_referrer_host_check CHECK (
    current_referrer_host IS NULL
    OR (
      length(current_referrer_host) BETWEEN 1 AND 253
      AND current_referrer_host ~ '^[a-z0-9.-]+$'
    )
  );

ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_site_host_check;

ALTER TABLE site_analytics_events
  ADD CONSTRAINT site_analytics_site_host_check CHECK (
    site_host IN (
      'unknown',
      'bountyboard.global',
      'agentbounties.app',
      'localhost'
    )
  );

ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_opportunity_exposure_check;

ALTER TABLE site_analytics_events
  ADD CONSTRAINT site_analytics_opportunity_exposure_check CHECK (
    event_name <> 'opportunity_exposed'
    OR opportunity_id IS NOT NULL
    OR bounty_contract IS NOT NULL
  );

CREATE INDEX IF NOT EXISTS site_analytics_events_opportunity_funnel_idx
  ON site_analytics_events (event_name, bounty_contract, opportunity_id, occurred_at);

CREATE INDEX IF NOT EXISTS site_analytics_events_current_source_idx
  ON site_analytics_events (current_source, current_campaign, occurred_at DESC);

CREATE INDEX IF NOT EXISTS site_analytics_events_site_host_idx
  ON site_analytics_events (site_host, occurred_at DESC);
