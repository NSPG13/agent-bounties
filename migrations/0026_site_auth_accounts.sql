CREATE TABLE IF NOT EXISTS site_auth_accounts (
  account_key TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  provider_subject TEXT NOT NULL,
  display_name TEXT NOT NULL,
  email TEXT NOT NULL DEFAULT '',
  avatar_url TEXT NOT NULL DEFAULT '',
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_signed_in_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (provider, provider_subject),
  CHECK (char_length(account_key) = 64),
  CHECK (char_length(provider) BETWEEN 1 AND 32),
  CHECK (char_length(provider_subject) BETWEEN 1 AND 512),
  CHECK (char_length(display_name) BETWEEN 1 AND 160),
  CHECK (char_length(email) <= 320),
  CHECK (char_length(avatar_url) <= 2048)
);

CREATE TABLE IF NOT EXISTS site_auth_wallets (
  wallet_address TEXT PRIMARY KEY,
  account_key TEXT NOT NULL REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
  chain_id BIGINT NOT NULL DEFAULT 8453,
  proof_method TEXT NOT NULL DEFAULT 'eip191_personal_sign',
  linked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (wallet_address ~ '^0x[0-9a-f]{40}$'),
  CHECK (chain_id > 0),
  CHECK (proof_method = 'eip191_personal_sign')
);

CREATE INDEX IF NOT EXISTS idx_site_auth_wallets_account
  ON site_auth_wallets (account_key, linked_at);
