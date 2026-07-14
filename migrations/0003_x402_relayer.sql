CREATE TABLE IF NOT EXISTS x402_relay_attempts (
  id UUID PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  network TEXT NOT NULL,
  bounty_contract TEXT NOT NULL,
  contributor TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  authorization_nonce TEXT NOT NULL,
  authorization_valid_before BIGINT NOT NULL CHECK (authorization_valid_before > 0),
  request_fingerprint TEXT NOT NULL,
  relayer_address TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('prepared', 'relaying', 'broadcast', 'confirmed', 'failed')),
  retryable BOOLEAN NOT NULL DEFAULT true,
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  lease_token UUID,
  lease_expires_at TIMESTAMPTZ,
  tx_hash TEXT,
  estimated_gas BIGINT CHECK (estimated_gas IS NULL OR estimated_gas > 0),
  gas_limit BIGINT CHECK (gas_limit IS NULL OR gas_limit > 0),
  error_code TEXT,
  error_message TEXT,
  canonical_event_id UUID,
  confirmed_block BIGINT CHECK (confirmed_block IS NULL OR confirmed_block >= 0),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (network, bounty_contract, authorization_nonce)
);

CREATE INDEX IF NOT EXISTS idx_x402_relay_attempts_status
  ON x402_relay_attempts (network, status, updated_at);

CREATE INDEX IF NOT EXISTS idx_x402_relay_attempts_tx_hash
  ON x402_relay_attempts (network, tx_hash)
  WHERE tx_hash IS NOT NULL;

CREATE TABLE IF NOT EXISTS x402_relayer_leases (
  network TEXT PRIMARY KEY,
  lease_token UUID NOT NULL,
  lease_expires_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
