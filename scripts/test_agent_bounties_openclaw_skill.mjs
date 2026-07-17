import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

import {
  collectInventory,
  githubIssueNumberFromSourceUrl,
  keccak256Hex,
  normalizeApiBaseUrl,
  rpcBatchTransport,
  STANDING_META_BOUNTY,
  verifyClaimableItem,
  verifyDirectChainInventory,
} from "../skills/agent-bounties/scripts/check-in.mjs";

async function fixture(name) {
  return JSON.parse(
    await readFile(new URL(`../skills/agent-bounties/fixtures/${name}`, import.meta.url), "utf8"),
  );
}

function abiWord(value) {
  const bigint = typeof value === "bigint" ? value : BigInt(value);
  return `0x${bigint.toString(16).padStart(64, "0")}`;
}

function abiAddress(value) {
  return `0x${value.toLowerCase().slice(2).padStart(64, "0")}`;
}

async function directManifest() {
  return JSON.parse(
    await readFile(
      new URL(
        "../skills/agent-bounties/fixtures/base-mainnet-canaries.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
}

async function permissionlessDirectManifest() {
  const manifest = await directManifest();
  for (const bounty of manifest.bounties) {
    bounty.verification_mode = "deterministic_module";
    bounty.verifier_module = "0x8888888888888888888888888888888888888888";
    bounty.verifier_runtime_code_hash = `0x${"77".repeat(32)}`;
    bounty.verifier_set_hash = `0x${"00".repeat(32)}`;
  }
  return manifest;
}

test("Base RPC transport chunks public-endpoint batches and retries rate limits", async () => {
  const calls = Array.from({ length: 12 }, (_, index) => ({
    key: `call_${index}`,
    method: "eth_call",
    params: [{ to: "0x0000000000000000000000000000000000000000" }, "safe"],
  }));
  const payloadSizes = [];
  const sleeps = [];
  let rateLimited = false;
  const fetchImpl = async (_url, options) => {
    const payload = JSON.parse(options.body);
    payloadSizes.push(payload.length);
    if (payload[0].id === 1 && !rateLimited) {
      rateLimited = true;
      return {
        ok: true,
        status: 200,
        json: async () => payload.map(({ id }) => (id <= 5 ? {
          jsonrpc: "2.0",
          id,
          result: `0x${id.toString(16)}`,
        } : {
          jsonrpc: "2.0",
          id,
          error: { code: -32016, message: "over rate limit" },
        })),
      };
    }
    return {
      ok: true,
      status: 200,
      json: async () => payload.map(({ id }) => ({
        jsonrpc: "2.0",
        id,
        result: `0x${id.toString(16)}`,
      })),
    };
  };

  const results = await rpcBatchTransport("https://rpc.example.test", calls, {
    fetchImpl,
    sleepImpl: async (milliseconds) => sleeps.push(milliseconds),
    batchSize: 50,
    minIntervalMs: 0,
  });

  assert.deepEqual(payloadSizes, [10, 5, 2]);
  assert.deepEqual([...results.keys()], calls.map((call) => call.key));
  assert.equal(results.get("call_11"), "0xc");
  assert.deepEqual(sleeps, [500]);
});

test("local Ethereum Keccak-256 matches canonical vectors", () => {
  assert.equal(
    keccak256Hex("0x"),
    "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
  );
  assert.equal(
    keccak256Hex("0x68656c6c6f"),
    "0x1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8",
  );
});

test("source issue parsing accepts only exact public GitHub issue URLs", () => {
  assert.equal(
    githubIssueNumberFromSourceUrl("https://github.com/NSPG13/agent-bounties/issues/275"),
    275,
  );
  for (const value of [
    undefined,
    "",
    "not a URL",
    "http://github.com/NSPG13/agent-bounties/issues/275",
    "https://github.com/NSPG13/agent-bounties/pull/275",
    "https://github.com/NSPG13/agent-bounties/issues/0",
    "https://github.com/NSPG13/agent-bounties/issues/275?claim=true",
    "https://github.com/NSPG13/agent-bounties/issues/275#comment",
    "https://example.com/NSPG13/agent-bounties/issues/275",
  ]) assert.equal(githubIssueNumberFromSourceUrl(value), null);
});

function directTransport(manifest, mutate = null) {
  const byIssue = new Map(manifest.bounties.map((item) => [String(item.issue), item]));
  return async (_rpcUrl, calls) => {
    const results = new Map();
    for (const call of calls) {
      let result;
      if (call.key === "safe_block") {
        result = {
          number: "0x12345",
          hash: "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        };
      } else if (call.key === "factory_proof") {
        result = { codeHash: manifest.factory_runtime_code_hash };
      } else if (call.key === "implementation_proof") {
        result = { codeHash: manifest.implementation_runtime_code_hash };
      } else if (call.key === "factory_protocol") {
        result = manifest.protocol_hash;
      } else if (call.key === "factory_implementation") {
        result = abiAddress(manifest.implementation);
      } else if (call.key === "factory_token") {
        result = abiAddress(manifest.native_usdc);
      } else {
        const match = /^bounty_([0-9]+)_(.+)$/.exec(call.key);
        assert.ok(match, `unexpected direct RPC key ${call.key}`);
        const bounty = byIssue.get(match[1]);
        assert.ok(bounty, `missing bounty fixture ${match[1]}`);
        const field = match[2];
        const fields = {
          proof: { codeHash: manifest.bounty_proxy_runtime_code_hash },
          canonical: abiWord(1),
          protocol: manifest.protocol_hash,
          id: bounty.bounty_id,
          creator: abiAddress(bounty.creator),
          factory: abiAddress(manifest.factory),
          token: abiAddress(manifest.native_usdc),
          solver_reward: abiWord(bounty.solver_reward_minor),
          verifier_reward: abiWord(bounty.verifier_reward_minor),
          target: abiWord(bounty.target_minor),
          funded: abiWord(bounty.target_minor),
          status: abiWord(1),
          timeout_bonus: abiWord(0),
          terms: bounty.terms_hash,
          policy: bounty.policy_hash,
          acceptance: bounty.acceptance_criteria_hash,
          benchmark: bounty.benchmark_hash,
          evidence: bounty.evidence_schema_hash,
          verifier_set: bounty.verifier_set_hash || manifest.verifier_set_hash,
          verification_mode: abiWord({
            deterministic_module: 0,
            signed_quorum: 1,
            ai_judge_quorum: 2,
          }[bounty.verification_mode]),
          verifier_module: abiAddress(
            bounty.verifier_module || "0x0000000000000000000000000000000000000000",
          ),
          verifier_proof: { codeHash: bounty.verifier_runtime_code_hash },
          token_balance: abiWord(bounty.target_minor),
          solver_balance: abiWord(500000),
          allowance: abiWord(0),
        };
        result = fields[field];
        assert.notEqual(result, undefined, `unhandled direct field ${field}`);
      }
      results.set(call.key, mutate ? mutate(call.key, result) : result);
    }
    return results;
  };
}

function standingMetaTransport(runtimeCodeHash = STANDING_META_BOUNTY.verifierRuntimeCodeHash) {
  return async (_rpcUrl, calls) => new Map(calls.map((call) => {
    if (call.key === "standing_meta_safe_block") {
      return [call.key, {
        number: "0x12345",
        hash: `0x${"dd".repeat(32)}`,
      }];
    }
    assert.equal(call.key, "standing_meta_verifier_proof");
    return [call.key, { codeHash: runtimeCodeHash }];
  }));
}

function makeHostedStandingMetaCandidate(item) {
  item.verifier_module = STANDING_META_BOUNTY.verifierModule;
  item.terms.acceptance_criteria_hash = STANDING_META_BOUNTY.acceptanceCriteriaHash;
  item.terms.document.acceptance_criteria = [...STANDING_META_BOUNTY.acceptanceCriteria];
  item.terms.document.benchmark = {
    engine: "standing_meta_v2_parent",
    lane: "cli",
    required_child_engine: STANDING_META_BOUNTY.childEngine,
    required_child_status: "settled",
    required_child_verifier_set_hash: STANDING_META_BOUNTY.childVerifierSetHash,
    required_child_verifier_threshold: STANDING_META_BOUNTY.childVerifierThreshold,
    participant_registry: STANDING_META_BOUNTY.participantRegistry,
    terms_registry: STANDING_META_BOUNTY.termsRegistry,
  };
  item.terms.document.evidence_schema = {
    type: "object",
    required: [...STANDING_META_BOUNTY.requiredEvidence],
  };
  item.terms.document.verification_policy = {
    mechanism: "deterministic_module",
    verifier_module: STANDING_META_BOUNTY.verifierModule,
    threshold: 1,
    self_verification_forbidden: true,
  };
  return item;
}

test("portable skill metadata and install contracts remain publishable", async () => {
  const skill = await readFile(
    new URL("../skills/agent-bounties/SKILL.md", import.meta.url),
    "utf8",
  );
  const readme = await readFile(new URL("../README.md", import.meta.url), "utf8");
  const llms = await readFile(new URL("../site/llms.txt", import.meta.url), "utf8");
  const discovery = JSON.parse(
    await readFile(
      new URL("../site/.well-known/agent-bounties.json", import.meta.url),
      "utf8",
    ),
  );
  const distribution = await readFile(
    new URL("../docs/openclaw-distribution.md", import.meta.url),
    "utf8",
  );
  const grouping = JSON.parse(
    await readFile(new URL("../skills.sh.json", import.meta.url), "utf8"),
  );
  const plugin = JSON.parse(
    await readFile(
      new URL(
        "../skills/agent-bounties/.claude-plugin/plugin.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const marketplace = JSON.parse(
    await readFile(
      new URL("../.claude-plugin/marketplace.json", import.meta.url),
      "utf8",
    ),
  );
  const chainManifest = await directManifest();
  const activation = JSON.parse(
    await readFile(
      new URL("../deployments/base-mainnet-activation.json", import.meta.url),
      "utf8",
    ),
  );
  const standingMetaActivation = JSON.parse(
    await readFile(
      new URL(
        "../deployments/canonical-child-seeds-base-mainnet.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const activationItems = [...activation.bounties, ...standingMetaActivation.bounties];

  assert.match(skill, /^---\r?\nname: agent-bounties\r?\n/);
  assert.match(skill, /\r?\nversion: 1\.4\.3\r?\n/);
  assert.match(skill, /\r?\nauthor: Agent Bounties contributors\r?\n/);
  assert.match(skill, /\r?\n  hermes:\r?\n/);
  assert.match(skill, /\r?\n    category: agent-commerce\r?\n/);
  assert.match(skill, /\r?\n  openclaw:\r?\n/);
  assert.match(skill, /\r?\n      bins: \[node\]\r?\n/);

  assert.equal(grouping.$schema, "https://skills.sh/schemas/skills.sh.schema.json");
  const categories = grouping.groupings.filter((item) =>
    item.skills.includes("agent-bounties"),
  );
  assert.deepEqual(categories.map((item) => item.title), ["Agent Commerce"]);

  assert.equal(plugin.name, "agent-bounties");
  assert.equal(plugin.displayName, "Agent Bounties");
  assert.equal(plugin.version, "1.4.3");
  assert.equal(plugin.license, "MIT");
  assert.equal(plugin.repository, "https://github.com/NSPG13/agent-bounties");
  assert.equal(plugin.homepage, "https://nspg13.github.io/agent-bounties/");
  assert.equal(plugin.mcpServers, undefined);
  assert.equal(plugin.hooks, undefined);
  assert.equal(plugin.experimental, undefined);

  assert.equal(marketplace.name, "agent-bounties");
  assert.deepEqual(marketplace.owner, { name: "Agent Bounties contributors" });
  assert.equal(marketplace.plugins.length, 1);
  assert.equal(marketplace.plugins[0].name, "agent-bounties");
  assert.equal(marketplace.plugins[0].source, "./skills/agent-bounties");
  assert.equal(marketplace.plugins[0].mcpServers, undefined);
  assert.equal(marketplace.plugins[0].hooks, undefined);

  const portableHelperUrl = "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/scripts/check-in.mjs";
  const directManifestUrl = "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/fixtures/base-mainnet-canaries.json";
  assert.equal(discovery.endpoints.portable_inventory_helper, portableHelperUrl);
  assert.equal(discovery.endpoints.direct_chain_canary_manifest, directManifestUrl);
  assert.ok(llms.includes(portableHelperUrl));
  assert.ok(llms.includes(directManifestUrl));
  assert.ok(llms.includes("--solver-wallet 0xYourPublicBaseAddress"));
  assert.ok(llms.includes("autonomous-bounty-plan"));
  assert.ok(skill.includes("autonomous-bounty-plan"));
  assert.ok(skill.includes("label:claimable-live"));
  assert.ok(skill.includes("funding-needed` is a crowdfunding opportunity"));

  assert.equal(chainManifest.factory, activation.deployment.expected_factory);
  assert.equal(chainManifest.implementation, activation.deployment.expected_implementation);
  assert.equal(
    chainManifest.factory_runtime_code_hash,
    activation.deployment.factory_runtime_code_hash,
  );
  assert.equal(
    chainManifest.implementation_runtime_code_hash,
    activation.deployment.implementation_runtime_code_hash,
  );
  for (const item of chainManifest.bounties) {
    const sourceTerms = JSON.parse(
      await readFile(
        new URL(`../bounties/autonomous-v1/${item.issue}.json`, import.meta.url),
        "utf8",
      ),
    );
    const bundledTerms = JSON.parse(
      await readFile(
        new URL(`../skills/agent-bounties/${item.terms_path}`, import.meta.url),
        "utf8",
      ),
    );
    const activationItem = activationItems.find((entry) => entry.issue === item.issue);
    assert.ok(activationItem, `missing activation artifact for issue #${item.issue}`);
    assert.deepEqual(bundledTerms, sourceTerms);
    assert.equal(item.title, sourceTerms.title);
    assert.equal(item.bounty_id, activationItem.bounty_id);
    assert.equal(item.contract, activationItem.predicted_bounty_contract);
    assert.equal(item.terms_hash, activationItem.commitments.terms_hash);
    assert.equal(item.policy_hash, activationItem.commitments.policy_hash);
    assert.equal(item.acceptance_criteria_hash, activationItem.commitments.acceptance_criteria_hash);
    assert.equal(item.benchmark_hash, activationItem.commitments.benchmark_hash);
    assert.equal(item.evidence_schema_hash, activationItem.commitments.evidence_schema_hash);
  }

  const commands = [
    "npx skills add NSPG13/agent-bounties --skill agent-bounties --yes",
    "claude plugin marketplace add NSPG13/agent-bounties",
    "claude plugin install agent-bounties@agent-bounties --scope user",
    "hermes skills install NSPG13/agent-bounties/skills/agent-bounties",
    "openclaw skills install git:NSPG13/agent-bounties@main --as agent-bounties",
  ];
  for (const command of commands) {
    assert.ok(readme.includes(command), `README is missing ${command}`);
    assert.ok(distribution.includes(command), `distribution docs are missing ${command}`);
  }

  const bundleFiles = [
    "LICENSE",
    ".claude-plugin/plugin.json",
    "README.md",
    "SKILL.md",
    "fixtures/unavailable.json",
    "fixtures/base-mainnet-canaries.json",
    "fixtures/terms/168.json",
    "fixtures/terms/169.json",
    "fixtures/terms/170.json",
    "fixtures/terms/171.json",
    "fixtures/terms/217.json",
    "fixtures/terms/218.json",
    "fixtures/terms/219.json",
    "fixtures/terms/220.json",
    "fixtures/verified-claimable.json",
    "references/payment-truth.md",
    "scripts/check-in.mjs",
  ];
  for (const path of bundleFiles) {
    await access(new URL(`../skills/agent-bounties/${path}`, import.meta.url));
  }
});

test("direct safe-chain verifier includes deterministic inventory and excludes unattested quorum", async () => {
  const manifest = await directManifest();
  const solver = "0x7777777777777777777777777777777777777777";
  const report = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest),
    solverWallet: solver,
  });

  const deterministic = manifest.bounties.filter(
    (item) => item.verification_mode === "deterministic_module",
  );
  const quorum = manifest.bounties.filter(
    (item) => item.verification_mode !== "deterministic_module",
  );
  assert.equal(report.status, "verified");
  assert.equal(report.observed_block.tag, "safe");
  assert.equal(report.verified.length, deterministic.length);
  assert.equal(report.excluded.length, quorum.length);
  assert.ok(report.excluded.every((item) => item.reason === "quorum_verifier_service_not_attested"));
});

test("direct safe-chain verifier accepts deterministic module earning inventory", async () => {
  const manifest = await permissionlessDirectManifest();
  const solver = "0x7777777777777777777777777777777777777777";
  const report = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest),
    solverWallet: solver,
  });

  assert.equal(report.status, "verified");
  assert.equal(report.verified.length, manifest.bounties.length);
  assert.equal(report.excluded.length, 0);
  for (const item of report.verified) {
    assert.equal(item.evidence_source, "direct_safe_chain");
    assert.equal(item.source_issue_number, item.issue);
    assert.equal(item.claim_plan.ready, true);
    assert.deepEqual(
      item.claim_plan.wallet_calls.map((call) => call.function),
      ["approve(address,uint256)", "claim()"],
    );
    assert.equal(item.claim_plan.wallet_calls[0].from, solver);
    assert.equal(item.claim_plan.wallet_calls[1].to, item.contract);
  }
});

test("direct safe-chain verifier uses each bounty's verifier-set commitment", async () => {
  const manifest = await permissionlessDirectManifest();
  const report = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest),
  });
  assert.equal(report.verified.length, manifest.bounties.length);
  assert.ok(report.verified.every((item) => item.verifier_set_hash === `0x${"00".repeat(32)}`));

  const drifted = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest, (key, value) => (
      key.endsWith("_verifier_set") ? manifest.verifier_set_hash : value
    )),
  });
  assert.equal(drifted.verified.length, 0);
  assert.equal(drifted.excluded.length, manifest.bounties.length);
});

