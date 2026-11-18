"use strict";
// Every visible button on the public site uses a page style, never the browser's default grey
// look, in both themes. Fixtures only: no real accounts, wallets or payments, and no network
// beyond the local static server.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".webp": "image/webp", ".woff2": "font/woff2", ".png": "image/png" };
// account.html is the isolated card payment page with its own stylesheet; it is included too.
const pages = fs.readdirSync(site, { recursive: true }).filter(file => file.endsWith(".html") && !file.startsWith("workspace")).map(file => file.replaceAll("\\", "/")).sort();
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://localhost");
  let file = path.resolve(site, "." + decodeURIComponent(url.pathname));
  if (file !== site && !file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try {
    if (fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
    res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(fs.readFileSync(file));
  } catch { res.writeHead(404).end(); }
});

// Buttons whose background and border still equal the browser's own button in the same color scheme.
function defaultLookingButtons() {
  const reference = scheme => {
    const host = document.createElement("iframe");
    host.style.cssText = "position:fixed;left:-9999px;width:200px;height:80px";
    document.body.append(host);
    const doc = host.contentDocument;
    doc.open(); doc.write(`<!doctype html><html style="color-scheme:${scheme}"><body><button>x</button></body></html>`); doc.close();
    const style = host.contentWindow.getComputedStyle(doc.querySelector("button"));
    const value = { background: style.backgroundColor, image: style.backgroundImage, border: `${style.borderTopStyle} ${style.borderTopColor}` };
    host.remove();
    return value;
  };
  const cache = new Map();
  const visible = el => {
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none" && !el.closest("[hidden], [inert]");
  };
  return [...document.querySelectorAll('button, input[type="button"], input[type="submit"], input[type="reset"]')].filter(visible).flatMap(el => {
    const scheme = getComputedStyle(el).colorScheme.includes("dark") ? "dark" : "light";
    if (!cache.has(scheme)) cache.set(scheme, reference(scheme));
    const ref = cache.get(scheme), s = getComputedStyle(el);
    const same = s.backgroundColor === ref.background && s.backgroundImage === ref.image && `${s.borderTopStyle} ${s.borderTopColor}` === ref.border;
    return same ? [`${el.tagName.toLowerCase()}${el.className ? "." + String(el.className).trim().replace(/\s+/g, ".") : ""} "${(el.textContent || el.value || el.getAttribute("aria-label") || "").trim().slice(0, 40)}"`] : [];
  });
}

// The signed-in account panel only renders with a session, so it gets synthetic records:
// a deletable saved draft (with its confirm step open), a linked wallet and email settings.
async function signedInAccount(browser, origin, theme, width) {
  const context = await browser.newContext({ viewport: { width, height: 900 }, serviceWorkers: "block" });
  await context.addInitScript(value => { try { localStorage.setItem("agent-bounties.theme", value); } catch (_) {} }, theme);
  const operation = "28000000-0000-4000-8000-000000000001";
  const session = { authenticated: true, account_status: "ready", account_complete: true, posting_drafts_enabled: true, providers: { google: true }, user: { id: "cd".repeat(32), name: "Button Test", email: "", provider: "google" } };
  const draft = { id: `draft:${operation}`, operation_id: operation, title: "Button style draft", group: "drafts", status: "Saved draft", next_actor: "you", next_action: "Continue reviewing this saved draft.", continuation_url: `${origin}/post.html?operation_id=${operation}#bounty-preview`, revision: 2, can_delete: true, updated_at: "2026-11-18T10:00:00Z" };
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (["/auth/session", "/v1/site-auth/session"].includes(url.pathname)) return route.fulfill({ json: session });
    if (["/auth/account", "/v1/site-auth/account"].includes(url.pathname)) return route.fulfill({ json: { ...session, data_status: "available", wallets: [{ address: "0x00000000000000000000000000000000000000b1", chain_id: 8453 }], saved_drafts: { status: "available", items: [draft, ...[2, 3, 4].map(n => ({ ...draft, id: `draft:${n}`, operation_id: `28000000-0000-4000-8000-00000000000${n}`, title: `Older draft ${n}` }))], next_offset: null } } });
    if (url.pathname.endsWith("/review-notifications")) return route.fulfill({ json: { enabled: false, email_verified: false, wallet_linked: true, status: "disabled" } });
    if (url.pathname === "/v1/opportunities") return route.fulfill({ json: { items: [], generated_at: "2026-11-18T10:00:00Z" } });
    if (url.origin !== origin) return route.fulfill({ status: 503, body: "" });
    return route.continue();
  });
  const page = await context.newPage();
  await page.goto(`${origin}/?analytics=off#account`, { waitUntil: "load" });
  const panel = page.locator("[data-account-inbox]");
  await panel.getByRole("link", { name: "Button style draft" }).waitFor();
  const hits = await page.evaluate(defaultLookingButtons);
  await panel.locator("[data-draft-delete]").first().click();
  hits.push(...(await page.evaluate(defaultLookingButtons)).filter(hit => !hits.includes(hit)));
  await context.close();
  return hits;
}

async function main() {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined });
  const failures = [];
  try {
    for (const theme of ["dark", "light"]) {
      for (const width of [390, 1440]) {
        const context = await browser.newContext({ viewport: { width, height: 900 }, serviceWorkers: "block" });
        await context.addInitScript(value => { try { localStorage.setItem("agent-bounties.theme", value); } catch (_) {} }, theme);
        await context.route("**/*", route => route.request().url().startsWith(origin) ? route.continue() : route.fulfill({ status: 503, body: "" }));
        const page = await context.newPage();
        for (const file of pages) {
          await page.goto(`${origin}/${file}`, { waitUntil: "load" });
          await page.waitForTimeout(150);
          for (const hit of await page.evaluate(defaultLookingButtons)) failures.push(`${theme} ${width}px ${file}: ${hit}`);
        }
        await context.close();
        for (const hit of await signedInAccount(browser, origin, theme, width)) failures.push(`${theme} ${width}px account panel: ${hit}`);
      }
    }
  } finally { await browser.close(); server.close(); }
  assert.deepEqual(failures, [], `browser-default buttons:\n${failures.join("\n")}`);
  console.log(`PASS no browser-default buttons on ${pages.length} pages, light and dark, 390px and 1440px`);
}
main().catch(error => { console.error(error.message || error); process.exitCode = 1; });
