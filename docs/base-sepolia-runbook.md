# Base Sepolia Autonomous Runbook

Base Sepolia is the required live rehearsal rail for
`agent-bounties/autonomous-v1`. It uses native test USDC and the same factory,
bounty, deterministic-verifier, and atomic-sponsor bytecode as production.
Testnet receipts are rehearsal evidence only and never prove a real payout.

## Pinned Activation

The unsigned bundle in
[`deployments/base-sepolia-sponsor-activation.json`](../deployments/base-sepolia-sponsor-activation.json)
pins:

- chain ID `84532`;
- native test USDC `0x036CbD53842c5426634e7929541eC2318f3dCF7e`;
- deployer `0x884834E884d6e93462655A2820140aD03E6747bC`;
- factory `0x9601a40b35Ad6843846732C6CB73c4C82f9Ba850`;
- implementation `0xe70b9d541a176307e50f308aa370a1661eabfd99`;
- 16-bit verifier `0x7231f1312448Fa60078Fb56cDB6e2c392Bd1269b`;
- atomic sponsor `0xa1E2E93530114F7FE64c251556b8De13Dad7d157`;
- one dedicated policy-signer address and capped sponsor economics;
- exactly `0.10` test USDC of initial acquisition budget.

The bundle contains no private key and proves no live deployment. Regenerate it
when the deployer nonce, constructor inputs, source bytecode, or pinned chain
state changes.

## Deterministic Gates

```powershell
$env:Path = "$PWD\.tools\foundry;$env:Path"
cd contracts\base-escrow
forge test --fuzz-runs 1000

$env:RUN_SEPOLIA_FORK = "true"
$env:BASE_SEPOLIA_RPC_URL = "https://your-managed-base-sepolia-rpc"
forge test `
  --match-contract AtomicClaimSponsorMainnetForkTest `
  --match-test testRealUsdcZeroBalanceSolverCompletesSponsoredLoopOnBaseSepolia -vv
```

The first gate covers replay, race rollback, quota, ownership, and conservation
invariants. The second runs the zero-balance solver path against native Base
Sepolia USDC on a fork. Neither broadcasts.

## Locked Wallet Flow

Serve the repository from localhost; do not open the HTML directly:

```powershell
python -m http.server 8879 --bind 127.0.0.1
```

Open
`http://127.0.0.1:8879/tools/base-sepolia-sponsor-activation.html` in the
browser profile containing the deployer wallet. The page discovers injected
wallets through EIP-6963 and does not discriminate by provider.

The console requires four explicit confirmations:

1. deploy the exact factory and implementation;
2. deploy the exact deterministic verifier;
3. deploy the exact capped atomic sponsor;
4. transfer exactly `0.10` test USDC to the verified sponsor.

Before each request it rechecks account, chain, nonce, address occupancy,
runtime bytecode, immutable getters, balances, and live gas estimation. It has
no editable address, amount, calldata, nonce, RPC, or contract field. A nonce
change before the three deployments requires bundle regeneration because it
changes the predicted addresses.

## Live Evidence

After all four transactions confirm:

1. record transaction hashes and deployment blocks;
2. independently read factory `implementation()` and `settlementToken()`;
3. compare all runtime bytecode with the bundle;
4. read verifier difficulty and every sponsor address/cap getter;
5. confirm the sponsor's native test-USDC balance;
6. run the indexer from the factory deployment block;
7. configure hosted Sepolia values only after those checks pass.

Required hosted settings are:

```text
BASE_SEPOLIA_BOUNTY_FACTORY=0x9601a40b35ad6843846732c6cb73c4c82f9ba850
BASE_SEPOLIA_BOUNTY_IMPLEMENTATION=0xe70b9d541a176307e50f308aa370a1661eabfd99
BOND_SPONSOR_BASE_SEPOLIA_CONTRACT=0xa1e2e93530114f7fe64c251556b8de13dad7d157
BOND_SPONSOR_GRANT_SIGNER_PRIVATE_KEY=<secret matching the committed public signer>
ENABLE_BOND_SPONSORSHIP=true
```

