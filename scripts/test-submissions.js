'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const history = require('../site/submissions.js');
const market = require('../site/marketplace.js');
const original = require('./fixtures/funded-bounty.cjs').opportunity;
const hash = char => '0x' + char.repeat(64);
const wallet = '0x' + 'b'.repeat(40), bountyId = hash('c');
const item = { ...original, opportunity_id: `canonical:base-mainnet:${original.source_id}`, work_state: 'completed', payment_state: 'paid', source_status: 'paid', payment_committed: true, updated_at: '2026-09-22T00:00:00Z' };
function event(kind, overrides = {}) {
  return { id: kind, tx_hash: hash('a'), block_number: 10, log_index: kind === 'bounty_settled' ? 2 : 1, contract_address: item.source_id, bounty_id: bountyId, kind, occurred_at: item.updated_at,
    data: { round: 1, solver: wallet, submission_hash: hash('d'), evidence_hash: hash('e') }, ...overrides };
}
function payload(items = [item]) { return { schema_version: 'agent-bounties/opportunity-projection-v1', network: 'base-mainnet', applied_view: 'recent', degraded: false, source_statuses: [{ source_type: 'canonical_base', available: true }], items }; }
test('completed history uses its own public paid projection and cannot relax ready-work filters', async () => {
  let url;
  const win = { location: { hostname: 'agentbounties.app' }, fetch: async (input, options) => { url = input; assert.equal(options.credentials, 'omit'); return { ok: true, json: async () => payload() }; } };
  assert.deepEqual((await history.loadCompleted(win)).items, [item]);
  assert.match(url, /work_state=completed&payment_state=paid/);
  assert.equal(market.isReadyToEarn(item), false);
  assert.equal(market.filterItems([item], '', 'now').length, 0);
  assert.equal(market.filterItems([item], '', 'completed').length, 1);
  for (const bad of [{ ...item, source_type: 'legacy_bounty' }, { ...item, payment_state: 'escrowed' }, { ...item, network: 'base-sepolia' }, { ...item, source_status: 'cancelled' }]) assert.throws(() => history.projection(payload([bad]), true));
  assert.throws(() => history.projection({ ...payload(), degraded: true }, true));
  const card = market.renderOpportunity(item, 0, Date.now());
  assert.match(card, /View Submissions/); assert.doesNotMatch(card, /Continue to proof|before claiming|If you win/);
  assert.match(market.renderOpportunity(original, 0, Date.now()), /View Submissions/);
});
test('a winner requires a scoped confirmed settlement matching round, solver and both hashes', () => {
  const submitted = event('submission_added'), settled = event('bounty_settled');
  assert.equal(history.buildHistory(item, [submitted, settled], bountyId).winner.number, 1);
  assert.equal(history.buildHistory(item, [submitted], bountyId).winner, null);
  for (const data of [{ round: 2 }, { solver: '0x' + 'f'.repeat(40) }, { submission_hash: hash('f') }, { evidence_hash: hash('f') }]) {
    assert.equal(history.buildHistory(item, [submitted, { ...settled, data: { ...settled.data, ...data } }], bountyId).winner, null);
  }
  for (const values of [{ contract_address: wallet }, { bounty_id: hash('f') }, { tx_hash: 'unconfirmed' }, { block_number: null }, { log_index: 0 }]) {
    assert.equal(history.buildHistory(item, [submitted, { ...settled, ...values }], bountyId).winner, null);
  }
  assert.equal(history.buildHistory(item, [submitted, settled, { ...settled, tx_hash: hash('f') }], bountyId).winner, null);
});
test('V1 and V2 competitions match the selected sequence and never treat qualification as winning', () => {
  for (const [prefix, version, kind, settleKind, entryKey, winnerKey] of [
    ['open-competition', 'open-competition-v1', 'solution_revealed', 'bounty_settled', 'submission_sequence', 'submission_sequence'],
    ['open-competition-v2', 'open-competition-v2-beta3', 'entry_qualified', 'competition_settled', 'sequence', 'winning_sequence']]) {
    const competition = { ...item, opportunity_id: `${prefix}:base-mainnet:${item.source_id}` };
    const submitted = event(kind, { protocol_version: `agent-bounties/${version}`, data: { passed: true, solver: wallet, submission_hash: hash('d'), evidence_hash: hash('e'), [entryKey]: 3 } });
    const settled = event(settleKind, { log_index: 2, protocol_version: submitted.protocol_version, data: { ...submitted.data, [winnerKey]: 3 } });
    delete settled.data[entryKey]; settled.data[winnerKey] = 3;
    assert.equal(history.buildHistory(competition, [submitted], bountyId).winner, null);
    assert.equal(history.buildHistory(competition, [submitted, settled], bountyId).winner.number, 3);
    submitted.data.passed = false;
    assert.equal(history.buildHistory(competition, [submitted, settled], bountyId).winner, null);
    submitted.data.passed = true;
    settled.data[winnerKey] = 2;
    assert.equal(history.buildHistory(competition, [submitted, settled], bountyId).winner, null);
    submitted.protocol_version = 'wrong-protocol';
    assert.equal(history.buildHistory(competition, [submitted], bountyId).entries.length, 0);
  }
});
test('published files must match the exact submission; unavailable records are not empty history', async () => {
  const submission = event('submission_added');
  const entry = history.buildHistory(item, [submission, event('bounty_settled')], bountyId).winner;
  const record = { network: 'base-mainnet', bounty_contract: item.source_id, bounty_id: bountyId, round: 1, solver_wallet: wallet, artifact_hash: hash('d'), evidence_hash: hash('e'), artifact_reference: 'https://github.com/example/project/commit/abc', evidence: { test: 'passed' } };
  assert.equal(history.matchingEvidence(item, entry, record), true);
  for (const values of [{ round: 2 }, { artifact_hash: hash('f') }, { network: 'base-sepolia' }, { solver_wallet: item.source_id }, { bounty_id: hash('f') }]) assert.equal(history.matchingEvidence(item, entry, { ...record, ...values }), false);
  const win = { location: { hostname: 'agentbounties.app' }, fetch: async url => url.includes('submission-evidence') ? { ok: false, status: 503 } : { ok: true, json: async () => [{ bounty_contract: item.source_id, bounty_id: bountyId, terms_valid: true, terms_hash: item.terms_hash, terms: { document: { acceptance_criteria: ['One check'] } }, events: [submission, event('bounty_settled')] }] } };
  const data = await history.loadHistory(win, item);
  assert.equal(data.entries.length, 1); assert.equal(data.winner.evidenceState, 'unavailable');
  assert.match(history.renderHistory(data), /could not load/);
});
test('rendered work and criteria cannot inject markup or claim an unwritten review', () => {
  const data = { item: { ...item, title: '<img src=x onerror=alert(1)>' }, terms: { acceptance_criteria: ['<script>bad()</script>'] }, ...history.buildHistory(item, [event('submission_added'), event('bounty_settled')], bountyId) };
  data.winner.evidence = { artifact_reference: 'javascript:alert(1)', evidence: { text: '</pre><img src=x>' } };
  const html = history.renderHistory(data);
  assert.doesNotMatch(html, /<script>|<img|href="javascript:/);
  assert.match(html, /View Winning Submission/); assert.match(html, /Why this won/);
  assert.match(html, /written review for each criterion is not available/);
  assert.match(html, /Evidence supplied by the solver/);
  assert.equal(history.safeUrl('https://user:secret@example.com'), null);
});
test('invalid old terms do not erase confirmed submissions or present unverified criteria', async () => {
  const win = { location: { hostname: 'agentbounties.app' }, fetch: async url => url.includes('submission-evidence') ? { ok: false, status: 404 } : { ok: true, json: async () => [{ bounty_contract: item.source_id, bounty_id: bountyId, terms_valid: false, terms_hash: item.terms_hash, terms: { document: { acceptance_criteria: ['Do not publish this unverified text'] } }, events: [event('submission_added'), event('bounty_settled')] }] } };
  const data = await history.loadHistory(win, item);
  assert.equal(data.winner.number, 1); assert.equal(data.terms, null);
  const html = history.renderHistory(data);
  assert.match(html, /original criteria could not be verified/);
  assert.doesNotMatch(html, /Do not publish this unverified text|Original test and review rules/);
});
test('submission links use exact identity lookup before the API limit', async () => {
  let responseItems = [item], url;
  const win = { location: { hostname: 'agentbounties.app' }, fetch: async input => { url = new URL(input); return { ok: true, json: async () => payload(responseItems) }; } };
  assert.equal((await history.loadOpportunity(win, item.opportunity_id)).opportunity_id, item.opportunity_id);
  assert.equal(url.searchParams.get('opportunity_id'), item.opportunity_id);
  assert.equal(url.searchParams.get('limit'), '1');
  responseItems = [];
  await assert.rejects(history.loadOpportunity(win, item.opportunity_id), /not available in the public history/);
  responseItems = [item, item];
  await assert.rejects(history.loadOpportunity(win, item.opportunity_id), /lookup is unavailable/);
  responseItems = [{ ...item, source_id: wallet, opportunity_id: `canonical:base-mainnet:${wallet}` }];
  await assert.rejects(history.loadOpportunity(win, item.opportunity_id), /lookup is unavailable/);
});
