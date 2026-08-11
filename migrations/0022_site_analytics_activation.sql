ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_event_name_check,
  ADD CONSTRAINT site_analytics_event_name_check CHECK (event_name IN (
    'page_view',
    'market_view',
    'funded_bounty_click',
    'unfunded_post_started',
    'unfunded_post_completed',
    'funding_started',
    'claim_started',
    'claim_confirmed',
    'competition_entry_started',
    'competition_entry_confirmed',
    'competition_reveal_started',
    'competition_reveal_confirmed',
    'canonical_post_started',
    'canonical_post_confirmed'
  )) NOT VALID;

ALTER TABLE site_analytics_events
  VALIDATE CONSTRAINT site_analytics_event_name_check;
