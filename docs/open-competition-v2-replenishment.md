# Private Open Competition V2 replenishment runbook

This runbook maintains transaction-ready marketplace inventory while external
poster and funder demand grows. It is an internal mechanism; all public reporting
continues to show one unified marketplace.

## Components

- `.github/workflows/bounty-inventory-guard.yml` enforces the unified public
  floor and writes the mechanism-specific state only to runner temporary storage.
- `scripts/plan_open_competition_v2_replenishment.py` builds a deterministic,
  unsigned, fail-closed plan.
- `scripts/materialize_open_competition_v2_replenishment.py` converts a ready
  plan into objective best-score GMV meta-competition terms without private
  ranking fields.
- `ops/open-competition-v2-forward-gmv-candidate-pool-v2.json` contains twenty
  announced forward campaign specifications and evidence references. Each
  competition fixes its scoring window, exclusions, and 2-of-2 deterministic
  snapshot-attester quorum before funding. The file contains no operational
  scores or private replenishment ranking.
- `ops/open-competition-v2-forward-gmv-reward-cohort-v1.json` contains five
  matched-window 6-USDC solver-prize treatments. It is a reviewed experiment
  specification, not an active reserve policy or spending authorization.
- `BoundedOpenCompetitionV2Wallet` holds the USDC reserve under the operator
  owner's on-chain control. Its delegate can create only reviewed, exact-value
  competitions; it cannot transfer or withdraw USDC.
- `BoundedOpenCompetitionV2WalletFactory` deterministically deploys that wallet
  and atomically funds it through an exact allowance or EIP-3009 authorization.
  Its release-bound salt makes the reserve address knowable before campaign
  policies are frozen, so every policy can exclude its exact funding wallet.
- The isolated delegate owns the private ranking, durable execution ledger,
  predicted addresses, and canonical reconciliation. The contract, not the
  hosted delegate, enforces candidate commitments and spending caps.

## Configuration

Create the protected GitHub environment `open-competition-v2-replenishment`.
Configure:

- secret `V2_REPLENISHMENT_SIGNER_URL`: credential-free HTTPS base URL;
- secret `V2_REPLENISHMENT_SIGNER_TOKEN`: revocable, replenishment-only token;
- variable `V2_REPLENISHMENT_ENABLED`: absent or `false` keeps the scheduled
  private planner dormant; set `true` only after the signer state service exists;
- variable `V2_REPLENISHMENT_EXECUTE`: absent or `false` for dry run; `true` only
  after the R3 launch gates pass.

The signer state endpoint returns `private_ranking` and `execution_ledger`.
The execute endpoint accepts the content-addressed request schema and returns
only `accepted`, `resuming`, or `canonically_activated`. Acceptance is not proof
of activation. The signer must reconcile primary and shadow indexers at a safe
block before recording activation.

## Signer policy

The on-chain reserve independently requires:

1. Base mainnet, the reviewed factory/release and independently reproduced
   `forward-canonical-gmv-attribution-metric-v2` profile, plus fresh safe-block evidence
   with primary/shadow agreement.
2. The exact economics pinned by the current owner-approved policy epoch:
   3.00 USDC solver plus 0.04 USDC keeper in the baseline epoch, or 6.00 USDC
   solver plus 0.04 USDC keeper in the reviewed five-item reward treatment.
   A single policy version cannot mix the two amounts.
3. No more than 30.40 USDC per UTC day or 77.668098 USDC lifetime. The initial
   policy cannot spend more than the exact owner-authorized reserve.
4. A candidate in the private ranking whose ID and public spec hash match and
   whose future scoring window and attester quorum were fixed before creation.
   After the window closes, primary/shadow snapshot hashes must be identical
   before either deterministic attester signs.
5. A durable idempotency reservation containing policy epoch, safe block,
   candidate hashes, derived nonce, and predicted contract address.
6. Exact USDC allowance immediately before factory creation and zero allowance
   after consumption.
7. No settlement, verifier, arbitrary-transfer, owner-recovery, or
   policy-changing call from the delegate.
8. `best_score`, `higher_is_better`, positive USDC-base-unit threshold, and the
   exact canonical-GMV program and journal hashes. First-proven and general
   artifact work revert in the reserve contract.

GMV attribution is `settlement GMV * entrant canonical funding / total
canonical funding`, rounded down for each settlement. Exclude operator/reserve
wallet funding, every settlement created by an operator or reserve wallet,
listed reward contracts, creator-as-solver and
entrant-as-solver settlements, and all noncanonical states. The prize
competition's own payout is excluded from the snapshot used to score it.

The owner remains `0x884834E884d6e93462655A2820140aD03E6747bC` for the
initial rollout. The delegate is not the owner and receives no reserve USDC; it
needs only enough Base ETH to submit authorized creation calls.

## Prepared reward-size cohort

