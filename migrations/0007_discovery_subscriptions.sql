ALTER TABLE webhook_subscriptions
  ADD COLUMN IF NOT EXISTS subscription_kind TEXT NOT NULL DEFAULT 'agent_wallet'
    CHECK (subscription_kind IN ('agent_wallet', 'public_discovery'));

ALTER TABLE webhook_subscriptions
  ADD COLUMN IF NOT EXISTS filters JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE webhook_subscriptions
  ADD COLUMN IF NOT EXISTS management_token_hash TEXT;

CREATE INDEX IF NOT EXISTS idx_webhook_subscriptions_public_discovery
  ON webhook_subscriptions (enabled, created_at, id)
  WHERE subscription_kind = 'public_discovery';

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_subscription_created
  ON webhook_deliveries (subscription_id, created_at DESC);
