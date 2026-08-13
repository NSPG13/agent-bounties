# Bounded Agent Wallet

`BoundedAgentWallet` lets a human authorize an agent once, then lets that agent
create, fund, claim, and submit canonical bounties without asking the human to
approve every transaction. The owner keeps revocation, policy replacement,
withdrawal, and ownership control.

`BoundedAgentWalletV2` also lets the owner cancel an unclaimed bounty created by
that wallet and recover only the wallet's contribution in one transaction. If a
third party already cancelled the expired bounty, the owner can pull the
wallet's refund separately. V1 bytecode remains immutable and does not gain
these V2 methods.

This does not give an agent the owner's MetaMask key. USDC moves into a separate
contract wallet, and the agent receives only a dedicated delegate signing key.
The contract enforces the limits even if the agent, its prompt, the hosted API,
or a relayer is compromised.

## Active 89 USDC Policy

Base mainnet wallet
`0x1eaa1c68772cf76bc5f4e4174766076e33ace662` currently uses policy
version 6:

| Boundary | Value |
| --- | ---: |
| Network and asset | Base mainnet native USDC |
| Lifetime gross spend | 89 USDC |
| Maximum one action | 5 USDC |
| Maximum each fixed 24-hour period | 10 USDC |
| Maximum bounty target | 5 USDC |
| Delegate | `0xe46741de0f379bff0ab8b01bce1b79a12d892fdb` |
| Expiry | No automatic expiry; owner can revoke or replace |
| Actions | create, fund, claim, submit |
| Deterministic verification | durable router `0x380c1af742593dd88b6f20387e9ee693a0536731` |
| Signed verification | exact `sandboxed_regression_v1` threshold-two set |

The exact policy hash is
`0xe865752db0df29aa0fc682fa837b7a68b91d0c88272cd6e0ae6718c831ada959`.
Owner transaction
`0x09532bbf5382cadac12c14c010cf332a7082d3e7ff018362e13991c5dfbb5704`
configured it at block `49902575`. This delegate-only rotation preserved every
financial cap, allowed action, and verifier constraint. Automation must pin all
policy fields, version, and hash; accepting any live policy is unsafe.

The five funded standing-meta-v2 parents are recovery-reserved and cannot be
selected for new earning actions. Returned claim bonds and bounty earnings
increase the wallet balance but do not restore gross lifetime authority. The
owner must explicitly replace the policy to change that authority.

## Security Status

This first release has deterministic bytecode pins, Slither review, 1,000-run
fuzz tests, adversarial tests, and Base mainnet and Sepolia fork rehearsals. It
has not received an independent external audit. The policy limits the rate,
duration, destinations, and gross amount of delegated spending; it cannot prove
that an agent chose a useful bounty. Treat the entire funded balance as exposed
to poor in-policy decisions over the policy lifetime, monitor it, and use the
owner revocation path when behavior is unexpected.

## Enforced Authority

The delegate can interact only with the immutable canonical
`AgentBountyFactory` and contracts registered by that factory. It cannot:

- transfer USDC or ETH to an arbitrary address;
- call an arbitrary contract or function;
- change the token, factory, policy, owner, caps, or expiry;
- withdraw funds;
- use an unapproved verifier module or verifier set;
- exceed the per-action, period, lifetime, or bounty-target cap.

Each direct or relayed action advances one shared nonce. Relayed actions use an
EIP-712 signature containing the wallet, action, payload hash, nonce, deadline,
and policy version. Policy replacement invalidates every queued signature.

The default agent path is gas-sponsored. A capped keeper submits the exact
signed action and pays Base gas from its own ETH reserve. The bounded wallet can
hold only USDC, needs no ETH, and never reimburses the keeper. Direct delegate
broadcast remains an optional fallback, not an onboarding requirement.

The caps bound financial loss; they do not prove that a task is useful. An
agent still needs a task-selection policy, and a compromised delegate can make
poor choices until a cap, expiry, or owner revocation stops it.

## One-Time Setup

1. Create a dedicated durable delegate signer. The currently available operator
   path is the [DPAPI-protected local delegate](local-delegate-wallet.md).
   Circle, CDP, Turnkey, an HSM, or MetaMask Agent Wallet can replace that
   adapter later. Record only its public Base address in the policy.
