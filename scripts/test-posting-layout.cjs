"use strict";
// Real layout/interaction gate. No remote requests, real wallets, or transactions.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");
const { execFileSync } = require("node:child_process");
const { createHash } = require("node:crypto");
let playwright;
try { playwright = require("../tools/browser-layout/node_modules/playwright"); }
catch { playwright = require("playwright"); }
const root = path.resolve(__dirname, "..");
const baseline = process.env.POSTING_LAYOUT_BASELINE;
const mime = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".json": "application/json", ".svg": "image/svg+xml", ".webp": "image/webp" };
const server = http.createServer((req, res) => {
  const relative = decodeURIComponent(new URL(req.url, "http://localhost").pathname).replace(/^\/+/, "") || "index.html";
  const filename = path.resolve(root, "site", relative);
  if (!filename.startsWith(path.join(root, "site") + path.sep)) { res.writeHead(403).end(); return; }
  try {
    const source = baseline ? execFileSync("git", ["show", baseline + ":site/" + relative], { cwd: root, stdio: ["ignore", "pipe", "ignore"] }) : fs.readFileSync(filename);
    res.writeHead(200, { "Content-Type": mime[path.extname(filename)] || "application/octet-stream" }).end(source);
  } catch { res.writeHead(404).end(); }
});
const fixture = {
  title: "An editable product model with assembly instructions",
  goal: "Create an editable concept model and clear assembly drawings that another person can open, measure, and use.",
  acceptance_criteria: [
    "Deliver editable source plus STEP and STL exports that open without errors.",
    "Include dimensioned drawings, an adjustable adapter, and a list of measurements needed before fabrication.",
    "Include assembly instructions and a bill of materials with each part clearly identified."
  ],
  solver_reward_usdc: "18.00", verifier_reward_usdc: "2.00", task_window_days: 7,
  review_mode: "creator", delivery_deadline: new Date(Date.now() + 7 * 86400000).toISOString()
};
const wallet = { address: "0x1111111111111111111111111111111111111111", label: "Test wallet", provider_id: "metamask", wallet_type: "browser", chain_ids: [8453], linked_at: "2026-01-01T00:00:00Z", last_verified_at: "2026-01-01T00:00:00Z" };
function sortedJson(value) {
  if (Array.isArray(value)) return value.map(sortedJson);
  if (value && typeof value === "object") return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortedJson(value[key])]));
  return value;
}
async function fixtures(context, origin, options = {}) {
  const mock = { authenticated: options.authenticated !== false, wallets: options.wallets || [], drafts: options.drafts || new Map(), rpcRequests: [], draftRequests: [], events: [], inventory: [] };
  await context.route("**/*", async route => {
    const request = route.request(), url = new URL(request.url());
    const session = { authenticated: mock.authenticated, account_status: mock.authenticated ? "ready" : "signed_out", account_complete: mock.authenticated, providers: { github: true }, user: mock.authenticated ? { id: "layout-qa", name: "Posting QA", email: "posting-qa@example.test" } : null };
    if (["/auth/session", "/v1/site-auth/session"].includes(url.pathname)) return route.fulfill({ json: session });
    if (["/auth/account", "/v1/site-auth/account"].includes(url.pathname)) return route.fulfill({ json: { ...session, wallets: mock.wallets, data_status: "unavailable", reason: "marketplace_evidence_unavailable" } });
    const operation = /^\/v1\/site-auth\/posting-drafts\/([0-9a-f-]{36})$/i.exec(url.pathname)?.[1];
    if (operation) {
      mock.draftRequests.push({ method: request.method(), operation });
      if (!mock.authenticated) return route.fulfill({ status: 401, json: { message: "Sign in to access this draft." } });
      const existing = mock.drafts.get(operation);
      if (request.method() === "GET") return route.fulfill(existing ? { json: existing } : { status: 404, json: { message: "Draft not found." } });
      assert.equal(request.method(), "POST");
      const body = request.postDataJSON();
      if (body.expected_revision !== (existing?.revision || 0)) return route.fulfill({ status: 409, json: { message: "Draft changed on another device." } });
      assert.equal(body.draft.id, operation);
      assert.equal(body.draft.schema, "agent-bounties/posting-draft-v1");
      const hash = createHash("sha256").update(JSON.stringify(sortedJson(body.draft))).digest("hex");
      assert.ok(!body.approved_draft_hash || body.approved_draft_hash === hash, "Only the exact approved envelope can be stored");
      const saved = { operation_id: operation, draft: body.draft, draft_hash: hash, approved_draft_hash: body.approved_draft_hash, recovery_state: body.recovery_state, revision: (existing?.revision || 0) + 1, updated_at: new Date().toISOString() };
      mock.drafts.set(operation, saved);
      return route.fulfill({ json: saved });
    }
    if (url.pathname === "/v1/base/autonomous-bounties/events") return route.fulfill({ json: mock.events });
    if (url.pathname === "/v1/opportunities") return route.fulfill({ json: { schema_version: "agent-bounties/opportunity-projection-v1", items: mock.inventory } });
    if (url.origin === "https://mainnet.base.org") {
      const call = request.postDataJSON(); mock.rpcRequests.push(call);
      const result = ({ eth_chainId: "0x2105", eth_blockNumber: "0x64", eth_call: "0x5f5e100", eth_getBalance: "0x2386f26fc10000" })[call.method];
      assert.ok(result, "Public balance RPC must remain read only: " + call.method);
      return route.fulfill({ json: { jsonrpc: "2.0", id: call.id, result } });
    }
    if (url.origin !== origin) return route.abort();
    if (url.pathname === "/phone-wallet-config.js") return route.fulfill({ contentType: "text/javascript", body: 'window.agentBountiesPhoneWalletConfig={projectId:"00000000000000000000000000000000"};' });
    // Synthetic pairing only: this provider has no relay, signing or payment.
    if (url.pathname === "/vendor/phone-wallet.bundle.js") return route.fulfill({ contentType: "text/javascript", body: `
      export async function createProvider() {
        const listeners = {};
        return { accounts: [], chainId: 8453, on: (name, fn) => listeners[name] = fn,
          signer: { cleanupPendingPairings: async () => {} },
          connect: () => { listeners.display_uri?.("wc:" + "0".repeat(64) + "@2?relay-protocol=irn&symKey=" + "0".repeat(64)); return new Promise(() => {}); }
        };
      }
      export async function qrDataUrl() { return "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="290" height="290"><rect width="290" height="290" fill="white"/><path d="M20 20h80v80H20zM190 20h80v80h-80zM20 190h80v80H20z" fill="black"/></svg>'); }
    ` });
    return route.continue();
  });
  await context.addInitScript(() => {
    window.__walletWrites = []; window.__walletRequests = [];
    window.ethereum = { isMetaMask: true, request: async ({ method }) => {
      window.__walletRequests.push(method);
      if (method === "eth_requestAccounts" || method === "eth_accounts") return ["0x1111111111111111111111111111111111111111"];
      // No Base ETH in the injected layout wallet keeps the top-up details
      // visible for keyboard/scroll checks; it cannot fund anything.
      const result = ({ eth_chainId: "0x2105", eth_blockNumber: "0x64", eth_call: "0x5f5e100", eth_getBalance: "0x0" })[method];
      if (result) return result;
      window.__walletWrites.push(method); throw new Error("Layout test prohibits wallet writes: " + method);
    } };
  });
  return mock;
}
async function awaitPosting(page) {
  await page.waitForFunction(() => window.AgentBountiesComposer && window.AgentBountiesPostingSession);
}
async function recoveryRegressions(browser, origin) {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, reducedMotion: "reduce" });
  const mock = await fixtures(context, origin, { wallets: [wallet] });
  const page = await context.newPage(), errors = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.goto(origin + "/post.html?from=webmcp&analytics=off");
    await awaitPosting(page);
    await page.evaluate(draft => window.AgentBountiesComposer.stage(draft), fixture);
    // Capture real fixture bytes through the UI. The recorded reference is
    // independent of a generated cover image and survives the server envelope.
    await page.locator(".posting-reference > summary").click();
    await page.getByLabel("Homepage background scene", { exact: true }).selectOption("day");
    await page.getByLabel("Background layout", { exact: true }).selectOption("mobile");
    await page.getByRole("button", { name: "Freeze this background", exact: true }).click();
    await page.waitForFunction(() => window.AgentBountiesWorkflow.createClient(window).load()?.reference_attachment?.phase === "day");
    const reference = await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().reference_attachment);
    assert.equal(reference.sha256, "sha256:" + createHash("sha256").update(fs.readFileSync(path.join(root, "site/assets/solarpunk/scene-day-mobile.webp"))).digest("hex"));
    assert.equal(reference.byte_length, fs.statSync(path.join(root, "site/assets/solarpunk/scene-day-mobile.webp")).size);
    await page.evaluate(draft => window.AgentBountiesComposer.stage(draft), { ...fixture, reference_attachment: reference });
    await page.waitForFunction(() => document.querySelector("[data-approve-card]").dataset.nextAction === "approve");
    await page.locator("[data-approve-card]").click();
    await page.locator("#funding-dialog[open]").waitFor();
    await page.waitForFunction(() => document.querySelector("[data-wallet-state]").textContent.includes("Balance checks only"));
    assert.equal(await page.locator("[data-fund-now]").isDisabled(), true, "Verified ownership must not authorize funding");
    assert.equal(await page.locator('[data-posting-step="wallet"]').getAttribute("data-complete"), "false");
    assert.equal(await page.evaluate(() => window.__walletRequests.includes("eth_requestAccounts")), false, "Linked wallet balances are checked without requesting connection");
    assert.ok(mock.rpcRequests.some(call => call.method === "eth_getBalance"));
    assert.ok(mock.rpcRequests.some(call => call.method === "eth_call"));
    await page.getByRole("button", { name: /Test wallet · Verified ownership/ }).click();
    await page.getByRole("button", { name: "Use this wallet", exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.__walletRequests.includes("eth_requestAccounts")), false, "Selecting verified ownership remains a public balance read");
    await page.locator("[data-close-funding]").click();
    const approved = await page.evaluate(async () => {
      const session = window.AgentBountiesPostingSession.create(window);
      await session.flush({ requireServer: true });
      return { review: window.AgentBountiesComposer.review(), journey: window.AgentBountiesWorkflow.createClient(window).load(), approval: JSON.parse(sessionStorage.getItem("agent-bounties.posting-approval.v1")), continuation: session.snapshot().continuation_url };
    });
    assert.equal(approved.review.explicitly_approved, true);
    assert.ok(approved.approval.hash);
    const exactTarget = page.url();
    const account = page.locator("[data-post-auth-start]");
    assert.equal(await account.textContent(), "Account");
    await Promise.all([page.waitForURL(origin + "/#account"), account.click()]);
    await page.locator("[data-auth-dialog][open]").waitFor();
    await page.getByRole("button", { name: "Return to bounty", exact: true }).waitFor();
    assert.equal(page.url(), origin + "/#account", "Account must not immediately bounce back to posting");
    await Promise.all([page.waitForURL(exactTarget), page.getByRole("button", { name: "Return to bounty", exact: true }).click()]);
    await awaitPosting(page);
    await page.waitForFunction(() => window.AgentBountiesComposer.review().explicitly_approved);
    assert.equal(await page.locator("[data-approve-card]").isVisible(), false, "Unchanged terms must not need another approval");
    const resumed = await page.evaluate(() => ({ draft: window.AgentBountiesWorkflow.createClient(window).load().draft, approval: JSON.parse(sessionStorage.getItem("agent-bounties.posting-approval.v1")), operation: window.AgentBountiesComposer.review().saved_operation.operation_id }));
    assert.equal(resumed.approval.hash, approved.approval.hash);
    assert.equal(resumed.operation, approved.journey.id);
    assert.deepEqual(resumed.draft.reference_attachment, reference);
    assert.deepEqual(resumed.draft.evidence_schema["x-agent-bounties-reference-attachment"], reference);
    assert.deepEqual(await page.evaluate(() => window.__walletWrites), []);

    // A clean second browser uses only the authenticated continuation. No
    // storage copying or injected approval is involved.
    const second = await browser.newContext({ viewport: { width: 390, height: 844 }, reducedMotion: "reduce" });
    try {
      await fixtures(second, origin, { wallets: [wallet], drafts: mock.drafts });
      const resumedPage = await second.newPage();
      resumedPage.on("pageerror", error => errors.push(error.message));
      await resumedPage.goto(approved.continuation);
      await awaitPosting(resumedPage);
      await resumedPage.waitForFunction(() => window.AgentBountiesComposer.review().explicitly_approved);
      const remote = await resumedPage.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load());
      assert.deepEqual(remote.draft.reference_attachment, reference);
      assert.deepEqual(remote.draft.evidence_schema["x-agent-bounties-reference-attachment"], reference);
      assert.equal(remote.id, approved.journey.id);
      assert.deepEqual(await resumedPage.evaluate(() => window.__walletRequests), [], "Continuing an approved draft must not reconnect or sign automatically");
    } finally { await second.close(); }

    // A transaction hash alone must never mark creation/funding/claimability
    // complete, including after navigation and repeated reconciliation.
    const pending = await page.evaluate(async () => {
      const session = window.AgentBountiesPostingSession.create(window);
      await session.refresh();
      const journal = window.AgentBountiesWorkflow.createPostingJournal(window);
      journal.prepare({ predicted_bounty_contract: "0x2222222222222222222222222222222222222222", bounty_id: "0x" + "3".repeat(64) });
      journal.checkpoint("submitted", "0x" + "4".repeat(64));
      await session.flush({ requireServer: true });
      return [await session.reconcile(), await session.reconcile()];
    });
    for (const result of pending) {
      assert.equal(result.creation_confirmed, false); assert.equal(result.funding_confirmed, false);
      assert.equal(result.claimable, false); assert.equal(result.public_inventory_verified, false); assert.equal(result.paid, false);
    }
    for (const step of ["creation", "funding", "claimable", "inventory"]) assert.equal(await page.locator(`[data-posting-step="${step}"]`).getAttribute("data-complete"), "false");
    await page.reload(); await awaitPosting(page);
    await page.waitForFunction(() => window.AgentBountiesComposer.review().canonical_status !== null);
    assert.equal(await page.locator("[data-open-funding]").isDisabled(), true, "Pending operation cannot create a second funding request");
    assert.deepEqual(await page.evaluate(() => window.AgentBountiesComposer.review().posting_operation.transactions), ["0x" + "4".repeat(64)]);
    assert.deepEqual(await page.evaluate(() => window.__walletWrites), []);
    assert.deepEqual(errors, [], "Recovery flow has no browser runtime errors");
    console.log("PASS account-return approval, exact immutable reference, private continuation, linked-wallet public balances, and pending canonical recovery");
  } finally { await context.close(); }

  const partialContext = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const partialMock = await fixtures(partialContext, origin);
  try {
    const partialPage = await partialContext.newPage();
    await partialPage.goto(origin + "/post.html"); await awaitPosting(partialPage);
    await partialPage.locator("#bounty-composer-input").fill("A short video of the homepage background");
    await partialPage.locator("[data-brief-budget]").fill("5");
    await partialPage.waitForFunction(() => window.AgentBountiesPostingSession.create(window).snapshot().status === "saved");
    const continuation = await partialPage.evaluate(async () => { const session = window.AgentBountiesPostingSession.create(window); await session.flush({ requireServer: true }); return session.snapshot().continuation_url; });
    const next = await browser.newContext({ viewport: { width: 390, height: 844 } });
    try {
      await fixtures(next, origin, { drafts: partialMock.drafts });
      const p = await next.newPage(); await p.goto(continuation); await awaitPosting(p);
      await p.waitForFunction(() => window.AgentBountiesPostingSession.create(window).snapshot().status === "saved");
      assert.equal(await p.locator("#bounty-composer-input").inputValue(), "A short video of the homepage background");
      assert.equal(await p.locator("[data-brief-budget]").inputValue(), "5");
      const review = await p.evaluate(() => window.AgentBountiesComposer.review());
      assert.equal(review.explicitly_approved, false); assert.equal(review.status, "no_staged_bounty");
      assert.equal(await p.evaluate(() => sessionStorage.getItem("agent-bounties.posting-approval.v1")), null);
      assert.deepEqual(await p.evaluate(() => window.__walletRequests), []);
    } finally { await next.close(); }
    console.log("PASS incomplete brief resumes across devices without inventing a draft or approving terms");
  } finally { await partialContext.close(); }
}
async function hitTarget(page, selector, minimumHeight = 43) {
  const result = await page.locator(selector).evaluate((el, minimumHeight) => {
    const r = el.getBoundingClientRect(), x = r.left + r.width / 2, y = r.top + r.height / 2;
    const hit = document.elementFromPoint(x, y);
    return { text: el.textContent.trim(), visible: r.width > 0 && r.height >= minimumHeight && r.left >= 0 && r.right <= innerWidth + 1 && r.top >= 0 && r.bottom <= innerHeight + 1, hit: hit === el || el.contains(hit), rect: r.toJSON() };
  }, minimumHeight);
  assert.ok(result.visible && result.hit, selector + " clipped or covered: " + JSON.stringify(result));
}
async function noHorizontalOverflow(page) {
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, "Page must reflow without horizontal overflow");
}
async function wheelToEnd(page, selector = null) {
  if (selector) {
    const r = await page.locator(selector).boundingBox();
    await page.mouse.move(r.x + r.width / 2, r.y + r.height * .7);
  } else {
    const v = page.viewportSize(); await page.mouse.move(v.width / 2, v.height / 2);
  }
  await page.mouse.wheel(0, 20000);
  await page.waitForFunction(sel => {
    const el = sel ? document.querySelector(sel) : document.scrollingElement;
    return el.scrollHeight - el.clientHeight - el.scrollTop <= 2;
  }, selector);
}
async function modalBounds(page, selector) {
  const value = await page.locator(selector).evaluate(el => {
    const r = el.getBoundingClientRect();
    return { bounded: r.top >= 0 && r.bottom <= innerHeight + 1 && r.left >= 0 && r.right <= innerWidth + 1, noHorizontal: el.scrollWidth <= el.clientWidth + 1, rect: r.toJSON(), viewport: [innerWidth, innerHeight] };
  });
  assert.ok(value.bounded && value.noHorizontal, selector + " exceeds the viewport: " + JSON.stringify(value));
}
async function main() {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = "http://127.0.0.1:" + server.address().port;
  const browser = await playwright.chromium.launch({
    headless: true,
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined,
  });
  const sizes = [
    { width: 1440, height: 900 }, { width: 946, height: 838 },
    { width: 640, height: 480 }, { width: 390, height: 600 },
    { width: 320, height: 480 }, { width: 768, height: 320 },
    // A 960x720 browser at 200% zoom has this 480x360 CSS viewport.
    { width: 480, height: 360, zoomReflow: true }
  ];
  try {
    for (const size of sizes) {
      const context = await browser.newContext({ viewport: { width: size.width, height: size.height }, reducedMotion: "reduce" });
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      await fixtures(context, origin);
      await page.goto(origin + "/post.html");
      await page.waitForFunction(() => window.AgentBountiesComposer);
      await page.evaluate(draft => window.AgentBountiesComposer.stage(draft), fixture);
      await page.locator("[data-approve-card]").waitFor({ state: "visible" });
      await noHorizontalOverflow(page);
      assert.notEqual(await page.evaluate(() => getComputedStyle(document.body).overflowY), "hidden", "Retired chat CSS must not lock the page");
      // Real wheel input, not scrollIntoView, catches the original silent scroll trap.
      await page.evaluate(() => scrollTo(0, 0));
      await page.mouse.move(size.width / 2, size.height / 2);
      await page.mouse.wheel(0, 400);
      await page.waitForFunction(() => scrollY > 20);
      await hitTarget(page, "[data-revise-card]");
      await hitTarget(page, "[data-approve-card]");
      if (size.width === 1440) {
        await page.setViewportSize({ width: 390, height: 320 });
        await hitTarget(page, "[data-approve-card]");
        await noHorizontalOverflow(page);
        await page.setViewportSize(size);
      }
      await wheelToEnd(page);
      const footer = await page.locator("[data-ai-options] > summary").boundingBox();
      const dock = await page.locator(".bounty-card-actions").boundingBox();
      assert.ok(footer.y + footer.height < dock.y, "Bottom content must clear the action dock");
      if (process.env.POSTING_LAYOUT_SCREENSHOTS) {
        fs.mkdirSync(process.env.POSTING_LAYOUT_SCREENSHOTS, { recursive: true });
        await page.locator("#bounty-preview").scrollIntoViewIfNeeded();
        await page.screenshot({ path: path.join(process.env.POSTING_LAYOUT_SCREENSHOTS, "proposal-" + size.width + "x" + size.height + ".png") });
      }
      // Only the isolated test fixture is approved. No real site or wallet is touched.
      await page.locator("[data-approve-card]").click();
      await page.locator(".wallet-option").first().waitFor();
      await modalBounds(page, "#funding-dialog");
      await hitTarget(page, "[data-close-funding]");
      await wheelToEnd(page, "#funding-dialog");
      await page.locator("[data-walletless-onramp-link]").focus();
      assert.equal(await page.locator("[data-walletless-onramp-link]").evaluate(el => {
        const r = el.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight;
      }), true, "Wallet setup link must be keyboard reachable");
      await page.locator("[data-close-funding]").click();
      await hitTarget(page, "[data-open-funding]");
      await page.locator("[data-open-funding]").click();
      await page.locator(".wallet-option").first().waitFor();
      if (process.env.POSTING_LAYOUT_SCREENSHOTS) await page.screenshot({ path: path.join(process.env.POSTING_LAYOUT_SCREENSHOTS, "wallet-" + size.width + "x" + size.height + ".png") });
      await page.getByRole("button", { name: "MetaMask", exact: false }).click();
      await page.locator("[data-wallet-readiness]").waitFor({ state: "visible" });
      assert.equal(await page.locator("#funding-dialog .legal-consent").evaluate(el => getComputedStyle(el).backgroundColor), "rgb(16, 30, 22)");
      assert.equal(await page.locator("#funding-dialog .legal-consent h3").evaluate(el => getComputedStyle(el).color), "rgb(237, 241, 232)");
      if ([1440,390].includes(size.width)) {
        await page.locator("[data-close-funding]").click();
        await page.locator('button[data-theme-choice="light"]').scrollIntoViewIfNeeded();
        await hitTarget(page, 'button[data-theme-choice="light"]', 32);
        await page.locator('button[data-theme-choice="light"]').click();
        await page.locator("[data-open-funding]").click();
        await page.locator("[data-wallet-readiness]").waitFor({ state: "visible" });
        assert.equal(await page.locator("#funding-dialog .legal-consent").evaluate(el => getComputedStyle(el).backgroundColor), "rgb(255, 255, 255)");
        assert.equal(await page.locator("#funding-dialog .legal-consent h3").evaluate(el => getComputedStyle(el).color), "rgb(21, 40, 28)");
        if (process.env.POSTING_LAYOUT_SCREENSHOTS) await page.screenshot({ path: path.join(process.env.POSTING_LAYOUT_SCREENSHOTS, "funding-light-" + size.width + ".png") });
        await page.locator("[data-close-funding]").click();
        await page.locator('button[data-theme-choice="dark"]').click();
        await page.locator("[data-open-funding]").click();
        await page.locator("[data-wallet-readiness]").waitFor({ state: "visible" });
      }
      await page.locator(".funding-help > summary").click();
      if (size.width === 1440) {
        await page.setViewportSize({ width: 390, height: 320 });
        await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
        await modalBounds(page, "#funding-dialog");
        await hitTarget(page, "[data-close-funding]");
        await page.setViewportSize(size);
      }
      await modalBounds(page, "#funding-dialog");
      await wheelToEnd(page, "#funding-dialog");
      await hitTarget(page, "[data-close-funding]");
      await hitTarget(page, "[data-fund-now]");
      await page.locator("[data-close-funding]").click();
      assert.equal(await page.locator(".ab-phone-launcher").count(), 0);
      await page.locator("[data-open-funding]").click();
      await page.getByRole("button", { name: "Choose or recover another wallet", exact: true }).click();
      await page.getByRole("button", { name: "Use a phone wallet", exact: false }).click();
      await page.locator(".ab-phone-qr").waitFor({ state: "visible" });
      await modalBounds(page, ".ab-phone-dialog");
      await wheelToEnd(page, ".ab-phone-dialog");
      await hitTarget(page, ".ab-phone-close");
      await page.locator(".ab-phone-close").click();
      await page.locator("[data-close-funding]").click();
      await page.keyboard.press("Tab");
      await page.locator("[data-revise-card]").click();
      assert.equal(await page.locator("#bounty-composer-input").inputValue(), fixture.goal);
      assert.equal(await page.locator("#bounty-composer-input").evaluate(el => document.activeElement === el), true);
      await noHorizontalOverflow(page);
      assert.deepEqual(await page.evaluate(() => window.__walletWrites), []);
      assert.deepEqual(errors, [], "No browser runtime errors");
      console.log("PASS posting, wallet review, QR, keyboard and scrolling at " + size.width + "x" + size.height + (size.zoomReflow ? " (200% browser-zoom reflow equivalent)" : ""));
      await context.close();
    }
    {
      const context = await browser.newContext({ viewport: { width: 390, height: 844 }, reducedMotion: "reduce" });
      const page = await context.newPage();
      await context.route("**/*", async route => {
        const url = new URL(route.request().url());
        if (url.pathname === "/auth/session") return route.fulfill({ json: { authenticated: false, account_status: "signed_out", account_complete: false, providers: { github: true } } });
        if (url.origin !== origin) return route.abort();
        if (url.pathname === "/phone-wallet-config.js") return route.fulfill({ contentType: "text/javascript", body: "window.agentBountiesPhoneWalletConfig={};" });
        return route.continue();
      });
      await page.addInitScript(() => {
        window.__walletWrites = [];
        window.ethereum = { request: async request => { window.__walletWrites.push(request.method); throw new Error("No wallet request is allowed during login."); } };
      });
      await page.goto(`${origin}/post.html?from=ai-app&title=Preserve+this+exact+draft&testMarker=opaque`);
      await page.waitForFunction(() => window.AgentBountiesComposer);
      await page.evaluate(draft => window.AgentBountiesComposer.stage(draft), fixture);
      const login = page.getByRole("button", { name: "LOG IN TO POST", exact: true });
      await login.waitFor();
      assert.equal(await login.isEnabled(), true);
      const exactTarget = page.url();
      await Promise.all([
        page.waitForURL(`${origin}/?postReturn=1#login`, { waitUntil: "domcontentloaded" }),
        login.click(),
      ]);
      await page.locator("[data-auth-dialog][open]").waitFor();
      assert.equal(await page.evaluate(() => JSON.parse(sessionStorage.getItem("agentbounties:post-auth-return:v1")).target), exactTarget);
      assert.deepEqual(await page.evaluate(() => window.__walletWrites), []);
      console.log("PASS signed-out posting action opens login with the exact draft preserved and no wallet request");
      await context.close();
    }
    await recoveryRegressions(browser, origin);
  } finally { await browser.close(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => server.close());
