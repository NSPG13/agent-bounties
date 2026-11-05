/* Local browser rehearsal. All API replies and wallet signatures are isolated fixtures. */
const { chromium } = require('playwright');
const fs = require('node:fs'), vm = require('node:vm'), assert = require('node:assert/strict');
const core = require('../site/creator-open-core.js'), vectors = require('../site/fixtures/creator-open-preparations.json');
const scope = { window: {}, TextEncoder, crypto: require('node:crypto').webcrypto, Uint8Array, BigInt };
vm.runInNewContext(fs.readFileSync(require.resolve('../site/evm.js'), 'utf8'), scope);
const evm = scope.window.AgentBountiesEvm, release = vectors.release;
const base = process.env.CREATOR_OPEN_SITE_URL || 'http://127.0.0.1:18680';
if (new URL(base).hostname !== '127.0.0.1') throw new Error('Browser rehearsal requires a loopback static server');
(async () => {
  const browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE, headless: true });
  const context = await browser.newContext();
  const errors = [], calls = [], relays = [], hash = '0x' + '99'.repeat(32);
  let funds = '0', drop = true, enabled = true, last, malformed = false, managedAction = null, managedRequest = null, managedDeadline = Math.floor(Date.now()/1000)+86400;
  await context.addInitScript(() => {
    const address = '0x' + (new URL(location.href).searchParams.has('fixture_solver') ? '09' : '06').repeat(20), listeners = {};
    window.fixtureWalletCalls = [];
    window.ethereum = { isMetaMask: true, on: (key, cb) => { listeners[key] = cb; }, removeListener: key => { delete listeners[key]; }, request: async r => {
      window.fixtureWalletCalls.push(r);
      if (r.method === 'eth_accounts' || r.method === 'eth_requestAccounts') return [address];
      if (r.method === 'eth_chainId') return '0x14a34';
      if (r.method === 'eth_signTypedData_v4') return '0x' + '12'.repeat(65);
      throw new Error('Unexpected fixture wallet method: ' + r.method);
    } };
  });
  await context.route('http://127.0.0.1:3000/**', async route => {
    const url = new URL(route.request().url()), r = route.request().postDataJSON();
    calls.push(url.pathname);
    const reply = value => route.fulfill({ status: 200, contentType: 'application/json', headers: { 'access-control-allow-origin': '*' }, body: JSON.stringify(value) });
    if (route.request().method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': 'content-type', 'access-control-allow-methods': 'GET, POST' } });
    if (url.pathname.endsWith('/capabilities')) return reply({ protocol: core.PROTOCOL, customer_gas_wei: '0', networks: [{ network: 'base-sepolia', available: enabled, release }] });
    if (url.pathname.endsWith('/import')) return reply({ protocol: core.PROTOCOL, network: 'base-sepolia', bounty_contract: core.predict(release, r, evm), creation_transaction_hash: r.creation_transaction_hash, imported: true, index_status: 'pending_canonical_scan' });
    if (url.pathname.endsWith('/prepare')) {
      const p = core.expected(release, r, evm);
      return reply({ prepared: { protocol: core.PROTOCOL, network: 'base-sepolia', request: r, bounty_contract: p.bounty, approval_hash: p.approval_hash, token_authorization: p.token_authorization, action_authorization: p.action_authorization, customer_principal_micro_usdc: malformed ? '999999999' : p.principal, customer_gas_wei: '0' }, wallet_balance_micro_usdc: funds, approval_required: true });
    }
    if (url.pathname.endsWith('/bounties')) {
      const source = managedRequest || relays[0].request, original = source.original_terms, document = { artifact_url: 'https://example.org/result', artifact_sha256: 'sha256:' + 'ab'.repeat(32), notes: 'The exact public result.' };
      const commitment = core.evidence(document, evm), bounty = core.predict(release, source, evm);
      const hasEntry = !['submit', 'increase', 'cancel'].includes(managedAction);
      return reply({ protocol: core.PROTOCOL, items: [{ protocol: core.PROTOCOL, network: 'base-sepolia', bounty_contract: bounty, original_terms: original, user_salt: source.user_salt, terms: original, terms_hash: core.hashJson(original, evm), revision: 1, status: managedAction === 'refund_principal' ? 'cancelled' : 'open', principal_micro_usdc: '21300000', entry_count: hasEntry ? 1 : 0, customer_gas_wei: '0', entries: hasEntry ? [{ id: 1, entry: { solver: '0x' + '09'.repeat(20), submission_hash: commitment, evidence_hash: commitment, status: 'pending', review_deadline: managedDeadline }, public_evidence: document }] : [] }] });
    }
    if (url.pathname.endsWith('/relay')) {
      relays.push(r); last = core.expected(release, r.request, evm);
      if (drop) { drop = false; return route.abort('failed'); }
      const event = { create: 'CreatorOpenBountyCreated', submit: 'OpenSubmissionAdded', review: r.request.action.passed ? 'OpenBountySettled' : 'OpenSubmissionRejected', increase: 'OpenRewardIncreased', cancel: 'OpenBountyCancelled', expire: 'OpenBountyCancelled', refund_bond: 'OpenBondRefunded', refund_principal: 'OpenPrincipalRefunded' }[r.request.action.kind];
      return reply({ protocol: core.PROTOCOL, network: 'base-sepolia', bounty_contract: last.bounty, approval_hash: last.approval_hash, customer_gas_wei: '0', confirmed: true, solver_paid: event === 'OpenBountySettled', operation: { transaction_hash: hash }, evidence: { event } });
    }
    if (url.pathname.endsWith('/status')) return reply({ protocol: core.PROTOCOL, network: 'base-sepolia', bounty_contract: last.bounty, approval_hash: last.approval_hash, customer_gas_wei: '0', confirmed: false, solver_paid: false, operation: { transaction_hash: hash }, evidence: null });
    throw new Error('Unexpected fixture API request: ' + url.pathname);
  });
  const page = await context.newPage(); page.on('pageerror', error => errors.push(error.message));
  await page.goto(base + '/creator-open.html?network=base-sepolia');
  await page.waitForFunction(() => document.querySelector('#open-status').textContent.includes('Review the terms'));
  await page.click('#open-connect'); await page.getByRole('button', { name: 'MetaMask in this browser' }).click();
  await page.waitForFunction(() => document.querySelector('#open-wallet').textContent.includes('Connected:'));
  await page.locator('[name=title]').fill('Browser rehearsal with zero ETH');
  await page.locator('[name=goal]').fill('Deliver the reviewed file.');
  await page.locator('[name=criteria]').fill('The published test passes.');
  await page.click('#open-create-prepare'); await page.waitForFunction(() => !document.querySelector('#open-review').hidden);
  assert.match(await page.locator('#open-balance').innerText(), /Add 21\.3 USDC/);
  await page.check('#open-consent'); assert.equal(await page.locator('#open-confirm').isDisabled(), true);
  assert.equal((await page.evaluate(() => window.fixtureWalletCalls)).filter(c => c.method.includes('sign')).length, 0);
  funds = '21300000'; await page.click('#open-refresh'); await page.waitForFunction(() => !document.querySelector('#open-confirm').disabled);
  await page.click('#open-confirm'); await page.waitForFunction(() => !document.querySelector('#open-retry').hidden && !document.querySelector('#open-retry').disabled);
  assert.equal(relays.length, 1); const first = JSON.stringify(relays[0]);
  const signatureCount = (await page.evaluate(() => window.fixtureWalletCalls)).filter(c => c.method.includes('sign')).length;
  assert.equal(signatureCount, 1);
  const dom = await page.content(); assert.equal(dom.includes('12'.repeat(65)), false, 'signatures must not enter the DOM');
  await page.reload(); await page.waitForFunction(() => document.querySelector('#open-status').textContent.includes('Confirmation is still pending'));
  assert.equal(relays.length, 1, 'reload and status read must not send');
  await page.click('#open-connect'); await page.getByRole('button', { name: 'MetaMask in this browser' }).click(); await page.check('#open-consent');
  await page.click('#open-retry'); await page.waitForFunction(() => !document.querySelector('#open-finish').hidden);
  assert.equal(relays.length, 2); assert.equal(JSON.stringify(relays[1]), first, 'the retry must preserve the entire signed request');
  assert.equal((await page.evaluate(() => window.fixtureWalletCalls)).filter(c => c.method.includes('sign')).length, 0, 'recovery must not request another signature');
  const out = process.env.CREATOR_OPEN_SCREENSHOTS || '/tmp/agent-bounties-report/creator-open-browser'; fs.mkdirSync(out, { recursive: true });
  for (const width of [320, 390, 768, 1280]) {
    await page.setViewportSize({ width, height: 900 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `no horizontal overflow at ${width}px`);
    await page.screenshot({ path: `${out}/creator-open-${width}.png`, fullPage: true });
  }
  assert.deepEqual(errors, []);
  funds = '100000000';
  for (const action of ['submit', 'review', 'increase', 'cancel', 'expire', 'refund_bond', 'refund_principal']) {
    managedDeadline = Math.floor(Date.now()/1000)+(action === 'refund_bond' ? -60 : 86400);
    managedAction = action; managedRequest = JSON.parse(JSON.stringify(relays[0].request));
    if (action === 'expire') managedRequest.original_terms.submission_deadline = Math.floor(Date.now()/1000) - managedRequest.original_terms.review_window_seconds - 60;
    const managedBounty = core.predict(release, managedRequest, evm);
    const managed = await context.newPage(); managed.on('pageerror', e => errors.push(e.message));
    await managed.goto(base + '/creator-open.html?network=base-sepolia&bounty=' + managedBounty + (action === 'submit' ? '&fixture_solver=1' : ''));
    await managed.waitForFunction(() => !document.querySelector('#open-bounty').hidden);
    await managed.click('#open-connect'); await managed.getByRole('button', { name: 'MetaMask in this browser' }).click();
    await managed.selectOption('#open-action', action);
    if (action === 'submit') { await managed.fill('#open-artifact', 'https://example.org/new-result'); await managed.fill('#open-artifact-hash', 'sha256:' + 'cd'.repeat(32)); await managed.fill('#open-notes', 'Exact checks and evidence.'); }
    if (action === 'review') {
      const downloadReady = managed.waitForEvent('download');
      await managed.getByRole('button', { name: 'Add review deadline to calendar' }).click();
      const download = await downloadReady, content = fs.readFileSync(await download.path(), 'utf8').replace(/\r\n /g, '');
      assert.match(content, /creator-open.html\?network=base-sepolia&bounty=/); assert.match(content, /&entry=1/);
    }
    if (action === 'review') { await managed.fill('#open-entry', '1'); await managed.selectOption('#open-verdict', 'pass'); await managed.fill('#open-response', 'The committed artifact meets every criterion.'); }
    if (action === 'refund_bond') await managed.fill('#open-entry', '1');
    if (action === 'increase') await managed.fill('#open-new-reward', '25.30');
    await managed.click('#open-action-prepare'); await managed.waitForFunction(() => !document.querySelector('#open-review').hidden);
    await managed.check('#open-consent'); await managed.click('#open-confirm'); await managed.waitForFunction(() => !document.querySelector('#open-finish').hidden);
    assert.equal(relays.at(-1).request.action.kind, action);
    assert.ok((await managed.evaluate(() => window.fixtureWalletCalls)).every(c => ['eth_accounts','eth_requestAccounts','eth_chainId','eth_signTypedData_v4'].includes(c.method)), action);
    await managed.close();
  }
  assert.deepEqual(errors, []);
  const unavailable = await context.newPage(); enabled = false;
  await unavailable.goto(base + '/creator-open.html?network=base-sepolia');
  await unavailable.waitForFunction(() => document.querySelector('#open-status').textContent.includes('paused'));
  assert.equal(await unavailable.locator('#open-create-prepare').isDisabled(), true);
  assert.equal((await unavailable.evaluate(() => window.fixtureWalletCalls)).length, 0);
  await unavailable.locator('#open-import summary').click();
  await unavailable.fill('#open-import-terms', JSON.stringify(relays[0].request.original_terms));
  await unavailable.fill('#open-import-salt', relays[0].request.user_salt); await unavailable.fill('#open-import-hash', hash);
  await unavailable.click('#open-import-submit'); await unavailable.waitForFunction(() => document.querySelector('#open-status').textContent.includes('Creation verified'));
  assert.equal((await unavailable.evaluate(() => window.fixtureWalletCalls)).length, 0, 'import needs no wallet even when sponsorship is disabled');
  await unavailable.close();
  enabled = true; malformed = true;
  const altered = await context.newPage(); await altered.goto(base + '/creator-open.html?network=base-sepolia');
  await altered.waitForFunction(() => document.querySelector('#open-status').textContent.includes('Review the terms'));
  await altered.click('#open-connect'); await altered.getByRole('button', { name: 'MetaMask in this browser' }).click();
  await altered.locator('[name=title]').fill('Reject altered amount'); await altered.locator('[name=goal]').fill('The exact amount matters.'); await altered.locator('[name=criteria]').fill('No extra spending.');
  await altered.click('#open-create-prepare'); await altered.waitForFunction(() => document.querySelector('#open-status').textContent.includes('differs'));
  assert.equal((await altered.evaluate(() => window.fixtureWalletCalls)).filter(c => c.method.includes('sign')).length, 0);
  console.log(JSON.stringify({ local_only: true, exact_creation: true, zero_eth_wallet: true, insufficient_funds_blocks_signing: true, lost_reply_recovered_without_new_signature: true, read_only_status: true, disabled_sponsorship: true, altered_authority_rejected: true, all_eight_actions: true, exact_calendar: true, external_import_without_wallet: true, viewports: [320, 390, 768, 1280], errors }));
  await browser.close();
})().catch(error => { console.error(error); process.exit(1); });
