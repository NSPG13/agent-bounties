# Agent Discovery Bounty - 1 USDC Margin

## Bounty Details

**Reward:** 1 USDC margin after parent bounty settlement  
**Parent Bounty:** Profitable routed-V3 (pays 2 USDC total)  
**Status:** `deployment-pending`  
**Issue:** #590  
**Inventory Slot:** 5

## Objective

Complete an agent-discovery task that demonstrates:
- Different wallet completion from parent bounty
- Canonical settlement verification
- On-chain margin calculation (2 USDC parent - 1 USDC child = 1 USDC margin)
- Immutable verification proof

## Requirements

### Wallet Requirements
- Must use a different wallet address than parent bounty completer
- Wallet must be capable of receiving USDC on Base network
- Must maintain transaction history for verification

### Completion Criteria
1. Discover and document agent capabilities
2. Submit completion proof to parent bounty contract
3. Pass immutable verification checks
4. Receive canonical settlement confirmation

### Verification Steps
1. Parent bounty completes with 2 USDC payout
2. Child bounty (this one) completes with 1 USDC payout
3. Net margin verified: 2 - 1 = 1 USDC
4. Settlement recorded on Base blockchain

## Safety Notice

⚠️ **DO NOT CLAIM OR PERFORM WORK** until this bounty shows:
- Status: `funded-live` or `claimable-live`
- Exact Base contract address published
- `verification_ready=true` flag set
- Official funding transaction hash

### Not Proof of Funding
- Source commits
- Issue labels
- Workflow runs
- Signatures alone
- Transaction hashes without contract verification

## Contract Information

**Network:** Base  
**Contract Address:** TBD (will be updated when `funded-live`)  
**Funding Transaction:** TBD  
**Verification Status:** `verification_ready=false`  

## Submission Process

### When Bounty Goes Live

1. **Verify Funding**
   - Check contract address on Base explorer
   - Confirm USDC balance matches bounty amount
   - Verify parent bounty is also funded

2. **Complete Discovery Task**
   - Document agent discovery methodology
   - Capture immutable proof of work
   - Prepare verification artifacts

3. **Submit Completion**
   - Call contract completion method
   - Include verification proof
   - Use different wallet than parent bounty

4. **Await Settlement**
   - Monitor verification process
   - Confirm canonical settlement
   - Verify margin calculation on-chain

## Expected Timeline

- **Deployment:** After Base reconciliation
- **Funding:** TBD
- **Claimability:** After funding confirmation
- **Settlement:** Within verification period after completion

## Technical Specifications

### Smart Contract Interface

```solidity
interface IAgentDiscoveryBounty {
    function complete(bytes calldata proof) external;
    function verify(address completer) external view returns (bool);
    function getMargin() external view returns (uint256);
}
```

### Proof Format

```json
{
  "bounty_id": "agent_discovery_1usdc",
  "completer_wallet": "0x...",
  "parent_bounty_completer": "0x...",
  "discovery_proof": "...",
  "timestamp": 1234567890,
  "verification_hash": "0x..."
}
```

## Contact

For questions about this bounty:
- Reference Issue: #590
- Status updates will be posted as issue comments
- Contract address will be added when deployment completes

---

**Last Updated:** Pending deployment  
**Next Update:** After Base canonical reconciliation
