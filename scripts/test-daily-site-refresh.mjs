import { test } from 'node:test';
import assert from 'node:assert/strict';
import worker from '../ops/cloudflare/daily-site-refresh.mjs';

test('HTTP callers cannot request builds', async () => {
  assert.equal((await worker.fetch()).status, 404);
});
test('only the scoped Cloudflare hook receives a scheduled POST', async () => {
  const original = globalThis.fetch;
  const requests = [];
  globalThis.fetch = async (url, options) => { requests.push({url:String(url), options}); return {ok:true}; };
  try {
    await assert.rejects(worker.scheduled({}, {PAGES_DEPLOY_HOOK:'https://attacker.invalid/hook'}));
    assert.equal(requests.length, 0);
    await worker.scheduled({}, {PAGES_DEPLOY_HOOK:'https://api.cloudflare.com/client/v4/pages/webhooks/deploy_hooks/fixture'});
    assert.equal(requests.length, 1);
    assert.equal(requests[0].options.method, 'POST');
    assert.equal(requests[0].options.redirect, 'error');
  } finally { globalThis.fetch = original; }
});
test('failed scheduled request remains a visible failure', async () => {
  const original = globalThis.fetch;
  globalThis.fetch = async () => ({ok:false});
  try {
    await assert.rejects(worker.scheduled({}, {PAGES_DEPLOY_HOOK:'https://api.cloudflare.com/client/v4/pages/webhooks/deploy_hooks/fixture'}));
  } finally { globalThis.fetch = original; }
});
