# Event-Driven Distribution Activation

This directory turns paid agent-distribution decisions into a deterministic,
event-driven gate. It does not buy media, authorize a wallet, run a mainnet
canary, infer beneficial ownership, or create payment evidence.

The policy freezes three initial placements:

| Vendor | Attributed rail | Initial spend | Minimum scale sample |
| --- | --- | ---: | ---: |
| Glama | `glama` | 23,779.28 MXN | 12 external funded poster wallets and 6 verified settlements |
| MCP.so | `mcp-so` | 11,872.65 MXN | 6 posters and 3 settlements |
| MCPServers.org | `mcpservers` | 10,191.12 MXN | 6 posters and 3 settlements |

The 45,843.05 MXN total uses the frozen 16.9852 MXN/USD planning rate. Confirm
the final invoice, inventory, category, and currency conversion before an owner
authorizes purchase. Bounty principal and canary USDC are not distribution
spend. The 2 November 2026 spending target is a backstop, never a review cadence
or permission to bypass route, evidence, safety, or owner-approval gates.

## Required order

1. Publish the vendor's exact `/r/<rail>/mcp` endpoint.
2. Complete at least three dry runs and one canonically settled 2 USDC mainnet
   canary for that route. Register every canary as excluded test activity.
3. Attach inspectable evidence references to the operator observation.
4. Complete all exclusion classes without asserting that an unlisted wallet is
   an independent person.
5. Run the gate. Purchase is still an explicit owner action outside this tool.

```bash
python scripts/distribution_gate.py validate-policy \
  --policy ops/distribution/activation-policy-v1.json

python scripts/distribution_gate.py validate-orders \
  --policy ops/distribution/activation-policy-v1.json \
  --orders ops/distribution/vendor-orders-v1.json

python scripts/distribution_gate.py evaluate \
  --policy ops/distribution/activation-policy-v1.json \
  --observation ops/distribution/activation-observation-template.json
```

The checked-in observation is intentionally incomplete and must fail closed.
Make an operator-owned copy outside the repository or add a separately reviewed,
redacted evidence artifact. Do not overwrite the template with mutable live
state.

The human-controlled 2 USDC lifecycle procedure is defined in
[`mainnet-canary-runbook.md`](mainnet-canary-runbook.md). It excludes canary
wallets before funding and requires canonical creation, claimability, verifier
evidence, and settlement rather than a transaction hash.

Join that reviewed control file to the live, operator-authenticated canonical
report without placing the operator token on the command line:

```bash
OPERATOR_API_TOKEN=... python scripts/distribution_dashboard.py \
  --api-base https://api.agentbounties.app \
  --control /path/to/operator-reviewed-distribution-control.json \
  --output target/distribution-dashboard.json
```

The dashboard reports the cumulative event-driven funnel, GMV, poster and
settlement CAC, coverage, failures, and an independent activation decision for
each paid rail. Wallet review comes from the durable first-party review-boundary
acknowledgement. It refuses to infer verified usefulness without reviewed
origin-completion evidence, and keeps LTV unavailable until platform revenue
exists.

## Decisions

- `blocked_exclusion_review`: required external-wallet exclusions are incomplete.
- `blocked_canary`: the route lacks complete dry-run or canonical settlement evidence.
- `activate_initial_placement`: route evidence is ready and initial spend may be presented for owner approval.
- `hold_attribution_coverage`: outcomes exist but less than 95% retain attributable acquisition evidence.
- `hold_no_incremental_spend`: the current test has not reached its event sample; spend does not increase.
- `scale_next_tranche`: both CAC caps, quality evidence, attribution, and safety gates pass; the next proposed tranche is twice observed spend.
- `do_not_renew`: the minimum event sample is reached but efficiency or usefulness fails.
- `halt_critical_incident`: any open critical payment, fraud, or security incident blocks spend.

First-touch attribution controls CAC. Assisted rails remain diagnostic and never
receive duplicate conversion credit. Only `BountyBecameClaimable` establishes
the funded outcome and only confirmed `BountySettled` plus valid verifier
evidence qualifies a useful settlement.

## Vendor contract requirements

Before an owner buys a placement, record:

- exact product, category, placement inventory, and renewal behavior;
- the attributed MCP endpoint and install destination;
- vendor reporting available without collecting wallet or prompt data;
- a fixed initial price and explicit cancellation path;
- confirmation that impressions, clicks, and installs are not billable outcome
  evidence;
- the owner who can approve the purchase and the incident owner who can pause it.

No script in this repository stores vendor credentials, submits an order,
contacts a vendor, or moves funds.

`vendor-orders-v1.json` and `vendor-procurement.md` turn the three planned
placements into independently gated procurement packets. Their state is
evidence-backed and must never be advanced speculatively.
