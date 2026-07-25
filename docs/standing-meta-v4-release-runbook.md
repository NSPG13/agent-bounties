# Standing Meta V4 release and incident runbook

This runbook covers the R4 path from reviewed source to one Base Sepolia
rehearsal, one Base mainnet canary, and bounded expansion. It never treats a
plan, source commit, workflow, signature, or transaction hash as deployment,
funding, settlement, or payment evidence.

## Immutable latency policy

The manifest marks this exact policy `review_frozen`. That means the values are
the release candidate submitted for independent review; it does not mean the
contracts are deployed or the review is complete. Any value or decision-string
change invalidates the deployment plan and requires a new review pass.

Successful paths proceed as soon as their prerequisite transaction is
confirmed:

| Control | Bound | Why it remains |
| --- | ---: | --- |
| Per-bounty solver enrollment | 0 | Active global tickets are snapshotted atomically |
| Selected solver response | 2 minutes | Fast agent wake-up with bounded promotion |
| VRF confirmations | 3 | Randomness security floor |
| VRF failure deadline | 2 hours | Outage bound; successful fulfillment never waits |
| Primary or backup response | 30 minutes each | Four ranked chances without a reroll |
| Appeal filing | 4 hours | Symmetric solver/creator challenge opportunity |
| Appeal voting | 2 hours | Three matching votes may finalize earlier |
| Verification envelope | 24 hours | Covers the 12h10m worst-case case budget with margin |
| Stake activation/unbonding | 7 days | One-time flash-ticket and exit control |

The only eligible appellant can waive immediately. Three matching appellate
votes are immediately decisive. Work windows are maxima; an early submission
does not wait for them to expire.

## Release authority

- The keeper may deploy reviewed components and fund the exact existing VRF
  subscription. It cannot withdraw the bounded wallet's USDC.
- The bounded-wallet owner signs the exact capped `withdrawToken` call from the
  manifest without exporting a private key.
- Chainlink selects wallets from frozen sets. It does not judge work or
  authorize payment.
- The primary and appellate wallets judge the committed submission policy.
- `AgentBounty` alone settles or rejects after the final verifier state.
- Only confirmed canonical `BountySettled` proves solver payment.

## Repository environment gate

Both `standing-meta-v4-sepolia` and `standing-meta-v4-mainnet` must have:

- exactly one deployment branch policy: `main`;
- administrator bypass disabled;
- at least one required user reviewer other than the maintainer author; and
- self-review prevention enabled.

Generate the read-back evidence with an admin-scoped token kept outside source:

```powershell
$env:GH_TOKEN = gh auth token
python scripts/standing_meta_v4_release_audit.py github-environments `
  --output target/standing-meta-v4-environments.json `
  --require-complete
Remove-Item Env:GH_TOKEN
```

The audit fails while an independent reviewer is absent. Do not set
`repository_environment_protection_complete=true` until this read-back passes
and the evidence receives review.

## Base Sepolia rehearsal

1. Run the deterministic plans and record the exact revision:

   ```powershell
   python scripts/standing_meta_v4_deploy.py plan --network base-sepolia `
     --require-clean `
     --output target/standing-meta-v4-base-sepolia-plan.json
   ```

   The plan must also read the coordinator's live `s_config` and pinned
   `s_provingKeys` entry. It fails before deployment when the coordinator's
   minimum confirmations exceed three, its callback-gas ceiling is below
   150,000, its reentrancy lock is active, or the pinned key hash is not
   registered. It also records the exact clean Git commit and tree, solc and
   Forge versions, optimizer/EVM/metadata settings, canonical-JSON
   readiness-manifest SHA-256,
   observation block, contract creation/runtime hashes, and a content hash over
   the plan. An address and runtime-code check alone is insufficient.

