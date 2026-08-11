# Open Competition V2 Beta1 Release

V2 Beta1 is implemented but blocked from public mainnet creation. The release
is R4: immutable contracts hold Base USDC, call SP1 gateways, and can settle
without operator approval. A deployment transaction is not release approval.

SP1 6.3.1 is currently quarantined by
`scripts/verify_sp1_advisory_quarantine.py` because its transcript dependency
remains covered by high-severity advisory `GHSA-vj64-rjf3-w3v7`. This Beta may
be built and reviewed, but it cannot satisfy the resolved-high gate, graduate,
or activate mainnet until compatible patched prover and verifier evidence
replaces that quarantine.

CI downloads the official SP1 installer to a file and requires SHA-256
`5f2b976287501d3f5feb62a2a96bbdfd1f5232c9badaf7547ed837c0366f3a7b`
before executing it. One exact compiled prover runner is then checksummed,
published as a workflow artifact, and reused by isolated proof jobs. The runner
advertises machine-readable `cpu` and `network` capabilities. A local CPU
backend produces Groth16. PLONK uses the SP1 Prover Network because SP1 6.3.1
documents at least 64 GB for PLONK and GitHub's standard public runner has only
16 GB. Swap is not treated as proof capacity. A trusted release dispatch fails
before submitting a proof request unless `SP1_NETWORK_PRIVATE_KEY` is present
and the runner was compiled with the pinned SDK's `network` feature.
An installer-byte change fails closed and requires a separately reviewed pin.
The runner uses Rust 1.96.1 because the locked SP1 network-client graph
requires Rust 1.94.1 or newer. Rust, SP1, source, ELF, and vkey identity are
bound together in
`programs/public-vector-metric-v1/release-identity.json`. Changing any build
toolchain component requires two fresh isolated builds and a new reviewed
identity; a prior ELF or vkey is never silently reused.

The Solidity compiler is published as the immutable image
`docker.io/ethereum/solc@sha256:0158f0b11d4cd88556af7eff7b76e98c1c058d4a3153fae342e3a90b75358be4`.
It reports `0.8.26+commit.8a97fa7a`. Every release bundle binds that full
build ID, image digest, optimizer settings, and EVM target.

