CREATE TABLE IF NOT EXISTS open_competition_v2_events (
  id UUID PRIMARY KEY,
  protocol_version TEXT NOT NULL
    CHECK (protocol_version = 'agent-bounties/open-competition-v2-beta1'),
  log_key TEXT NOT NULL,
  network TEXT NOT NULL,
  factory_contract TEXT NOT NULL,
  tx_hash TEXT NOT NULL,
  block_number BIGINT NOT NULL CHECK (block_number >= 0),
  block_hash TEXT NOT NULL,
  log_index BIGINT NOT NULL CHECK (log_index >= 0),
  contract_address TEXT NOT NULL,
  bounty_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN (
    'canonical_competition_created',
    'canonical_competition_economics',
    'canonical_competition_verification',
    'canonical_competition_policies',
    'funding_added',
    'competition_activated',
    'entry_qualified',
    'leader_updated',
    'competition_settled',
    'competition_cancelled',
    'refund_withdrawn'
  )),
  data JSONB NOT NULL,
  occurred_at TIMESTAMPTZ NOT NULL,
  safe_block_number BIGINT NOT NULL CHECK (safe_block_number >= block_number),
  safe_block_hash TEXT NOT NULL,
  inserted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (network, factory_contract, log_key),
  UNIQUE (network, factory_contract, tx_hash, log_index)
);

CREATE INDEX IF NOT EXISTS idx_open_competition_v2_events_bounty
  ON open_competition_v2_events
  (network, factory_contract, bounty_id, block_number, log_index);

CREATE INDEX IF NOT EXISTS idx_open_competition_v2_events_settlement
  ON open_competition_v2_events
  (network, factory_contract, bounty_id, block_number, log_index)
  WHERE kind = 'competition_settled';

CREATE TABLE IF NOT EXISTS open_competition_v2_projections (
  network TEXT NOT NULL,
  factory_contract TEXT NOT NULL,
  bounty_id TEXT NOT NULL,
  competition_contract TEXT NOT NULL,
  creator TEXT NOT NULL,
  creation_nonce TEXT,
  beta_risk_hash TEXT,
  state TEXT NOT NULL CHECK (state IN ('announced', 'funding', 'active', 'settled', 'cancelled')),
  solver_reward TEXT NOT NULL CHECK (solver_reward ~ '^[0-9]+$'),
  keeper_reward TEXT NOT NULL CHECK (keeper_reward ~ '^[0-9]+$'),
  funding_deadline BIGINT CHECK (funding_deadline IS NULL OR funding_deadline >= 0),
  proof_window_seconds BIGINT CHECK (proof_window_seconds IS NULL OR proof_window_seconds > 0),
  winner_mode TEXT CHECK (winner_mode IN ('first_proven', 'best_score')),
  score_direction TEXT CHECK (score_direction IN ('higher_is_better', 'lower_is_better')),
  score_threshold TEXT CHECK (score_threshold IS NULL OR score_threshold ~ '^-?[0-9]+$'),
  proof_system TEXT CHECK (proof_system IN ('groth16', 'plonk')),
  verifier_adapter TEXT,
  program_vkey TEXT,
  source_hash TEXT,
  elf_hash TEXT,
  journal_schema_hash TEXT,
  metric_program_hash TEXT,
  execution_policy_hash TEXT,
  verification_policy_hash TEXT,
  settlement_policy_hash TEXT,
  funded_amount TEXT NOT NULL CHECK (funded_amount ~ '^[0-9]+$'),
  proof_deadline BIGINT CHECK (proof_deadline IS NULL OR proof_deadline >= 0),
  accepted_entries BIGINT NOT NULL CHECK (accepted_entries >= 0),
  leader TEXT,
  winner TEXT,
  refund_pool_remaining TEXT NOT NULL CHECK (refund_pool_remaining ~ '^[0-9]+$'),
  last_block BIGINT NOT NULL CHECK (last_block >= 0),
  last_log_index BIGINT NOT NULL CHECK (last_log_index >= 0),
  safe_block_number BIGINT NOT NULL CHECK (safe_block_number >= last_block),
  safe_block_hash TEXT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (network, factory_contract, bounty_id)
);