test("direct safe-chain verifier fails closed on factory code mismatch", async () => {
  const manifest = await directManifest();
  const report = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest, (key, value) => (
      key === "factory_proof"
        ? { codeHash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
        : value
    )),
  });

  assert.equal(report.status, "verification_failed");
  assert.equal(report.verified.length, 0);
  assert.equal(report.warning, "direct_safe_chain_verification_failed");
});

test("direct safe-chain verifier excludes one bounty with altered terms", async () => {
  const manifest = await permissionlessDirectManifest();
  const report = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest, (key, value) => (
      key === "bounty_169_terms"
        ? "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        : value
    )),
  });

  assert.equal(
    report.status,
    manifest.bounties.length === 1 ? "no_claimable_bounties" : "verified",
  );
  assert.equal(report.verified.length, manifest.bounties.length - 1);
  assert.deepEqual(report.excluded.map((item) => item.id), [manifest.bounties[0].bounty_id]);
});

test("direct safe-chain inventory marks only the exact canonical child verifier as meta", async () => {
  const manifest = await permissionlessDirectManifest();
  const bounty = manifest.bounties[0];
  bounty.verifier_module = STANDING_META_BOUNTY.verifierModule;
  bounty.verifier_runtime_code_hash = STANDING_META_BOUNTY.verifierRuntimeCodeHash;
  bounty.acceptance_criteria_hash = STANDING_META_BOUNTY.acceptanceCriteriaHash;
  const report = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest),
  });

  assert.equal(report.verified.length, manifest.bounties.length);
  const marked = report.verified.filter((item) => item.standing_meta_bounty);
  assert.equal(marked.length, 1);
  assert.equal(
    marked[0].standing_meta_bounty.inventory_class,
    STANDING_META_BOUNTY.inventoryClass,
  );
  assert.equal(marked[0].standing_meta_bounty.requires_different_solver_wallet, true);
});

