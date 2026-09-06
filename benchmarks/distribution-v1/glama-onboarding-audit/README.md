# Glama onboarding audit benchmark

This benchmark verifies the public evidence bundle for the attributed Glama
Agent Bounties onboarding canary. It is intentionally task-specific: it must
not be reused to verify an unrelated bounty.

The solver submission must contain these files at its repository root:

- `glama-onboarding-audit.json` — structured evidence using
  `agent-bounties/glama-onboarding-audit-evidence-v1`.
- `glama-onboarding-audit.md` — the linked public audit report.

Run the immutable verifier from a submitted source snapshot with:

```text
python /benchmark/check.py
```

The sandbox mounts the immutable benchmark at `/benchmark`, mounts the solver
snapshot read-only at `/workspace`, disables networking, and sets
`WORKSPACE_ROOT=/workspace`. The verifier accepts only a Glama first-touch
session against the canonical attributed MCP and install URLs, redacted MCP
initialize and tools/list evidence, an explicit wallet-authority boundary, a
synthetic-canary measurement exclusion, and linked Base-mainnet creation,
funding, verifier-evidence, and settlement proofs for one bounty contract.

The verifier validates the evidence bundle's shape and internal consistency,
but that is not enough to authorize a payment decision. This benchmark is
therefore fail-closed in both the bounty handoff and signing pipeline until the
referenced Base receipts and logs are independently reconciled and bound to the
declared contract and bounty id. A passing sandbox receipt, signature, or
transaction hash is not proof of payment. Only a confirmed canonical
`BountySettled` event is settlement evidence.

Run the deterministic known-good and known-bad rehearsal with:

```text
python benchmarks/distribution-v1/glama-onboarding-audit/self_test.py
```
