# mini-SWE-agent paid-work environment (v1)

This integration gives mini-SWE-agent a reproducible, fail-closed path from a
canonical Agent Bounties inventory snapshot to one verification-ready coding
change. It never turns a GitHub label, pull request, model response, or payment
plan into claimability or settlement evidence.

## Run the selector

Save the canonical `ready_to_earn` feed as `inventory.json`, then execute the
selector as direct argv (not a command passed through a shell):

```json
["python", "integrations/mini-swe-agent/select_bounty.py", "--input", "inventory.json", "--solver-wallet", "0xYourPublicBaseAddress"]
```

The selector emits exactly one `action` and one `next_action`:

- `claim`: one fresh, verification-ready coding bounty has positive gross cash
  margin and no conflicting exclusive claimant.
- `wait`: the inventory is empty or has no canonical coding work.
- `refresh`: the snapshot is stale or invalid.
- `skip`: work has no positive margin or belongs to another exclusive claimant.

Run mini-SWE-agent with the versioned configuration:

```text
mini -c mini.yaml -c integrations/mini-swe-agent/config.yaml
```

The first config supplies mini-SWE-agent's current defaults; the second adds the
paid-work lifecycle, a 40-step limit, and a USD 0.50 model-cost limit. Keep the
default confirmation mode. The operator supplies only a public Base address and
separately reviews any claim or submission request.

## Focused verification

From the repository root, run:

```json
["python", "benchmarks/direct-growth-v2/mini-swe-agent-environment/check.py"]
```

For local development, also run:

```json
["python", "-m", "unittest", "integrations.mini-swe-agent.test_select_bounty"]
```

The hyphenated integration directory is not importable as a Python package, so
the portable local command is:

```json
["python", "integrations/mini-swe-agent/test_select_bounty.py"]
```

## Evidence package

After focused checks pass, prepare the exact committed evidence fields:

- `repository`: public GitHub repository URL containing the submitted commit.
- `commit`: immutable commit SHA.
- `test_command`: direct argv used by the verifier.
- `source_snapshot_digest`: SHA-256 directory digest required by the committed
  sandbox verifier.
- `discovery_source`: where the canonical opportunity was found.
- `participation_reason`: why its observed margin and verification path justified
  the attempt.
- `improvement_feedback`: what would make discovery or completion easier.

The agent may prepare this package but must stop for operator authorization before
claiming, signing, relaying, pushing, opening a PR, or submitting evidence. A
passing local check is not income. A `SubmissionAdded` event proves only a
submission. Only the canonical Base `BountySettled` event proves payment.
