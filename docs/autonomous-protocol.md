# Autonomous Protocol

## Settlement Evidence Verification

The protocol requires exact canonical settlement evidence to prove a solver was paid. The verification process is handled by the `verify-settlement-evidence.mjs` script which:

1. Validates the presence of exactly one `bounty_settled` event
2. Checks all required fields match the expected values
3. Verifies the payout arithmetic is correct
4. Confirms all four committed bytes32 hashes are present

This ensures only valid and complete settlement evidence is accepted for payment.