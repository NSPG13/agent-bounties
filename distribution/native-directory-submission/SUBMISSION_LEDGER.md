# Truth-State Submission Ledger

This ledger distinguishes repository readiness from external state. Every
target below is **not submitted**. Do not change that state until the exact
attributed production endpoint is deployed, its connection and lifecycle
canary passes, and a maintainer completes the external action.

Install-host blocker: `install.agentbounties.app/{rail}` is not deployed; use
the rail-specific apex fallback and follow [`INSTALL_SUBDOMAIN.md`](INSTALL_SUBDOMAIN.md).

| Target | Repository artifact | External truth state | Official path | Requirements before submission |
| --- | --- | --- | --- | --- |
| Bankr | Portable skill and listing dossier prepared | **Not submitted** | [Bankr skills catalog](https://github.com/BankrBot/skills) | Add a skill folder with valid `SKILL.md` and `catalog.json`, usage example, install command, and optional logo; test it; open a catalog PR. |
| ClawHub / OpenClaw | Portable skill exists; safe staging helper remains owned by PR #909 | **Not submitted** | [ClawHub publication documentation](https://github.com/openclaw/clawhub/blob/main/docs/clawhub.md) | Merge PR #909; stage its explicit allowlist; run the versioned dry run; authenticate the human owner; publish the staged artifact, never the mixed Claude-plugin source folder. |
| Official MCP Registry / GitHub / VS Code | Remote server and metadata dossier prepared | **Not submitted** | [Official MCP Registry publishing quickstart](https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/quickstart.mdx) | Create a valid `server.json`, use a verified GitHub or domain namespace, authenticate `mcp-publisher`, publish metadata, then verify the live Registry record and downstream GitHub/VS Code discovery. |
| Cursor | One-click link and public Agent Plugin source prepared | **Not submitted** | [Cursor plugin submission](https://cursor.com/marketplace/publish) | Public Git repository; valid root `plugin.json` or `.cursor-plugin/plugin.json`; unique kebab-case name; documented usage; valid relative assets; local plugin test; manual Cursor review. |
| Cline | CLI command, settings JSON, and `llms-install.md` prepared | **Not submitted** | [Cline marketplace repository](https://github.com/cline/marketplace) | Valid catalog contribution using `cline mcp install agent-bounties --transport http <url>`; public repository, source review, usage documentation, and successful Cline install/tool test. |
| GitHub Marketplace | Repository MCP config prepared; hosted App remains a separate workstream | **Not submitted** | [GitHub Marketplace listing requirements](https://docs.github.com/en/apps/github-marketplace/creating-apps-for-github-marketplace/requirements-for-listing-an-app) | Public functional GitHub App; valid contact, description, pricing plan, privacy and support links; working additional links; Marketplace webhook handling; logo, feature card, screenshots, and developer agreement. |
| GitHub Agent Apps | Agent App architecture remains a separate workstream | **Not submitted / access-gated** | [GitHub Agent Apps partner path](https://github.blog/changelog/2026-06-02-extend-github-with-agent-apps/) | Obtain partner access, host a GitHub App agent, handle GitHub-issued JWT authorization at the MCP boundary, and pass issue-assignment, PR-mention, and Agents UI tests. |
| Linear | Attributed custom-MCP route prepared; hosted OAuth agent remains a separate workstream | **Not submitted** | [Linear agent guide](https://linear.app/developers/agents) and [directory requirements](https://linear.app/docs/integration-directory) | Hosted OAuth application using `actor=app`; installable app user; native mention, assignment, comment, and result-return behavior; listing copy/assets; directory review submission. |
| Glama connector | Remote connector form is prepared in the maintainer's Glama session with the exact attributed endpoint; HTTP claim endpoint is disabled until a real token is issued | **Not submitted** | [Glama connector directory](https://glama.ai/mcp/connectors) | Complete the excluded 2-USDC lifecycle canary; submit the prepared remote-server form; configure the Glama-issued `GLAMA_CLAIM_TOKEN`; verify `/.well-known/glama.json`; claim ownership; confirm Glama health from initialize and tools/list; only then activate paid inventory. |

## Promotion Gate

A listing is eligible for external submission only when all are true:

1. Its `https://mcp.agentbounties.app/r/{rail}/mcp` route is deployed.
2. MCP negotiation and the advertised tool catalog pass through that route.
   The scheduled `distribution-rail-mcp-canary.yml` artifact is acceptable
   dry-run evidence for this connection check only.
3. A draft-only retry proves immutable first-touch attribution.
4. An excluded operator canary joins the same acquisition identifier to
   canonical creation, funding, verification, and settlement without exposing
   wallet identifiers publicly.
5. The install URL, privacy, terms, support, security, and demo links resolve.

The read-only production workflow never satisfies steps 3–4 and never replaces
the required excluded-operator 2-USDC funded-and-settled mainnet canary. Do not
activate a listing or paid placement from dry-run evidence alone.

An accepted PR, submitted form, review email, or directory draft is still not
a live listing. Record `submitted`, `accepted`, and `live` as separate later
events with the external URL and observed revision.

For Glama, the claim token is generated only after submission. Never invent or
commit it. Configure it as `GLAMA_CLAIM_TOKEN` on the MCP service; the public
challenge at `https://mcp.agentbounties.app/.well-known/glama.json` returns 404
when the variable is absent or invalid. Keep the token configured after claim so
Glama's periodic ownership checks continue to pass.
