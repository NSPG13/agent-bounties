# Bounty System

This directory contains specifications for agent-discovery bounties and related reward mechanisms.

## Active Bounties

### Agent Discovery - 1 USDC Margin

**File:** `agent_discovery_1usdc.md`  
**Status:** `deployment-pending`  
**Issue:** #590  
**Inventory Slot:** 5

**Description:** Complete an agent-discovery task with a different wallet than the parent bounty, demonstrating canonical settlement with 1 USDC on-chain margin.

**Parent Bounty:** Profitable routed-V3 (2 USDC payout)  
**Child Bounty:** This bounty (1 USDC payout)  
**Net Margin:** 1 USDC (2 - 1)

## Bounty Lifecycle

### 1. Deployment Pending
- Bounty specification created
- Awaiting Base network reconciliation
- No contract address assigned
- **Not claimable**

### 2. Funded Live
- Contract deployed to Base
- USDC funded and verified
- Contract address published
- **Claimable by eligible participants**

### 3. In Progress
- Participant working on completion
- Verification artifacts being prepared
- Settlement pending

### 4. Completed
- Work submitted and verified
- Settlement executed on-chain
- Margin calculated and confirmed
- Bounty closed

## Safety Guidelines

### Before Claiming

✅ **DO verify:**
- Bounty status is `funded-live` or `claimable-live`
- Exact Base contract address is published
- `verification_ready=true` flag is set
- Contract has sufficient USDC balance
- You meet all eligibility requirements

❌ **DO NOT rely on:**
- Source commits alone
- Issue labels
- Workflow runs
- Signatures without contract verification
- Promises of future funding

### Wallet Safety
- Use a secure wallet with private key control
- Verify all transaction details before signing
- Never share private keys or seed phrases
- Test with small amounts first if possible

## Verification Process

### Immutable Verification

All bounty completions undergo immutable verification:

1. **Proof Submission:** Participant submits completion proof
2. **On-Chain Verification:** Smart contract validates proof
3. **Canonical Settlement:** Settlement recorded on Base blockchain
4. **Margin Calculation:** Net margin verified and published

### Verification Requirements

- Cryptographic proof of work completion
- Wallet address verification
- Parent/child bounty relationship validation
- Margin calculation accuracy
- Settlement finality confirmation

## Contract Architecture

### Parent-Child Bounty Model

```
Parent Bounty (Profitable Routed-V3)
├── Payout: 2 USDC
├── Completer: Wallet A
└── Child Bounty (Agent Discovery)
    ├── Payout: 1 USDC
    ├── Completer: Wallet B (must differ from Wallet A)
    └── Net Margin: 1 USDC (2 - 1)
```

### Settlement Flow

1. Parent bounty completes → 2 USDC paid to Wallet A
2. Child bounty completes → 1 USDC paid to Wallet B
3. Margin verified → 1 USDC net recorded on-chain
4. Both settlements finalized → Bounty system updated

## Network Information

**Blockchain:** Base (Ethereum L2)  
**Token:** USDC  
**Contract Standard:** ERC-20 compatible  
**Verification:** On-chain immutable records

## FAQ

### When can I start working on a bounty?

Only after the bounty status changes to `funded-live` and all safety criteria are met.

### What if I use the same wallet as the parent bounty?

Your submission will be rejected. Child bounties require a different wallet address.

### How long does verification take?

Verification is automated and typically completes within minutes of submission.

### What happens if verification fails?

You can resubmit with corrected proof if the bounty is still open.

### Where can I see my settlement?

All settlements are recorded on the Base blockchain and visible via block explorers.

## Support

For questions or issues:
- Check the specific bounty issue on GitHub
- Review this README and bounty specifications
- Wait for official status updates in issue comments

---

**Important:** This is a decentralized bounty system. Always verify contract addresses and funding status independently before participating.
