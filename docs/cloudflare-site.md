# Cloudflare website operations

Pages builds `main` with `bash scripts/build-static-site.sh` and publishes `site`.
The build verifies wallet assets and origin, prepares licensed fonts and schemas,
refreshes privacy-safe public GitHub participation aggregates, and runs site tests.
The static website manifest at [site-build.json](https://agentbounties.app/.well-known/site-build.json) identifies the source revision and
metrics collection time. Only aggregate counts are published; raw identities and
comment text are kept in memory during collection. Private repository traffic
analytics remain explicitly unavailable without administration permissions.

Public build variables are the existing Coinbase and WalletConnect project IDs,
analytics measurement ID, `NODE_VERSION=22`, `PYTHON_VERSION=3.12` and
`SKIP_DEPENDENCY_INSTALL=true`. The encrypted `GITHUB_METRICS_TOKEN` must have only
public-repository read access and no account or write permissions. Renew it before
its expiration. A failed collection fails the build and retains the previous
successful deployment and its original data timestamp; it never publishes a
new timestamp with incomplete or fabricated totals.

The free scheduled Worker in `ops/cloudflare/` requests one Pages rebuild daily
at 12:17 UTC. Its `PAGES_DEPLOY_HOOK` secret is scoped to this project and the
production branch. It has no signing credentials or general Cloudflare API token.
It rejects foreign hook destinations and HTTP visitors cannot trigger builds.
Cloudflare build and Worker failure logs are the diagnostic surfaces. A successful
hook request is not a completed deployment: check Pages status, then the public
source and metrics timestamps. New source pushes also trigger Pages directly.

Keep the previous host available until both production domains serve the exact
source, wallet configuration, notification handler and manifest over valid TLS.
Then disable the old Pages deployment schedule to avoid competing publishers.
API/MCP hosting, mail records, account origins, budgets and signing keys are
separate from this website cutover and remain unchanged.
