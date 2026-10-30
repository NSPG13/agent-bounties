const test = require('node:test');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const core = require('../site/managed-service-core.js');
const fixture = require('../site/fixtures/managed-service-quote.json');
const account = 'a'.repeat(64), id = fixture.quote.input.quote_id;
const copy = () => structuredClone(fixture);
const hash = q => createHash('sha256').update('agent-bounties:managed-service-quote:v2\0' + JSON.stringify(q)).digest('hex');

test('shared Rust/browser fixture preserves reward, itemized MXN total and Unicode digest', async () => {
  const q = await core.quote(fixture.quote, fixture.quote_digest, account, id);
  assert.equal(hash(q), fixture.quote_digest);
  assert.equal(q.input.solver_reward_micro_usdc, 100000000);
  assert.equal(q.platform_fee_micro_usdc, 7500000);
  assert.equal(core.money(q.total_minor, 'mxn'), 'MXN 2,173.75');
  assert.equal(core.money(Number.MAX_SAFE_INTEGER, 'usd'), 'USD 90,071,992,547,409.91');
});
test('JSON property reordering cannot change the accepted quote digest', async () => {
  const reorder = v => Array.isArray(v) ? v.map(reorder) : v && typeof v === 'object' ? Object.fromEntries(Object.entries(v).reverse().map(([k, value]) => [k, reorder(value)])) : v;
  assert.deepEqual(await core.quote(reorder(fixture.quote), fixture.quote_digest, account, id), fixture.quote);
});
test('fee and fiat lines round upward independently without subtracting the reward', async () => {
  const { quote: q } = copy();
  q.input.solver_reward_micro_usdc++; q.input.verifier_reserve_micro_usdc++;
  q.platform_fee_micro_usdc++; q.bounty_target_micro_usdc += 3;
  for (const line of q.lines.slice(0, 3)) { line.amount_micro_usdc++; line.amount_minor++; }
  q.total_minor += 3;
  assert.equal((await core.quote(q, hash(q), account)).total_minor, 217378);
});
test('wrong owner, IDs, digest, extra fields and altered amounts fail closed', async () => {
  await assert.rejects(core.quote(fixture.quote, fixture.quote_digest, 'b'.repeat(64), id));
  await assert.rejects(core.quote(fixture.quote, fixture.quote_digest, account, '22222222-2222-4222-8222-222222222222'));
  await assert.rejects(core.quote(fixture.quote, '0'.repeat(64), account, id));
  for (const change of [
    q => { q.total_minor++; }, q => { q.platform_fee_bps = 700; },
    q => { q.input.solver_reward_micro_usdc--; }, q => { q.lines[0].amount_minor--; },
    q => { q.input.livemode = 'true'; }, q => { q.input.currency = 'eur'; },
    q => { q.input.price_basis.minor_units_per_usdc_denominator = 0; },
    q => { q.input.terms.delivery_deadline = q.input.expires_at; },
    q => { q.input.terms.deliverable = '\ud800'; }, q => { q.extra = 'unapproved'; },
    q => { q.input.additional_costs[0].code = 'solver_reward'; },
    q => { q.lines[0].label = 'Secret extra'; }, q => { q.total_minor = Number.MAX_SAFE_INTEGER + 1; },
    q => { q.input.price_basis.valid_until--; }
  ]) {
    const { quote: q } = copy(); change(q);
    await assert.rejects(core.quote(q, hash(q), account, id));
  }
});
test('target accepts one non-nil UUID, strips untrusted return fields and rejects ambiguity', () => {
  assert.deepEqual(core.target('?managed_order=' + id + '&checkout_session_id=cs_fake&paid=true#evil'), { kind: 'managed_order', id, path: '/account.html?managed_order=' + id });
  for (const search of ['', '?managed_quote=evil', '?managed_order=00000000-0000-0000-0000-000000000000', `?managed_quote=${id}&managed_order=${id}`, `?managed_order=${id}&managed_order=${id}`]) assert.throws(() => core.target(search));
});
test('API origin and sign-in identity cannot be redirected or coerced', () => {
  assert.equal(core.apiOrigin(new URL('https://agentizable.ai')), 'https://api.agentizable.ai');
  assert.equal(core.apiOrigin(new URL('http://127.0.0.1:18680')), 'http://127.0.0.1:3000');
  for (const url of ['https://evil.example', 'https://agentizable.ai.evil.example', 'file:///tmp/page', 'http://agentizable.ai']) assert.throws(() => core.apiOrigin(new URL(url)));
  assert.equal(core.session({ authenticated: false }), null);
  assert.equal(core.session({ authenticated: true, user: { id: account } }), account);
  for (const value of [{}, { authenticated: 'true' }, { authenticated: true, user: { id: account.toUpperCase() } }, { authenticated: true, user: { id: 123 } }, { authenticated: true, user: { id: [account] } }]) assert.throws(() => core.session(value));
});
test('retries preserve one quote/digest-bound idempotency key across refreshes', () => {
  const key = core.acceptanceKey(fixture.quote, fixture.quote_digest);
  assert.equal(key, core.acceptanceKey(copy().quote, fixture.quote_digest));
  assert.match(key, /^[a-zA-Z0-9_.:-]{1,200}$/);
  assert.notEqual(key, core.acceptanceKey(fixture.quote, '0'.repeat(64)));
});
test('checkout handoff rejects alternate schemes, userinfo, deceptive hosts and paths', () => {
  assert.equal(core.checkoutURL('https://checkout.stripe.com/c/pay/cs_test_example#fid'), 'https://checkout.stripe.com/c/pay/cs_test_example#fid');
  for (const url of ['javascript:alert(1)', 'http://checkout.stripe.com/c/pay/x', 'https://checkout.stripe.com.evil.example/c/pay/x', 'https://checkout.stripe.com@evil.example/c/pay/x', 'https://evil@checkout.stripe.com/c/pay/x', 'https://checkout.stripe.com/c/pay/', 'https://checkout.stripe.com/other', 'https://checkout.stripe.com/c/pay/../evil', 'https://checkout.stripe.com/c/pay/\nx', 'https://checkout.stripe.com\\evil.example/c/pay/x']) assert.throws(() => core.checkoutURL(url));
});
test('only defined owner order states are displayed; paid/payout return assertions are rejected', async () => {
  const order = { order_id: '22222222-2222-4222-8222-222222222222', quote: fixture.quote, quote_digest: fixture.quote_digest, accepted_at: 1800000100, status: 'awaiting_payment' };
  assert.equal((await core.order(order, account, order.order_id, fixture.quote_digest)).status, 'awaiting_payment');
  for (const state of ['refund_requested', 'refund_pending', 'refund_action_required', 'refund_sent', 'refund_failed',
    'funding_reserved', 'funding_broadcast', 'funded', 'funding_reconciliation_required']) assert.equal((await core.order({ ...order, status: state }, account)).status, state);
  for (const state of ['paid', 'paid_out', 'settled', 'succeeded', 'cancelled', undefined]) await assert.rejects(core.order({ ...order, status: state }, account));
  await assert.rejects(core.order(order, account, id));
  await assert.rejects(core.order({ ...order, accepted_at: fixture.quote.input.expires_at }, account));
});
test('bounty commitment binds payment mode to network and every field to the digest', async () => {
  for (const change of [
    q => { q.input.bounty.network = 'base-mainnet'; },
    q => { q.input.livemode = true; },
    q => { q.input.bounty.factory = q.input.bounty.funding_wallet; },
    q => { q.input.bounty.predicted_bounty = '0x' + '0'.repeat(40); },
    q => { q.input.bounty.terms_hash = '0x' + 'AB'.repeat(32); },
    q => { q.input.bounty.create_call_keccak256 = '0x' + 'ca'.repeat(31); },
    q => { q.input.bounty.funding_deadline = q.input.expires_at; },
    q => { q.input.bounty.funding_deadline = q.input.terms.delivery_deadline + 1; },
    q => { q.input.bounty.verification_setup = 'bad setup'; },
    q => { q.input.bounty.extra = 'unapproved'; },
    q => { delete q.input.bounty; }
  ]) {
    const { quote: q } = copy();
    change(q);
    await assert.rejects(core.quote(q, hash(q), account, id));
  }
  // A well-formed but different bounty under the original digest is rejected.
  const { quote: swapped } = copy();
  swapped.input.bounty.predicted_bounty = '0x' + 'b1'.repeat(20);
  await assert.rejects(core.quote(swapped, fixture.quote_digest, account, id));
});

