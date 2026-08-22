# Open Competition V2 Beta3 Release

Beta3 is deployed to Base mainnet as an opt-in public beta. On 2026-08-21, the
runtime release endpoint reported `activation_state=public_beta`, public
creation and hosted proving enabled, and agreeing primary/shadow safe-block
indexers. Those conditions are runtime state, not a permanent promise: clients
must read
`GET /v1/base/open-competition-v2-beta3/release?network=base-mainnet` and stop
unless the release remains public and `indexer_agreement.agrees=true`. A static
document, build, transaction hash, or deployment receipt does not clear or
preserve a gate.

## Proof Stack

Beta3 pins the immutable fork
`NSPG13/sp1@f6a2dffc42c322d0a6d8f5b5ae06fb76986ae12d`, identified as
`agent-bounties-sp1-safe-v5`. The fork backports an injective Fiat-Shamir
transcript into native proving and recursion, and carries regressions for
partial-chunk padding, upper squeeze bits, and high digest bits.

Both metric Cargo roots patch `p3-challenger` to that exact commit. They resolve
exactly one `p3-field 0.4.3-succinct` from the canonical registry with its pinned
checksum. `scripts/verify_sp1_patched_graph.py` rejects a challenger registry
fallback, a field fork, revision drift, duplicate package, or release-identity mismatch. Dependency
review permits only `GHSA-vj64-rjf3-w3v7` because GitHub matches the retained
upstream package name and version without considering the patched source. The
exact-source graph gate and transcript attack regressions must pass; any
registry fallback or additional advisory still blocks the release.

GPU proving and the public SP1 Prover Network are disabled for Beta3. A labeled
x86-64 Linux runner with at least 256 GiB physical memory consumes the frozen
trusted-setup bundle, builds project-owned Groth16 and PLONK verifier bytecode,
then creates one Groth16 and two
PLONK proofs on CPU. This limit comes from a measured Groth16 resident-memory
peak near 247 GiB; 128 GiB and 192 GiB runners exhausted memory or entered
release-invalid paging. Swap is emergency headroom, not qualifying capacity.
The contracts call
the exact generated verifiers directly; no gateway, proxy, owner, or upgrade
route exists.

Mainnet Groth16 keys never come from `groth16.Setup`. They are finalized from
the exact frozen R1CS with gnark's BN254 Phase 1 and circuit-specific Phase 2
MPC. At least two ephemeral contributions are required in each phase, each
contribution is hash chained, and post-contribution beacons are recorded.
`tools/open-competition-v2-ceremony` verifies both complete chains and exports
the proving key in the dump format consumed by the SP1 CPU prover. Sepolia may
use explicitly labeled single-party test assets, but the release builder
rejects them for Base mainnet.

The ceremony output includes the exact hash-bound `groth16_circuit.bin` used
to derive the keys. The release runner consumes that bundled file and never
substitutes a separately copied constraint system.

PLONK uses the Aztec Ignition public MPC KZG SRS downloaded and verified by the
pinned SP1 builder. Its circuit, proving key, verifying key, SRS transcript,
contribution evidence, and generated verifier are frozen with the Groth16
assets. `scripts/build_open_competition_v2_trusted_setup_manifest.py` binds both
systems into one manifest. The verifier-asset builder rehashes every referenced
file and mainnet generation fails unless `trusted_setup_provenance_complete` is
recorded for the exact repository subject.

The official SP1 installer is used only to install the compatible zkVM compiler
toolchain. The host builder is Rust 1.96.1 and the SP1 guest compiler reports
Rust 1.94.0-dev; both values are bound into the release identity and runtime
manifest. CI verifies the pinned installer hash, installs SP1 6.4.0, then
overwrites `cargo-prove` with a binary compiled from the safe fork.

The gnark CLI is not pulled from Succinct's circuit-version registry. The
release builds `ops/open-competition-v2-gnark-safe.Dockerfile` locally from the
exact safe-fork checkout, pins all three base-image digests and the Rust
toolchain, verifies source-commit and circuit-version labels, and publishes the
resulting image inspection plus the `/gnark-cli` SHA-256 with the proof bundle. The Dockerfile is
passed over stdin so the exact SP1 checkout remains the only image build
context, and stale generated circuit output is removed before every attempt.
The release circuit wrapper resolves the SP1 checkout before invoking either
builder because Docker bind mounts reject SP1's relative Makefile output path.