The Prover Network signer must be separately funded with sufficient PROVE.
Neither a configured secret nor a submitted provider request is proof evidence.
Only a returned proof that self-verifies, matches the exact release fixture,
and succeeds through the fork or Sepolia transaction replay satisfies a proof
gate. See the official [hardware requirements](https://docs.succinct.xyz/docs/sp1/getting-started/hardware-requirements)
and [Prover Network quickstart](https://docs.succinct.xyz/docs/sp1/prover-network/quickstart).

## Evidence Order

1. Run the repository gate and 10,000-run V2 fuzz/invariant suite.
2. Reproduce the SP1 ELF, vkey, and golden journal in two isolated builders.
3. Triage static analysis against the V2 threat model.
4. Build safe-block Base Sepolia and Base mainnet release bundles.
5. Replay the exact mainnet deployment calldata and runtimes on a fork.
6. Generate real Groth16 and PLONK proofs and rehearse first-proven,
   best-score, pooled funding, expiry, and refunds.
7. Run the same paths on Base Sepolia and reconcile at a safe block.
8. Complete three independent hash-bound reviews and the graduation evidence.
9. Announce a separate mainnet Beta review. Never change Beta1 bytecode or a
   vkey in place.

The source of truth is
`deployments/open-competition-v2-beta1-release-gates.json`. A gate is true only
when its same-named `evidence` entry records an exact source commit, evidence
hash, and public HTTPS artifact. The release builder rejects an unevidenced
true Boolean. Gates are staged so Beta launch does not depend on evidence that
can exist only after launch:

1. **Prelaunch** gates authorize only signing the immutable factory deployment.
   They include repository, reproducibility, static-analysis, Base Sepolia,
   mainnet-fork, resolved high-severity findings, and deployment-review evidence.
2. **Public Beta** gates additionally require both 0.25 USDC mainnet canaries,
   exact canary accounting, production-indexer agreement, and a separately
   announced activation review before public creation is enabled.
3. **Graduation** gates additionally require independent reviews, external paid
   loops, proof-job accounting, positive realized solver economics, unassisted
   usability, and a separately announced graduation review before V2 can become
   the default.

No external paid loop or graduation criterion is required to deploy or open the
Beta. No canary or post-launch observation can be claimed before its canonical
evidence exists.

## Local Commands

```powershell
$env:Path = "$PWD\.tools\foundry;$env:Path"
forge build --root contracts/base-escrow --force --ast

python scripts/build_open_competition_v2_beta1_release.py `
  --network base-mainnet `
  --source-commit (git rev-parse HEAD) `
  --output target/open-competition-v2-mainnet-plan.json

python scripts/run_open_competition_v2_mainnet_fork_replay.py `
  --bundle target/open-competition-v2-mainnet-plan.json `
  --output target/open-competition-v2-mainnet-fork-replay.json
```

The real-proof replay requires SP1 6.3.1 and is intentionally separate because
proof generation is expensive:

```powershell
python scripts/run_open_competition_v2_mainnet_fork_replay.py `
  --bundle target/open-competition-v2-mainnet-plan.json `
  --proof-rehearsal `
  --output target/open-competition-v2-mainnet-real-proof-replay.json
```

The local command is sequential and requires hardware appropriate to the
selected proof system. CI first prepares one hash-bound context, generates
`groth16_first` on CPU, and generates `plonk_best_a` and `plonk_best_b` as two
independent network jobs during a trusted release dispatch. A final job rejects
any proof whose mode, vkey, ELF hashes, or full 640-byte journal differs from
the prepared fixture before replaying transactions. Pull requests do not
receive the network credential and therefore run only deterministic gates and
the real CPU Groth16 proof; a trusted dispatch against the exact candidate
commit is required before merge or Sepolia rehearsal.

Configure the proving credential without exposing it to source or pull-request
workflows:

```bash
gh secret set SP1_NETWORK_PRIVATE_KEY --repo NSPG13/agent-bounties
gh workflow run open-competition-v2-beta1-release.yml --ref <candidate-branch> \
  -f run_real_proof_fork_rehearsal=true \
  -f run_live_sepolia_rehearsal=false
```

`scripts/check_open_competition_v2_prover_backend.py` returns exact readiness
codes. `V2_PROVER_NETWORK_KEY_MISSING` means the secret is absent;
`V2_PROVER_RUNNER_LACKS_NETWORK` means the release runner was not built with
network support; and `V2_PROVER_MEMORY_INSUFFICIENT` prevents a CPU backend
from silently attempting a proof system beyond its documented capacity.

The live Base Sepolia rehearsal is a protected default-branch workflow. Its
protected preparation step uses `BASE_KEEPER_PRIVATE_KEY`, derives test-only
solver accounts without publishing their keys, deploys or reuses the exact
factory, and publishes only bound proof inputs. Unprivileged parallel jobs
generate the three proofs. A protected execution step validates those
artifacts, moves testnet funds, reclaims remaining test USDC, and reconciles
the receipts. It does not enable mainnet creation:

```bash
gh workflow run open-competition-v2-beta1-release.yml --ref main \
  -f run_real_proof_fork_rehearsal=false \
  -f run_live_sepolia_rehearsal=true
```

The workflow waits until every rehearsal receipt is included at a canonical
Base Sepolia `safe` block. A successful receipt alone is not rehearsal
evidence.

## Published Bundle

Each bundle records:

- source commit and normalized source hashes;
- Solidity compiler image, settings, and exact deployment calldata;
- factory, adapter, and implementation addresses and runtime hashes;
- SP1 version, commit, program vkey, ELF and schema hashes;
- canonical USDC, gateways, verifier routes, and safe-block runtime hashes;
- Beta risk preimage and hash;
- canary economics and the synthetic-metric exclusion;
- every incomplete release and graduation gate.

The factory deployer has no contract authority. It need not fund a canary.
Any external wallet may fund an isolated competition after acknowledging the
same risk hash. The factory never holds USDC or token allowances.

## Mainnet Boundary

The repository does not contain an unreviewed mainnet broadcast job. That is
deliberate. After every prelaunch gate is evidenced, add a separately announced,
protected-environment deployment change that requires the exact release commit,
bundle hash, deployment-review evidence, and explicit risk acknowledgement.
Run and reconcile both mainnet canaries before enabling public Beta creation.
Graduation remains a later, separately reviewed state. Only a safe-block
`CompetitionSettledV2` event proves solver payment.