The initial ten 3-USDC solver-prize competitions produced zero accepted entries
at review time. The public competition workspace shows why this can be rational:
with a 3-USDC child bounty and 0.11 USDC hosted proof/relay cost, winning returns
-0.11 USDC and losing returns -3.11 USDC before gas or labor. The reviewed
comparison doubles the solver prize while holding the scoring profile,
instructions, child template, exclusions, and five UTC windows constant.

Build and inspect the treatment without changing on-chain state:

```powershell
python scripts/build_open_competition_v2_reward_cohort.py `
  --approved-at <actual-UTC-review-time> `
  --active-reward-contract <repeat-for-the-exact-ten-live-contracts>
python scripts/inspect_open_competition_v2_reward_policy.py `
  --output <private-safe-state.json>
python scripts/build_open_competition_v2_reward_policy.py `
  --safe-state <private-safe-state.json> `
  --output <private-policy-bundle.json>
```

The builder fails closed unless safe-block policy version 1 still has exactly
30.40 USDC lifetime spend, 47.268098 USDC uncommitted balance, the reviewed
factory/profile hashes, and the unchanged 30.40-USDC day and 77.668098-USDC
lifetime caps. Five 6.04-USDC creations consume 30.20 USDC and leave 17.068098
USDC, of which 15.20 USDC is reserved for a later five-item 3.04-USDC floor.

After tests and review, serve the exact owner transactions:

```powershell
python scripts/serve_open_competition_v2_reward_policy_confirmation.py `
  --bundle <private-policy-bundle.json> `
  --result-output <private-policy-result.json>
```

The page selects MetaMask explicitly when available, requires Base mainnet and
the exact owner, and requests two zero-value confirmations. First it revokes
policy version 1 and verifies the unchanged lifetime spend and reserve balance
at a safe block, closing the old-delegate race. It then simulates and submits
the exact `configurePolicy` call and checks the sender, destination, value,
calldata, receipt, safe-block policy version/hash, unchanged lifetime spend,
and unchanged reserve balance again. Only after that evidence may the delegate submit
the five preapproved creation calls. Each creation still requires separate
canonical activation evidence; policy confirmation is not competition funding
or GMV. The server persists the safe revocation result separately so a process
restart between the two owner confirmations resumes at configuration instead
of asking the owner to repeat or bypass the revocation boundary.

Do not promote the reward treatment from clicks alone. Require at least ten
qualified starts, compare confirmed entry and child-post conversion with the
matched controls, and require improvement in externally funded canonical GMV
without weakening payment integrity. Public feedback and observable
participation are evidence; an AI-generated opinion is not real user feedback.

## Recovery

Recovery does not depend on the delegate, hosted API, relayer, or wallet
provider. The owner can:

1. call `revokePolicy()` to stop every future delegate creation;
2. call `recoverUncommitted()` to return the wallet's remaining USDC directly
   to the current owner;
3. after an owned competition is cancelled, expired, or its pinned verifier is
   unavailable, pull the creator refund into the reserve and recover it; and
4. transfer recovery authority only through two-step ownership acceptance.

For a production reserve, use the localhost confirmation server instead of
copying calldata by hand. It binds both transactions to Base, the reviewed
deployment, the exact owner and reserve, simulates each call, persists submitted
hashes before reconciliation, and verifies the USDC transfer at a safe block:

Download the exact reviewed production artifacts first. Do not substitute the
older checked-in deployment manifest, which describes an earlier deployment:

```powershell
gh run download 32606926043 --repo NSPG13/agent-bounties `
  --name open-competition-v2-beta3-mainnet-deployment-5a351f3e373691be58a9575b4374812b494b6086 `
  --dir <reviewed-artifact-directory>

python scripts/serve_open_competition_v2_reserve_recovery.py `
  --deployment <reviewed-artifact-directory>/bounded-open-competition-v2-wallet-base-mainnet.json `
  --deployment-evidence <reviewed-artifact-directory>/bounded-open-competition-v2-wallet-deployment-evidence.json `
  --expected-balance <safe-block-USDC-base-units> `
  --expected-lifetime-spent <safe-block-USDC-base-units> `
  --result-output output/open-competition-v2-reserve-recovery.json
```

The two owner confirmations have zero ETH value. The first moves no USDC. The
second asks the reserve to return its full current, uncommitted USDC balance to
the owner. Neither transaction can cancel, settle, enter, or withdraw from an
active competition; healthy active escrow remains governed by its immutable
terms and deadlines.

Funds already escrowed in a healthy active competition are deliberately not
clawbackable before its committed proof window ends. This preserves participant
trust. They become recoverable only through the competition's canonical refund
paths; settled rewards belong to the solver and keeper.

## Staged rollout

1. Merge read-only unified metrics and private guard observability with execution
   disabled.
