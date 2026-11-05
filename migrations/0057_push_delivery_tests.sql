CREATE TABLE IF NOT EXISTS push_delivery_tests (
 id UUID PRIMARY KEY,
 account_id TEXT NOT NULL REFERENCES site_auth_accounts(account_key) ON DELETE CASCADE,
 subscription_id UUID NOT NULL REFERENCES creator_push_subscriptions(id) ON DELETE CASCADE,
 subscription_updated_at TIMESTAMPTZ NOT NULL,
 status TEXT NOT NULL CHECK (status IN ('attempting','accepted','failed','unknown','cancelled')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS push_delivery_tests_account_time ON push_delivery_tests(account_id,created_at);
