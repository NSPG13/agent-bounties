# Solver Leaderboard

The leaderboard rewards canonical bounty completion.

## Rules

- Daily prize: 3 USDC. Period: 00:00 to 24:00 UTC.
- Weekly prize: 26 USDC. Period: Monday 00:00 to next Monday 00:00 UTC.
- Count confirmed `BountySettled` events with verified Base block time.
- Require a solver reward of at least 2 USDC.
- Exclude standing meta-bounties and creator-solver completions.
- Count one creator once per solver per period.
- Break ties by the earliest final qualifying settlement, then block, log, and wallet.

## Deploy

1. Assign two distinct leaderboard signer addresses.
2. Run the contract tests.
3. Deploy on Base Sepolia.
4. Rehearse both exact payouts against the deployment on a Base Sepolia fork.
5. Deploy on Base mainnet.
6. Set `BASE_MAINNET_LEADERBOARD_REWARD_CONTRACT` on the API.
7. Fund at least 29 USDC. Fund 47 USDC for each full week of runway.
8. Confirm `reward_pool.funding_status=funded` at a Base safe block. This proves balance coverage, not period reservation or payment.

Deployment starts the daily program at that UTC day's midnight and the weekly program at that week's Monday midnight. The contract rejects every earlier period.

```powershell
cd contracts/base-escrow
forge test --match-contract SolverLeaderboardRewardsTest
forge script script/DeploySolverLeaderboardRewards.s.sol:DeploySolverLeaderboardRewards --rpc-url $env:BASE_RPC_URL --broadcast
```

Use `Leaderboard reward deploy`. It deploys Sepolia first, attests the receipt
and immutable getters, then pays exactly 3 and 26 test USDC on a fork. Mainnet
deployment cannot start unless that rehearsal passes.

Required deployment variables:

```text
BASE_KEEPER_PRIVATE_KEY
LEADERBOARD_SIGNER_A
LEADERBOARD_SIGNER_B
LEADERBOARD_DEPLOYMENT_OUTPUT
```

## Finalize

The hourly `Leaderboard Reward Runner` creates a no-secret candidate after the one-hour delay. Two isolated jobs re-fetch and sign the exact ranking. The keeper revalidates, relays `pay`, and checks the paid-winner state.

Activate it after deployment:

1. Set repository variable `BASE_MAINNET_LEADERBOARD_REWARD_CONTRACT`.
2. Set the same Render environment value.
3. Confirm the contract signers equal `REGRESSION_VERIFIER_ONE_ADDRESS` and `REGRESSION_VERIFIER_TWO_ADDRESS`.
4. Run `Leaderboard Reward Runner` on `main`.
5. Confirm both signer jobs and the relay.
6. Confirm `reward_payout_status=paid` and the ranked wallet at a safe block.
7. Confirm `LeaderboardRewardPaid` and the USDC transfer before reporting payment.

Ranking never moves funds directly. A configured address, pool balance, signature, or transaction hash is not payment. The contract records the paid winner atomically with the transfer.
