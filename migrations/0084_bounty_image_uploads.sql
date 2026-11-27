-- Signed-in website uploads of bounty images. The image bytes stay in the
-- content-addressed bounty_image_assets table. This records which account
-- uploaded which digest and when, for the per-account hourly upload limit.
CREATE TABLE IF NOT EXISTS bounty_image_uploads (
    id BIGSERIAL PRIMARY KEY,
    account_id TEXT NOT NULL CHECK (length(account_id) BETWEEN 1 AND 128),
    sha256 TEXT NOT NULL REFERENCES bounty_image_assets (sha256),
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS bounty_image_uploads_account_time_idx
    ON bounty_image_uploads (account_id, uploaded_at DESC);
