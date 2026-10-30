(function () {
  'use strict';
  const core = window.ManagedService, el = id => document.getElementById('managed-' + id);
  const preparationPage = location.search === '' && location.hash === ''; // Before any URL normalization.
  let target, origin, epoch = 0, controller, poll, expiry, polls = 0, account = null, record = null, digest = null, order = null, busy = false, pending = null;
  const status = message => { el('status').textContent = message; };
  function clear() {
    epoch++; if (controller) controller.abort(); clearTimeout(poll); clearTimeout(expiry);
    account = record = digest = order = null; busy = false;
    el('private').hidden = true; el('acceptance').hidden = true; el('checkout').hidden = true;
    el('refund-form').hidden = true; el('refund-consent').checked = false; el('refund-info').textContent = ''; el('refund').disabled = true;
    el('bounty-link').hidden = true; el('bounty-link').removeAttribute('href');
    pending = null; el('review').hidden = true; el('review-form').hidden = true; el('review-artifact-row').hidden = true;
    el('review-artifact').removeAttribute('href'); el('review-reason').value = ''; el('review-consent').checked = false;
    el('verdict-pass').checked = el('verdict-reject').checked = false;
    for (const id of ['review-info', 'review-reference', 'review-deposit', 'deposit-note']) el(id).textContent = '';
    el('verdict-reject').disabled = el('verdict-pass').disabled = false; el('deposit-note').hidden = true;
    el('login').hidden = true; el('logout').hidden = true; el('providers').replaceChildren();
    for (const id of ['deliverable', 'criteria', 'deadline', 'remedies', 'version', 'bounty', 'reference', 'mode', 'total', 'expiry', 'next']) el(id).textContent = '';
    el('lines').replaceChildren(); el('consent').checked = false;
    el('accept').disabled = el('checkout').disabled = true; el('refresh').disabled = false;
    controller = new AbortController();
    return epoch;
  }
  function current(token) { if (token !== epoch || document.hidden) throw new Error('Obsolete read'); }
  function controls() {
    el('accept').disabled = busy || !record || !el('consent').checked;
    el('checkout').disabled = busy || !order || order.status !== 'awaiting_payment';
    el('review-submit').disabled = busy || !reviewBody();
    el('refund').disabled = busy || !order || order.status !== 'awaiting_bank_settlement' || !el('refund-consent').checked;
    el('refresh').disabled = busy;
  }
  async function request(path, token, body) {
    current(token);
    const requestController = controller;
    const timeout = setTimeout(() => requestController.abort(), 15000);
    try {
      const response = await fetch(origin + path, {
        method: body === undefined ? 'GET' : 'POST', credentials: 'include', cache: 'no-store', redirect: 'error',
        referrerPolicy: 'no-referrer', signal: requestController.signal,
        headers: body === undefined ? { Accept: 'application/json' } : { Accept: 'application/json', 'Content-Type': 'application/json' },
        ...(body === undefined ? {} : { body: JSON.stringify(body) })
      });
      current(token);
      if (!response.ok) throw Object.assign(new Error('Service unavailable'), { status: response.status });
      if (response.status === 204) return null;
      if (!response.headers.get('content-type')?.includes('application/json')) throw new Error('Invalid response');
      const reader = response.body.getReader(), chunks = []; let count = 0;
      try {
        for (;;) {
          const { done, value } = await reader.read(); if (done) break;
          count += value.byteLength; if (count > 131072) throw new Error('Response too large'); chunks.push(value);
        }
      } finally { await reader.cancel(); }
      current(token);
      const bytes = new Uint8Array(count); let offset = 0;
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
      return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    } finally { clearTimeout(timeout); }
  }
  async function sameAccount(token, expected) {
    const session = await request('/v1/site-auth/session', token);
    if (core.session(session) !== expected) throw Object.assign(new Error('Account changed'), { status: 401 });
    current(token);
  }
  function signIn(session) {
    const providers = { google: 'Google', github: 'GitHub', microsoft: 'Microsoft', amazon: 'Amazon' };
    for (const [name, label] of Object.entries(providers)) {
      if (session.providers?.[name] !== true) continue;
      const link = document.createElement('a'); link.className = 'login-link'; link.textContent = 'Sign in with ' + label;
      link.href = origin + '/v1/site-auth/login/' + name + '?return_to=' + encodeURIComponent(target.path);
      link.referrerPolicy = 'no-referrer'; el('providers').append(link);
    }
    el('login').hidden = false;
    status(el('providers').childElementCount ? 'Sign in to view your private service order.' : 'Sign-in is currently unavailable. Try refreshing later.');
  }
  function date(seconds) {
    const d = new Date(seconds * 1000);
    if (!Number.isFinite(d.getTime())) throw new Error('Unsupported deadline');
    return d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short', timeZoneName: undefined }) + ' (your local time)';
  }
  function drawQuote(q) {
    el('deliverable').textContent = q.input.terms.deliverable;
    el('criteria').textContent = q.input.terms.acceptance_criteria;
    el('deadline').textContent = date(q.input.terms.delivery_deadline);
    el('remedies').textContent = q.input.terms.remedies;
    el('version').textContent = 'Terms version: ' + q.input.terms.terms_version;
    const b = q.input.bounty, network = b.network === 'base-mainnet' ? 'Base' : 'Base Sepolia (test network)';
    el('bounty').textContent = 'Your task becomes a public bounty on ' + network + ' so solvers can work on it. Its description and criteria are public; your identity and price are not.';
    el('bounty-link').href = (b.network === 'base-mainnet' ? 'https://basescan.org' : 'https://sepolia.basescan.org') + '/address/' + b.predicted_bounty;
    el('bounty-link').textContent = 'Its address on Basescan (active once funded)'; el('bounty-link').hidden = false;
    const deposit = q.input.additional_costs.find(c => c.code === 'review_deposit');
    el('deposit-note').hidden = !deposit;
    el('deposit-note').textContent = deposit ? 'The review deposit (' + core.money(deposit.amount_minor, q.input.currency) + ', 10% of the solver reward) is a refundable hold, not a fee. It returns to your card when the bounty is paid. You lose it only if you reject a delivery and an appeal finds it met your criteria; then it goes to that solver. The card processing on it is not refunded.' : '';
    el('mode').textContent = q.input.livemode ? 'Service order' : 'Test mode — no real payment';
    el('total').textContent = core.money(q.total_minor, q.input.currency);
    el('expiry').textContent = 'Offer valid until ' + date(q.input.expires_at) + '.';
    el('lines').replaceChildren(...q.lines.map(line => {
      const row = document.createElement('div'), label = document.createElement('dt'), value = document.createElement('dd');
      label.textContent = line.label; value.textContent = core.money(line.amount_minor, q.input.currency); row.append(label, value); return row;
    }));
    el('private').hidden = false;
  }
  function quoteState() {
    const expired = Date.now() >= record.input.expires_at * 1000;
    const future = Date.now() < record.input.issued_at * 1000;
    el('heading').textContent = 'Review your quote'; el('reference').textContent = 'Quote ' + record.input.quote_id;
    status(expired ? 'This quote has expired. You can check for an earlier acceptance; a new acceptance will be declined.' : future ? 'This quote is not valid yet. Refresh when its offer period starts.' : 'Review the deliverable, remedies, and full price before accepting.');
    el('accept').textContent = expired ? 'Recover earlier acceptance' : 'Accept quote';
    el('acceptance').hidden = future;
    el('next').textContent = 'Accepting saves your agreement. You will choose whether to continue to Stripe in the next step.';
    clearTimeout(expiry);
    if (!expired && !future) expiry = setTimeout(() => { if (record && !order) quoteState(); }, Math.min(2147483647, record.input.expires_at * 1000 - Date.now() + 1));
    controls();
  }
  function drawOrder(value) {
    order = value; record = order.quote; digest = order.quote_digest; drawQuote(record);
    el('heading').textContent = 'Your accepted order'; el('reference').textContent = 'Order ' + order.order_id;
    el('acceptance').hidden = true;
    const states = {
      awaiting_payment: ['Payment is not confirmed. You can continue or recover the existing Stripe checkout.', 'Payment confirmation may arrive after you return. Opening or closing Checkout does not confirm or cancel a payment.'],
      payment_pending: ['Your payment is processing. No additional payment is needed.', 'We are waiting for the payment outcome. This order is not yet confirmed as funded.'],
      payment_failed: ['The payment attempt failed. This order is not funded.', 'Keep this order reference for help with payment recovery. Do not create a duplicate order to retry.'],
      payment_expired: ['The payment session expired. This order is not funded.', 'Keep this order reference for help with payment recovery. An expired session does not establish a refund.'],
      awaiting_bank_settlement: ['Payment received. Bounty funding has not started.', 'We fund your public bounty from our USDC treasury after a short safety delay. Until funding starts you can cancel for a full refund. Funding is not yet confirmed.'],
      refund_requested: ['Your full refund has been requested.', 'Funding is stopped for this order. The refund has not yet been confirmed as sent. Keep this order reference and refresh for updates.'],
      refund_pending: ['Your full refund is processing.', 'Funding remains stopped. We are waiting for Stripe’s refund confirmation; your bank has not been confirmed credited.'],
      refund_action_required: ['Your refund needs additional attention.', 'Funding remains stopped. Keep this order reference and contact support for the next step. Do not submit another payment.'],
      refund_sent: ['Stripe confirms your full refund was sent.', 'Your bank may take additional time to credit it. This status does not confirm that the credit has reached your bank account.'],
      refund_failed: ['The refund could not be completed.', 'Funding remains stopped. Keep this order reference and contact support to resolve the refund. Do not create another order or payment.'],
      funding_reserved: ['Your bounty is being funded.', 'Funding has started from our USDC treasury, so this order can no longer be cancelled for a refund. The bounty is not confirmed until Base records its exact funding.'],
      funding_broadcast: ['Your bounty funding transaction was sent.', 'We are waiting for Base to confirm the exact bounty and its full funding. A sent transaction is not confirmed funding.'],
      funded: ['Your bounty is funded on Base.', 'Base confirmed the exact bounty you accepted and its full funding. Solvers can now claim it, and you review the delivered work against your published criteria.'],
      funding_reconciliation_required: ['Bounty funding needs our attention.', 'Your payment is held. We will complete the exact funding you accepted or resolve the order with you. Keep this order reference and do not create another order or payment.']
    };
    status(states[order.status][0]); el('next').textContent = states[order.status][1];
    el('checkout').hidden = order.status !== 'awaiting_payment';
    el('refund-form').hidden = order.status !== 'awaiting_bank_settlement';
    if (order.status === 'funded') el('bounty-link').textContent = 'View your funded bounty on Basescan';
    el('refund-consent').checked = false;
    el('refund-info').textContent = order.status === 'awaiting_bank_settlement'
      ? 'You can cancel before funding and request a full refund of ' + core.money(record.total_minor, record.input.currency) + ', including the platform fee and all listed costs. This stops funding of the service.' : '';
    controls();
  }
  function drawRecoveredRefund(refund) {
    if (!refund || refund.status === 'awaiting_eligible_recovery') return false;
    const expected = record.lines.filter(line => ['solver_reward', 'verification'].includes(line.code)).reduce((sum, line) => sum + line.amount_minor, 0);
    if (!Number.isSafeInteger(refund.amount_minor) || refund.amount_minor !== expected || refund.currency !== record.input.currency) throw new Error('Invalid recovered refund');
    const messages = {
      pending_binding: 'Your bounty was cancelled and its USDC recovered. Your card refund is being checked.',
      awaiting_signed_event: 'Your recovered bounty refund is awaiting payment confirmation.',
      reconciliation_required: 'Your recovered bounty refund needs our attention.',
      pending: 'Your recovered bounty refund is processing.',
      requires_action: 'Your recovered bounty refund needs additional attention.',
      succeeded: 'Stripe confirms your recovered bounty refund was sent. Your bank may take additional time to credit it.',
      failed: 'Your recovered bounty refund could not be completed. Contact support with this order reference.',
      canceled: 'Your recovered bounty refund was cancelled. Contact support with this order reference.'
    };
    if (!messages[refund.status]) throw new Error('Unknown recovered refund');
    status(messages[refund.status]);
    el('next').textContent = 'Refund amount: ' + core.money(refund.amount_minor, refund.currency) + '. This returns the original solver reward and verification charges. Fees and the review deposit are excluded; the deposit follows its separate rules.' + (refund.reconciliation_required ? ' We are also checking a reconciliation issue.' : '');
    el('bounty-link').textContent = 'View the bounty’s funding and recovery history on Basescan';
    return true;
  }
  function failure(error, token, action = false) {
    if (token !== epoch || document.hidden) return;
    const code = error.status;
    if ([401, 403, 404].includes(code) || !action) clear();
    else { busy = false; controller = new AbortController(); controls(); }
    const messages = {
      401: 'Your sign-in changed or expired. Refresh and sign in again to this order’s account.',
      403: 'This account cannot perform that action. Refresh your sign-in before trying again.',
      404: 'This quote or order is not available to the signed-in account. Check the link and account.',
      409: 'The existing request needs reconciliation. Keep this link and refresh status; do not create a replacement order.',
      410: 'No earlier acceptance was recovered, and the offer has expired. Ask for a new quote.',
      429: 'Too many requests. Wait a minute, then refresh this order.',
      503: 'Payments are currently unavailable. Keep this link and refresh later to recover the same request.'
    };
    status(messages[code] || (action ? 'The result could not be confirmed. Keep this link. Retrying uses the same request; do not start another order.' : 'We could not safely load this record. Refresh to retry. This status check does not start a payment.'));
  }
  function reviewBody() {
    if (!pending || !el('review-consent').checked || !(el('verdict-pass').checked || el('verdict-reject').checked)) return null;
    try { return core.verdict(pending, el('verdict-pass').checked, el('review-reason').value); } catch (_) { return null; }
  }
  const when = seconds => new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
  function appealText(r, deposit) {
    const forfeit = deposit ? 'your ' + core.money(deposit.amount_minor, deposit.currency) + ' review deposit is owed to them' : 'your review deposit is owed to them';
    switch (r.appeal && r.appeal.status) {
      case 'window_open': return 'You rejected attempt ' + r.round + '. The solver can appeal until ' + when(r.appeal.window_ends_at) + '. If they don’t, the rejection becomes final and the bounty reopens for another solver.';
      case 'appealed': return 'The solver appealed your rejection of attempt ' + r.round + '. Agentizable is checking the delivery against your published criteria. If your rejection is upheld, the bounty reopens; if it is reversed, the solver is paid and ' + forfeit + '.';
      case 'reversed': return 'On appeal, attempt ' + r.round + ' was found to meet your published criteria. The solver is paid when the bounty settles on Base, and ' + forfeit + '.';
      default: return 'Your rejection of attempt ' + r.round + ' is final. The bounty stays funded and open for another solver; only Base confirmation reopens it.';
    }
  }
  // What silence until decide_by does. Only an explicit true from the API promises acceptance.
  function silenceText(p) {
    if (p.auto_accept === true) return 'If you don’t decide by then, it is accepted automatically. The solver is paid only once Base confirms settlement.';
    if (p.auto_accept === false) return 'It was published too late for automatic acceptance: if you don’t decide by then, nothing is accepted, the attempt expires on ' + when(p.expires_at) + ' and the bounty reopens.';
    return 'Decide before then.';
  }
  function depositText(d) {
    if (!d) return '';
    const amount = core.money(d.amount_minor, d.currency);
    return {
      held: 'Refundable review deposit: ' + amount + '. It returns to your card when the bounty is paid, unless an appeal reverses a rejection you made. The card processing on it is not refunded.',
      refund_pending: 'Your ' + amount + ' review deposit refund is being sent to your card.',
      refunded: 'Stripe confirms your ' + amount + ' review deposit refund was sent.',
      refund_failed: 'Stripe could not complete your ' + amount + ' review deposit refund. Agentizable will contact you to send it another way.',
      forfeited_to_solver: 'Your ' + amount + ' review deposit is owed to the solver because an appeal reversed your rejection. Agentizable pays it to them in USDC.'
    }[d.status];
  }
  function drawReview(v) {
    el('review').hidden = false; pending = null; el('review-form').hidden = true; el('review-artifact-row').hidden = true;
    el('review-deposit').textContent = depositText(v.deposit);
    const recorded = v.pending && v.reviews.find(r => r.round === v.pending.round);
    const latest = v.reviews[0];
    if (!v.pending) {
      el('review-info').textContent = v.status === 'paid'
        ? 'Base confirms the solver was paid after your acceptance.'
        : latest && !latest.passed ? appealText(latest, v.deposit)
          : 'No delivery awaits your review. Solvers can claim the bounty; refresh to check for a submission.';
    } else if (recorded) {
      el('review-info').textContent = recorded.passed
        ? 'You accepted attempt ' + recorded.round + '. It is being recorded on Base; only Base confirmation pays the solver.'
        : appealText(recorded, v.deposit);
    } else if (!v.pending.published) {
      el('review-info').textContent = 'A solver submitted attempt ' + v.pending.round + ', but its artifact is not published yet. Refresh later; you can review it once it is.';
    } else {
      pending = v.pending; el('review-form').hidden = false;
      el('review-info').textContent = 'A solver submitted attempt ' + pending.round + '. Inspect it against every published criterion and accept or reject it by ' + when(pending.decide_by) + '. ' + silenceText(pending);
      el('review-reference').textContent = 'Submitted reference: ' + pending.reference;
      if (pending.artifact) { el('review-artifact').href = pending.artifact; el('review-artifact-row').hidden = false; }
      el('verdict-reject').disabled = el('verdict-pass').disabled = Date.now() / 1000 >= pending.decide_by;
    }
    controls();
  }
  async function loadReview(token) {
    const reply = await request('/v1/managed-service/orders/' + order.order_id + '/review', token);
    drawReview(core.review(reply, record.input.bounty.predicted_bounty)); current(token);
  }
  async function submitReview() {
    const body = reviewBody();
    if (busy || !order || order.status !== 'funded' || !body) return;
    const token = epoch, expected = account, bounty = record.input.bounty.predicted_bounty;
    busy = true; clearTimeout(poll); controls(); status('Recording your verdict…');
    try {
      await sameAccount(token, expected);
      const reply = await request('/v1/managed-service/orders/' + order.order_id + '/review', token, body); current(token);
      const d = reply && reply.document;
      if (!d || d.bounty_contract !== bounty || d.round !== body.round || d.passed !== body.passed) throw new Error('Wrong verdict');
      busy = false; status('Your verdict is recorded. Only Base confirmation settles or reopens the bounty.');
      await loadReview(token);
    } catch (error) { failure(error, token, true); }
  }
  async function load(automatic = false) {
    if (!target || document.hidden) return;
    if (!automatic) polls = 0;
    const token = clear(); status('Checking your account and latest order status…');
    try {
      const session = await request('/v1/site-auth/session', token); account = core.session(session);
      if (!account) { signIn(session); return; }
      el('logout').hidden = false;
      if (target.kind === 'managed_quote') {
        const reply = await request('/v1/managed-service/quotes/' + target.id, token);
        const q = await core.quote(reply.quote, reply.quote_digest, account, target.id); current(token);
        record = q; digest = reply.quote_digest; drawQuote(q); quoteState();
      } else {
        const reply = await request('/v1/managed-service/orders/' + target.id, token);
        const value = await core.order(reply.order, account, target.id); current(token); drawOrder(value);
        const recovered = drawRecoveredRefund(reply.recovered_bounty_refund);
        if (order.status === 'funded' && !recovered) await loadReview(token);
        if (polls++ < 8 && ['awaiting_payment', 'payment_pending', 'refund_requested', 'refund_pending', 'funding_reserved', 'funding_broadcast'].includes(order.status)) poll = setTimeout(() => load(true), 15000);
      }
    } catch (error) { failure(error, token); }
  }
  async function accept() {
    if (busy || !record || order || !el('consent').checked) return;
    const token = epoch, expected = account, q = record, hash = digest;
    busy = true; clearTimeout(poll); controls(); status('Saving your acceptance…');
    try {
      await sameAccount(token, expected);
      const reply = await request('/v1/managed-service/quotes/' + q.input.quote_id + '/accept', token, { quote_digest: hash, idempotency_key: core.acceptanceKey(q, hash) });
      const value = await core.order(reply.order, expected, null, hash); current(token);
      target = core.target('?managed_order=' + value.order_id); history.replaceState(null, '', target.path);
      busy = false; drawOrder(value);
    } catch (error) { failure(error, token, true); }
  }
  async function checkout() {
    if (busy || !order || order.status !== 'awaiting_payment') return;
    const token = epoch, expected = account, id = order.order_id, live = record.input.livemode;
    busy = true; clearTimeout(poll); controls(); status('Recovering your order’s Stripe checkout…');
    try {
      await sameAccount(token, expected);
      const reply = await request('/v1/managed-service/orders/' + id + '/checkout', token, {}); current(token);
      if (reply.order_id !== id) throw new Error('Wrong order');
      if (reply.status === 'payment_received') { await load(); return; }
      if (reply.checkout?.status === 'busy') { busy = false; controls(); status('The existing checkout is being recovered. Wait a moment, then refresh status.'); return; }
      const c = reply.checkout;
      if (c?.status !== 'bound' || typeof c.session_id !== 'string' || !new RegExp('^cs_' + (live ? 'live' : 'test') + '_[a-zA-Z0-9]+$').test(c.session_id)) throw new Error('Invalid Checkout response');
      if (c.checkout_url === null) { await load(); return; }
      const url = core.checkoutURL(c.checkout_url);
      if (new URL(url).pathname !== '/c/pay/' + c.session_id) throw new Error('Wrong Checkout URL');
      clear(); status('Opening Stripe for this order…'); window.location.assign(url);
    } catch (error) { failure(error, token, true); }
  }
  async function refund() {
    if (busy || !order || order.status !== 'awaiting_bank_settlement' || !el('refund-consent').checked) return;
    const token = epoch, expected = account, id = order.order_id, hash = order.quote_digest;
    busy = true; clearTimeout(poll); controls(); status('Recording your cancellation and full refund request…');
    try {
      await sameAccount(token, expected);
      const reply = await request('/v1/managed-service/orders/' + id + '/refund', token, {});
      const value = await core.order(reply.order, expected, id, hash); current(token);
      if (!['refund_requested', 'refund_pending', 'refund_action_required', 'refund_sent', 'refund_failed'].includes(value.status)) throw new Error('Refund not reserved');
      busy = false; drawOrder(value);
    } catch (error) { failure(error, token, true); }
  }
  // A page-local preparation surface: no shared analytics, cookies, wallet access or
  // automatic acceptance. The server owns account admission, price and funding rules.
  let preparationAttempted = false;
  async function prepareCardQuote(input) {
    const body = core.cardQuoteRequest(input), currency = input.currency, maximum = input.maximum_charge_minor;
    if (!preparationPage || target || busy || preparationAttempted) throw new Error('Keep the current quote or order; do not create a replacement.');
    const token = clear(); busy = true; controls(); status('Preparing an itemized card quote…');
    try {
      const session = await request('/v1/site-auth/session', token); account = core.session(session);
      if (!account) throw Object.assign(new Error('Sign in first'), { status: 401 });
      preparationAttempted = true;
      const reply = await request('/v1/managed-service/bounty-quotes', token, body);
      await sameAccount(token, account);
      const q = await core.quote(reply.quote, reply.quote_digest, account); current(token);
      const now = Math.floor(Date.now() / 1000);
      if (q.input.currency !== currency || q.total_minor > maximum
          || q.input.solver_reward_micro_usdc !== body.solver_reward_micro_usdc
          || q.input.issued_at > now || q.input.expires_at <= now) throw new Error('Quote does not match the requested budget.');
      target = core.target('?managed_quote=' + q.input.quote_id);
      history.replaceState(null, '', target.path);
      record = q; digest = reply.quote_digest; busy = false; drawQuote(q); quoteState();
      return { status: 'quote_ready_for_personal_review', quote_id: q.input.quote_id, quote_digest: digest,
        currency: q.input.currency, total_minor: q.total_minor, review_path: target.path,
        public_terms_published: true, accepted: false, checkout_created: false, payment_made: false };
    } catch (error) {
      failure(error, token, true);
      if (token === epoch && !document.hidden && preparationAttempted)
        status('The quote result could not be confirmed. Public terms may have been saved, but no order or payment was started. Do not repeat preparation automatically.');
      throw new Error('Card quote preparation unavailable; check the page status.');
    }
  }
  const preparationLifecycle = new AbortController();
  function registerPreparation() {
    if (window.top !== window.self) return;
    try { core.apiOrigin(location); } catch (_) { return; }
    const contexts = [...new Set([document.modelContext, window.navigator?.modelContext])]
      .filter(context => typeof context?.registerTool === 'function');
    const tool = { name: 'agent_bounties_prepare_card_quote', title: 'Prepare an itemized card bounty quote',
      description: 'Publish the intended public bounty terms and prepare an account-owned card quote using the current signed-in session. Supply only the intended public task, never private files or credentials. The server checks account admission and price limits. This displays the exact full price for personal review; it never accepts terms, creates Checkout, pays, signs, or funds. Use a blank account.html page. A failed or lost result must not be automatically retried.',
      annotations: { readOnlyHint: false, untrustedContentHint: true },
      inputSchema: { type: 'object', additionalProperties: false,
        required: ['document', 'solver_reward_micro_usdc', 'currency', 'maximum_charge_minor'],
        properties: { document: { type: 'object', description: 'Public title, goal, acceptance_criteria, benchmark {engine: customer_review_v1, delivery_deadline: Unix seconds}, evidence_schema; optional source_url and image.' },
          solver_reward_micro_usdc: { type: 'integer', minimum: 1, maximum: Number.MAX_SAFE_INTEGER },
          currency: { type: 'string', enum: ['mxn', 'usd'] },
          maximum_charge_minor: { type: 'integer', minimum: 1, maximum: Number.MAX_SAFE_INTEGER } } },
      execute: prepareCardQuote };
    for (const context of contexts) {
      try { Promise.resolve(context.registerTool(tool, { signal: preparationLifecycle.signal })).catch(() => {}); }
      catch (_) { /* Ordinary quote review works without WebMCP support. */ }
    }
  }
  window.addEventListener('pagehide', () => preparationLifecycle.abort());
  registerPreparation();
  el('refund-consent').addEventListener('change', controls); el('refund').addEventListener('click', refund);
  for (const id of ['verdict-pass', 'verdict-reject', 'review-consent']) el(id).addEventListener('change', controls);
  el('review-reason').addEventListener('input', controls); el('review-submit').addEventListener('click', submitReview);
  el('consent').addEventListener('change', controls);
  el('accept').addEventListener('click', accept); el('checkout').addEventListener('click', checkout);
  el('refresh').addEventListener('click', () => load());
  el('logout').addEventListener('click', async () => {
    const token = clear(); status('Signing out…');
    try { await request('/v1/site-auth/logout', token, {}); await load(); }
    catch (error) { failure(error, token); }
  });
  window.addEventListener('pagehide', () => { clear(); status('Refresh to securely load your order.'); });
  window.addEventListener('pageshow', event => { if (event.persisted) load(); });
  document.addEventListener('visibilitychange', () => { if (document.hidden) { clear(); status('Refresh to securely load your order.'); } else load(); });
  try {
    if (window.top !== window.self) throw new Error('Embedded payment page');
    origin = core.apiOrigin(location); target = core.target(location.search); history.replaceState(null, '', target.path); load();
  }
  catch (_) { clear(); history.replaceState(null, '', '/account.html'); el('refresh').disabled = true; status('Open the quote or order link provided by Agentizable to view its details.'); }
})();
