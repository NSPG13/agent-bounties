-- New, explicit authority for initial dispatch of immutable, never-attempted bytes.
-- Historical retirement plans, approvals, envelopes and the common send latch stay intact.
CREATE UNIQUE INDEX IF NOT EXISTS nonce_retirement_envelope_binding
 ON nonce_retirement_envelopes(operation_id,plan_id,tx_hash);

CREATE TABLE IF NOT EXISTS retained_nonce_dispatch_proposals (
 id UUID PRIMARY KEY,
 operation_id UUID NOT NULL,
 retirement_plan_id UUID NOT NULL,
 tx_hash TEXT NOT NULL,
 digest TEXT NOT NULL UNIQUE CHECK(digest ~ '^0x[0-9a-f]{64}$'),
 proposal JSONB NOT NULL CHECK(jsonb_typeof(proposal)='object'),
 observation JSONB NOT NULL CHECK(jsonb_typeof(observation)='object'),
 created_at TIMESTAMPTZ NOT NULL,
 expires_at TIMESTAMPTZ NOT NULL,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '15 minutes'),
 FOREIGN KEY(operation_id,retirement_plan_id,tx_hash)
  REFERENCES nonce_retirement_envelopes(operation_id,plan_id,tx_hash),
 UNIQUE(id,operation_id,digest)
);

CREATE TABLE IF NOT EXISTS retained_nonce_dispatch_authorizations (
 proposal_id UUID PRIMARY KEY,
 operation_id UUID NOT NULL UNIQUE REFERENCES nonce_retirement_send_events(operation_id),
 approved_digest TEXT NOT NULL,
 operator_identity TEXT NOT NULL CHECK(length(operator_identity) BETWEEN 1 AND 128),
 lease_token UUID NOT NULL,
 observation JSONB NOT NULL CHECK(jsonb_typeof(observation)='object'),
 budget_day DATE NOT NULL,
 reserved_micro_usdc BIGINT NOT NULL CHECK(reserved_micro_usdc BETWEEN 1 AND 1000000),
 created_at TIMESTAMPTZ NOT NULL,
 expires_at TIMESTAMPTZ NOT NULL,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '120 seconds'),
 FOREIGN KEY(proposal_id,operation_id,approved_digest)
  REFERENCES retained_nonce_dispatch_proposals(id,operation_id,digest)
);

CREATE OR REPLACE TRIGGER retained_nonce_dispatch_proposals_immutable
 BEFORE UPDATE OR DELETE ON retained_nonce_dispatch_proposals
 FOR EACH ROW EXECUTE FUNCTION nonce_retirement_append_only();
CREATE OR REPLACE TRIGGER retained_nonce_dispatch_authorizations_immutable
 BEFORE UPDATE OR DELETE ON retained_nonce_dispatch_authorizations
 FOR EACH ROW EXECUTE FUNCTION nonce_retirement_append_only();
