"use strict";

const assert = require("node:assert/strict");
const {
  parseDistributionAttribution,
  parsePreparedRewardSplit,
  rewardSplitForTotal,
  verificationReadiness,
} = require("../site/bounty-composer-v2.js");

const staged = parsePreparedRewardSplit("9", "1");
const reviewed = rewardSplitForTotal(10, staged);
const submitted = rewardSplitForTotal(10, staged);

assert.deepEqual(reviewed, {
  total: 10_000_000n,
  solver: 9_000_000n,
  verifier: 1_000_000n,
});
assert.deepEqual(submitted, reviewed);
assert.equal(submitted.verifier, 1_000_000n, "the verifier-sized solver bond must retain the approved amount");

const manual = rewardSplitForTotal(10);
assert.deepEqual(manual, {
  total: 10_000_000n,
  solver: 9_800_000n,
  verifier: 200_000n,
});

assert.throws(
  () => parsePreparedRewardSplit("9.0000001", "1"),
  /up to six places/,
);

assert.throws(
  () => parsePreparedRewardSplit("1.999999", "0.010001"),
  /at least 2 USDC for the solver/,
);
assert.throws(
  () => parsePreparedRewardSplit("2.009", "0.001"),
  /at least 0.01 USDC for the verifier/,
);

assert.deepEqual(rewardSplitForTotal(2.01), {
  total: 2_010_000n,
  solver: 2_000_000n,
  verifier: 10_000n,
});

const acquisition = `aba1_${"ab".repeat(32)}.${"cd".repeat(32)}`;
const handoff = "10000000-0000-4000-8000-000000000001";
const attributed = new URLSearchParams({ acquisition, handoff });
assert.deepEqual(parseDistributionAttribution(attributed), { acquisition, handoff });

for (const missing of ["acquisition", "handoff"]) {
  const partial = new URLSearchParams(attributed);
  partial.delete(missing);
  assert.throws(() => parseDistributionAttribution(partial), /must include both/);
}

const malformed = new URLSearchParams(attributed);
malformed.set("acquisition", "aba1_not-opaque");
assert.throws(() => parseDistributionAttribution(malformed), /malformed/);

