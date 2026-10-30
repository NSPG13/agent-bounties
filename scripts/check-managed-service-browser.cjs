/* Real browser/DOM acceptance, with synthetic account/API/Stripe replies only. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const http = require('node:http'), fs = require('node:fs'), path = require('node:path');
const { createHash } = require('node:crypto');
const fixture = require('../site/fixtures/managed-service-quote.json');
const account = 'a'.repeat(64), other = 'b'.repeat(64);
const quoteId = fixture.quote.input.quote_id, orderId = '22222222-2222-4222-8222-222222222222';
const sign = quote => ({ quote, quote_digest: createHash('sha256').update('agent-bounties:managed-service-quote:v2\0' + JSON.stringify(quote)).digest('hex') });
const root = path.resolve(__dirname, '../site');
const server = http.createServer((req, res) => {
  const name = new URL(req.url, 'http://localhost').pathname.slice(1);
  if (!['account.html', 'managed-service-core.js', 'managed-service-page.js', 'managed-service.css', 'favicon.svg'].includes(name)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'Content-Type': name.endsWith('.js') ? 'application/javascript' : name.endsWith('.css') ? 'text/css' : name.endsWith('.svg') ? 'image/svg+xml' : 'text/html', 'Cache-Control': 'no-store' });
  res.end(fs.readFileSync(path.join(root, name)));
});
const tests = [];
const test = (name, fn) => tests.push([name, fn]);
const visible = async (page, text) => page.waitForFunction(value => document.getElementById('managed-status').textContent.includes(value), text);
let browser, base;
async function setup(options = {}) {
  const context = await browser.newContext({ serviceWorkers: 'block' });
  const page = await context.newPage(), calls = [], requests = [], errors = [], accepts = [];
  await page.addInitScript(() => { document.modelContext = { registerTool(tool) { window.testCardQuoteTool = tool; } }; });
  const q = structuredClone(fixture.quote), now = Math.floor(Date.now() / 1000);
  Object.assign(q.input, { issued_at: now - 10, expires_at: now + 7200 }); q.input.price_basis.valid_until = now + 7200; q.input.terms.delivery_deadline = now + 86400; q.input.bounty.funding_deadline = now + 86400;
  const state = { account, quote: sign(q), status: 'awaiting_payment', verdicts: [], ...options };
  state.review = state.review || { order_id: orderId, network: 'base-sepolia', bounty_contract: q.input.bounty.predicted_bounty, bounty_status: 'submitted', reviews: [],
    pending_submission: { round: 1, solver: '0x' + '5'.repeat(40), submission_hash: '0x' + '1'.repeat(64), evidence_hash: '0x' + '2'.repeat(64),
      verification_expires_at: now + 86400, decide_by: now + 43200, evidence: { artifact_reference: 'https://example.com/delivery.txt', artifact_hash: '0x' + '1'.repeat(64), evidence: {} } },
    deposit: { amount_minor: 935, currency: 'mxn', status: 'held' } };
  function order() { return { order_id: orderId, ...state.quote, accepted_at: state.quote.quote.input.issued_at + 1, status: state.status }; }
  page.on('pageerror', error => errors.push(error.message));
  context.on('request', request => requests.push({ url: request.url(), headers: request.headers() }));
  await context.route('**/*', async route => {
    const req = route.request(), url = new URL(req.url());
    if (url.origin === base) return route.continue();
    if (url.origin === 'https://checkout.stripe.com') return route.fulfill({ status: 200, contentType: 'text/html', body: '<h1>Synthetic Stripe destination</h1>' });
    if (url.origin !== 'http://127.0.0.1:3000') { errors.push('Unexpected network destination: ' + url.origin); return route.abort(); }
    const call = { path: url.pathname, method: req.method(), body: req.postDataJSON(), headers: req.headers() }; calls.push(call);
    const reply = (value, code = 200) => route.fulfill({ status: code, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': base, 'Access-Control-Allow-Credentials': 'true' }, body: JSON.stringify(value) });
    if (req.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'Access-Control-Allow-Origin': base, 'Access-Control-Allow-Credentials': 'true', 'Access-Control-Allow-Headers': 'content-type', 'Access-Control-Allow-Methods': 'GET, POST' } });
    if (url.pathname === '/v1/site-auth/session') return reply({ authenticated: !!state.account, user: state.account ? { id: state.account } : null, providers: { google: true, github: true, amazon: false, evil: true }, needs_wallet: true });
    if (url.pathname === '/v1/site-auth/logout') { state.account = null; return route.fulfill({ status: 204, headers: { 'Access-Control-Allow-Origin': base, 'Access-Control-Allow-Credentials': 'true' } }); }
    if (url.pathname === '/v1/managed-service/bounty-quotes') {
      if (state.prepareWait) await state.prepareWait;
      if (state.prepareDrop) return route.abort('failed');
      if (state.switchAfterPrepare) state.account = other;
      return reply(state.quote, state.prepareCode || 200);
    }
    if (url.pathname.endsWith('/accept')) {
      accepts.push(call.body);
      if (state.acceptWait) await state.acceptWait;
      if (state.dropAccept) { state.dropAccept = false; return route.abort('failed'); }
      if (state.acceptCode) return reply({}, state.acceptCode);
      return reply({ order: order() });
    }
    if (url.pathname.endsWith('/refund')) {
      if (state.refundWait) await state.refundWait;
      if (state.refundCode) return reply({}, state.refundCode);
      state.status = state.refundStatus || 'refund_requested';
      if (state.dropRefund) { state.dropRefund = false; return route.abort('failed'); }
      return reply({ order: order(), refund_execution: 'deferred' });
    }
    if (url.pathname === '/v1/managed-service/orders/' + orderId + '/review') {
      if (req.method() === 'POST') {
        state.verdicts.push(call.body);
        state.review.reviews.unshift({ document: { schema: 'agent-bounties/customer-review-v1', bounty_contract: state.review.bounty_contract, round: call.body.round,
          submission_hash: call.body.submission_hash, evidence_hash: call.body.evidence_hash, passed: call.body.passed, reason: call.body.reason },
          appeal: call.body.passed ? null : { status: 'window_open', window_ends_at: now + 172800 } });
        return reply({ document: state.review.reviews[0].document, relay: { status: 'broadcast' } });
      }
      return reply(state.review);
    }
    if (url.pathname.endsWith('/checkout')) {
      if (state.checkoutCode) return reply({}, state.checkoutCode);
      return reply(state.checkoutReply || { order_id: orderId, checkout: { status: 'bound', session_id: 'cs_test_fixture', checkout_url: 'https://checkout.stripe.com/c/pay/cs_test_fixture#synthetic' } });
    }
    if (req.method() === 'GET' && url.pathname === '/v1/managed-service/quotes/' + quoteId) {
      if (state.quoteWait) await state.quoteWait;
      return reply(state.quote, state.readCode || 200);
    }
    if (req.method() === 'GET' && url.pathname === '/v1/managed-service/orders/' + orderId) return reply({ order: order(), recovered_bounty_refund: state.recovered }, state.readCode || 200);
    errors.push('Unexpected API call: ' + call.path); return reply({}, 404);
  });
  const go = (kind = 'managed_quote', suffix = '') => page.goto(base + '/account.html?' + kind + '=' + (kind === 'managed_quote' ? quoteId : orderId) + suffix);
  async function close() { assert.deepEqual(errors, []); await context.close(); }
  return { context, page, calls, requests, accepts, state, go, close };
}

