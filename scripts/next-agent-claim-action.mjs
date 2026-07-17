#!/usr/bin/env node

import { readFileSync, existsSync } from 'fs';
import { resolve } from 'path';

const args = process.argv.slice(2);
if (args.length !== 1) {
  process.exit(1);
}

const filePath = resolve(args[0]);
if (!existsSync(filePath)) {
  process.exit(1);
}

let claim;
try {
  const data = readFileSync(filePath, 'utf8');
  claim = JSON.parse(data);
} catch (e) {
  process.exit(1);
}

const status = claim.status;
const eventId = claim.eventId;
const candidateEventId = claim.candidate?.eventId;
const authorization = claim.authorization;
const candidate = claim.candidate;

if (status === 'authorization_ready' || status === 'relaying') {
  if (!eventId || !candidateEventId || eventId !== candidateEventId) {
    process.exit(1);
  }
}

if (status === 'authorization_ready') {
  if (!authorization || authorization.type !== 'eth_signTypedData_v4') {
    process.exit(1);
  }
  if (
    candidate &&
    authorization.signer &&
    candidate.solver &&
    authorization.signer.toLowerCase() !== candidate.solver.toLowerCase()
  ) {
    process.exit(1);
  }
}

const actionMap = {
  waitlisted: 'wait',
  authorization_ready: 'authorize',
  relaying: 'relay',
  claimed: 'settle',
  'agent-bounties/claim-problem-v1': 'report_problem'
};

