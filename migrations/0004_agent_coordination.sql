CREATE TABLE IF NOT EXISTS recovery_obligations (
  id UUID PRIMARY KEY,
  issue_number BIGINT NOT NULL CHECK (issue_number > 0),
  source_contract TEXT NOT NULL,
  recipient_wallet TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL CHECK (currency = 'usdc'),
  reason TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('open', 'broadcast', 'confirmed', 'disputed')),
  transaction_hash TEXT,
  evidence_url TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (issue_number, recipient_wallet, amount)
);

CREATE TABLE IF NOT EXISTS claim_candidates (
  id UUID PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  network TEXT NOT NULL,
  bounty_contract TEXT NOT NULL,
  solver_wallet TEXT NOT NULL,
  agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
  eligibility_evidence JSONB NOT NULL,
  eligibility_decision JSONB NOT NULL,
  status TEXT NOT NULL CHECK (status IN (
    'waitlisted', 'exclusive', 'sponsoring', 'authorization_ready',
    'relaying', 'claimed', 'superseded', 'withdrawn', 'failed'
  )),
  exclusive_until TIMESTAMPTZ,
  authorization_nonce TEXT,
  authorization_valid_before BIGINT CHECK (
    authorization_valid_before IS NULL OR authorization_valid_before > 0
  ),
  claim_transaction_hash TEXT,
  canonical_event_id UUID,
  failure_code TEXT,
  failure_message TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_claim_candidates_one_exclusive
  ON claim_candidates (network, bounty_contract)
  WHERE status IN ('exclusive', 'sponsoring', 'authorization_ready', 'relaying');

CREATE INDEX IF NOT EXISTS idx_claim_candidates_waitlist
  ON claim_candidates (network, bounty_contract, created_at, id)
  WHERE status = 'waitlisted';

CREATE UNIQUE INDEX IF NOT EXISTS idx_claim_candidates_one_active_per_solver
  ON claim_candidates (network, bounty_contract, solver_wallet)
  WHERE status IN (
    'waitlisted', 'exclusive', 'sponsoring', 'authorization_ready', 'relaying'
  );

CREATE TABLE IF NOT EXISTS bond_sponsorships (
  id UUID PRIMARY KEY,
  claim_candidate_id UUID NOT NULL UNIQUE REFERENCES claim_candidates(id) ON DELETE CASCADE,
  network TEXT NOT NULL,
  bounty_contract TEXT NOT NULL,
  solver_wallet TEXT NOT NULL,
  sponsor_wallet TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  status TEXT NOT NULL CHECK (status IN ('reserved', 'broadcast', 'confirmed', 'failed')),
  transaction_hash TEXT,
  confirmed_block BIGINT CHECK (confirmed_block IS NULL OR confirmed_block >= 0),
  failure_code TEXT,
  failure_message TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_bond_sponsorships_rolling_caps
  ON bond_sponsorships (network, solver_wallet, created_at);

CREATE TABLE IF NOT EXISTS webhook_subscriptions (
  id UUID PRIMARY KEY,
  owner_wallet TEXT NOT NULL,
  endpoint_url TEXT NOT NULL,
  event_types JSONB NOT NULL,
  secret_version INTEGER NOT NULL DEFAULT 1 CHECK (secret_version > 0),
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (owner_wallet, endpoint_url)
);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
  id UUID PRIMARY KEY,
  subscription_id UUID NOT NULL REFERENCES webhook_subscriptions(id) ON DELETE CASCADE,
  event_id UUID NOT NULL,
  event_type TEXT NOT NULL,
  payload JSONB NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'delivering', 'delivered', 'dead')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  lease_token UUID,
  lease_expires_at TIMESTAMPTZ,
  response_status INTEGER,
  last_error TEXT,
  delivered_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (subscription_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_ready
  ON webhook_deliveries (status, next_attempt_at)
  WHERE status IN ('pending', 'delivering');

CREATE TABLE IF NOT EXISTS regression_verification_runs (
  id UUID PRIMARY KEY,
  bounty_contract TEXT NOT NULL,
  round BIGINT NOT NULL CHECK (round > 0),
  manifest_hash TEXT NOT NULL,
  artifact_digest TEXT NOT NULL,
  sandbox_image TEXT NOT NULL,
  command JSONB NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'accepted', 'rejected', 'failed')),
  exit_code INTEGER,
  stdout_hash TEXT,
  stderr_hash TEXT,
  result JSONB,
  started_at TIMESTAMPTZ,
  completed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (bounty_contract, round, manifest_hash, artifact_digest)
);