const quoteDraft = () => ({ document: { title: 'Check the public website', goal: 'Return a timestamped public report.',
  acceptance_criteria: ['Give the public URL and HTTP status.'], benchmark: { engine: 'customer_review_v1', delivery_deadline: Math.floor(Date.now() / 1000) + 86400 * 7 },
  evidence_schema: { type: 'object' } }, solver_reward_micro_usdc: 100000000, currency: 'mxn', maximum_charge_minor: 217375 });
const prepare = (page, input = quoteDraft()) => page.evaluate(async value => {
  try { return await window.testCardQuoteTool.execute(value); } catch (error) { return { error: error.message }; }
}, input);
test('WebMCP prepares an account-bound quote visibly without accepting or opening Checkout', async () => {
  const f = await setup(); await f.page.goto(base + '/account.html');
  const tool = await f.page.evaluate(() => ({ name: window.testCardQuoteTool.name, annotations: window.testCardQuoteTool.annotations, description: window.testCardQuoteTool.description }));
  assert.equal(tool.name, 'agent_bounties_prepare_card_quote'); assert.equal(tool.annotations.readOnlyHint, false); assert.match(tool.description, /Publish.*public bounty terms/);
  const result = await prepare(f.page); assert.equal(result.status, 'quote_ready_for_personal_review');
  assert.equal(result.total_minor, 217375); assert.equal(result.accepted, false); assert.equal(result.checkout_created, false);
  await visible(f.page, 'Review the deliverable'); assert.equal(await f.page.locator('#managed-consent').isChecked(), false);
  assert.equal(await f.page.locator('#managed-accept').isDisabled(), true);
  assert.deepEqual(f.calls.filter(c => c.method === 'POST').map(c => c.path), ['/v1/managed-service/bounty-quotes']);
  assert.match((await prepare(f.page)).error, /current quote/); assert.equal(f.calls.filter(c => c.method === 'POST').length, 1); await f.close();
});
test('a fractional USDC reward can display exactly MXN10 with all itemized costs', async () => {
  const f = await setup(), q = f.state.quote.quote;
  q.input.solver_reward_micro_usdc = 178000; q.input.verifier_reserve_micro_usdc = 100000;
  q.platform_fee_micro_usdc = 13350; q.bounty_target_micro_usdc = 291350;
  const amounts = [[178000,344],[100000,193],[13350,26]];
  q.lines = q.lines.slice(0,3).map((line,i) => ({ ...line, amount_micro_usdc: amounts[i][0], amount_minor: amounts[i][1] }));
  q.input.additional_costs = [ { code: 'currency_conversion', label: 'Currency conversion buffer (1.00%)', amount_minor: 6 },
    { code: 'review_deposit', label: 'Review deposit (refundable, 10% of solver reward)', amount_minor: 35 },
    { code: 'payment_processing', label: 'Card processing', amount_minor: 396 } ];
  q.lines.push(...q.input.additional_costs.map(line => ({ ...line, amount_micro_usdc: null }))); q.total_minor = 1000; f.state.quote = sign(q);
  await f.page.goto(base + '/account.html');
  const result = await prepare(f.page,{ ...quoteDraft(), solver_reward_micro_usdc: 178000, maximum_charge_minor: 1000 });
  assert.equal(result.total_minor,1000); assert.equal(await f.page.locator('#managed-total').innerText(),'MXN 10.00');
  assert.equal(await f.page.locator('#managed-consent').isChecked(),false); assert.equal(f.accepts.length,0); await f.close();
});
test('concurrent preparation and a late response after navigation cannot publish twice or expose a quote', async () => {
  let finish; const wait = new Promise(resolve => { finish=resolve; });
  const f = await setup({ prepareWait: wait }); await f.page.goto(base + '/account.html');
  const first=prepare(f.page); await f.page.waitForRequest(r => r.url().endsWith('/bounty-quotes'));
  assert.match((await prepare(f.page)).error,/current quote/);
  await f.page.evaluate(() => window.dispatchEvent(new Event('pagehide'))); finish(); await first;
  assert.equal(f.calls.filter(c => c.method === 'POST').length,1);
  assert.equal(await f.page.locator('#managed-private').isVisible(),false); assert.equal(f.accepts.length,0); await f.close();
});

