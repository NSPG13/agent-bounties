CREATE TABLE IF NOT EXISTS trial_bounties (
  id UUID PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  request_fingerprint TEXT NOT NULL,
  title TEXT NOT NULL,
  goal TEXT NOT NULL,
  acceptance_criteria JSONB NOT NULL,
  source_url TEXT,
  discovery_source TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('open', 'closed')),
  demo_agent_solution JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL,
  CHECK (expires_at > created_at)
);

CREATE INDEX IF NOT EXISTS idx_trial_bounties_recent
  ON trial_bounties (created_at DESC, id);

CREATE TABLE IF NOT EXISTS unfunded_bounty_solutions (
  id UUID PRIMARY KEY,
  trial_bounty_id UUID NOT NULL REFERENCES trial_bounties(id) ON DELETE CASCADE,
  agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  summary TEXT NOT NULL,
  deliverable_markdown TEXT NOT NULL,
  evidence JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (trial_bounty_id, agent_id)
);

CREATE INDEX IF NOT EXISTS idx_unfunded_bounty_solutions_bounty
  ON unfunded_bounty_solutions (trial_bounty_id, created_at, id);
