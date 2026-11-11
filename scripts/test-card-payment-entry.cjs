// Runs the posting page's real card-payment controller against a stubbed quote API: every refusal
// keeps the draft and says why, and only a confirmed quote id opens the order page.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const source = fs.readFileSync(require.resolve('../site/bounty-composer-v2.js'), 'utf8');
const start = source.indexOf('  function syncCardPayment() {');
const end = source.indexOf('  async function openFunding() {', start);
assert.ok(start > 0 && end > start, 'card payment controller not found');

function controller(response, released = 'true') {
  const button = () => ({ hidden: true, disabled: true, dataset: { cardPaymentEnabled: released } });
  const context = {
    API: 'https://api.agentizable.ai',
    ui: { card: button(), cardNote: button(), cardStatus: button() },
    state: {
      draft: { title: 'T', goal: 'G', acceptance_criteria: ['C'], evidence_schema: { type: 'object' } },
      reviewStale: false, imageReady: true, postingAccountStatus: 'ready',
      deliveryDeadline: '2030-01-01T00:00:00Z', sourceUrl: null, bountyImage: null,
    },
    requests: [], statuses: [], opened: null, loggedIn: false,
    cardEligible: () => true,
    beginPostingLogin() { context.loggedIn = true; },
    canonicalJsonValue: value => value,
    stripVisualExtension: value => value,
    currentRewardSplit: () => ({ solver: 25000000n }),
    setStatus(message, kind) { context.statuses.push([kind, message]); },
    async fetch(url, init) { context.requests.push([url, JSON.parse(init.body)]); return response; },
    window: {
      AgentBountiesWorkflow: { UUID: /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/ },
      location: { assign(url) { context.opened = url; } },
    },
  };
  const api = vm.runInNewContext(`${source.slice(start, end)}\n({ syncCardPayment, payByCard })`, context);
  return { ...api, context };
}

const ID = '11111111-1111-4111-8111-111111111111';

test('the published page keeps card checkout off until its separate release', () => {
  const html = fs.readFileSync(require.resolve('../site/post.html'), 'utf8');
  assert.match(html, /data-pay-card data-card-payment-enabled="true" hidden>Pay with credit\/debit card</);
});

for (const released of [undefined, 'false', 'TRUE', '1']) {
  test(`an absent or inactive release flag (${released}) hides card entry and makes no request`, async () => {
    const { syncCardPayment, payByCard, context } = controller({ ok: true });
    context.ui.card.dataset.cardPaymentEnabled = released;
    syncCardPayment();
    assert.equal(context.ui.card.hidden, true);
    assert.equal(context.ui.cardNote.hidden, true);
    assert.equal(context.ui.card.disabled, true);
    await payByCard();
    assert.equal(context.requests.length, 0);
    assert.equal(context.loggedIn, false);
    assert.equal(context.opened, null);
  });
}

test('a confirmed quote opens its itemized order page and sends the exact solver reward', async () => {
  const { payByCard, context } = controller({ ok: true, json: async () => ({ quote: { input: { quote_id: ID } } }) });
  await payByCard();
  assert.equal(context.opened, `account.html?managed_quote=${ID}`);
  const [url, body] = context.requests[0];
  assert.equal(url, 'https://api.agentizable.ai/v1/managed-service/bounty-quotes');
  assert.equal(body.solver_reward_micro_usdc, 25000000);
  assert.equal(body.document.benchmark.engine, 'customer_review_v1');
});

for (const [status, text] of [
  [422, /above the current card payment limit/],
  [503, /isn't available right now/],
  [401, /Sign in again/],
  [400, /can't be paid by card/],
]) {
  test(`a ${status} refusal keeps the draft, opens nothing and re-enables the button`, async () => {
    const { payByCard, context } = controller({ ok: false, status, json: async () => ({}) });
    await payByCard();
    assert.equal(context.opened, null);
    assert.deepEqual(context.statuses.at(-1)[0], 'error');
    assert.match(context.statuses.at(-1)[1], text);
    assert.equal(context.ui.card.disabled, false);
    assert.equal(context.ui.cardStatus.hidden, false, 'the refusal shows beside the card button');
    assert.match(context.ui.cardStatus.textContent, new RegExp(`code ${status}`));
  });
}

for (const [setup, text] of [
  [state => { state.imageReady = false; }, /still loading/],
  [state => { state.reviewStale = true; }, /Choose Review draft/],
]) {
  test(`a card that is not ready says why instead of doing nothing (${text})`, async () => {
    const { payByCard, context } = controller({ ok: true, json: async () => ({}) });
    setup(context.state);
    await payByCard();
    assert.equal(context.requests.length, 0);
    assert.equal(context.ui.cardStatus.hidden, false);
    assert.match(context.ui.cardStatus.textContent, text);
  });
}

test('an unconfirmed quote id opens nothing', async () => {
  const { payByCard, context } = controller({ ok: true, json: async () => ({ quote: { input: { quote_id: 'not-a-uuid' } } }) });
  await payByCard();
  assert.equal(context.opened, null);
  assert.match(context.statuses.at(-1)[1], /could not be confirmed\. Nothing was charged/);
});

test('a signed-out poster is sent to login before any quote request', async () => {
  const { payByCard, context } = controller({ ok: true, json: async () => ({}) });
  context.state.postingAccountStatus = 'checking';
  await payByCard();
  assert.equal(context.loggedIn, true);
  assert.equal(context.requests.length, 0);
});

test('a signed-in poster without a wallet gets a card quote instead of wallet setup', async () => {
  const composer = require('../site/bounty-composer-v2.js');
  for (const session of [
    { authenticated: true, account_status: 'ready', account_complete: true, solver_status: 'wallet_required', solver_ready: false },
    // The previous API reported the same wallet-less account this way during a rolling release.
    { authenticated: true, account_status: 'wallet_required', account_complete: false },
  ]) {
    const { payByCard, context } = controller({ ok: true, json: async () => ({ quote: { input: { quote_id: ID } } }) });
    context.state.postingAccountStatus = composer.postingAccountStatus(session);
    await payByCard();
    assert.equal(context.loggedIn, false);
    assert.equal(context.requests.length, 1);
    assert.equal(context.opened, `account.html?managed_quote=${ID}`);
  }
});
