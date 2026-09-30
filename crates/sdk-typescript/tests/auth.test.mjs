import assert from 'node:assert/strict';
import test from 'node:test';
import { AgentBountiesClient } from '../dist/index.js';

test('wallet auth preserves chain, scopes headers, idempotency, and clears revoked session', async () => {
  const previous = globalThis.fetch;
  const calls = [];
  const token = `abws_${'a'.repeat(64)}`;
  const challenge = {challenge_id:'00000000-0000-4000-8000-000000000001',chain_id:84532,address:'0x1111111111111111111111111111111111111111',message:'fixed SIWE message',expires_at:'2030-01-01T00:00:00Z'};
  globalThis.fetch = async (url, init) => {
    calls.push({url,init});
    const response = url.endsWith('/challenge') ? challenge : url.endsWith('/wallet/session') ? {token,token_type:'Bearer',expires_at:challenge.expires_at,principal:{kind:'wallet',chain_id:84532,address:challenge.address}} : {};
    return new Response(JSON.stringify(response), {status:200});
  };
  try {
    const client = new AgentBountiesClient({baseUrl:'https://api.example'});
    const returned = await client.createWalletChallenge(challenge.address, 84532);
    const session = await client.createWalletSession(returned, '0x1234');
    assert.deepEqual(JSON.parse(calls[1].init.body), {challenge_id:challenge.challenge_id,chain_id:84532,signature:'0x1234'});
    client.setSessionToken(session.token);
    await client.registerAgent('test',challenge.address,'stable-registration');
    assert.equal(calls[2].init.headers.authorization,`Bearer ${token}`);
    assert.equal(calls[2].init.redirect,'error');
    assert.equal(calls[2].init.headers['Idempotency-Key'],'stable-registration');
    await client.revokeSession();
    await client.getSessionStatus();
    assert.equal(calls[3].init.headers.authorization,`Bearer ${token}`);
    assert.equal(calls[4].init.headers.authorization,undefined);
    assert.throws(() => new AgentBountiesClient({operatorApiToken:'operator',sessionToken:token}),/Choose either/);
    assert.throws(() => client.setSessionToken('not-a-token'),/Invalid wallet session/);
  } finally {globalThis.fetch=previous;}
});
