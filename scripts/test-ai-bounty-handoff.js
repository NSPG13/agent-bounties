"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const assert = require("node:assert/strict");
const postingPrompt = require("../site/posting-prompt.js");

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
    style: {},
    handlers: {},
    addEventListener(name, callback) { this.handlers[name] = callback; },
    removeAttribute(name) { delete this[name]; },
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
  ["[data-ai-web-fallback]", element()],
]);
const chatgptButton = { ...element(), dataset: { aiProvider: "chatgpt", providerLabel: "ChatGPT" } };
elements.get("[data-ai-handoff]").querySelectorAll = () => [chatgptButton];
elements.get("[data-ai-handoff]").querySelector = () => null;

const desktopLaunches = [];
const webLaunches = [];
const copies = [];
const navigator = { clipboard: { async writeText(text) { copies.push(text); } } };
const window = {
  AgentBountiesPostingPrompt: postingPrompt,
  AgentBountiesCreatorReview: require("../site/creator-review.js"),
  AgentBountiesMetaChild: require("../site/meta-child.js"),
  addEventListener() {},
  dispatchEvent() {},
  open(...args) { webLaunches.push(args); },
};
const document = {
  documentElement: { dataset: {} },
  querySelector(selector) { return elements.get(selector) || null; },
  body: { append() {} },
  createElement(tag) {
    assert.equal(tag, "a");
    return { click() { desktopLaunches.push(this.href); }, remove() {} };
  },
};

vm.runInNewContext(source, {
  window,
  document,
  navigator,
  requestAnimationFrame(callback) { callback(); },
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
const child = api.parseDraft({ ...draft, solver_reward_usdc: "0.99", verifier_reward_usdc: "0.01", meta_child: { parent_bounty_contract: "0x" + "11".repeat(20), intended_child_solver: "0x" + "22".repeat(20) } });
if (child.meta_child.intended_child_solver !== "0x" + "22".repeat(20) || child.solver_reward_usdc !== "0.99") throw new Error("The AI handoff lost the exact child context or reward.");
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
assert.equal(api.promptFor(), postingPrompt.build(), "both entry points must use the same canonical prompt");
assert.equal(prompt, postingPrompt.build({ request: "Build a public climate dashboard" }));

async function verifyDesktopHandoff() {
  const context = {
    draft,
    solver_reward_usdc: "0.99",
    verifier_reward_usdc: "0.01",
    task_window_days: 3,
    meta_child: child.meta_child,
  };
  const preparedPrompt = api.show('Keep the tests; change the title to "A & B".', context);
  assert.match(preparedPrompt, /Discover WebMCP tools/);
  assert.match(preparedPrompt, /preserve my answers and draft/);
  assert.match(preparedPrompt, /only for missing business decisions/);
  assert.ok(preparedPrompt.startsWith(postingPrompt.build()));
  assert.match(preparedPrompt, /qualifying_child_constraints/);
  assert.match(preparedPrompt, /"total_funding_usdc": "1.00"/);
  assert.match(preparedPrompt, /distinct_intended_child_solver_required/);
  assert.match(preparedPrompt, /ordinary hosted prepare_bounty_post/);
  assert.ok(preparedPrompt.includes(draft.benchmark.source.commit));
  assert.ok(preparedPrompt.includes(draft.source_url));
  await chatgptButton.handlers.click();
  assert.equal(desktopLaunches.length, 1);
  assert.equal(webLaunches.length, 0, "desktop selection must not open a web conversation");
  const desktop = new URL(desktopLaunches[0]);
  assert.equal(desktop.protocol, "codex:");
  assert.equal(desktop.hostname, "threads");
  assert.equal(desktop.pathname, "/new");
  assert.equal(desktop.searchParams.get("prompt"), preparedPrompt);
  const browser = new URL(desktop.searchParams.get("browserUrl"));
  assert.equal(browser.origin, "https://agentbounties.app");
  assert.equal(browser.pathname, "/post.html");
  assert.equal(browser.searchParams.get("parentBounty"), child.meta_child.parent_bounty_contract);
  const fallback = elements.get("[data-ai-web-fallback]");
  assert.equal(fallback.hidden, false);
  assert.equal(new URL(fallback.href).searchParams.get("prompt"), preparedPrompt);
  assert.equal(copies[0], preparedPrompt);
  assert.match(elements.get("[data-ai-import-status]").textContent, /launch requested/i);
  assert.doesNotMatch(elements.get("[data-ai-import-status]").textContent, /app opened|blocked the new tab/i);
  api.show("A different outcome");
  assert.equal(fallback.hidden, true, "new drafts must clear old fallback links");
  assert.equal(fallback.href, undefined);
  navigator.clipboard.writeText = async () => { throw new Error("clipboard denied"); };
  await chatgptButton.handlers.click();
  assert.equal(desktopLaunches.length, 2, "clipboard denial must not prevent the prefilled launch");
  assert.equal(webLaunches.length, 0);
  assert.equal(new URL(desktopLaunches[1]).searchParams.get("prompt"), elements.get("[data-ai-prompt]").value);
  assert.match(elements.get("[data-ai-import-status]").textContent, /clipboard access was unavailable/i);
  assert.equal(api.providerLinks("unknown", "draft"), null);
  assert.equal(api.providerLinks("chatgpt", ""), null);
  assert.throws(() => api.providerLinks("chatgpt", "draft", { meta_child: { parent_bounty_contract: "https://evil.example" } }));
  console.log("user-owned AI handoff validates drafts, desktop launch, explicit fallback, preserved context and clipboard recovery");
}
verifyDesktopHandoff().catch((error) => { console.error(error); process.exitCode = 1; });
