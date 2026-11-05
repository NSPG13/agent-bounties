-- One durable financial outcome per processor object, independent of event IDs.
CREATE TABLE IF NOT EXISTS stripe_reconciled_objects (
    object_kind TEXT NOT NULL CHECK (object_kind IN ('checkout.session', 'transfer')),
    object_id TEXT NOT NULL CHECK (length(object_id) BETWEEN 1 AND 255),
    binding_hash TEXT NOT NULL CHECK (binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    outcome JSONB NOT NULL CHECK (jsonb_typeof(outcome) = 'object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (object_kind, object_id)
);
