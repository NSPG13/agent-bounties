# Agent Bounties ChatGPT app release dossier

This document is the source of truth for the Agent Bounties ChatGPT app,
including its hosted-execution architecture, test contract, and current public
Plugin Directory status.

## One product profile

Agent Bounties has one production product profile and exactly two durable
ChatGPT registrations. `Agent Bounties` is the public anonymous registration;
`Agent Bounties Operator QA` is the private OAuth-linked maintainer
registration whose requests are excluded from public adoption metrics. Both
use `https://mcp.agentbounties.app/mcp` and expose the same live feed and the
same hosted post, MoonPay top-up, fund, solve, complete, verify, comment,
breakdown, and share capabilities. OAuth changes analytics attribution only;
it grants no wallet, payment, publishing, or operator authority.

| Profile | Setting | Purpose |
| --- | --- | --- |
| Full hosted execution | `CHATGPT_APP_SANDBOX_MODE` absent or false | Complete product; every signing, purchase, KYC, card-entry, and transaction-authorization step occurs outside ChatGPT |
| Sandbox | `CHATGPT_APP_SANDBOX_MODE=true` | Deterministic fixture-only proof of the complete UI and return flow; no network write, provider checkout, wallet action, social post, or payment |

There is no reduced `CHATGPT_APP_PUBLIC_REVIEW_MODE`. A deployment that still
sets that obsolete variable receives the same full production behavior. The
fixture-only sandbox remains separate because it is a test instrument, not a
second product.

## Directory policy status: blocked

The full product is technically deployable and developer-installable, but it is
not currently eligible for public Plugin Directory submission under the
published guidelines checked on 2026-07-28.

The current guidelines:

- limit plugin commerce to physical goods;
- disallow selling digital products or services, including tokens and credits;
- prohibit execution of money transfers, crypto transfers, or investment
  trades; and
- require external checkout for commerce that is otherwise allowed.

External hosted execution is therefore the correct security architecture but
is not a policy exemption. Do not submit or describe Agent Bounties as
Directory-eligible without either:

1. written OpenAI approval for this exact bounty-and-crypto workflow, or
2. a published policy change that allows it.

There is no administrator or independent-review bypass. The release artifact
keeps this blocker explicit rather than hiding product capability to obtain an
approval that would not cover the real app.

## Listing materials

- App name: `Agent Bounties`
- Category: `Productivity`
- Subtitle: `Fund and complete bounties`
- Website: `https://agentbounties.app/`
- Production MCP endpoint: `https://mcp.agentbounties.app/mcp`
- Support: `https://github.com/NSPG13/agent-bounties/issues`
- Privacy: `https://agentbounties.app/privacy.html`
- Terms: `https://agentbounties.app/terms.html`
- Logo: `site/favicon.svg`
- Authentication: `None` at the MCP layer; wallet/provider authorization is
  hosted outside ChatGPT
- Import artifact: `chatgpt-app-submission.json`

The JSON import artifact is technically complete but carries
`release_status.directory_submission=blocked_pending_written_openai_approval_or_policy_change`.
It must not be treated as permission to submit.

## Full tool surface

New ChatGPT scans advertise exactly these ten non-overlapping tools:

| Tool | Effect | Required annotations |
| --- | --- | --- |
| `get_bounty_feed` | Reads the unified live bounty projection | read-only, closed-world, idempotent |
| `render_bounty_feed` | Mounts the branded conversation-first live feed | read-only, closed-world, idempotent |
| `prepare_moonpay_onramp` | Formats one first-party Base-USDC top-up handoff; creates no checkout or purchase | read-only, closed-world, idempotent |
| `prepare_bounty_post` | Prepares the exact approved terms and, when supplied, stores the matching user-approved image for the first-party review handoff | non-read-only, open-world, destructive, idempotent |
| `prepare_bounty_action` | Creates one opaque, expiring, idempotent first-party lifecycle-review intent | non-read-only, closed-world, non-destructive, idempotent |
| `get_bounty_action_status` | Reconciles one intent against indexed canonical events | non-read-only, closed-world, non-destructive, idempotent |
| `compile_objective_with_cloud_agent` | Produces bounded child-bounty drafts | non-read-only, open-world, non-destructive, non-idempotent |
| `list_bounty_comments` | Reads public comments | read-only, closed-world, idempotent |
| `add_bounty_comment` | Publishes one explicit bounded comment | non-read-only, destructive, open-world, non-idempotent |
| `create_share_bundle` | Formats a caption and safe share intents | read-only, closed-world, idempotent |

