# Wallet on-ramps

The hosted wallet top-up page is [Add money to your wallet](https://agentizable.ai/onramp).
It helps the user add USDC on Base to their own wallet. A provider purchase is
separate from funding a bounty; the user must review the exact bounty funding
request afterward.

The current MCP server exposes `POST /v1/onramps/moonpay/checkout` for a signed
MoonPay checkout link. Its configuration, origin checks, client-IP handling and
limits are documented in [MoonPay onramp](moonpay-onramp.md). The website also
links to the user's wallet provider. Availability and purchase requirements are
shown by the selected provider.

The previously proposed Stripe crypto-onramp and MoonPay sell handlers are not
mounted in this runtime. Their old environment settings do not enable hosted
routes. Do not offer them as available API operations. Restoring either feature
requires a separate implementation, authorization review and provider rehearsal.

## Evidence boundaries

| Observation | What it proves |
| --- | --- |
| Onramp session or checkout link | A purchase was prepared; no transfer is established |
| Provider completion and observed wallet balance | The wallet has funds; the bounty remains unfunded |
| Signed authorization or relay transaction hash | Intent or a broadcast, pending canonical confirmation |
| Canonical `FundingAdded` | Funding recorded by the bounty contract |
| Canonical `BountySettled` | Solver payment recorded by the bounty contract |

The current v1 deployment is recovery-only. New funding must use an enabled,
qualified successor; topping up a wallet does not lift a protocol hold. See
[protocol status](https://agentizable.ai/protocol.json) and
[the autonomous protocol](autonomous-protocol.md).