## Release Order

1. Run repository tests, 10,000-run Foundry fuzz/invariants, dependency review,
   and pinned Slither triage.
2. Reproduce ELF, vkey, source hash, and golden journal in two isolated Linux
   builders.
3. Commit the reproduced identity as `reproduced_beta3`; stale Beta1 values may
   never be reused.
4. On the capacity-gated CPU runner, verify the frozen trusted-setup bundle,
   build verifier bytecode, and create three real self-verified proofs.
5. Replay the exact verifier and factory deployment plus both winner modes on a
   fresh Base-mainnet fork.
6. In protected environment `v2-beta2-sepolia`, deploy the same bytecode and
   rehearse first-proven, best-score, pooled funding, BYO submission, expiry,
   verifier failure, and permissionless refunds at a safe block.
7. Record owner deployment approval against the exact repository subject.
8. In protected environment `v2-beta2-mainnet`, deploy immutable verifiers and
   factory while public creation remains disabled.
9. Fund a dedicated broker address with at least 0.11 USDC of segregated
   refund reserve and 0.00002 ETH for relay gas. The broker, keeper and
   deployment signer must be three distinct keys. Safe-block reserve evidence
   is required before the broker can be enabled. For the initial release, fund
   the deployment signer with 0.745 USDC and 0.0012 ETH. The protected job
   idempotently moves 0.11 USDC and 0.0001 ETH to the broker, leaving the
   signer with the exact 0.635 USDC canary and generated-agent budget plus
   deployment gas.
10. Run the two 0.25 USDC canaries, x402 success/failure refund, and
   primary/shadow indexer comparison. Derive a unique solver wallet for the
   release run and attempt, fund its bounded gas and USDC budgets, and require
   that exact wallet to pay, authorize relay, and settle without manual state
   correction before clearing the fresh-wallet gate.
11. Recheck the broker reserve, record owner activation approval, then enable
    the exact runtime manifest.

The source of truth is
`deployments/open-competition-v2-beta3-release-gates.json`. Each true gate must
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

Both stages require trusted setup for a Base-mainnet bundle. Test-only setup is
accepted only when `--allow-test-only-setup` is explicit, and the resulting
asset record is permanently marked `mainnet_eligible: false`.

Both `programs/public-vector-metric-v1/release-identity.json` and
`programs/structured-artifact-metric-v1/release-identity.json` must be
`reproduced_beta3`: two isolated builders reproduce each pinned ELF and vkey.
Production bundle generation rejects any other state.

## Commands

```powershell
python scripts/verify_sp1_patched_graph.py
$env:PYTHONPATH = "$PWD\scripts"
python -m unittest scripts.test_build_open_competition_v2_beta3_release `
  scripts.test_build_open_competition_v2_trusted_setup_manifest `
  scripts.test_build_open_competition_v2_verifier_assets -v
$env:FOUNDRY_INVARIANT_RUNS = "10000"
$env:FOUNDRY_INVARIANT_DEPTH = "50"
forge test --root contracts/base-escrow `
  --match-contract "(OpenCompetitionBountyV2Beta3Test|OpenCompetitionBountyV2Beta3InvariantTest)" `
  --fuzz-runs 10000
```

The real release runs through `.github/workflows/open-competition-v2-beta3-release.yml`:

```bash
gh workflow run open-competition-v2-beta3-release.yml --ref main \
  -f build_release_assets=true \
  -f run_live_sepolia_rehearsal=true \
  -f deploy_mainnet=true \
  -f run_mainnet_canaries=true
```

It requires a self-hosted runner with labels `linux`, `x64`, `ram-256gb`, and
`open-competition-v2-prover`, plus the verified setup bundle at
`/mnt/agent-bounties-artifacts/sp1-safe-v5-trusted`. Configure protected
environments as follows:

The environment names retain their original `beta2` labels so existing
write-only GitHub secrets remain usable. They are deployment-control labels,
not protocol or bytecode identifiers.

- `v2-beta2-sepolia`: `BASE_SEPOLIA_RPC_URL`,
  `BASE_SEPOLIA_DEPLOYER_PRIVATE_KEY`, a dedicated
  `OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY` secret, and its public
  `OPEN_COMPETITION_V2_BROKER_ADDRESS` variable. The protected job derives the
  broker address from the key, rejects deployer reuse, and seeds only its
  bounded Base Sepolia ETH relay reserve before rehearsal;
