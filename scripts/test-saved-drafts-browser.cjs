"use strict";
// Saved drafts on post.html: a signed-in person sees their account drafts, opens one by
// its first-party link, and deletes only drafts the server marks deletable.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site"), shots = process.env.SAVED_DRAFTS_SHOTS || "";
const A = "12345678-1234-4234-8234-123456789abc", B = "22345678-1234-4234-8234-123456789abc";
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  const file = path.resolve(site, "." + (pathname === "/" ? "/index.html" : pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { const body = fs.readFileSync(file); res.writeHead(200, { "Content-Type": ({ ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".woff2": "font/woff2" })[path.extname(file)] || "application/octet-stream" }).end(body); }
  catch (_) { res.writeHead(404).end(); }
});
const drafts = (origin) => ({ schema_version: "agent-bounties/account-drafts-v1", status: "available", next_offset: null, items: [
  { id: `draft:${A}`, operation_id: A, title: "Research three competitors", group: "drafts", status: "Saved draft", next_action: "Continue reviewing this saved draft.",
    continuation_url: `${origin}/post.html?operation_id=${A}#bounty-preview`, updated_at: new Date(Date.now() - 3600e3).toISOString(), revision: 2, can_delete: true },
  { id: `draft:${B}`, operation_id: B, title: "Translate the onboarding guide into Spanish", group: "needs_action", status: "Check posting status", next_action: "Resume this operation.",
    continuation_url: `${origin}/post.html?operation_id=${B}#bounty-preview`, updated_at: new Date(Date.now() - 86400e3).toISOString(), revision: 5, can_delete: false },
] });

(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  async function open({ width, theme = "light", signedIn = true, list = "ok" }) {
    const context = await browser.newContext({ viewport: { width, height: 900 }, deviceScaleFactor: shots ? 2 : 1, reducedMotion: "reduce" });
    await context.addInitScript((value) => { try { localStorage.setItem("agent-bounties.theme", value); } catch (_) {} }, theme);
    const calls = [];
    await context.route("**/*", (route) => {
      const request = route.request(), url = new URL(request.url());
      if (url.origin === origin && url.pathname === "/auth/session") return route.fulfill({ json: signedIn
        ? { authenticated: true, user: { id: "owner-a" }, account_status: "ready", account_complete: true, posting_drafts_enabled: true }
        : { authenticated: false, providers: { google: true } } });
      if (url.origin === origin) return route.continue();
      if (url.origin === "http://127.0.0.1:3000" && url.pathname.startsWith("/v1/site-auth/posting-drafts")) {
        calls.push({ method: request.method(), path: url.pathname, body: request.postData() });
        if (!signedIn) return route.fulfill({ status: 401, json: { error: "authentication_required" } });
        if (request.method() === "POST" && url.pathname === `/v1/site-auth/posting-drafts/${A}/delete`) return route.fulfill({ json: { deleted: true } });
        if (url.pathname === "/v1/site-auth/posting-drafts") return list === "ok" ? route.fulfill({ json: drafts(origin) }) : route.fulfill({ status: 503, json: { status: "unavailable", items: [] } });
        return route.fulfill({ status: 404, json: { error: "draft_not_found" } });
      }
      return route.fulfill({ status: 503, body: "Isolated browser test" });
    });
    const page = await context.newPage(), errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(`${origin}/post.html`);
    return { context, page, calls, errors };
  }
  try {
    for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
      const { context, page, calls, errors } = await open({ width, theme });
      const panel = page.locator("[data-saved-drafts]");
      await panel.locator(".saved-draft").first().waitFor();
      assert.equal(await panel.evaluate((node) => node.open), true, "with no draft in this tab the list starts open");
      assert.equal(await panel.locator("[data-saved-drafts-count]").innerText(), "2");
      assert.equal(await panel.getByRole("link", { name: "Research three competitors" }).getAttribute("href"), `${origin}/post.html?operation_id=${A}#bounty-preview`);
      assert.equal(await panel.getByRole("link", { name: /Translate the onboarding guide/ }).getAttribute("href"), `${origin}/post.html?operation_id=${B}#bounty-preview`);
      assert.equal(await panel.locator("[data-saved-draft-delete]").count(), 1, "only the server-deletable draft offers Delete");
      assert.equal(await panel.locator(`[data-operation-id="${B}"] [data-saved-draft-delete]`).count(), 0);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, "no horizontal scroll");
      if (shots) {
        await page.evaluate(() => document.fonts.ready); await panel.scrollIntoViewIfNeeded();
        await page.screenshot({ path: path.join(shots, `saved-drafts-${width}-${theme}.png`) });
      }
      if (width === 1440 && theme === "light") {
        await panel.getByRole("button", { name: "Delete draft “Research three competitors”" }).click();
        await panel.getByRole("button", { name: "Delete draft", exact: true }).click();
        await page.waitForFunction(() => /Deleted/.test(document.querySelector("[data-saved-drafts-status]").textContent));
        assert.deepEqual(calls.filter((call) => call.method === "POST").map((call) => [call.path, JSON.parse(call.body)]), [[`/v1/site-auth/posting-drafts/${A}/delete`, { expected_revision: 2 }]]);
        assert.equal(await panel.locator(".saved-draft").count(), 1); assert.equal(await panel.locator("[data-saved-drafts-count]").innerText(), "1");
        // Opening a saved draft continues that exact operation on this page.
        await Promise.all([page.waitForURL(`**/post.html?operation_id=${B}#bounty-preview`), panel.getByRole("link", { name: /Translate the onboarding guide/ }).click()]);
        await panel.locator(".saved-draft").first().waitFor({ state: "attached" });
        assert.equal(await panel.evaluate((node) => node.open), false, "with a draft open here, the form stays first and the list starts collapsed");
        await panel.locator("summary").click();
        assert.equal(await panel.locator(`[data-operation-id="${B}"] .saved-draft-chip`).innerText(), "Open now");
        assert.equal(await panel.locator(`[data-operation-id="${B}"] [data-saved-draft-delete]`).count(), 0, "the open draft is never deleted from the list");
      }
      assert.deepEqual(errors, []);
      await context.close();
    }
    {
      const { context, page, calls } = await open({ width: 1440, signedIn: false });
      await page.waitForFunction(() => document.querySelector("[data-saved-drafts]") && true);
      await page.waitForTimeout(500);
      assert.ok(calls.length >= 1, "the list was requested");
      assert.equal(await page.locator("[data-saved-drafts]").isVisible(), false, "signed out: no saved-drafts list");
      await context.close();
    }
    {
      const { context, page } = await open({ width: 390, list: "down" });
      await page.waitForFunction(() => /temporarily unavailable/.test(document.querySelector("[data-saved-drafts-status]")?.textContent || ""));
      assert.equal(await page.locator("[data-saved-drafts]").isVisible(), true, "an unavailable list is said, not hidden as zero drafts");
      await context.close();
    }
    console.log("Saved drafts: listed, opened and deleted on post.html at 1440px and 390px, light and dark");
  } finally { await browser.close(); server.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
