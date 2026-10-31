CREATE TABLE IF NOT EXISTS creator_push_subscriptions (
 id UUID PRIMARY KEY, account_id TEXT NOT NULL REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
 endpoint_hash TEXT NOT NULL UNIQUE CHECK (endpoint_hash ~ '^[0-9a-f]{64}$'),
 endpoint TEXT NOT NULL CHECK (char_length(endpoint) BETWEEN 20 AND 2048),
 p256dh TEXT NOT NULL CHECK (char_length(p256dh) BETWEEN 80 AND 100),
 auth TEXT NOT NULL CHECK (char_length(auth) BETWEEN 20 AND 30),
 active BOOLEAN NOT NULL DEFAULT TRUE, expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '90 days',
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS creator_push_subscriptions_account ON creator_push_subscriptions(account_id);
CREATE TABLE IF NOT EXISTS creator_push_deliveries (
 id UUID PRIMARY KEY, notification_id UUID NOT NULL REFERENCES verifier_review_notifications(id) ON DELETE CASCADE,
 subscription_id UUID NOT NULL REFERENCES creator_push_subscriptions(id) ON DELETE CASCADE,
 status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','leased','accepted','failed')),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(), lease_token UUID, lease_expires_at TIMESTAMPTZ,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE(notification_id,subscription_id),
 CHECK ((status='leased')=(lease_token IS NOT NULL AND lease_expires_at IS NOT NULL))
);