test('review replies must name the committed bounty and verdicts must be exact and reasoned', () => {
  const bounty = fixture.quote.input.bounty.predicted_bounty;
  const reply = { order_id: '22222222-2222-4222-8222-222222222222', network: 'base-sepolia', bounty_contract: bounty, bounty_status: 'submitted', reviews: [],
    pending_submission: { round: 1, solver: '0x' + '5'.repeat(40), submission_hash: '0x' + '1'.repeat(64), evidence_hash: '0x' + '2'.repeat(64),
      verification_expires_at: 1900000000, decide_by: 1899740800, evidence: { artifact_reference: 'https://example.com/a.txt', artifact_hash: 'x', evidence: {} } },
    deposit: { amount_minor: 935, currency: 'mxn', status: 'held' } };
  const view = core.review(reply, bounty);
  assert.equal(view.pending.artifact, 'https://example.com/a.txt');
  assert.throws(() => core.review({ ...reply, bounty_contract: '0x' + 'e'.repeat(40) }, bounty));
  assert.throws(() => core.review({ ...reply, extra: true }, bounty));
  assert.throws(() => core.review({ ...reply, pending_submission: { ...reply.pending_submission, submission_hash: '0xAB' } }, bounty));
  assert.equal(core.verdict(view.pending, true, '').reason, '');
  assert.throws(() => core.verdict(view.pending, false, 'Too short'));
  assert.throws(() => core.verdict(view.pending, false, ' leading space and long enough to pass'));
  assert.throws(() => core.verdict({ ...view.pending, published: false }, true, ''));
  assert.equal(core.artifactURL('http://example.com/a'), null);
  const rejected = { document: { schema: 'agent-bounties/customer-review-v1', bounty_contract: bounty, round: 1, submission_hash: '0x' + '1'.repeat(64),
    evidence_hash: '0x' + '2'.repeat(64), passed: false, reason: 'The report omits the requested benchmark.' }, appeal: { status: 'appealed', window_ends_at: 1899900000 } };
  assert.equal(core.review({ ...reply, reviews: [rejected] }, bounty).reviews[0].appeal.status, 'appealed');
  for (const bad of [{ ...rejected, appeal: { status: 'settled', window_ends_at: 1 } }, { ...rejected, document: { ...rejected.document, passed: true } }]) {
    assert.throws(() => core.review({ ...reply, reviews: [bad] }, bounty));
  }
  assert.throws(() => core.review({ ...reply, deposit: { amount_minor: 935, currency: 'mxn', status: 'kept' } }, bounty));
  assert.throws(() => core.review({ ...reply, pending_submission: { ...reply.pending_submission, decide_by: 1900000001 } }, bounty));
  // Older replies carry no schedule fields: eligibility is unknown, never assumed.
  assert.equal(view.pending.auto_accept, null);
  const scheduled = { ...reply.pending_submission, auto_accept: false, sign_by: 1899996400 };
  assert.deepEqual([core.review({ ...reply, pending_submission: scheduled }, bounty).pending.auto_accept, core.review({ ...reply, pending_submission: scheduled }, bounty).pending.sign_by], [false, 1899996400]);
  for (const bad of [{ auto_accept: false }, { sign_by: 1899996400 }, { auto_accept: 1, sign_by: 1899996400 }, { auto_accept: true, sign_by: 1899740800 },
    { auto_accept: true, sign_by: 1900000000 }, { auto_accept: true, sign_by: 1899996400, extra: 1 }]) {
    assert.throws(() => core.review({ ...reply, pending_submission: { ...reply.pending_submission, ...bad } }, bounty));
  }
});