2. Provision the isolated wallet with testnet funds and run the 10/9/5/4/0,
   concurrency, cap, crash-boundary, and reconciliation suite on Base Sepolia.
3. Set `V2_REPLENISHMENT_ENABLED=true`, keep
   `V2_REPLENISHMENT_EXECUTE=false`, and compare dry-run plans with manual
   canonical inventory for at least one review cycle.
4. Reproduce the GMV program twice, freeze and publish each canonical epoch
   snapshot, and obtain explicit R4 maintainer risk approval naming signer,
   rollback, and incident owners.
5. Have the owner atomically create and fund the bounded reserve with exactly
   77.668098 USDC. Fund the delegate separately with minimal Base ETH only.
6. Enable one internal canary, confirm allowance returns to zero and both
   indexers observe canonical activation, then enable the first batch.
7. Continue until ten are canonically active or a hard policy condition blocks.

Never increase reserve spending just to improve a count or reported GMV. The
checked-in confirmation page remains blocked while the new profile or snapshots
are pending; the superseded structured-artifact batch must not be submitted.

### Exact owner confirmation

The protected production release builds and deploys the exact reserve factory,
waits for a Base safe block, and uploads both
`bounded-open-competition-v2-wallet-base-mainnet.json` and
`bounded-open-competition-v2-wallet-deployment-evidence.json` with the mainnet
deployment artifact. Download that exact manifest; do not rebuild it from a
different source revision. After the production release and candidate pool name
the same factory and release hash, build the activation bundle:

```powershell
python scripts/build_forward_open_competition_v2_gmv_candidate_pool.py `
  --factory <exact-production-factory> `
  --release-hash <exact-production-release-hash> `
  --reserve-deployment deployments/bounded-open-competition-v2-wallet-base-mainnet.json `
  --approved-at <actual-UTC-review-time>
python scripts/build_open_competition_v2_gmv_activation.py `
  --release <exact-production-runtime.json> `
  --reserve-deployment deployments/bounded-open-competition-v2-wallet-base-mainnet.json `
  --output <private-temporary-activation.json>
python scripts/serve_open_competition_v2_gmv_confirmation.py `
  --bundle <private-temporary-activation.json>
```

Open the printed loopback URL in the owner's wallet-enabled browser. The page
checks Base mainnet and the exact owner before requesting one EIP-3009 typed-data
signature for 77.668098 USDC. It states the predicted recoverable reserve
address, the 3.04-USDC creation limit, the 30.40-USDC UTC-day cap, and the
77.668098-USDC lifetime cap. The page verifies the signature locally and sends
it nowhere except the loopback process.

Build relay calldata only from the verified signature and the same bundle:

```powershell
python scripts/build_open_competition_v2_gmv_relay.py `
  --bundle <private-temporary-activation.json> `
  --signature <verified-eip3009-signature> `
  --output <private-temporary-relay.json>
```

Any gas-paying address may submit the exact relay call. The destination is the
bounded reserve factory, while the USDC destination remains the predicted
reserve owned by `0x884834E884d6e93462655A2820140aD03E6747bC`. A valid
signature, relay call, or transaction hash is not funding evidence; reconcile
the reserve creation event and the exact USDC balance at a canonical safe block.

## Incident response

On cap exhaustion, stale/conflicting evidence, a release change, candidate
exhaustion, signer rejection, unexpected allowance, or pending execution beyond
the finality window:

1. Set `V2_REPLENISHMENT_EXECUTE=false`; set
   `V2_REPLENISHMENT_ENABLED=false` as well when the planner or signer state
   service must be isolated completely.
2. Preserve the plan and signer ledger privately; do not upload them to the
   public workflow or issue.
3. Reconcile safe-block factory and activation events through both indexers.
4. Revoke the on-chain policy, revoke the signer token, and recover uncommitted
   USDC to the owner if unexpected calls or amounts are present.
5. Repair forward with a reviewed policy epoch; canonical history is never
   rewritten.
6. Publish only a generic unified-liquidity incident statement unless a public
   protocol defect requires exact technical disclosure.

## Validation

```powershell
python -m unittest scripts.test_bounty_inventory_guard -v
python -m unittest scripts.test_plan_open_competition_v2_replenishment -v
python -m unittest scripts.test_materialize_open_competition_v2_replenishment -v
python -m unittest scripts.test_open_competition_v2_replenishment_workflow -v
python -m unittest scripts.test_build_bounded_open_competition_v2_wallet_bundle -v
python scripts/build_bounded_open_competition_v2_wallet_bundle.py
cd contracts/base-escrow
forge test --match-contract BoundedOpenCompetitionV2WalletTest -vv
```

The implementation is not operationally complete until the signer exists, the
reserve is funded, the Sepolia and canary gates pass, and ten competitions are
confirmed active. A ready plan or transaction hash does not satisfy that gate.