test("direct safe-chain inventory rejects deterministic verifier bytecode drift", async () => {
  const manifest = await permissionlessDirectManifest();
  const report = await verifyDirectChainInventory({
    manifest,
    rpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest, (key, value) => (
      key.endsWith("_verifier_proof") ? { codeHash: `0x${"66".repeat(32)}` } : value
    )),
  });

  assert.equal(report.verified.length, 0);
  assert.equal(report.excluded.length, manifest.bounties.length);
});

test("unavailable hosted services fall back to direct safe-chain inventory", async () => {
  const manifest = await permissionlessDirectManifest();
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("unavailable.json"),
    chainManifest: manifest,
    baseRpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest),
  });

  assert.equal(report.hosted_api_healthy, false);
  assert.equal(report.protocol_status, "active");
  assert.equal(report.protocol_source, "direct_safe_chain");
  assert.equal(report.direct_chain_status, "verified");
  assert.equal(report.verified_claimable_bounties.length, manifest.bounties.length);
  assert.equal(report.recommended_action, "claim_verified_bounty");
  assert.ok(report.warnings.includes("autonomous_feed_unavailable"));
  assert.ok(!report.warnings.includes("autonomous_protocol_not_active"));
});

test("verified direct factory stays active when no bundled bounty is claimable", async () => {
  const manifest = await permissionlessDirectManifest();
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("unavailable.json"),
    chainManifest: manifest,
    baseRpcUrl: "https://rpc.example.test",
    rpcTransport: directTransport(manifest, (key, value) => (
      key.endsWith("_status") ? abiWord(2) : value
    )),
  });

  assert.equal(report.protocol_status, "active");
  assert.equal(report.protocol_source, "direct_safe_chain");
  assert.equal(report.direct_chain_status, "no_claimable_bounties");
  assert.equal(report.verified_claimable_bounties.length, 0);
  assert.equal(report.excluded_claimable_candidates.length, manifest.bounties.length);
  assert.ok(!report.warnings.includes("autonomous_protocol_not_active"));
  assert.ok(report.warnings.includes("no_verified_funded_bounty_is_claimable"));
});

