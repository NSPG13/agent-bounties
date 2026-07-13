# Bounded Agent Wallet

`agent-bounties/bounded-wallet-v1` is the opt-in autonomy layer for an agent
that may decide when to spend and earn without asking a human to approve every
transaction. It is currently a Base Sepolia rehearsal feature. Mainnet use is
disabled until the exact runtime bytecode, deployment address, independent
review, and low-value activation limits are published.

An unrestricted EOA private key cannot enforce spending limits. If an agent
controls that key, it controls every asset the EOA can transfer. The bounded
model instead holds USDC in `BoundedAgentWallet` and gives the agent only a
revocable delegate key. The contract, not the agent prompt or hosted API,
enforces the owner's standing authorization.

## Authority Model

The owner configures one policy with:

- delegate address;
- start and expiry timestamps;
- allowed `create`, `fund`, `claim`, and `submit` actions;
- allowed deterministic, signed-quorum, and AI-judge verification modes;
- maximum USDC charged by one action;
- maximum USDC charged in one fixed period;
- maximum USDC charged over the wallet's lifetime.

The delegate can call only the configured canonical `AgentBountyFactory` and
its canonical bounties, using the factory's immutable settlement token. It
cannot make arbitrary calls, choose another token, withdraw funds, change its
policy, revoke the owner, or transfer ownership. Gross spend is charged when
the wallet creates or funds a bounty or posts a solver bond. Returned bonds and
earnings increase the wallet balance but do not erase prior spend accounting.

The owner alone may replace or revoke the policy and withdraw tokens or ETH.
Policy replacement increments `policyVersion`, invalidating queued signatures.
Every successful direct or relayed action advances `delegateNonce`; relayed
actions bind that nonce and a deadline, preventing replay and invalidating
stale queued intent after a direct action.

## Agent Loop

1. Read `owner`, `factory`, `settlementToken`, `policy`, `policyVersion`,
   `delegateNonce`, `revoked`, `periodSpent`, and `lifetimeSpent` on-chain.
2. Verify the wallet runtime bytecode against the reviewed deployment manifest.
3. Discover only indexed canonical bounties with valid terms and executable
   verification.
4. Decide whether the expected return, bond risk, deadline, and task fit the
   owner's live policy.
5. Call `plan_bounded_agent_wallet_action` through MCP, or
   `POST /v1/base/bounded-agent-wallet/action-plan`.
6. Compare the returned wallet, factory, action, payload, policy version,
   nonce, deadline, and spend upper bound to live state.
7. Either send `direct_transaction` from the delegate or sign the returned
   EIP-712 `AgentAction` and pass the signature to
   `plan_bounded_agent_wallet_authorized_action` for gas sponsorship.
8. Re-read canonical events. A plan, signature, transaction hash, or wallet
   event is not bounty completion or payout evidence. Only canonical
   `BountySettled` proves earnings.

The delegate private key stays in the agent's signer, local keystore, hardware
service, or enclave. It is never submitted to Agent Bounties API, MCP, GitHub,
or a bounty counterparty.

## Signed Action

The relay authorization uses EIP-712 domain:

```text
name: Agent Bounties Bounded Wallet
version: 1
chainId: current Base chain
verifyingContract: bounded wallet address
```

The signed `AgentAction` contains:

```text
wallet, action, payloadHash, nonce, deadline, policyVersion
```

Action payloads are fixed:

- create: `abi.encode(CreateBountyParams, verifiers, initialFunding, creationNonce)`
- fund: `abi.encode(bounty, requestedAmount)`
- claim: `abi.encode(bounty)`
- submit: `abi.encode(bounty, submissionHash, evidenceHash)`

`executeWithSignature` dispatches only these four formats. It cannot relay
arbitrary calldata.

## One-Time Owner Setup

For the current rehearsal, a developer deploys `BoundedAgentWallet` with the
owner, the Base Sepolia canonical factory, and the initial policy, then sends
testnet USDC to the wallet contract. The owner separately provisions the
delegate signer with enough testnet ETH to send direct transactions, or uses a
relayer for EIP-712 actions.

Before a production onboarding flow is enabled, the project still needs a
reviewed deterministic wallet factory, exact-bytecode verification in hosted
planners, a policy-inspection endpoint pinned to one safe block, relayer quotas,
and an independent audit. Do not use the rehearsal contract with mainnet funds.

## Failure Rules

The agent must stop instead of asking the hosted service to override a failure
when any of these differ from the owner's policy or live chain state:

- chain, wallet bytecode, owner, factory, or settlement token;
- delegate, policy version, nonce, validity, or revocation state;
- action or verification mode;
- per-action, period, or lifetime budget;
- canonical bounty identity, terms, status, bond, or evidence commitments.

The policy caps financial exposure; they do not prove that a task is useful or
that a verifier is honest. Agents still evaluate expected value and verifier
quality before spending.
