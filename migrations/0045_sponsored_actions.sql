CREATE TABLE IF NOT EXISTS sponsored_actions (
  id UUID PRIMARY KEY,
  network TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  wallet TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'prepared' CHECK (status IN ('prepared', 'relaying', 'broadcast', 'confirmed', 'failed')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 3),
  lease_token UUID,
  lease_expires_at TIMESTAMPTZ,
  transaction_hash TEXT,
  error_message TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (network, request_hash)
);
CREATE INDEX IF NOT EXISTS sponsored_actions_quota ON sponsored_actions(network, created_at, wallet);
