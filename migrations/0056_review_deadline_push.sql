-- Separate from initial push deliveries so older processes remain compatible.
CREATE TABLE IF NOT EXISTS review_deadline_push_deliveries (
 id UUID PRIMARY KEY,
 notification_id UUID NOT NULL REFERENCES verifier_review_notifications(id) ON DELETE CASCADE,
 subscription_id UUID NOT NULL REFERENCES creator_push_subscriptions(id) ON DELETE CASCADE,
 stage TEXT NOT NULL CHECK (stage IN ('remaining_24h','remaining_6h','remaining_1h','overdue')),
 status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','leased','accepted','failed')),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(), lease_token UUID, lease_expires_at TIMESTAMPTZ,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE(notification_id,subscription_id,stage),
 CHECK ((status='leased')=(lease_token IS NOT NULL AND lease_expires_at IS NOT NULL))
);
