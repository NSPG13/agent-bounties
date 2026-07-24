CREATE TABLE IF NOT EXISTS social_mention_ingestions (
  id UUID PRIMARY KEY,
  provider TEXT NOT NULL,
  provider_event_id TEXT NOT NULL,
  source_network TEXT NOT NULL,
  mention_id TEXT NOT NULL,
  mention_url TEXT NOT NULL,
  author_fid BIGINT NOT NULL,
  author_handle TEXT,
  mention_text TEXT NOT NULL,
  status TEXT NOT NULL,
  draft JSONB,
  idempotency_key TEXT,
  reply_cast_hash TEXT,
  last_error TEXT,
  reply_attempt_count INTEGER NOT NULL DEFAULT 0,
  reply_lease_token UUID,
  reply_lease_expires_at TIMESTAMPTZ,
  received_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT social_mention_provider_check CHECK (provider = 'neynar'),
  CONSTRAINT social_mention_network_check CHECK (source_network = 'farcaster'),
  CONSTRAINT social_mention_author_fid_check CHECK (author_fid > 0),
  CONSTRAINT social_mention_status_check CHECK (status IN (
    'ignored',
    'blocked',
    'draft_ready',
    'reply_pending',
    'replying',
    'reply_failed',
    'replied'
  )),
  CONSTRAINT social_mention_reply_attempt_check CHECK (reply_attempt_count >= 0),
  CONSTRAINT social_mention_provider_event_unique UNIQUE (provider, provider_event_id),
  CONSTRAINT social_mention_network_mention_unique UNIQUE (source_network, mention_id)
);

CREATE INDEX IF NOT EXISTS social_mention_ingestions_status_idx
  ON social_mention_ingestions (status, updated_at);

CREATE INDEX IF NOT EXISTS social_mention_ingestions_received_idx
  ON social_mention_ingestions (received_at DESC);
