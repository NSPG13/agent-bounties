import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../site");
const ADDRESS = "0x" + "11".repeat(20);
const EMBEDDED = "0x" + "22".repeat(20);
let browser, server, origin;
before(async () => {
  server = createServer(async (request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
    const filename = path.resolve(root, `.${pathname === "/" ? "/index.html" : pathname}`);
    if (!filename.startsWith(root + path.sep)) { response.writeHead(403).end(); return; }
    try {
      let body = await readFile(filename);
      if (pathname === "/wallet-config.js") body = Buffer.from(body.toString().replace("__COINBASE_CDP_PROJECT_ID__", "qa-wallet-project"));
      response.setHeader("Content-Type", ({ ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".webp": "image/webp" })[path.extname(filename)] || "application/octet-stream");
      response.end(body);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server) await new Promise((resolve) => server.close(resolve));
});

async function account({ installed = true, linked = false, mobile = false } = {}) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 } });
  const page = await context.newPage();
  const proofs = [], errors = [];
  let wallets = linked ? [{ address: ADDRESS }] : [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("https://**/*", (route) => route.fulfill({ json: {} }));
  await page.route("**/auth/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let body = {};
    if (pathname.endsWith("/session")) body = { authenticated: true, user: { id: "qa", name: "Test member", provider: "google" }, providers: { google: true } };
    if (pathname.endsWith("/account")) body = { wallets, available: false };
    if (pathname.endsWith("/wallet/challenge")) {
      const request = route.request().postDataJSON(); proofs.push(request);
      body = { challenge_id: "ownership-only", message: `Ownership proof for ${request.address}. No payment.` };
    }
    if (pathname.endsWith("/wallet/verify")) {
      const request = route.request().postDataJSON(); proofs.push(request);
      wallets = [...wallets, { address: request.address }]; body = { wallets };
    }
    await route.fulfill({ json: body });
  });
  await page.addInitScript(({ installed, address }) => {
    window.walletTestCalls = [];
    if (installed) window.ethereum = { isMetaMask: true, request: async (request) => {
      window.walletTestCalls.push({ wallet: "MetaMask", ...request });
      return request.method === "eth_requestAccounts" ? [address] : "0x" + "ab".repeat(65);
    } };
  }, { installed, address: ADDRESS });
  // Replace only the external SDK boundary; exercise the real chooser and account handler.
  await page.route("**/vendor/coinbase-embedded-wallet.bundle.js?*", (route) => route.fulfill({
    contentType: "text/javascript",
    body: `window.AgentBountiesCoinbaseEmbeddedWallet = { enabled: true, provider: { request: async (request) => {
      window.walletTestCalls.push({ wallet: "embedded", ...request });
      return request.method === "eth_requestAccounts" ? ["${EMBEDDED}"] : "0x" + "cd".repeat(65);
    } } };`,
  }));
  await page.goto(origin);
  await page.getByRole("button", { name: "Account", exact: true }).click();
  const link = page.locator("[data-wallet-link]");
  await page.waitForFunction(() => document.querySelector("[data-wallet-list]").textContent !== "Checking verified wallets…");
  return { context, page, proofs, errors, link };
}

for (const installed of [true, false]) {
  for (const linked of [true, false]) {
    test(`${linked ? "Link another" : "Link wallet"} offers creation with ${installed ? "MetaMask" : "no wallet"} installed`, async () => {
      const { context, page, proofs, errors, link } = await account({ installed, linked, mobile: !installed });
      try {
        assert.equal(await link.innerText(), linked ? "Link another" : "Link wallet");
        await link.click();
        assert.equal(await page.getByRole("dialog", { name: "Choose a wallet" }).isVisible(), true);
        assert.deepEqual(await page.evaluate(() => window.walletTestCalls), []);
        await page.getByRole("button", { name: "I don’t have a wallet — create one", exact: false }).click();
        await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("verified and linked"));
        const calls = await page.evaluate(() => window.walletTestCalls);
        assert.deepEqual(calls.map(({ wallet, method }) => [wallet, method]), [["embedded", "eth_requestAccounts"], ["embedded", "personal_sign"]]);
        assert.equal(calls[1].params[1], EMBEDDED);
        assert.equal(proofs[1].address, EMBEDDED);
        assert.equal(proofs[1].challenge_id, "ownership-only");
        assert.deepEqual(errors, []);
      } finally { await context.close(); }
    });
  }
}

test("Escape returns to the account without a wallet call; MetaMask works only after selection", async () => {
  const { context, page, proofs, link } = await account();
  try {
    await link.click();
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector("[data-wallet-link]").disabled);
    assert.deepEqual(await page.evaluate(() => window.walletTestCalls), []);
    assert.deepEqual(proofs, []);
    assert.equal(await link.evaluate((element) => element === document.activeElement), true);
    await link.click();
    await page.getByRole("button", { name: "MetaMask Choose an address in this wallet." }).click();
    await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("verified and linked"));
    assert.deepEqual((await page.evaluate(() => window.walletTestCalls)).map(({ wallet, method }) => [wallet, method]), [["MetaMask", "eth_requestAccounts"], ["MetaMask", "personal_sign"]]);
    assert.equal(proofs[1].address, ADDRESS);
  } finally { await context.close(); }
});
