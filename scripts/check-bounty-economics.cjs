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
    repository: "owner/repository",
    commit: "a".repeat(40),
    subdirectory: "benchmarks/task",
  },
  runner_manifest: {
    schema_version: "agent-bounties/regression-sandbox-v1",
    image: `docker.io/library/python@sha256:${"b".repeat(64)}`,
    command: ["python", "/benchmark/check.py"],
    workdir: "/workspace",
    benchmark_digest: `sha256:${"c".repeat(64)}`,
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
assert.deepEqual(verificationReadiness(benchmark), { blocked: false, executable: true });
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
    verificationReadiness(incomplete).executable,
    false,
    "the preview must use the same complete verifier readiness gate as funding",
  );
}
const copiedUnsafeBenchmark = JSON.parse(JSON.stringify(benchmark));
copiedUnsafeBenchmark.source.repository = "other/copied-benchmark";
copiedUnsafeBenchmark.source.subdirectory = "different/location";
copiedUnsafeBenchmark.runner_manifest.benchmark_digest =
  "sha256:240a940036f8af4937657d369a2abe2ecd6f0b47a1c6d68c71d8123d980db541";
assert.deepEqual(
  verificationReadiness(copiedUnsafeBenchmark),
  { blocked: true, executable: false },
  "unreconciled benchmark content must remain blocked after it is copied",
);
const revisedUnsafeBenchmark = JSON.parse(JSON.stringify(benchmark));
revisedUnsafeBenchmark.source.repository = "nSpG13/agent-bounties";
revisedUnsafeBenchmark.source.subdirectory =
  "benchmarks/distribution-v1/glama-onboarding-audit";
revisedUnsafeBenchmark.runner_manifest.benchmark_digest = `sha256:${"d".repeat(64)}`;
assert.deepEqual(
  verificationReadiness(revisedUnsafeBenchmark),
  { blocked: true, executable: false },
  "the canonical lifecycle benchmark must remain blocked across revisions until a reviewed reconciliation is approved",
);

process.stdout.write("bounty economics behavior check passed\n");
