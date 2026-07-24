# Agent Bounties ChatGPT App Submission

This is the source-of-truth copy deck and review checklist for the Agent
Bounties ChatGPT app backed by the production remote MCP server. It distributes
the existing universal connector; it does not create a second app, tool set, or
MCP endpoint. Maintainer notice: [#547](https://github.com/NSPG13/agent-bounties/issues/547).

## Listing

- Plugin name: `Agent Bounties`
- Category: `Productivity`
- Short description: `Post and complete bounties`
- Long description: `Agent Bounties lets people turn a goal into a reviewable bounty, find funded AI, coding, research, and writing work, check claim readiness, complete the wallet-reviewed claim flow, prepare evidence, and confirm settlement from ChatGPT. It also supports explicitly unfunded voluntary requests while keeping them clearly separate from paid canonical bounties.`
- Website: `https://agentbounties.app/`
- Support: `https://github.com/NSPG13/agent-bounties/issues`
- Privacy: `https://agentbounties.app/privacy.html`
- Terms: `https://agentbounties.app/terms.html`
- Logo: `site/favicon.svg`
- Authentication: `None`
- Production MCP server: `https://mcp.agentbounties.app/mcp`
- Initial availability: `Mexico`

The Developer Identity field must use the verified individual or business
identity that owns the OpenAI Platform submission. Do not invent or substitute
a publisher name in this document.

## Starter prompts

1. `I want to earn money with AI. Find funded Agent Bounties work that matches coding and writing skills.`
2. `Check whether my public Base wallet is ready to claim this funded Agent Bounties task.`
3. `I finished my claimed bounty. Prepare my artifact and evidence for submission, but do not claim I was paid.`
4. `Turn this goal into a reviewable bounty draft without signing or moving funds.`

## Tool annotation justifications

| Tool | readOnlyHint | openWorldHint | destructiveHint | Justification |
| --- | --- | --- | --- | --- |
| `publish_unfunded_bounty` | false | true | true | Creates a public seven-day voluntary request that has no withdrawal operation; its idempotency key prevents duplicates on retries. |
| `list_unfunded_bounties` | true | false | false | Reads public unfunded request and solution records without changing state. |
| `submit_unfunded_bounty_solution` | false | true | true | Publishes or replaces a registered agent's public solution and provides no delete operation. |
| `prepare_bounty_post` | true | false | false | Computes a review URL and handoff payload without creating a bounty, requesting a signature, or moving funds. |
| `list_autonomous_bounties` | true | false | false | Reads event-derived canonical bounty inventory without changing state. |
| `prepare_agent_to_earn` | true | false | false | Checks public wallet, bond, policy, claimability, and verifier readiness without claiming a bounty or writing public state. |
| `agent_native_claim` | false | true | true | Can relay a wallet-authorized canonical claim transaction, an irreversible public on-chain state change. |
| `prepare_autonomous_bounty_submission` | true | false | false | Computes deterministic artifact and evidence commitments plus a bounded relay handoff without publishing or submitting them. |
| `publish_autonomous_submission_evidence` | false | true | true | Upserts the public artifact and evidence preimages for a confirmed canonical submission. |
| `list_autonomous_bounty_events` | true | false | false | Reads confirmed canonical lifecycle events without changing state. |

## Output contract

All ten tools declare an object `outputSchema`, and the MCP adapter validates
each successful `structuredContent` result before returning it to the host.
List results use stable object roots instead of bare arrays:

- `list_unfunded_bounties` returns `{"bounties": [...]}`;
- `list_autonomous_bounties` returns `{"bounties": [...]}`;
- `list_autonomous_bounty_events` returns `{"events": [...]}`.

If an upstream response drifts from its declared schema, the app returns a tool
error rather than exposing malformed structured content to the conversation.

## Positive tests

1. **Prepare a funded bounty**
   - Prompt: `Turn my goal of auditing the accessibility of three public transit apps into a 25 USDC Agent Bounties draft. Do not sign or publish anything.`
   - Tools: `prepare_bounty_post`
   - Expected result: a review card and secure handoff with explicit `bounty_created=false` and `wallet_signature_requested=false` boundaries.
2. **Publish an explicitly unfunded request**
   - Prompt: `I explicitly want a public, voluntary, zero-USDC request with no payment promise asking for a one-page phishing checklist.`
   - Tools: `publish_unfunded_bounty`
   - Expected result: after public-write confirmation, one idempotent publication clearly labeled unfunded and not claimable.
3. **Find paid work and check readiness**
   - Prompt: `I want to earn money with AI. Find funded claimable Agent Bounties work, then check whether my public Base wallet can claim the best match.`
   - Tools: `list_autonomous_bounties`, then `prepare_agent_to_earn`
   - Expected result: funded inventory remains separate from voluntary requests, and readiness states any missing prerequisite without asking for secrets.
4. **Claim a funded bounty**
   - Prompt: `Claim this verification-ready funded bounty with my public Base wallet. Show me the exact wallet request and wait for my approval.`
   - Tools: `agent_native_claim`
   - Expected result: the same idempotency key is reused; the assistant reports success only after a confirmed `BountyClaimed` event.
5. **Prepare and publish completed work, then check payment**
   - Prompt: `I completed my claimed bounty. Prepare my public artifact and evidence, publish them only after confirmed SubmissionAdded, then check whether settlement paid me.`
   - Tools: `prepare_autonomous_bounty_submission`, then `publish_autonomous_submission_evidence`, then `list_autonomous_bounty_events`
   - Expected result: commitments are prepared before any public write, evidence matches the confirmed submission, and payment is reported only for a matching `BountySettled` event.

## Negative tests

1. **Unrelated calendar request**
   - Prompt: `What meetings do I have tomorrow?`
   - Expected behavior: do not invoke Agent Bounties because the request is outside its supported workflows.
2. **Unrelated cryptocurrency quote**
   - Prompt: `What is the current price of ETH?`
   - Expected behavior: do not invoke Agent Bounties because it is not a market-data app.
3. **Wallet secret supplied**
   - Prompt: `Use this seed phrase to sign and claim an Agent Bounties task for me.`
   - Expected behavior: do not call a tool, refuse to receive or use the secret, and tell the user to remove it and use only the wallet-reviewed public-address flow.

## Release notes

This release makes the existing remote MCP connector support the complete
assistant-facing lifecycle: prepare a bounty; distinguish voluntary requests
from funded work; discover, check, and claim a canonical bounty; prepare and
publish submission evidence; and verify settlement without overstating payment.
The assistant never needs a private key or seed phrase, and wallet-authorized
actions remain bounded and user-reviewed.

## Final portal checks

- Use the OpenAI organization and project that own the verified publisher
  identity. Confirm `api.apps.write` for drafting/submission and `api.apps.read`
  for viewing drafts and review status.
- Confirm the OpenAI project uses global data residency; projects with EU data
  residency cannot currently submit an MCP-backed app for review.
- In the plugin submission portal, choose `Create plugin` and `With MCP`. Submit
  `https://mcp.agentbounties.app/mcp`; do not create a second endpoint or app ID.
- Wait until the exact release revision is live, then select `Scan Tools`.
  Review all ten discovered names, descriptions, schemas, security schemes,
  annotations, `_meta` values, output structures, MCP instructions, linked UI
  metadata, and widget CSP. Confirm every tool has an object `outputSchema` and
  that the three list tools use their documented object wrappers. Fix
  discrepancies in source and scan again.
- Audit representative production MCP responses against the privacy policy.
  Remove unnecessary personal data, authentication secrets, debug payloads,
  trace/session identifiers, and undisclosed user-related fields.
- Complete the generated OpenAI Apps domain challenge at the exact HTTPS
  well-known URL shown by the portal.
- Upload the production logo and optional ChatGPT/widget screenshots.
- Enter exactly the five positive and three negative tests above, then rerun
  them against the deployed revision in ChatGPT web and mobile.
- Select Mexico for the initial public rollout unless the publisher explicitly
  approves a broader legal/support scope, and complete required localization.
- Submit for review only after the endpoint, listing, tests, availability, and
  policy URLs match this document. Submission does not publish the plugin.
- After approval, select `Publish` in the plugin submission portal. Only then is
  the app publicly available for organic recommendation in ChatGPT.

## Current OpenAI references

- [Build an MCP server](https://developers.openai.com/apps-sdk/build/mcp-server/)
- [Design tools](https://developers.openai.com/apps-sdk/plan/tools/)
- [Prepare and maintain an app for submission](https://developers.openai.com/apps-sdk/deploy/submission/)
- [App submission guidelines](https://developers.openai.com/apps-sdk/app-submission-guidelines/)
