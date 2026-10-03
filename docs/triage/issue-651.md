# Triage note: issue #651 ([META] 1 USDC margin bounty distribution)

Issue #651 is an on-chain coordination bounty, not a repository code defect.
It requires real Base mainnet actions that cannot be represented as file edits:

1. Two distinct registered participants (`/agent-bounty register <wallet>`).
2. Publishing parent-bound child terms for a concrete bounty distribution task
   using the `sandboxed_regression_v1` threshold-two verifier quorum.
3. Creating and fully funding a child bounty with a total target of exactly
   1.00 USDC (parent pays 2.00 USDC, yielding 1.00 USDC gross margin).
4. Claiming the parent and receiving canonical settlement.

No source file in this repository encodes distribution, margin, or payout
logic whose behavior differs from the issue's expectations. The related
artifacts (`deployments/canonical-child-verifier-base-mainnet-deployment.json`,
`deployments/bounded-agent-wallet-base-mainnet.json`,
`docs/evidence/standing-meta-bounties-base-mainnet-2026-07-13.json`) are
deployment/evidence manifests; their own `evidence_boundary` fields state they
prove deployment, funding, and claimability — not completion or payout.

Verification for this bounty must therefore come from on-chain evidence (a
confirmed canonical `BountySettled` event for the child, followed by parent
settlement), not from the repository test suite. Any patch that edits evidence
manifests to imply creation, funding, or settlement that did not occur on-chain
would be forged financial evidence and must be rejected.

Scope for automated verification: none — no test subset in this repository
encodes the expected behavior of issue #651.
