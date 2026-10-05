import assert from 'node:assert/strict';
import test from 'node:test';
import {AgentBountiesClient} from '../dist/index.js';

test('autonomous-v2 quote and creation plans post the exact routes and bodies', async () => {
  const previous = globalThis.fetch, calls = [];
  const quote = {
    protocol_version: 'agent-bounties/autonomous-v2', solver_reward: '1000000', verifier_reward: '100000',
    claim_bond: '100000', platform_fee_bps: 750, platform_fee: '75000',
    platform_fee_recipient: '0x884834e884d6e93462655a2820140ad03e6747bc', target_amount: '1175000',
    fee_boundary: 'fee is paid by the poster',
  };
  globalThis.fetch = async (url, init) => { calls.push({url, init}); return new Response(JSON.stringify(quote), {status: 200}); };
  try {
    const client = new AgentBountiesClient({baseUrl: 'https://api.example'});
    const reward = {amount: 1_000_000, currency: 'USDC'}, verifier = {amount: 100_000, currency: 'USDC'};
    const result = await client.quoteAutonomousV2Bounty(reward, verifier, 'base-sepolia');
    assert.equal(result.target_amount, '1175000');
    assert.equal(BigInt(result.target_amount), BigInt(result.solver_reward) + BigInt(result.verifier_reward) + BigInt(result.platform_fee));
    const create = {creator: '0x' + '11'.repeat(20), terms_hash: '0x' + '22'.repeat(32)};
    await client.planAutonomousV2BountyCreation(create);
    const signature = {v: 27, r: '0x' + '33'.repeat(32), s: '0x' + '44'.repeat(32)};
    await client.planAutonomousV2BountyAuthorizedCreation(create, signature, 'base-sepolia', '0x' + '55'.repeat(20));
    assert.deepEqual(calls.map(({url}) => new URL(url).pathname), [
      '/v1/base/autonomous-bounties/v2/quote',
      '/v1/base/autonomous-bounties/v2/creation-plan',
      '/v1/base/autonomous-bounties/v2/authorized-creation-plan',
    ]);
    for (const {init} of calls) assert.equal(init.method, 'POST');
    assert.deepEqual(JSON.parse(calls[0].init.body), {network: 'base-sepolia', solver_reward: reward, verifier_reward: verifier});
    assert.deepEqual(JSON.parse(calls[1].init.body), {network: null, create});
    assert.deepEqual(JSON.parse(calls[2].init.body), {network: 'base-sepolia', create, signature, relayer: '0x' + '55'.repeat(20)});
  } finally { globalThis.fetch = previous; }
});
