# QICSWU production evidence pipeline

[`scripts/qicswu_evidence.py`](../scripts/qicswu_evidence.py) is the
fail-closed bridge between a frozen QICSWU candidate set and retained raw
Base evidence. It performs read-only JSON-RPC calls and cannot sign, broadcast,
move funds, approve a provider, infer identity, or authorize a deployment.

## Provider control registry

The candidate registry is
[`ops/metrics/qicswu-rpc-provider-registry-candidate-v1.json`](../ops/metrics/qicswu-rpc-provider-registry-candidate-v1.json).
It proposes Base's official RPC and Tenderly's Base gateway as separately
controlled archive sources. The classification is based on the operators'
documentation, but its status remains `candidate`. A valid production approval
requires two different reviewers in the `data_owner` and `security_privacy`
roles, each approving the exact registry body hash with retained evidence.
Hostname difference is never treated as organizational independence.

The checker returns exit code `2` for a structurally valid but pending registry:

```bash
python3 scripts/qicswu_evidence.py check-registry \
  --registry ops/metrics/qicswu-rpc-provider-registry-candidate-v1.json
```

## Read-only capture

Pass endpoints at runtime so credentials, URL paths, and query strings are not
stored in the evidence artifact. Each endpoint's hostname must match its exact
registry authority. The current public candidate endpoints require no secret:

```bash
python3 scripts/qicswu_evidence.py capture \
  --input ops/metrics/qicswu-baseline-input-2026-08-04-2026-09-01-v1.json \
  --registry ops/metrics/qicswu-rpc-provider-registry-candidate-v1.json \
  --rpc base_official=https://mainnet.base.org \
  --rpc tenderly_gateway=https://base.gateway.tenderly.co \
  --output target/qicswu-canonical-evidence.json
```

The capture verifies both providers' Base chain ID, safe head, transaction
receipt, settlement block, exact bounty log, decoded solver/reward amounts, and
the single bounty-to-solver USDC transfer. It retains the raw RPC results,
canonical hashes, provider IDs, and content-addressed JSON-pointer references.
It refuses to overwrite a differing artifact.

## Deliberate incomplete boundary

A successful capture proves the canonical settlement/finality layer for its
candidate set. It deliberately reports `verified_partial_blocked`, not an
available north star, while any of these remain incomplete:

- approved RPC provider-control registry;
- complete dual-RPC protocol event streams over the exact UTC window;
- funding and first-participation lifecycle receipts;
- dual-RPC payout state views;
- beneficial-principal and reimbursement adjudication;
- root-work lineage, standalone-value review, and all-history deduplication.

Those controls must be completed without changing the frozen metric semantics.
Only then may a separately reviewed policy version remove the publication gate.
Passing this tool never authorizes public reporting, a deployment, outreach,
an incentive, or spend.

## Deployed production shadow

The `QICSWU Production Shadow` workflow runs once per day over the preceding
complete UTC day and may also be dispatched for a named historical day. It
retains the current autonomous, Open Competition V1, and Open Competition V2
event streams; constructs the bounded settlement candidate input; evaluates
the fail-closed metric; verifies any observed settlement receipt, block, log,
and solver transfer against both candidate RPC providers; and uploads the
immutable internal evidence for 30 days.

Open Competition is the primary participation path. The workflow treats V1
`solution_committed` and V2 `entry_qualified` as their canonical participation
events, keeps their protocol identities in the artifacts, and fails if either
Open Competition source disappears or is redefined as claim-based. Autonomous
exclusive claims remain measured at equal unit weight so the marketplace-wide
north star is not manipulated by changing protocol labels.

This shadow is deliberately not a public dashboard. It emits a null north star
until the historical-factory stream, lifecycle, payout-state, identity,
reimbursement, and root-work gates are complete and the provider registry is
independently approved.

## Verification

```bash
python3 scripts/qicswu_evidence.py verify \
  --input ops/metrics/qicswu-baseline-input-2026-08-04-2026-09-01-v1.json \
  --registry ops/metrics/qicswu-rpc-provider-registry-candidate-v1.json \
  --bundle target/qicswu-canonical-evidence.json \
  --output target/qicswu-canonical-evidence-resolution.json

python3 -m unittest scripts.test_qicswu_evidence -v
```
