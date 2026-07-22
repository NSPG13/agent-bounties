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
  "source_url": "https://example.com/context"
}
\`\`\``);

if (draft.task_window_days !== 21 || draft.acceptance_criteria.length !== 2) {
  throw new Error(`valid AI draft was not normalized: ${JSON.stringify(draft)}`);
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

const prompt = api.promptFor("Build a public climate dashboard");
for (const marker of ["prepare_bounty_post", api.mcpUrl, "return ONLY one JSON object", "Do not claim that anything is posted"]) {
  if (!prompt.includes(marker)) throw new Error(`AI handoff prompt missing: ${marker}`);
}

console.log("user-owned AI handoff validates portable drafts and preserves the MCP path");
