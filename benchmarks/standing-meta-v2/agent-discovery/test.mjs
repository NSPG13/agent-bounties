import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-agent-discovery.mjs");

if (!existsSync(checker)) {
  console.error(`missing child implementation: ${checker}`);
  process.exit(1);
}

const ready = {
  ready: true,
  network: "base-mainnet",
  agent_registration: "https://api.agentbounties.app/v1/base/agents/register",
  agent_discovery: "https://api.agentbounties.app/v1/base/agents/discover",
  agent_card_path: ".well-known/agent-card.json",
  required_capabilities: [
    "claim_detection",
    "funding_verification",
    "settlement_tracking",
    "child_bounty_creation",
  ],
};

const invalidErrors = [
  "schema_mismatch",
  "protocol_network_mismatch",
  "protocol_chain_id_mismatch",
  "registration_endpoint_mismatch",
  "discovery_endpoint_mismatch",
  "agent_card_path_mismatch",
  "required_capability_missing:claim_detection",
  "required_capability_missing:funding_verification",
  "required_capability_missing:settlement_tracking",
  "required_capability_missing:child_bounty_creation",
];

const cases = [
  {
    name: "missing argument",
    args: [],
    status: 2,
    output: { ready: false, errors: ["manifest_path_required"] },
  },
  {
    name: "unreadable manifest",
    args: ["/nonexistent/path.json"],
    status: 2,
    output: { ready: false, errors: ["manifest_unreadable"] },
  },
  {
    name: "invalid JSON",
    args: [join(benchmarkRoot, "fixtures", "malformed.json")],
    status: 2,
    output: { ready: false, errors: ["manifest_invalid_json"] },
  },
  {
    name: "not an object",
    args: [join(benchmarkRoot, "fixtures", "not-an-object.json")],
    status: 2,
    output: { ready: false, errors: ["manifest_not_object"] },
  },
  {
    name: "wrong protocol",
    args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")],
    status: 1,
    output: { ready: false, errors: invalidErrors },
  },
  {
    name: "missing capability",
    args: [join(benchmarkRoot, "fixtures", "missing-capability.json")],
    status: 1,
    output: { ready: false, errors: ["required_capability_missing:settlement_tracking"] },
  },
  {
    name: "valid discovery manifest",
    args: [join(benchmarkRoot, "fixtures", "valid.json")],
    status: 0,
    output: ready,
  },
];

let failed = 0;
for (const tc of cases) {
  const result = spawnSync(process.execPath, [checker, ...tc.args], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });
  const got = result.stdout?.trim() || "";
  const want = JSON.stringify(tc.output);
  const statusMatch = result.status === tc.status;
  const outputMatch = got === want;
  if (statusMatch && outputMatch) {
    console.log(`PASS: ${tc.name}`);
  } else {
    console.error(`FAIL: ${tc.name}`);
    if (!statusMatch) console.error(`  expected status ${tc.status}, got ${result.status}`);
    if (!outputMatch) console.error(`  expected: ${want}\n  got:      ${got}`);
    failed++;
  }
}

if (failed > 0) {
  console.error(`\n${failed} test(s) failed`);
  process.exit(1);
}
console.log(`\nALL ${cases.length} tests passed`);
process.exit(0);
