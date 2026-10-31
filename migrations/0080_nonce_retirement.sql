-- Append-only operator retirement evidence. Original outbox identity is never rewritten.
CREATE TABLE IF NOT EXISTS nonce_retirement_plans (
 id UUID PRIMARY KEY,
 operation_id UUID NOT NULL REFERENCES relay_transaction_outbox(operation_id),
 digest TEXT NOT NULL UNIQUE CHECK(digest ~ '^0x[0-9a-f]{64}$'),
 network TEXT NOT NULL CHECK(network='base-mainnet'),
 relayer_address TEXT NOT NULL CHECK(relayer_address ~ '^0x[0-9a-f]{40}$'),
 nonce BIGINT NOT NULL CHECK(nonce>=0),
 plan JSONB NOT NULL CHECK(jsonb_typeof(plan)='object'),
 observation JSONB NOT NULL CHECK(jsonb_typeof(observation)='object'),
 created_at TIMESTAMPTZ NOT NULL,
 expires_at TIMESTAMPTZ NOT NULL,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '15 minutes'),
 UNIQUE(id,operation_id),
 UNIQUE(id,digest)
);
CREATE TABLE IF NOT EXISTS nonce_retirement_approvals (
 plan_id UUID PRIMARY KEY REFERENCES nonce_retirement_plans(id),
 operation_id UUID NOT NULL REFERENCES relay_transaction_outbox(operation_id),
 approved_digest TEXT NOT NULL REFERENCES nonce_retirement_plans(digest),
 operator_identity TEXT NOT NULL CHECK(length(operator_identity) BETWEEN 1 AND 128),
 lease_token UUID NOT NULL,
 observation JSONB NOT NULL CHECK(jsonb_typeof(observation)='object'),
 budget_day DATE NOT NULL,
 reserved_micro_usdc BIGINT NOT NULL CHECK(reserved_micro_usdc BETWEEN 1 AND 1000000),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 expires_at TIMESTAMPTZ NOT NULL,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '120 seconds'),
 FOREIGN KEY(plan_id,operation_id) REFERENCES nonce_retirement_plans(id,operation_id),
 FOREIGN KEY(plan_id,approved_digest) REFERENCES nonce_retirement_plans(id,digest)
);
CREATE TABLE IF NOT EXISTS nonce_retirement_envelopes (
 operation_id UUID PRIMARY KEY REFERENCES relay_transaction_outbox(operation_id),
 plan_id UUID NOT NULL UNIQUE REFERENCES nonce_retirement_approvals(plan_id),
 tx_hash TEXT NOT NULL UNIQUE CHECK(tx_hash ~ '^0x[0-9a-f]{64}$'),
 signed_transaction TEXT NOT NULL CHECK(signed_transaction ~ '^0x([0-9a-f]{2})+$'),
 transaction_metadata JSONB NOT NULL CHECK(jsonb_typeof(transaction_metadata)='object'),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(plan_id,operation_id) REFERENCES nonce_retirement_plans(id,operation_id)
);
CREATE TABLE IF NOT EXISTS nonce_retirement_send_events (
 operation_id UUID PRIMARY KEY REFERENCES nonce_retirement_envelopes(operation_id),
 observation JSONB NOT NULL CHECK(jsonb_typeof(observation)='object'),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE IF NOT EXISTS nonce_retirement_confirmations (
 operation_id UUID PRIMARY KEY REFERENCES nonce_retirement_envelopes(operation_id),
 tx_hash TEXT NOT NULL CHECK(tx_hash ~ '^0x[0-9a-f]{64}$'),
 observation JSONB NOT NULL CHECK(jsonb_typeof(observation)='object'),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE OR REPLACE VIEW nonce_retirement_fences AS
 SELECT DISTINCT p.network,p.relayer_address FROM nonce_retirement_plans p
 WHERE (p.expires_at>clock_timestamp() OR EXISTS(SELECT 1 FROM nonce_retirement_envelopes e WHERE e.operation_id=p.operation_id))
 AND NOT EXISTS(SELECT 1 FROM nonce_retirement_confirmations c WHERE c.operation_id=p.operation_id);

-- Triggers preserve INSERT ... ON CONFLICT compatibility on existing journals.
CREATE OR REPLACE FUNCTION nonce_retirement_guard_outbox() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 -- UPDATE already holds an outbox row. Do not wait in the reverse of the
 -- recovery lease->outbox order: an occupied lease fails this write closed.
 BEGIN
  PERFORM 1 FROM x402_relayer_leases WHERE network=NEW.network FOR UPDATE NOWAIT;
 EXCEPTION WHEN lock_not_available THEN RETURN NULL;
 END;
 IF EXISTS(SELECT 1 FROM nonce_retirement_fences f WHERE f.network=NEW.network AND f.relayer_address=NEW.relayer_address) THEN
  IF TG_OP='INSERT' THEN RETURN NULL; END IF;
  IF NEW.send_attempts>OLD.send_attempts THEN RETURN NULL; END IF;
 END IF;
 IF TG_OP='UPDATE' THEN
  IF NEW.send_attempts>OLD.send_attempts AND EXISTS(SELECT 1 FROM nonce_retirement_plans p WHERE p.operation_id=OLD.operation_id) THEN RETURN NULL; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE OR REPLACE TRIGGER nonce_retirement_guard_outbox BEFORE INSERT OR UPDATE ON relay_transaction_outbox FOR EACH ROW EXECUTE FUNCTION nonce_retirement_guard_outbox();
CREATE OR REPLACE FUNCTION nonce_retirement_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RETURN NULL; END $$;
CREATE OR REPLACE TRIGGER nonce_retirement_plans_immutable BEFORE UPDATE OR DELETE ON nonce_retirement_plans FOR EACH ROW EXECUTE FUNCTION nonce_retirement_append_only();
CREATE OR REPLACE TRIGGER nonce_retirement_approvals_immutable BEFORE UPDATE OR DELETE ON nonce_retirement_approvals FOR EACH ROW EXECUTE FUNCTION nonce_retirement_append_only();
CREATE OR REPLACE TRIGGER nonce_retirement_envelopes_immutable BEFORE UPDATE OR DELETE ON nonce_retirement_envelopes FOR EACH ROW EXECUTE FUNCTION nonce_retirement_append_only();
CREATE OR REPLACE TRIGGER nonce_retirement_send_events_immutable BEFORE UPDATE OR DELETE ON nonce_retirement_send_events FOR EACH ROW EXECUTE FUNCTION nonce_retirement_append_only();
CREATE OR REPLACE TRIGGER nonce_retirement_confirmations_immutable BEFORE UPDATE OR DELETE ON nonce_retirement_confirmations FOR EACH ROW EXECUTE FUNCTION nonce_retirement_append_only();
