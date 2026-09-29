'use strict';
const test = require('node:test'), assert = require('node:assert/strict');
const awards = require('../site/creator-awards.js');
const submissions = require('../site/submissions.js');
const registry = require('../site/data/creator-awards.json');
const copy = () => structuredClone(registry);
const paidAt = Date.parse(registry.awards[0].payment.occurred_at);
const window = (a, b) => ({ started_at: new Date(a).toISOString(), ended_at: new Date(b).toISOString() });

test('the reviewed transfer counts once as a separate award with exact six-decimal USDC', () => {
  const rows = awards.validate(copy());
  const sum = awards.summarize(rows, null, paidAt + 1);
  assert.equal(sum.count, 1);
  assert.equal(sum.amount_base_units, '15068098');
  assert.equal(awards.amount(sum.amount_base_units), '15.068098 USDC');
  assert.equal(rows[0].payment.log_index, 3);
  assert.equal(rows[0].payment.block_number, 51929073);
  assert.equal(submissions.completed(rows[0]), false, 'a direct transfer must never become a completed contract bounty');
  assert.throws(() => submissions.projection({ schema_version: 'agent-bounties/opportunity-projection-v1', network: 'base-mainnet', applied_view: 'recent', degraded: false, source_statuses: [{ source_type: 'canonical_base', available: true }], items: rows }, true));
});
test('duplicate IDs and duplicate transfer logs fail closed, even with different IDs or hash case', () => {
  for (const mutate of [row => { row.payment.log_index++; }, row => { row.id += '-copy'; row.payment.transaction_hash = row.payment.transaction_hash.toUpperCase().replace('0X', '0x'); }]) {
    const data = copy(), duplicate = structuredClone(data.awards[0]); mutate(duplicate); data.awards.push(duplicate);
    assert.throws(() => awards.validate(data), /Duplicate/);
  }
});
test('unconfirmed, private, wrong-chain/token, malformed and non-transfer records cannot enter totals', () => {
  const changes = [r => r.visibility = 'private', r => r.status = 'pending', r => r.kind = 'contract_settlement', r => r.payment.chain_id = 1,
    r => r.payment.token = '0x' + '1'.repeat(40), r => r.payment.receipt_status = 'failed', r => r.payment.event = 'BountySettled',
    r => r.payment.transaction_hash = 'pending', r => r.payment.to_wallet = 'not-an-address', r => r.payment.log_index = -1,
    r => r.payment.block_number = 0, r => r.payment.amount_base_units = 15068098, r => r.payment.amount_base_units = '15.068098',
    r => r.payment.amount_base_units = '-1', r => r.payment.amount_base_units = '0', r => r.payment.occurred_at = 'unknown',
    r => r.reviewed_at = '2026-09-01T00:00:00Z'];
  changes.forEach(change => { const data = copy(); change(data.awards[0]); assert.throws(() => awards.validate(data)); });
  assert.throws(() => awards.validate({ awards: [] }));
});
test('reporting windows include the start, exclude the end, and exclude future payments', () => {
  const rows = awards.validate(copy());
  assert.equal(awards.summarize(rows, window(paidAt, paidAt + 1), paidAt).count, 1);
  assert.equal(awards.summarize(rows, window(paidAt - 1, paidAt), paidAt).count, 0);
  assert.equal(awards.summarize(rows, null, paidAt - 1).count, 0);
  assert.equal(awards.summarize(rows, window(paidAt + 1, paidAt + 1000), paidAt + 1000).amount_base_units, '0');
  assert.throws(() => awards.summarize(rows, { started_at: 'bad', ended_at: 'bad' }));
  assert.throws(() => awards.summarize(rows, window(paidAt, paidAt)));
});
test('large and fractional totals use integer base units without rounding losses', () => {
  const rows = awards.validate(copy());
  rows[0].payment.amount_base_units = '9007199254740993';
  assert.equal(awards.amount(awards.summarize(rows, null, paidAt + 1).amount_base_units), '9007199254.740993 USDC');
});
test('only the two explicitly public CAD contract identities link to the award', () => {
  const rows = awards.validate(copy());
  for (const address of rows[0].bounty_contracts) assert.equal(awards.related(rows, `canonical:base-mainnet:${address}`).length, 1);
  for (const id of ['canonical:base-mainnet:0x' + '1'.repeat(40), `open-competition:base-mainnet:${rows[0].bounty_contracts[1]}`, `canonical:base-sepolia:${rows[0].bounty_contracts[1]}`, rows[0].bounty_contracts[1], null]) assert.equal(awards.related(rows, id).length, 0);
});
test('the public result preserves A’s passing work, B’s exact files, review expiry responsibility and payment limits', () => {
  const row = awards.validate(copy())[0], html = awards.renderDetail(row);
  for (const phrase of ['A also passed', 'that was our responsibility', 'not a verified bonded contract submission', 'not an on-chain rejection', '15.068098 USDC', 'View Winning Submission', 'No formal contract submission passed', 'not yet proven', row.winner.revision, row.payment.transaction_hash]) assert.ok(html.includes(phrase), phrase);
  assert.equal(row.reviews.length, 11);
  assert.equal(row.criteria.length, 3);
  assert.ok(html.includes('200 mm instead of 350 mm'));
  assert.ok(html.includes('55 mm instead of 45 mm'));
});
test('untrusted text and URLs in review data cannot create HTML or script links', () => {
  const row = awards.validate(copy())[0]; row.title = '<script>bad()</script>'; row.winner.source_url = 'javascript:bad()'; row.winner.image = '../private.png';
  const html = awards.renderDetail(row);
  assert.ok(html.includes('&lt;script&gt;'));
  assert.doesNotMatch(html, /<script>|javascript:|private.png/);
});

