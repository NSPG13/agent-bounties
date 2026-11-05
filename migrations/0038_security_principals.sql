CREATE TABLE IF NOT EXISTS security_wallet_challenges (
  id UUID PRIMARY KEY,
  domain TEXT NOT NULL,
  chain_id BIGINT NOT NULL CHECK (chain_id > 0),
  address TEXT NOT NULL CHECK (address ~ '^0x[0-9a-f]{40}$'),
  message TEXT NOT NULL CHECK (octet_length(message) <= 4096),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
  CHECK (expires_at > created_at AND expires_at <= created_at + interval '5 minutes')
);
CREATE INDEX IF NOT EXISTS security_wallet_challenges_expiry ON security_wallet_challenges(expires_at);
CREATE TABLE IF NOT EXISTS security_wallet_sessions (
  token_hash TEXT PRIMARY KEY CHECK (token_hash ~ '^[0-9a-f]{64}$'),
  challenge_id UUID NOT NULL UNIQUE REFERENCES security_wallet_challenges(id),
  chain_id BIGINT NOT NULL CHECK (chain_id > 0),
  address TEXT NOT NULL CHECK (address ~ '^0x[0-9a-f]{40}$'),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  contract_signature TEXT CHECK (contract_signature IS NULL OR (length(contract_signature) BETWEEN 4 AND 16386 AND contract_signature ~ '^0x[0-9a-fA-F]+$')),
  CHECK (expires_at > created_at AND expires_at <= created_at + interval '15 minutes')
);
CREATE INDEX IF NOT EXISTS security_wallet_sessions_expiry ON security_wallet_sessions(expires_at);
CREATE TABLE IF NOT EXISTS security_resource_owners (
  resource_kind TEXT NOT NULL CHECK (length(resource_kind) BETWEEN 1 AND 40),
  resource_id TEXT NOT NULL CHECK (length(resource_id) BETWEEN 1 AND 200),
  principal_key TEXT NOT NULL CHECK (length(principal_key) BETWEEN 1 AND 200),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY(resource_kind,resource_id)
);
CREATE TABLE IF NOT EXISTS security_resource_grants (
  resource_kind TEXT NOT NULL,
  resource_id TEXT NOT NULL,
  grantee_key TEXT NOT NULL CHECK (length(grantee_key) BETWEEN 1 AND 200),
  permission TEXT NOT NULL CHECK (permission IN ('read','write')),
  expires_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY(resource_kind,resource_id,grantee_key,permission),
  FOREIGN KEY(resource_kind,resource_id) REFERENCES security_resource_owners(resource_kind,resource_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS security_quota_reservations (
  principal_key TEXT NOT NULL CHECK (length(principal_key) BETWEEN 1 AND 200),
  action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 40),
  idempotency_hash TEXT NOT NULL CHECK (idempotency_hash ~ '^[0-9a-f]{64}$'),
  payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^[0-9a-f]{64}$'),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '7 days',
  PRIMARY KEY(principal_key,action,idempotency_hash)
);
CREATE INDEX IF NOT EXISTS security_quota_window ON security_quota_reservations(principal_key,action,created_at);
CREATE INDEX IF NOT EXISTS security_quota_expiry ON security_quota_reservations(expires_at);
