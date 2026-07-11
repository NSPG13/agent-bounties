CREATE TABLE IF NOT EXISTS autonomous_bounty_events (
  id UUID PRIMARY KEY,
  log_key TEXT NOT NULL UNIQUE,
  network TEXT NOT NULL,
  tx_hash TEXT NOT NULL,
  block_number BIGINT NOT NULL CHECK (block_number >= 0),
  log_index BIGINT NOT NULL CHECK (log_index >= 0),
  contract_address TEXT NOT NULL,
  bounty_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  data JSONB NOT NULL,
  occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_autonomous_bounty_events_bounty
  ON autonomous_bounty_events (network, bounty_id, block_number, log_index);

CREATE INDEX IF NOT EXISTS idx_autonomous_bounty_events_contract
  ON autonomous_bounty_events (network, contract_address, block_number, log_index);

CREATE TABLE IF NOT EXISTS autonomous_bounty_terms (
  terms_hash TEXT PRIMARY KEY,
  policy_hash TEXT NOT NULL,
  acceptance_criteria_hash TEXT NOT NULL,
  benchmark_hash TEXT NOT NULL,
  evidence_schema_hash TEXT NOT NULL,
  creator_wallet TEXT NOT NULL,
  document JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_autonomous_bounty_terms_creator
  ON autonomous_bounty_terms (creator_wallet, created_at DESC);

CREATE TABLE IF NOT EXISTS autonomous_submission_evidence (
  network TEXT NOT NULL,
  bounty_contract TEXT NOT NULL,
  bounty_id TEXT NOT NULL,
  round BIGINT NOT NULL CHECK (round > 0),
  solver_wallet TEXT NOT NULL,
  artifact_reference TEXT NOT NULL,
  artifact_hash TEXT NOT NULL,
  evidence JSONB NOT NULL,
  evidence_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (network, bounty_contract, round)
);

CREATE INDEX IF NOT EXISTS idx_autonomous_submission_evidence_bounty
  ON autonomous_submission_evidence (network, bounty_id, round DESC);
