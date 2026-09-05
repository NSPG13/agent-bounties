# Vendor Procurement Runbook

This runbook reserves native, install-adjacent inventory while preserving the
measurement and wallet boundaries. The machine-readable order packets are in
`vendor-orders-v1.json`; their checked-in state is deliberately `not_contacted`.
Never mark a reservation, invoice, payment, deployment, canary, or settlement
complete without an inspectable vendor or canonical-chain reference.

## Send every vendor this request

Subject: attributed Agent Bounties placement

> Agent Bounties lets an agent or agentic engineer delegate backlog as a funded
> USDC bounty and receive a verified solution with settlement proof. We want to
> reserve native inventory next to MCP discovery, connection, or installation —
> not impression-only brand advertising.
>
> Please confirm the exact surface, position, category, rotation/share of voice,
> start behavior, duration, renewal/cancellation terms, and source reporting. The
> placement must link to the assigned Agent Bounties install destination and the
> connection must use its assigned `/r/<rail>/mcp` endpoint. We do not share
> wallet keys, payment authority, prompt contents, or user identities. We will
> activate only after a route-specific attribution and 2 USDC settlement canary
> passes. Can you hold this inventory pending that canary and provide a written
> quote?

Until the install subdomain's DNS and edge binding pass their deployment
checks, the order packet deliberately uses the live apex
`https://agentbounties.app/install/<rail>/` destination. The preferred
`install.agentbounties.app/<rail>` alias is recorded separately and must never
replace a working campaign URL before it resolves.

Add the vendor-specific proposed inventory and maximum initial spend from the
order packet. A reply is a reservation only when it identifies inventory and an
expiry or hold condition. A sales acknowledgement without those terms is not a
reservation.

## Evidence states

Advance each order independently:

1. `not_contacted` — no outbound message has been sent.
2. `contacted` — attach the sent-message reference and recipient.
3. `quoted` — attach the written inventory, price, currency, and terms.
4. `reserved` — attach the vendor's hold confirmation and any expiry.
5. `canary_ready` — attach three joined dry runs and one canonical 2 USDC
   settlement, all excluded from external metrics.
6. `approved` — attach the owner, approved amount, currency, and gate output.
7. `purchased` — attach the invoice and payment receipt; an order confirmation
   alone does not prove the placement is live.
8. `live` — attach the public placement and an observed request at the attributed
   endpoint.

Never infer one state from another. In particular, do not infer `purchased` from
`approved`, `live` from an invoice, or a conversion from a click or MCP request.

## Approval packet

Before asking an owner to pay, attach:

- the current vendor row from `vendor-orders-v1.json`;
- the written quote and inventory definition;
- the `activate_initial_placement` decision emitted by
  `scripts/distribution_gate.py` for that rail;
- route health plus dry-run and canonical settlement evidence;
- the cancellation owner and critical-incident pause owner;
- confirmation that bounty principal is accounted for separately.

MCP.so publicly lists its Gold Sponsor tier at 699 USD per month for listing and
detail pages; this is separate from its 39 USD one-time paid submission.
MCPServers.org publicly lists exclusive category sponsorship from 600 USD per
month and currently identifies Productivity and Finance at that price. Confirm
the selected category, availability, rotation, target URL, and final checkout or
quote in writing. Every planned amount remains a hard ceiling, not authority to
buy undefined inventory.

## After activation

Evaluate each rail whenever a lifecycle event changes its observation. Do not
wait for a calendar review. Pause immediately on a critical payment, fraud, or
security incident. Scale only from a `scale_next_tranche` result; cancel or do
not renew from a `do_not_renew` result. Impressions, clicks, installs, drafts,
signatures, transaction hashes, and PRs remain diagnostics, never conversions.
