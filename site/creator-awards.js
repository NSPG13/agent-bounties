(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.AgentBountiesAwards = api;
  if (root?.document) api.start(root, root.document);
})(typeof window !== 'undefined' ? window : globalThis, function () {
  'use strict';
  // Reviewed public receipts, separate from every canonical settlement aggregate.
  const DATA = 'data/creator-awards.json';
  const ADDRESS = /^0x[0-9a-f]{40}$/i, HASH = /^0x[0-9a-f]{64}$/i;
  const TOKEN = '0x833589fcd6edb6e08f4c7c32d4f71b54bda02913';
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  function link(url, label) {
    try { const u = new URL(url); if (u.protocol === 'https:' && !u.username && !u.password) return `<a href="${escape(u.href)}" target="_blank" rel="noopener noreferrer">${escape(label)} ↗</a>`; } catch (_) { /* Unusable evidence stays unlinked. */ }
    return escape(label);
  }
  const pageUrl = award => `submissions.html?award=${encodeURIComponent(award.id)}`;
  const receiptUrl = award => `https://basescan.org/tx/${award.payment.transaction_hash}`;
  function amount(units) {
    const value = BigInt(units), fraction = (value % 1000000n).toString().padStart(6, '0').replace(/0+$/, '');
    return `${value / 1000000n}${fraction ? `.${fraction}` : ''} USDC`;
  }
  function validate(payload) {
    if (payload?.schema_version !== 'agent-bounties/creator-awards-v1' || !Array.isArray(payload.awards)) throw new Error('Award records are unavailable.');
    const ids = new Set(), payments = new Set();
    for (const row of payload.awards) {
      const p = row?.payment;
      if (!/^[a-z0-9-]+$/.test(row?.id || '') || !row.title || row.visibility !== 'public'
        || row.kind !== 'separate_creator_award' || row.status !== 'paid_confirmed'
        || p?.chain_id !== 8453 || p.token?.toLowerCase() !== TOKEN || p.receipt_status !== 'success' || p.event !== 'Transfer'
        || !HASH.test(p.transaction_hash) || !ADDRESS.test(p.from_wallet) || !ADDRESS.test(p.to_wallet)
        || !Number.isSafeInteger(p.log_index) || p.log_index < 0 || !Number.isSafeInteger(p.block_number) || p.block_number < 1
        || typeof p.amount_base_units !== 'string' || !/^[1-9][0-9]{0,77}$/.test(p.amount_base_units)
        || !Number.isFinite(Date.parse(p.occurred_at)) || !Number.isFinite(Date.parse(row.reviewed_at))
        || Date.parse(row.reviewed_at) < Date.parse(p.occurred_at)
        || !Array.isArray(row.bounty_contracts) || row.bounty_contracts.some(c => !ADDRESS.test(c))
        || !row.winner?.name || !/^[0-9a-f]{40}$/.test(row.winner.revision || '')
        || !Array.isArray(row.criteria) || !Array.isArray(row.reviews)) throw new Error('An award record could not be verified.');
      const key = `${p.chain_id}:${p.transaction_hash.toLowerCase()}:${p.log_index}`;
      if (ids.has(row.id) || payments.has(key)) throw new Error('Duplicate award evidence needs review.');
      ids.add(row.id); payments.add(key);
    }
    return payload.awards;
  }
  function summarize(rows, window = null, now = Date.now()) {
    const start = window ? Date.parse(window.started_at) : 0, end = window ? Date.parse(window.ended_at) : now + 1;
    if (!Number.isFinite(start) || !Number.isFinite(end) || start >= end) throw new Error('The reporting dates are unavailable.');
    const selected = rows.filter(row => { const at = Date.parse(row.payment.occurred_at); return at >= start && at < end && at <= now; });
    return { rows: selected, count: selected.length, amount_base_units: selected.reduce((sum, row) => sum + BigInt(row.payment.amount_base_units), 0n).toString() };
  }
  function related(rows, id) {
    const match = /^canonical:base-mainnet:(0x[0-9a-f]{40})$/i.exec(id || '');
    return match ? rows.filter(row => row.bounty_contracts.some(c => c.toLowerCase() === match[1].toLowerCase())) : [];
  }
  let pending;
  function load(win) {
    if (!pending) pending = (async () => {
      const response = await win.fetch(DATA, { cache: 'no-store', credentials: 'omit', headers: { Accept: 'application/json' } });
      if (!response.ok) throw new Error('Award records could not load. Try Refresh.');
      return validate(await response.json());
    })().catch(error => { pending = null; throw error; });
    return pending;
  }
  function card(row) {
    return `<article class="creator-award-card"><h3>${escape(row.title)}</h3><p><strong>${amount(row.payment.amount_base_units)} paid separately</strong> · Direct creator award</p><p>The contract expired. The creator paid a reviewed GitHub design separately. Read why it was chosen and see every review.</p><a class="market-button market-button-primary" href="${pageUrl(row)}">View Submissions</a></article>`;
  }
  function renderDetail(row) {
    const w = row.winner, p = row.payment;
    const image = /^assets\/cad-award\/[a-z0-9-]+\.png$/.test(w.image || '') ? `<figure><img class="creator-award-image" src="${escape(w.image)}" alt="Reviewed CAD model of the flower-shaped collector, with petals, central funnel and tank mount"><figcaption>Rendered from the reviewed CAD files. Colors are for display.</figcaption></figure>` : '';
    return `<section class="submission-overview"><p class="market-eyebrow">Paid separately · Creator award</p><h1>${escape(row.title)}</h1><p><strong>${amount(p.amount_base_units)} paid to ${escape(w.name)} on Base.</strong> ${link(receiptUrl(row), 'View payment')}</p><p>${escape(row.why_separate)}</p><p>${escape(row.eligibility_note)}</p><a class="market-button market-button-primary" href="#winning-design">View Winning Submission</a></section>
      <section class="submission-card submission-winner" id="winning-design"><h2>Winning design: B, by ${escape(w.name)}</h2>${image}<h3>Why B was chosen</h3><p>${escape(row.why_selected)}</p><p>${link(w.source_url, 'Open the winning files')} · ${link(w.review_url, 'Read the technical review')}</p><details><summary>Exact reviewed version</summary><p><code>${escape(w.revision)}</code></p>${link(w.proposal_url, 'Original proposal')}</details></section>
      <section class="submission-criteria"><h2>The three success checks</h2><ol>${row.criteria.map(c => `<li><strong>${escape(c.title)} — Pass.</strong><p>${escape(c.finding)}</p></li>`).join('')}</ol><p>${escape(row.limits)}</p></section>
      <section class="submission-card"><h2>What the other submissions were missing</h2><p>A also passed. The remaining distinct designs or packages needed fixes or were incomplete. Technical review and contract eligibility are separate checks.</p><p>${escape(row.audit_scope)}</p><details><summary>See every review</summary>${row.reviews.map(r => `<article class="creator-award-review"><h3>${escape(r.name)} · ${link(r.source_url, r.reference)}</h3><p>Files: <strong>${escape(r.checks[0])}</strong> · Water path and mount: <strong>${escape(r.checks[1])}</strong> · Assembly information: <strong>${escape(r.checks[2])}</strong></p><p>${escape(r.summary)}</p>${r.review_url ? link(r.review_url, 'Review findings') : ''}${r.revision ? `<small>Reviewed version: ${escape(r.revision)}</small>` : ''}</article>`).join('')}</details></section>
      <section class="submission-card" id="award-payment"><h2>Payment and record</h2><p>Paid ${escape(new Date(p.occurred_at).toISOString().replace('T', ' ').replace('.000Z', ' UTC'))}. ${link(receiptUrl(row), 'Successful USDC transfer')} · ${link(row.refund_url, 'Earlier bounty refund')}</p><p>${escape(row.budget_note)}</p><p>Receipt checked ${escape(new Date(row.reviewed_at).toISOString().slice(0, 10))}. This is one separate creator award. It adds no contract settlement, protocol GMV, platform revenue or completed contract bounty.</p><details><summary>Check the payment details</summary><dl><dt>Amount</dt><dd>${amount(p.amount_base_units)}</dd><dt>From</dt><dd>${escape(p.from_wallet)}</dd><dt>To</dt><dd>${escape(p.to_wallet)}</dd><dt>Native USDC token on Base (8453)</dt><dd>${escape(p.token)}</dd><dt>Transaction / transfer log</dt><dd>${escape(p.transaction_hash)} / ${p.log_index}</dd><dt>Block</dt><dd>${p.block_number}</dd></dl>${link(w.confirmation_url, 'Recipient’s public address confirmation')} · ${link(row.announcement_url, 'Payment announcement')}</details><p><a href="metrics.html#creator-awards">View separate award metrics</a> · <a href="${DATA}">Download the public award record</a></p></section>`;
  }
  function setWindow(doc, window) {
    const node = doc.querySelector('[data-direct-awards="metrics"]');
    if (node) { node.awardWindow = window; node.renderAwards?.(); }
  }
  function start(win, doc) {
    const hosts = [...doc.querySelectorAll('[data-direct-awards]')];
    if (!hosts.length) return;
    let rows = null, error = null;
    function render() {
      for (const host of hosts) {
        const mode = host.dataset.directAwards;
        const board = mode === 'board', isRelated = mode === 'related';
        host.hidden = board && doc.querySelector('[data-market-timing]')?.value !== 'completed';
        if (host.hidden) continue;
        const id = new URLSearchParams(win.location.search).get('opportunity');
        if (isRelated && !/^canonical:base-mainnet:0x[0-9a-f]{40}$/i.test(id || '')) { host.hidden = true; continue; }
        if (error || !rows) { host.textContent = error ? 'Separate award records could not load. Refresh this page to try again.' : 'Loading separate award records…'; continue; }
        let selected = summarize(rows).rows;
        if (isRelated) { selected = related(selected, id); host.hidden = !selected.length; }
        if (board) {
          const search = doc.querySelector('[data-market-search]')?.value.trim().toLowerCase() || '';
          const kind = doc.querySelector('[data-market-kind]')?.value || 'all';
          selected = selected.filter(row => ['all', 'direct'].includes(kind) && `${row.title} ${row.winner.name} CAD rainwater`.toLowerCase().includes(search));
        }
        if (mode === 'metrics') {
          if (!host.awardWindow) { host.innerHTML = '<h2>Separate creator awards</h2><p>Reporting dates are unavailable. <a href="earn.html?view=completed">Browse the award records</a>.</p>'; continue; }
          try { selected = summarize(rows, host.awardWindow).rows; } catch (_) { host.textContent = 'Separate award reporting dates are unavailable.'; continue; }
        }
        const sum = summarize(selected);
        host.innerHTML = `<h2>${isRelated ? 'This bounty has a separate award' : 'Separate creator awards'}</h2><p><strong>${sum.count} paid ${sum.count === 1 ? 'award' : 'awards'} · ${amount(sum.amount_base_units)}</strong>${mode === 'metrics' ? ' in the selected period' : ' · All time'}</p><p>Direct payments reviewed by maintainers. Shown separately from contract payouts and completed bounties.</p>${selected.map(row => mode === 'summary' || mode === 'metrics' ? `<p><a href="${pageUrl(row)}">${escape(row.title)} — see the winning work</a>${mode === 'metrics' ? ` · ${link(receiptUrl(row), amount(row.payment.amount_base_units))}` : ''}</p>` : card(row)).join('')}${board && !selected.length ? '<p>No separate awards match these filters.</p>' : ''}${mode === 'metrics' ? `<small>Only reviewed public awards are listed. <a href="${DATA}">Source records</a>. These payments are excluded from the contract totals and charts above.</small>` : ''}`;
      }
    }
    hosts.forEach(host => { host.renderAwards = render; });
    for (const [selector, event] of [['[data-market-timing]', 'change'], ['[data-market-kind]', 'change'], ['[data-market-search]', 'input']]) doc.querySelector(selector)?.addEventListener(event, render);
    async function refresh() { try { rows = await load(win); error = null; } catch (e) { error = e; rows = null; } render(); }
    doc.querySelector('[data-market-refresh]')?.addEventListener('click', refresh);
    doc.querySelector('[data-submissions-refresh]')?.addEventListener('click', refresh);
    render(); void refresh();
  }
  return { validate, summarize, related, amount, pageUrl, receiptUrl, load, renderDetail, setWindow, start };
});
