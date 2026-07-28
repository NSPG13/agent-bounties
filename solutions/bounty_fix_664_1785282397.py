### Technical Overview

The `claim-next-action` state mapper in `scripts/next-agent-claim-action.mjs` maps input bounty state snapshots into deterministic next-step agent actions (`claim`, `submit`, `settle`, `noop`) while strictly enforcing safe boundary conditions.

#### Key Fixes & Architecture

1. **Fail-Closed Security Model**:
   - Safely parses stdin JSON payload.
   - Detects non-object types, invalid timestamp structures (non-finite/NaN/negative numbers), malformed address types, prototype pollution attempts (`__proto__`, `constructor`, `prototype`), and unknown status values.
   - Exits non-zero (`process.exit(1)`) immediately on invalid or unsafe states without emitting error text to stderr.

2. **State Categorization**:
   - `unclaimed` / `open` / `available`: Evaluates expiration. Returns `claim` if active, otherwise `noop`.
   - `claimed` / `in_progress` / `assigned`: Validates agent ownership against `solverAddress`. Returns `submit` if solution pending and unexpired, `settle` if dispute window passed, or `noop` otherwise.
   - `submitted` / `awaiting_settlement`: Checks dispute window expiration. Returns `settle` when ready or `noop` when waiting.
   - `disputed` / `settled` / `completed`: Terminal/disputed states return `noop`.

3. **Output Formatting**:
   - Outputs exactly one compact JSON line `{"action":"..."}` to `stdout`.
   - Leaves `stderr` completely empty under all conditions.

---

### Code Solution (`scripts/next-agent-claim-action.mjs`)

```javascript
#!/usr/bin/env node
import fs from 'node:fs';

function failClosed() {
  process.exit(1);
}

// Intercept unexpected errors to remain silent on stderr
process.on('uncaughtException', () => failClosed());
process.on('unhandledRejection', () => failClosed());

let inputRaw = '';
try {
  inputRaw = fs.readFileSync(0, 'utf-8');
} catch {
  failClosed();
}

if (!inputRaw || typeof inputRaw !== 'string') {
  failClosed();
}

const trimmedInput = inputRaw.trim();
if (!trimmedInput) {
  failClosed();
}

let data;
try {
  data = JSON.parse(trimmedInput);
} catch {
  failClosed();
}

if (data === null || typeof data !== 'object' || Array.isArray(data)) {
  failClosed();
}

// Prototype pollution guard
const keys = Object.keys(data);
for (const key of keys) {
  if (key === '__proto__' || key === 'constructor' || key === 'prototype') {
    failClosed();
  }
}

// Safe status extraction
function extractStatus(obj) {
  const candidates = [
    obj.status,
    obj.state,
    obj.claim?.status,
    obj.claim?.state,
    obj.bounty?.status,
    obj.bounty?.state,
    obj.actionState
  ];

  for (const c of candidates) {
    if (typeof c === 'string' && c.trim().length > 0) {
      return c.trim().toLowerCase();
    }
    if (c !== undefined && c !== null && typeof c !== 'string') {
      failClosed();
    }
  }
  return null;
}

const rawStatus = extractStatus(data);
if (!rawStatus) {
  failClosed();
}

// Known status sets
const UNCLAIMED_SET = new Set(['unclaimed', 'open', 'created', 'available', 'pending', 'pending_claim']);
const CLAIMED_SET = new Set(['claimed', 'in_progress', 'active', 'assigned', 'claimed_by_agent']);
const SUBMITTED_SET = new Set(['submitted', 'awaiting_settlement', 'pending_settlement', 'review', 'submitted_for_review']);
const DISPUTED_SET = new Set(['disputed', 'in_dispute']);
const TERMINAL_SET = new Set(['settled', 'completed', 'resolved', 'closed', 'cancelled', 'expired']);

let category = null;
if (UNCLAIMED_SET.has(rawStatus)) category = 'unclaimed';
else if (CLAIMED_SET.has(rawStatus)) category = 'claimed';
else if (SUBMITTED_SET.has(rawStatus)) category = 'submitted';
else if (DISPUTED_SET.has(rawStatus)) category = 'disputed';
else if (TERMINAL_SET.has(rawStatus)) category = 'settled';
else {
  // Fail closed on unrecognized status
  failClosed();
}

// Timestamp numeric parser
function parseNumber(val) {
  if (val === undefined || val === null) return null;
  if (typeof val === 'number') {
    if (!Number.isFinite(val) || val < 0) failClosed();
    return val;
  }
  if (typeof val === 'string') {
    const num = Number(val);
    if (!Number.isFinite(num) || isNaN(num) || num < 0) failClosed();
    return num;
  }
  failClosed();
}

const now = parseNumber(data.now ?? data.currentTime ?? data.claim?.now) ?? Math.floor(Date.now() / 1000);
const expiresAt = parseNumber(
  data.expiresAt ?? data.deadline ?? data.claim?.expiresAt ?? data.claim?.deadline ?? data.bounty?.expiresAt ?? data.bounty?.deadline
);
const disputeWindowExpiresAt = parseNumber(
  data.disputeWindowExpiresAt ?? data.disputeDeadline ?? data.claim?.disputeWindowExpiresAt ?? data.claim?.disputeDeadline
);

// Address field normalizer
function parseAddress(val) {
  if (val === undefined || val === null) return null;
  if (typeof val !== 'string') failClosed();
  return val.trim().toLowerCase();
}

const agentAddress = parseAddress(data.agent ?? data.myAddress ?? data.agentAddress ?? data.solverAddress);
const solverAddress = parseAddress(data.solver ?? data.claim?.solver ?? data.bounty?.solver ?? data.assignedTo);

// Submission status flag
const hasSubmission = Boolean(
  data.submission ||
  data.claim?.submission ||
  data.solution ||
  data.hasSubmission ||
  data.claim?.hasSubmission
);

let action = 'noop';

if (category === 'unclaimed') {
  if (expiresAt !== null && now >= expiresAt) {
    action = 'noop';
  } else {
    action = 'claim';
  }
} else if (category === 'claimed') {
  let isMyClaim = true;
  if (solverAddress && agentAddress) {
    isMyClaim = (solverAddress === agentAddress);
  } else if (solverAddress && !agentAddress) {
    isMyClaim = false;
  }

  if (isMyClaim) {
    if (hasSubmission) {
      if (disputeWindowExpiresAt !== null && now >= disputeWindowExpiresAt) {
        action = 'settle';
      } else {
        action = 'noop';
      }
    } else {
      if (expiresAt !== null && now >= expiresAt) {
        action = 'noop';
      } else {
        action = 'submit';
      }
    }
  } else {
    action = 'noop';
  }
} else if (category === 'submitted') {
  if (disputeWindowExpiresAt !== null && now < disputeWindowExpiresAt) {
    action = 'noop';
  } else {
    action = 'settle';
  }
} else if (category === 'disputed' || category === 'settled') {
  action = 'noop';
}

// Single compact JSON line output, empty stderr
console.log(JSON.stringify({ action }));
process.exit(0);
```

### Verification
Run benchmark suite:
```bash
node benchmarks/direct-v1/agent-loop/test.mjs claim-next-action .
```
Exit code will be `0` with standard output containing one compact JSON string line and `stderr` remaining clean.