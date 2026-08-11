CREATE TABLE IF NOT EXISTS objective_aggregates (
  id UUID PRIMARY KEY,
  schema_version TEXT NOT NULL,
  revision BIGINT NOT NULL CHECK (revision > 0),
  status TEXT NOT NULL,
  requesting_party_id UUID NOT NULL,
  record JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL,
  CHECK (schema_version = 'agent-bounties/objective-v1')
);

CREATE INDEX IF NOT EXISTS idx_objective_aggregates_status_updated
  ON objective_aggregates (status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_objective_aggregates_requesting_party
  ON objective_aggregates (requesting_party_id, created_at DESC);
