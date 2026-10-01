const test = require('node:test'), assert = require('node:assert/strict'), fs = require('node:fs'), vm = require('node:vm');
const core = require('../site/creator-open-core.js');
const scope = { window: {}, TextEncoder, crypto: require('node:crypto').webcrypto, Uint8Array, BigInt };
vm.runInNewContext(fs.readFileSync(require.resolve('../site/evm.js'), 'utf8'), scope);
const evm = scope.window.AgentBountiesEvm;
const vectors = require('../site/fixtures/creator-open-preparations.json');
const copy = value => JSON.parse(JSON.stringify(value));
for (const p of vectors.plans) test(`browser independently matches Rust authority: ${p.request.action.kind}`, () => {
  const local = core.validatePrepared(p, vectors.release, p.request, evm, vectors.now);
  assert.equal(local.bounty, p.bounty_contract);
  assert.equal(local.approval_hash, p.approval_hash);
  assert.equal(local.principal, p.customer_principal_micro_usdc);
});
test('rejects each changed typed message field, domain and advertised amount before a wallet request', () => {
  for (const p of vectors.plans) {
    for (const key of ['token_authorization', 'action_authorization']) {
      if (!p[key]) continue;
      for (const part of ['domain', 'message']) for (const field of Object.keys(p[key].typed_data[part])) {
        const altered = copy(p), value = altered[key].typed_data[part][field];
        altered[key].typed_data[part][field] = typeof value === 'boolean' ? !value : 'altered';
        assert.throws(() => core.validatePrepared(altered, vectors.release, p.request, evm, vectors.now), `${key}.${part}.${field}`);
      }
      const altered = copy(p); altered[key].typed_data.types[altered[key].typed_data.primaryType].push({ name: 'extra', type: 'address' });
      assert.throws(() => core.validatePrepared(altered, vectors.release, p.request, evm, vectors.now));
    }
    for (const field of ['approval_hash', 'bounty_contract', 'customer_principal_micro_usdc', 'customer_gas_wei', 'network']) {
      const altered = copy(p); altered[field] = 'changed';
      assert.throws(() => core.validatePrepared(altered, vectors.release, p.request, evm, vectors.now));
    }
  }
});
test('rejects changed evidence, non-HTTPS links and credential-bearing links', () => {
  const p = vectors.plans.find(p => p.request.action.kind === 'submit');
  const r = copy(p.request); r.public_evidence.notes += ' Changed';
  assert.throws(() => core.expected(vectors.release, r, evm, vectors.now));
  for (const url of ['javascript:alert(1)', 'http://example.org', 'https://user:secret@example.org']) {
    assert.throws(() => core.evidence({ ...r.public_evidence, artifact_url: url }, evm));
  }
});
test('expired signatures permit local recovery inspection but never a fresh signing review', () => {
  const p = vectors.plans[0], later = p.request.valid_before + 1;
  assert.throws(() => core.expected(vectors.release, p.request, evm, later));
  assert.equal(core.expected(vectors.release, p.request, evm, later, true).approval_hash, p.approval_hash);
  assert.throws(() => core.expected(vectors.release, p.request, evm, p.request.valid_before - 7201));
});
test('money uses exact six-decimal arithmetic and rejects ambiguous or negative input', () => {
  assert.equal(core.units('20.30'), 20300000); assert.equal(core.units('0.000001'), 1); assert.equal(core.money('21300000'), '21.3 USDC');
  for (const invalid of ['', '1e6', '-1', '0', '0.0000001', '01.00', '1,000', 'NaN', '9999999999.999999']) assert.throws(() => core.units(invalid));
});
test('the workspace has no wallet transaction, raw signing or paid fallback path', () => {
  const source = fs.readFileSync(require.resolve('../site/creator-open-workspace.js'), 'utf8');
  assert.doesNotMatch(source, /eth_sendTransaction|wallet_sendCalls|eth_sendRawTransaction|personal_sign|eth_sign["']/);
  assert.match(source, /eth_signTypedData_v4/); assert.match(source, /event\.isTrusted/);
});

test('action blockers explain creator, queue, deadline and refund constraints', () => {
  const creator = '0x' + '06'.repeat(20), solver = '0x' + '09'.repeat(20), now = 1000;
  const item = { terms: { creator, submission_deadline: 2000, review_window_seconds: 3600, max_pending_entries: 1 }, status: 'open', entry_count: 0, entries: [], principal_micro_usdc: '21000000' };
  assert.equal(core.actionBlocker(item, 'submit', solver, now), null);
  assert.match(core.actionBlocker(item, 'submit', creator, now), /creator cannot/);
  assert.match(core.actionBlocker(item, 'increase', solver, now), /creator wallet/);
  assert.match(core.actionBlocker(item, 'expire', creator, now), /full review window/);
  assert.match(core.actionBlocker(item, 'refund_principal', creator, now), /Close/);
  const started = { ...item, entry_count: 1, entries: [{ id: 1, entry: { status: 'pending', solver, review_deadline: 1500 } }] };
  assert.match(core.actionBlocker(started, 'submit', solver, now), /existing submission/);
  assert.match(core.actionBlocker(started, 'cancel', creator, now), /first submission/);
  assert.match(core.actionBlocker(started, 'refund_bond', solver, now), /No pending bond/);
  assert.equal(core.actionBlocker(started, 'review', creator, now), null);
  assert.equal(core.actionBlocker(started, 'refund_bond', solver, 1501), null);
  assert.equal(core.actionBlocker({ ...started, status: 'cancelled' }, 'refund_principal', creator, now), null);
});