test('a review deposit line must be exactly 10% of the solver reward at the quoted rate', async () => {
  const q = structuredClone(fixture.quote), n = BigInt(q.input.price_basis.minor_units_per_usdc_numerator), d = BigInt(q.input.price_basis.minor_units_per_usdc_denominator) * 1000000n;
  const deposit = Number(((BigInt(q.input.solver_reward_micro_usdc) * 1000n + 9999n) / 10000n * n + d - 1n) / d);
  const withDeposit = amount => {
    const v = structuredClone(q);
    v.input.additional_costs = [...v.input.additional_costs, { code: 'review_deposit', label: 'Review deposit (refundable, 10% of solver reward)', amount_minor: amount }];
    v.lines = [...v.lines, { code: 'review_deposit', label: 'Review deposit (refundable, 10% of solver reward)', amount_minor: amount, amount_micro_usdc: null }];
    v.total_minor += amount;
    return v;
  };
  const sign = v => require('node:crypto').createHash('sha256').update('agent-bounties:managed-service-quote:v2\0' + JSON.stringify(v)).digest('hex');
  const ok = withDeposit(deposit);
  assert.equal((await core.quote(ok, sign(ok), account)).lines.at(-1).amount_minor, deposit);
  const bad = withDeposit(deposit - 1);
  await assert.rejects(core.quote(bad, sign(bad), account));
});


test('card preparation input admits fractional rewards while bounding public data and budget', () => {
  const i = { document: { title: 'Check the site', goal: 'Publish a report.', acceptance_criteria: ['Show its URL.'],
    benchmark: { engine: 'customer_review_v1', delivery_deadline: 1900000000 }, evidence_schema: { type: 'object' } },
    solver_reward_micro_usdc: 178000, currency: 'mxn', maximum_charge_minor: 1000 };
  const body = core.cardQuoteRequest(i); assert.deepEqual(Object.keys(body), ['document','solver_reward_micro_usdc']);
  assert.notEqual(body.document,i.document); assert.equal(body.solver_reward_micro_usdc,178000);
  for (const change of [v => { v.currency='eur'; },v => { v.maximum_charge_minor=0; },v => { v.solver_reward_micro_usdc=1.2; },
    v => { v.document.private_key='secret'; },v => { v.document.benchmark.engine='other'; },v => { v.document.evidence_schema=null; },
    v => { v.document.goal='x'.repeat(4001); },v => { v.document.image={ text:'x'.repeat(60001) }; }]) {
    const value=structuredClone(i); change(value); assert.throws(() => core.cardQuoteRequest(value));
  }
});
