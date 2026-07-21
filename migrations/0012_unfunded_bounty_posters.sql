ALTER TABLE trial_bounties
  ADD COLUMN IF NOT EXISTS poster_status TEXT NOT NULL DEFAULT 'disabled'
    CHECK (poster_status IN ('disabled', 'pending', 'ready', 'failed')),
  ADD COLUMN IF NOT EXISTS poster_image BYTEA,
  ADD COLUMN IF NOT EXISTS poster_content_type TEXT,
  ADD COLUMN IF NOT EXISTS poster_error TEXT,
  ADD COLUMN IF NOT EXISTS poster_generated_at TIMESTAMPTZ;

ALTER TABLE trial_bounties
  ADD CONSTRAINT trial_bounties_poster_ready_fields
  CHECK (
    poster_status <> 'ready'
    OR (poster_image IS NOT NULL AND poster_content_type IS NOT NULL AND poster_generated_at IS NOT NULL)
  );