test('preparation rejects invalid input and a signed-out account before publishing terms', async () => {
  for (const bad of [ { ...quoteDraft(), maximum_charge_minor: -1 }, { ...quoteDraft(), document: { ...quoteDraft().document, private_key: 'never-transmit' } } ]) {
    const f = await setup(); await f.page.goto(base + '/account.html'); assert.ok((await prepare(f.page, bad)).error); assert.equal(f.calls.length, 0); await f.close();
  }
  const f = await setup({ account: null }); await f.page.goto(base + '/account.html'); assert.ok((await prepare(f.page)).error);
  assert.equal(f.calls.filter(c => c.method === 'POST').length, 0); await f.close();
});
test('wrong owner, digest, amount, currency or reward never exposes acceptance after preparation', async () => {
  for (const change of [f => { f.state.switchAfterPrepare = true; }, f => { f.state.quote.quote_digest = '0'.repeat(64); },
    f => {}]) {
    const f = await setup(); change(f); const i = quoteDraft();
    if (!f.state.switchAfterPrepare && f.state.quote.quote_digest !== '0'.repeat(64)) i.maximum_charge_minor = 1000;
    await f.page.goto(base + '/account.html'); assert.ok((await prepare(f.page, i)).error);
    assert.equal(await f.page.locator('#managed-private').isVisible(), false); assert.equal(f.accepts.length, 0); await f.close();
  }
  for (const input of [{ ...quoteDraft(), currency: 'usd' }, { ...quoteDraft(), solver_reward_micro_usdc: 1 }]) {
    const f = await setup(); await f.page.goto(base + '/account.html'); assert.ok((await prepare(f.page,input)).error); assert.equal(f.accepts.length,0); await f.close();
  }
});
test('malformed and ambiguous incoming operation links cannot become fresh preparation pages', async () => {
  for (const query of ['?managed_quote=invalid','?managed_quote='+quoteId+'&managed_order='+orderId,'?unrelated=1','#managed_quote='+quoteId]) {
    const f=await setup(); await f.page.goto(base+'/account.html'+query); assert.match((await prepare(f.page)).error,/current quote/);
    assert.equal(f.calls.filter(c => c.method === 'POST').length,0); await f.close();
  }
});
test('uncertain quote creation cannot be retried automatically and never creates an order', async () => {
  const f = await setup({ prepareDrop: true }); await f.page.goto(base + '/account.html');
  assert.ok((await prepare(f.page)).error); assert.match((await prepare(f.page)).error,/replacement/);
  assert.equal(f.calls.filter(c => c.method === 'POST').length,1); assert.equal(f.accepts.length,0); await f.close();
});

