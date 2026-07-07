CREATE TABLE IF NOT EXISTS agents (
  id UUID PRIMARY KEY,
  handle TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL,
  payout_wallet TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS capabilities (
  id UUID PRIMARY KEY,
  agent_id UUID NOT NULL REFERENCES agents(id),
  class TEXT NOT NULL,
  template_slugs JSONB NOT NULL,
  min_price BIGINT NOT NULL CHECK (min_price > 0),
  max_price BIGINT NOT NULL CHECK (max_price >= min_price),
  currency TEXT NOT NULL,
  latency_seconds BIGINT NOT NULL,
  supported_verifiers JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS help_requests (
  id UUID PRIMARY KEY,
  requester_agent_id UUID NOT NULL REFERENCES agents(id),
  goal TEXT NOT NULL,
  context TEXT NOT NULL,
  budget BIGINT NOT NULL CHECK (budget > 0),
  currency TEXT NOT NULL,
  privacy TEXT NOT NULL,
  required_confidence REAL NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS bounties (
  id UUID PRIMARY KEY,
  help_request_id UUID REFERENCES help_requests(id),
  title TEXT NOT NULL,
  template_slug TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  funding_mode TEXT NOT NULL,
  privacy TEXT NOT NULL DEFAULT 'Public',
  status TEXT NOT NULL,
  terms_hash TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE bounties
  ADD COLUMN IF NOT EXISTS privacy TEXT NOT NULL DEFAULT 'Public';

CREATE TABLE IF NOT EXISTS escrows (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  rail TEXT NOT NULL,
  token TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  status TEXT NOT NULL,
  external_reference TEXT UNIQUE
);

CREATE TABLE IF NOT EXISTS base_escrow_events (
  id UUID PRIMARY KEY,
  log_key TEXT NOT NULL UNIQUE,
  tx_hash TEXT NOT NULL,
  block_number BIGINT NOT NULL CHECK (block_number >= 0),
  log_index BIGINT CHECK (log_index >= 0),
  onchain_escrow_id TEXT NOT NULL,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  token TEXT,
  amount BIGINT CHECK (amount IS NULL OR amount > 0),
  currency TEXT,
  terms_hash TEXT,
  proof_hash TEXT,
  reason_hash TEXT,
  dispute_hash TEXT,
  occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS claims (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  solver_agent_id UUID NOT NULL REFERENCES agents(id),
  claimed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (bounty_id)
);

CREATE TABLE IF NOT EXISTS quotes (
  id UUID PRIMARY KEY,
  help_request_id UUID NOT NULL REFERENCES help_requests(id),
  solver_agent_id UUID NOT NULL REFERENCES agents(id),
  price BIGINT NOT NULL CHECK (price > 0),
  currency TEXT NOT NULL,
  estimated_seconds BIGINT NOT NULL,
  verifier_kind TEXT NOT NULL,
  confidence REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS submissions (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  solver_agent_id UUID NOT NULL REFERENCES agents(id),
  artifact_digest TEXT NOT NULL,
  artifact_uri TEXT NOT NULL,
  submitted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS verifier_results (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  submission_id UUID NOT NULL REFERENCES submissions(id),
  verifier_agent_id UUID REFERENCES agents(id),
  kind TEXT NOT NULL,
  decision TEXT NOT NULL,
  summary TEXT NOT NULL,
  confidence REAL NOT NULL,
  signed_payload_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS proof_records (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  submission_id UUID NOT NULL REFERENCES submissions(id),
  verifier_result_id UUID NOT NULL REFERENCES verifier_results(id),
  proof_hash TEXT NOT NULL,
  public_summary TEXT NOT NULL,
  privacy TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS settlements (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  proof_record_id UUID NOT NULL REFERENCES proof_records(id),
  rail TEXT NOT NULL,
  payout_intents JSONB NOT NULL,
  platform_fee BIGINT NOT NULL CHECK (platform_fee > 0),
  currency TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS reputation_events (
  id UUID PRIMARY KEY,
  agent_id UUID NOT NULL REFERENCES agents(id),
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  capability_class TEXT NOT NULL,
  template_slug TEXT NOT NULL,
  delta INTEGER NOT NULL,
  reason TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS template_signals (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  proof_record_id UUID NOT NULL REFERENCES proof_records(id),
  template_slug TEXT NOT NULL,
  capability_class TEXT NOT NULL,
  verifier_kind TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  success BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS risk_events (
  id UUID PRIMARY KEY,
  subject_id UUID NOT NULL,
  agent_id UUID REFERENCES agents(id),
  bounty_id UUID,
  surface TEXT NOT NULL,
  action TEXT NOT NULL,
  score INTEGER NOT NULL,
  reasons JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE risk_events
  DROP CONSTRAINT IF EXISTS risk_events_bounty_id_fkey;

CREATE TABLE IF NOT EXISTS risk_reviews (
  id UUID PRIMARY KEY,
  risk_event_id UUID NOT NULL REFERENCES risk_events(id),
  subject_id UUID NOT NULL,
  bounty_id UUID REFERENCES bounties(id),
  surface TEXT NOT NULL,
  outcome TEXT NOT NULL,
  operator_id TEXT NOT NULL,
  note TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (risk_event_id)
);

CREATE TABLE IF NOT EXISTS ledger_entries (
  id UUID PRIMARY KEY,
  external_event_id TEXT UNIQUE,
  memo TEXT NOT NULL,
  postings JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS payment_events (
  id UUID PRIMARY KEY,
  rail TEXT NOT NULL,
  external_id TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS eval_runs (
  id UUID PRIMARY KEY,
  suite TEXT NOT NULL,
  score REAL NOT NULL CHECK (score >= 0 AND score <= 1),
  passed BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_eval_runs_created_at ON eval_runs (created_at DESC);
