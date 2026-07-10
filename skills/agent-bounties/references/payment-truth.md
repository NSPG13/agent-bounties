# Payment Truth

Use the strongest statement supported by durable evidence and no stronger.

| Evidence | Allowed statement | Not allowed |
| --- | --- | --- |
| Posted issue or suggested amount | A bounty candidate was posted | Funded, claimable, payable |
| Funding comment or intent | Someone signaled funding interest | Money received, funded |
| Stripe Checkout created/returned | Checkout was created/returned | Ledger credit, funded |
| Verified `checkout.session.completed` reconciliation | Stripe funding was applied | Solver paid |
| Base transaction plan/hash | A transaction was planned/broadcast | Escrow funded or released |
| Indexed `EscrowCreated` reconciled into scoped status | Base escrow funding was applied | Work accepted, solver paid |
| Deterministic verifier accepted submission | Completion was verified | Payout completed |
| Indexed `EscrowReleased` or reconciled Stripe transfer | The matching payout was paid | Other bounties or recipients paid |
| Simulated ledger/demo | The local harness completed | Real value moved |

For claimable work require all of:

1. Scoped bounty status is `Claimable`.
2. `funding_summary.claimable` is true and applied amount covers the target.
3. The advertised real rail has matching reconciled evidence.
4. The bounty is unclaimed and has no clearly active implementation lane.
5. Scope and acceptance evidence are available to the solver.

AI judges can filter quality, request clarification, or route review. They never
authorize escrow release, Stripe transfer, refund, or settlement.