CREATE INDEX IF NOT EXISTS idx_open_competition_v2_projection_inventory
  ON open_competition_v2_projections
  (network, state, solver_reward, proof_deadline);

CREATE TABLE IF NOT EXISTS open_competition_v2_programs (
  program_vkey TEXT PRIMARY KEY,
  profile_id TEXT NOT NULL UNIQUE,
  classification TEXT NOT NULL CHECK (classification IN ('reviewed', 'custom_unreviewed', 'disabled')),
  source_hash TEXT NOT NULL,
  elf_hash TEXT NOT NULL,
  journal_schema_hash TEXT NOT NULL,
  metric_program_hash TEXT NOT NULL,
  manifest JSONB NOT NULL,
  release_evidence_hash TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS open_competition_v2_proof_jobs (
  id UUID PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  network TEXT NOT NULL,
  competition_contract TEXT NOT NULL,
  solver TEXT NOT NULL,
  solver_nonce TEXT NOT NULL CHECK (solver_nonce ~ '^[0-9]+$'),
  artifact_hash TEXT NOT NULL,
  program_input JSONB NOT NULL,
  expected_public_values TEXT NOT NULL,
  requested_relay BOOLEAN NOT NULL,
  proof_system TEXT NOT NULL CHECK (proof_system IN ('groth16', 'plonk')),
  state TEXT NOT NULL CHECK (state IN (
    'quoted', 'payment_pending', 'paid', 'proving', 'proved', 'relaying', 'confirmed',
    'refund_due', 'refunded', 'lost_competition'
  )),
  gross_prize TEXT NOT NULL CHECK (gross_prize ~ '^[0-9]+$'),
  proof_fee_quote TEXT NOT NULL CHECK (proof_fee_quote ~ '^[0-9]+$'),
  relay_fee_quote TEXT NOT NULL CHECK (relay_fee_quote ~ '^[0-9]+$'),
  net_prize_if_win TEXT NOT NULL CHECK (net_prize_if_win ~ '^-?[0-9]+$'),
  maximum_charge TEXT NOT NULL CHECK (maximum_charge ~ '^[0-9]+$'),
  winner_mode TEXT NOT NULL CHECK (winner_mode IN ('first_proven', 'best_score')),
  competition_risk TEXT NOT NULL,
  quote_expires_at TIMESTAMPTZ NOT NULL,
  proof_sla_deadline TIMESTAMPTZ NOT NULL,
  payer TEXT,
  payment_authorization_nonce TEXT,
  payment_authorization JSONB,
  payment_tx_hash TEXT,
  payment_block_number BIGINT CHECK (payment_block_number IS NULL OR payment_block_number >= 0),
  payment_evidence JSONB,
  proof_hash TEXT,
  public_values_hash TEXT,
  proof TEXT,
  public_values TEXT,
  proof_provider_job_id TEXT,
  solver_authorization_deadline BIGINT
    CHECK (solver_authorization_deadline IS NULL OR solver_authorization_deadline >= 0),
  solver_signature TEXT,
  relay_tx_hash TEXT,
  settlement_event_id UUID REFERENCES open_competition_v2_events(id),
  refund_evidence JSONB,
  refund_tx_hash TEXT,
  refund_block_number BIGINT CHECK (refund_block_number IS NULL OR refund_block_number >= 0),
  refund_due_at TIMESTAMPTZ,
  failure_code TEXT,
  failure_message TEXT,
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  lease_token UUID,
  lease_expires_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (network, competition_contract, solver, solver_nonce)
);

CREATE INDEX IF NOT EXISTS idx_open_competition_v2_proof_jobs_state
  ON open_competition_v2_proof_jobs (state, updated_at);

CREATE INDEX IF NOT EXISTS idx_open_competition_v2_proof_jobs_refund_due
  ON open_competition_v2_proof_jobs (refund_due_at)
  WHERE state = 'refund_due';
