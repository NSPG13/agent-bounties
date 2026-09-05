"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const source = fs.readFileSync(
  path.join(__dirname, "..", "site", "ai-bounty-handoff.js"),
  "utf8",
);

function element() {
  return {
    dataset: {},
    hidden: true,
    textContent: "",
    value: "",
    addEventListener() {},
    scrollIntoView() {},
  };
}

const elements = new Map([
  ["[data-ai-handoff]", element()],
  ["[data-conversation-log]", { ...element(), append() {} }],
  ["[data-ai-original]", element()],
  ["[data-ai-prompt]", element()],
  ["[data-ai-draft-import]", element()],
  ["[data-ai-import-status]", element()],
  ["[data-composer-status]", element()],
  ["[data-assistant-prompt]", element()],
]);
elements.get("[data-ai-handoff]").querySelectorAll = () => [];
elements.get("[data-ai-handoff]").querySelector = () => null;

const window = {
  addEventListener() {},
  dispatchEvent() {},
  open() {},
};
const document = {
  documentElement: { dataset: {} },
  querySelector(selector) { return elements.get(selector) || null; },
};

vm.runInNewContext(source, {
  window,
  document,
  navigator: {},
  URL,
  JSON,
  Number,
  String,
  Boolean,
  Object,
  Array,
  console,
}, { filename: "site/ai-bounty-handoff.js" });

const api = window.AgentBountyAI;
if (!api || api.mcpUrl !== "https://mcp.agentbounties.app/mcp") {
  throw new Error("user-owned AI handoff API did not initialize");
}

const draft = api.parseDraft(`\`\`\`json
{
  "title": "Publish the water capture design",
  "goal": "Produce a printable, source-backed rooftop rainwater capture design.",
  "acceptance_criteria": ["STL files pass a documented manifold check", "Assembly instructions identify every part"],
  "solver_reward_usdc": "4.00",
  "verifier_reward_usdc": "0.10",
  "task_window_days": 21,
  "source_url": "https://example.com/context",
  "benchmark": {
    "engine": "sandboxed_regression_v1",
    "source": {
      "kind": "github_commit",
      "repository": "NSPG13/agent-bounties",
      "commit": "0fae18cf9be464132cde52dfb9d464d836e8f024",
      "subdirectory": "benchmarks/distribution-v1/glama-onboarding-audit"
    },
    "runner_manifest": {
      "schema_version": "agent-bounties/regression-sandbox-v1"
    }
  },
  "evidence_schema": {
    "type": "object",
    "required": ["source_snapshot_digest"]
  }
}
\`\`\``);

if (draft.task_window_days !== 21 || draft.acceptance_criteria.length !== 2) {
  throw new Error(`valid AI draft was not normalized: ${JSON.stringify(draft)}`);
}
if (draft.benchmark?.source?.commit !== "0fae18cf9be464132cde52dfb9d464d836e8f024"
  || draft.evidence_schema?.required?.[0] !== "source_snapshot_digest") {
  throw new Error("the exact benchmark and evidence schema were stripped from the AI draft");
}

const approvedImage = {
  source: "chatgpt_user_generated",
  asset_url: "https://agentbounties.app/public/bounty-images/approved.webp",
  sha256: "a".repeat(64),
  mime_type: "image/webp",
  prompt: "A restrained editorial illustration of a reproducible API defect.",
  alt_text: "An engineer tracing a reproducible API defect.",
};
const chatgptHandoff = api.parseDraft({
  ...draft,
  image_required: true,
  image: approvedImage,
});
if (chatgptHandoff.image_required !== true || chatgptHandoff.image !== approvedImage) {
  throw new Error("the ChatGPT-owned approved image was stripped from the hosted handoff");
}

for (const invalid of [
  { ...draft, acceptance_criteria: [] },
  { ...draft, solver_reward_usdc: "0" },
  { ...draft, task_window_days: 31 },
  { ...draft, source_url: "http://example.com" },
]) {
  let rejected = false;
  try { api.parseDraft(invalid); } catch (_error) { rejected = true; }
  if (!rejected) throw new Error(`unsafe AI draft was accepted: ${JSON.stringify(invalid)}`);
}

for (const invalid of [
  { ...draft },
  { ...draft, deadline_days: 14 },
]) {
  delete invalid.task_window_days;
  let message = "";
  try { api.parseDraft(invalid); } catch (error) { message = error.message; }
  if (!message.includes("task_window_days")) {
    throw new Error(`AI draft without an exact task window was not rejected clearly: ${message}`);
  }
  if ("deadline_days" in invalid && !message.includes("instead of deadline_days")) {
    throw new Error(`deadline_days alias did not receive a targeted correction: ${message}`);
  }
}

const prompt = api.promptFor("Build a public climate dashboard");
for (const marker of ["prepare_bounty_post", api.mcpUrl, "return ONLY one JSON object", "Otherwise omit all three image fields", "exact public sandboxed_regression_v1 benchmark", "Do not claim that anything is posted"]) {
  if (!prompt.includes(marker)) throw new Error(`AI handoff prompt missing: ${marker}`);
}

console.log("user-owned AI handoff validates portable drafts and preserves the MCP path");
