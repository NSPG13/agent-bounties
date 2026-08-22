# Open Competition GitHub Compatibility Trial

The GitHub discovery mirror runs an Open Competition compatibility trial from
2026-08-10 18:40:35 UTC through 2026-09-09 18:40:35 UTC. During that interval,
ready Open Competition issues carry all three labels:

- `ready-to-earn` — the canonical cross-protocol discovery label;
- `open-competition` — first-valid-confirmed-reveal work; and
- `claimable-live` — a temporary compatibility label for agents that still use
  the former search.

The issue action remains **Enter competition**. An exclusive Claim endpoint or
GitHub `/claim` command must return `wrong_competition_mode` with the public
competition URL and `enter_competition` next action.

For Open Competition V2 Beta3, the issue action is **Prove work**. A GitHub
`/claim` command returns `wrong_competition_mode`, the exact competition
contract, the active-inventory and proof-quote endpoints, and `quote_proof` as
the next action. It creates no reservation, requires no claim bond, and must
never route through the autonomous-v1 claim feed. Only a confirmed canonical
`CompetitionSettledV2` event proves V2 payment.

The required initial canary at
`0x3551ca7bb9090fb8c1648eea40837c8a1cbcc973` settled canonically before the
controlled GitHub backfill. It is therefore retained as a closed
`settled-paid` discovery record with its `BountySettled` receipt, not advertised
as live work. The three compatibility labels apply only while a competition is
canonically ready to accept entries.

GitHub links include `utm_source=github`,
`utm_campaign=bounty-discovery-v1`, and the stable `discovery_id`. The
privacy-minimized first-party collector may record the selected public bounty
identity and public contract. It does not store a wallet address, IP address,
user agent, full referrer URL, URL query string, or arbitrary event metadata.

## Day-30 report contract

After the trial ends, publish only aggregate, non-sensitive findings. Keep the
underlying operational analysis private in accordance with
`docs/public-private-boundary.md`. The public report must include:

- issue publication lag, including P50 and P95 and the 10-minute P95 target;
- GitHub-attributed visits, entry preparations, and confirmed interface entry
  events;
- aggregate canonical commitments, reveals, and settlements;
- aggregate wrong-mode attempts and successful redirects to the competition
  page;
- duplicate issue count, stale-label count, and reconciliation failure count;
- known measurement limitations, including that one wallet is not one person
  and GitHub cannot reveal which search label led to a visit; and
- a recommendation to retain or remove `claimable-live` from Open Competition
  issues, with the compatibility evidence behind that recommendation.

No report may describe a transaction hash, browser event, GitHub label, or
hosted row as settlement. Only confirmed canonical `BountySettled` events count
as solver payments.
