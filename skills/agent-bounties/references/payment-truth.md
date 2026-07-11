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
| `BountyBecameClaimable` with matching indexed feed state, or exact code/config/commitment/economic/funding checks at a Base `safe` block | Bounty is fully funded and claimable at the observed state | Solver paid or claim still available at a later block |
| `SubmissionAdded` plus matching evidence preimages | Work was submitted for verification | Accepted or paid |
| `SubmissionRejected` | Verifiers rejected and were paid; bounty reopened | Solver paid |
| `BountySettled` | Exact solver and verifier amounts were paid | Any other bounty paid |
| `RefundWithdrawn` | Named contributor refund was transferred | All contributors refunded |

For claimable work require:

1. The hosted protocol document is active with a non-null verified factory and
   implementation, or direct `safe`-block reads verify their exact code and
   configuration against the installed canary manifest.
2. The creation event emitter is that factory.
3. All creation events and content-addressed terms agree.
4. Funding equals the immutable target and the latest state is claimable.
5. Solver reward, positive verifier reward, and equal claim bond are explicit.
6. Acceptance evidence and verifier risk are inspectable before signing.

Advisory AI filters cannot authorize custody changes. A precommitted
AI-judge quorum of at least two exact signatures may settle under the immutable
on-chain policy. One model response never can.