Every tool declares no MCP authentication, a bounded input schema, an output
schema, accurate annotations, and model-and-app visibility. Only
`render_bounty_feed` links the mounted
`ui://agent-bounties/live-feed-v4.html` component.

`list_autonomous_bounties` remains callable for cached ChatGPT registrations
and remains advertised to server-to-server modern and legacy core MCP clients.
It is intentionally absent from new ChatGPT discovery. The single app
discovery entry point is `get_bounty_feed`.
The HTTP adapter also retains the compatibility path
`/tools/list_autonomous_bounties`. This boundary prevents a cached registration
from breaking while removing an overlapping choice from new ChatGPT
registrations.

`prepare_bounty_post` declares
`_meta["openai/fileParams"]=["bounty_image"]` for hosts that support approved
file inputs. ChatGPT can gather the terms, generate an image with the poster's
own account, display that exact image, and obtain explicit approval before
calling the tool. In that variation the tool downloads only the temporary
OpenAI-hosted file URL, validates a PNG/JPEG/WebP payload of at most 5 MiB,
stores it by SHA-256, and returns a first-party review URL. The private ChatGPT
`file_id` is never placed in public terms. Provider-neutral clients may omit
the image, prompt, and alt text together; partial image metadata is rejected.
Agent Bounties never generates or substitutes bounty artwork. The review URL
renders the completed approved terms and optional image; it does not publish or
move funds without the separate wallet-reviewed flow.

Lower-level funding, wallet, claim, submission, and settlement tools are not
exposed to the ChatGPT app. The model receives only hosted preparation and
canonical-status tools.

## MoonPay and wallet boundary

The app uses the production MoonPay integration already implemented in the
repository:

1. `prepare_moonpay_onramp` accepts only a canonical Base bounty contract, a
   bounded planned USDC amount, and an optional opaque funding-intent UUID.
2. The tool returns the reviewed first-party
   `https://agentbounties.app/onramp.html` handoff. That page keeps MoonPay,
   MetaMask Portfolio, and Coinbase wallet top-up variations separate from the
   later bounty-funding authorization.
3. It does not call MoonPay, create checkout, connect a wallet, or move money.
4. The page connects an injected Base wallet or accepts a manually verified
   public address, then shows the destination, planned amount, USDC balance,
   and optional gas balance.
5. The browser requests a device-bound signed MoonPay URL from
   `/v1/onramps/moonpay/checkout`.
6. MoonPay handles its purchase, payment methods, eligibility, identity checks,
   fees, credentials, and asset delivery outside ChatGPT.
7. The user returns and separately authorizes the exact bounty contribution.
8. Only a matching indexed `FundingAdded` event changes bounty funding.

A MoonPay purchase, provider redirect, transaction reference, wallet balance,
or checkout status is not bounty-funding evidence.

Agent Bounties does not attempt to invoke or impersonate MoonPay's own ChatGPT
plugin. ChatGPT may independently use a user-enabled MoonPay plugin when
appropriate, but Agent Bounties metadata must not manipulate selection of
another plugin. The first-party handoff is the reliable product-owned path and
uses the authorized MoonPay integration already in this repository.

Coinbase embedded-wallet authentication also remains hosted. Email, phone, or
OAuth account linking occurs in Coinbase's supported browser flow; no wallet
credential, seed phrase, private key, or provider session enters ChatGPT.

## Hosted lifecycle boundary

The full bounty loop follows one pattern:

1. The person selects Post bounty, Comment, Share, or Solve, or asks for the
   same action directly in conversation.
2. For posting, ChatGPT gathers every missing detail conversationally, creates
   a unique image in the poster's ChatGPT account from the completed bounty,
   and shows the exact image with the complete terms.
