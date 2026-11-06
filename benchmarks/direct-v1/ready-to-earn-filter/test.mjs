import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const task = process.argv[2];
const sourceRoot = resolve(process.argv[3] ?? "/workspace");
const scripts = {
  "ready-to-earn-filter": "ready-to-earn-filter.mjs",
};

if (!Object.hasOwn(scripts, task)) {
  console.error(`unknown task: ${task ?? "missing"}`);
  process.exit(1);
}

const implementation = join(sourceRoot, "scripts", scripts[task]);
if (!existsSync(implementation)) {
  console.error(`missing implementation: ${implementation}`);
  process.exit(1);
}

const temporary = mkdtempSync(join(tmpdir(), `agent-bounties-${task}-`));
const address = (digit) => `0x${digit.repeat(40)}`;
const hash = (digit) => `0x${digit.repeat(64)}`;

function fixture(name, value, raw = false) {
  const path = join(temporary, name);
  writeFileSync(path, raw ? value : `${JSON.stringify(value)}\n`);
  return path;
}

function invoke(args) {
  return spawnSync(process.execPath, [implementation, ...args], {
    encoding: "utf8",
    timeout: 5_000,
    windowsHide: true,
  });
}

function expectRun(name, args, status, output) {
  const result = invoke(args);
  if (result.error) throw new Error(`${name}: ${result.error.message}`);
  if (result.status !== status) {
    throw new Error(
      `${name}: expected exit ${status}, received ${result.status}; stdout=${JSON.stringify(result.stdout)} stderr=${JSON.stringify(result.stderr)}`,
    );
  }
  if (result.stderr !== "") {
    throw new Error(`${name}: stderr must be empty: ${JSON.stringify(result.stderr)}`);
  }
  const expected = `${JSON.stringify(output)}\n`;
  if (result.stdout !== expected) {
    throw new Error(
      `${name}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(result.stdout)}`,
    );
  }
}

function readyToEarnCases() {
  const contract = address("a");
  const id = hash("b");

  // Helper: base healthy bounty
  function healthy(overrides = {}) {
    return {
      bounty_id: id,
      bounty_contract: contract,
      title: "Healthy direct bounty",
      status: "claimable",
      verification_ready: true,
      contract_terms: {
        protocol_version: "agent-bounties/autonomous-v1",
        creator_wallet: address("c"),
        network: "base-mainnet",
        settlement_token: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        solver_reward: { amount: 900000, currency: "usdc" },
        verifier_reward: { amount: 100000, currency: "usdc" },
        claim_bond: { amount: 100000, currency: "usdc" },
        initial_funding: { amount: 1000000, currency: "usdc" },
        funding_deadline: 1787636789,
        claim_window_seconds: 604800,
        verification_window_seconds: 259200,
        creation_nonce: hash("n"),
      },
      ...overrides,
    };
  }

  // 1. Missing args
  expectRun("missing args", [], 2, { ok: false, errors: ["feed_path_required"] });

  // 2. Unreadable file
  expectRun("unreadable feed", [join(temporary, "absent.json")], 2, { ok: false, errors: ["feed_not_found"] });

  // 3. Invalid JSON
  expectRun("invalid JSON", [fixture("feed-invalid.json", "{", true)], 2, { ok: false, errors: ["feed_invalid_json"] });

  // 4. Feed not an array
  expectRun("feed not array", [fixture("feed-not-array.json", {})], 2, { ok: false, errors: ["feed_must_be_array"] });

  // 5. Healthy bounty passes through
  const healthyBounty = healthy();
  expectRun("healthy bounty", [fixture("feed-healthy.json", [healthyBounty])], 0, {
    ok: true,
    ready_count: 1,
    excluded_count: 0,
    ready: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Healthy direct bounty",
        status: "claimable",
        verification_ready: true,
      },
    ],
    excluded: [],
  });

  // 6. Verification not ready is excluded
  const notReady = healthy({ verification_ready: false, title: "Not ready bounty" });
  expectRun("verification not ready", [fixture("feed-not-ready.json", [notReady])], 0, {
    ok: true,
    ready_count: 0,
    excluded_count: 1,
    ready: [],
    excluded: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Not ready bounty",
        exclusion_reasons: ["verification_not_ready"],
      },
    ],
  });

  // 7. Terminal status is excluded
  const terminalBounty = healthy({ status: "settled", title: "Settled bounty" });
  expectRun("terminal status", [fixture("feed-terminal.json", [terminalBounty])], 0, {
    ok: true,
    ready_count: 0,
    excluded_count: 1,
    ready: [],
    excluded: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Settled bounty",
        exclusion_reasons: ["terminal_status"],
      },
    ],
  });

  // 8. Recovery-reserved is excluded
  const recoveryBounty = healthy({ status: "recovery_reserved", title: "Recovery bounty" });
  expectRun("recovery reserved", [fixture("feed-recovery.json", [recoveryBounty])], 0, {
    ok: true,
    ready_count: 0,
    excluded_count: 1,
    ready: [],
    excluded: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Recovery bounty",
        exclusion_reasons: ["recovery_reserved"],
      },
    ],
  });

  // 9. Invalid terms is excluded
  const invalidTermsBounty = healthy({
    contract_terms: null,
    title: "Invalid terms bounty",
  });
  expectRun("invalid terms", [fixture("feed-invalid-terms.json", [invalidTermsBounty])], 0, {
    ok: true,
    ready_count: 0,
    excluded_count: 1,
    ready: [],
    excluded: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Invalid terms bounty",
        exclusion_reasons: ["invalid_terms"],
      },
    ],
  });

  // 10. Mixed feed — healthy + 3 excluded states
  const mixedFeed = [
    healthy(),                                                          // ready
    healthy({ verification_ready: false, title: "Not ready" }),         // excluded: verification
    healthy({ status: "expired", title: "Expired" }),                   // excluded: terminal
    healthy({ status: "recovery-pending", title: "Recovery" }),         // excluded: recovery
  ];
  expectRun("mixed feed", [fixture("feed-mixed.json", mixedFeed)], 0, {
    ok: true,
    ready_count: 1,
    excluded_count: 3,
    ready: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Healthy direct bounty",
        status: "claimable",
        verification_ready: true,
      },
    ],
    excluded: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Not ready",
        exclusion_reasons: ["verification_not_ready"],
      },
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Expired",
        exclusion_reasons: ["terminal_status"],
      },
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Recovery",
        exclusion_reasons: ["recovery_reserved"],
      },
    ],
  });

  // 11. Bounty with multiple exclusion reasons
  const multiExcluded = healthy({
    verification_ready: false,
    status: "cancelled",
    title: "Multiple reasons",
  });
  expectRun("multiple exclusion reasons", [fixture("feed-multi.json", [multiExcluded])], 0, {
    ok: true,
    ready_count: 0,
    excluded_count: 1,
    ready: [],
    excluded: [
      {
        bounty_id: id,
        bounty_contract: contract,
        title: "Multiple reasons",
        exclusion_reasons: ["terminal_status", "verification_not_ready"],
      },
    ],
  });
}

try {
  readyToEarnCases();
  console.log("ready_to_earn_filter_benchmark=passed");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}