test('recovered refund is separate from historical funding and never offers a new verdict', async () => {
  const amount = fixture.quote.lines.filter(l => ['solver_reward', 'verification'].includes(l.code)).reduce((n, l) => n + l.amount_minor, 0);
  for (const outcome of ['pending_binding', 'awaiting_signed_event', 'pending', 'succeeded', 'reconciliation_required']) {
    const f = await setup({ status: 'funded', recovered: { status: outcome, amount_minor: amount, currency: 'mxn', reconciliation_required: false } });
    await f.go('managed_order');
    await f.page.waitForFunction(() => document.getElementById('managed-next').textContent.includes('Refund amount:'));
    assert.match(await f.page.locator('#managed-next').innerText(), /Fees and the review deposit are excluded/);
    assert.equal(await f.page.locator('#managed-review-form').isVisible(), false);
    assert.equal(f.calls.some(c => c.path.endsWith('/review') || c.method === 'POST'), false);
    if (outcome === 'succeeded') assert.match(await f.page.locator('#managed-status').innerText(), /Stripe confirms.*sent/);
    else assert.doesNotMatch(await f.page.locator('#managed-status').innerText(), /confirms.*sent/);
    await f.close();
  }
});
test('sign-in preserves only the scoped offer; no wallet or analytics setup', async () => {
  const f = await setup({ account: null }); await f.go('managed_quote', '&checkout_session_id=cs_fake&paid=true'); await visible(f.page, 'Sign in to view');
  assert.equal(await f.page.locator('#managed-providers a').count(), 2);
  for (const link of await f.page.locator('#managed-providers a').evaluateAll(els => els.map(e => e.href))) {
    const u = new URL(link); assert.equal(u.origin, 'http://127.0.0.1:3000'); assert.equal(u.searchParams.get('return_to'), '/account.html?managed_quote=' + quoteId);
  }
  assert.equal(new URL(f.page.url()).search, '?managed_quote=' + quoteId);
  assert.equal(f.calls.length, 1); assert.equal(await f.page.locator('#managed-private').isVisible(), false);
  assert.deepEqual(await f.page.evaluate(() => [localStorage.length, sessionStorage.length, document.querySelector('meta[name="referrer"]').content]), [0, 0, 'no-referrer']); await f.close();
});
test('quote review shows full amounts and terms, with safe mobile layout', async () => {
  const f = await setup(); await f.go(); await visible(f.page, 'Review the deliverable');
  assert.match(await f.page.locator('#managed-lines').innerText(), /1930|1,930/);
  assert.equal(await f.page.locator('#managed-total').innerText(), 'MXN 2,173.75');
  assert.match(await f.page.locator('#managed-lines').innerText(), /7\.5%/);
  assert.equal(await f.page.locator('#managed-accept').isDisabled(), true);
  assert.equal(f.calls.filter(c => c.method === 'POST').length, 0);
  const out = process.env.MANAGED_SERVICE_SCREENSHOTS;
  if (out) fs.mkdirSync(out, { recursive: true });
  for (const width of [320, 390, 768, 1280]) {
    await f.page.setViewportSize({ width, height: 900 });
    assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'no overflow at ' + width);
    if (out) await f.page.screenshot({ path: path.join(out, 'managed-quote-' + width + '.png'), fullPage: true });
  }
  assert.equal(f.requests.some(r => /analytics|wallet|ethereum|onramp/.test(r.url)), false); await f.close();
});
test('lost acceptance response survives reload with the original key, then explicit Stripe handoff', async () => {
  const f = await setup({ dropAccept: true }); await f.go(); await visible(f.page, 'Review the deliverable');
  await f.page.check('#managed-consent'); await f.page.click('#managed-accept'); await visible(f.page, 'result could not be confirmed');
  assert.equal(f.accepts.length, 1); await f.page.reload(); await visible(f.page, 'Review the deliverable');
  assert.equal(f.accepts.length, 1); await f.page.check('#managed-consent'); await f.page.click('#managed-accept'); await visible(f.page, 'Payment is not confirmed');
  assert.equal(f.accepts.length, 2); assert.deepEqual(f.accepts[0], f.accepts[1]);
  assert.equal(new URL(f.page.url()).search, '?managed_order=' + orderId);
  assert.equal(f.calls.filter(c => c.path.endsWith('/checkout')).length, 0);
  await f.page.click('#managed-checkout'); await f.page.waitForURL('https://checkout.stripe.com/**');
  const destination = f.requests.find(r => r.url.startsWith('https://checkout.stripe.com/'));
  assert.equal(destination.headers.referer, undefined);
  for (const call of f.calls.filter(c => c.method === 'POST')) {
    assert.equal(call.headers.origin, base); assert.equal(call.headers.authorization, undefined); assert.equal(call.headers['x-operator-token'], undefined);
  }
  await f.close();
});
test('concurrent clicks cannot send two acceptances', async () => {
  let release; const f = await setup({ acceptWait: new Promise(resolve => { release = resolve; }) });
  await f.go(); await visible(f.page, 'Review the deliverable'); await f.page.check('#managed-consent');
  await f.page.evaluate(() => { const b = document.getElementById('managed-accept'); b.click(); b.click(); });
  await f.page.waitForFunction(() => document.getElementById('managed-status').textContent.includes('Saving'));
  await new Promise(resolve => setTimeout(resolve, 50)); release(); await visible(f.page, 'Payment is not confirmed'); assert.equal(f.accepts.length, 1); await f.close();
});
test('return parameters never prove payment; only later owner status changes the display', async () => {
  const f = await setup(); await f.go('managed_order', '&checkout_session_id=cs_live_forged&paid=true&cancelled=true'); await visible(f.page, 'Payment is not confirmed');
  assert.equal(f.calls.some(c => c.method === 'POST'), false);
  f.state.status = 'awaiting_bank_settlement'; await f.page.click('#managed-refresh'); await visible(f.page, 'Bounty funding has not started');
  assert.match(await f.page.locator('#managed-next').innerText(), /not yet confirmed/);
  assert.equal(await f.page.locator('#managed-checkout').isVisible(), false); await f.close();
});
test('pending, failed and expired payments never offer another charge', async () => {
  for (const [state, text] of [['payment_pending', 'payment is processing'], ['payment_failed', 'payment attempt failed'], ['payment_expired', 'payment session expired']]) {
    const f = await setup({ status: state }); await f.go('managed_order'); await visible(f.page, text);
    assert.equal(await f.page.locator('#managed-checkout').isVisible(), false); assert.equal(f.calls.some(c => c.method === 'POST'), false); await f.close();
  }
});
test('bad URLs make no account/API requests', async () => {
  for (const suffix of ['', '?managed_order=bad', `?managed_order=${orderId}&managed_order=${orderId}`, `?managed_order=${orderId}&managed_quote=${quoteId}`]) {
    const f = await setup(); await f.page.goto(base + '/account.html' + suffix); await visible(f.page, 'Open the quote or order link');
    assert.equal(f.calls.length, 0); assert.equal(new URL(f.page.url()).search, ''); await f.close();
  }
});
test('embedded payment pages cannot read accounts or accept a quote', async () => {
  const f = await setup();
  await f.page.setContent('<iframe src="' + base + '/account.html?managed_quote=' + quoteId + '"></iframe>');
  const child = f.page.frames().find(frame => frame !== f.page.mainFrame());
  await child.waitForFunction(() => document.getElementById('managed-status')?.textContent.includes('Open the quote or order link'));
  assert.equal(f.calls.length, 0);
  assert.equal(await child.locator('#managed-private').isVisible(), false); await f.close();
});
test('untrusted deliverables and labels render as text, never HTML', async () => {
  const f = await setup(); const q = f.state.quote.quote;
  q.input.terms.deliverable = '<img src=x onerror="window.compromised=true">';
  q.input.additional_costs[0].label = '<svg onload="window.compromised=true">'; q.lines[3].label = q.input.additional_costs[0].label; f.state.quote = sign(q);
  await f.go(); await visible(f.page, 'Review the deliverable');
  assert.equal(await f.page.locator('#managed-private img, #managed-private svg').count(), 0);
  assert.match(await f.page.locator('#managed-deliverable').innerText(), /<img/);
  assert.equal(await f.page.evaluate(() => window.compromised), undefined); await f.close();
});
test('tampered quote, wrong account, and unknown payout status never render private details', async () => {
  for (const mode of ['quote', 'owner', 'paid_out']) {
    const f = await setup(); if (mode === 'quote') f.state.quote.quote.total_minor++;
    if (mode === 'owner') f.state.account = other; if (mode === 'paid_out') f.state.status = 'paid_out';
    await f.go(mode === 'paid_out' ? 'managed_order' : 'managed_quote'); await visible(f.page, 'could not safely load');
    assert.equal(await f.page.locator('#managed-private').isVisible(), false); assert.equal(await f.page.locator('#managed-deliverable').innerText(), ''); await f.close();
  }
});
test('account switch before acceptance clears private details and sends no write', async () => {
  const f = await setup(); await f.go(); await visible(f.page, 'Review the deliverable'); await f.page.check('#managed-consent');
  f.state.account = other; await f.page.click('#managed-accept'); await visible(f.page, 'sign-in changed or expired');
  assert.equal(f.accepts.length, 0); assert.equal(await f.page.locator('#managed-deliverable').innerText(), ''); await f.close();
});
test('hide and sign-out clear private details; late responses cannot repaint them', async () => {
  let release; const f = await setup({ quoteWait: new Promise(resolve => { release = resolve; }) }); await f.go();
  await f.page.waitForFunction(() => !document.getElementById('managed-logout').hidden);
  await f.page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange')); });
  release(); await new Promise(resolve => setTimeout(resolve, 50));
  assert.equal(await f.page.locator('#managed-deliverable').innerText(), '');
  await f.page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: false }); document.dispatchEvent(new Event('visibilitychange')); });
  await visible(f.page, 'Review the deliverable'); await f.page.click('#managed-logout'); await visible(f.page, 'Sign in to view');
  assert.equal(await f.page.locator('#managed-deliverable').innerText(), ''); await f.close();
});
test('read failures clear private data and never imply zero balance or payment', async () => {
  for (const [code, text] of [[401, 'sign-in changed'], [404, 'not available to'], [503, 'currently unavailable']]) {
    const f = await setup({ readCode: code }); await f.go(); await visible(f.page, text);
    assert.equal(await f.page.locator('#managed-private').isVisible(), false); await f.close();
  }
});
test('containment and uncertain Checkout keep the existing order link', async () => {
  for (const [code, text] of [[409, 'needs reconciliation'], [503, 'currently unavailable']]) {
    const f = await setup({ checkoutCode: code }); await f.go('managed_order'); await visible(f.page, 'Payment is not confirmed');
    await f.page.click('#managed-checkout'); await visible(f.page, text);
    assert.equal(new URL(f.page.url()).search, '?managed_order=' + orderId); assert.equal(f.calls.filter(c => c.path.endsWith('/checkout')).length, 1); await f.close();
  }
});
test('malicious, wrong-session or wrong-mode Checkout URLs cannot navigate', async () => {
  for (const checkout of [
    { status: 'bound', session_id: 'cs_test_fixture', checkout_url: 'https://evil.example/c/pay/cs_test_fixture' },
    { status: 'bound', session_id: 'cs_test_fixture', checkout_url: 'https://checkout.stripe.com/c/pay/cs_test_another' },
    { status: 'bound', session_id: 'cs_live_fixture', checkout_url: 'https://checkout.stripe.com/c/pay/cs_live_fixture' }
  ]) {
    const f = await setup({ checkoutReply: { order_id: orderId, checkout } }); await f.go('managed_order'); await visible(f.page, 'Payment is not confirmed');
    await f.page.click('#managed-checkout'); await visible(f.page, 'result could not be confirmed');
    assert.equal(new URL(f.page.url()).origin, base); await f.close();
  }
});
test('expired quotes support exact prior-acceptance recovery without pretending to renew the offer', async () => {
  const f = await setup(); const q = f.state.quote.quote; q.input.issued_at -= 86400; q.input.expires_at = Math.floor(Date.now() / 1000) - 5; f.state.quote = sign(q);
  await f.go(); await visible(f.page, 'quote has expired'); await f.page.check('#managed-consent'); await f.page.click('#managed-accept');
  await visible(f.page, 'Payment is not confirmed'); assert.equal(f.accepts.length, 1); assert.equal(f.accepts[0].quote_digest, f.state.quote.quote_digest); await f.close();
});
test('a timed-out acceptance can retry only the original request', async () => {
  let release; const f = await setup({ acceptWait: new Promise(resolve => { release = resolve; }) });
  await f.page.clock.install(); await f.go(); await visible(f.page, 'Review the deliverable');
  await f.page.check('#managed-consent'); await f.page.click('#managed-accept');
  for (let i = 0; !f.accepts.length && i < 100; i++) await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(f.accepts.length, 1);
  await f.page.clock.runFor(15100); await visible(f.page, 'result could not be confirmed');
  release(); await f.page.click('#managed-accept'); await visible(f.page, 'Payment is not confirmed');
  assert.equal(f.accepts.length, 2); assert.deepEqual(f.accepts[0], f.accepts[1]); await f.close();
});
test('return polling is read-only and stops after two minutes', async () => {
  const f = await setup(); await f.page.clock.install(); await f.go('managed_order'); await visible(f.page, 'Payment is not confirmed');
  for (let i = 0; i < 8; i++) { await f.page.clock.runFor(15010); await visible(f.page, 'Payment is not confirmed'); }
  const reads = f.calls.filter(c => c.method === 'GET' && c.path.includes('/orders/')).length;
  assert.equal(reads, 9); await f.page.clock.runFor(60000);
  assert.equal(f.calls.filter(c => c.method === 'GET' && c.path.includes('/orders/')).length, reads);
  assert.equal(f.calls.filter(c => c.method === 'POST').length, 0); await f.close();
});
test('expired new acceptance and rate limits stay recoverable without a changed key', async () => {
  for (const [code, text] of [[410, 'offer has expired'], [429, 'Wait a minute']]) {
    const f = await setup({ acceptCode: code }); await f.go(); await visible(f.page, 'Review the deliverable');
    await f.page.check('#managed-consent'); await f.page.click('#managed-accept'); await visible(f.page, text);
    assert.equal(f.accepts.length, 1); assert.equal(new URL(f.page.url()).search, '?managed_quote=' + quoteId); await f.close();
  }
});

