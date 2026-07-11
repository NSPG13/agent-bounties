# Payment Truth

Use the strongest statement supported by confirmed canonical evidence and no
stronger.

| Evidence | Allowed statement | Not allowed |
| --- | --- | --- |
| Published terms | A bounty specification exists | Contract exists, funded |
| Wallet prompt or signature | A wallet was asked or authorized | USDC moved |
| Transaction hash | A transaction was submitted | Confirmed funding or payout |
| Four factory creation events | Canonical bounty configuration exists | Fully funded |
| `FundingAdded` | Canonical contribution was recorded | Claimable unless target reached |
| `BountyBecameClaimable` with matching feed state | Bounty is fully funded and claimable | Solver paid |
| `SubmissionAdded` plus matching evidence preimages | Work was submitted for verification | Accepted or paid |
| `SubmissionRejected` | Verifiers rejected and were paid; bounty reopened | Solver paid |
| `BountySettled` | Exact solver and verifier amounts were paid | Any other bounty paid |
| `RefundWithdrawn` | Named contributor refund was transferred | All contributors refunded |

For claimable work require:

1. `site/protocol.json` is active with non-null verified factory and
   implementation.
2. The creation event emitter is that factory.
3. All creation events and content-addressed terms agree.
4. Funding equals the immutable target and the latest state is claimable.
5. Solver reward, positive verifier reward, and equal claim bond are explicit.
6. Acceptance evidence and verifier risk are inspectable before signing.

Advisory AI filters cannot authorize custody changes. A precommitted
AI-judge quorum of at least two exact signatures may settle under the immutable
on-chain policy. One model response never can.