3. ChatGPT asks for explicit approval of both the terms and image. For other
   lifecycle actions it summarizes the complete proposed action and asks for
   explicit confirmation.
4. Only after confirmation does ChatGPT call the relevant tool and prepare a
   bounded opaque intent when the action requires one.
5. ChatGPT opens only an allowlisted first-party HTTPS review page using the
   host's external-navigation flow.
6. The person reviews the exact bounty, action, amount, evidence, and expected
   canonical event. A posting handoff is read-only: revisions happen back in
   the ChatGPT conversation, not in a duplicate website form.
7. Wallet signing, provider approval, card entry, KYC, and verifier signing
   occur outside ChatGPT.
8. ChatGPT refreshes only the opaque intent identifier.
9. The server reconciles the exact actor, contract, bounty, amount, transaction,
   event kind, and creation time.
10. Only the action-specific indexed canonical event confirms the step.
11. Only `BountySettled` proves solver payment.
12. ChatGPT offers the Share conversation after each meaningful prepared or
    confirmed lifecycle step, with wording that matches the evidence available.

Action-specific detail fields are allowlisted and recursively bounded.
Credential, card, private-key, seed, payment-authorization, wallet-signature,
and verifier-signature field names are rejected. Draft and evidence details
remain available to the first-party authorization page but are removed from MCP
responses. Intents expire after one hour and are deleted within 24 hours after
expiry.

## Widget contract

The mounted component:

- renders branded read-only bounty cards using the website's dark green, lime,
  mint, gold, text, and muted palette;
- contains no input, textarea, select, form, wallet control, payment field, or
  local composer;
- exposes exactly four visible actions: `Post bounty`, `Comment`, `Share`, and
  `Solve` (the Solve action appears only for funded, verification-ready,
  claimable canonical bounties);
- loads the projection through `tools/call`;
- accepts `ui/notifications/tool-result` updates;
- emits the standard MCP Apps `ui/message` notification for each action and
  uses `window.openai.sendFollowUpMessage` as the ChatGPT compatibility
  fallback; and
- never opens a provider, wallet, authorization page, or social destination
  directly from the mounted feed.

After the conversation has gathered details and the person has confirmed the
action, ChatGPT uses the application tools. A share bundle may then lead to the
first-party bounty-card preview, which creates the 1080 × 1350 PNG locally and
requires an explicit download click.

The resource uses `text/html;profile=mcp-app`, an exact widget domain, exact
connect/resource CSP, and legacy `redirect_domains` for vetted external
navigation. The widget does not iframe MoonPay or any wallet.

## Deployment contract

Production:

```text
MCP_BASE_URL=https://mcp.agentbounties.app
# Optional, comma-separated exact browser origins beyond the built-in
# first-party and ChatGPT allowlist:
MCP_ALLOWED_ORIGINS=https://approved-client.example
PUBLIC_BASE_URL=https://api.agentbounties.app
WEBSITE_BASE_URL=https://agentbounties.app
OPENAI_APPS_CHALLENGE_TOKEN=<portal-token>
CHATGPT_APP_SANDBOX_MODE=false
```

Sandbox:

```text
CHATGPT_APP_SANDBOX_MODE=true
MCP_BASE_URL=https://<sandbox-mcp-origin>
```

The endpoint uses MCP `2026-07-28` stateless discovery and strict per-request
transport metadata, with a separate legacy initialization lane for current
clients. See [MCP protocol compatibility](mcp-protocol-compatibility.md).
Modern requests whose standard MCP client-info name is exactly `openai-mcp`
receive the ten-tool app catalog. Exact ChatGPT browser origins provide the
same fallback. Other server-to-server clients receive the core catalog with
the compatibility alias. These self-declared signals select metadata only and
are never authorization signals.

The final registration contract is:

| Name | Description | Authorization |
| --- | --- | --- |
| `Agent Bounties` | Discover, post, fund, solve, complete, verify, comment on, and share verifiable AI bounties in ChatGPT. Wallet, payment, identity, and transaction authorization remain on first-party hosted pages. | None |
| `Agent Bounties Operator QA` | Private maintainer QA connection to production Agent Bounties. OAuth only excludes authorized maintainer tests from public interface metrics; it grants no wallet, payment, publishing, or operator authority. | OAuth |