function harness(mode = 'summary') {
  const handlers = {}, host = { dataset: { directAwards: mode }, innerHTML: '', textContent: '', hidden: false };
  const controls = Object.fromEntries(['market-timing', 'market-kind', 'market-search', 'market-refresh', 'submissions-refresh'].map(key => [key, { value: key === 'market-timing' ? 'now' : key === 'market-kind' ? 'all' : '', addEventListener: (event, fn) => handlers[`${key}:${event}`] = fn }]));
  const doc = { querySelectorAll: () => [host], querySelector: selector => selector === '[data-direct-awards="metrics"]' ? host : controls[selector.slice(6, -1)] || null };
  const win = { location: { search: '' }, fetch: async () => ({ ok: true, json: async () => copy() }) };
  return { host, controls, handlers, doc, win };
}
const tick = () => new Promise(resolve => setImmediate(resolve));
test('one shared load serves summary and metrics; date changes use the selected canonical window', async () => {
  const h = harness('summary'); let reads = 0;
  h.win.fetch = async (_, options) => { reads++; assert.equal(options.credentials, 'omit'); return { ok: true, json: async () => copy() }; };
  awards.start(h.win, h.doc); await tick();
  assert.match(h.host.innerHTML, /1 paid award · 15.068098 USDC/);
  h.host.dataset.directAwards = 'metrics';
  awards.setWindow(h.doc, window(paidAt - 1000, paidAt + 1000));
  assert.match(h.host.innerHTML, /1 paid award · 15.068098 USDC/);
  assert.match(h.host.innerHTML, /selected period/);
  awards.setWindow(h.doc, window(paidAt - 1000, paidAt));
  assert.match(h.host.innerHTML, /0 paid awards · 0 USDC/);
  awards.setWindow(h.doc, null);
  assert.match(h.host.innerHTML, /dates are unavailable/);
  assert.equal(reads, 1);
});
test('past-work cards obey availability, search and work-type filters without entering open inventory', async () => {
  const h = harness('board'); awards.start(h.win, h.doc); await tick();
  assert.equal(h.host.hidden, true);
  h.controls['market-timing'].value = 'completed'; h.handlers['market-timing:change']();
  assert.equal(h.host.hidden, false); assert.match(h.host.innerHTML, /View Submissions/);
  h.controls['market-kind'].value = 'competition'; h.handlers['market-kind:change']();
  assert.doesNotMatch(h.host.innerHTML, /View Submissions/);
  h.controls['market-kind'].value = 'all'; h.controls['market-search'].value = 'tinaco'; h.handlers['market-search:input']();
  assert.match(h.host.innerHTML, /View Submissions/);
  h.controls['market-search'].value = 'unrelated'; h.handlers['market-search:input']();
  assert.doesNotMatch(h.host.innerHTML, /View Submissions/);
});
test('load failures never turn into zero paid awards, and a user can retry', async () => {
  // A fresh instance isolates a failed request from the previously verified cache.
  delete require.cache[require.resolve('../site/creator-awards.js')]; const fresh = require('../site/creator-awards.js');
  const h = harness(); h.win.fetch = async () => { throw new Error('offline'); };
  fresh.start(h.win, h.doc); await tick();
  assert.match(h.host.textContent, /could not load/); assert.doesNotMatch(h.host.innerHTML, /0 paid/);
  h.win.fetch = async () => ({ ok: true, json: async () => copy() });
  await h.handlers['market-refresh:click']();
  assert.match(h.host.innerHTML, /1 paid award/);
});
test('the submissions route opens the paid public award and rejects an unknown award ID', async () => {
  const page = { innerHTML: '', setAttribute() {} }, refresh = {}, status = {};
  const doc = { querySelector: selector => selector.includes('page') ? page : selector.includes('refresh') ? refresh : status };
  refresh.addEventListener = () => {};
  const win = { location: { search: '?award=cad-rainwater-2026' }, AgentBountiesAwards: { ...awards, load: async () => awards.validate(copy()) } };
  await submissions.start(win, doc);
  assert.match(page.innerHTML, /View Winning Submission/);
  assert.match(status.textContent, /Separate creator award/);
  assert.equal(refresh.disabled, false);
  win.location.search = '?award=unknown'; await submissions.start(win, doc);
  assert.match(status.textContent, /not available/); assert.doesNotMatch(page.innerHTML, /Winning/);
});
