# Feed snapshots

Production RSS, Atom, and JSON Feed documents are rendered live by the API from
the unified `/v1/opportunities` projection. No committed file in this directory
is current bounty inventory.

With a local API running, validate all three formats and their cross-format item
identity with:

```bash
bash tools/feed_proof.sh
```

Set `API_BASE_URL=https://api.bountyboard.global` to validate the hosted API.
The proof snapshot is written under the ignored `feeds/proof/` directory and
removed when the command exits.
