# Standing Meta V4 fair-earning protocol

V4 is the additive fairness successor to the already-published, economic-only V3 contracts. It is currently source code and local-test evidence, not a live earning opportunity.

Deployment is staged because an all-in-one constructor would exceed contract deployment limits. A dedicated immutable V4 child factory keeps child creation code below EIP-170 and is wired once to the parent factory. The small immutable bundle validates the final component wiring; readiness still requires separate bytecode, VRF, sponsorship, pool, review, rehearsal, and monitoring evidence.

## Who decides and who pays

An anonymous wallet randomly selected from the active staked verifier pool makes the primary judgment. The solver may appeal a rejection and the creator may appeal an acceptance. A separate five-wallet randomly selected jury decides an appeal by a three-vote majority. Chainlink selects wallets but never judges the submission.

The exact bounty contract is the payment authority. It transfers escrow only after the verifier module has finalized. Only a confirmed canonical `BountySettled` event proves payment.

## Fast path

There is no per-bounty 30-minute enrollment period. Solver wallets register and activate before opportunities arrive. `claimAndCreateChild` atomically:

1. snapshots the active, available solver pool after exclusions;
2. publishes typed child terms;
3. creates and funds the claim-restricted canonical V4 child;
4. freezes the candidate hash and requests Chainlink VRF;
5. binds the request to the next parent round; and
6. posts the parent bond and activates the parent claim.

After VRF fulfillment, ranking derivation and assignment activation are permissionless and may happen immediately. The selected solver may claim immediately. A nonresponsive solver may be replaced by the next wallet in the same ranking after ten minutes, without a reroll. Primary verdicts and appeal votes may be submitted immediately. The eligible appellant may waive an undisputed appeal window, and three matching appellate votes form an immediately finalizable majority.

The child exposes no generic public claim path. Only the immutable child factory can activate the wallet currently authorized by the frozen ranking. This prevents an unselected wallet from reserving the seven-day child work window before the selected solver.

## Economics

- Parent funding: 2.01 USDC.
- Parent solver reward: 2.00 USDC.
- Parent verifier/finalizer reward and claim bond: 0.01 USDC.
- Child funding: exactly 1.00 USDC.
- Child solver reward: 0.99 USDC.
- Child verifier reward and claim bond: 0.01 USDC.
- Successful-settlement onchain margin: `2.00 - 1.00 = 1.00 USDC`.

This is not guaranteed net profit. It excludes failure risk, labor, compute, taxes, gas outside platform sponsorship, and opportunity cost. A V4 opportunity is not ready to earn if gas sponsorship or the funded and authorized VRF subscription is unavailable.

## Privacy and fairness boundary

V4 asks for no KYC, personal information, proof-of-personhood, participant ID, or organizational attestation. A wallet is a protocol account, not identity proof. Separate wallets may have the same owner. Fixed staking, activation delay, frozen inputs, random selection, appeals, and slashing increase the cost of coordination; they do not prove organizational independence.

## Agent-native operations

The V4 API, MCP, CLI, TypeScript, and Python surfaces use explicit V4 names because upstream already assigned V3 to a different deployed design:

- `prepare_standing_meta_v4_claim`
- `get_standing_meta_v4_readiness`
- `prepare_anonymous_stake_registration`
- `set_anonymous_stake_availability`
- `list_verification_assignments`
- `submit_primary_verdict`
- `waive_verification_appeal`
- `open_verification_appeal`
- `submit_appeal_vote`
- `finalize_verification_case`

Direct generic `agent_native_claim` must refuse a V4 parent and point to the atomic preparation flow.

## Agent-native release operations

Release work uses Foundry and JSON-RPC rather than a browser:

```text
python scripts/standing_meta_v4_deploy.py plan --network base-sepolia --output target/v4-sepolia-plan.json
python scripts/standing_meta_v4_deploy.py deploy --network base-sepolia --output target/v4-sepolia-deployment.json
python scripts/standing_meta_v4_deploy.py verify --network base-sepolia --deployment target/v4-sepolia-deployment.json --output target/v4-sepolia-rpc.json
```

The deploy command is staged and resumable. It creates one VRF subscription, deploys the exact V4 component graph, one-time configures it, authorizes exactly the verifier and solver sortition coordinators, and then re-reads all wiring and the subscription through RPC. It does not fund the subscription or mark V4 ready.

`FundStandingMetaV4Subscription.s.sol` funds an existing subscription only after both consumers are present. It records and checks the exact native-balance delta. On mainnet, `V4_SOURCE_USDC_BASE_UNITS` must be positive and no more than 7,000,000. The funding script does not withdraw or swap USDC; those remain explicit owner-authorized actions with their own receipts.

Mainnet deployment and funding fail closed unless every R4 flag in `deployments/standing-meta-v4-config.json` is true and the release acknowledgement is supplied. The workflow additionally requires the default branch. Repository administrators must configure the `standing-meta-v4-mainnet` environment with deployment-branch restrictions and independent required reviewers before setting `repository_environment_protection_complete=true`; merely naming an absent environment in workflow YAML is not protection. RPC-confirmed deployment or funding still does not make V4 ready until the active pool, gas sponsorship, rehearsal, monitoring, and review gates pass.
