CREATE TABLE IF NOT EXISTS site_auth_identities (
  provider TEXT NOT NULL,
  provider_subject TEXT NOT NULL,
  account_key TEXT NOT NULL REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
  verified_email TEXT,
  verified_email_key TEXT,
  email_verified_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_signed_in_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (provider, provider_subject),
  CHECK (char_length(provider) BETWEEN 1 AND 32),
  CHECK (char_length(provider_subject) BETWEEN 1 AND 512),
  CHECK (verified_email IS NULL OR char_length(verified_email) <= 320),
  CHECK (verified_email_key IS NULL OR char_length(verified_email_key) <= 320),
  CHECK ((verified_email_key IS NULL) = (email_verified_at IS NULL))
);

INSERT INTO site_auth_identities
  (provider, provider_subject, account_key, created_at, last_signed_in_at)
SELECT provider, provider_subject, account_key, created_at, last_signed_in_at
FROM site_auth_accounts
ON CONFLICT (provider, provider_subject) DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_site_auth_identities_account
  ON site_auth_identities (account_key, last_signed_in_at DESC);

CREATE TABLE IF NOT EXISTS site_auth_verified_emails (
  email_key TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  account_key TEXT NOT NULL REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
  verified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (char_length(email_key) BETWEEN 3 AND 320),
  CHECK (char_length(email) BETWEEN 3 AND 320)
);

CREATE INDEX IF NOT EXISTS idx_site_auth_verified_emails_account
  ON site_auth_verified_emails (account_key, verified_at ASC);

CREATE TABLE IF NOT EXISTS site_auth_password_credentials (
  account_key TEXT PRIMARY KEY REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
  password_phc TEXT NOT NULL,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (char_length(password_phc) BETWEEN 32 AND 1024)
);

CREATE TABLE IF NOT EXISTS site_auth_email_actions (
  token_hash TEXT PRIMARY KEY,
  setup_hash TEXT UNIQUE,
  purpose TEXT NOT NULL,
  email TEXT NOT NULL,
  email_key TEXT NOT NULL,
  account_key TEXT REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  verified_at TIMESTAMPTZ,
  consumed_at TIMESTAMPTZ,
  delivery_id TEXT,
  idempotency_key TEXT NOT NULL UNIQUE,
  CHECK (char_length(token_hash) = 64),
  CHECK (setup_hash IS NULL OR char_length(setup_hash) = 64),
  CHECK (purpose IN ('registration', 'reset')),
  CHECK (char_length(email) BETWEEN 3 AND 320),
  CHECK (char_length(email_key) BETWEEN 3 AND 320),
  CHECK (char_length(idempotency_key) BETWEEN 16 AND 128),
  CHECK (expires_at > created_at),
  CHECK (verified_at IS NULL OR verified_at >= created_at),
  CHECK (consumed_at IS NULL OR verified_at IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_site_auth_email_actions_lookup
  ON site_auth_email_actions (email_key, purpose, created_at DESC);

CREATE TABLE IF NOT EXISTS site_auth_sessions (
  token_hash TEXT PRIMARY KEY,
  account_key TEXT NOT NULL REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
  sign_in_method TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at TIMESTAMPTZ NOT NULL,
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  revoked_at TIMESTAMPTZ,
  CHECK (char_length(token_hash) = 64),
  CHECK (char_length(sign_in_method) BETWEEN 1 AND 32),
  CHECK (expires_at > created_at)
);

CREATE INDEX IF NOT EXISTS idx_site_auth_sessions_account
  ON site_auth_sessions (account_key, expires_at DESC);

CREATE TABLE IF NOT EXISTS site_auth_attempts (
  scope TEXT NOT NULL,
  subject_hash TEXT NOT NULL,
  window_started_at TIMESTAMPTZ NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  blocked_until TIMESTAMPTZ,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (scope, subject_hash),
  CHECK (char_length(scope) BETWEEN 1 AND 32),
  CHECK (char_length(subject_hash) = 64),
  CHECK (attempts >= 0)
);
