#!/usr/bin/env node

import { readFileSync } from "node:fs";

function out(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function err(code, errors) {
  out({ ok: false, errors });
  process.exit(code);
}

const arg = process.argv[2];
if (!arg) {
  err(2, ["claim_response_path_required"]);
}

let raw;
try {
  raw = readFileSync(arg, "utf8");
} catch {
  err(2, ["claim_response_unreadable"]);
}

let doc;
try {
  doc = JSON.parse(raw);
} catch {
  err(2, ["claim_response_invalid_json"]);
}

if (!doc || typeof doc !== "object" || Array.isArray(doc)) {
  err(2, ["claim_response_object_required"]);
}

const schemaVersion = doc.schema_version;

if (schemaVersion === "agent-bounties/claim-problem-v1") {
  const state = doc.state ?? "failed";
  const failedTransition = doc.failed_transition;
  const error = doc.error;
  out({ ok: true, state, action: "follow_error_next_action", may_sign: false, may_start_work: false, error, failed_transition: failedTransition });
  process.exit(0);
}

// State can be at doc.state or doc.candidate.status
const state = doc.state ?? doc.candidate?.status;
if (!state) {
  err(2, ["claim_response_missing_state"]);
}

switch (state) {
  case "waitlisted":
    out({ ok: true, state, action: "poll_same_idempotency_key", may_sign: false, may_start_work: false });
    break;

  case "authorization_ready": {
    const walletRequest = doc.wallet_request;
    if (!walletRequest || typeof walletRequest !== "object") {
      err(1, ["authorization_request_invalid"]);
      break;
    }
    if (walletRequest.method !== "eth_signTypedData_v4") {
      err(1, ["authorization_request_invalid"]);
      break;
    }
    const params = walletRequest.params;
    if (!Array.isArray(params) || params.length < 1) {
      err(1, ["authorization_request_invalid"]);
      break;
    }
    const solverWallet = doc.candidate?.solver_wallet;
    if (!solverWallet) {
      err(1, ["authorization_request_invalid"]);
      break;
    }
    const providedAddress = params[0];
    if (typeof providedAddress !== "string" || providedAddress.toLowerCase() !== solverWallet.toLowerCase()) {
      err(1, ["authorization_request_invalid"]);
      break;
    }
    if (!doc.next_request || typeof doc.next_request !== "object") {
      err(1, ["authorization_request_invalid"]);
      break;
    }
    out({ ok: true, state, action: "sign_wallet_request_and_replay", may_sign: true, may_start_work: false });
    break;
  }

  case "relaying":
    out({ ok: true, state, action: "replay_same_signed_request", may_sign: false, may_start_work: false });
    break;

  case "claimed": {
    const candidateEventId = doc.candidate?.canonical_event_id;
    const topLevelEventId = doc.canonical_event_id;
    if (!candidateEventId || !topLevelEventId || candidateEventId !== topLevelEventId) {
      err(1, ["canonical_claim_evidence_invalid"]);
      break;
    }
    out({ ok: true, state, action: "start_work", may_sign: false, may_start_work: true, canonical_event_id: topLevelEventId });
    break;
  }

  default:
    err(1, [`claim_state_unsupported:${state}`]);
}
