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
  plan into objective schema-based terms without private ranking fields.
- `ops/open-competition-v2-gmv-candidate-pool-v1.json` contains twenty public,
  reviewed candidate specifications and evidence references. It intentionally
  contains no operational scores or active/standby ranking.
- `BoundedOpenCompetitionV2Wallet` holds the USDC reserve under the operator
  owner's on-chain control. Its delegate can create only reviewed, exact-value
  competitions; it cannot transfer or withdraw USDC.
- `BoundedOpenCompetitionV2WalletFactory` deterministically deploys that wallet
  and atomically funds it through an exact allowance or EIP-3009 authorization.
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

1. Base mainnet, the reviewed factory/release/profile, and fresh safe-block
   evidence with primary/shadow agreement.
2. Exactly 3.00 USDC solver reward and 0.04 USDC keeper reward per creation.
3. No more than 30.40 USDC per UTC day or 77.668098 USDC lifetime. The initial
   policy cannot spend more than the exact owner-authorized reserve.
4. A candidate in the private ranking whose ID and public spec hash match.
5. A durable idempotency reservation containing policy epoch, safe block,
   candidate hashes, derived nonce, and predicted contract address.
6. Exact USDC allowance immediately before factory creation and zero allowance
   after consumption.
7. No settlement, verifier, arbitrary-transfer, owner-recovery, or
   policy-changing call from the delegate.

The owner remains `0x884834E884d6e93462655A2820140aD03E6747bC` for the
initial rollout. The delegate is not the owner and receives no reserve USDC; it
needs only enough Base ETH to submit authorized creation calls.

## Recovery

Recovery does not depend on the delegate, hosted API, relayer, or wallet
provider. The owner can:

1. call `revokePolicy()` to stop every future delegate creation;
2. call `recoverUncommitted()` to return the wallet's remaining USDC directly
   to the current owner;
3. after an owned competition is cancelled, expired, or its pinned verifier is
   unavailable, pull the creator refund into the reserve and recover it; and
4. transfer recovery authority only through two-step ownership acceptance.

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
4. Obtain explicit R3 maintainer risk approval and name signer, rollback, and
   incident owners.
5. Have the owner atomically create and fund the bounded reserve with exactly
   77.668098 USDC. Fund the delegate separately with minimal Base ETH only.
6. Enable one internal canary, confirm allowance returns to zero and both
   indexers observe canonical activation, then enable the first batch.
7. Continue until ten are canonically active or a hard policy condition blocks.

Never increase reserve spending just to improve a count or reported GMV.

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
