# Native Directory Submission Package

This directory is the reusable, source-controlled dossier for Agent Bounties
skill, MCP, plugin, and agent-directory submissions. It prepares truthful
materials; it does not submit a listing, spend money, approve a platform
policy exception, or prove that a listing is live.

Public maintainer notice: [#1274](https://github.com/NSPG13/agent-bounties/issues/1274).

## Reusable Listing

- **Name:** Agent Bounties
- **Tagline:** Delegate work. Fund exact terms. Receive a verified solution.
- **Category:** Productivity
- **Install hub:** <https://agentbounties.app/install/>
- **Repository:** <https://github.com/NSPG13/agent-bounties>
- **Privacy:** <https://agentbounties.app/privacy.html>
- **Terms:** <https://agentbounties.app/terms.html>
- **Support:** <https://github.com/NSPG13/agent-bounties/issues>
- **Security:** <https://github.com/NSPG13/agent-bounties/blob/main/SECURITY.md>
- **Logo:** <https://agentbounties.app/favicon.svg>

Use the description, keywords, exact platform status, install URL, and
attributed endpoint in [`manifest.json`](manifest.json). The authoritative
external-state record and official requirements are in
[`SUBMISSION_LEDGER.md`](SUBMISSION_LEDGER.md). Never replace an
attributed endpoint with the canonical untagged endpoint in a measured listing.

The requested `install.agentbounties.app/{rail}` host is not deployed. Its hard
DNS/edge blocker and activation contract are recorded in
[`INSTALL_SUBDOMAIN.md`](INSTALL_SUBDOMAIN.md); use the rail-specific apex
routes in the manifest until that contract passes.

## Submission Order

Prepare all submissions in parallel, then publish each as soon as its endpoint
is deployed and passes the tests in [`TESTING.md`](TESTING.md):

1. Bankr catalog pull request
2. ClawHub publication
3. MCP Registry / VS Code
4. Cursor marketplace
5. Cline marketplace
6. GitHub Marketplace and Agent Apps review
7. Linear directory

Claude custom connectors and ChatGPT developer mode are usable testing paths.
Their public directory submissions stay blocked until written policy clearance
for the exact bounty-and-crypto workflow is recorded.

## Review Checklist

- [ ] Use the exact platform install URL and attributed MCP URL from the submission manifest.
- [ ] Verify the route returns the canonical MCP catalog without redirecting to an untagged rail.
- [ ] Paste no API key, wallet secret, seed phrase, signing key, or publishing token into the package.
- [ ] Link [`SECURITY.md`](SECURITY.md), [`TESTING.md`](TESTING.md), and [`DEMO.md`](DEMO.md).
- [ ] Confirm the target remains `not_submitted` in [`SUBMISSION_LEDGER.md`](SUBMISSION_LEDGER.md) until the external action actually occurs.
- [ ] Describe wallet review as external to the agent host.
- [ ] Describe a draft, intent, signature, transaction hash, PR, and issue status as non-settlement evidence.
- [ ] Describe payment only after a confirmed canonical `BountySettled` event.
- [ ] Mark the listing `submitted` only after the external submission is accepted by that platform.
- [ ] Capture the listing URL and revision in the resulting pull request or operator record.

## ClawHub Ownership Boundary

Pull request [#909](https://github.com/NSPG13/agent-bounties/pull/909)
owns the explicit-allowlist ClawHub staging helper
`scripts/prepare-clawhub-skill.mjs`. Do not copy or reimplement that helper in
this package. After #909 merges, use its documented staging and dry-run flow,
then require separate human-authenticated publication.