test("only active canonical autonomous inventory is claimable", async () => {
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("verified-claimable.json"),
  });

  assert.equal(report.hosted_api_healthy, true);
  assert.equal(report.protocol_status, "active");
  assert.equal(report.verified_claimable_bounties.length, 1);
  assert.equal(
    report.verified_claimable_bounties[0].id,
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  );
  assert.equal(
    report.verified_claimable_bounties[0].evidence,
    "confirmed_canonical_autonomous_bounty",
  );
  assert.equal(
    report.verified_claimable_bounties[0].claim_plan_url,
    "https://api.example.test/v1/base/autonomous-bounties/claim-plan",
  );
  assert.equal(
    report.verified_claimable_bounties[0].source_url,
    "https://github.com/NSPG13/agent-bounties/issues/275",
  );
  assert.equal(report.verified_claimable_bounties[0].source_issue_number, 275);
  assert.equal(report.verified_claimable_bounties[0].claim_handoff.ready, false);
  assert.equal(
    report.verified_claimable_bounties[0].claim_handoff.reason,
    "public_solver_wallet_required",
  );
  assert.equal(
    report.verified_claimable_bounties[0].claim_handoff.github_claim.comment_body_template,
    "/claim #275 wallet: 0xYOUR_PUBLIC_BASE_ADDRESS",
  );
  assert.equal(report.next_action.action, "rerun_with_solver_wallet");
  assert.equal(report.next_action.required_input, "public_base_address");
  assert.deepEqual(report.next_action.never_request, ["private_key", "seed_phrase"]);
  assert.deepEqual(
    report.excluded_claimable_candidates.map((item) => [item.id, item.reason]),
    [
      [
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "terms_or_contract_commitments_invalid",
      ],
    ],
  );
  assert.equal(report.recommended_action, "claim_verified_bounty");
  assert.equal(report.funding_candidates.length, 1);
  assert.equal(report.live_verification_jobs.length, 1);
});

