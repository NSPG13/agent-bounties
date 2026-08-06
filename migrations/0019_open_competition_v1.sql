CREATE TABLE IF NOT EXISTS open_competition_events (
  id UUID PRIMARY KEY,
  protocol_version TEXT NOT NULL CHECK (protocol_version = 'agent-bounties/open-competition-v1'),
  log_key TEXT NOT NULL UNIQUE,
  network TEXT NOT NULL,
  factory_contract TEXT NOT NULL,
  tx_hash TEXT NOT NULL,
  block_number BIGINT NOT NULL CHECK (block_number >= 0),
  log_index BIGINT NOT NULL CHECK (log_index >= 0),
  contract_address TEXT NOT NULL,
  bounty_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  data JSONB NOT NULL,
  occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  block_time_verified BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_open_competition_events_bounty
  ON open_competition_events
  (protocol_version, network, factory_contract, bounty_id, block_number, log_index);

CREATE INDEX IF NOT EXISTS idx_open_competition_events_contract
  ON open_competition_events
  (protocol_version, network, factory_contract, contract_address, block_number, log_index);

CREATE INDEX IF NOT EXISTS idx_open_competition_events_unverified_blocks
  ON open_competition_events (network, factory_contract, block_number)
  WHERE block_time_verified = FALSE;

CREATE INDEX IF NOT EXISTS idx_open_competition_events_settlement
  ON open_competition_events (network, factory_contract, bounty_id, block_number, log_index)
  WHERE kind = 'bounty_settled' AND block_time_verified = TRUE;
