CREATE TABLE IF NOT EXISTS agents (
  id UUID PRIMARY KEY,
  handle TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL,
  payout_wallet TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS contributor_contacts (
  id UUID PRIMARY KEY,
  github_login TEXT NOT NULL,
  github_login_normalized TEXT NOT NULL UNIQUE,
  email TEXT,
  payout_wallet TEXT,
  associated_prs JSONB NOT NULL DEFAULT '[]'::jsonb,
  contact_consent BOOLEAN NOT NULL DEFAULT false,
  wallet_consent BOOLEAN NOT NULL DEFAULT false,
  outreach_allowed BOOLEAN NOT NULL DEFAULT false,
  source TEXT NOT NULL,
  notes TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS github_login_normalized TEXT;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS email TEXT;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS payout_wallet TEXT;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS associated_prs JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS contact_consent BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS wallet_consent BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS outreach_allowed BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'operator';
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS notes TEXT;
ALTER TABLE contributor_contacts
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

UPDATE contributor_contacts
SET github_login_normalized = lower(github_login)
WHERE github_login_normalized IS NULL;

ALTER TABLE contributor_contacts
  ALTER COLUMN github_login_normalized SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_contributor_contacts_github_login_normalized
  ON contributor_contacts (github_login_normalized);

CREATE TABLE IF NOT EXISTS audience_members (
  id UUID PRIMARY KEY,
  provider TEXT NOT NULL,
  external_id TEXT NOT NULL,
  external_id_normalized TEXT NOT NULL,
  handle TEXT NOT NULL,
  public_profile_url TEXT,
  roles JSONB NOT NULL DEFAULT '[]'::jsonb,
  lifecycle_stage TEXT NOT NULL,
  first_seen_at TIMESTAMPTZ NOT NULL,
  last_seen_at TIMESTAMPTZ NOT NULL,
  UNIQUE (provider, external_id_normalized),
  CHECK (length(trim(external_id)) > 0),
  CHECK (length(trim(handle)) > 0),
  CHECK (last_seen_at >= first_seen_at)
);

CREATE INDEX IF NOT EXISTS idx_audience_members_last_seen_at
  ON audience_members (last_seen_at DESC);

CREATE TABLE IF NOT EXISTS audience_interactions (
  id UUID PRIMARY KEY,
  audience_member_id UUID NOT NULL REFERENCES audience_members(id) ON DELETE CASCADE,
  provider_event_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  public_url TEXT,
  occurred_at TIMESTAMPTZ NOT NULL,
  referrer_url TEXT,
  campaign TEXT,
  source_interaction_id UUID REFERENCES audience_interactions(id) ON DELETE SET NULL,
  UNIQUE (audience_member_id, provider_event_id),
  CHECK (length(trim(provider_event_id)) > 0)
);

CREATE INDEX IF NOT EXISTS idx_audience_interactions_member_occurred
  ON audience_interactions (audience_member_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_audience_interactions_kind_occurred
  ON audience_interactions (kind, occurred_at DESC);

CREATE TABLE IF NOT EXISTS discovery_responses (
  id UUID PRIMARY KEY,
  audience_member_id UUID NOT NULL REFERENCES audience_members(id) ON DELETE CASCADE,
  interaction_id UUID REFERENCES audience_interactions(id) ON DELETE SET NULL,
  provider_response_id TEXT NOT NULL,
  public_source_url TEXT,
  found_via TEXT NOT NULL,
  motivation TEXT NOT NULL,
  improvement_suggestion TEXT NOT NULL,
  agent_or_tool TEXT,
  private_storage_consent BOOLEAN NOT NULL DEFAULT false,
  captured_at TIMESTAMPTZ NOT NULL,
  UNIQUE (audience_member_id, provider_response_id),
  CHECK (length(trim(provider_response_id)) > 0),
  CHECK (length(trim(found_via)) > 0),
  CHECK (length(trim(motivation)) > 0),
  CHECK (length(trim(improvement_suggestion)) > 0),
  CHECK (public_source_url IS NOT NULL OR private_storage_consent)
);

CREATE TABLE IF NOT EXISTS outreach_attempts (
  id UUID PRIMARY KEY,
  audience_member_id UUID NOT NULL REFERENCES audience_members(id) ON DELETE CASCADE,
  provider_event_id TEXT NOT NULL,
  channel TEXT NOT NULL,
  public_url TEXT,
  prompt_version TEXT NOT NULL,
  status TEXT NOT NULL,
  consent_contact_id UUID REFERENCES contributor_contacts(id) ON DELETE CASCADE,
  sent_at TIMESTAMPTZ NOT NULL,
  UNIQUE (audience_member_id, provider_event_id),
  CHECK (length(trim(provider_event_id)) > 0),
  CHECK (length(trim(prompt_version)) > 0),
  CHECK (channel != 'EmailPrivate' OR consent_contact_id IS NOT NULL),
  CHECK (channel = 'EmailPrivate' OR public_url IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_outreach_attempts_member_sent
  ON outreach_attempts (audience_member_id, sent_at);

CREATE TABLE IF NOT EXISTS capabilities (
  id UUID PRIMARY KEY,
  agent_id UUID NOT NULL REFERENCES agents(id),
  class TEXT NOT NULL,
  template_slugs JSONB NOT NULL,
  min_price BIGINT NOT NULL CHECK (min_price > 0),
  max_price BIGINT NOT NULL CHECK (max_price >= min_price),
  currency TEXT NOT NULL,
  latency_seconds BIGINT NOT NULL,
  supported_verifiers JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS help_requests (
  id UUID PRIMARY KEY,
  requester_agent_id UUID NOT NULL REFERENCES agents(id),
  goal TEXT NOT NULL,
  context TEXT NOT NULL,
  budget BIGINT NOT NULL CHECK (budget > 0),
  currency TEXT NOT NULL,
  privacy TEXT NOT NULL,
  required_confidence REAL NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS bounties (
  id UUID PRIMARY KEY,
  help_request_id UUID REFERENCES help_requests(id),
  title TEXT NOT NULL,
  template_slug TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  funding_targets JSONB NOT NULL DEFAULT '[]'::jsonb,
  funding_mode TEXT NOT NULL,
  privacy TEXT NOT NULL DEFAULT 'Public',
  status TEXT NOT NULL,
  terms_hash TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE bounties
  ADD COLUMN IF NOT EXISTS privacy TEXT NOT NULL DEFAULT 'Public';
ALTER TABLE bounties
  ADD COLUMN IF NOT EXISTS funding_targets JSONB NOT NULL DEFAULT '[]'::jsonb;

CREATE TABLE IF NOT EXISTS escrows (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  rail TEXT NOT NULL,
  token TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  status TEXT NOT NULL,
  external_reference TEXT UNIQUE
);

CREATE TABLE IF NOT EXISTS funding_contributions (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  contributor_agent_id UUID REFERENCES agents(id),
  source_organization_id UUID,
  rail TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  status TEXT NOT NULL,
  funding_ledger_entry_id UUID,
  refund_ledger_entry_id UUID,
  settlement_id UUID,
  external_reference TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE funding_contributions
  ADD COLUMN IF NOT EXISTS source_organization_id UUID;
ALTER TABLE funding_contributions
  ADD COLUMN IF NOT EXISTS funding_ledger_entry_id UUID;
ALTER TABLE funding_contributions
  ADD COLUMN IF NOT EXISTS refund_ledger_entry_id UUID;
ALTER TABLE funding_contributions
  ADD COLUMN IF NOT EXISTS settlement_id UUID;

CREATE UNIQUE INDEX IF NOT EXISTS idx_funding_contributions_external_reference
  ON funding_contributions (bounty_id, external_reference)
  WHERE external_reference IS NOT NULL;

CREATE TABLE IF NOT EXISTS funding_intents (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  contributor_agent_id UUID REFERENCES agents(id),
  source_organization_id UUID,
  rail TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  status TEXT NOT NULL,
  external_reference TEXT,
  stripe_success_url TEXT,
  stripe_cancel_url TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE funding_intents
  ADD COLUMN IF NOT EXISTS stripe_success_url TEXT;
ALTER TABLE funding_intents
  ADD COLUMN IF NOT EXISTS stripe_cancel_url TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_funding_intents_external_reference
  ON funding_intents (bounty_id, external_reference)
  WHERE external_reference IS NOT NULL;

CREATE TABLE IF NOT EXISTS base_escrow_events (
  id UUID PRIMARY KEY,
  log_key TEXT NOT NULL UNIQUE,
  tx_hash TEXT NOT NULL,
  block_number BIGINT NOT NULL CHECK (block_number >= 0),
  log_index BIGINT CHECK (log_index >= 0),
  onchain_escrow_id TEXT NOT NULL,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  token TEXT,
  amount BIGINT CHECK (amount IS NULL OR amount > 0),
  currency TEXT,
  terms_hash TEXT,
  proof_hash TEXT,
  reason_hash TEXT,
  dispute_hash TEXT,
  occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS base_release_attestations (
  id UUID PRIMARY KEY,
  network TEXT NOT NULL,
  tx_hash TEXT NOT NULL,
  log_key TEXT NOT NULL,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  onchain_escrow_id TEXT NOT NULL,
  calldata_hash TEXT,
  proof_hash TEXT,
  recipients JSONB NOT NULL DEFAULT '[]'::jsonb,
  escrow_contract TEXT NOT NULL,
  settlement_signer TEXT NOT NULL,
  platform_fee_wallet TEXT,
  verdict TEXT NOT NULL,
  reason TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (network, tx_hash, log_key)
);

CREATE INDEX IF NOT EXISTS idx_base_release_attestations_bounty
  ON base_release_attestations (bounty_id, created_at);

CREATE TABLE IF NOT EXISTS base_log_cursors (
  network TEXT NOT NULL,
  escrow_contract TEXT NOT NULL,
  last_scanned_block BIGINT NOT NULL CHECK (last_scanned_block >= 0),
  last_log_key TEXT,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (network, escrow_contract)
);

CREATE TABLE IF NOT EXISTS base_indexer_heartbeats (
  network TEXT NOT NULL,
  escrow_contract TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at TIMESTAMPTZ NOT NULL,
  completed_at TIMESTAMPTZ,
  latest_block BIGINT CHECK (latest_block IS NULL OR latest_block >= 0),
  confirmed_to_block BIGINT CHECK (confirmed_to_block IS NULL OR confirmed_to_block >= 0),
  from_block BIGINT CHECK (from_block IS NULL OR from_block >= 0),
  to_block BIGINT CHECK (to_block IS NULL OR to_block >= 0),
  fetched_logs BIGINT NOT NULL DEFAULT 0 CHECK (fetched_logs >= 0),
  persisted_cursor_block BIGINT CHECK (persisted_cursor_block IS NULL OR persisted_cursor_block >= 0),
  skipped_reason TEXT,
  error_message TEXT,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (network, escrow_contract)
);

CREATE TABLE IF NOT EXISTS claims (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  solver_agent_id UUID NOT NULL REFERENCES agents(id),
  claimed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (bounty_id)
);

CREATE TABLE IF NOT EXISTS quotes (
  id UUID PRIMARY KEY,
  help_request_id UUID NOT NULL REFERENCES help_requests(id),
  solver_agent_id UUID NOT NULL REFERENCES agents(id),
  price BIGINT NOT NULL CHECK (price > 0),
  currency TEXT NOT NULL,
  estimated_seconds BIGINT NOT NULL,
  verifier_kind TEXT NOT NULL,
  confidence REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS submissions (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  solver_agent_id UUID NOT NULL REFERENCES agents(id),
  artifact_digest TEXT NOT NULL,
  artifact_uri TEXT NOT NULL,
  submitted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS verifier_results (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  submission_id UUID NOT NULL REFERENCES submissions(id),
  verifier_agent_id UUID REFERENCES agents(id),
  kind TEXT NOT NULL,
  decision TEXT NOT NULL,
  summary TEXT NOT NULL,
  confidence REAL NOT NULL,
  signed_payload_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS proof_records (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  submission_id UUID NOT NULL REFERENCES submissions(id),
  verifier_result_id UUID NOT NULL REFERENCES verifier_results(id),
  proof_hash TEXT NOT NULL,
  public_summary TEXT NOT NULL,
  privacy TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS settlements (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  proof_record_id UUID NOT NULL REFERENCES proof_records(id),
  rail TEXT NOT NULL,
  payout_intents JSONB NOT NULL,
  platform_fee BIGINT NOT NULL CHECK (platform_fee >= 0),
  currency TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE settlements DROP CONSTRAINT IF EXISTS settlements_platform_fee_check;
ALTER TABLE settlements
  ADD CONSTRAINT settlements_platform_fee_check CHECK (platform_fee >= 0);

CREATE TABLE IF NOT EXISTS reputation_events (
  id UUID PRIMARY KEY,
  agent_id UUID NOT NULL REFERENCES agents(id),
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  capability_class TEXT NOT NULL,
  template_slug TEXT NOT NULL,
  delta INTEGER NOT NULL,
  reason TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS template_signals (
  id UUID PRIMARY KEY,
  bounty_id UUID NOT NULL REFERENCES bounties(id),
  proof_record_id UUID NOT NULL REFERENCES proof_records(id),
  template_slug TEXT NOT NULL,
  capability_class TEXT NOT NULL,
  verifier_kind TEXT NOT NULL,
  amount BIGINT NOT NULL CHECK (amount > 0),
  currency TEXT NOT NULL,
  success BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS risk_events (
  id UUID PRIMARY KEY,
  subject_id UUID NOT NULL,
  agent_id UUID REFERENCES agents(id),
  bounty_id UUID,
  surface TEXT NOT NULL,
  action TEXT NOT NULL,
  score INTEGER NOT NULL,
  reasons JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE risk_events
  DROP CONSTRAINT IF EXISTS risk_events_bounty_id_fkey;

CREATE TABLE IF NOT EXISTS risk_reviews (
  id UUID PRIMARY KEY,
  risk_event_id UUID NOT NULL REFERENCES risk_events(id),
  subject_id UUID NOT NULL,
  bounty_id UUID REFERENCES bounties(id),
  surface TEXT NOT NULL,
  outcome TEXT NOT NULL,
  operator_id TEXT NOT NULL,
  note TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (risk_event_id)
);

CREATE TABLE IF NOT EXISTS ledger_entries (
  id UUID PRIMARY KEY,
  external_event_id TEXT UNIQUE,
  memo TEXT NOT NULL,
  postings JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

UPDATE funding_contributions fc
SET funding_ledger_entry_id = le.id
FROM ledger_entries le
WHERE fc.funding_ledger_entry_id IS NULL
  AND (
    le.external_event_id = 'fund-contribution:' || fc.id::text
    OR le.external_event_id = 'fund:' || fc.bounty_id::text
  );

CREATE TABLE IF NOT EXISTS payment_events (
  id UUID PRIMARY KEY,
  rail TEXT NOT NULL,
  external_id TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS eval_runs (
  id UUID PRIMARY KEY,
  suite TEXT NOT NULL,
  score REAL NOT NULL CHECK (score >= 0 AND score <= 1),
  passed BOOLEAN NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_eval_runs_created_at ON eval_runs (created_at DESC);
