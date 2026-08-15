# Open Competition V2 Beta2 Release

Beta2 is implemented but not deployed. Public creation and hosted proving stay
disabled until the release manifest contains evidence for every launch gate.
A build, transaction hash, or deployment receipt does not clear a gate.

## Proof Stack

Beta2 pins the immutable fork
`NSPG13/sp1@87c57583c77a15fa6dd191a1c6ff6947564e4ef8`, identified as
`agent-bounties-sp1-safe-v1`. The fork backports an injective Fiat-Shamir
transcript into native proving and recursion, and carries regressions for
partial-chunk padding, upper squeeze bits, and high digest bits.

Both metric Cargo roots patch `p3-challenger` and `p3-field` to that exact
commit. `scripts/verify_sp1_patched_graph.py` rejects a registry fallback,
revision drift, duplicate package, or release-identity mismatch. Dependency
review permits only `GHSA-vj64-rjf3-w3v7` because GitHub matches the retained
upstream package name and version without considering the patched source. The
exact-source graph gate and transcript attack regressions must pass; any
registry fallback or additional advisory still blocks the release.

GPU proving and the public SP1 Prover Network are disabled for Beta2. A labeled
128 GiB x86-64 Linux runner builds both circuits and project-owned Groth16 and
PLONK verifiers, then creates one Groth16 and two PLONK proofs on CPU. The
contracts call those exact verifiers directly; no gateway, proxy, owner, or
upgrade route exists.

The official SP1 installer is used only to install the compatible zkVM compiler
toolchain. CI verifies its pinned installer hash, installs SP1 6.4.0, then
overwrites `cargo-prove` with a binary compiled from the safe fork.

## Release Order

1. Run repository tests, 10,000-run Foundry fuzz/invariants, dependency review,
   and pinned Slither triage.
2. Reproduce ELF, vkey, source hash, and golden journal in two isolated Linux
   builders.
3. Commit the reproduced identity as `reproduced_beta2`; stale Beta1 values may
   never be reused.
4. On the 128 GiB CPU runner, build circuits, verifier bytecode, and three real
   self-verified proofs.
5. Replay the exact verifier and factory deployment plus both winner modes on a
   fresh Base-mainnet fork.
6. In protected environment `v2-beta2-sepolia`, deploy the same bytecode and
   rehearse first-proven, best-score, pooled funding, BYO submission, expiry,
   verifier failure, and permissionless refunds at a safe block.
7. Record owner deployment approval against the exact repository subject.
8. In protected environment `v2-beta2-mainnet`, deploy immutable verifiers and
   factory while public creation remains disabled.
9. Run the two 0.25 USDC canaries, x402 success/failure refund, fresh-wallet
   flow, and primary/shadow indexer comparison.
10. Record owner activation approval, then enable the exact runtime manifest.

The source of truth is
`deployments/open-competition-v2-beta2-release-gates.json`. Each true gate must
contain an HTTPS evidence URI, evidence hash, source commit, and repository
subject hash. The subject commits to every tracked entry except the gate
manifest itself.

## Build Stages

Verifier generation has two explicit stages:

- `verifier_bytecode_only` may predict addresses and prepare proof-bound
  fixtures. The release builder accepts it only with
  `--allow-pending-proof-evidence` and it is never deployable.
- `self_verified` contains hashes of all three CPU proof records. Only this
  state can produce a deployable release bundle.

`programs/public-vector-metric-v1/release-identity.json` is
`reproduced_beta2`: two isolated builders reproduced the pinned ELF and vkey.
Production bundle generation rejects any other state.

## Commands

```powershell
python scripts/verify_sp1_patched_graph.py
$env:PYTHONPATH = "$PWD\scripts"
python -m unittest scripts.test_build_open_competition_v2_beta2_release `
  scripts.test_build_open_competition_v2_verifier_assets -v
$env:FOUNDRY_INVARIANT_RUNS = "10000"
$env:FOUNDRY_INVARIANT_DEPTH = "50"
forge test --root contracts/base-escrow `
  --match-contract "(OpenCompetitionBountyV2Beta2Test|OpenCompetitionBountyV2Beta2InvariantTest)" `
  --fuzz-runs 10000
