ALTER TABLE recovery_attempts
  ADD COLUMN IF NOT EXISTS authorized_signer TEXT;

UPDATE recovery_attempts
SET authorized_signer = '0xc49e5374f0072abc0b4c134b2fd413d87aa6354a'
WHERE authorized_signer IS NULL;

ALTER TABLE recovery_attempts
  ALTER COLUMN authorized_signer SET NOT NULL;

ALTER TABLE recovery_attempts
  DROP CONSTRAINT IF EXISTS recovery_attempts_authorized_signer_pin;

ALTER TABLE recovery_attempts
  ADD CONSTRAINT recovery_attempts_authorized_signer_pin
  CHECK (lower(authorized_signer) = '0xc49e5374f0072abc0b4c134b2fd413d87aa6354a');