test("hosted check-in emits an exact side-effect-free claim handoff", async () => {
  const solver = "0x7777777777777777777777777777777777777777";
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("verified-claimable.json"),
    solverWallet: solver,
  });
  const bounty = report.verified_claimable_bounties[0];
  const handoff = bounty.claim_handoff;

  assert.equal(handoff.schema_version, "agent-bounties/check-in-claim-handoff-v1");
  assert.equal(handoff.ready, true);
  assert.equal(handoff.ready_scope, "claim_handoff_only");
  assert.equal(handoff.wallet_readiness_checked, false);
  assert.equal(handoff.reason, "claim_handoff_complete");
  assert.equal(handoff.preferred_path, "github_claim_comment");
  assert.equal(handoff.github_claim.issue_url, bounty.source_url);
  assert.equal(handoff.github_claim.comment_body, `/claim #275 wallet: ${solver}`);
  assert.equal(handoff.mcp.tool, "agent_native_claim");
  assert.deepEqual(handoff.mcp.arguments, {
    idempotency_key: `portable-check-in:${bounty.contract.slice(2)}:${solver.slice(2)}`,
    network: "base-mainnet",
    bounty_contract: bounty.contract,
    solver_wallet: solver,
    request_bond_sponsorship: true,
    source: "portable-check-in",
  });
  assert.equal(handoff.api.method, "POST");
  assert.equal(
    handoff.api.url,
    "https://api.example.test/v1/base/autonomous-bounties/claims",
  );
  assert.deepEqual(handoff.api.body, handoff.mcp.arguments);
  assert.match(handoff.evidence_boundary, /did not post a comment/);
  assert.deepEqual(report.next_action, {
    schema_version: "agent-bounties/check-in-claim-handoff-v1",
    action: "post_github_claim_comment",
    ready: true,
    ready_scope: "claim_handoff_only",
    bounty_id: bounty.id,
    source_issue_number: 275,
    issue_url: bounty.source_url,
    comment_body: `/claim #275 wallet: ${solver}`,
    follow_up: handoff.follow_up,
  });
});

