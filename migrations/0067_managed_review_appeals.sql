-- Card bounty review lifecycle. An acceptance is signed when recorded. A customer rejection waits
-- unsigned for the solver's appeal until appeal_ends_at, then the review key signs it, or the
-- arbiter's ruling on the appeal is signed instead. Silence after the decision period records a
-- platform acceptance (decided_by deadline). The signed_* columns describe the one attestation
-- the review key released for the round. Nothing here is ever replaced or deleted. This runner
-- splits files on semicolons, so comments here must not contain one.
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS decided_by TEXT NOT NULL DEFAULT 'customer';
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS appeal_ends_at BIGINT
    CHECK (appeal_ends_at IS NULL OR appeal_ends_at > 0);
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS appeal JSONB
    CHECK (appeal IS NULL OR jsonb_typeof(appeal) = 'object');
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS appeal_hash TEXT
    CHECK (appeal_hash IS NULL OR appeal_hash ~ '^0x[0-9a-f]{64}$');
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS appeal_signature TEXT
    CHECK (appeal_signature IS NULL OR (appeal_signature ~ '^0x[0-9a-f]+$'
        AND length(appeal_signature) % 2 = 0 AND length(appeal_signature) <= 8194));
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS appealed_at TIMESTAMPTZ;
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS ruling JSONB
    CHECK (ruling IS NULL OR jsonb_typeof(ruling) = 'object');
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS ruled_by TEXT
    CHECK (ruled_by IS NULL OR length(ruled_by) BETWEEN 1 AND 200);
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS ruled_at TIMESTAMPTZ;
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS signed_passed BOOLEAN;
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS signed_response_hash TEXT
    CHECK (signed_response_hash IS NULL OR signed_response_hash ~ '^0x[0-9a-f]{64}$');
ALTER TABLE managed_customer_reviews ADD COLUMN IF NOT EXISTS signed_at TIMESTAMPTZ;
-- Rows from 0065 were signed verdicts when recorded.
UPDATE managed_customer_reviews
   SET signed_passed = (verdict->>'passed')::boolean, signed_response_hash = response_hash,
       signed_at = recorded_at
 WHERE signature IS NOT NULL AND signed_response_hash IS NULL;
ALTER TABLE managed_customer_reviews ALTER COLUMN verifier DROP NOT NULL;
ALTER TABLE managed_customer_reviews ALTER COLUMN signature DROP NOT NULL;
ALTER TABLE managed_customer_reviews DROP CONSTRAINT IF EXISTS managed_review_lifecycle_check;
ALTER TABLE managed_customer_reviews ADD CONSTRAINT managed_review_lifecycle_check CHECK (
    decided_by IN ('customer', 'deadline')
    AND jsonb_typeof(verdict->'passed') = 'boolean'
    AND (decided_by = 'customer' OR (verdict->>'passed')::boolean)
    AND (signature IS NULL) = (verifier IS NULL)
    AND (signature IS NULL) = (signed_passed IS NULL)
    AND (signature IS NULL) = (signed_response_hash IS NULL)
    AND (signature IS NULL) = (signed_at IS NULL)
    AND ((verdict->>'passed')::boolean = false OR signature IS NOT NULL)
    AND ((verdict->>'passed')::boolean = false OR appeal_ends_at IS NULL)
    AND (appeal IS NULL) = (appeal_hash IS NULL)
    AND (appeal IS NULL) = (appeal_signature IS NULL)
    AND (appeal IS NULL) = (appealed_at IS NULL)
    AND (appeal IS NULL OR appeal_ends_at IS NOT NULL)
    AND (ruling IS NULL) = (ruled_by IS NULL)
    AND (ruling IS NULL) = (ruled_at IS NULL)
    AND (ruling IS NULL OR appeal IS NOT NULL)
    AND (ruling IS NULL OR signature IS NOT NULL)
    AND (appeal IS NULL OR ruling IS NOT NULL OR signature IS NULL)
);
CREATE INDEX IF NOT EXISTS managed_review_awaiting_signature
    ON managed_customer_reviews(appeal_ends_at) WHERE signature IS NULL;
-- One card order per committed bounty, found by its public address.
CREATE UNIQUE INDEX IF NOT EXISTS managed_funding_bounty
    ON managed_bounty_funding_operations ((plan->>'network'), (plan->>'predicted_bounty'));