const benchmark = {
  engine: "sandboxed_regression_v1",
  source: {
    kind: "github_commit",
    repository: "NSPG13/agent-bounties",
    commit: "fa946859a3379b8c9128183e20dedb3b8319a646",
    subdirectory: "benchmarks/direct-growth-v2/a2a-agent-card",
  },
  runner_manifest: {
    schema_version: "agent-bounties/regression-sandbox-v1",
    image: `docker.io/library/python@sha256:${"b".repeat(64)}`,
    command: ["python", "/benchmark/check.py"],
    workdir: "/workspace",
    benchmark_digest: "sha256:b61a96a7d07ca01337ea3576de734f5b62ccab966a6d0da42a8736cfc0287ce6",
    timeout_seconds: 60,
    cpu_millis: 1_000,
    memory_bytes: 134_217_728,
    pids_limit: 64,
    max_output_bytes: 1_048_576,
    tmpfs_bytes: 67_108_864,
    max_source_bytes: 104_857_600,
    max_source_files: 10_000,
    max_benchmark_bytes: 1_048_576,
    max_benchmark_files: 100,
    platform: "linux/amd64",
    test_seed: 1,
  },
};
const evidenceSchema = {
  type: "object",
  required: ["source_snapshot_digest"],
  properties: {
    source_snapshot_digest: {
      type: "string",
      pattern: "^sha256:[0-9a-f]{64}$",
    },
  },
};
assert.deepEqual(
  verificationReadiness(benchmark, evidenceSchema),
  { blocked: false, executable: true },
);
for (const mutate of [
  (value) => { delete value.source.commit; },
  (value) => { delete value.runner_manifest.image; },
  (value) => { value.runner_manifest.command = []; },
  (value) => { delete value.runner_manifest.workdir; },
  (value) => { delete value.runner_manifest.benchmark_digest; },
  (value) => { delete value.runner_manifest.timeout_seconds; },
  (value) => { value.runner_manifest.max_source_bytes = 0; },
  (value) => { value.runner_manifest.platform = "windows/amd64"; },
  (value) => { value.runner_manifest.test_seed = Number.MAX_SAFE_INTEGER + 1; },
]) {
  const incomplete = JSON.parse(JSON.stringify(benchmark));
  mutate(incomplete);
  assert.equal(
    verificationReadiness(incomplete, evidenceSchema).executable,
    false,
    "the preview must use the same complete verifier readiness gate as funding",
  );
}
for (const mutate of [
  (value) => { value.type = "array"; },
  (value) => { value.required = []; },
  (value) => { delete value.properties.source_snapshot_digest; },
  (value) => { value.properties.source_snapshot_digest.type = "number"; },
  (value) => { value.properties.source_snapshot_digest.pattern = "^sha256:.+$"; },
]) {
  const incomplete = JSON.parse(JSON.stringify(evidenceSchema));
  mutate(incomplete);
  assert.equal(
    verificationReadiness(benchmark, incomplete).executable,
    false,
    "wallet review must reject an incompatible source snapshot evidence schema",
  );
}
for (const image of [
  `docker.io/a@tag@sha256:${"b".repeat(64)}`,
  `docker.io/a..b@sha256:${"b".repeat(64)}`,
]) {
  const malformedImage = JSON.parse(JSON.stringify(benchmark));
  malformedImage.runner_manifest.image = image;
  assert.equal(
    verificationReadiness(malformedImage, evidenceSchema).executable,
    false,
    "wallet review must reject every image rejected by the server's pinned-image validator",
  );
}
const wrongApprovedSource = JSON.parse(JSON.stringify(benchmark));
wrongApprovedSource.source.commit = "a".repeat(40);
assert.deepEqual(
  verificationReadiness(wrongApprovedSource, evidenceSchema),
  { blocked: true, executable: false },
  "an approved digest must remain bound to its reviewed repository, commit, and subdirectory",
);
const copiedUnsafeBenchmark = JSON.parse(JSON.stringify(benchmark));
const originalOpenHands = JSON.parse(JSON.stringify(benchmark));
originalOpenHands.source.commit = "aa28ec742efd4063260653510ba324e291267515";
assert.equal(verificationReadiness(originalOpenHands, evidenceSchema).executable, false);
originalOpenHands.source.subdirectory = "benchmarks/direct-growth-v2/openhands-integration";
originalOpenHands.runner_manifest.benchmark_digest = "sha256:30bb17e3e3916747144c7087f49fb1ce41ddaf1aec4d717f878d2840203895a2";
assert.equal(verificationReadiness(originalOpenHands, evidenceSchema).executable, true);
assert.equal(verificationReadiness(originalOpenHands, { type: "object", required: ["source_snapshot_digest"] }).executable, false);
copiedUnsafeBenchmark.source.repository = "other/copied-benchmark";
copiedUnsafeBenchmark.source.subdirectory = "different/location";
copiedUnsafeBenchmark.runner_manifest.benchmark_digest =
  "sha256:240a940036f8af4937657d369a2abe2ecd6f0b47a1c6d68c71d8123d980db541";
assert.deepEqual(
  verificationReadiness(copiedUnsafeBenchmark, evidenceSchema),
  { blocked: true, executable: false },
  "unreconciled benchmark content must remain blocked after it is copied",
);
const revisedUnsafeBenchmark = JSON.parse(JSON.stringify(benchmark));
revisedUnsafeBenchmark.source.repository = "nSpG13/agent-bounties";
revisedUnsafeBenchmark.source.subdirectory =
  "benchmarks/distribution-v1/glama-onboarding-audit";
revisedUnsafeBenchmark.runner_manifest.benchmark_digest = `sha256:${"d".repeat(64)}`;
assert.deepEqual(
  verificationReadiness(revisedUnsafeBenchmark, evidenceSchema),
  { blocked: true, executable: false },
  "the canonical lifecycle benchmark must remain blocked across revisions until a reviewed reconciliation is approved",
);
const relocatedAlteredBenchmark = JSON.parse(JSON.stringify(benchmark));
relocatedAlteredBenchmark.source.repository = "other/copied-benchmark";
relocatedAlteredBenchmark.source.subdirectory = "different/location";
relocatedAlteredBenchmark.runner_manifest.benchmark_digest = `sha256:${"d".repeat(64)}`;
assert.deepEqual(
  verificationReadiness(relocatedAlteredBenchmark, evidenceSchema),
  { blocked: true, executable: false },
  "relocating and altering an unreviewed benchmark must not bypass the digest allowlist",
);

process.stdout.write("bounty economics behavior check passed\n");