```

The real release runs through `.github/workflows/open-competition-v2-beta2-release.yml`:

```bash
gh workflow run open-competition-v2-beta2-release.yml --ref main \
  -f build_release_assets=true \
  -f run_live_sepolia_rehearsal=true \
  -f deploy_mainnet=true \
  -f run_mainnet_canaries=true
```

It requires a self-hosted runner with labels `linux`, `x64`, `ram-128gb`, and
`open-competition-v2-prover`. Configure protected environments as follows:

- `v2-beta2-sepolia`: `BASE_SEPOLIA_RPC_URL` and
  `BASE_SEPOLIA_DEPLOYER_PRIVATE_KEY`;
- `v2-beta2-mainnet`: `BASE_MAINNET_RPC_URL` and
  `BASE_MAINNET_DEPLOYER_PRIVATE_KEY`.

The mainnet deployment job refuses to sign unless every prelaunch gate is true,
has HTTPS hash-bound evidence, targets the exact repository subject, and the
protected environment is approved. The canary job then spends exactly 0.525
USDC on the two synthetic competitions and excludes them from adoption data.

After downloading a workflow artifact, record each passed gate without editing
hashes by hand:

```powershell
python scripts/record_open_competition_v2_beta2_gate.py `
  --gate repository_gate_complete `
  --evidence target/repository-gate.json `
  --uri https://github.com/NSPG13/agent-bounties/actions/runs/RUN_ID
```

Owner gates additionally require `--owner-risk-hash` equal to the exact risk
hash printed by the release bundle. The recorder permits changes only to the
gate manifest and binds the artifact digest, source commit, repository subject,
and HTTPS evidence location.

## Production Services

`render.yaml` defines four inert-until-configured Beta2 workers:

- the primary safe-block indexer;
- a read-only shadow indexer using an independent RPC;
- the permissionless settlement/refund keeper;
- the x402 proof broker.

The shadow process persists a canonical event-set digest and common safe-block
identity. Public creation and new broker quotes fail closed if that agreement
is absent, false, older than 120 seconds, or predates deployment.

The 128 GiB host runs
`scripts/open_competition_v2_prover_service.py` with
`ops/open-competition-v2-prover.service`. Caddy terminates HTTPS using
`ops/open-competition-v2-prover.Caddyfile`. Set:

```text
OPEN_COMPETITION_V2_PROVER_API_KEY=<random 32+ character secret>
OPEN_COMPETITION_V2_PROVER_BINARY=/opt/agent-bounties/bin/public-vector-metric-v1-script
OPEN_COMPETITION_V2_PROVER_JOB_DIR=/var/lib/agent-bounties-prover/jobs
OPEN_COMPETITION_V2_PROVER_MAX_SECONDS=600
OPEN_COMPETITION_V2_PROVER_MAX_QUEUED=2
OPEN_COMPETITION_V2_PROVER_BIND=127.0.0.1
PORT=9070
SP1_PROVER=cpu
```

Give the same API key only to the Render broker and point
`OPEN_COMPETITION_V2_PROVER_URL` to
`https://prover.agentbounties.app/v1/prove`. The service permits one proof at a
time, stores idempotent job records durably, resumes pending records after a
restart, rejects journal drift, and never enables GPU or network proving.

The initial deployment manifest has both public flags false. After immutable
deployment and live primary/shadow indexer agreement are recorded, rebuild the
same manifest to enable the broker only for internal canaries while
`public_creation_enabled` stays false. Run one paid x402 proof job and one
forced provider failure, then record canonical USDC success or refund evidence.
Only the complete public launch gate enables creation.

## Evidence Boundary

Only `CompetitionSettledV2` proves solver payment. Proof generation, broker
acceptance, transaction broadcast, database state, and individual receipts do
not. Mainnet deployment and public activation are separate approvals.
Graduation remains a later, separately announced decision and does not block
the public Beta.
