ALTER TABLE site_analytics_events
  DROP CONSTRAINT IF EXISTS site_analytics_event_name_check;

ALTER TABLE site_analytics_events
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
