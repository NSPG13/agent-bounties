CREATE TABLE IF NOT EXISTS chatgpt_action_intents (
  id UUID PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  action TEXT NOT NULL,
  network TEXT NOT NULL,
  opportunity_id TEXT,
  bounty_contract TEXT,
  bounty_id TEXT,
  actor_wallet TEXT,
  amount_base_units BIGINT,
  details JSONB NOT NULL DEFAULT '{}'::jsonb,
  request_fingerprint TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'review_required',
  transaction_hash TEXT,
  canonical_event_id UUID REFERENCES autonomous_bounty_events(id),
  canonical_event_kind TEXT,
  confirmed_block BIGINT,
  expires_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT chatgpt_action_intents_action_check CHECK (
    action IN ('post', 'fund', 'compete', 'complete', 'verify')
  ),
  CONSTRAINT chatgpt_action_intents_network_check CHECK (
    network IN ('base-mainnet', 'base-sepolia')
  ),
  CONSTRAINT chatgpt_action_intents_opportunity_id_check CHECK (
    opportunity_id IS NULL
    OR (
      length(opportunity_id) BETWEEN 1 AND 200
      AND opportunity_id ~ '^[A-Za-z0-9:._-]+$'
    )
  ),
  CONSTRAINT chatgpt_action_intents_bounty_contract_check CHECK (
    bounty_contract IS NULL OR bounty_contract ~ '^0x[0-9a-f]{40}$'
  ),
  CONSTRAINT chatgpt_action_intents_bounty_id_check CHECK (
    bounty_id IS NULL OR bounty_id ~ '^0x[0-9a-f]{64}$'
  ),
  CONSTRAINT chatgpt_action_intents_actor_wallet_check CHECK (
    actor_wallet IS NULL OR actor_wallet ~ '^0x[0-9a-f]{40}$'
  ),
  CONSTRAINT chatgpt_action_intents_amount_check CHECK (
    amount_base_units IS NULL OR amount_base_units > 0
  ),
  CONSTRAINT chatgpt_action_intents_details_check CHECK (
    jsonb_typeof(details) = 'object'
    AND pg_column_size(details) <= 16384
  ),
  CONSTRAINT chatgpt_action_intents_fingerprint_check CHECK (
    request_fingerprint ~ '^[0-9a-f]{64}$'
  ),
  CONSTRAINT chatgpt_action_intents_status_check CHECK (
    status IN (
      'review_required',
      'pending_confirmation',
      'confirmed',
      'failed',
      'expired'
    )
  ),
  CONSTRAINT chatgpt_action_intents_transaction_hash_check CHECK (
    transaction_hash IS NULL OR transaction_hash ~ '^0x[0-9a-f]{64}$'
  ),
  CONSTRAINT chatgpt_action_intents_canonical_kind_check CHECK (
    canonical_event_kind IS NULL
    OR canonical_event_kind IN (
      'canonical_bounty_created',
      'funding_added',
      'bounty_claimed',
      'submission_added',
      'submission_rejected',
      'bounty_settled'
    )
  ),
  CONSTRAINT chatgpt_action_intents_confirmation_check CHECK (
    (
      status = 'confirmed'
      AND transaction_hash IS NOT NULL
      AND canonical_event_id IS NOT NULL
      AND canonical_event_kind IS NOT NULL
      AND confirmed_block IS NOT NULL
    )
    OR status <> 'confirmed'
  )
);

CREATE INDEX IF NOT EXISTS chatgpt_action_intents_status_idx
  ON chatgpt_action_intents (status, expires_at, created_at);

CREATE INDEX IF NOT EXISTS chatgpt_action_intents_transaction_idx
  ON chatgpt_action_intents (network, transaction_hash)
  WHERE transaction_hash IS NOT NULL;
