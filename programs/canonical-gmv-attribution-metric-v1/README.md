# Canonical GMV attribution metric v1

Status: R4 candidate, not reviewed or enabled for funded production use.

This SP1 program scores the wallet bound as the Open Competition V2 solver by
its contribution to externally funded, canonically settled marketplace GMV in
one frozen epoch:

```text
score = sum(settlement GMV * entrant funding / total funding)
```

Division rounds down for each settlement and the score uses native USDC base
units. The snapshot is strictly ordered and content-addressed. The verification
policy commits the epoch, safe block, end block hash, exclusion lists, minimum
score, exact snapshot hash, and reviewed program source identity.

The scorer assigns zero for operator/reserve entrants and skips excluded reward
contracts, creator-equals-solver settlements, and entrant-equals-solver
settlements. It proves wallet-level accounting, not that wallets are unrelated
people. Primary/shadow indexer agreement and public reproduction from canonical
events are required before a snapshot may enter a funded competition.

The checked-in candidate pool remains `awaiting_reproduction`; the planner and
owner confirmation page fail closed until two isolated ELF/vkey builds agree,
fixtures and adversarial tests pass, exact release hashes are reviewed, and
each selected epoch snapshot is marked `ready`.
