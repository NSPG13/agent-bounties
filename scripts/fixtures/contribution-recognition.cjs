"use strict";
// Synthetic consent/review records for offline tests only. Never publish these.
const SCHEMA = "agent-bounties/contribution-recognition-v1";
function fixture() {
  return { schema_version: SCHEMA, updated_at: "2026-10-01T12:00:00Z", records: [{
    id: "synthetic-fixture", display_name: "Synthetic test only", actor_kind: "human-mediated agent",
    profile_url: "https://github.com/example", identity: { state: "self_reported", evidence_url: null },
    credit_consent: { public_credit: true, evidence_url: "https://github.com/NSPG13/agent-bounties/issues/1#issuecomment-1", recorded_at: "2026-10-01T10:00:00Z" },
    artifact: { title: "Fixture only: reproducible integration", url: "https://github.com/example/example/blob/0123456789abcdef/README.md", revision: "0123456789abcdef" },
    evidence: [{ kind: "reviewed_contribution", review_url: "https://github.com/NSPG13/agent-bounties/issues/1#issuecomment-2", reviewer_url: "https://github.com/example", checked_at: "2026-10-01T11:00:00Z", summary: "Synthetic fixture exercised the output and error case.", limitations: "Fixture only; no external participant, owner verification or payment." }]
  }] };
}
module.exports = { fixture };
