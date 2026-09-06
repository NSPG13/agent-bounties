CREATE TABLE IF NOT EXISTS distribution_competition_bindings (
  network TEXT NOT NULL CHECK (network IN ('base-mainnet', 'base-sepolia')),
  factory_contract TEXT NOT NULL CHECK (factory_contract ~ '^0x[a-f0-9]{40}$'),
  bounty_id TEXT NOT NULL CHECK (bounty_id ~ '^0x[a-f0-9]{64}$'),
  protocol_version TEXT NOT NULL CHECK (
    protocol_version = 'agent-bounties/open-competition-v2-beta3'
  ),
  competition_contract TEXT NOT NULL CHECK (competition_contract ~ '^0x[a-f0-9]{40}$'),
  creator_wallet TEXT NOT NULL CHECK (creator_wallet ~ '^0x[a-f0-9]{40}$'),
  acquisition_id UUID NOT NULL REFERENCES distribution_acquisitions(id) ON DELETE CASCADE,
  prepared_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (network, factory_contract, bounty_id),
  UNIQUE (network, competition_contract)
);

CREATE INDEX IF NOT EXISTS distribution_competition_bindings_acquisition_idx
  ON distribution_competition_bindings (acquisition_id);
