#!/usr/bin/env node

import { readFileSync } from "node:fs";

const emit = (value, status = 0) => {
  process.stdout.write(`${JSON.stringify(value)}\n`);
  process.exit(status);
};

if (process.argv.length !== 3) emit({ ok: false, errors: ["claim_response_path_required"] }, 2);

let text;
try {
  text = readFileSync(process.argv[2], "utf8");
} catch {
  emit({ ok: false, errors: ["claim_response_unreadable"] }, 2);
}

let value;
try {
  value = JSON.parse(text);
} catch {
  emit({ ok: false, errors: ["claim_response_invalid_json"] }, 2);
}
if (!value || Array.isArray(value) || typeof value !== "object") {
  emit({ ok: false, errors: ["claim_response_object_required"] }, 2);
}

if (value.schema_version === "agent-bounties/claim-problem-v1") {
  if (
    value.state !== "failed" ||
    typeof value.error !== "string" ||
    !value.error ||
    typeof value.failed_transition !== "string" ||
    !value.failed_transition ||
    typeof value.next_action !== "string" ||
    !value.next_action
  ) {
    emit({ ok: false, errors: ["claim_problem_invalid"] }, 1);
  }
  emit({
    ok: true,
    state: "failed",
    action: "follow_error_next_action",
    may_sign: false,
    may_start_work: false,
    error: value.error,
    failed_transition: value.failed_transition,
  });
}

if (value.schema_version !== "agent-bounties/agent-native-claim-v1") {
  emit({ ok: false, errors: ["claim_response_schema_unsupported"] }, 1);
}

const candidate = value.candidate;
if (
  !candidate ||
  Array.isArray(candidate) ||
  typeof candidate !== "object" ||
  typeof candidate.status !== "string"
) {
  emit({ ok: false, errors: ["claim_candidate_invalid"] }, 1);
}

const state = candidate.status;

if (state === "waitlisted") {
  emit({ ok: true, state, action: "poll_same_idempotency_key", may_sign: false, may_start_work: false });
}

if (state === "authorization_ready") {
  const request = value.wallet_request;
  const next = value.next_request;
  const params = request?.params;
  const solver = String(candidate.solver_wallet ?? "").toLowerCase();
  const valid =
    request?.method === "eth_signTypedData_v4" &&
    Array.isArray(params) &&
    params.length === 2 &&
    String(params[0]).toLowerCase() === solver &&
    /^0x[0-9a-f]{40}$/.test(solver) &&
    typeof params[1] === "string" &&
    params[1].length > 0 &&
    next &&
    !Array.isArray(next) &&
    typeof next === "object" &&
    typeof next.url === "string" &&
    next.method === "POST" &&
    next.body &&
    typeof next.body.idempotency_key === "string";
  if (!valid) emit({ ok: false, errors: ["authorization_request_invalid"] }, 1);
  emit({ ok: true, state, action: "sign_wallet_request_and_replay", may_sign: true, may_start_work: false });
}

if (state === "relaying") {
  emit({ ok: true, state, action: "replay_same_signed_request", may_sign: false, may_start_work: false });
}

if (state === "claimed") {
  const event = value.canonical_event_id;
  if (typeof event !== "string" || !event || candidate.canonical_event_id !== event) {
    emit({ ok: false, errors: ["canonical_claim_evidence_invalid"] }, 1);
  }
  emit({
    ok: true,
    state,
    action: "start_work",
    may_sign: false,
    may_start_work: true,
    canonical_event_id: event,
  });
}

emit({ ok: false, errors: [`claim_state_unsupported:${state}`] }, 1);