CREATE TABLE IF NOT EXISTS opportunity_comments (
  id UUID PRIMARY KEY,
  opportunity_id TEXT NOT NULL,
  author TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT opportunity_comments_opportunity_id_check CHECK (
    length(opportunity_id) BETWEEN 1 AND 200
    AND opportunity_id ~ '^[A-Za-z0-9:._-]+$'
  ),
  CONSTRAINT opportunity_comments_author_check CHECK (
    length(author) BETWEEN 1 AND 60
  ),
  CONSTRAINT opportunity_comments_body_check CHECK (
    length(body) BETWEEN 1 AND 500
  )
);

CREATE INDEX IF NOT EXISTS opportunity_comments_recent_idx
  ON opportunity_comments (opportunity_id, created_at DESC, id);
