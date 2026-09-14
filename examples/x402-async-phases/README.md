# Synthetic async-outcome phase fixtures

Prepared for x402 delivery-receipt discussion #2833 and escrow discussion #2222.
These are application semantic test vectors, not an adopted x402 extension,
production verification code, signatures, chain receipts or evidence of real payments.

Run `python3 verify.py` offline. The verifier checks canonical bytes, SHA-256
digests, semantic expectations and lifecycle-binding negative controls. It requires
only the Python standard library and performs no network or wallet operations.

The three primary cases are funded but not delivered, delivered but not settled,
and settled to the expected solver. Negative controls cover failed transactions
despite HTTP 200, wrong payout recipient, authorization without settlement,
duplicate event identity, and issuer-attested sequence/anchoring without a
completeness proof. A ninth case combines delivery from lifecycle A with funding
and settlement from lifecycle B while all content hashes match. Its result is
`funded=true, delivered=false, settled=true`: settlement for B remains a separate
fact, but delivery from A does not count toward B.

Canonicalization uses an explicitly restricted RFC 8785-compatible subset:
ASCII object keys/string values, safe integers, booleans, null and arrays. Money
and identifiers are strings. Every file is UTF-8 with sorted object properties,
no insignificant whitespace and no final newline. This is not a general JCS
implementation; non-ASCII and floating-point test cases are intentionally rejected.

`canonical`, `transaction_status`, delivery `verified`, and `lifecycle_verified`
are **mock adapter observations**. An integrating implementation must independently validate its
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

`binding.lifecycle` selects one lifecycle instance by a chain event anchor:
`chain`, `contract`, `tx_hash`, and `log_index`. Each event and delivery observation
must carry that same identity and `lifecycle_verified=true`. This flag models
verification of the observation's relationship to the selected instance, beyond
checking the content or the existence of an anchor. Funding and settlement have
their own event identities; they need not occur in the anchor transaction.

A real adapter must derive this identity from authenticated protocol state and
verify that the delivery commitment or verifier attestation binds to that exact
instance. Its protocol rules must also establish which funding and settlement
events apply to it. A caller-selected tag, copied event locator, matching content
hash, or boolean cannot establish this relationship. Repeated attempts may reuse
content and a contract, so the adapter must select an occurrence-specific anchor.
This example supplies no such cryptographic or chain verifier.

The original eight phase expectations are unchanged; their fixtures now include
explicit mock lifecycle bindings and their canonical digests have been updated.
Missing, unverified, malformed or mismatched lifecycle identities fail closed.
The built-in controls test all four identity components, missing delivery/event
bindings, unverified bindings, and malformed shared anchors. Integrations using
older fixture bytes must establish the binding rather than infer it from hashes.

No fixture claims completeness. Showing an anchored record exists does not show
that every attempt or conflicting record was emitted. That needs a separately
specified, independently verifiable coverage commitment.

Sources:
- https://github.com/x402-foundation/x402/issues/2833
- https://github.com/x402-foundation/x402/issues/2222
- https://agentbounties.app/x402.html
- https://agentbounties.app/x402-test-vectors.json