2. Fund the keeper with faucet Base Sepolia ETH from an option in the
   [official Base faucet directory](https://docs.base.org/base-chain/network-information/network-faucets).
   The Coinbase Developer Platform option supports agent-native SDK claims when
   an operator has configured its API credentials. Record the faucet source,
   transaction, confirmed balance, and observation block. Faucet ETH is test
   value and never mainnet sponsorship evidence.
3. Acquire canonical Base Sepolia test USDC for at least eight verifier stakes,
   three solver stakes, canary escrows, claim bonds, and appeal bonds. The role
   stake floor alone is 55 test USDC. Record the source and balances.
4. Dispatch `deploy-consumers` from `main` through the protected Sepolia
   environment. The deployment creates one subscription and authorizes the two
   distinct sortition consumers, but does not fund it.
5. Read the deployment artifact and independently run `verify` through a second
   RPC endpoint. Confirm code hashes, constructor arguments, immutable getters,
   controller one-time configuration, subscription owner, and the exact two
   consumers. The evidence must also derive and hash the parent factory's
   internally created `OnchainTermsRegistryV4` and
   `CanonicalIndependentChildVerifierV4`; recording only the externally
   deployed factory graph is incomplete.
6. Dispatch `fund-subscription` with `source_usdc_base_units=0` and the exact
   faucet ETH allocation. Confirm the native subscription balance delta through
   RPC.
7. Register eight verifier and three solver role tickets. Wait the immutable
   seven-day activation once, activate them, and confirm availability.
8. Execute and record: immediate waiver, upheld appeal, overturned appeal,
   primary timeout/promotion, appeal timeout, cancellation, contributor pull
   refund, child settlement, parent settlement, and first-valid open competition
   settlement. Testnet events are rehearsal evidence only.
9. Assemble `target/standing-meta-v4-base-sepolia-rehearsal-draft.json` using
   schema `agent-bounties/standing-meta-v4-sepolia-rehearsal-v1`. It must contain
   the exact twelve named scenarios enforced by
   `scripts/standing_meta_v4_rehearsal_audit.py`, their canonical subject type,
   exact clean source commit, subject and actor addresses, bounty/case IDs,
   confirmed transaction and log
   locations, the open-competition factory, faucet receipts, positive reserve
   floors, and primary/secondary RPC observation blocks. Then seal and audit it
   through two distinct RPC providers:

   ```powershell
   python scripts/standing_meta_v4_rehearsal_audit.py `
     --evidence target/standing-meta-v4-base-sepolia-rehearsal-draft.json `
     --seal-output target/standing-meta-v4-base-sepolia-rehearsal.json `
     --deployment target/standing-meta-v4-base-sepolia-deployment.json `
     --rpc-url $env:BASE_SEPOLIA_RPC_URL `
     --secondary-rpc-url $env:BASE_SEPOLIA_SECONDARY_RPC_URL `
     --output target/standing-meta-v4-base-sepolia-rehearsal-audit.json `
     --require-complete
   ```

   The validator compiles and immutable-normalizes all ten canonical V4
   components, the Sepolia base child factory, and the open-competition factory
   against the exact clean source commit. It verifies canonical subject
   provenance, decodes verdict/appeal/order/payout
   facts from receipt logs, matches USDC transfers, proves the parent-to-child
   relationship and 1 USDC successful-settlement margin, checks both VRF
   consumers and live pool/reserve floors, and requires both RPC passes to
   agree. It never records RPC URLs or credentials.

The rehearsal is complete only when a content-addressed evidence bundle maps
every path to confirmed receipts, code/runtime hashes, configuration reads,
balances, canonical events, and matching token transfers. A structurally valid
or SHA-256-sealed JSON file without both successful live RPC passes remains
incomplete.

The deployment manifest may use intermediate descriptive states while evidence
is being assembled. Set its exact status to `ready_to_earn` only after both
network evidence sections contain the exact required component set, every
address and reserve has been read back, and all R4 gates are complete. The
release audit rejects any other status.

## Independent review and mainnet authorization

The independent reviewer examines USDC conservation, stake locking/slashing,
VRF frozen sets and no-reroll behavior, the two-minute wake-up risk, primary and
appeal timing, immutable wiring, consumer authorization, bytecode-size margins,
the owner withdrawal cap, and incident containment. Resolve findings or record
an explicit risk acceptance before setting `independent_review_complete=true`.
The same manifest change must populate `independent_review_evidence` with the
exact 40-character source commit and tree, a non-maintainer reviewer identity,
an HTTPS report URL, the report's SHA-256, and
`findings_resolved_or_accepted=true`. A bare completion boolean fails the
release audit. Mainnet deployment accepts the reviewed commit or a later squash
commit only when its complete Git tree is identical; any different tree fails
closed.

Re-run the exact Base-mainnet fork at the reviewed commit. Record compiler
version/settings, source hashes, creation/runtime bytecode hashes, constructor
arguments, and expected immutable getter values. A later source change
invalidates this evidence.

## Mainnet funding and deployment

1. Confirm every R4 evidence flag and environment read-back. The release
   acknowledgement does not replace those facts.
2. Deploy the reviewed component graph from protected `main`, then verify it
   through an independent RPC pass. The second pass must repeat the live
   coordinator-configuration and proving-key checks; a previously valid plan
   cannot authorize deployment after coordinator drift. The deployment command
   refuses a dirty checkout and stores the same commit/compiler/manifest tuple
   in its checkpoint. A resume under any different tuple fails closed before
   another transaction is sent.
3. Generate an unsigned, live-RPC-validated withdrawal request without loading
   the owner key:

   ```powershell
   python scripts/standing_meta_v4_deploy.py prepare-owner-withdrawal `
     --network base-mainnet `
     --source-usdc-base-units 7000000 `
     --output target/standing-meta-v4-owner-withdrawal-request.json
   ```

   The request pins the live owner, token, recipient, balance, observation
   block, code hashes, amount, and exact calldata. It deliberately omits nonce,
   fees, signature, and private key, and reports `ready_to_submit=false`.
4. The bounded-wallet owner reviews and signs only the manifest's exact
   `withdrawToken(nativeUSDC, keeper, amount)` calldata. The amount must be
   positive and no more than 7,000,000 micro-USDC. Confirm `TokenWithdrawn`, the
   native-USDC `Transfer`, the wallet debit, and the keeper credit.
5. Convert no more than that received amount through a separately reviewed
   Base swap route with explicit minimum ETH output, deadline, recipient,
   allowance, and slippage cap. Confirm the USDC debit and ETH credit. Revoke a
   residual allowance when the route requires approval.
6. Create the mainnet subscription, authorize exactly the verifier and solver
   sortition consumers, and fund the exact native ETH amount. Confirm owner,
   native balance, request count, and consumer set through RPC.
7. Record positive minimum subscription and keeper gas reserves in the manifest.
   A boolean without an explicit positive reserve threshold is not readiness.

No private key, recovery phrase, raw signature, private RPC credential, or
swap-session secret belongs in an artifact, issue, workflow log, or prompt.

## Canary and expansion

Deploy one low-value standing-meta V4 parent and one low-value Open Competition
V1 bounty. Keep both out of ready-to-earn until runtime, terms, funding, timing,
pool, sponsorship, VRF, appeal, and monitoring checks pass.

For the V4 canary, prove child assignment, child claim, primary verdict, optional
appeal, child `BountySettled`, deterministic parent predicate, and parent
`BountySettled`. Verify the exact 1 USDC successful-settlement onchain margin;
do not call it net profit. For open competition, prove the one-block
commit/reveal separation, first passing reveal sequence, losing-bond withdrawal,
and canonical settlement.

Expand only after the canary settles. Legacy V2 cancellation/refund remains a
separate owner action; confirm `BountyCancelled` and each `RefundWithdrawn`
before repointing public issues to funded, claimable replacements.

## Monitoring and stop conditions

Create a non-secret activity file using schema
`agent-bounties/standing-meta-v4-monitor-activity-v1`. It pins the first block
to scan, the canonical Open Competition factory, and the exact standing-meta
and Open Competition canary addresses. Generate a snapshot against two
independent RPC providers:

```powershell
python scripts/standing_meta_v4_monitor.py `
  --network base-mainnet `
  --deployment target/standing-meta-v4-base-mainnet-deployment.json `
  --activity target/standing-meta-v4-mainnet-monitor-activity.json `
  --rpc-url $env:BASE_MAINNET_RPC_URL `
  --secondary-rpc-url $env:BASE_MAINNET_SECONDARY_RPC_URL `
  --output target/standing-meta-v4-mainnet-monitor.json `
  --require-healthy
```

The monitor independently revalidates immutable wiring, subscription authority
and reserve, keeper gas, available stake pools, every observed VRF request,
open verification cases, canonical canary provenance and settlement events,
the actual standing-meta margin, and competition activity. It emits a SHA-256
commitment and `earning_suppressed=true` on any failure. It never performs a
top-up, reroll, deployment, judgment, settlement, cancellation, refund, swap,
withdrawal, or other governance/value mutation.

Agent-native readiness may set `*_MONITORING_ACTIVE=true` only while the most
recent successful snapshot's `observed_at_unix` is also supplied as
`*_MONITORING_OBSERVED_AT_UNIX`. The API fails closed when that timestamp is
missing, in the future, or more than 300 seconds old. A boolean flag alone is
not monitoring evidence.

Suppress new earning immediately when any of these is false or stale:

- exact runtime hashes and immutable wiring;
- VRF subscription owner, exact two consumers, and native balance at or above
  the configured minimum;
- keeper gas reserve at or above the configured minimum;
- eight available verifier tickets and three eligible solver tickets after
  exclusions;
- callback latency below the two-hour failure deadline;
- an executable primary, appeal, timeout, and stake-unlock path;
- valid terms, funded canary economics, safe work/verification timing, and
  canonical indexer freshness.

Track assignment response time, rank promotions, primary responses, appeals,
overturns, timeouts, slashes, settlement margin, failed reveals, competition
capacity, and canonical settlement events. Never auto-top-up, withdraw, swap,
deploy, judge, settle, cancel, or refund in response to an alert.

## Incident and forward repair

1. Suppress affected opportunities from earning and verification jobs while
   retaining their canonical history.
2. Capture the safe block, runtime hashes, subscription state, pool counts,
   request/case IDs, deadlines, and redacted logs.
3. Classify false funding/payment, unexpected value movement, signer exposure,
   runtime mismatch, or canonical-event corruption as SEV0.
4. Do not reroll failed VRF requests or substitute platform randomness. Let the
   affected round fail closed and refund/unlock according to the contracts.
5. Immutable defects use cancellation where authorized, contributor pull
   refunds, and migration to a separately reviewed version. Application code
   may roll back; chain history cannot.
6. Add a deterministic regression fixture and update the threat model/runbook
   before resuming earning.

An incident is closed only after the triggering invariant is restored, all
canonical balances/events reconcile, monitoring is green for the canary, and
the same review class authorizes removal of containment.