test("check-in refuses a creator wallet and invalid public addresses", async () => {
  const input = await fixture("verified-claimable.json");
  const creator = input.autonomous_feed.body[0].creator;
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: input,
    solverWallet: creator,
  });
  const handoff = report.verified_claimable_bounties[0].claim_handoff;
  assert.equal(handoff.ready, false);
  assert.equal(handoff.reason, "creator_cannot_claim");
  assert.equal(handoff.required_input, "non_creator_public_base_address");
  assert.equal(handoff.github_claim.comment_body, null);
  assert.equal(handoff.mcp.arguments, null);
  assert.equal(handoff.api.body, null);
  assert.equal(report.next_action.action, "rerun_with_solver_wallet");

  await assert.rejects(
    collectInventory({
      apiBaseUrl: "https://api.example.test",
      fixture: await fixture("verified-claimable.json"),
      solverWallet: "not-a-wallet",
    }),
    /solver wallet is not an address/,
  );
});

test("hosted inventory keeps absent or malformed source metadata non-authoritative", async () => {
  for (const sourceUrl of [undefined, "not a URL"]) {
    const input = await fixture("verified-claimable.json");
    if (sourceUrl === undefined) delete input.autonomous_feed.body[0].terms.document.source_url;
    else input.autonomous_feed.body[0].terms.document.source_url = sourceUrl;
    const report = await collectInventory({
      apiBaseUrl: "https://api.example.test",
      fixture: input,
    });
    const bounty = report.verified_claimable_bounties[0];
    assert.equal(bounty.source_url, sourceUrl ?? null);
    assert.equal(bounty.source_issue_number, null);
    assert.equal(bounty.claim_handoff.preferred_path, "agent_native_claim");
    assert.equal(bounty.claim_handoff.github_claim, null);
  }
});

