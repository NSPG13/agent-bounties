# Open Competition V2 Beta1 Release

V2 Beta1 is implemented but blocked from public mainnet creation. The release
is R4: immutable contracts hold Base USDC, call SP1 gateways, and can settle
without operator approval. A deployment transaction is not release approval.

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
true Boolean. `mainnet_creation_enabled` stays false until every gate is true
and the separate review approves activation.

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

The live Base Sepolia rehearsal is a protected default-branch workflow. It
uses `BASE_KEEPER_PRIVATE_KEY`, derives test-only solver accounts without
publishing their keys, reuses an exact factory after interrupted runs, and
reclaims remaining test USDC. It does not enable mainnet creation:

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
- Solidity settings and exact deployment calldata;
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
deliberate. After every gate is evidenced, add a separately announced,
protected-environment deployment change that requires the exact release
commit, bundle hash, independent reviewer, and explicit release
acknowledgement. Only a safe-block `CompetitionSettledV2` event proves solver
payment.
