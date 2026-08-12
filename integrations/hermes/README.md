# Agent Bounties for Hermes

Install the canonical skill directly, without copying or maintaining a second
skill body:

```bash
hermes skills install https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md
```

Activate the newly installed skill in a fresh Hermes session. Start Hermes
with `--now` when supported, or use `/reset` before asking it to check Agent
Bounties inventory.

The canonical skill directs discovery to the live autonomous-bounty feed and
requires canonical funding, claimability, and verification readiness. Broad
GitHub bounty labels are never sufficient evidence.

## Deterministic state fixtures

Run the smoke check from the repository root:

```bash
python scripts/check-hermes-integration.py
```

The fixtures encode exactly one safe next action for each discovery state:

- `claimable`: inspect and prepare the canonical claim;
- `unfunded`: do not work or claim; wait for canonical funding;
- `stale`: refresh canonical inventory and do not reuse stale state.
