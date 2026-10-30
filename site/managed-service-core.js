/* Price/identity validation shared by the private account page and its tests. */
(function (root) {
  'use strict';
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  const HEX = /^[0-9a-f]{64}$/;
  const STATES = ['awaiting_payment', 'payment_pending', 'payment_failed', 'payment_expired', 'awaiting_bank_settlement',
    'refund_requested', 'refund_pending', 'refund_action_required', 'refund_sent', 'refund_failed',
    'funding_reserved', 'funding_broadcast', 'funded', 'funding_reconciliation_required'];
  function requireValue(ok) { if (!ok) throw new Error('Invalid service record'); }
  function uuid(v) { return typeof v === 'string' && UUID.test(v) && v !== '00000000-0000-0000-0000-000000000000'; }
  function fields(v, names) {
    requireValue(v && typeof v === 'object' && !Array.isArray(v));
    requireValue(Object.keys(v).sort().join(',') === names.slice().sort().join(','));
  }
  // Exact keys, plus `optional` keys that older responses omit.
  function fieldsAllowing(v, names, optional) {
    requireValue(v && typeof v === 'object' && !Array.isArray(v));
    const keys = Object.keys(v);
    requireValue(names.every(n => keys.includes(n)) && keys.every(k => names.includes(k) || optional.includes(k)));
  }
  function integer(v, positive = true) { requireValue(Number.isSafeInteger(v) && v >= (positive ? 1 : 0)); return BigInt(v); }
  function key(v) { requireValue(typeof v === 'string' && /^[a-zA-Z0-9_.:-]{1,200}$/.test(v)); }
  function text(v, max) {
    requireValue(typeof v === 'string' && v.length > 0 && v.trim() === v && new TextEncoder().encode(v).length <= max);
    requireValue(!/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(v));
    requireValue(Array.from(v).every(c => { const n = c.codePointAt(0); return n < 0xd800 || n > 0xdfff; }));
  }
  function target(search) {
    const p = new URLSearchParams(search), entries = [...p].filter(([k]) => k === 'managed_order' || k === 'managed_quote');
    requireValue(entries.length === 1 && uuid(entries[0][1]));
    const [kind, id] = entries[0];
    return { kind, id, path: '/account.html?' + kind + '=' + id };
  }
  function apiOrigin(location) {
    if (['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname) && ['http:', 'https:'].includes(location.protocol)) return 'http://127.0.0.1:3000';
    requireValue(location.origin === 'https://agentizable.ai');
    return 'https://api.agentizable.ai';
  }
  function session(v) {
    requireValue(v && typeof v.authenticated === 'boolean');
    if (!v.authenticated) return null;
    requireValue(v.user && typeof v.user.id === 'string' && HEX.test(v.user.id));
    return v.user.id;
  }
  function canonicalQuote(q, account, quoteId) {
    fields(q, ['schema', 'input', 'platform_fee_bps', 'platform_fee_micro_usdc', 'bounty_target_micro_usdc', 'lines', 'total_minor']);
    requireValue(typeof account === 'string' && HEX.test(account) && q.schema === 'agent-bounties/managed-service-quote-v2' && q.platform_fee_bps === 750);
    const i = q.input;
    fields(i, ['quote_id', 'buyer_principal', 'stripe_account_id', 'livemode', 'issued_at', 'expires_at', 'terms', 'currency', 'price_basis', 'solver_reward_micro_usdc', 'verifier_reserve_micro_usdc', 'additional_costs', 'bounty']);
    requireValue(uuid(i.quote_id) && (!quoteId || i.quote_id === quoteId) && i.buyer_principal === 'account:' + account);
    key(i.stripe_account_id); requireValue(i.stripe_account_id.startsWith('acct_') && i.stripe_account_id.length > 5);
    requireValue(typeof i.livemode === 'boolean' && ['usd', 'mxn'].includes(i.currency));
    integer(i.issued_at); integer(i.expires_at); requireValue(i.expires_at > i.issued_at);
    const t = i.terms, b = i.price_basis;
    fields(t, ['deliverable', 'acceptance_criteria', 'delivery_deadline', 'terms_version', 'remedies']);
    text(t.deliverable, 16384); text(t.acceptance_criteria, 16384); text(t.remedies, 8192); key(t.terms_version);
    integer(t.delivery_deadline); requireValue(t.delivery_deadline > i.expires_at);
    fields(b, ['reference', 'minor_units_per_usdc_numerator', 'minor_units_per_usdc_denominator', 'valid_until']);
    key(b.reference); integer(b.valid_until); requireValue(b.valid_until >= i.expires_at);
    const n = integer(b.minor_units_per_usdc_numerator), d = integer(b.minor_units_per_usdc_denominator) * 1000000n;
    const reward = integer(i.solver_reward_micro_usdc), verifier = integer(i.verifier_reserve_micro_usdc);
    const fee = (reward * 750n + 9999n) / 10000n, bounty = reward + verifier + fee;
    requireValue(integer(q.platform_fee_micro_usdc) === fee && integer(q.bounty_target_micro_usdc) === bounty);
    const price = amount => { const value = Number((amount * n + d - 1n) / d); integer(value); return value; };
    const lines = [
      { code: 'solver_reward', label: 'Solver reward', amount_minor: price(reward), amount_micro_usdc: Number(reward) },
      { code: 'verification', label: 'Verification reserve', amount_minor: price(verifier), amount_micro_usdc: Number(verifier) },
      { code: 'platform_fee', label: 'Platform fee (7.5% of solver reward)', amount_minor: price(fee), amount_micro_usdc: Number(fee) }
    ];
    requireValue(Array.isArray(i.additional_costs) && i.additional_costs.length <= 8);
    const codes = new Set(lines.map(l => l.code));
    const extras = i.additional_costs.map(e => {
      fields(e, ['code', 'label', 'amount_minor']); key(e.code); text(e.label, 160); integer(e.amount_minor);
      requireValue(!codes.has(e.code)); codes.add(e.code);
      // The refundable review deposit is exactly 10% of the solver reward at the quoted rate.
      if (e.code === 'review_deposit') requireValue(e.amount_minor === price((reward * 1000n + 9999n) / 10000n));
      lines.push({ code: e.code, label: e.label, amount_minor: e.amount_minor, amount_micro_usdc: null });
      return { code: e.code, label: e.label, amount_minor: e.amount_minor };
    });
    requireValue(Array.isArray(q.lines) && q.lines.length === lines.length);
    q.lines.forEach((l, index) => {
      fields(l, ['code', 'label', 'amount_minor', 'amount_micro_usdc']);
      for (const name of Object.keys(l)) requireValue(l[name] === lines[index][name]);
    });
    requireValue(integer(q.total_minor) === lines.reduce((sum, l) => sum + BigInt(l.amount_minor), 0n));
    const commitment = bountyCommitment(i.bounty, i, t);
    // Rust's serde struct field order is part of the v2 SHA-256 contract.
    return {
      schema: q.schema,
      input: {
        quote_id: i.quote_id, buyer_principal: i.buyer_principal, stripe_account_id: i.stripe_account_id, livemode: i.livemode,
        issued_at: i.issued_at, expires_at: i.expires_at,
        terms: { deliverable: t.deliverable, acceptance_criteria: t.acceptance_criteria, delivery_deadline: t.delivery_deadline, terms_version: t.terms_version, remedies: t.remedies },
        currency: i.currency,
        price_basis: { reference: b.reference, minor_units_per_usdc_numerator: b.minor_units_per_usdc_numerator, minor_units_per_usdc_denominator: b.minor_units_per_usdc_denominator, valid_until: b.valid_until },
        solver_reward_micro_usdc: i.solver_reward_micro_usdc, verifier_reserve_micro_usdc: i.verifier_reserve_micro_usdc, additional_costs: extras,
        bounty: commitment
      },
      platform_fee_bps: 750, platform_fee_micro_usdc: Number(fee), bounty_target_micro_usdc: Number(bounty), lines, total_minor: q.total_minor
    };
  }
  // Live payments may fund only Base mainnet; test-mode payments only Base Sepolia.
  function bountyCommitment(b, i, t) {
    fields(b, ['network', 'factory', 'funding_wallet', 'creation_nonce', 'terms_hash', 'verification_setup', 'funding_deadline', 'predicted_bounty', 'create_call_keccak256']);
    const address = v => requireValue(typeof v === 'string' && /^0x[0-9a-f]{40}$/.test(v) && !/^0x0+$/.test(v));
    const word = v => requireValue(typeof v === 'string' && /^0x[0-9a-f]{64}$/.test(v) && !/^0x0+$/.test(v));
    requireValue(b.network === (i.livemode ? 'base-mainnet' : 'base-sepolia'));
    address(b.factory); address(b.funding_wallet); address(b.predicted_bounty);
    requireValue(new Set([b.factory, b.funding_wallet, b.predicted_bounty]).size === 3);
    word(b.creation_nonce); word(b.terms_hash); word(b.create_call_keccak256); key(b.verification_setup);
    integer(b.funding_deadline); requireValue(b.funding_deadline > i.expires_at && b.funding_deadline <= t.delivery_deadline);
    return {
      network: b.network, factory: b.factory, funding_wallet: b.funding_wallet, creation_nonce: b.creation_nonce, terms_hash: b.terms_hash,
      verification_setup: b.verification_setup, funding_deadline: b.funding_deadline, predicted_bounty: b.predicted_bounty, create_call_keccak256: b.create_call_keccak256
    };
  }
  async function quote(q, digest, account, quoteId, cryptoProvider = root.crypto) {
    requireValue(typeof digest === 'string' && HEX.test(digest));
    const canonical = canonicalQuote(q, account, quoteId);
    const bytes = new TextEncoder().encode('agent-bounties:managed-service-quote:v2\0' + JSON.stringify(canonical));
    const actual = Array.from(new Uint8Array(await cryptoProvider.subtle.digest('SHA-256', bytes)), n => n.toString(16).padStart(2, '0')).join('');
    requireValue(actual === digest);
    return canonical;
  }
  async function order(v, account, id, expectedDigest) {
    fields(v, ['order_id', 'quote', 'quote_digest', 'accepted_at', 'status']);
    requireValue(uuid(v.order_id) && (!id || v.order_id === id) && STATES.includes(v.status));
    requireValue(!expectedDigest || v.quote_digest === expectedDigest);
    integer(v.accepted_at);
    const q = await quote(v.quote, v.quote_digest, account);
    requireValue(v.accepted_at >= q.input.issued_at && v.accepted_at < q.input.expires_at);
    return { ...v, quote: q };
  }
  function acceptanceKey(q, digest) { requireValue(uuid(q.input.quote_id) && HEX.test(digest)); return 'managed-web-v1:' + q.input.quote_id + ':' + digest; }
  function checkoutURL(value) {
    requireValue(typeof value === 'string' && value.length <= 8192 && !/[\s\u0000-\u001f\u007f\\]/u.test(value));
    const u = new URL(value);
    requireValue(u.origin === 'https://checkout.stripe.com' && !u.username && !u.password && u.pathname.startsWith('/c/pay/') && u.pathname.length > 7);
    return u.href;
  }
  function money(minor, currency) {
    const value = integer(minor, false), whole = (value / 100n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
    requireValue(['usd', 'mxn'].includes(currency));
    return currency.toUpperCase() + ' ' + whole + '.' + (value % 100n).toString().padStart(2, '0');
  }
  const WORD = /^0x[0-9a-f]{64}$/;
  function artifactURL(value) {
    try {
      requireValue(typeof value === 'string' && value.length <= 2048 && !/[\s\u0000-\u001f\u007f\\]/u.test(value));
      const u = new URL(value);
      requireValue(u.protocol === 'https:' && !u.username && !u.password);
      return u.href;
    } catch (_) { return null; }
  }
  // The review state of the order's own committed bounty; anything else fails closed.
  const APPEALS = ['window_open', 'appealed', 'upheld', 'final', 'reversed'];
  const DEPOSITS = ['held', 'refund_pending', 'refunded', 'refund_failed', 'forfeited_to_solver'];
  function review(v, bountyContract) {
    fields(v, ['order_id', 'network', 'bounty_contract', 'bounty_status', 'pending_submission', 'reviews', 'deposit']);
    requireValue(uuid(v.order_id) && v.bounty_contract === bountyContract && Array.isArray(v.reviews)); key(v.bounty_status);
    let pending = null;
    if (v.pending_submission !== null) {
      const p = v.pending_submission;
      fieldsAllowing(p, ['round', 'solver', 'submission_hash', 'evidence_hash', 'verification_expires_at', 'decide_by', 'evidence'], ['auto_accept', 'sign_by']);
      integer(p.round); integer(p.verification_expires_at); integer(p.decide_by, false);
      requireValue(WORD.test(p.submission_hash) && WORD.test(p.evidence_hash) && p.decide_by <= p.verification_expires_at);
      // Older responses carry neither schedule field; the page then promises nothing about silence.
      const scheduled = 'auto_accept' in p || 'sign_by' in p;
      if (scheduled) {
        requireValue(typeof p.auto_accept === 'boolean'); integer(p.sign_by);
        requireValue(p.decide_by < p.sign_by && p.sign_by < p.verification_expires_at);
      }
      requireValue(p.evidence === null || (p.evidence && typeof p.evidence.artifact_reference === 'string'));
      pending = {
        round: p.round, submission_hash: p.submission_hash, evidence_hash: p.evidence_hash, expires_at: p.verification_expires_at,
        decide_by: p.decide_by, auto_accept: scheduled ? p.auto_accept : null, sign_by: scheduled ? p.sign_by : null,
        published: p.evidence !== null, reference: p.evidence ? p.evidence.artifact_reference : null,
        artifact: p.evidence ? artifactURL(p.evidence.artifact_reference) : null
      };
    }
    const reviews = v.reviews.map(r => {
      const d = r && r.document;
      requireValue(d && d.schema === 'agent-bounties/customer-review-v1' && d.bounty_contract === bountyContract);
      integer(d.round); requireValue(typeof d.passed === 'boolean' && typeof d.reason === 'string');
      let appeal = null;
      if (r.appeal !== null && r.appeal !== undefined) {
        requireValue(!d.passed && APPEALS.includes(r.appeal.status)); integer(r.appeal.window_ends_at);
        appeal = { status: r.appeal.status, window_ends_at: r.appeal.window_ends_at };
      }
      return { round: d.round, passed: d.passed, reason: d.reason, appeal };
    });
    let deposit = null;
    if (v.deposit !== null) {
      fields(v.deposit, ['amount_minor', 'currency', 'status']);
      integer(v.deposit.amount_minor); requireValue(['usd', 'mxn'].includes(v.deposit.currency) && DEPOSITS.includes(v.deposit.status));
      deposit = { ...v.deposit };
    }
    return { status: v.bounty_status, pending, reviews, deposit };
  }
  // The exact verdict body the API accepts; a rejection must give a public reason.
  function verdict(pending, passed, reason) {
    requireValue(pending && pending.published && typeof passed === 'boolean' && typeof reason === 'string');
    const length = Array.from(reason).length;
    requireValue(reason.trim() === reason && length <= 2000 && (passed || length >= 20) && !/[\u0000-\u0009\u000b-\u001f\u007f]/u.test(reason));
    return { round: pending.round, submission_hash: pending.submission_hash, evidence_hash: pending.evidence_hash, passed, reason };
  }
  // Preparation publishes intended public terms, but grants no acceptance or payment consent.
  function cardQuoteRequest(input) {
    fields(input, ['document', 'solver_reward_micro_usdc', 'currency', 'maximum_charge_minor']);
    integer(input.solver_reward_micro_usdc); integer(input.maximum_charge_minor);
    requireValue(['mxn', 'usd'].includes(input.currency));
    const d = input.document;
    fieldsAllowing(d, ['title', 'goal', 'acceptance_criteria', 'benchmark', 'evidence_schema'], ['source_url', 'image']);
    text(d.title, 200); text(d.goal, 4000);
    requireValue(Array.isArray(d.acceptance_criteria) && d.acceptance_criteria.length > 0 && d.acceptance_criteria.length <= 20);
    d.acceptance_criteria.forEach(c => text(c, 1000));
    fields(d.benchmark, ['engine', 'delivery_deadline']);
    requireValue(d.benchmark.engine === 'customer_review_v1'); integer(d.benchmark.delivery_deadline);
    requireValue(d.evidence_schema && typeof d.evidence_schema === 'object' && !Array.isArray(d.evidence_schema));
    requireValue(new TextEncoder().encode(JSON.stringify(input)).length <= 60000);
    return JSON.parse(JSON.stringify({ document: d, solver_reward_micro_usdc: input.solver_reward_micro_usdc }));
  }
  const api = { target, apiOrigin, session, quote, order, acceptanceKey, checkoutURL, money, review, verdict, artifactURL, cardQuoteRequest };
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.ManagedService = Object.freeze(api);
})(typeof window === 'object' ? window : globalThis);
