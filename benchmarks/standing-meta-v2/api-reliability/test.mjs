import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-api-reliability.mjs");

if (!existsSync(checker)) {
  console.error();
  process.exit(1);
}

const ready = {
  ready: true,
  protocol: "https",
  expected_statuses: [200, 201, 204],
  timeout: 5,
  retry_strategy: "exponential_backoff",
  max_retries: 3,
  health_check_interval: 30,
  required_metrics: ["latency_p95", "error_rate", "uptime_percentage", "throughput_rps"],
};

const invalidErrors = [
  "schema_mismatch",
  "protocol_mismatch",
  "endpoint_pattern_mismatch",
  "status_code_unknown",
  "timeout_mismatch",
  "retry_strategy_mismatch",
  "max_retries_mismatch",
  "health_check_interval_mismatch",
  "required_metric_missing:latency_p95",
  "required_metric_missing:error_rate",
  "required_metric_missing:uptime_percentage",
  "required_metric_missing:throughput_rps",
];

const cases = [
  { name: "missing argument", args: [], status: 2, output: { ready: false, errors: ["manifest_path_required"] } },
  { name: "unreadable manifest", args: [join(benchmarkRoot, "fixtures", "absent.json")], status: 2, output: { ready: false, errors: ["manifest_unreadable"] } },
  { name: "malformed JSON", args: [join(benchmarkRoot, "fixtures", "malformed.json")], status: 2, output: { ready: false, errors: ["manifest_invalid_json"] } },
  { name: "non-object root", args: [join(benchmarkRoot, "fixtures", "not-an-object.json")], status: 2, output: { ready: false, errors: ["manifest_root_object_required"] } },
  { name: "missing required field", args: [join(benchmarkRoot, "fixtures", "missing-field.json")], status: 1, output: { ready: false, errors: invalidErrors } },
  { name: "wrong protocol", args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")], status: 1, output: { ready: false, errors: invalidErrors } },
  { name: "valid manifest", args: [join(benchmarkRoot, "fixtures", "valid.json")], status: 0, output: ready },
];

let passed = 0;
let failed = 0;

for (const c of cases) {
  const result = spawnSync(process.execPath, [checker, ...c.args], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });

  const actualStatus = result.status ?? (result.error ? 1 : 0);
  let actualOutput;
  try {
    actualOutput = JSON.parse((result.stdout ?? "").trim());
  } catch {
    actualOutput = null;
  }

  const statusOk = actualStatus === c.status;
  const outputOk = c.status !== 0 ? (actualOutput && actualOutput.ready === false && Array.isArray(actualOutput.errors)) : (actualOutput && actualOutput.ready === true);

  if (statusOk && outputOk) {
    passed++;
  } else {
    failed++;
    console.error(`FAIL ${c.name}: status=${actualStatus}/${c.status} output=${JSON.stringify(actualOutput)}`);
  }
}

console.log(`${passed}/${passed + failed} passed`);
process.exit(failed > 0 ? 1 : 0);