- `v2-beta2-mainnet`: `BASE_MAINNET_RPC_URL` and
  `BASE_MAINNET_DEPLOYER_PRIVATE_KEY`, plus the isolated
  `OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY` secret and the public
  `OPEN_COMPETITION_V2_BROKER_ADDRESS`, `BASE_DEPLOYER_ADDRESS`, and
  `BASE_KEEPER_ADDRESS` variables. Keeper signing authority stays in its
  existing operations environment and is not exposed to release control-plane
  jobs.

The mainnet deployment job refuses to sign unless every prelaunch gate is true,
has HTTPS hash-bound evidence, targets the exact repository subject, and the
protected environment is approved. The canary job then spends exactly 0.525
USDC on the two synthetic competitions and excludes them from adoption data.

After downloading a workflow artifact, record each passed gate without editing
hashes by hand:

```powershell
python scripts/record_open_competition_v2_beta3_gate.py `
  --gate repository_gate_complete `
  --evidence target/repository-gate.json `
  --uri https://github.com/NSPG13/agent-bounties/actions/runs/RUN_ID
```

Owner gates additionally require `--owner-risk-hash` equal to the exact risk
hash printed by the release bundle. The recorder permits changes only to the
gate manifest and binds the artifact digest, source commit, repository subject,
and HTTPS evidence location.

## Production Services

`render.yaml` defines four inert-until-configured Beta3 workers:

- the primary safe-block indexer;
- a read-only shadow indexer using an independent RPC;
- the permissionless settlement/refund keeper;
- the x402 proof broker.

The shadow process persists a canonical event-set digest and common safe-block
identity. Public creation and new broker quotes fail closed if that agreement
is absent, false, older than 120 seconds, or predates deployment.

The same high-memory host runs
`scripts/open_competition_v2_prover_service.py` with
`ops/open-competition-v2-prover.service`. Caddy terminates HTTPS using
`ops/open-competition-v2-prover.Caddyfile`. Set:

```text
OPEN_COMPETITION_V2_PROVER_API_KEY=<random 32+ character secret>
OPEN_COMPETITION_V2_PROVER_BINARIES={"public-vector-metric-v1":"/opt/agent-bounties/bin/public-vector-metric-v1-script","structured-artifact-metric-v1":"/opt/agent-bounties/bin/structured-artifact-metric-v1-script"}
OPEN_COMPETITION_V2_PROVER_JOB_DIR=/var/lib/agent-bounties-prover/jobs
OPEN_COMPETITION_V2_PROVER_MAX_SECONDS=1800
OPEN_COMPETITION_V2_PROVER_MAX_QUEUED=2
OPEN_COMPETITION_V2_PROVER_BIND=127.0.0.1
PORT=9070
SP1_PROVER=cpu
SP1_GROTH16_PK_PATH=/mnt/agent-bounties-artifacts/sp1-safe-v5-trusted/groth16/groth16_pk.bin
SP1_PLONK_PK_PATH=/mnt/agent-bounties-artifacts/sp1-safe-v5-trusted/plonk/plonk_pk.bin
```

Give the same API key only to the Render broker and point
`OPEN_COMPETITION_V2_PROVER_URL` to
`https://prover.agentbounties.app/v1/prove`. The service permits one proof at a
time, stores idempotent job records durably, resumes pending records after a
restart, rejects journal drift, and never enables GPU or network proving.

The initial deployment manifest has both public flags false. After immutable
deployment and live primary/shadow indexer agreement are recorded, rebuild the
same manifest to enable the broker only for internal canaries while
`public_creation_enabled` stays false. Render configuration rejects a broker
key that resolves to the keeper or deployment address. The release also checks
at a Base safe block that the broker controls at least 0.11 USDC for one full
refund and 0.00002 ETH for bounded relay gas. Run one paid x402 proof job and one
forced provider failure, then record canonical USDC success or refund evidence.
Only the complete public launch gate enables creation.

## Evidence Boundary

Only `CompetitionSettledV2` proves solver payment. Proof generation, broker
acceptance, transaction broadcast, database state, and individual receipts do
not. Mainnet deployment and public activation are separate approvals.
Graduation remains a later, separately announced decision and does not block
the public Beta.
