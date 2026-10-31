CREATE TABLE IF NOT EXISTS security_mutation_results (
  principal_key TEXT NOT NULL,
  operation TEXT NOT NULL,
  idempotency_hash TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  result_json JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (principal_key, operation, idempotency_hash)
);

-- Existing rows are deliberately unverified. No wallet string is an ownership proof.
ALTER TABLE claim_candidates ADD COLUMN IF NOT EXISTS principal_key TEXT;
ALTER TABLE claim_candidates ADD COLUMN IF NOT EXISTS request_fingerprint TEXT;
ALTER TABLE claim_candidates ADD COLUMN IF NOT EXISTS relay_guard_until TIMESTAMPTZ;
ALTER TABLE claim_candidates ADD COLUMN IF NOT EXISTS maintenance_checked_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_claim_candidates_maintenance
  ON claim_candidates (maintenance_checked_at NULLS FIRST, network, bounty_contract)
  WHERE status IN ('waitlisted', 'exclusive', 'authorization_ready', 'sponsoring', 'relaying');