test("hosted inventory emits a standing meta marker after exact-code safe-block attestation", async () => {
  const input = await fixture("verified-claimable.json");
  makeHostedStandingMetaCandidate(input.autonomous_feed.body[0]);
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: input,
    baseRpcUrl: "https://rpc.example.test",
    rpcTransport: standingMetaTransport(),
  });

  const meta = report.verified_claimable_bounties[0].standing_meta_bounty;
  assert.equal(meta.schema_version, STANDING_META_BOUNTY.schemaVersion);
  assert.equal(meta.inventory_class, STANDING_META_BOUNTY.inventoryClass);
  assert.equal(meta.observed_block_number, 0x12345);
  assert.ok(!report.warnings.includes("standing_meta_verifier_attestation_failed"));
});

test("hosted meta-looking terms do not count when verifier bytecode differs", async () => {
  const input = await fixture("verified-claimable.json");
  makeHostedStandingMetaCandidate(input.autonomous_feed.body[0]);
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: input,
    baseRpcUrl: "https://rpc.example.test",
    rpcTransport: standingMetaTransport(`0x${"66".repeat(32)}`),
  });

  assert.equal(report.verified_claimable_bounties.length, 1);
  assert.equal(report.verified_claimable_bounties[0].standing_meta_bounty, undefined);
  assert.ok(report.warnings.includes("standing_meta_verifier_code_mismatch"));
});

