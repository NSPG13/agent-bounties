#!/usr/bin/env node

import { readFileSync } from 'fs';

const [feedPath, agentWallet] = process.argv.slice(2);

if (!feedPath || !agentWallet) {
  process.stderr.write('Usage: select-funded-bounty.mjs <feed.json> <agent-wallet>\n');
  process.exit(2);
}

let feed;
try {
  const raw = readFileSync(feedPath, 'utf8');
  feed = JSON.parse(raw);
} catch {
  process.exit(2);
}

if (!Array.isArray(feed)) {
  process.exit(2);
}

function isValidEntry(e) {
  if (typeof e !== 'object' || e === null) return false;
  const required = ['id', 'creator', 'solverReward', 'claimBond', 'termsValid', 'verificationReady', 'fullyFunded', 'claimable'];
  for (const key of required) {
    if (!(key in e)) return false;
  }
  if (typeof e.id !== 'string' && typeof e.id !== 'number') return false;
  if (typeof e.creator !== 'string') return false;
  if (typeof e.solverReward !== 'number' || typeof e.claimBond !== 'number') return false;
  if (typeof e.termsValid !== 'boolean' || typeof e.verificationReady !== 'boolean') return false;
  if (typeof e.fullyFunded !== 'boolean' || typeof e.claimable !== 'boolean') return false;
  return true;
}

const eligible = feed.filter(e => {
  if (!isValidEntry(e)) return false;
  if (!e.claimable) return false;
  if (!e.fullyFunded) return false;
  if (!e.termsValid) return false;
  if (!e.verificationReady) return false;
  if (e.solverReward <= 0) return false;
  if (e.claimBond <= 0) return false;
  if (e.creator.toLowerCase() === agentWallet.toLowerCase()) return false;
  return true;
});

if (eligible.length === 0) {
  process.exit(1);
}

eligible.sort((a, b) => {
  // primary: solverReward descending
  if (b.solverReward !== a.solverReward) return b.solverReward - a.solverReward;
  // secondary: claimBond ascending
  if (a.claimBond !== b.claimBond) return a.claimBond - b.claimBond;
  // tertiary: bounty ID ascending (compare as strings to handle mixed types)
  const aId = String(a.id);
  const bId = String(b.id);
  if (aId < bId) return -1;
  if (aId > bId) return 1;
  return 0;
});

const selected = eligible[0];

// Emit canonical identifiers and next action
const output = {
  bountyId: selected.id,
  creator: selected.creator,
  solverReward: selected.solverReward,
  claimBond: selected.claimBond,
  nextAction: 'agent_native_claim',
};

process.stdout.write(JSON.stringify(output) + '\n');
process.exit(0);