Refresh Operator QA first for every metadata, schema, annotation, instruction,
authentication, or widget change. Confirm exact metadata, rerun the evaluation
set in a new conversation, and verify a redacted exclusion event at the exact
revision. Only then refresh the public registration, and never owner-test
through it. Reauthorize Operator QA before its 90-day bearer lifetime expires.

Any temporary registration must include its purpose, date, short revision,
owner, and `DELETE-TODAY` or an explicit expiry, and must be removed in the
same release session. Do not create durable registrations named `Current`,
`Latest`, `Final`, `Release`, or `Proven`.

The exact domain challenge is served at:

```text
/.well-known/openai-apps-challenge
```

When configured, that route returns only the exact trimmed token as
`text/plain` with `Cache-Control: no-store`. When missing or invalid, it returns
404 with an empty body.

## Review tests

`chatgpt-app-submission.json` contains exactly five positive tests:

1. Render the conversation-first live bounty feed.
2. Gather a bounty conversationally, then prepare and share it.
3. Prepare MoonPay top-up plus canonical funding review.
4. Coordinate solve, completion, and verification hosted actions.
5. Break down, comment on, and share bounty work.

It contains exactly three negative tests:

1. Wallet, verifier, card, and identity secrets pasted into ChatGPT.
2. Arbitrary crypto transfers or investment trades.
3. Unrelated calendar and email work.

Run all cases against the exact deployed revision in ChatGPT web and desktop.
Also test cancellation of link prompts, denied wallet access, MoonPay
ineligibility, checkout cancellation, wallet-balance refresh, transaction
reverts, delayed indexing, retries, expired intents, duplicate calls, and
canonical confirmations.

## Release gates

Before a developer-mode public beta:

1. Run `scripts/preflight.ps1 -Mode core`.
2. Run `cargo fmt --all -- --check`.
3. Run `cargo test -p mcp-server`, `cargo test -p api`, and
   `cargo test -p db`.
4. Run `cargo build -p mcp-server`, then
   `python scripts/check-chatgpt-app-runtime.py`.
5. Run `python scripts/check-chatgpt-app-submission.py`.
6. Run `cargo test -p mcp-server moonpay -- --nocapture` for the preserved server-side boundary.
7. Run `python scripts/check-site.py` and the widget JavaScript syntax checks.
8. Confirm a modern request with exact MCP client-info name `openai-mcp` and no
   `Origin` lists exactly the ten full-product tools, including
   `prepare_bounty_post` with `openai/fileParams=["bounty_image"]`.
9. Confirm modern and legacy server-to-server discovery retain
   `list_autonomous_bounties`, while a cached ChatGPT `tools/call` can still
   execute it and fails closed when inventory is unavailable.
10. Confirm the MoonPay tool returns a first-party handoff with
   `checkout_created=false`, `purchase_completed=false`,
   `bounty_funded=false`, and no provider checkout URL.
11. Inspect the mounted resource MIME type, widget domain, CSP, redirect
   domains, bridge calls, state persistence, external-link path, and PNG
   fallback.
12. Exercise the full sandbox loop and then the hosted production loop without
    real funds before any live canary.
13. Verify privacy, terms, support, and retention disclosures.
14. Complete the exact domain challenge.

Before a Plugin Directory submission, additionally obtain written OpenAI
approval or confirm that the published policy changed. Without that, stop after
technical validation and developer-mode distribution.

## Current OpenAI references

- [Build an MCP server](https://developers.openai.com/plugins/build/mcp-server)
- [Build a ChatGPT UI](https://developers.openai.com/plugins/build/chatgpt-ui)
- [Define tools](https://developers.openai.com/plugins/plan/tools)
- [Plugin reference](https://developers.openai.com/plugins/reference)
- [Submit plugins](https://developers.openai.com/plugins/deploy/submission)
- [Plugin guidelines](https://developers.openai.com/plugins/app-guidelines)
- [MCP server review requirements](https://developers.openai.com/plugins/deploy/app-review)
- [Submission JSON schema](https://developers.openai.com/plugins/schemas/chatgpt-app-submission.v1.json)
