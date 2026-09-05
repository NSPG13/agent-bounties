# Bounty Activation Checklist

## Parent Bounty #648 - MCP Interoperability

This document tracks the activation requirements for the parent bounty before child bounties can be claimed.

### Required On-Chain Evidence (Base Mainnet)

- [ ] Exact routed-V3 parent contract address published
- [ ] Full 2.01 USDC funding confirmed on-chain
- [ ] BountyBecameClaimable event emitted
- [ ] Valid immutable terms recorded
- [ ] Executable routed-V3 child-preparation path verified
- [ ] Pinned sandboxed_regression_v1 verifier path confirmed

### Activation Workflow

The following labels will be applied **only after** all readiness checks pass:

1. `bounty` - Marks as official bounty
2. `funded-live` - Confirms on-chain funding
3. `claimable-live` - Enables claiming by participants

### Important Notes

- Draft issues, labels, transaction plans, signatures, or transaction hashes are **NOT** funding, claimability, completion, or payment
- Only canonical `BountySettled` event proves payment
- Do not claim, sign, bond, fund a child, or start work until activation is complete

### Verification Steps

1. Check Base mainnet explorer for contract deployment
2. Verify USDC balance matches 2.01 USDC
3. Query contract for BountyBecameClaimable event
4. Validate immutable terms match issue description
5. Test child-preparation path execution
6. Confirm verifier path is pinned and accessible

### Post-Activation

Once activated, the solver will:

1. Post one concrete 1.00 USDC MCP interoperability coding child bounty
2. Fully fund the child bounty
3. Wait for a different registered participant to complete the work
4. Process canonical settlement upon completion
5. Receive 2.00 USDC payout, leaving 1.00 USDC gross margin

### Contact

For questions about activation status, check the parent issue #648 for updates.
