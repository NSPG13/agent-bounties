#!/usr/bin/env node
import fs from 'fs';
import path from 'path';

function isBuffer32Hex(s) {
  if (typeof s !== 'string') return false;
  let hex = s.startsWith('0x') ? s.slice(2) : s;
  return /^[0-9a-fA-F]{64}$/.test(hex);
}

function toBigInt(v) {
  if (typeof v === 'bigint') return v;
  if (typeof v === 'number') return BigInt(v);
  if (typeof v === 'string') {
    if (/^-?\d+$/.test(v) || /^0x[0-9a-fA-F]+$/.test(v)) return BigInt(v);
  }
  return null;
}

function main() {
  const args = process.argv.slice(2);
  if (args.length !== 3) {
    process.stdout.write('{"paid":false}\n');
    return;
  }
  const [jsonPath, expectedContract, expectedSolver] = args;

  let raw;
  try {
    raw = fs.readFileSync(path.resolve(jsonPath), 'utf8');
  } catch {
    process.stdout.write('{"paid":false}\n');
    return;
  }

  let events;
  try {
    events = JSON.parse(raw);
  } catch {
    process.stdout.write('{"paid":false}\n');
    return;
  }

  if (!Array.isArray(events)) {
    process.stdout.write('{"paid":false}\n');
    return;
  }

  const ec = expectedContract.toLowerCase();
  const es = expectedSolver.toLowerCase();

  const matching = events.filter(ev => {
    return ev &&
      (ev.status === 'bounty_settled' || ev.status === 'bountySettled') &&
      typeof ev.contract === 'string' && ev.contract.toLowerCase() === ec &&
      typeof ev.solver === 'string' && ev.solver.toLowerCase() === es;
  });

  if (matching.length !== 1) {
    process.stdout.write('{"paid":false}\n');
    return;
  }

  const ev = matching[0];

  // Validate all required fields
  if (!ev.bountyId || !isBuffer32Hex(ev.bountyId)) {
    process.stdout.write('{"paid":false}\n');
    return;
  }
  if (!ev.transactionHash || !isBuffer32Hex(ev.transactionHash)) {
    process.stdout.write('{"paid":false}\n');
    return;
  }
  if (typeof ev.logIndex !== 'number' || ev.logIndex < 0 || !Number.isInteger(ev.logIndex)) {
    process.stdout.write('{"paid":false}\n');
    return;
  }
  if (typeof ev.round !== 'number' || ev.round <= 0 || !Number.isInteger(ev.round)) {
    process.stdout.write('{"paid":false}\n');
    return;
  }

  // Amounts as bigint
  const solverReward = toBigInt(ev.solverReward);
  const returnedBond = toBigInt(ev.returnedBond);
  const verifierReward = toBigInt(ev.verifierReward);
  const timeoutBonus = toBigInt(ev.timeoutBonus);
  const payout = toBigInt(ev.payout);
  if (solverReward === null || returnedBond === null || verifierReward === null || timeoutBonus === null || payout === null) {
    process.stdout.write('{"paid":false}\n');
    return;
  }
  const computed = solverReward + returnedBond + verifierReward + timeoutBonus;
  if (computed !== payout) {
    process.stdout.write('{"paid":false}\n');
