# Open Competition V2 replenishment threat model

## Scope and authority

This is an R3 path because it can spend reserved USDC. The public repository and
GitHub runner can observe and plan, but cannot sign. The isolated signer is
authorized only for exact USDC approval and reviewed factory-creation calls.
It is not authorized to settle, verify, transfer arbitrary funds, change policy,
or infer paid status.

```text
canonical safe-block sources -> private guard -> deterministic planner
                                     |                 |
public candidate specs -------------/                  |
private ranking + durable ledger (isolated service) ---/
                                                       |
                                              unsigned exact request
                                                       |
                                                isolated signer
                                                       |
                                         USDC + approved V2 factory
                                                       |
                                      canonical activation reconciliation
```

The reserve wallet holds at most 152 USDC plus minimal Base gas. GitHub stores
only an HTTPS signer endpoint and revocable bearer credential, never the wallet
key. Public output is unified and contains no mechanism-specific floor, deficit,
ranking, or private feedback.

## Assets and invariants

- Reserve principal: at most 152,000,000 USDC base units.
- Daily exposure: at most 30,400,000 base units per UTC day.
- One creation: exactly 3,000,000 solver plus 40,000 keeper base units.
- Allowed target: the reviewed Beta3 factory and release hash in the candidate
  specifications and fresh canonical inventory.
- Allowance: exact amount, consumed by the matching creation, then zero.
- Idempotency: safe block, release, candidate/spec hashes, predicted contract,
  policy epoch, and durable execution record are bound before broadcast.
- Evidence: only canonical safe-block activation and settlement events establish
  active inventory, GMV, or payment.

## Threats and mitigations

| Threat | Impact | Required mitigation |
|---|---|---|
| Stale, future, missing, or conflicting inventory | overspend or false availability | maximum 15-minute evidence age, safe block, primary/shadow agreement in the signer, fail closed |
| Release/factory substitution | arbitrary contract call | exact address/release allowlist rechecked by planner and signer |
| Duplicate worker, retry, or crash | duplicate funding | serialized workflow, content-addressed request, durable signer reservation before broadcast, predicted-address check |
| Pending broadcast followed by another batch | target overshoot | any planned/broadcast ledger record blocks new plans until reconciled |
| Tampered candidate pool or private scores | unauthorized terms | exact public spec hash, private ranking schema, twenty-ID equality, 50/30/20 weights, expiry |
| Cap or integer bypass | reserve loss | integer base units only; independent signer day/lifetime counters; exact per-call amount |
| Stuck or excessive allowance | token loss | approve exact amount immediately before the matching creation; require zero afterward; stop on mismatch |
| Runner/token compromise | repeated valid requests | signer revalidates every request, narrowly scoped revocable token, rate limit, durable idempotency, no arbitrary calldata |
| Signer compromise | reserve loss | isolated wallet with lifetime-limited balance, minimal gas, owner revocation, no settlement authority |
| AI-generated feedback or ranking | bad growth direction | at least one genuine user source and one quantitative source per candidate; AI is advisory only |
| Public artifact leakage | strategy/privacy breach | temporary runner files only; no artifact upload, body logging, public type counts, or private comments |
| Broadcast called funded/paid | false trust claim | response language says submitted only; later safe-block reconciliation is mandatory |

## Abuse tests

Tests must cover inventory counts 10, 9, 5, 4, and 0; five and six concurrent
exits; duplicate candidates and ledger keys; concurrent workers; crashes before
reservation, after reservation, after approval, after creation broadcast, and
after receipt; stale/future evidence; release changes; indexer disagreement;
candidate exhaustion; daily/lifetime caps; malformed schemas; non-integer
amounts; allowance not returning to zero; and a signer response without
canonical reconciliation.

## Detection, containment, and recovery

Alert on a floor breach, planning block, signer-policy rejection, nonzero stale
allowance, unexpected target/calldata, cap mismatch, pending execution beyond
the canonical finality window, or primary/shadow disagreement. Contain by setting
`V2_REPLENISHMENT_EXECUTE=false`, revoking the signer bearer credential, and
revoking wallet authority or moving the bounded remainder through the owner path.
Do not delete the ledger. Reconcile canonical events, mark every reservation
activated or rejected, rotate credentials, and rerun the deterministic plan.

Rollback disables future execution; it cannot undo a canonical creation or
settlement. Those records stay in inventory and GMV according to canonical state.
