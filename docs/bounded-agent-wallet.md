# Bounded Agent Wallet

`BoundedAgentWallet` lets a human authorize an agent once, then lets that agent
create, fund, claim, and submit canonical bounties without asking the human to
approve every transaction. The owner keeps revocation, policy replacement,
withdrawal, and ownership control.

This does not give an agent the owner's MetaMask key. USDC moves into a separate
contract wallet, and the agent receives only a dedicated delegate signing key.
The contract enforces the limits even if the agent, its prompt, the hosted API,
or a relayer is compromised.

## Initial 89 USDC Policy

The proposed first mainnet policy is:

| Boundary | Value |
| --- | ---: |
| Network and asset | Base mainnet native USDC |
| Initial funding and lifetime gross spend | 89 USDC |
| Maximum one action | 5 USDC |
| Maximum each fixed 24-hour period | 10 USDC |
| Maximum bounty target | 5 USDC |
| Expiry | 30 days |
| Actions | create, fund, claim, submit |
| Verification | exact approved deterministic module only |

Signed-quorum and AI-judge bounties are disabled in the initial policy. Returned
claim bonds and bounty earnings increase the wallet balance but do not restore
the gross lifetime budget. The owner must explicitly replace the policy to
extend authority.

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
   python scripts/build_bounded_agent_wallet_bundle.py
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

An EOA that cannot use EIP-3009 may use the plan's exact approval plus
`createWalletAndFund` fallback. That takes two owner transactions. Never send a
private key or seed phrase to the API, MCP server, repository, or a bounty.

## Owner Escape Hatch

The plan includes exact calldata for `revokePolicy()`. Revocation stops new
delegate actions immediately. The owner may then call
`withdrawToken(nativeUsdc, owner, balance)` or install a reviewed replacement
policy. Ownership transfer is two-step.

## Activation State

The deterministic mainnet manifest is
[`deployments/bounded-agent-wallet-base-mainnet.json`](../deployments/bounded-agent-wallet-base-mainnet.json).
Until its factory address contains the exact reviewed runtime bytecode, it is a
deployment plan, not a live wallet. Do not transfer USDC to a predicted address
before deployment and inspection pass.

The harness covers policy substitution, unauthorized modules and verifier
sets, target and spend caps, replay, signature malleability, policy rotation,
gross bond accounting, deterministic bytecode, native Base USDC, Base Sepolia,
and an exact mainnet fork. A wallet action or transaction hash is not earned
value; only confirmed canonical `BountySettled` proves payout.
