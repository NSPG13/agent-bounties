"use strict";
// Real layout/interaction gate. No remote requests, real wallets, or transactions.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");
const { execFileSync } = require("node:child_process");
let playwright;
try { playwright = require("../tools/browser-layout/node_modules/playwright"); }
catch { playwright = require("playwright"); }
const root = path.resolve(__dirname, "..");
const baseline = process.env.POSTING_LAYOUT_BASELINE;
const mime = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".json": "application/json", ".svg": "image/svg+xml" };
const server = http.createServer((req, res) => {
  const relative = decodeURIComponent(new URL(req.url, "http://localhost").pathname).replace(/^\/+/, "");
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
async function hitTarget(page, selector) {
  const result = await page.locator(selector).evaluate(el => {
    const r = el.getBoundingClientRect(), x = r.left + r.width / 2, y = r.top + r.height / 2;
    const hit = document.elementFromPoint(x, y);
    return { text: el.textContent.trim(), visible: r.width > 0 && r.height >= 43 && r.left >= 0 && r.right <= innerWidth + 1 && r.top >= 0 && r.bottom <= innerHeight + 1, hit: hit === el || el.contains(hit), rect: r.toJSON() };
  });
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
  const browser = await playwright.chromium.launch({ headless: true });
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
      await context.route("**/*", async route => {
        const url = new URL(route.request().url());
        if (url.origin !== origin) return route.abort();
        if (url.pathname === "/phone-wallet-config.js") return route.fulfill({ contentType: "text/javascript", body: 'window.agentBountiesPhoneWalletConfig={projectId:"00000000000000000000000000000000"};' });
        // Inert provider renders a synthetic QR image, with no relay connection.
        if (url.pathname === "/vendor/phone-wallet.bundle.js") return route.fulfill({ contentType: "text/javascript", body: `
          export async function createProvider() {
            const listeners = {};
            return { accounts: [], chainId: 8453, on: (name, fn) => listeners[name] = fn,
              signer: { cleanupPendingPairings: async () => {} },
              connect: () => { listeners.display_uri?.("wc:" + "0".repeat(64) + "@2?symKey=" + "0".repeat(64)); return new Promise(() => {}); }
            };
          }
          export async function qrDataUrl() { return "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="290" height="290"><rect width="290" height="290" fill="white"/><path d="M20 20h80v80H20zM190 20h80v80h-80zM20 190h80v80H20z" fill="black"/></svg>'); }
        ` });
        return route.continue();
      });
      await page.addInitScript(() => {
        window.__walletWrites = [];
        window.ethereum = { isMetaMask: true, request: async ({ method }) => {
          if (method === "eth_requestAccounts" || method === "eth_accounts") return ["0x1111111111111111111111111111111111111111"];
          if (method === "eth_chainId") return "0x2105";
          if (method === "eth_call") return "0x5f5e100";
          if (method === "eth_getBalance") return "0x2386f26fc10000";
          window.__walletWrites.push(method); throw new Error("Layout test prohibits wallet writes: " + method);
        } };
      });
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
      await page.locator(".ab-phone-launcher").click();
      await page.locator(".ab-phone-qr").waitFor({ state: "visible" });
      await modalBounds(page, ".ab-phone-dialog");
      await wheelToEnd(page, ".ab-phone-dialog");
      await hitTarget(page, ".ab-phone-close");
      await page.locator(".ab-phone-close").click();
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
  } finally { await browser.close(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => server.close());
