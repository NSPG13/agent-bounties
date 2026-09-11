# Synthetic async-outcome phase fixtures

Prepared for x402 delivery-receipt discussion #2833 and escrow discussion #2222.
These are application semantic test vectors, not an adopted x402 extension,
production verification code, signatures, chain receipts or evidence of real payments.

Run `python3 verify.py` offline. The verifier checks canonical bytes, SHA-256
digests, semantic expectations and one binding mutation per vector. It requires
only the Python standard library and performs no network or wallet operations.

The three primary cases are funded but not delivered, delivered but not settled,
and settled to the expected solver. Negative controls cover failed transactions
despite HTTP 200, wrong payout recipient, authorization without settlement,
duplicate event identity, and issuer-attested sequence/anchoring without a
completeness proof.

Canonicalization uses an explicitly restricted RFC 8785-compatible subset:
ASCII object keys/string values, safe integers, booleans, null and arrays. Money
and identifiers are strings. Every file is UTF-8 with sorted object properties,
no insignificant whitespace and no final newline. This is not a general JCS
implementation; non-ASCII and floating-point test cases are intentionally rejected.

`canonical`, `transaction_status`, and delivery `verified` are **mock adapter
observations**. An integrating implementation must independently validate its
chain ID, factory/contract provenance, recognized asset, confirmed/reorg-safe
transaction and event, payer/payee roles, amount, committed evidence and verifier
policy before supplying equivalent observations. The local evaluator cannot
turn caller-supplied JSON, a transaction hash, HTTP status, an authorization
nonce, a receipt signature or an existence anchor into proof of payment.

Funding is one or more contribution events. Delivery and outcome settlement are
independent evidence types. Event identity includes chain, emitting contract,
transaction and log index. The example uses the autonomous-v1 `BountySettled`
terminal event; a V2 adapter must explicitly map its own `CompetitionSettledV2`
semantics. Synthetic identifiers are intentionally not transaction-ready.

No fixture claims completeness. Showing an anchored record exists does not show
that every attempt or conflicting record was emitted. That needs a separately
specified, independently verifiable coverage commitment.

Sources:
- https://github.com/x402-foundation/x402/issues/2833
- https://github.com/x402-foundation/x402/issues/2222
- https://agentbounties.app/x402.html
- https://agentbounties.app/x402-test-vectors.json
