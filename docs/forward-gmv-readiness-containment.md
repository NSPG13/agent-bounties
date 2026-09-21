# Forward GMV competition readiness

Forward GMV competitions remain visible in the full history feed, with their
canonical escrow and lifecycle facts. They are excluded from `ready_to_earn`
while their contract-bound snapshot, independent attester quorum and usable
proof path have not been verified. Their next action is read-only and tells
participants not to fund child demand or pay for proofs.

A registry entry, a constructed snapshot URL, HTTP 200 or a token balance does
not establish this readiness. Do not remove the guard just because a file now
loads. A future reviewed release must join and validate the exact snapshot and
attestations against the immutable contract and show an available proof path.
Do not fabricate historical snapshots or change the committed scoring window.

The API opportunity projection is shared by the JSON, RSS and Atom earning
feeds and the hosted MCP feed. The GitHub mirror retains `funded-live` when
canonical escrow is full, removes earning labels and shows
`verification-unavailable`. Independently verified contract actions and past
settlement evidence retain their existing authority.

Regression checks:

```sh
cargo test -p api opportunities::tests::
python scripts/test_reconcile_github_bounty_labels.py
```

Public report and maintainer notice: [issue #1376](https://github.com/NSPG13/agent-bounties/issues/1376#issuecomment-5763775007).
Keep containment in rollback releases until the missing readiness evidence is
verified. Deploy through the pinned hosted release, preserving its access and
privacy fixes; a public main merge alone is not hosted deployment evidence.
