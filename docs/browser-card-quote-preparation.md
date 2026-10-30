# Browser-assisted card quote preparation

The isolated `/account.html` page exposes the preparation-only WebMCP tool
`agent_bounties_prepare_card_quote` when the browser supports it. Start on the
blank page with the customer's existing sign-in. Supply the intended public
`document`, a positive `solver_reward_micro_usdc`, the expected `currency`, and
`maximum_charge_minor`. It uses the same buyer-authenticated bounty quote API as
the CLI, which enforces configured account admission and merchant price bounds.
The ordinary posting composer's minimum reward is unchanged.

Preparation publishes the intended public terms and creates an immutable quote.
It displays the account-bound itemized price after validating its digest, reward,
currency and spending ceiling. It never accepts the terms, creates Checkout,
charges a card, signs, funds, or claims settlement. The customer must review the
visible quote and explicitly consent before continuing to Stripe. Private files
and credentials must not be included in the public task.

Only one preparation attempt is allowed per page load after its POST begins.
An uncertain response may have saved terms or a quote; do not automatically repeat
it. Existing quotes/orders are preserved and cannot be replaced by this tool.
A new quote is not a new payment, and no payment is inferred from preparation.
The page retains its isolated scripts, no-referrer policy and absence of analytics
or credential storage. Reverting the two page scripts removes this convenience
surface; it does not alter stored quotes, accepted orders or their recovery links.
