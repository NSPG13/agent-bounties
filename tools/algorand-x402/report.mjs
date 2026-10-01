import { createHash } from 'node:crypto';
import { FEED_URL } from './config.mjs';

function units(value) {
  if (!/^\d{1,20}$/.test(String(value ?? ''))) throw new Error('Invalid source amount');
  return BigInt(value);
}
function usd(value) {
  const n = units(value);
  return `${n / 1000000n}.${String(n % 1000000n).padStart(6, '0')}`;
}
export function parseFilters(url) {
  const q = new URL(url).searchParams;
  if ([...q.keys()].some(k => !['q', 'maxBond', 'limit'].includes(k))) throw new Error('Unknown query parameter');
  for (const k of q.keys()) if (q.getAll(k).length !== 1) throw new Error('Duplicate query parameter');
  const query = q.get('q') || '';
  const maxBond = q.get('maxBond') || '1000000';
  const limit = q.get('limit') || '10';
  if (query.length > 120 || !/^\d{1,12}$/.test(maxBond) || !/^(?:[1-9]|[1-4]\d|50)$/.test(limit)) throw new Error('Invalid filter: q <= 120 characters; maxBond in micro-USDC; limit 1–50');
  return { query, maxBond, limit: Number(limit) };
}

export function buildReport(rows, filters, observedAt = new Date().toISOString()) {
  if (!Array.isArray(rows) || rows.length > 10000) throw new Error('Invalid upstream feed');
  const terms = filters.query.toLowerCase().split(/\s+/).filter(Boolean);
  const candidates = [];
  for (const row of rows) {
    const doc = row.terms?.document;
    if (row.status !== 'claimable' || row.terms_valid !== true || row.verification_ready !== true || !doc) continue;
    if (!/^0x[\da-fA-F]{40}$/.test(row.bounty_contract || '')) continue;
    const bond = units(row.claim_bond), reward = units(row.solver_reward);
    const target = units(row.target_amount), funded = units(row.funded_amount);
    if (!target || funded < target || !reward || bond > BigInt(filters.maxBond)) continue;
    const title = String(doc.title || 'Untitled bounty');
    const goal = String(doc.goal || '');
    if (!terms.every(term => `${title} ${goal}`.toLowerCase().includes(term))) continue;
    const deadline = doc.benchmark?.delivery_deadline;
    if (deadline != null && (!Number.isSafeInteger(deadline) || deadline * 1000 <= Date.parse(observedAt))) continue;
    const fundingEvidence = (row.events || []).filter(e => ['funding_added', 'bounty_became_claimable'].includes(e.kind)).map(e => ({ kind: e.kind, transaction: e.tx_hash, block: e.block_number }));
    if (!fundingEvidence.some(e => e.kind === 'bounty_became_claimable' && /^0x[\da-fA-F]{64}$/.test(e.transaction || '') && Number.isSafeInteger(e.block))) continue;
    candidates.push({
      contract: row.bounty_contract, title, goal, rewardUSDC: usd(reward), bondUSDC: usd(bond),
      fundedUSDC: usd(funded), requiredExternalSpendUSDC: usd(row.required_external_spend), termsHash: row.terms_hash,
      review: { mode: row.verification_mode, runner: row.runner_identifier, disclosure: row.verification_readiness_reason },
      deliveryDeadline: deadline ? new Date(deadline * 1000).toISOString() : null,
      claimWindowSeconds: doc.contract_terms?.claim_window_seconds ?? null,
      acceptanceCriteria: doc.acceptance_criteria || [], fundingEvidence,
      inspectContract: `https://basescan.org/address/${row.bounty_contract}`,
    });
  }
  candidates.sort((a, b) => Number(b.rewardUSDC) - Number(a.rewardUSDC) || a.contract.localeCompare(b.contract));
  return {
    schema: 'agent-bounties/opportunity-report-v1', observedAt, source: FEED_URL,
    sourceSha256: createHash('sha256').update(JSON.stringify(rows)).digest('hex'),
    filters, matchingCount: candidates.length, opportunities: candidates.slice(0, filters.limit),
    evidenceBoundary: 'A snapshot of the published Base indexer, not independent chain verification or a reservation. Recheck live terms and chain state before claiming. Rewards are conditional; bond and required work can be lost. Algorand payment purchases this report only.',
  };
}

export function createFeedLoader(fetchImpl = fetch) {
  let cached, expires = 0, pending;
  return async () => {
    if (cached && Date.now() < expires) return cached;
    if (pending) return pending;
    pending = (async () => {
      const response = await fetchImpl(FEED_URL, { signal: AbortSignal.timeout(12000), redirect: 'error' });
      if (!response.ok) throw new Error('Upstream unavailable');
      const reader = response.body.getReader();
      const chunks = []; let size = 0;
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        size += value.length;
        if (size > 8000000) { await reader.cancel(); throw new Error('Upstream response too large'); }
        chunks.push(Buffer.from(value));
      }
      const rows = JSON.parse(Buffer.concat(chunks).toString());
      if (!Array.isArray(rows)) throw new Error('Invalid upstream feed');
      cached = { rows, observedAt: new Date().toISOString() }; expires = Date.now() + 15000;
      return cached;
    })();
    try { return await pending; } finally { pending = undefined; }
  };
}
