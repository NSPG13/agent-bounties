# Retired V1 sponsored posting acceptance

> **Retired flow.** This suite qualifies the autonomous-v1 sponsored relay, which
> was retired for new creation (the API's V1 creation relay returns 410 Gone on
> Base mainnet). New website USDC posting uses autonomous-v2 from the creator's
> own wallet, which pays Base gas; see [website-v2-posting.md](website-v2-posting.md)
> and its local checks. This suite's required outcome of zero customer
> `eth_sendTransaction` calls does not apply to that flow, and its results do not
> qualify it. Keep this suite for a future V2 sponsored relay.

The V1 posting flow asked for one funding signature and then used the creation
relay. It never asked the customer to send a gas-paid creation transaction.
Changes to that flow had to pass the local browser suite below before release.

`npm run test:posting-flow --prefix tools/coinbase-embedded-wallet` runs the current
56-case desktop/phone matrix. The older transaction-dialog flow remains available
as `test:posting-legacy-flow`; the older metrics and safety commands also describe
that historical flow. Their results do not qualify the current sponsor path.

The suite uses the real local API, PostgreSQL, linked-wallet proof, authenticated
terms publication, draft persistence and EIP-712 signatures from a public test
key. Coinbase transport, Base reads, sponsor replies and canonical inventory are
simulated. No production payment, Stripe charge or chain settlement is tested.
The fixture's `authorized-creation-plan` request validates the signed payload; it
is not a browser request to pay gas.

Required outcomes are 56/56 scenarios, no skipped cases and zero customer
`eth_sendTransaction` calls. Retried and concurrent HTTP requests must preserve
one exact signature and payload; their count is not a count of chain broadcasts.
Backend nonce and broadcast reconciliation remain separate required gates.

The matrix covers normal posting, consent, cancellation, capacity, reload,
changed wallet/draft, expired authorization, lost replies, wrong relay identity
or gas payer, exact inventory, signature privacy, tampering and concurrent tabs.
Fresh-policy tests cover changed policy, HTTP failure and bad statement hashes
in the same tab and after reload. Restoring the original policy reuses the saved
signature. Confirmed funding reconciles without another relay during an outage.

## Required local run

Use Node 22+, installed locked wallet dependencies, Playwright Chromium and local
PostgreSQL tools. Use a reviewed API binary with recorded source revision and
SHA-256; this command does not rebuild it. No production keys or RPC settings may
be supplied. Ports 39230, 39231 and 39232 must be free; the site's fixture port,
API website origin and database settings below must agree.

```bash
set -eu
npm ci --prefix tools/coinbase-embedded-wallet --ignore-scripts --no-audit --no-fund
npm rebuild --prefix tools/coinbase-embedded-wallet esbuild
# Install Chromium if absent: (cd tools/coinbase-embedded-wallet && npx playwright install chromium)

: "${POSTING_TEST_API_BINARY:?Use the reviewed local API binary}"
: "${POSTING_TEST_API_SHA256:?Use its verified SHA-256}"
printf '%s  %s\n' "$POSTING_TEST_API_SHA256" "$POSTING_TEST_API_BINARY" | sha256sum --check --status
posting_pg=$(mktemp -d /tmp/ab-posting-test.XXXXXX)
posting_api_pid=
cleanup_posting_test() {
  if [ -n "$posting_api_pid" ]; then
    kill "$posting_api_pid" 2>/dev/null || true
    wait "$posting_api_pid" 2>/dev/null || true
  fi
  pg_ctl -D "$posting_pg" -m fast -w stop >/dev/null 2>&1 || true
  rm -rf -- "$posting_pg"
}
trap cleanup_posting_test EXIT INT TERM
initdb -D "$posting_pg" -U posting_test -A trust --no-locale
pg_ctl -D "$posting_pg" -l "$posting_pg/server.log" -o '-h 127.0.0.1 -p 39232 -k /tmp' -w start
createdb -h 127.0.0.1 -p 39232 -U posting_test posting_test
env -i API_BIND_ADDR=127.0.0.1:39231 \
  DATABASE_URL=postgres://posting_test@127.0.0.1:39232/posting_test \
  AUTH_SESSION_SECRET=local-posting-test-only-not-a-real-secret \
  SITE_POSTING_DRAFTS_ENABLED=true PUBLIC_BASE_URL=http://127.0.0.1:39231 \
  WEBSITE_BASE_URL=http://127.0.0.1:39230 \
  "$POSTING_TEST_API_BINARY" > "$posting_pg/api.log" 2>&1 &
posting_api_pid=$!
posting_api_ready=false
for posting_api_attempt in $(seq 1 30); do
  kill -0 "$posting_api_pid" 2>/dev/null || { echo 'Local API exited before readiness' >&2; exit 1; }
  if curl --silent --fail --max-time 1 http://127.0.0.1:39231/health >/dev/null; then
    posting_api_ready=true
    break
  fi
  sleep 1
done
[ "$posting_api_ready" = true ] || { echo 'Local API readiness timed out' >&2; exit 1; }
POSTING_TEST_API_URL=http://127.0.0.1:39231 \
  POSTING_TEST_DATABASE_URL=postgres://posting_test@127.0.0.1:39232/posting_test \
  PSQL_BIN="$(command -v psql)" \
  npm run test:posting-flow --prefix tools/coinbase-embedded-wallet
```

Run this suite sequentially, separate from legacy suites. The fixture rejects
non-loopback API/database addresses. Between scenarios it resets only the public
fixture principal's drafts and publication quota in that disposable database;
within-scenario journals and retries remain intact. Application limits stay
unchanged. `SPONSORED_EVIDENCE_DIR` may point to an existing private output folder
for per-scenario results; record the exact site/test files and API binary hashes
alongside it. A missing, incomplete or failed run does not count as acceptance.
