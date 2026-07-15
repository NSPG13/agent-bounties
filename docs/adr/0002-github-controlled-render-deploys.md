# ADR 0002: GitHub-Controlled Render Deploys

- Status: accepted
- Date: 2026-07-14
- Change class: R2, with R3-R4 exclusions

## Context

The Blueprint declared commit-triggered Render deploys, but Render did not
receive or act on reviewed main commits consistently. The scheduled operations
workflow detected revision skew but had no bounded application-deploy action.
Manual dashboard deploys restored service but were slow, unaudited, and not a
self-healing release path.

## Decision

- Disable native Render auto-deploy for API, MCP, and worker.
- Trigger deployment from a dedicated GitHub Actions `workflow_run` only after
  `CI` succeeds for a same-repository push to `main`.
- Require the target to be the latest successful CI SHA reachable from current
  main. A newer failed commit does not suppress the last known-good release.
- Resolve services by exact name and verify repository, branch, and type before
  mutation.
- Use Render's API to deploy the exact SHA and poll all three service deploys
  to terminal `live`.
- Verify API and MCP `/health` body, protocol, and exact revision headers.
- Keep the Render API key only in GitHub Actions and retain redacted evidence.
- Leave the scheduled public observer read-only and fail-closed.

## Consequences

Deployment no longer depends on Render receiving a GitHub commit event. CI is
the single release gate, and the application revision is explicit. The worker
has provider-level terminal evidence even though it has no public endpoint.

The GitHub secret can deploy application services and therefore requires
rotation and repository-administration controls. Provisioning, rotating, or
revoking it is R3; bounded exact-SHA application deploy use is R2. A missing or
revoked key stops deployment visibly. The controller cannot deploy contracts,
sign wallet transactions, move money, verify work, or settle bounties.

## Rejected Alternatives

- **Keep native commit deploys only:** already failed without a repository-side
  recovery path and deploys before CI completes.
- **Use per-service deploy hooks:** narrower credentials, but hook acceptance
  alone cannot prove the background worker reached `live`.
- **Run deployment from the scheduled observer:** would expose mutation
  credentials to a broad runtime-diagnosis surface and blur detection with
  release authority.
