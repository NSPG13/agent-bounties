import assert from 'node:assert/strict';
import test from 'node:test';
import { AgentBountiesClient, AgentBountiesHttpError } from '../dist/index.js';

test('managed service helpers preserve caller cookies, exact acceptance, and server failures', async () => {
  const original = globalThis.fetch;
  const calls = [];
  let failure = false;
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(failure
      ? { error: 'reconciliation_required' }
      : { status: 'awaiting_payment' }), { status: failure ? 409 : 200 });
  };
  try {
    const client = new AgentBountiesClient({ baseUrl: 'https://api.example', credentials: 'include' });
    const id = '00000000-0000-4000-8000-000000000001';
    const digest = 'a'.repeat(64);
    await client.readManagedServiceQuote(id);
    await client.acceptManagedServiceQuote(id, digest, 'accepted-key');
    await client.getManagedServiceOrder(id);
    await client.openManagedServiceCheckout(id);
    await client.requestManagedServiceRefund(id);
    assert.deepEqual(JSON.parse(calls[1].init.body), {
      quote_digest: digest, idempotency_key: 'accepted-key',
    });
    assert.deepEqual(JSON.parse(calls[3].init.body), {});
    assert.deepEqual(JSON.parse(calls[4].init.body), {});
    assert.equal(calls[4].url, 'https://api.example/v1/managed-service/orders/' + id + '/refund');
    for (const call of calls) {
      assert.equal(call.init.credentials, 'include');
      assert.equal(call.init.redirect, 'error');
      assert.equal(call.init.headers['x-operator-token'], undefined);
    }
    await assert.rejects(client.getManagedServiceOrder('../stripe/connect-transfers'), TypeError);
    failure = true;
    await assert.rejects(client.openManagedServiceCheckout(id), AgentBountiesHttpError);
    assert.equal(calls.length, 6); // Never retries with a new order or retry key.
  } finally {
    globalThis.fetch = original;
  }
});

test('card bounty quotes send only the draft and a positive integer reward', async () => {
  const original = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return new Response('{}', { status: 200 }); };
  try {
    const client = new AgentBountiesClient({ baseUrl: 'https://api.example', credentials: 'include' });
    const document = { title: 'Fixture', benchmark: { engine: 'customer_review_v1', delivery_deadline: 1 } };
    await client.issueManagedBountyQuote(document, 10_000_000);
    assert.equal(calls[0].url, 'https://api.example/v1/managed-service/bounty-quotes');
    assert.deepEqual(JSON.parse(calls[0].init.body), { document, solver_reward_micro_usdc: 10_000_000 });
    assert.equal(calls[0].init.credentials, 'include');
    for (const bad of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      await assert.rejects(client.issueManagedBountyQuote(document, bad), TypeError);
    }
    assert.equal(calls.length, 1);
  } finally {
    globalThis.fetch = original;
  }
});

test('bounty reviews read the order and send only an exact, reasoned verdict', async () => {
  const original = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return new Response('{}', { status: 200 }); };
  try {
    const client = new AgentBountiesClient({ baseUrl: 'https://api.example', credentials: 'include' });
    const id = '00000000-0000-4000-8000-000000000001';
    const verdict = { round: 1, submission_hash: '0x' + '11'.repeat(32), evidence_hash: '0x' + '22'.repeat(32), passed: false,
      reason: 'The artifact omits the requested report.' };
    await client.getManagedBountyReview(id);
    await client.recordManagedBountyReview(id, verdict);
    assert.equal(calls[0].url, 'https://api.example/v1/managed-service/orders/' + id + '/review');
    assert.deepEqual(JSON.parse(calls[1].init.body), verdict);
    await client.recordManagedBountyReview(id, { ...verdict, passed: true, reason: undefined });
    assert.equal(JSON.parse(calls[2].init.body).reason, '');
    for (const bad of [{ round: 0 }, { submission_hash: '0x' + 'AA'.repeat(32) }, { passed: 'yes' }, { reason: 'Too short.' }, { reason: ' padded reason that is long enough' }]) {
      await assert.rejects(client.recordManagedBountyReview(id, { ...verdict, ...bad }), TypeError);
    }
    await assert.rejects(client.getManagedBountyReview('../stripe/connect-transfers'), TypeError);
    assert.equal(calls.length, 3);
  } finally {
    globalThis.fetch = original;
  }
});

