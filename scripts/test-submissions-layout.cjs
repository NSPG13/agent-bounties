'use strict';
// Read-only browser journeys with synthetic records. Never contact a wallet or external service.
const assert = require('node:assert/strict'), fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const { chromium } = require('../tools/browser-layout/node_modules/playwright');
const fixture = require('./fixtures/funded-bounty.cjs');
const site = path.resolve(__dirname, '../site');
const hash = char => '0x' + char.repeat(64), wallet = '0x' + 'b'.repeat(40);
const paid = { ...fixture.opportunity, work_state: 'completed', payment_state: 'paid', source_status: 'paid', verification_method: 'deterministic_module', source_url: 'https://github.com/example/design/issues/1', updated_at: '2026-09-22T00:00:00Z' };
const event = (kind, index) => ({ id: kind, kind, tx_hash: hash('a'), block_number: 10, log_index: index, contract_address: paid.source_id, bounty_id: fixture.id, occurred_at: paid.updated_at,
  data: { round: 1, solver: wallet, submission_hash: hash('d'), evidence_hash: hash('e') } });
const evidence = { network: 'base-mainnet', bounty_contract: paid.source_id, bounty_id: fixture.id, round: 1, solver_wallet: wallet, artifact_hash: hash('d'), evidence_hash: hash('e'), artifact_reference: 'https://github.com/example/design/commit/abc', evidence: { checks: ['Files open', 'Dimensions included'] } };
const projection = (view, items) => ({ schema_version: 'agent-bounties/opportunity-projection-v1', network: 'base-mainnet', applied_view: view, degraded: false, source_statuses: [{ source_type: 'canonical_base', available: true }], items });
const server = http.createServer((req, res) => {
  const file = path.resolve(site, '.' + new URL(req.url, 'http://localhost').pathname);
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { const body = fs.readFileSync(file); res.writeHead(200, { 'Content-Type': ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' })[path.extname(file)] || 'application/octet-stream' }).end(body); }
  catch { res.writeHead(404).end(); }
});
async function journey(browser, origin, width) {
  const context = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: 'reduce' });
  let mode = 'ready', heldRoute, feedReads = 0;
  const unexpected = [], mutations = [], errors = [];
  await context.addInitScript(() => { window.ethereum = { request: () => { throw new Error('Submissions must not request a wallet'); } }; });
  await context.route('**/*', route => {
    const request = route.request(), url = new URL(request.url());
    if (request.method() !== 'GET') mutations.push(request.url());
    if (url.pathname === '/v1/opportunities') {
      const completed = url.searchParams.get('work_state') === 'completed';
      const identity = url.searchParams.get('opportunity_id');
      if (identity) return route.fulfill({ json: projection('recent', identity === paid.opportunity_id ? [mode === 'expired' ? { ...paid, work_state: 'claimable', source_status: 'claimable', payment_state: 'escrowed' } : paid] : []) });
      if (completed && mode === 'held') { heldRoute = route; return; }
      return route.fulfill({ status: completed && mode === 'offline' ? 503 : 200, json: projection(url.searchParams.get('view'), completed || url.searchParams.get('view') === 'recent' ? [paid] : [{ ...fixture.opportunity, competition_mode: 'exclusive_claim', deadline: '2099-01-01T00:00:00Z' }]) });
    }
    if (url.pathname === '/v1/base/autonomous-bounties/feed') {
      feedReads++;
      assert.equal(url.searchParams.get('bounty_contract'), paid.source_id, 'history must be scoped to the selected bounty');
      const result = mode === 'expired' ? { ...event('submission_expired', 2), data: { round: 1, solver: wallet, claim_bond_refunded: 2000000 } } : event('bounty_settled', 2);
      return route.fulfill({ json: [{ ...fixture.item, events: [event('submission_added', 1), result] }] });
    }
    if (url.pathname.startsWith('/v1/base/autonomous-bounties/submission-evidence/')) return route.fulfill({ status: mode === 'evidenceOffline' ? 503 : 200, json: evidence });
    if (url.origin !== origin) { unexpected.push(request.url()); return route.abort(); }
    return route.continue();
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin + '/earn.html');
  await page.locator('.opportunity-row').waitFor();
  await page.getByRole('link', { name: 'View Submissions', exact: true }).waitFor();
  await page.locator('[data-market-timing]').selectOption('completed');
  await page.locator('.opportunity-row[data-phase="completed"]').waitFor();
  assert.match(await page.locator('[data-market-summary]').innerText(), /1 completed bounty/);
  assert.doesNotMatch(await page.locator('.opportunity-row').innerText(), /Open participation|before claiming|If you win/);
  await page.getByRole('link', { name: 'View Submissions', exact: true }).click();
  await page.getByRole('link', { name: 'View Winning Submission', exact: true }).waitFor();
  assert.equal(await page.locator('.submission-card').count(), 1);
  assert.equal(await page.getByRole('link', { name: 'View proposals on GitHub' }).getAttribute('href'), paid.source_url);
  assert.equal(await page.locator('.submission-criteria li').count(), 3);
  assert.equal(await page.getByRole('link', { name: 'Open submitted work' }).getAttribute('href'), evidence.artifact_reference);
  await page.getByRole('link', { name: 'View Winning Submission', exact: true }).click();
  assert.equal(new URL(page.url()).hash, '#submission-1');
  await page.getByText('Submission evidence', { exact: true }).click();
  await page.getByText(/Files open/).waitFor();
  assert.match(await page.locator('.submission-result').innerText(), /Why this won/);
  assert.match(await page.locator('.submission-result').innerText(), /written review for each criterion is not available/);
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'history fits the screen');
  if (process.env.MARKET_LAYOUT_ARTIFACTS) {
    fs.mkdirSync(process.env.MARKET_LAYOUT_ARTIFACTS, { recursive: true });
    await page.evaluate(() => scrollTo(0, 0));
    await page.screenshot({ path: path.join(process.env.MARKET_LAYOUT_ARTIFACTS, `submissions-${width}.png`), fullPage: true });
  }
  mode = 'evidenceOffline';
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByText('The work details could not load. Use Refresh to try again.').waitFor();
  assert.equal(await page.locator('.submission-card').count(), 1, 'a failed artifact read must preserve the attempt');
  mode = 'ready';
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByRole('link', { name: 'Open submitted work' }).waitFor();
  mode = 'expired';
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByText('Review expired', { exact: true }).waitFor();
  assert.match(await page.locator('.submission-result').innerText(), /2 USDC of claim bond was returned/);
  assert.equal(await page.getByRole('link', { name: 'View confirmed payment' }).count(), 0);
  assert.equal(await page.getByRole('link', { name: 'View confirmed round outcome' }).count(), 1);
  await page.reload();
  await page.getByText('Review expired', { exact: true }).waitFor();
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'expired history fits the screen');
  mode = 'ready';
  const before = feedReads;
  await page.goto(`${origin}/submissions.html?opportunity=canonical:base-mainnet:0x${'f'.repeat(40)}`);
  await page.getByText('This bounty is not available in the public history.').waitFor();
  assert.equal(feedReads, before, 'unlisted identities cannot trigger artifact or event reads');
  assert.equal(await page.locator('.submission-card').count(), 0);
  await page.goto(origin + '/earn.html?view=completed');
  await page.locator('.opportunity-row[data-phase="completed"]').waitFor();
  mode = 'offline';
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByText('Completed bounty history could not load. Try Refresh.').waitFor();
  assert.equal(await page.locator('.opportunity-row').count(), 0, 'failed refresh hides stale results');
  mode = 'ready';
  await page.locator('[data-market-timing]').selectOption('now');
  await page.locator('.opportunity-row[data-phase="now"]').waitFor();
  mode = 'held';
  const pending = page.waitForRequest(request => new URL(request.url()).searchParams.get('work_state') === 'completed');
  await page.locator('[data-market-timing]').selectOption('completed');
  await pending;
  await page.locator('[data-market-timing]').selectOption('now');
  await page.locator('.opportunity-row[data-phase="now"]').waitFor();
  const stale = page.waitForResponse(response => new URL(response.url()).searchParams.get('work_state') === 'completed');
  await heldRoute.fulfill({ json: projection('recent', [paid]) });
  await stale;
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await page.locator('.opportunity-row[data-phase="completed"]').count(), 0, 'late history response cannot replace open work');
  assert.equal(await page.locator('.opportunity-row[data-phase="now"]').count(), 1);
  assert.deepEqual(mutations, []); assert.deepEqual(unexpected, []); assert.deepEqual(errors, []);
  await context.close();
  console.log(`Completed bounty and submissions journey passed at ${width}px`);
}
(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined });
  try { for (const width of [1440, 390, 320]) await journey(browser, `http://127.0.0.1:${server.address().port}`, width); }
  finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; server.close(); });
