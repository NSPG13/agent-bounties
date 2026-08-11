CREATE TABLE IF NOT EXISTS open_competition_entrant_relays (
  id UUID PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  network TEXT NOT NULL,
  wallet TEXT NOT NULL,
  bounty_contract TEXT NOT NULL,
  delegate TEXT NOT NULL,
  action SMALLINT NOT NULL CHECK (action BETWEEN 0 AND 2),
  wallet_nonce BIGINT NOT NULL CHECK (wallet_nonce >= 0),
  deadline BIGINT NOT NULL CHECK (deadline > 0),
  payload_hash TEXT NOT NULL,
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
  receipt_block BIGINT CHECK (receipt_block IS NULL OR receipt_block >= 0),
  receipt_block_hash TEXT,
  canonical_safe_block BIGINT CHECK (canonical_safe_block IS NULL OR canonical_safe_block >= 0),
  canonical_safe_block_hash TEXT,
  canonical_event TEXT,
  payment_proven BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_open_competition_entrant_relays_live_nonce
  ON open_competition_entrant_relays (network, wallet, wallet_nonce)
  WHERE status <> 'failed' OR retryable;

CREATE INDEX IF NOT EXISTS idx_open_competition_entrant_relays_status
  ON open_competition_entrant_relays (network, status, updated_at);

CREATE INDEX IF NOT EXISTS idx_open_competition_entrant_relays_tx_hash
  ON open_competition_entrant_relays (network, tx_hash)
  WHERE tx_hash IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_open_competition_entrant_relays_wallet_created
  ON open_competition_entrant_relays (network, wallet, created_at DESC);
