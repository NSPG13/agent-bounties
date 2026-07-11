# Main Branch Ruleset

The canonical configuration is `.github/rulesets/main.json`. It protects only
the default branch; contributor branches and forks remain unrestricted.

## Security Properties

- Every update to `main` has a pull request audit trail.
- The latest reviewable push needs one independent approval.
- Review threads must be resolved before merge.
- `full-check` and `postgres-sync` must come from GitHub Actions and pass.
- Force pushes and deletion of `main` are blocked.
- Only squash merges are accepted.

The status checks use loose mode. Contributors do not have to rebase and rerun
CI solely because another pull request merged first. Maintainers must still
inspect conflict risk before merging.

Repository administrators have PR-only bypass. They can recover from a broken
gate, but cannot update `main` without opening a pull request. Bypasses must be
explained in that pull request and followed by a corrective issue when a normal
gate was skipped.

Signed commits, strict up-to-date checks, merge queues, and broad restrictions
on contributor branches are intentionally omitted because they add recurring
friction without improving the current threat boundary enough to justify it.

## Apply And Verify

```powershell
gh api --method POST repos/NSPG13/agent-bounties/rulesets `
  --input .github/rulesets/main.json

gh api repos/NSPG13/agent-bounties/rulesets
```

If a ruleset with this name already exists, update its numeric endpoint with
`PUT` instead of creating a duplicate. Any future required check must first run
successfully on a pull request and must be bound to its expected GitHub App.

## Drift Check

`scripts/ruleset_drift_check.py` is a read-only checker that confirms the live
ruleset still matches this canonical file. It authenticates through `gh`, reads
(never writes) the live ruleset, ignores only server-owned fields (ids,
timestamps, and source links), and semantically validates every protection
listed above. A maintainer with `gh` authenticated runs:

```bash
python scripts/ruleset_drift_check.py
```

It exits non-zero and prints each difference when the live ruleset drifts from
the canonical file or when either side stops encoding a documented protection.
Offline fixture coverage runs on every pull request via the `ruleset-drift` CI
job (`python scripts/test_ruleset_drift_check.py`), so no live credentials are
needed in CI.
