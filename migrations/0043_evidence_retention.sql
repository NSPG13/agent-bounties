-- Each authorized participant controls their own hold. One participant cannot
-- release another's dispute, and closure does not alter canonical outcomes.
CREATE TABLE IF NOT EXISTS verification_artifact_disputes (
    id UUID PRIMARY KEY,
    artifact_id UUID NOT NULL REFERENCES verification_artifacts(id),
    principal_key TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 2000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    resolution TEXT CHECK (resolution IS NULL OR length(resolution) BETWEEN 1 AND 2000),
    CHECK ((resolved_at IS NULL) = (resolution IS NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS verification_artifact_disputes_open_idx
    ON verification_artifact_disputes(artifact_id,principal_key) WHERE resolved_at IS NULL;
