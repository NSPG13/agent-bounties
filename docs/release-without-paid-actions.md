# Releases without paid GitHub Actions

The website uses GitHub Pages and standard runners in the public repository.
Those build minutes and Pages hosting are free. Scheduled website refreshes run
once daily; reviewed website changes still deploy on push. Live bounty and payment
data comes from the API and does not wait for a website rebuild.

The private runtime uses local release checks and direct Render builds. Its
duplicate CI, Containers and SDK Live Smoke Actions workflows are disabled in
GitHub's workflow settings. The private Pages workflow stays disabled. Do not
reenable them merely to get a green check. No new CI subscription is required.

For a private release:

1. Review the exact commit in a clean checkout. Run the repository gate,
   PostgreSQL checks and production-container smoke checks from that checkout.
   Preserve their exit codes, commit and tree IDs, and logs as private evidence.
2. Run the relevant contract, signature and payment tests. Any frozen verifier
   source mismatch still blocks that verifier; never refresh its expected hashes
   just to release unrelated code. Never mark an unrun check as passed.
3. For database changes, take a backup and rehearse upgrade and rollback against
   an isolated copy. Verify canonical submissions and payment records survive.
4. Use the reviewed maintainer release process to merge. Required checks and
   branch protections remain in place; a maintainer bypass needs actual local
   evidence. Do not auto-merge unreviewed contributor changes.
5. Deploy the reviewed commit through Render. Check both services' health and
   `x-agent-bounties-revision` headers, run the hosted smoke check, then update the
   production monitor's expected revision. Keep the previous release available.

The release gate is still mandatory. Local validation replaces the duplicate
private Actions jobs; Render builds alone do not replace tests. Local checks
currently require a maintainer machine and are not an unattended hosted CI system.

Payment workers, signing policies, review notices and production monitoring keep
their existing schedules. New contract deployments still need independent review
and a testnet rehearsal. Hosted AI generation stays disabled; never provision a
model-provider key as part of release recovery.

GitHub artifact storage, larger runners and other products can still cost money.
Keep logs only as long as needed and inspect billing before buying another host.
See [GitHub's billing rules](https://docs.github.com/en/billing/concepts/product-billing/github-actions).