2. Build and review the deterministic factory manifest:

   ```powershell
   python scripts/build_bounded_agent_wallet_bundle.py --version v2
   ```

3. Build the exact owner plan:

   ```powershell
   python scripts/plan_bounded_agent_budget.py `
     --owner 0xOWNER `
     --delegate 0xDELEGATE
   ```

4. Verify every policy field and the predicted wallet address. The predicted
   address commits `keccak256(policy)` in its CREATE2 salt, so changing the
   delegate, cap, verifier, or expiry changes the USDC destination.
5. If the reviewed factory is not deployed, send only the manifest's exact
   deterministic deployment transaction and confirm its runtime hashes.
6. Sign the plan's one EIP-3009 `TransferWithAuthorization`. A gas relayer calls
   `createWalletWithAuthorization`, atomically deploying the exact policy-bound
   wallet and moving only the authorized USDC amount.
7. Independently inspect owner, delegate, factory, token, policy, policy
   version, nonce, counters, wallet balance, registration, and runtime hashes.
8. Start the delegate loop. No further owner prompt is required while the live
   action remains inside the policy.

A smart-contract owner that cannot produce an EIP-3009 EOA signature uses the
plan's exact approval plus `createWalletAndFund` fallback. The hosted page
detects this account type, verifies the exact allowance after the first owner
transaction, submits the reviewed factory calldata in the second, and requires
the allowance to be fully consumed. Never send a private key or seed phrase to
the API, MCP server, repository, or a bounty.

## Owner Escape Hatch

The plan includes exact calldata for `revokePolicy()`. Revocation stops new
delegate actions immediately. The owner may then call
`withdrawToken(nativeUsdc, owner, balance)` or install a reviewed replacement
policy. Ownership transfer is two-step.

V2 adds two owner-only bounty recovery methods:

- `cancelAndWithdrawUnclaimedBounty(bounty)` requires a canonical bounty
  created by that wallet, status `Open` or `Claimable`, no solver, no active
  bond, no submission, and a positive wallet contribution. It atomically
  cancels and pulls only the wallet's refund.
- `withdrawCancelledBountyRefund(bounty)` handles an expired bounty already
  cancelled by another caller. It pulls only the wallet's recorded
  contribution and pro-rata timeout-bond bonus.

Other contributors retain their own `withdrawRefund()` rights. Neither method
can cancel claimed work, recover another creator's bounty, target a
non-canonical contract, or move another contributor's principal. The API and
MCP tool `plan_bounded_wallet_cancel_refund` choose the valid V2 action from
canonical indexed state. The owner still reviews the exact zero-value Base
transaction; no platform administrator can invoke it as the owner.

An existing wallet may replace its policy with one zero-value
`configurePolicy` transaction. Use a fresh delegate when rotating a compromised
signer. The activation page must read the complete live policy, accept only
reviewed verifier authority, simulate the exact call, and verify the receipt,
version, hash, caps, balance, and lifetime spend. Policy replacement starts a
fresh policy-period counter; the review page must disclose that before signing.

## Activation State

New activation plans use the V2 deterministic contract manifest:
[`deployments/bounded-agent-wallet-v2-base-mainnet.json`](../deployments/bounded-agent-wallet-v2-base-mainnet.json).
The historical V1 manifest remains at
[`deployments/bounded-agent-wallet-base-mainnet.json`](../deployments/bounded-agent-wallet-base-mainnet.json).
The current live V1 policy constants and owner-transaction evidence are in
[`scripts/bounded_wallet_policy.py`](../scripts/bounded_wallet_policy.py).
Runtime inspection remains authoritative. Do not transfer USDC to a predicted
wallet before deployment and inspection pass.

The harness covers policy substitution, unauthorized modules and verifier
sets, target and spend caps, replay, signature malleability, policy rotation,
gross bond accounting, deterministic bytecode, native Base USDC, Base Sepolia,
and an exact mainnet fork. A wallet action or transaction hash is not earned
value; only confirmed canonical `BountySettled` proves payout.