test("hosted meta-looking terms do not count when the child quorum drifts", async () => {
  const input = await fixture("verified-claimable.json");
  const item = makeHostedStandingMetaCandidate(input.autonomous_feed.body[0]);
  item.terms.document.benchmark.required_child_verifier_set_hash = `0x${"66".repeat(32)}`;
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: input,
    baseRpcUrl: "https://rpc.example.test",
    rpcTransport: standingMetaTransport(),
  });

  assert.equal(report.verified_claimable_bounties.length, 1);
  assert.equal(report.verified_claimable_bounties[0].standing_meta_bounty, undefined);
});

test("hosted quorum bounty is excluded without a service availability attestation", async () => {
  const input = await fixture("verified-claimable.json");
  const item = structuredClone(input.autonomous_feed.body[0]);
  item.verification_mode = "signed_quorum";
  item.verifier_module = null;
  item.verification_ready = false;
  item.verification_readiness_reason =
    "quorum verifier service availability is not canonically attested";

  assert.deepEqual(verifyClaimableItem(item, input.protocol.body), {
    ok: false,
    reason: "verification_path_not_executable",
  });
});

test("unavailable hosted API cannot create imaginary inventory", async () => {
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("unavailable.json"),
  });

  assert.equal(report.hosted_api_healthy, false);
  assert.deepEqual(report.verified_claimable_bounties, []);
  assert.equal(report.recommended_action, "post_own_bounty");
  assert.equal(report.next_action.action, "post_own_bounty");
  assert.ok(report.warnings.includes("hosted_api_health_not_confirmed"));
  assert.ok(report.warnings.includes("autonomous_feed_unavailable"));
  assert.ok(report.warnings.includes("autonomous_protocol_not_active"));
});

test("API URL rejects credentials and insecure remote HTTP", () => {
  assert.throws(
    () => normalizeApiBaseUrl("https://user:secret@api.example.test"),
    /credentials/,
  );
  assert.throws(() => normalizeApiBaseUrl("http://api.example.test"), /HTTPS/);
  assert.equal(normalizeApiBaseUrl("http://127.0.0.1:8080/"), "http://127.0.0.1:8080");
});
