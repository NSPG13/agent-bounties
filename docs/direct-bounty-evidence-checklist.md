# Direct-Bounty Evidence Checklist

Bind four things. Keep three buckets separate. One rule above all:
**a pull request, a green run, and a payment receipt are three different claims.**

## The four bindings

| Bind | Field | Rule |
|---|---|---|
| Where the code came from | `repository` | `https://github.com/<owner>/<repo>` |
| Exactly which code | `source_commit` | pinned 40- or 64-char hex — **never `main`, `HEAD`, `latest`** |
| Where in the repo | `subdirectory` | relative path, `.` at root, no `..` |
| How it was delivered | `pull_request_url` | `https://github.com/<o>/<r>/pull/<n>` |
| What was produced | `artifact_url` + `artifact_sha256` | `https://` only, immutable, 64-hex sha256 |
| What checked it | `check_run_urls` | non-empty, all `https://`, no duplicates |

## The three buckets — do not merge them

```
submission     what was delivered    commit · repo · subdir · PR · artifact
verification   what checked it       check-run / benchmark URLs
payment        what paid             canonical settlement receipt ONLY
```

- `complete: true` means the **deliverable is bound** (submission + verification).
- It does **not** mean paid. Payment is a separate claim with a separate boundary.

## The payment rule

Local input can never assert payment.

| Local says | Result | Why |
|---|---|---|
| nothing | `unverified` | no receipt |
| `settled: true` | `unverified` | self-reported — that is the claim being checked |
| tx hash only | `unverified` | a hash is not a settlement event |
| `provenance: self_reported` | `unverified` | not live-canonical |

Settled requires **all four**: `provenance == "canonical_live"`, `tx_hash` (`0x` + 64 hex),
`log_key`, and a positive integer `block_number`.

## Mutable references — auto-rejected

```
/latest   /main/   /master/   /HEAD/   ?ref=main   branch=   @main
```

A URL that can change under you is not evidence. Pin the commit.

## Payload safety

Keys matching `token · secret · password · api_key · private_key · seed · mnemonic ·
email · author · maintainer · wallet · address · cookie · session · credential`
are scrubbed before the payload is returned, so it is safe to drop into an
MCP or API response.

## Use

```python
from direct_evidence_checklist import build_checklist

out = build_checklist({
    "repository": "https://github.com/NSPG13/agent-bounties",
    "source_commit": "94b49dcf1e2ac0f3b7d5a6c8e90f1a2b3c4d5e6f",
    "subdirectory": "crates/sdk-python",
    "pull_request_url": "https://github.com/NSPG13/agent-bounties/pull/9001",
    "check_run_urls": ["https://github.com/NSPG13/agent-bounties/actions/runs/1001"],
    "artifact_url": "https://example.org/artifacts/bounty-686.tar.gz",
    "artifact_sha256": "e" * 64,
})
# out["complete"] -> True ; out["payment"]["status"] -> "unverified"
```

Tests: `python3 test_direct_evidence_checklist.py` — 18/18, offline, exits 0.
