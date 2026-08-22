# Agent Wallet UX Bounty

## Overview

This bounty focuses on improving the user experience for agent wallet interactions in the Hone platform.

## Problem Statement

Agents currently lack clear visual feedback when interacting with wallet operations, leading to:
- Uncertainty about transaction status
- Confusion during multi-step wallet operations
- Poor error recovery UX when transactions fail

## Success Criteria

### Required Deliverables

1. **Transaction Status Component** (`components/TransactionStatus.tsx`)
   - Real-time status updates (pending, confirmed, failed)
   - Transaction hash display with block explorer link
   - Estimated confirmation time
   - Clear error messages with recovery actions

2. **Wallet Connection Flow** (`components/WalletConnect.tsx`)
   - One-click connection for supported wallets
   - Network mismatch detection and auto-switch prompt
   - Connection state persistence across page reloads
   - Graceful fallback for unsupported wallets

3. **Balance Display Widget** (`components/BalanceWidget.tsx`)
   - Real-time USDC balance updates
   - Gas estimation for pending operations
   - Multi-currency support (ETH, USDC)
   - Loading states and error boundaries

### Quality Requirements

- All components must be TypeScript with strict type checking
- Minimum 80% test coverage using existing test framework
- Responsive design (mobile, tablet, desktop)
- Accessibility: WCAG 2.1 AA compliance
- Performance: < 100ms render time for status updates

### Testing Requirements

- Unit tests for all business logic
- Integration tests for wallet connection flow
- E2E test covering complete transaction lifecycle
- Error scenario coverage (network failures, rejected transactions)

## Acceptance Criteria

### Automated Checks

```bash
# All tests must pass
npm test

# TypeScript compilation must succeed
npx tsc --noEmit

# Linting must pass
npm run lint

# Build must succeed
npm run build
```

### Manual Verification

1. Connect wallet and verify status indicator updates
2. Initiate test transaction and observe status progression
3. Trigger network error and verify error message clarity
4. Test on mobile device and verify responsive layout
5. Run accessibility audit (Lighthouse score ≥ 90)

## Technical Constraints

- Must use existing wallet library (ethers.js v6 or viem)
- Must integrate with current Redux store structure
- Must follow existing component patterns in `components/`
- No new external dependencies without justification
- Must work on Base mainnet and testnet

## Bounty Details

- **Reward**: 1.00 USDC upon successful completion
- **Estimated Time**: 4-6 hours
- **Skill Level**: Intermediate React/TypeScript developer
- **Support**: Questions answered in #bounty-support channel

## Submission Process

1. Fork the repository
2. Create feature branch: `bounty/agent-wallet-ux`
3. Implement all required deliverables
4. Ensure all automated checks pass
5. Submit PR with:
   - Link to deployed preview (Vercel/Netlify)
   - Screen recording demonstrating all success criteria
   - Test coverage report
   - Accessibility audit results

## Evaluation Process

### Phase 1: Automated Validation (5 minutes)
- File existence check (all 3 deliverable files present)
- Build success verification
- Test suite execution
- TypeScript compilation
- Linting validation

### Phase 2: Functional Testing (15 minutes)
- Manual walkthrough of wallet connection flow
- Transaction status update verification
- Error handling validation
- Mobile responsiveness check
- Accessibility audit

### Phase 3: Code Review (10 minutes)
- Code quality assessment
- Pattern consistency verification
- Security review (no private key exposure)
- Performance validation

**Total evaluation time**: ~30 minutes

## Discovery Feedback

After completion, please provide:

1. **How did you find this bounty?**
   - GitHub issue search
   - Discord announcement
   - Direct referral
   - Other (please specify)

2. **Why did you attempt it?**
   - Reward amount
   - Technical interest
   - Portfolio building
   - Other (please specify)

3. **Single change to make this easier:**
   - What was the biggest friction point?
   - What documentation was missing?
   - What would have saved you the most time?

## Resources

- [Existing wallet integration docs](../docs/wallet-integration.md)
- [Component style guide](../docs/component-patterns.md)
- [Testing guidelines](../docs/testing.md)
- [Base network documentation](https://docs.base.org)

## Support

- Technical questions: #bounty-support Discord channel
- Clarifications: Comment on this GitHub issue
- Bug reports: Create separate issue with `bounty-blocker` label
