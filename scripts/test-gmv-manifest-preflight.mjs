import test from 'node:test';
import assert from 'node:assert/strict';
import { preflight } from './gmv-manifest-preflight.mjs';
const address = n => '0x' + n.repeat(40), hash = n => '0x' + n.repeat(64);
const policy = { competition: address('1'), bounty_id: hash('1'), epoch_id: hash('2'), verification_policy_hash: hash('3'), window_start: 100, window_end: 200, excluded_wallets: [address('9')], excluded_bounty_contracts: [address('9')] };
const payload = () => ({ schema: 'https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json', network: 'base-mainnet', chain_id: 8453, competition: policy.competition, bounty_id: policy.bounty_id, epoch_id: policy.epoch_id, verification_policy_hash: policy.verification_policy_hash, entrant: address('2'), settlement: { creator: address('3'), solver: address('4'), funder: address('2'), child_bounty_contract: address('5'), settled_at: 150, tx_hash: hash('4'), log_index: 0, gmv_base_units: '9007199254740993' } });
test('valid structure, caller booleans and large exact amounts never establish eligibility', () => {
  const result = preflight({ ...payload(), ready: true, eligible: true, canonical: true }, policy);
  assert.equal(result.structure_valid, true); assert.equal(result.ready, false); assert.equal(result.eligible, false);
  assert.equal(result.status, 'settlement_unverified');
});
test('entrant-solver overlap, another funder, exclusions and malformed identities fail closed', () => {
  for (const update of [{ solver: address('2') }, { funder: address('6') }, { creator: address('4') }, { solver: address('9') }, { child_bounty_contract: address('9') }, { solver: 'alice' }, { tx_hash: 'broadcast' }, { log_index: -1 }, { log_index: 0.5 }, { settled_at: 200 }, { settled_at: '150' }]) {
    const p = payload(); Object.assign(p.settlement, update); assert.equal(preflight(p, policy).structure_valid, false, JSON.stringify(update));
  }
});
test('money uses bounded integer strings without coercion', () => {
  for (const value of [1, 1.5, true, null, '1e3', '-1', '01', '0', (1n << 256n).toString(), '1'.repeat(10000)]) {
    const p = payload(); p.settlement.gmv_base_units = value; assert.equal(preflight(p, policy).structure_valid, false);
  }
});
test('duplicate events, unknown policy and malformed roots cannot pass', () => {
  const p = payload(); p.settlements = [p.settlement, { ...p.settlement }];
  assert.equal(preflight(p, policy).structure_valid, false);
  assert.equal(preflight(payload(), { ...policy, epoch_id: hash('8') }).structure_valid, false);
  for (const root of [null, [], 'yes', true]) assert.equal(preflight(root, policy).structure_valid, false);
});
test('zero identifiers are rejected for either prefix casing', () => {
  for (const prefix of ['0x', '0X']) {
    for (const field of ['competition', 'bounty_id', 'epoch_id', 'verification_policy_hash']) {
      const zero = prefix + '0'.repeat(field === 'competition' ? 40 : 64);
      assert.equal(preflight({ ...payload(), [field]: zero }, { ...policy, [field]: zero }).structure_valid, false, `${prefix} ${field}`);
    }
    for (const field of ['creator', 'solver', 'funder', 'child_bounty_contract', 'tx_hash']) {
      const p = payload(); p.settlement[field] = prefix + '0'.repeat(field === 'tx_hash' ? 64 : 40);
      if (field === 'funder') p.entrant = p.settlement.funder;
      assert.equal(preflight(p, policy).structure_valid, false, `${prefix} ${field}`);
    }
  }
  const p = payload(); p.entrant = p.entrant.toUpperCase(); p.settlement.funder = p.entrant;
  p.competition = p.competition.toUpperCase(); p.settlement.tx_hash = p.settlement.tx_hash.toUpperCase();
  assert.equal(preflight(p, policy).structure_valid, true, 'nonzero uppercase identifiers remain supported');
});
