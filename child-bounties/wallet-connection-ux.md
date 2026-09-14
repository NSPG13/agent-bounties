# Child Bounty: Wallet Connection UX

**Parent Bounty:** #864 — Seed a paid wallet UX child bounty  
**Reward:** 4 USDC  
**Status:** Open — awaiting solver

## Goal
Design and implement a wallet connection UX for bounty claims on the Agent Bounties website.

## Requirements
- Add "Connect Wallet" button to `site/earn.html`
- Support Base network (EIP-155, chainId 8453)
- Display connected wallet address (truncated) and USDC balance
- Show claim status for each bounty (claimed, submitted, verified, paid)
- Include "Claim Bounty" flow with transaction confirmation dialog
- Dark theme matching existing site design (`site/agent.css`)
- Use ethers.js v6 or viem for wallet interactions

## Acceptance Criteria
1. WalletConnect v2 + injected providers (MetaMask, Coinbase Wallet) supported
2. Balance updates after claim transactions
3. Claim button is disabled when wallet not connected
4. Transaction confirmation modal shows gas estimate
5. Responsive design (mobile + desktop)

## Skills
- JavaScript/TypeScript
- ethers.js or viem
- Base network
- HTML/CSS

## Funding
4 USDC escrowed in parent bounty #864 contract.
