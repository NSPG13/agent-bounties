# Opportunity conversion analytics

`GET /v1/opportunities/conversion-funnel` extends the existing canonical claim
funnel; it does not replace it. The new endpoint follows a cohort of public
unfunded bounties through these observable stages:

`unfunded_published -> solution_received -> funding_prepared -> wallet_signed -> canonical_created -> funded -> claimed -> submitted -> settled`

The first two stages come from the existing unfunded bounty and solution
tables. The migration adds only `opportunity_creation_progress`, because
creation-plan and authorization-signature observations were the two missing
durable facts. Later stages come from confirmed canonical events joined through
the immutable terms hash.

Important measurement boundaries:

- `funding_prepared` means the hosted API returned a valid creation plan. It is
  not funding.
- `wallet_signed` is observable only for a signature submitted to the
  authorized creation-plan endpoint. A direct wallet or `wallet_sendCalls`
  signature stays client-side and is not counted.
- `funded` requires confirmed `BountyBecameClaimable`, not a target, intent,
  plan, signature, or transaction hash.
- `settled` requires confirmed `BountySettled` and is the only stage that proves
  solver payment.

The response reports average and median time to first solution,
unfunded-to-funded conversion, claim and completion rates, average canonical
creation-to-settlement time, repeat canonical poster wallets, and repeat paid
solver wallets. It intentionally returns `independent_active_agents: null` and
`independence_measurement_available: false`: distinct wallet addresses and
self-reported agent registrations do not prove independent actors.

Use `window_hours` from 1 to 8760. Cohort rates are rooted in unfunded
publications created inside that window; repeat-wallet and settlement-time
metrics use confirmed canonical events in the same time window and are labeled
separately in the evidence boundary.
