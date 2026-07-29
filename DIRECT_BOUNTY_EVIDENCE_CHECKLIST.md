# Direct Bounty Evidence Checklist

Bind the repository commit, benchmark/check run, artifact digest, and canonical settlement boundary for direct coding bounties.

## Validation Rules

- All artifact references MUST use HTTPS URLs. Plain HTTP, IPFS-only, or mutable references (e.g. `latest`, `main`, branch tips) are REJECTED.
- Empty or missing required fields produce a validation failure.
- A PR URL, test result, or verifier response is NOT payment evidence. Only canonical `BountySettled` proves payment.

---

## Checklist Template

| # | Field | Required | Evidence | Source |
|---|-------|----------|----------|--------|
| **Submission Evidence** |
| 1 | Source Commit | Yes | Full commit SHA | `git rev-parse HEAD` |
| 2 | Repository URL | Yes | HTTPS clone URL | `.git/config` remote origin |
| 3 | Subdirectory | No | Relative path within repo | Affected source tree |
| 4 | Pull Request URL | Yes | HTTPS PR URL | GitHub PR link |
| **Verification Evidence** |
| 5 | Check‑run URL(s) | Yes | HTTPS URL(s) to CI/CD results | CI workflow run |
| 6 | Artifact Digest | Yes | SHA‑256 of release artifact | `sha256sum <artifact>` |
| 7 | Benchmark / Coverage | No | Percentage, score, or threshold | CI artifact or report |
| **Payment Evidence** |
| 8 | Bounty ID | Yes | Issue / bounty number | Bounty tracker |
| 9 | Settlement TX | Yes | HTTPS block‑explorer link to `BountySettled` | On‑chain event |
| 10 | Payout Amount | Yes | USDC amount settled | `BountySettled` event log |

---

## Example (passing)

| # | Field | Required | Evidence | Source |
|---|-------|----------|----------|--------|
| 1 | Source Commit | Yes | `abc123def4567890abcdef1234567890abcdef12` | `git rev-parse HEAD` |
| 2 | Repository URL | Yes | `https://github.com/NSPG13/agent-bounties.git` | `.git/config` |
| 3 | Subdirectory | No | `crates/verifier-sdk` | Source tree |
| 4 | Pull Request URL | Yes | `https://github.com/NSPG13/agent-bounties/pull/1` | GitHub |
| 5 | Check‑run URL(s) | Yes | `https://github.com/NSPG13/agent-bounties/actions/runs/123` | CI |
| 6 | Artifact Digest | Yes | `sha256:4bfb...e0ae` | `sha256sum` |
| 7 | Benchmark / Coverage | No | `85.2%` | CI coverage report |
| 8 | Bounty ID | Yes | `#686` | GitHub Issues |
| 9 | Settlement TX | Yes | `https://basescan.org/tx/0x...` | BaseScan |
| 10 | Payout Amount | Yes | `2.00 USDC` | `BountySettled` event |

## Malformed / Rejected Examples

| Case | Issue | Expected Outcome |
|------|-------|-----------------|
| Missing commit | Field #1 empty | REJECTED |
| Plain‑HTTP artifact | Field #6 uses `http://` | REJECTED |
| Mutable reference | Field #6 uses `latest` tag | REJECTED |
| Missing PR URL | Field #4 empty | REJECTED |
| No settlement TX | Field #9 empty | REJECTED |
| Verifier response as payment | Field #9 is a verifier URL | REJECTED (not `BountySettled`) |

---

## MCP / API Compact Form

```json
{
  "commit": "<sha>",
  "repo": "https://github.com/NSPG13/agent-bounties.git",
  "subdir": "crates/verifier-sdk",
  "pr_url": "https://github.com/NSPG13/agent-bounties/pull/1",
  "check_runs": ["https://github.com/NSPG13/agent-bounties/actions/runs/123"],
  "artifact_digest": "sha256:4bfb...e0ae",
  "benchmark": "85.2%",
  "bounty_id": "#686",
  "settlement_tx": "https://basescan.org/tx/0x...",
  "payout_usdc": "2.00"
}
```