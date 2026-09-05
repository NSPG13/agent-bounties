# Install Subdomain Deployment Contract

## Current Truth State

`https://install.agentbounties.app/{rail}` is **not deployed**. The current
GitHub Pages static site owns the apex `agentbounties.app` CNAME and provides
the deployable source routes at
`https://agentbounties.app/install/{rail}/`. A second custom hostname cannot be
claimed from this repository until DNS and an edge/static-host binding are
configured outside the Pages CNAME.

This is a hard deployment blocker, not an application-code TODO. Do not publish
the install-subdomain URLs in listings until all acceptance checks below pass.

## Required Edge Behavior

- Bind TLS and DNS for `install.agentbounties.app` to an approved edge/static
  deployment without changing `site/CNAME`.
- Serve or reverse-proxy `/{rail}` and `/{rail}/` from the matching
  `/install/{rail}/` source page. A permanent redirect is acceptable only if it
  preserves the full query string.
- Permit only `GET`, `HEAD`, and `OPTIONS` on the install host.
- Preserve CSP, analytics, privacy, and canonical evidence language from the
  source page; inject no wallet or publishing credentials.
- Return 404 for unknown rails instead of falling back to an untagged install.

Supported rails are `bankr`, `openclaw`, `vscode`, `cursor`, `cline`, `github`,
`linear`, `claude-custom`, `chatgpt-dev`, `glama`, `mcp-so`, and `mcpservers`.
The page route selects content only. Its copy/install action must retain the
exact `https://mcp.agentbounties.app/r/{rail}/mcp` endpoint.

## Activation Acceptance

1. HTTPS certificate and DNS resolve from an external network.
2. Every supported alias returns 200 or one query-preserving redirect to its
   matching apex source page.
3. Every unknown rail returns 404.
4. Each page exposes the exact attributed MCP URL and never substitutes
   `https://mcp.agentbounties.app/mcp`.
5. The attributed endpoint passes MCP negotiation plus the excluded-operator
   lifecycle canary.

Until then, vendor orders use the rail-specific deployable apex campaign URLs
recorded in `manifest.json`.
