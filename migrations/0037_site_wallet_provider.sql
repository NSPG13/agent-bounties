ALTER TABLE site_auth_wallets ADD COLUMN IF NOT EXISTS provider_id TEXT;
ALTER TABLE site_auth_wallets ADD COLUMN IF NOT EXISTS last_verified_at TIMESTAMPTZ;
