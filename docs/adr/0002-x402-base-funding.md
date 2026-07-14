# ADR 0002: x402 For Base Funding Before MPP

Status: accepted

## Decision

Implement x402 v2 first for machine-readable Base USDC funding and agent
discovery. Use a protocol-specific `agent-bounty-fund` scheme that carries an
exact Circle USDC EIP-3009 authorization and maps it to autonomous-v1
`fundWithAuthorization`.

Expose:

- `/.well-known/x402.json` for agent-oriented capability discovery;
- `GET /v1/x402/base/bounties/{bounty_contract}/funding` for the `402` challenge
  and hosted relay;
- `GET /v1/x402/base/relays/{relay_id}` for durable confirmation polling;
- the x402 endpoints in `/.well-known/agent-bounties.json` and `/llms.txt`.

A valid signed retry is persisted by authorization nonce, simulated, and
broadcast by a dedicated gas-only relayer. It returns `200` with
`PAYMENT-RESPONSE` only after the exact canonical `FundingAdded` has the
configured confirmations. If confirmation exceeds the request window, it
returns `202` with a durable status URL. Neither response treats a mere
transaction hash as funding.

## Why A Custom Scheme Is Required

The standard x402 EVM `exact` scheme settles by calling USDC
`transferWithAuthorization` to `payTo`. Setting `payTo` to a bounty contract is
unsafe: an ERC-20 transfer does not call `fundWithAuthorization`, update
`fundedAmount`, attribute a refundable contribution, or emit `FundingAdded`.
The USDC would be present but unavailable to the protocol state machine.

The custom scheme keeps one economic path:

`EIP-3009 signature -> fundWithAuthorization -> FundingAdded`

It also lets any gas-paying wallet relay the contribution without receiving
custody or settlement authority.

## Security And Reliability Rules

- Bound the signed challenge to x402 v2, CAIP-2 network, native USDC, exact
  amount, canonical bounty address, configured resource URL, timeout, nonce,
  and complete requirement echo.
- Recover the EIP-712 signer before persistence or RPC work. Shape validation
  alone is insufficient because forged authorizations can exhaust service
  capacity even though contract simulation would eventually reject them.
- Enforce a hosted minimum amount plus atomic rolling-24-hour network and
  contributor quotas. Idempotent retries of one authorization do not consume a
  second quota slot.
- Limit HTTP header size and reject malformed base64, JSON, addresses, nonce,
  signature, validity window, resource, extension, or requirement data.
- Bound authorization expiry to the advertised timeout plus narrow clock skew.
- Treat server validation as structural. The canonical bounty contract performs
  the cryptographic EIP-3009 signature and nonce validation during relay.
- Reject standard `exact` funding to bounty contracts.
- Do not infer funding from a challenge, signature, plan, broadcast, receipt,
  token balance, or transaction hash.
- Let USDC EIP-3009 enforce single-use nonces on-chain and let the indexer
  reconcile only confirmed canonical events.
- Give the relayer gas only. Isolate its key from MCP and workers; serialize
  sends with a database lease; cap USDC, gas, and fee per transaction; and
  persist request fingerprints without storing signatures.
- Keep free bounty discovery free. Do not add a paid endpoint solely to obtain
  a Bazaar listing.

## Agent Discovery And Bazaar

The well-known x402 document and existing discovery manifest make the custom
funding capability directly discoverable to agents. Generic x402 facilitators
are not assumed to support this custom scheme, and the platform must not claim
that Bazaar indexed it.

A later standard `exact` resource may use Bazaar when it delivers independent
paid value, such as metered verification compute or a premium routing service,
and after a production facilitator is configured. It must never be presented
as the canonical bounty funding transfer.

## MPP Boundary

Add MPP after the x402 Base path is measured in production. MPP is valuable for
payment-method negotiation, Stripe-backed fiat credentials, recurring access,
and metered sessions. It does not replace bounty contracts, verification, or
canonical events.

The future adapter should translate a successful MPP payment into a funding
intent and then acquire native USDC for the bounty through an auditable onramp
or treasury process. Stripe webhook reconciliation and the final canonical
`FundingAdded` remain separate facts. MPP support must therefore be introduced
behind its own adapter and ledger boundary rather than inside autonomous-v1.
