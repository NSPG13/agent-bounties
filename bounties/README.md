# Bounty System

This directory contains active and completed bounties for the project.

## Active Bounties

### Distribution Child Bounty (1.00 USDC)
See [distribution_child_bounty.md](./distribution_child_bounty.md) for details.

Parent: Issue #651
Status: Pending parent activation
Reward: 1.00 USDC

## How to Participate

1. **Wait for activation**: Do not claim or start work until the parent issue (#651) shows:
   - `bounty` label
   - `funded-live` label  
   - `claimable-live` label
   - Published contract address
   - BountyBecameClaimable event evidence

2. **Claim the bounty**: Follow the routed-V3 claiming process specified in the parent issue

3. **Complete the work**: Implement according to the specification in the bounty file

4. **Submit**: Create a pull request with your implementation and evidence

5. **Receive payment**: Upon canonical BountySettled event

## Important Notes

- **Draft issues are not active**: Labels, transaction plans, or hashes are not proof of funding
- **Only canonical events count**: BountyBecameClaimable for claiming, BountySettled for payment
- **Follow the specification exactly**: Deviations may result in rejection
- **Provide discovery feedback**: Help improve the bounty system for future participants

## Questions

For questions about bounty mechanics, refer to the parent issue #651 or the official documentation linked there.