test('refund requires explicit consent, includes all costs and prevents duplicate clicks', async () => {
  let release; const f = await setup({ status: 'awaiting_bank_settlement', refundWait: new Promise(resolve => { release = resolve; }) });
  await f.go('managed_order'); await visible(f.page, 'Bounty funding has not started');
  assert.equal(await f.page.locator('#managed-refund').isDisabled(), true);
  assert.match(await f.page.locator('#managed-refund-info').innerText(), /MXN 2,173.75/);
  assert.equal(f.calls.some(c => c.method === 'POST'), false);
  for (const width of [320, 390, 768, 1280]) {
    await f.page.setViewportSize({ width, height: 900 });
    assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    if (process.env.MANAGED_SERVICE_SCREENSHOTS) await f.page.screenshot({ path: path.join(process.env.MANAGED_SERVICE_SCREENSHOTS, 'managed-refund-' + width + '.png'), fullPage: true });
  }
  await f.page.check('#managed-refund-consent');
  await f.page.evaluate(() => { const b = document.getElementById('managed-refund'); b.click(); b.click(); });
  await visible(f.page, 'Recording your cancellation'); release(); await visible(f.page, 'full refund has been requested');
  const writes = f.calls.filter(c => c.path.endsWith('/refund')); assert.equal(writes.length, 1);
  assert.equal(writes[0].path, '/v1/managed-service/orders/' + orderId + '/refund'); assert.deepEqual(writes[0].body, {});
  assert.equal(await f.page.locator('#managed-refund-form').isVisible(), false);
  assert.equal(await f.page.locator('#managed-checkout').isVisible(), false); await f.close();
});
test('lost refund response retries the same order without a new amount or payment', async () => {
  const f = await setup({ status: 'awaiting_bank_settlement', dropRefund: true });
  await f.go('managed_order'); await visible(f.page, 'Bounty funding has not started');
  await f.page.check('#managed-refund-consent'); await f.page.click('#managed-refund'); await visible(f.page, 'result could not be confirmed');
  await f.page.click('#managed-refund'); await visible(f.page, 'full refund has been requested');
  const writes = f.calls.filter(c => c.method === 'POST'); assert.equal(writes.length, 2);
  assert.equal(writes[0].path, writes[1].path); assert.deepEqual(writes[0].body, writes[1].body);
  await f.page.reload(); await visible(f.page, 'full refund has been requested');
  assert.equal(f.calls.filter(c => c.method === 'POST').length, 2); await f.close();
});
test('account change before cancellation sends no refund request', async () => {
  const f = await setup({ status: 'awaiting_bank_settlement' }); await f.go('managed_order'); await visible(f.page, 'Bounty funding has not started');
  await f.page.check('#managed-refund-consent'); f.state.account = other; await f.page.click('#managed-refund'); await visible(f.page, 'sign-in changed or expired');
  assert.equal(f.calls.some(c => c.method === 'POST'), false); assert.equal(await f.page.locator('#managed-refund-form').isVisible(), false); await f.close();
});
test('refund states distinguish requested, pending, attention, sent and failed without claiming bank credit', async () => {
  for (const [status, message] of [['refund_requested', 'has been requested'], ['refund_pending', 'is processing'], ['refund_action_required', 'needs additional attention'], ['refund_sent', 'refund was sent'], ['refund_failed', 'could not be completed']]) {
    const f = await setup({ status }); await f.go('managed_order', '&refunded=true'); await visible(f.page, message);
    assert.equal(await f.page.locator('#managed-refund-form').isVisible(), false); assert.equal(await f.page.locator('#managed-checkout').isVisible(), false);
    assert.equal(f.calls.some(c => c.method === 'POST'), false);
    if (status === 'refund_sent') assert.match(await f.page.locator('#managed-next').innerText(), /does not confirm.*bank account/);
    await f.close();
  }
});
test('refund polling only reads and stops on signed-result status', async () => {
  const f = await setup({ status: 'refund_pending' }); await f.page.clock.install(); await f.go('managed_order'); await visible(f.page, 'is processing');
  f.state.status = 'refund_sent'; await f.page.clock.runFor(15010); await visible(f.page, 'refund was sent');
  const reads = f.calls.filter(c => c.path.includes('/orders/')).length; await f.page.clock.runFor(60000);
  assert.equal(f.calls.filter(c => c.path.includes('/orders/')).length, reads); assert.equal(f.calls.some(c => c.method === 'POST'), false); await f.close();
});
test('funding statuses never offer cancellation and always link the exact committed bounty', async () => {
  for (const [state, text] of [['funding_reserved', 'being funded'], ['funding_broadcast', 'transaction was sent'], ['funding_reconciliation_required', 'needs our attention']]) {
    const f = await setup({ status: state }); await f.go('managed_order'); await visible(f.page, text);
    assert.equal(await f.page.locator('#managed-refund-form').isVisible(), false); assert.equal(await f.page.locator('#managed-checkout').isVisible(), false);
    assert.match(await f.page.locator('#managed-bounty-link').innerText(), /active once funded/); assert.equal(f.calls.some(c => c.method === 'POST'), false); await f.close();
  }
  const f = await setup({ status: 'funding_broadcast' }); await f.page.clock.install(); await f.go('managed_order'); await visible(f.page, 'transaction was sent');
  assert.match(await f.page.locator('#managed-next').innerText(), /not confirmed funding/);
  f.state.status = 'funded'; await f.page.clock.runFor(15010); await visible(f.page, 'funded on Base');
  const bounty = f.state.quote.quote.input.bounty;
  assert.equal(await f.page.locator('#managed-bounty-link').getAttribute('href'), 'https://sepolia.basescan.org/address/' + bounty.predicted_bounty);
  assert.equal(await f.page.locator('#managed-bounty-link').innerText(), 'View your funded bounty on Basescan');
  assert.equal(await f.page.locator('#managed-refund-form').isVisible(), false);
  const reads = f.calls.filter(c => c.path.includes('/orders/')).length; await f.page.clock.runFor(60000);
  assert.equal(f.calls.filter(c => c.path.includes('/orders/')).length, reads); assert.equal(f.calls.some(c => c.method === 'POST'), false); await f.close();
});
test('a funded order reviews only its committed submission and sends an exact, reasoned verdict', async () => {
  const f = await setup({ status: 'funded' }); await f.go('managed_order'); await visible(f.page, 'funded on Base');
  await f.page.waitForSelector('#managed-review-form', { state: 'visible' });
  assert.equal(await f.page.locator('#managed-review-artifact').getAttribute('href'), 'https://example.com/delivery.txt');
  await f.page.check('#managed-verdict-reject'); await f.page.check('#managed-review-consent');
  await f.page.fill('#managed-review-reason', 'Too short');
  assert.equal(await f.page.locator('#managed-review-submit').isDisabled(), true);
  await f.page.fill('#managed-review-reason', 'The delivery omits the requested benchmark table.');
  assert.equal(await f.page.locator('#managed-review-submit').isDisabled(), false);
  await f.page.click('#managed-review-submit'); await visible(f.page, 'verdict is recorded');
  assert.deepEqual(f.state.verdicts, [{ round: 1, submission_hash: '0x' + '1'.repeat(64), evidence_hash: '0x' + '2'.repeat(64), passed: false,
    reason: 'The delivery omits the requested benchmark table.' }]);
  await f.page.waitForFunction(() => document.getElementById('managed-review-info').textContent.includes('You rejected attempt 1. The solver can appeal until'));
  assert.match(await f.page.locator('#managed-review-deposit').innerText(), /Refundable review deposit: MXN 9\.35/);
  assert.equal(await f.page.locator('#managed-review-form').isVisible(), false); await f.close();
});
test('the order page promises automatic acceptance only when the API confirms it', async () => {
  const info = page => page.locator('#managed-review-info').innerText();
  for (const [schedule, expected, absent] of [
    [{ auto_accept: true }, /If you don’t decide by then, it is accepted automatically\. The solver is paid only once Base confirms settlement\./, /too late|accepted automatically and the solver is paid/],
    [{ auto_accept: false }, /published too late for automatic acceptance: if you don’t decide by then, nothing is accepted/, /accepted automatically/],
    [null, /accept or reject it by .*\. Decide before then\.$/, /automatic/],
  ]) {
    const f = await setup({ status: 'funded' });
    if (schedule) Object.assign(f.state.review.pending_submission, schedule, { sign_by: f.state.review.pending_submission.verification_expires_at - 3600 });
    await f.go('managed_order'); await f.page.waitForSelector('#managed-review-form', { state: 'visible' });
    assert.match(await info(f.page), expected); assert.doesNotMatch(await info(f.page), absent); await f.close();
  }
});

