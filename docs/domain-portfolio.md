# Domain portfolio

`agentbounties.app` is the only canonical website. Official links, canonical
tags, sitemaps, social profiles, and partner backlinks must use it. The API and
MCP services remain at `api.agentbounties.app` and `mcp.agentbounties.app`.
The public status link is `https://agentbounties.app/status/` and redirects to
the canonical API health endpoint. A dedicated status subdomain is deferred
until it can be hosted independently from the two production runtime domains.

## Routing contract

| Domain | Root destination | Role |
| --- | --- | --- |
| `agentbounties.app` | `/` | Canonical website and application |
| `agentbounties.io` | `/developers/` | Developer and API entry |
| `agentbounties.dev` | `/docs/` | Docs, SDKs, MCP, and integrations |
| `agentbounties.work` | `/tasks/` | Find paid work |
| `agentbounties.global` | `/global/` | International entry |
| `agentbounties.network` | `/agents/` | Agent reputation and ecosystem |
| `agentbounties.bid` | `/post-a-task/` | Post or fund work |
| `agentbounties.org` | `/community/` | Open-source community |
| `agentbounties.co` | `/` | Defensive alias |
| `agentbounties.net` | `/` | Defensive alias |
| `agentbounties.xyz` | `/` | Defensive alias until Labs exists |
| `bountyboard.global` | `/` | Legacy compatibility redirect |

The registrar redirect edge owns alternate hosts and returns a permanent,
unmasked redirect. Runtime traffic uses only the canonical API and MCP hosts;
agents must not send state-changing requests through a vanity-domain redirect.

## DNS records

Create these records on `agentbounties.app`:

| Type | Host | Value |
| --- | --- | --- |
| `A` | `@` | `185.199.108.153` |
| `A` | `@` | `185.199.109.153` |
| `A` | `@` | `185.199.110.153` |
| `A` | `@` | `185.199.111.153` |
| `CNAME` | `www` | `nspg13.github.io` |
| `CNAME` | `api` | `agent-bounties-api.onrender.com` |
| `CNAME` | `mcp` | `agent-bounties-mcp.onrender.com` |

Configure each alternate apex and `www` host as a permanent, unmasked redirect
to its routing-contract destination. Keep API and MCP clients on the canonical
`.app` hosts. Render's two custom-domain slots are reserved for those runtime
origins, and GitHub Pages owns the canonical website certificate.

## Migration order

1. Publish the maintainer notice and merge active contributor work.
2. Add new DNS records without removing old records.
3. Attach and verify the new GitHub Pages and Render custom domains.
4. Deploy canonical URL, discovery, CORS, analytics, and redirect changes.
5. Set repository variables `PRODUCTION_API_BASE_URL`,
   `PRODUCTION_MCP_BASE_URL`, and `PRODUCTION_WEBSITE_BASE_URL` to the new HTTPS
   origins.
6. Verify the website, API, MCP, discovery documents, TLS, redirects, deep
   paths, and query preservation.
7. Redirect the old website only after all new checks pass. Keep old API/MCP
   aliases for at least one documented client-migration window.

## Analytics and search

First-party analytics remains the product-funnel source. GA4 is optional and
consent-based. Create or reuse one GA4 web stream for `agentbounties.app`, set
the repository variable `GA_MEASUREMENT_ID` to its public `G-...` ID, and
verify Realtime after deployment. Do not send wallet, bounty, evidence,
payment, email, or task-content fields to GA4.

Create a DNS-verified Search Console Domain property for every registered
domain and one URL-prefix property for `https://agentbounties.app/`. Submit only
`https://agentbounties.app/sitemap.xml`. Use Search Console's Change of Address
flow for `bountyboard.global` after its permanent redirects are live.

Redirect request counts belong at the Render edge/service logs; post-redirect
conversion belongs in first-party analytics and GA4. Campaign-specific vanity
links may add a stable `utm_source`, but general redirects should stay clean.

## Security

Enable registrar lock, auto-renew, account MFA, a backup payment method, and
DNSSEC where supported. Use email only on `agentbounties.app` with configured
SPF, DKIM, and DMARC. For a domain that will never send or receive mail, publish
a Null MX, `v=spf1 -all`, and rejecting DMARC policy only after confirming no
mail service depends on it.
