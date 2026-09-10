# Posting journey recovery

Operator-requested maintainer change: preserve a posting journey through account,
wallet, device and network changes, with one exact terms review and canonical
completion evidence. This application release does not change bounty contracts,
verifier authority, relayer limits or wallet signing policy.

The open PR queue was inspected at main
`933c9c446a76d26f148a4f2defacf6453d02a7b2`. No proposed wallet/posting fix was
main-ready in the inspected queue. Account work such as PR #1196 may need to
retain the additive wallet metadata and owner-bound draft endpoints. Other open
benchmark, solver and inventory contributions keep their protocol contracts.

The implementation joins compact outcome/budget/calendar intake, immutable
reference image bindings, account-owned drafts, unchanged terms approvals,
explicit wallet capabilities, mobile app linking, exact Base balance reads,
provider purchase recovery and one canonical posting tracker. Creator-review
reserve terminology changes the display only. The committed reward still pays
the creator on pass or fail after a confirmed verdict.

Privacy and deployment risk are R3 for account-owned draft storage and R1 for
presentation. The database migrations are additive; prior app revisions remain
compatible with the added tables and nullable provider metadata. Rolling back
application code must retain saved drafts and all chain/transaction history.

Release gates include owner isolation, draft revision conflicts, unchanged
approval hashes, pending-wallet preservation, signature/account changes, exact
canonical event and ready-inventory matching, supported mobile links, bounded
provider startup failures and browser layout. No test may use customer funds.

Known external boundaries: Coinbase embedded wallets remain relay-only for
supported existing-bounty actions. This change cannot make a blocked Coinbase
endpoint reachable on every network, repair Trust Wallet/KOYWE, guarantee a
provider's fiat quote or minimum, or guarantee a fixed future Base gas price.
These limitations must be visible before a person attempts the unsupported step.

Contributor feedback: how did you find Agent Bounties, what made participation
worthwhile, which tool or workflow brought you here, and what would make the
posting journey easier or more trustworthy?
