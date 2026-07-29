CREATE TABLE IF NOT EXISTS bounty_image_assets (
  sha256 TEXT PRIMARY KEY,
  mime_type TEXT NOT NULL,
  content BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT bounty_image_assets_sha256_check CHECK (
    sha256 ~ '^[0-9a-f]{64}$'
  ),
  CONSTRAINT bounty_image_assets_mime_type_check CHECK (
    mime_type IN ('image/png', 'image/jpeg', 'image/webp')
  ),
  CONSTRAINT bounty_image_assets_content_check CHECK (
    octet_length(content) BETWEEN 1 AND 5242880
  )
);

CREATE INDEX IF NOT EXISTS bounty_image_assets_created_idx
  ON bounty_image_assets (created_at DESC, sha256);