test('a forfeited deposit and a reversed rejection read as owed and pending, never as sent or paid', async () => {
  const f = await setup({ status: 'funded' });
  Object.assign(f.state.review, { pending_submission: null, deposit: { amount_minor: 935, currency: 'mxn', status: 'forfeited_to_solver' },
    reviews: [{ document: { schema: 'agent-bounties/customer-review-v1', bounty_contract: f.state.review.bounty_contract, round: 1, submission_hash: '0x' + '1'.repeat(64),
      evidence_hash: '0x' + '2'.repeat(64), passed: false, reason: 'The delivery omits the requested benchmark table.' }, appeal: { status: 'reversed', window_ends_at: 1900000000 } }] });
  await f.go('managed_order'); await f.page.waitForFunction(() => document.getElementById('managed-review-info').textContent.includes('was found to meet'));
  const text = (await f.page.locator('#managed-review-info').innerText()) + ' ' + (await f.page.locator('#managed-review-deposit').innerText());
  assert.match(text, /The solver is paid when the bounty settles on Base, and your MXN 9\.35 review deposit is owed to them\./);
  assert.match(text, /review deposit is owed to the solver because an appeal reversed your rejection/);
  assert.doesNotMatch(text, /went to|being paid|was sent/); await f.close();
});

test('unpublished or non-HTTPS evidence is never offered as a reviewable link, and another bounty fails closed', async () => {
  let f = await setup({ status: 'funded' }); f.state.review.pending_submission.evidence = null;
  await f.go('managed_order'); await f.page.waitForFunction(() => document.getElementById('managed-review-info').textContent.includes('not published yet'));
  assert.equal(await f.page.locator('#managed-review-form').isVisible(), false); await f.close();
  f = await setup({ status: 'funded' }); f.state.review.pending_submission.evidence.artifact_reference = 'javascript:alert(1)';
  await f.go('managed_order'); await f.page.waitForSelector('#managed-review-form', { state: 'visible' });
  assert.equal(await f.page.locator('#managed-review-artifact-row').isVisible(), false);
  assert.match(await f.page.locator('#managed-review-reference').innerText(), /javascript:alert/); await f.close();
  f = await setup({ status: 'funded' }); f.state.review.bounty_contract = '0x' + 'e'.repeat(40);
  await f.go('managed_order'); await visible(f.page, 'could not safely load');
  assert.equal(await f.page.locator('#managed-review').isVisible(), false); assert.equal(f.state.verdicts.length, 0); await f.close();
});

(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); base = 'http://127.0.0.1:' + server.address().port;
  browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE, headless: true });
  try { for (const [name, fn] of tests) { await fn(); console.log('PASS ' + name); } console.log(`${tests.length} managed-service browser scenarios passed (synthetic providers only).`); }
  finally { await browser.close(); server.close(); }
})().catch(error => { console.error(error); server.close(); process.exitCode = 1; });
