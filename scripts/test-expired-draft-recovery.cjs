"use strict";
// Saved-draft recovery contract: all browser requests are intercepted and synthetic.
const fs = require("node:fs"), path = require("node:path"), assert = require("node:assert/strict");
const { createHash } = require("node:crypto");
const root = path.resolve(process.env.RECOVERY_SOURCE_ROOT || path.join(__dirname, ".."));
let playwright;
try { playwright = require("../tools/browser-layout/node_modules/playwright"); }
catch { playwright = require("playwright"); }
const { chromium } = playwright;
const session = require(path.join(root, "site/posting-session.js"));
const origin = "http://127.0.0.1:39187", site = path.join(root, "site");
const operation = "11111111-1111-4111-8111-111111111111";
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".webp": "image/webp" };
const output = { observed_at: new Date().toISOString(), kind: "offline browser regression", external_requests_sent: 0, external_adoption: false, cases: [] };
function fixture(expired) {
  const cutoff = new Date(Date.now() + (expired ? -1 : 7) * 86400000).toISOString();
  const draft = { title: "Synthetic recovery fixture", goal: "Return to the same draft", acceptance_criteria: ["Preserve the saved terms"], solver_reward_usdc: "9", verifier_reward_usdc: "1", task_window_days: 3,
    review_mode: "creator", delivery_deadline: cutoff, benchmark: { engine: "creator_review_v1", delivery_deadline: Math.floor(Date.parse(cutoff) / 1000), acceptance: "all_published_criteria", reviewer: "creator" },
    evidence_schema: require(path.join(root, "site/creator-review.js")).evidenceSchema(), reference_attachment: null };
  const journey = { schema: "agent-bounties/guided-journey-v1", id: operation, role: "post", goal: draft.goal, preferences: "", steps: {}, draft, draft_stale: false, reference_attachment: null,
    brief: { goal: draft.goal, budget_usdc: "10", deadline_at: cutoff }, updated_at: new Date().toISOString() };
  const envelope = session.envelope(journey), hash = createHash("sha256").update(session.stable(envelope)).digest("hex");
  return { journey, row: { operation_id: operation, draft: envelope, draft_hash: hash, approved_draft_hash: hash, recovery_state: {}, revision: 2, updated_at: journey.updated_at } };
}
(async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    for (const postPath of ["/post.html", "/post"]) for (const expired of [true, false]) {
      const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
      const f = fixture(expired), requests = [], pageErrors = [];
      await context.route("**/*", async route => {
        const request = route.request(), url = new URL(request.url());
        if (url.pathname === "/v1/site-auth/session" || url.pathname === "/auth/session") return route.fulfill({ json: { authenticated: true, account_status: "ready", account_complete: true, posting_drafts_enabled: true, user: { id: "synthetic-recovery-account" }, providers: { github: true } } });
        if (url.pathname === "/v1/site-auth/account" || url.pathname === "/auth/account") return route.fulfill({ json: { authenticated: true, account_status: "ready", account_complete: true, wallets: [], data_status: "unavailable" } });
        if (url.pathname === "/v1/site-auth/posting-drafts/" + operation) {
          requests.push(request.method());
          if (request.method() === "GET") return route.fulfill({ json: f.row });
          assert.equal(request.method(), "POST");
          const data = request.postDataJSON();
          const hash = createHash("sha256").update(session.stable(data.draft)).digest("hex");
          f.row = { ...f.row, draft: data.draft, draft_hash: hash, approved_draft_hash: data.approved_draft_hash, recovery_state: data.recovery_state, revision: f.row.revision + 1 };
          return route.fulfill({ json: f.row });
        }
        if (url.pathname === "/v1/base/gas-sponsorship") return route.fulfill({ json: { network: "base-mainnet", creation: { available: false } } });
        if (url.origin !== origin) return route.abort();
        const file = path.resolve(site, "." + (url.pathname === "/post" ? "/post.html" : url.pathname));
        if (!file.startsWith(site + path.sep)) return route.abort();
        try { return route.fulfill({ body: fs.readFileSync(file), contentType: mime[path.extname(file)] || "application/octet-stream" }); }
        catch { return route.fulfill({ status: 404, body: "Not found" }); }
      });
      await context.addInitScript(() => {
        window.__walletCalls = [];
        window.ethereum = { request: async ({ method }) => { window.__walletCalls.push(method); throw Error("No wallet use in recovery diagnostic"); } };
      });
      const page = await context.newPage();
      page.on("pageerror", e => pageErrors.push(e.message));
      await page.goto(origin + postPath + "?operation_id=" + operation + "&analytics=off" + (expired ? "&funding_review=1" : ""));
      async function inspect(phase) {
        await page.waitForFunction(() => window.AgentBountiesWebMCP && window.AgentBountiesComposer);
        const value = await page.evaluate(async () => {
          const tools = {};
          for (const name of ["agent_bounties_get_bounty_review", "agent_bounties_get_journey"]) {
            try { tools[name] = { result: await window.AgentBountiesWebMCP.call(name) }; }
            catch (error) { tools[name] = { error: error.message }; }
          }
          const journey = window.AgentBountiesWorkflow.createClient(window).load();
          return { tools, stored: { operation_id: journey?.id, draft: journey?.draft, draft_stale: journey?.draft_stale }, ui: { status: document.querySelector("[data-composer-status]")?.textContent, preview_hidden: document.getElementById("bounty-preview")?.hidden, funding_disabled: document.querySelector("[data-open-funding]")?.disabled }, wallet_calls: window.__walletCalls };
        });
        assert.equal(value.stored.operation_id, operation);
        assert.equal(value.stored.draft.delivery_deadline, f.journey.draft.delivery_deadline);
        assert.deepEqual(value.stored.draft, f.journey.draft);
        assert.deepEqual(value.stored.draft.acceptance_criteria, f.journey.draft.acceptance_criteria);
        assert.deepEqual(value.wallet_calls, []);
        output.cases.push({ postPath, expired, phase, ...value, fixture_http_methods: [...requests], page_errors: [...pageErrors] });
      }
      await inspect("cold-account-continuation");
      await page.reload();
      await inspect("warm-reload");
      await inspect("repeated-read");
      assert.equal(requests.includes("POST"), false, "Recovery reads must not rewrite saved account terms");
      const originalHash = f.row.approved_draft_hash;
      for (const record of output.cases.filter(c => c.postPath === postPath && c.expired === expired)) {
        const review = record.tools.agent_bounties_get_bounty_review.result;
        assert.ok(review, "Review must remain readable");
        assert.equal(review.review_mode, "creator");
        assert.equal(review.delivery_deadline, f.journey.draft.delivery_deadline);
        assert.equal(review.explicitly_approved, true, "Preserve historical approval of unchanged terms");
        assert.equal(review.saved_operation.revision, 2);
        assert.equal(review.saved_operation.operation_id, operation);
        assert.equal(review.funding_ready, !expired);
        assert.equal(record.ui.preview_hidden, false);
        assert.equal(review.recovery_code, expired ? "delivery_deadline_expired" : null);
        assert.equal(record.tools.agent_bounties_get_journey.result.journey.id, operation);
        if (expired) {
          assert.equal(record.ui.funding_disabled, true);
          assert.match(review.next_action, /new agreed delivery deadline/);
          assert.deepEqual(review.saved_draft, f.journey.draft);
          assert.equal(record.tools.agent_bounties_get_journey.result.next_action.tool, "agent_bounties_stage_funded_bounty");
        }
      }
      if (expired) {
        assert.equal(await page.locator("[data-draft-recovery]").isVisible(), true);
        assert.match(await page.locator("[data-draft-recovery]").textContent(), /Edit the brief.*new deadline/);
        assert.equal(await page.locator('[data-stage-target="review"]').getAttribute("aria-current"), "step");
        assert.equal(await page.locator("[data-open-funding]").textContent(), "Update deadline first");
        if (process.env.RECOVERY_SCREENSHOT) await page.locator("#bounty-preview").screenshot({ path: process.env.RECOVERY_SCREENSHOT });
        const revised = { ...f.journey.draft, delivery_deadline: new Date(Date.now() + 86400000).toISOString(), posting_operation_id: operation };
        delete revised.reference_attachment;
        const result = await page.evaluate(async draft => {
          await window.AgentBountiesWebMCP.call("agent_bounties_stage_funded_bounty", draft);
          return window.AgentBountiesWebMCP.call("agent_bounties_get_bounty_review");
        }, revised);
        assert.equal(result.recovery_code, null);
        assert.equal(result.explicitly_approved, false, "New deadline cannot borrow the old approval");
        assert.equal(result.saved_operation.operation_id, operation);
        assert.equal(result.delivery_deadline, revised.delivery_deadline);
        assert.equal(await page.evaluate(() => window.AgentBountiesPostingSession.create(window).approved()), false);
        assert.deepEqual(await page.evaluate(() => window.__walletCalls), []);
        output.correction = { same_operation: true, old_approval_invalidated: true, recovery_code: result.recovery_code, wallet_calls: 0 };
        await page.close();
        for (const [name, change, expected] of [
          ["malformed-deadline", { delivery_deadline: "yesterday" }, /timestamp/],
          ["incompatible-benchmark", { benchmark: { engine: "sandboxed_regression_v1" } }, /replace/],
          ["mismatched-saved-policy", { benchmark: { ...f.journey.draft.benchmark, delivery_deadline: f.journey.draft.benchmark.delivery_deadline + 1 } }, /does not match/],
        ]) {
          const invalid = { ...f.journey, draft: { ...f.journey.draft, ...change } };
          const envelope = session.envelope(invalid), hash = createHash("sha256").update(session.stable(envelope)).digest("hex");
          f.row = { ...f.row, draft: envelope, draft_hash: hash, approved_draft_hash: hash };
          const invalidPage = await context.newPage();
          await invalidPage.goto(origin + postPath + "?operation_id=" + operation + "&analytics=off");
          await invalidPage.waitForFunction(() => window.AgentBountiesWebMCP && window.AgentBountiesComposer);
          const failures = await invalidPage.evaluate(async () => {
            const result = {};
            for (const tool of ["agent_bounties_get_bounty_review", "agent_bounties_get_journey"]) {
              try { await window.AgentBountiesWebMCP.call(tool); result[tool] = null; }
              catch (error) { result[tool] = error.message; }
            }
            return result;
          });
          for (const [tool, message] of Object.entries(failures)) assert.match(message, expected, `${postPath} ${name}: ${tool}`);
          assert.deepEqual(await invalidPage.evaluate(() => window.__walletCalls), []);
          (output.invalid_cases ||= []).push({ postPath, name, failures, wallet_calls: 0 });
          if (name === "mismatched-saved-policy") {
            const journal = { phase: "submitted", bounty_contract: "0x" + "11".repeat(20), bounty_id: "0x" + "22".repeat(32), transactions: ["0x" + "33".repeat(32)], authorizationIssued: true };
            const recovery = await invalidPage.evaluate(async journal => {
              const key = "agent-bounties.posting-operation.v1";
              sessionStorage.setItem(key, JSON.stringify(journal));
              return { journey: await window.AgentBountiesWebMCP.call("agent_bounties_get_journey"), review: await window.AgentBountiesWebMCP.call("agent_bounties_get_bounty_review"), journal: JSON.parse(sessionStorage.getItem(key)), wallet_calls: window.__walletCalls };
            }, journal);
            assert.equal(recovery.journey.next_action.tool, "agent_bounties_get_posting_status");
            assert.equal(recovery.review.funding_ready, false);
            assert.match(recovery.review.blocker, /does not match/);
            assert.match(recovery.review.next_action, /reconcile/);
            assert.deepEqual(recovery.journal, journal);
            assert.deepEqual(recovery.wallet_calls, []);
            (output.pending_recovery ||= []).push({ postPath, malformed_policy: true, unchanged_journal: true, next_tool: recovery.journey.next_action.tool, wallet_calls: 0 });
          }
          await invalidPage.close();
        }
      }
      assert.equal(originalHash.length, 64);
      await context.close();
    }
    output.regression_passed = true;
    if (process.env.RECOVERY_TEST_OUTPUT) fs.writeFileSync(process.env.RECOVERY_TEST_OUTPUT, JSON.stringify(output, null, 2) + "\n");
    console.log(JSON.stringify({ regression_passed: true, cases: output.cases.map(c => ({ expired: c.expired, phase: c.phase, review: c.tools.agent_bounties_get_bounty_review.error || c.tools.agent_bounties_get_bounty_review.result?.status, journey: c.tools.agent_bounties_get_journey.error || "readable", wallet_calls: c.wallet_calls.length, fixture_http_methods: c.fixture_http_methods })) }, null, 2));
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
