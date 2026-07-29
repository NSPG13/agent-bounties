#!/usr/bin/env node

import { readFileSync } from "node:fs";

const ADDRESS = /^0x[0-9a-f]{40}$/;

function emit(value, code) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
  process.exit(code);
}

function fail(error, code = 1) {
  emit({ ok: false, errors: [error] }, code);
}

if (process.argv.length !== 3) fail("claim_response_path_required", 2);

let claim;
try {
  claim = JSON.parse(readFileSync(process.argv[2], "utf8"));
} catch (error) {
  fail(error instanceof SyntaxError ? "claim_response_invalid_json" : "claim_response_unreadable", 2);
}
if (!claim || Array.isArray(claim) || typeof claim !== "object") {
  fail("claim_response_object_required", 2);
}

if (claim.schema_version === "agent-bounties/claim-problem-v1") {
  emit({
    ok: true,
    state: claim.state,
    action: "follow_error_next_action",
    may_sign: false,
    may_start_work: false,
    error: claim.error,
    failed_transition: claim.failed_transition,
  }, 0);
}
if (claim.schema_version !== "agent-bounties/agent-native-claim-v1") {
  fail("claim_schema_unsupported");
}

const candidate = claim.candidate;
const state = candidate?.status;

if (state === "waitlisted") {
  emit({ ok: true, state, action: "poll_same_idempotency_key", may_sign: false, may_start_work: false }, 0);
}
if (state === "relaying") {
  emit({ ok: true, state, action: "replay_same_signed_request", may_sign: false, may_start_work: false }, 0);
}
if (state === "authorization_ready") {
  const request = claim.wallet_request;
  const replay = claim.next_request;
  const solver = candidate?.solver_wallet?.toLowerCase();
  const valid = ADDRESS.test(solver ?? "")
    && request?.method === "eth_signTypedData_v4"
    && Array.isArray(request.params)
    && request.params[0]?.toLowerCase() === solver
    && typeof request.params[1] === "string"
    && replay?.method === "POST"
    && typeof replay.url === "string"
    && replay.url.startsWith("https://api.agentbounties.app/")
    && typeof replay.body?.idempotency_key === "string";
  if (!valid) fail("authorization_request_invalid");
  emit({ ok: true, state, action: "sign_wallet_request_and_replay", may_sign: true, may_start_work: false }, 0);
}
if (state === "claimed") {
  const eventId = claim.canonical_event_id;
  if (!eventId || candidate.canonical_event_id !== eventId) {
    fail("canonical_claim_evidence_invalid");
  }
  emit({
    ok: true,
    state,
    action: "start_work",
    may_sign: false,
    may_start_work: true,
    canonical_event_id: eventId,
  }, 0);
}
fail(`claim_state_unsupported:${state}`);
