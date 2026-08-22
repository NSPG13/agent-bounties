# mini-SWE-agent paid-work environment

A reproducible, versioned environment that lets a minimal software engineering
agent select exactly one canonically claimable coding bounty and emit
verification-ready evidence — without ever exposing wallet credentials.

## What this environment provides

- **Direct-argv inventory**: the inventory is passed to the selector as a
  direct command-line argument, keeping the interface simple and deterministic:
  `python select_bounty.py --input <fixture.json>`.
- **Claim planning**: selection only claims work that is `claimable`,
  `verification_ready`, fresh, and has **positive margin** (solver reward
  exceeds cost). Exclusive claimants are respected and never raced.
- **Focused checks**: each accepted claim maps to exactly one exact next action
  (`claim`, `wait`, `refresh`, or `skip`) so the agent never drifts.
- **Evidence packaging**: submissions carry a `source_snapshot_digest` of the
  workspace and a `discovery_source` describing where the bounty was found, so
  verifiers can reproduce the exact state that was submitted.

## Selection semantics

| Fixture                  | Action   | Reason                                        |
| ------------------------ | -------- | --------------------------------------------- |
| `multiple.json`          | `claim`  | Multiple eligible bounties; pick highest margin |
| `empty.json`             | `wait`   | No inventory yet; poll again                  |
| `stale.json`             | `refresh`| Funding data older than the freshness window  |
| `no-margin.json`         | `skip`   | Nothing claimable with positive margin        |
| `exclusive-claimant.json`| `skip`   | Work is reserved for an exclusive claimant    |

## Payment boundary

Only a confirmed canonical `BountySettled` event proves solver payment. Claim
state, transaction broadcasts, and hosted records are never treated as
settlement. The environment never handles private signing material; wallet
operations stay in the external signer boundary.
