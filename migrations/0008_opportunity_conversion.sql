CREATE TABLE IF NOT EXISTS opportunity_creation_progress (
  terms_hash TEXT PRIMARY KEY,
  unfunded_bounty_id UUID REFERENCES trial_bounties(id) ON DELETE SET NULL,
  network TEXT NOT NULL,
  funding_prepared_at TIMESTAMPTZ,
  wallet_signed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (funding_prepared_at IS NOT NULL OR wallet_signed_at IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_opportunity_creation_progress_unfunded
  ON opportunity_creation_progress (unfunded_bounty_id, created_at)
  WHERE unfunded_bounty_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_opportunity_creation_progress_network
  ON opportunity_creation_progress (network, created_at);
