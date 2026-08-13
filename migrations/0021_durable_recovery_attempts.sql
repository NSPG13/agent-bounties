CREATE TABLE IF NOT EXISTS recovery_attempts (
  recovery_identity TEXT PRIMARY KEY,
  network TEXT NOT NULL CHECK (network = 'base-mainnet'),
  pending_nonce BIGINT NOT NULL CHECK (pending_nonce >= 0),
  contract_address TEXT NOT NULL,
  contract_code_hash TEXT NOT NULL,
  bounty_id TEXT NOT NULL,
  expected_status SMALLINT NOT NULL,
  expected_round BIGINT NOT NULL,
  solver_address TEXT NOT NULL,
  verification_expires_at BIGINT NOT NULL,
  active_bond BIGINT NOT NULL,
  calldata TEXT NOT NULL,
  lease_source TEXT NOT NULL,
  lease_token UUID NOT NULL,
  lease_expires_at TIMESTAMPTZ NOT NULL,
  lease_attested_at TIMESTAMPTZ NOT NULL,
  lease_recovered_at TIMESTAMPTZ,
  status TEXT NOT NULL CHECK (status IN ('reserved', 'broadcast', 'confirmed')),
  signed_transaction_hash TEXT NOT NULL,
  signed_transaction TEXT NOT NULL,
  broadcast_started_at TIMESTAMPTZ,
  rpc_transaction_hash TEXT,
  receipt_block BIGINT,
  receipt_status SMALLINT,
  canonical_event_id UUID,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (recovery_identity = 'agent-bounties-772-round4-expiry-v2'),
  CHECK (network = 'base-mainnet' AND pending_nonce = 41),
  CHECK (lower(contract_address) = '0x9baa8a4a2ad3096c6ebfb2c994a93afb7a299274'),
  CHECK (lower(bounty_id) = '0x34e8d16cdbfff635e77ce703cc6efea8fc64a3adb1ee2ef293c604b85bb6a8cb'),
  CHECK (expected_status = 3 AND expected_round = 4),
  CHECK (lower(solver_address) = '0xc49e5374f0072abc0b4c134b2fd413d87aa6354a'),
  CHECK (lower(contract_code_hash) = '0x6e7d6297e170d10e6484c9b72314bb0e2173cd967aa8e05231ee369dbde0c0a1'),
  CHECK (verification_expires_at = 1786586903 AND active_bond = 10000),
  CHECK (lower(calldata) = '0xf9251ec7'),
  CHECK ((status = 'reserved' AND broadcast_started_at IS NULL)
      OR (status IN ('broadcast', 'confirmed') AND broadcast_started_at IS NOT NULL)),
  CHECK (rpc_transaction_hash IS NULL OR lower(rpc_transaction_hash) = lower(signed_transaction_hash)),
  CHECK (status <> 'confirmed' OR (receipt_status = 1 AND receipt_block IS NOT NULL))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_recovery_attempts_nonce
  ON recovery_attempts (network, pending_nonce);
CREATE UNIQUE INDEX IF NOT EXISTS idx_recovery_attempts_signed_hash
  ON recovery_attempts (signed_transaction_hash);

COMMENT ON TABLE recovery_attempts IS
  'Dedicated durable money-recovery state; generic relay state is prohibited.';
