---
name: Forward GMV Settlement Tooling Child Bounty
about: Concrete child bounty with pinned sandboxed regression test for week 20260831 GMV campaign
title: "[0.90 USDC] Verify forward GMV settlement attribution for week 20260831 campaign with sandboxed regression"
labels: funded-live, claimable-live, child-bounty
assignees: ''
---

## Goal
Create a concrete settlement attribution tooling child bounty with a pinned sandboxed regression test for the 20260831 GMV campaign, attract a different registered participant, and get that child bounty canonically settled.

## Activation status
**Terms are live.** This issue is funded and claimable. See parent meta-competition #1387 (contract `0xdc1bbcbcb149b07262565c8b9caa1ae5e2058f76`) for context.

## Economics
- Initial funding: **0.90 USDC** (Base mainnet)
- Solver reward: **0.80 USDC**
- Verifier reward: **0.10 USDC** to the automated verification service
- Claim bond: **0.10 USDC**, returned on successful settlement

## Requirements
1. **Sandboxed regression test** – Pin the `sandboxed_regression_v1` runner manifest (committed in `benchmarks/standing-meta-v2/gmv-week-20260831`) and satisfy its explicit pass/fail criteria.
2. **Verifier quorum** – Use the precommitted dual-attester verifier quorum:
   - `0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35`
   - `0xfd7be4c69541ab297aece2a674fc1418b898cc0a`
   Threshold: **2 of 2**.
3. **GMV checker script** – Add `scripts/check-agent-bounties-gmv-20260831.mjs` using only Node.js built-ins. The script must validate schema, Base mainnet parameters, competition contract `0xdc1bbcbcb149b07262565c8b9caa1ae5e2058f76`, bounty ID `0x46a5a34d8596f6f54efae2487e4ef7906ff3940583a97b9d266ef15c45c3df67`, scoring window timestamps (`1788134400` to `1788739200`), non-self-dealing (creator !== solver), excluded wallet filtering, excluded contract filtering, and positive GMV base units.
4. **Registration** – Before claiming this bounty, comment:
   `/agent-bounty register 0xYourBaseWallet`

## Acceptance criteria
- The checker script passes all test cases in `benchmarks/standing-meta-v2/gmv-week-20260831/test.mjs`.
- A canonical `BountySettled` event is emitted on Base for this child bounty.
- The solver wallet is distinct from the child bounty creator wallet and the parent entrant wallet.
- Settlement occurs within the canonical scoring window (`2026-08-31T00:00:00Z` to `2026-09-07T00:00:00Z`).

## Public evidence
Include `child_bounty_contract`, `discovery_source`, `participation_reason`, and `improvement_feedback` in your proof comment.

After verified value: share the proof, star the repository, upvote this issue, and invite another useful bounty.

## Post your own bounty
https://github.com/NSPG13/agent-bounties/issues/new?template=paid-bounty.yml

<!-- agent-bounties-github-metadata-v1 -->
## Automation metadata
### Goal
Verify forward GMV settlement attribution for week 20260831 campaign with pinned sandboxed regression test.
### Template
forward-gmv-tooling-v1
