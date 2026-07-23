CREATE TABLE IF NOT EXISTS legal_acceptances (
  id UUID PRIMARY KEY,
  terms_version TEXT NOT NULL,
  privacy_version TEXT NOT NULL,
  action TEXT NOT NULL,
  wallet_address TEXT NOT NULL,
  statement_hash TEXT NOT NULL,
  acceptance_method TEXT NOT NULL,
  accepted_at TIMESTAMPTZ NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT legal_acceptances_action_not_blank CHECK (length(trim(action)) > 0),
  CONSTRAINT legal_acceptances_wallet_format CHECK (wallet_address ~ '^0x[0-9a-f]{40}$'),
  CONSTRAINT legal_acceptances_statement_hash_format CHECK (statement_hash ~ '^sha256:[0-9a-f]{64}$'),
  CONSTRAINT legal_acceptances_method_check CHECK (acceptance_method IN ('web_clickwrap', 'api_explicit'))
);

CREATE INDEX IF NOT EXISTS legal_acceptances_wallet_recorded_idx
  ON legal_acceptances (wallet_address, recorded_at DESC);

CREATE INDEX IF NOT EXISTS legal_acceptances_action_recorded_idx
  ON legal_acceptances (action, recorded_at DESC);
