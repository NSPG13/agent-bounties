# Releases without paid GitHub Actions

The website uses GitHub Pages and standard runners in the public repository.
Those build minutes and Pages hosting are free. Scheduled website refreshes run
once daily; reviewed website changes still deploy on push. Live bounty and payment
data comes from the API and does not wait for a website rebuild.

Runtime source is released from public `NSPG13/agent-bounties` main through the
[public runtime release controller](public-runtime-provenance.md). Its CI and
Containers checks must pass for the exact revision being deployed. Private
operational records may remain private; executable production source may not.
The duplicate private CI, Containers, SDK Live Smoke and Pages workflows stay
disabled. Do not reenable them or the old environment-reconciling deploy
controller to obtain a green check.

For a runtime release:

1. Review the exact commit in a clean checkout. Run the repository gate,
   PostgreSQL checks and production-container smoke checks from that checkout.
   Preserve their exit codes, commit and tree IDs, and logs as release evidence.
   The public-source parity check must pass; local private-only changes cannot
   qualify a production release.
2. Run the relevant contract, signature and payment tests. Any frozen verifier
   source mismatch still blocks that verifier; never refresh its expected hashes
   just to release unrelated code. Never mark an unrun check as passed.
3. For database changes, take a backup and rehearse upgrade and rollback against
   an isolated copy. Verify canonical submissions and payment records survive.
4. Use the reviewed maintainer release process to merge. Required checks and
   branch protections remain in place. Do not auto-merge unreviewed contributor
   changes or substitute local evidence for the controller's exact public CI
   and container requirements.
5. Release the exact public-main revision through `Public Runtime Release`.
   It verifies all reviewed service bindings, live revisions and marketplace
   reader readiness. Run the hosted smoke check, then update the production
   monitor's expected revision only after verification. Keep prior public
   releases available and account for forward-only database migrations.

The release gate is still mandatory. Local validation avoids duplicate private
Actions jobs; Render builds alone do not replace tests. The public release and
provenance workflows enforce publication and detect source drift without
reading or rewriting production secrets.

Payment workers, signing policies, review notices and production monitoring keep
their existing schedules. New contract deployments still need independent review
and a testnet rehearsal. Hosted AI generation stays disabled; never provision a
model-provider key as part of release recovery.

GitHub artifact storage, larger runners and other products can still cost money.
Keep logs only as long as needed and inspect billing before buying another host.
See [GitHub's billing rules](https://docs.github.com/en/billing/concepts/product-billing/github-actions).
