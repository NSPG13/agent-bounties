# Direct Bounty Evidence Checklist

> Machine-readable and human-readable evidence checklist for direct coding bounties.
> Binds: repository commit, benchmark/check run, artifact digest, and settlement boundary.

## Required Evidence

- [ ] **Source Commit**: SHA-256 of the exact commit fulfilling the bounty
  - Format: `commit: <full-40-char-sha>`
- [ ] **Repository**: Full GitHub URL with org/repo
  - Format: `https://github.com/<org>/<repo>`
- [ ] **Subdirectory**: If the change targets a monorepo sub-path
  - Format: `subdir: <path>`
- [ ] **Pull Request URL**: Link to the merged or submitted PR
  - Format: `https://github.com/<org>/<repo>/pull/<number>`
- [ ] **Check-Run URLs**: CI/CD verification that the deliverable passes
  - Format: `https://github.com/<org>/<repo>/actions/runs/<run_id>`
- [ ] **Artifact Digest**: Content hash (SHA-256) of the deliverable artifact
  - Format: `sha256:<hex-hash>`

## Submission Boundary

- The evidence above covers the submitter's claim scope.
- Any additional features, refactors, or fixes beyond the claim must be documented separately.
- The canonical settlement boundary is defined by the intersection of: the bounty description, the PR diff, and the verified check runs.

## Example

```yaml
commit: abc123def456789012345678901234567890abcd
repo: https://github.com/example-org/example-repo
subdir: packages/core
pr: https://github.com/example-org/example-repo/pull/42
check_runs:
  - https://github.com/example-org/example-repo/actions/runs/1234567890
artifact_digest: sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
```
