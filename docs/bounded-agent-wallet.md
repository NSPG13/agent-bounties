# Bounded Agent Wallet

`agent-bounties/bounded-wallet-v1` is the opt-in autonomy layer for an agent
that may decide when to spend and earn without asking a human to approve every
transaction. The canonical Base Sepolia factory and policy wallets have passed
a complete create, claim, submit, deterministic verify, and settle rehearsal.
Mainnet remains disabled until its exact factory is deployed and the same
low-value canary passes.

## Current Activation

- Base Sepolia wallet factory:
  `0x38b5bec0b16d25ff1b0a6bb09f8f7f5a54dd3397`
- Factory runtime code hash:
  `0x119c73cb4442cf5a792e6b9e0ed20f1b811f6596b76cb3377c766732a2235a4c`
- Wallet runtime code hash:
  `0xca08c0045ab20776437a0443aeda5a5558126820043088e55a8040e2a0d03311`
- Canonical settlement rehearsal:
  `0xe39cc311c714579c6fdeff1702a28861a559ec25a17fb2a26f7a34e86ce414ee`
- Machine-readable deployment bundle:
  [`deployments/bounded-wallet-base-activation.json`](../deployments/bounded-wallet-base-activation.json)
- Compact rehearsal evidence:
  [`docs/evidence/bounded-wallet-base-sepolia-2026-07-13.json`](evidence/bounded-wallet-base-sepolia-2026-07-13.json)

Testnet USDC is not money. The evidence proves the integration path, not a
mainnet payout or production audit.

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

This is the only human-authority step. It establishes standing authorization;
it does not give the agent the owner's wallet or seed phrase.

1. The agent creates a dedicated delegate signer in its own keystore, hardware
   service, or enclave and gives only the public address to the owner.
2. The owner chooses an expiry, allowed actions, allowed verifier modes, and
   three USDC caps. Amounts use USDC minor units, so `1000000` is 1 USDC.
3. On Base Sepolia, the owner calls the canonical factory's
   `createWalletAndFund(owner, policy, userSalt, initialFunding)`. A smart wallet
   can batch the exact USDC approval and factory call in one confirmation.
   `createWalletWithAuthorization` is the gas-sponsored EIP-3009 alternative.
4. The agent obtains the emitted wallet address and calls the hosted inspection
   endpoint before doing any work:

```bash
curl "https://agent-bounties-api.onrender.com/v1/base/bounded-agent-wallet/0xWALLET/inspection?network=base-sepolia"
```

5. The agent requires exact agreement on factory and wallet bytecode, owner,
   delegate, token, policy, policy version, nonce, counters, and active status.
6. The agent may then choose and execute allowed actions without another human
   confirmation until a cap, expiry, revocation, nonce, or chain-state check
   fails.

For direct delegate transactions, the delegate needs Base Sepolia ETH. For
relayed actions, it signs only the returned EIP-712 `AgentAction`; the gas
sponsor sends the transaction. Never send either private key to the hosted API,
MCP server, GitHub, or a bounty counterparty.

Mainnet onboarding is intentionally unavailable while the pinned mainnet
factory address has no code. Do not transfer mainnet assets to a predicted or
unverified address.

## Agent Request Example

After inspection, request one exact action. Values must match the safe-block
observation, and the deadline may be at most 15 minutes after that block:

```json
{
  "network": "base-sepolia",
  "wallet_action": {
    "wallet_contract": "0xWALLET",
    "delegate": "0xDELEGATE",
    "policy_version": 1,
    "delegate_nonce": 0,
    "deadline": 1800000000,
    "action": {
      "kind": "claim",
      "bounty_contract": "0xCANONICAL_BOUNTY",
      "expected_claim_bond": { "amount": 100000, "currency": "usdc" }
    }
  }
}
```

Send this body to MCP tool `plan_bounded_agent_wallet_action` or REST endpoint
`POST /v1/base/bounded-agent-wallet/action-plan`. Re-inspect after every action
because every successful direct or relayed action increments the shared nonce.

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
