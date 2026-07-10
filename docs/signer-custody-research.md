# Settlement Signer Custody Research

This note records the design constraint for Agent Bounties settlement signing:
make payout broadcast easy, but do not put an unrestricted private key in the
repo, browser logs, GitHub issues, or hosted API process by default.

## Comparable patterns

- Coinbase Developer Platform wallets support user wallets and server-controlled
  wallets. Their docs describe API-key authenticated system wallets for
  automated workflows and agentic applications, with private key operations
  handled inside a Trusted Execution Environment:
  <https://docs.cdp.coinbase.com/wallets/non-custodial-wallets/overview> and
  <https://docs.cdp.coinbase.com/wallets/security-and-policies/security-overview>.
- Privy supports server-side wallet access for offline actions, but its docs
  pair that with wallet policies for transfer limits, contract allowlists, and
  calldata constraints:
  <https://docs.privy.io/wallets/wallets/server-side-access> and
  <https://docs.privy.io/security/wallet-infrastructure/policy-and-controls>.
- Turnkey's agentic-wallet docs frame AI agents and automated backends as
  delegated actors with granular policies. Their policy examples constrain
  destination address, contract address, function selector, chain ID, and value:
  <https://docs.turnkey.com/features/policies/delegated-access/agentic-wallets>
  and <https://docs.turnkey.com/solutions/company-wallets/agentic-wallets>.
- Gitcoin Allo separates pooled funding from distribution by routing funds
  through an allocation strategy. Pools can be funded by others, but the strategy
  controls distribution:
  <https://docs.allo.gitcoin.co/allo/flow-of-funds> and
  <https://docs.allo.gitcoin.co/overview/pool>.
- StandardBounties separates issuer, contributor, fulfiller, and approver roles;
  approvers accept fulfillments and choose payout amounts:
  <https://github.com/Bounties-Network/StandardBounties>.

## Recommendation

Use three signing tiers:

1. Browser injected wallet for the first live Base mainnet payouts.
   The operator connects the current settlement signer wallet, reviews
   `/v1/base/release-plan`, signs `release(uint256,address[],uint256[],bytes32)`,
   then reconciles `/v1/base/transaction-receipt` with `reconcile_logs=true`.
2. Managed signer after repeated low-value loops. CDP, Privy, or Turnkey can
   sign programmatically only if policies restrict chain `8453`, escrow contract
   `0x150C6dFbCe7803cc7f634f59b0624e87349CEAce`, allowed function selectors,
   zero ETH value, daily limits, and required platform state.
3. Smart-contract strategy or multisig for higher-value work. Higher-value or
   dispute-prone bounties should require a strategy role, Safe-style threshold,
   or human/operator quorum before release.

Raw private-key signing is acceptable only for local or testnet rehearsals. It
must not be the production default.

## Current implementation

The static site exposes `operator.html`, an injected-wallet path for release
signing. It does not ask for a private key. The hosted API remains the source of
release calldata and receipt reconciliation. A release transaction hash is not
payout evidence; an indexed `EscrowReleased` event applied by the hosted API is
the settlement boundary.
