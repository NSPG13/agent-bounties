(function (root, factory) {
  const api = factory(typeof module === 'object' && module.exports ? require('./marketplace-workflow.js') : root.AgentBountiesWorkflow);
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.AgentBountiesSubmissions = api;
  if (root?.document) api.start(root, root.document);
})(typeof window !== 'undefined' ? window : globalThis, function (flow) {
  'use strict';
  const HASH = /^0x[0-9a-f]{64}$/i;
  const lower = value => String(value || '').toLowerCase();
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  function canonical(item) {
    return item?.network === flow.NETWORK && item.source_type === 'canonical_base' && flow.ADDRESS.test(item.source_id)
      && ['canonical', 'open-competition', 'open-competition-v2'].some(prefix => item.opportunity_id === `${prefix}:${flow.NETWORK}:${item.source_id}`);
  }
  function completed(item) {
    return canonical(item) && item.work_state === 'completed' && item.payment_state === 'paid'
      && item.payment_committed === true && ['paid', 'settled'].includes(item.source_status);
  }
  function pageUrl(item) {
    // Some older clients use a short opportunity ID; always construct the full ID from validated identity.
    if (!flow.ADDRESS.test(item?.source_id) || item.network !== flow.NETWORK) throw new Error('Bounty identity is unavailable.');
    const prefix = flow.isV2(item) ? 'open-competition-v2' : String(item.opportunity_id).startsWith('open-competition:') ? 'open-competition' : 'canonical';
    return `submissions.html?opportunity=${encodeURIComponent(`${prefix}:${flow.NETWORK}:${item.source_id}`)}`;
  }
  function safeUrl(value) {
    try { const url = new URL(value); return ['https:', 'http:'].includes(url.protocol) && !url.username && !url.password ? url.href : null; }
    catch (_) { return null; }
  }
  function txUrl(event) { return HASH.test(event?.tx_hash) ? `https://basescan.org/tx/${event.tx_hash}` : null; }
  async function request(win, path, allowMissing = false) {
    const response = await win.fetch(`${flow.apiBase(win.location)}${path}`, { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer', headers: { Accept: 'application/json' } });
    if (allowMissing && response.status === 404) return null;
    if (!response.ok) throw new Error(`Public records could not load (${response.status}). Try Refresh.`);
    return response.json();
  }
  function projection(payload, paidOnly = false) {
    if (payload?.schema_version !== 'agent-bounties/opportunity-projection-v1' || payload.network !== flow.NETWORK
      || payload.applied_view !== 'recent' || payload.degraded !== false || !Array.isArray(payload.items)
      || payload.source_statuses?.find(source => source.source_type === 'canonical_base')?.available !== true
      || payload.items.some(item => !canonical(item) || (paidOnly && !completed(item)))) throw new Error('Public bounty history is unavailable. Try Refresh.');
    return { payload, items: payload.items };
  }
  const recentPath = '/v1/opportunities?network=base-mainnet&view=recent&source_type=canonical_base';
  async function loadCompleted(win) { return projection(await request(win, `${recentPath}&limit=300&work_state=completed&payment_state=paid`), true); }
  async function loadOpportunity(win, id) {
    if (!/^(canonical|open-competition|open-competition-v2):base-mainnet:0x[0-9a-f]{40}$/i.test(id || '')) throw new Error('Open View Submissions from a bounty card.');
    const { items } = projection(await request(win, `${recentPath}&opportunity_id=${encodeURIComponent(id)}&limit=1`));
    if (items.length > 1 || items.some(item => item.opportunity_id !== id)) throw new Error('Bounty lookup is unavailable. Try Refresh.');
    const item = items[0];
    if (!item) throw new Error('This bounty is not available in the public history.');
    return item;
  }
  function protocol(item) { return item.opportunity_id.split(':')[0]; }
  function sequence(event) { return event.data?.round ?? event.data?.submission_sequence ?? event.data?.sequence ?? event.data?.winning_sequence; }
  function confirmed(event) {
    return Boolean(event?.id) && HASH.test(event.tx_hash) && HASH.test(event.bounty_id)
      && Number.isSafeInteger(event.block_number) && event.block_number >= 0 && Number.isSafeInteger(event.log_index) && event.log_index >= 0;
  }
  function sameSubmission(a, b) {
    return Number.isSafeInteger(sequence(a)) && sequence(a) > 0 && sequence(a) === sequence(b)
      && flow.ADDRESS.test(a.data?.solver) && lower(a.data.solver) === lower(b.data?.solver)
      && ['submission_hash', 'evidence_hash'].every(key => HASH.test(a.data?.[key]) && lower(a.data[key]) === lower(b.data?.[key]))
      && a.bounty_id === b.bounty_id && lower(a.contract_address) === lower(b.contract_address);
  }
  function buildHistory(item, sourceEvents, bountyId) {
    if (!canonical(item) || !HASH.test(bountyId) || !Array.isArray(sourceEvents)) throw new Error('Bounty evidence is incomplete.');
    const version = protocol(item);
    const events = sourceEvents.filter(event => confirmed(event) && lower(event.contract_address) === lower(item.source_id) && event.bounty_id === bountyId
      && (version === 'canonical' || event.protocol_version === `agent-bounties/${version === 'open-competition' ? 'open-competition-v1' : 'open-competition-v2-beta3'}`));
    const unique = [...new Map(events.map(event => [`${event.tx_hash}:${event.log_index}`, event])).values()];
    const submissionKind = version === 'canonical' ? 'submission_added' : version === 'open-competition' ? 'solution_revealed' : 'entry_qualified';
    const settlementKind = version === 'open-competition-v2' ? 'competition_settled' : 'bounty_settled';
    const settlements = unique.filter(event => event.kind === settlementKind);
    const settlement = completed(item) && settlements.length === 1 ? settlements[0] : null;
    const entries = unique.filter(event => event.kind === submissionKind && Number.isSafeInteger(sequence(event)) && sequence(event) > 0
      && flow.ADDRESS.test(event.data?.solver) && HASH.test(event.data?.submission_hash) && HASH.test(event.data?.evidence_hash))
      .sort((a, b) => sequence(b) - sequence(a)).map(event => {
        const rejected = event.data.passed === false || unique.some(result => result.kind === 'submission_rejected'
          && sequence(result) === sequence(event) && lower(result.data?.solver) === lower(event.data.solver));
        const winning = Boolean(!rejected && (version !== 'open-competition' || event.data.passed === true)
          && settlement && sameSubmission(event, settlement)
          && (settlement.block_number > event.block_number || (settlement.block_number === event.block_number && settlement.log_index > event.log_index)));
        return { event, number: sequence(event), winning, settlement: winning ? settlement : null,
          status: winning ? 'Winning submission' : rejected ? 'Did not pass' : completed(item) ? 'Not selected' : 'Submitted', evidence: null, evidenceState: 'missing' };
      });
    // Conflicting submission records cannot produce multiple winning badges.
    if (entries.filter(entry => entry.winning).length > 1) entries.forEach(entry => { entry.winning = false; entry.settlement = null; entry.status = 'Result needs review'; });
    return { entries, winner: entries.find(entry => entry.winning) || null, settlement };
  }
  function matchingEvidence(item, entry, record) {
    return record?.network === flow.NETWORK && lower(record.bounty_contract) === lower(item.source_id)
      && record.bounty_id === entry.event.bounty_id && record.round === entry.number
      && lower(record.solver_wallet) === lower(entry.event.data.solver)
      && lower(record.artifact_hash) === lower(entry.event.data.submission_hash)
      && lower(record.evidence_hash) === lower(entry.event.data.evidence_hash);
  }
  async function loadHistory(win, item) {
    let terms = null, termsUnavailable = false, sourceEvents, bountyId;
    if (protocol(item) === 'canonical') {
      const feed = await request(win, `/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false&bounty_contract=${item.source_id}`);
      if (!Array.isArray(feed) || feed.length > 1 || feed.some(row => lower(row.bounty_contract) !== lower(item.source_id))) throw new Error('Bounty history lookup is unavailable. Try Refresh.');
      const bounty = feed[0];
      if (!bounty) throw new Error('This bounty is not available in the public history.');
      termsUnavailable = bounty.terms_valid !== true || Boolean(bounty.validation_errors?.length) || !HASH.test(item.terms_hash) || bounty.terms_hash !== item.terms_hash || !bounty.terms?.document;
      if (!termsUnavailable) terms = bounty.terms.document;
      sourceEvents = bounty.events; bountyId = bounty.bounty_id;
    } else {
      const path = protocol(item) === 'open-competition' ? 'open-competition-v1' : 'open-competition-v2-beta3';
      const payload = await request(win, `/v1/base/${path}/events?network=base-mainnet&bounty_contract=${item.source_id}`);
      if (payload.network !== flow.NETWORK || !Array.isArray(payload.events) || payload.events.some(event => lower(event.contract_address) !== lower(item.source_id))) throw new Error('Competition records could not be verified.');
      sourceEvents = payload.events;
      const identities = new Set(sourceEvents.filter(event => confirmed(event) && lower(event.contract_address) === lower(item.source_id)).map(event => event.bounty_id));
      if (identities.size !== 1) throw new Error('Competition identity could not be verified.');
      bountyId = [...identities][0];
    }
    const history = buildHistory(item, sourceEvents, bountyId);
    if (protocol(item) === 'canonical') {
      // Bound concurrent reads while preserving every recorded attempt.
      for (let i = 0; i < history.entries.length; i += 4) await Promise.all(history.entries.slice(i, i + 4).map(async entry => {
        try {
          const record = await request(win, `/v1/base/autonomous-bounties/submission-evidence/${item.source_id}/${entry.number}?network=base-mainnet`, true);
          if (record && matchingEvidence(item, entry, record)) { entry.evidence = record; entry.evidenceState = 'available'; }
          else entry.evidenceState = record ? 'mismatch' : 'missing';
        } catch (_) { entry.evidenceState = 'unavailable'; }
      }));
    }
    return { item, terms, termsUnavailable, ...history };
  }
  function link(url, label, className = '') { const safe = safeUrl(url); return safe ? `<a class="${className}" href="${escape(safe)}" target="_blank" rel="noopener noreferrer">${escape(label)} ↗</a>` : ''; }
  function renderEntry(entry, item) {
    const record = entry.evidence;
    const artifact = record && safeUrl(record.artifact_reference);
    const unavailable = entry.evidenceState === 'unavailable' ? 'The work details could not load. Use Refresh to try again.'
      : entry.evidenceState === 'mismatch' ? 'The stored work details do not match this submission. They are hidden until checked.'
        : 'The original work link was not published in the available record. The submission hashes and transaction are shown below.';
    const reason = entry.winning ? `<section class="submission-result"><h3>Why this won</h3><p>${escape(protocol(item) === 'open-competition' ? 'This was the first confirmed passing reveal under the published competition rules.' : protocol(item) === 'open-competition-v2' ? 'The contract selected this qualified entry under its published winner rule.' : 'The bounty’s recorded review accepted this submission and released the reward.')}</p><p>See the original success criteria and review method above. A written review for each criterion is not available in this public record.</p>${link(txUrl(entry.settlement), 'View confirmed payment')}</section>` : '';
    return `<article class="submission-card${entry.winning ? ' submission-winner' : ''}" id="submission-${entry.number}"><header><h2>Submission ${entry.number}</h2><strong>${escape(entry.status)}</strong></header><p>Recorded ${escape(new Date(entry.event.occurred_at).toLocaleString())}</p><p class="submission-wallet">By ${escape(entry.event.data.solver)}</p>${entry.event.data.score !== undefined ? `<p>Recorded score: <strong>${escape(entry.event.data.score)}</strong></p>` : ''}${artifact ? link(artifact, 'Open submitted work', 'market-button market-button-primary') : `<p>${escape(unavailable)}</p>`}${record && !artifact ? `<p>Work reference: <code>${escape(record.artifact_reference)}</code></p>` : ''}${reason}<details><summary>Submission evidence</summary>${record ? `<p>Evidence supplied by the solver. This is not a written verdict from the reviewer.</p><pre>${escape(JSON.stringify(record.evidence, null, 2))}</pre>` : ''}<dl><dt>Submission hash</dt><dd>${escape(entry.event.data.submission_hash)}</dd><dt>Evidence hash</dt><dd>${escape(entry.event.data.evidence_hash)}</dd></dl>${link(txUrl(entry.event), 'View submission transaction')}</details></article>`;
  }
  function reviewMethod(item) {
    const method = item.verification_method || '';
    if (method === 'deterministic_module' || method.startsWith('approved deterministic verifier:')) return 'An automatic test checked the work against the published rules.';
    if (method.startsWith('sp1_')) return 'A proof checked that the work met the published computer-test rules.';
    if (method === 'ai_judge_quorum') return 'The chosen AI reviewers checked the work against the published rules.';
    return method ? `Recorded review method: ${method}` : 'The review method is not available in this record.';
  }
  function renderHistory(data) {
    const { item, terms, termsUnavailable, entries, winner } = data;
    const criteria = Array.isArray(terms?.acceptance_criteria) ? terms.acceptance_criteria : [];
    const rules = terms ? { benchmark: terms.benchmark, review_policy: terms.verification_policy } : item.evidence_requirements;
    const sourceUrl = safeUrl(terms?.source_url || item.source_url);
    const github = sourceUrl && new URL(sourceUrl).hostname === 'github.com';
    const source = link(sourceUrl, github ? 'View proposals on GitHub' : 'Open original bounty source');
    return `<section class="submission-overview"><h1>${escape(item.title)}</h1><p>${escape(item.goal || 'Review the original rules and recorded result below.')}</p><p>${entries.length} recorded ${entries.length === 1 ? 'submission' : 'submissions'}</p>${winner ? `<a class="market-button market-button-primary" href="#submission-${winner.number}">View Winning Submission</a>` : `<p>${completed(item) ? 'Payment is recorded, but a matching winning submission could not be verified here.' : 'No confirmed winner yet.'}</p>`}${source}<p>GitHub proposals and discussion are separate from confirmed submissions.</p></section><section class="submission-criteria"><h2>Success criteria</h2>${criteria.length ? `<ol>${criteria.map(criterion => `<li>${escape(criterion)}</li>`).join('')}</ol>` : termsUnavailable ? '<p>The original criteria could not be verified. Confirmed submissions and payment records are still shown below.</p>' : '<p>A written checklist was not published in this record. The available test rules are below.</p>'}<h3>How the work was checked</h3><p>${escape(reviewMethod(item))}</p>${item.decision_authority ? `<details><summary>Who decided the result?</summary><p>${escape(item.decision_authority)}</p></details>` : ''}${item.competition_mode === 'best_score' ? '<p>Winner rule: best qualifying score under the published policy.</p>' : ''}${termsUnavailable ? '' : `<details><summary>Original test and review rules</summary><pre>${escape(JSON.stringify(rules, null, 2))}</pre></details>`}</section>${entries.length ? entries.map(entry => renderEntry(entry, item)).join('') : '<section class="submission-card"><h2>No confirmed submissions yet</h2><p>Once a solution is submitted and recorded, it will appear here. You can still read proposals in the original discussion.</p></section>'}`;
  }
  async function start(win, doc) {
    const page = doc.querySelector('[data-submissions-page]');
    if (!page) return;
    const status = doc.querySelector('[data-submissions-status]'), refresh = doc.querySelector('[data-submissions-refresh]');
    let busy = false;
    async function load() {
      if (busy) return;
      busy = true; refresh.disabled = true; page.setAttribute('aria-busy', 'true'); page.innerHTML = '<h1>Bounty submissions</h1>'; status.textContent = 'Loading public submissions…';
      try {
        const item = await loadOpportunity(win, new URLSearchParams(win.location.search).get('opportunity'));
        page.innerHTML = renderHistory(await loadHistory(win, item)); status.textContent = 'Public records checked. Private work is not shown here.';
        doc.title = `Submissions: ${item.title} | AgentBounties.app`;
      } catch (error) { status.textContent = error.message; }
      finally { busy = false; refresh.disabled = false; page.setAttribute('aria-busy', 'false'); }
    }
    refresh.addEventListener('click', load);
    await load();
  }
  return { canonical, completed, pageUrl, safeUrl, projection, loadCompleted, loadOpportunity, buildHistory, matchingEvidence, loadHistory, renderHistory, start };
});
