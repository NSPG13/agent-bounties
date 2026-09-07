(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesCreatorReview = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const ENGINE = "creator_review_v1";
  const DISCLOSURE = "The creator reviews every published acceptance check and signs the verdict. This is human review, not independent or automated verification. The verifier reward is paid to the creator on pass or fail; a missed review deadline returns the solver bond. The delivery cutoff is checked against the canonical submission time; the contract's claim timeout remains a separate relative window.";
  const evidenceSchema = () => ({ type: "object", required: ["artifact_url", "artifact_sha256"], properties: { artifact_url: { type: "string", pattern: "^https://" }, artifact_sha256: { type: "string", pattern: "^sha256:[0-9a-f]{64}$" } } });
  function deadline(value) {
    if (value == null || value === "") return null;
    if (typeof value !== "string" || !/^\d{4}-\d\d-\d\dT\d\d:\d\d(?::\d\d(?:\.\d{1,3})?)?(?:Z|[+-]\d\d:\d\d)$/.test(value) || !Number.isFinite(Date.parse(value))) throw new Error("Supply the agreed delivery deadline as an ISO timestamp including its time-zone offset.");
    return new Date(value).toISOString();
  }
  function prepare(draft) {
    if (draft.review_mode !== "creator") return draft;
    if (draft.meta_child) throw new Error("Qualifying meta-bounty children require their existing automated verifier.");
    const cutoff = deadline(draft.delivery_deadline);
    if (!cutoff || Date.parse(cutoff) <= Date.now()) throw new Error("Creator review requires a future delivery deadline.");
    if (Date.parse(cutoff) > Date.now() + 366 * 86400000) throw new Error("The delivery deadline must be within 366 days; prepare a nearer milestone for longer work.");
    if (draft.benchmark && draft.benchmark.engine !== ENGINE) throw new Error("Do not replace an automated benchmark with creator review. Explicitly restage the selected policy without that benchmark.");
    return { ...draft, delivery_deadline: cutoff, benchmark: { engine: ENGINE, delivery_deadline: Math.floor(Date.parse(cutoff) / 1000), acceptance: "all_published_criteria", reviewer: "creator" }, evidence_schema: evidenceSchema() };
  }
  function ready(benchmark, schema) {
    return benchmark?.engine === ENGINE && benchmark.reviewer === "creator" && benchmark.acceptance === "all_published_criteria"
      && Number.isSafeInteger(benchmark.delivery_deadline) && benchmark.delivery_deadline > Date.now() / 1000
      && benchmark.delivery_deadline <= Date.now() / 1000 + 366 * 86400
      && schema?.type === "object" && ["artifact_url", "artifact_sha256"].every((key) => schema.required?.includes(key))
      && schema.properties?.artifact_url?.pattern === "^https://" && schema.properties?.artifact_sha256?.pattern === "^sha256:[0-9a-f]{64}$";
  }
  function policy(wallet) { return { mechanism: "signed_quorum", engine: ENGINE, verifiers: wallet ? [wallet.toLowerCase()] : [], threshold: 1, rubric: "Pass only when every published acceptance criterion is satisfied and canonical submission time is no later than the committed delivery deadline.", public_disclosure: DISCLOSURE }; }
  return { ENGINE, DISCLOSURE, deadline, prepare, ready, policy, evidenceSchema };
});
