# Bounty Template Checklist

Use this checklist when creating new bounties to ensure all critical elements are present.

## Essential Elements

### Problem Definition
- [ ] Clear problem statement
- [ ] Context explaining why this matters
- [ ] User impact description

### Deliverables
- [ ] Specific file paths listed
- [ ] Exact component/function names specified
- [ ] File format requirements stated
- [ ] Expected file structure documented

### Success Criteria
- [ ] Automated validation commands provided
- [ ] Manual verification steps listed
- [ ] Minimum quality thresholds defined (test coverage, performance, accessibility)
- [ ] Example outputs or screenshots included

### Technical Constraints
- [ ] Required libraries/frameworks specified
- [ ] Forbidden approaches documented
- [ ] Integration points identified
- [ ] Performance requirements stated

### Testing Requirements
- [ ] Unit test expectations defined
- [ ] Integration test scenarios listed
- [ ] E2E test coverage specified
- [ ] Error case coverage required

### Evaluation Process
- [ ] Automated checks documented with exact commands
- [ ] Manual verification steps with time estimates
- [ ] Pass/fail criteria clearly defined
- [ ] Total evaluation time estimated

### Submission Process
- [ ] Branch naming convention specified
- [ ] PR requirements listed
- [ ] Required artifacts enumerated (screenshots, videos, reports)
- [ ] Review timeline communicated

### Discovery Feedback
- [ ] Feedback questions included
- [ ] Response format specified
- [ ] Submission method documented

## Anti-Patterns to Avoid

### Vague Requirements
❌ "Improve the wallet UX"
✅ "Create TransactionStatus.tsx component with real-time status updates"

### Missing File Paths
❌ "Add a new component for balance display"
✅ "Create components/BalanceWidget.tsx with USDC balance display"

### Unclear Success Criteria
❌ "Make sure it works well"
✅ "All tests pass (npm test), TypeScript compiles (npx tsc --noEmit), Lighthouse accessibility score ≥ 90"

### Ambiguous Testing
❌ "Add some tests"
✅ "Minimum 80% test coverage, unit tests for all business logic, E2E test for complete transaction flow"

### No Validation Commands
❌ "Ensure quality is good"
✅ "Run: npm test && npx tsc --noEmit && npm run lint && npm run build"

## Grep-Anchored Validation

For automated pre-QA checks, ensure bounty briefs enable deterministic file verification:

```bash
# Example validation script
#!/bin/bash

# Extract required files from brief
REQUIRED_FILES=(
  "components/TransactionStatus.tsx"
  "components/WalletConnect.tsx"
  "components/BalanceWidget.tsx"
)

# Check git diff touches all required files
for file in "${REQUIRED_FILES[@]}"; do
  if ! git diff --name-only HEAD~1 | grep -q "^${file}$"; then
    echo "FAIL: Required file ${file} not modified"
    exit 1
  fi
done

echo "PASS: All required deliverable files present in diff"
```

## Quality Gates

### Tier 1: Automated (Must Pass)
- File existence verification
- Build success
- Test suite execution
- Linting validation
- Type checking

### Tier 2: Functional (Must Pass)
- Manual feature verification
- Error handling validation
- Responsive design check
- Accessibility audit

### Tier 3: Code Quality (Should Pass)
- Pattern consistency
- Security review
- Performance validation
- Documentation completeness

## Template Usage

1. Copy `docs/agent-wallet-ux-bounty.md` as starting point
2. Replace domain-specific content (wallet → your domain)
3. Update file paths to match your project structure
4. Adjust quality thresholds to match project standards
5. Verify all checklist items are addressed
6. Test automated validation commands
7. Estimate evaluation time realistically

## Lessons Applied

This template incorporates lessons from previous bounty attempts:

- **Grep-anchored validation**: Specific file paths enable deterministic pre-QA checks
- **Structured evaluation**: Three-phase process with time estimates
- **Clear quality gates**: Automated checks before human review
- **Discovery feedback**: Systematic collection of UX improvement data
- **Anti-pattern documentation**: Learn from common failure modes
