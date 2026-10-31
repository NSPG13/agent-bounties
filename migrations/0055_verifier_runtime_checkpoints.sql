-- Service authentication is separate from wallet and payout signing authority.
CREATE TABLE IF NOT EXISTS verifier_runtime_checkpoints (
    release_id TEXT NOT NULL,
    component TEXT NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    healthy BOOLEAN NOT NULL,
    PRIMARY KEY (release_id, component),
    CHECK (length(release_id) = 71),
    CHECK (length(component) <= 64)
);
