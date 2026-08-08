# Mini-SWE-Agent: Paid-Work Coding Environment

A sandboxed mini-SWE-agent environment for autonomous bounty hunting on the
NSPG13 agent-bounties platform. Handles discovery, claim planning, evidence
submission, and settlement of paid coding tasks.

## Architecture

```
inventory snapshot → select_bounty.py → action (claim/wait/skip/refresh)
                                        ↓
                               operator-authorized claim
                                        ↓
                              implementation + evidence
                                        ↓
                              BountySettled (canonical)
```

## Selector

`select_bounty.py` reads a canonical inventory snapshot and emits one exact
next action:

| Action  | Trigger |
|---------|---------|
| `claim` | Claimable coding task with positive margin |
| `wait`  | No claimable tasks or empty inventory |
| `skip`  | Already claimed by another solver, or zero margin |
| `refresh` | Stale snapshot (>24h) or stale records |

### Required Fixtures

Five fixture files under `fixtures/` exercise all selector code paths:

| Fixture | Expected action |
|---------|----------------|
| `multiple.json` | `claim` (highest-margin task selected) |
| `empty.json` | `wait` (no records) |
| `stale.json` | `refresh` (stale snapshot) |
| `no-margin.json` | `skip` (zero/negative margin) |
| `exclusive-claimant.json` | `skip` (claimed by another solver) |

## Evidence Package

Each submission must include:

- `repository` — GitHub repository URL
- `commit_hash` — The commit containing the implementation
- `test_command` — The command to verify the change
- `source_snapshot_digest` — Hash of the canonical inventory used for selection
- `discovery_source` — The canonical inventory endpoint or path
- `participation_reason` — Why this task was selected
- `improvement_feedback` — Notes on the implementation approach

## Settlement

A submission is NOT settlement. Only the canonical `BountySettled` on-chain
event confirms payment. The environment preserves the boundary between
submission and settlement to prevent premature payment assumptions.

## Security

- No private keys, seed phrases, mnemonics, or `eth_sendTransaction` calls
- All wallet actions require explicit operator authorization
- Selector uses direct argv (`["python", "select_bounty.py", "--input", "file.json"]`)
- Never executes shell command strings; uses argument lists exclusively

## Config

See `config.yaml` for the full agent configuration including templates,
environment variables, model settings, and evidence requirements.
