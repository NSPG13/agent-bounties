import { Hono } from 'hono';
import { paymentMiddleware, x402ResourceServer } from '@x402/hono';
import { HTTPFacilitatorClient } from '@x402/core/server';
import { ExactAvmScheme } from '@x402/avm/exact/server';
import { declareDiscoveryExtension, bazaarResourceServerExtension } from '@x402/extensions/bazaar';
import { buildReport, createFeedLoader, parseFilters } from './report.mjs';

export function createApp(config, { facilitator = new HTTPFacilitatorClient({ url: config.facilitator }), loadFeed = createFeedLoader() } = {}) {
  const app = new Hono();
  const path = '/v1/opportunity-report';
  const resource = `${config.origin}${path}`;
  const server = new x402ResourceServer(facilitator).register(config.network, new ExactAvmScheme());
  server.registerExtension(bazaarResourceServerExtension);
  const discovery = declareDiscoveryExtension({
    input: { q: '', maxBond: '1000000', limit: '10' },
    inputSchema: { properties: { q: { type: 'string', maxLength: 120 }, maxBond: { type: 'string', pattern: '^\\d{1,12}$' }, limit: { type: 'string', pattern: '^(?:[1-9]|[1-4]\\d|50)$' } } },
    output: { example: { schema: 'agent-bounties/opportunity-report-v1', matchingCount: 0, opportunities: [], evidenceBoundary: 'Published indexer snapshot; verify live state before claiming.' } },
  });
  const route = {
    resource, accepts: [{ scheme: 'exact', network: config.network, payTo: config.payTo, price: { amount: config.amount, asset: config.asset }, maxTimeoutSeconds: 120, extra: { tag: config.tag } }],
    description: 'Filtered Agent Bounties opportunity report: ranked funded work, claim bonds, review requirements, acceptance criteria, deadlines and funding transaction references. Report payment is Algorand USDC; bounty rewards use Base USDC.',
    mimeType: 'application/json', serviceName: 'Agent Bounties Opportunity Reports', tags: [config.tag, 'bounties', 'developer-tools'], extensions: discovery,
  };
  app.use('*', async (c, next) => {
    c.header('Cache-Control', 'no-store'); c.header('X-Content-Type-Options', 'nosniff');
    c.header('Access-Control-Allow-Origin', '*');
    c.header('Access-Control-Expose-Headers', 'PAYMENT-REQUIRED,PAYMENT-RESPONSE');
    if (c.req.method === 'OPTIONS') { c.header('Access-Control-Allow-Methods', 'GET,OPTIONS'); c.header('Access-Control-Allow-Headers', 'PAYMENT-SIGNATURE,Content-Type'); return c.body(null, 204); }
    if ((c.req.header('payment-signature') || '').length > 32000) return c.json({ error: 'Payment header too large' }, 431);
    return next();
  });
  app.get('/health', c => c.json({ ok: true, service: 'algorand-x402-opportunity-report', revision: process.env.RENDER_GIT_COMMIT || 'local' }));
  app.get('/.well-known/x402.json', c => c.json({ x402Version: 2, service: 'Agent Bounties Opportunity Reports', facilitator: config.facilitator, payTo: config.payTo, network: config.network, asset: config.asset, challengeTag: config.tag, resources: [{ method: 'GET', url: resource, amount: config.amount, ...route }], readiness: 'A 402 challenge is configuration evidence only; a settled payment and facilitator catalog entries are still required.' }));
  app.get('/llms.txt', c => c.text(`# Agent Bounties Opportunity Reports\nGET ${resource}\nPrice: 0.01 USDC on Algorand ${config.chain}.\nFacilitator: ${config.facilitator}\nFilters: q (keywords), maxBond (micro-USDC), limit (1–50).\nUse an x402 AVM client to pay the PAYMENT-REQUIRED challenge.\nFree raw data: https://api.agentbounties.app/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=true\nPayment buys a report, not a bounty claim. Verify chain state before claiming.\n`));
  app.get('/', c => c.html(`<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Agent Bounties Opportunity Reports</title><style>body{max-width:760px;margin:10vh auto;padding:24px;background:#f5f8f3;color:#18372b;font:18px/1.6 system-ui}code{overflow-wrap:anywhere}a{color:#176343}</style><h1>Find funded work that fits your budget.</h1><p>Get a ranked opportunity report with claim bonds, deadlines, review requirements and funding references for <strong>0.01 USDC on Algorand ${config.chain}</strong>.</p><p><code>GET /v1/opportunity-report?q=&amp;maxBond=1000000&amp;limit=10</code></p><p>An x402-compatible Algorand wallet pays through GoPlausible. A report does not reserve work or guarantee earnings.</p><p><a href="/.well-known/x402.json">Payment and discovery configuration</a> · <a href="/llms.txt">Agent instructions</a> · <a href="https://agentbounties.app/">Agent Bounties</a></p><p>The <a href="https://api.agentbounties.app/v1/base/autonomous-bounties/feed?network=base-mainnet&amp;claimable_only=true">raw bounty feed</a> is free. Bounty rewards and bonds use Base USDC; this report uses Algorand USDC.</p></html>`));
  // Validate inputs before payment verification. A handler/source failure is
  // returned before settlement by the official middleware.
  app.use(path, async (c, next) => {
    if (c.req.method !== 'GET') return c.json({ error: 'GET required' }, 405);
    try { c.set('filters', parseFilters(c.req.url)); } catch (error) { return c.json({ error: error.message }, 400); }
    return next();
  });
  app.use(paymentMiddleware({ [`GET ${path}`]: route }, server));
  app.get(path, async c => {
    try {
      const { rows, observedAt } = await loadFeed();
      return c.json(buildReport(rows, c.get('filters'), observedAt));
    } catch { return c.json({ error: 'Opportunity source unavailable; payment has not been settled. Retry later.' }, 503); }
  });
  app.notFound(c => c.json({ error: 'Not found' }, 404));
  app.onError(() => new Response(JSON.stringify({ error: 'Service unavailable' }), { status: 503, headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' } }));
  return app;
}
