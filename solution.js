#!/usr/bin/env node

// scripts/next-agent-claim-action.mjs
// Dependency-free Node.js CLI to determine the single safe next action for an agent claim.

const { readFileSync } = require('fs');

// Parse claim response from stdin or first argument
function getInput() {
  if (process.argv[2]) {
    return process.argv[2];
  }
  const stdin = readFileSync(0, 'utf8').trim();
  if (!stdin) {
    console.error('Error: No input provided. Provide claim response as argument or via stdin.');
    process.exit(1);
  }
  return stdin;
}

// Main logic: determine safe next action
function determineAction(claim) {
  // Expected claim structure (simplified for bounty):
  // { status: string, settlement?: { accepted: boolean, txHash?: string }, bond: number }

  if (!claim || typeof claim !== 'object') {
    return { action: 'invalid_input', reason: 'Claim must be a JSON object' };
  }

  const { status, settlement, bond } = claim;

  // Status-based checks
  switch (status) {
    case 'open':
      return { action: 'submit_solution', reason: 'Claim is open, submit a solution' };

    case 'claimed':
      if (!settlement) {
        return { action: 'wait_for_settlement', reason: 'Claim claimed but no settlement yet, wait' };
      }
      if (settlement.accepted === true) {
        return { action: 'claim_reward', reason: 'Settlement accepted, claim reward' };
      }
      if (settlement.accepted === false) {
        return { action: 'dispute', reason: 'Settlement rejected, consider disputing' };
      }
      return { action: 'wait_for_verification', reason: 'Settlement pending verification' };

    case 'settled':
      if (settlement && settlement.accepted) {
        return { action: 'claim_reward', reason: 'Claim settled, reward ready' };
      }
      return { action: 'no_action', reason: 'Claim settled without acceptance, no action needed' };

    case 'expired':
      return { action: 'reclaim_bond', reason: 'Claim expired, reclaim bond if applicable' };

    default:
      return { action: 'unknown_status', reason: `Unrecognized status: ${status}` };
  }
}

// Execution
const rawInput = getInput();
let claim;
try {
  claim = JSON.parse(rawInput);
} catch (e) {
  console.error('Error: Input must be valid JSON');
  process.exit(1);
}

const result = determineAction(claim);
process.stdout.write(JSON.stringify(result, null, 2) + '\n');