The API also requires Postgres, the hosted x402 gas relayer, and its existing
bounded gas/fee controls. Store the policy key only in the hosted secret store;
never place it in Git, a GitHub issue, browser storage, shell history, or a
wallet-import form.

## Full Live Loop

Activation is complete only after a fresh zero-USDC, zero-ETH solver:

1. receives one exact sponsorship offer;
2. signs the bounded USDC EIP-3009 claim authorization;
3. reaches confirmed canonical `BountyClaimed` through one atomic relay;
4. submits committed evidence;
5. passes the precommitted deterministic verifier;
6. reaches confirmed canonical `BountySettled` on Base Sepolia.

A grant, signature, relay row, transaction hash, submission, or verifier output
is not payment evidence. Only `BountySettled` proves protocol settlement, and a
Base Sepolia settlement still has no monetary value.

## Autonomous-v2 (Platform Fee) Rehearsal

`tools/rehearse_autonomous_v2_sepolia.py` deploys `AgentBountyFactoryV2` with
the launch fee terms: 750 bps to `0x884834E884d6e93462655A2820140aD03E6747bC`.
It then runs the full gasless loop with real test USDC:
1. relayed EIP-3009 creation;
2. relayed EIP-3009 claim bond;
3. `submitWithSignature`;
4. `settleWithAttestations` from a two-verifier quorum.

**Participants.** The poster, solver and verifiers are fresh in-memory wallets.
They only sign planner typed data and never hold ETH. The shared keeper deploys,
funds them with test USDC, and relays every transaction.

**Pass condition.** The run passes only when every step emits its canonical
events and the confirmed settlement receipt carries a canonical USDC `Transfer`
of exactly the platform fee from the bounty to the fee recipient. Evidence comes
from receipts, not `latest` balances, because public RPC endpoints are
load-balanced and a read can reach a node that is behind. Reads after writes are
retried, and the keeper nonce is tracked locally.
The evidence JSON lists the factory, its deployment block, each transaction,
and the hosted settings (`BASE_SEPOLIA_BOUNTY_V2_*`, and
`BASE_INDEXER_PROTOCOL=autonomous-v2` with `BASE_INDEXER_START_BLOCK`).

**How it runs** (`.github/workflows/autonomous-v2-sepolia-rehearsal.yml`):
- **Fork mode**, on pull requests: forks Base Sepolia, so the loop runs
  against Circle's real test-USDC contract with no secrets and no testnet
  spend.
- **Live mode**, a manual dispatch from `main` with `live: true`: uses
  `BASE_KEEPER_PRIVATE_KEY` under the shared keeper concurrency group. The
  keeper needs about 0.0003 Base Sepolia ETH and 0.15 test USDC. The tool
  refuses to start, and reports the shortfall, when it holds less.

**Latest live result.** Passed on 2026-10-07 on the hardened contracts from
#1613 ([run
37562059809](https://github.com/NSPG13/agent-bounties/actions/runs/37562059809)).

| Item | Value |
| --- | --- |
| Factory | `0x44de6074440fc006c20087991364ce65c87e6d88` |
| Factory deployment block | 47785903 |
| Bounty | `0xb71aadca2cf50750339552f5f73a8f1d06dde7a3` |
| Settlement block | 47785909 |
| `PlatformFeePaid` | 7,500 |

All four relayed steps emitted their canonical events.
`deployments/autonomous-v2-base-sepolia-rehearsal.json` holds:
- every transaction;
- these checks, repeated directly on-chain;
- the superseded factories, which must not be configured: the pre-hardening
  factory and the one left by the first, lag-failed run.

A Base Sepolia settlement still has no monetary value. The Base mainnet
factory is recorded in `deployments/autonomous-v2-base-mainnet.json`.