test('card bounty appeals send only the signed appeal and operator rulings', async () => {
  const original = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return new Response('{}', { status: 200 }); };
  try {
    const client = new AgentBountiesClient({ baseUrl: 'https://api.example' });
    const bounty = '0x' + 'AB'.repeat(20);
    const appeal = { round: 2, solver: '0x' + '5E'.repeat(20), reason: 'The report is in section 2 of the artifact.', signature: '0x' + '12'.repeat(65) };
    await client.getCardBountyReview('base-sepolia', bounty);
    await client.appealCardBountyReview('base-sepolia', bounty, appeal);
    await client.listCardBountyAppeals();
    await client.ruleCardBountyAppeal('base-sepolia', bounty, 2, { reversed: true, reason: 'Section 2 contains the requested report.' });
    const base = 'https://api.example/v1/managed-service/';
    assert.deepEqual(calls.map(c => c.url), [
      base + 'bounties/base-sepolia/0x' + 'ab'.repeat(20) + '/review',
      base + 'bounties/base-sepolia/0x' + 'ab'.repeat(20) + '/appeal',
      base + 'appeals',
      base + 'appeals/base-sepolia/0x' + 'ab'.repeat(20) + '/2/ruling',
    ]);
    assert.deepEqual(JSON.parse(calls[1].init.body), { ...appeal, solver: appeal.solver.toLowerCase() });
    assert.deepEqual(JSON.parse(calls[3].init.body), { reversed: true, reason: 'Section 2 contains the requested report.' });
    for (const bad of [{ round: 0 }, { solver: 'alice' }, { signature: '0x1' }, { reason: 'Disagree.' }]) {
      await assert.rejects(client.appealCardBountyReview('base-sepolia', bounty, { ...appeal, ...bad }), TypeError);
    }
    await assert.rejects(client.getCardBountyReview('ethereum', bounty), TypeError);
    await assert.rejects(client.getCardBountyReview('base-sepolia', '../appeals'), TypeError);
    await assert.rejects(client.ruleCardBountyAppeal('base-sepolia', bounty, 2, { reversed: 'yes', reason: 'x'.repeat(30) }), TypeError);
    assert.equal(calls.length, 4);
  } finally {
    globalThis.fetch = original;
  }
});

test('deposit payouts are listed and recorded only by exact transaction hash', async () => {
  const original = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return new Response('{}', { status: 200 }); };
  try {
    const client = new AgentBountiesClient({ baseUrl: 'https://api.example' });
    const bounty = '0x' + 'ab'.repeat(20);
    const hash = '0x' + 'A1'.repeat(32);
    await client.listCardBountyDepositPayouts();
    await client.recordCardBountyDepositPayout('base-mainnet', bounty, hash);
    assert.deepEqual(calls.map(c => c.url), [
      'https://api.example/v1/managed-service/deposit-payouts',
      'https://api.example/v1/managed-service/deposit-payouts/base-mainnet/' + bounty,
    ]);
    assert.deepEqual(JSON.parse(calls[1].init.body), { tx_hash: hash.toLowerCase() });
    await assert.rejects(client.recordCardBountyDepositPayout('base-mainnet', bounty, '0x12'), TypeError);
    await assert.rejects(client.recordCardBountyDepositPayout('base-mainnet', '../appeals', hash), TypeError);
    assert.equal(calls.length, 2);
  } finally {
    globalThis.fetch = original;
  }
});
