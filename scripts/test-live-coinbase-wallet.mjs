#!/usr/bin/env node
"use strict";

import { execFileSync, spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";

const DEFAULT_URL = "https://agentbounties.app/earn.html";
const DEFAULT_TIMEOUT_MS = 60_000;

function parseArgs(argv) {
  const args = { url: DEFAULT_URL, timeoutMs: DEFAULT_TIMEOUT_MS, output: null };
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === "--url") args.url = argv[++index];
    else if (value === "--timeout-ms") args.timeoutMs = Number(argv[++index]);
    else if (value === "--output") args.output = argv[++index];
    else throw new Error(`Unknown argument: ${value}`);
  }
  const parsed = new URL(args.url);
  if (parsed.protocol !== "https:" || parsed.hostname !== "agentbounties.app") {
    throw new Error("The live Coinbase smoke URL must use https://agentbounties.app.");
  }
  if (!Number.isSafeInteger(args.timeoutMs) || args.timeoutMs < 5_000 || args.timeoutMs > 180_000) {
    throw new Error("--timeout-ms must be an integer from 5000 to 180000.");
  }
  return args;
}

function findChrome() {
  const candidates = [
    process.env.CHROME_BIN,
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      return execFileSync("which", [candidate], { encoding: "utf8" }).trim();
    } catch (_error) {
      // Continue to the next candidate.
    }
  }
  throw new Error("A Chromium-compatible browser was not found.");
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function poll(callback, timeoutMs, description, intervalMs = 250) {
  const started = Date.now();
  let lastError = null;
  while (Date.now() - started < timeoutMs) {
    try {
      const value = await callback();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await delay(intervalMs);
  }
  const suffix = lastError ? ` Last error: ${lastError.message || lastError}` : "";
  throw new Error(`Timed out waiting for ${description}.${suffix}`);
}

async function jsonFetch(url, options = {}) {
  const response = await fetch(url, options);
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}.`);
  return response.json();
}

class CdpConnection {
  constructor(webSocketUrl) {
    this.socket = new WebSocket(webSocketUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.events = [];
    this.opened = new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", () => reject(new Error("Chrome DevTools WebSocket failed.")), { once: true });
    });
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(`${pending.method}: ${message.error.message}`));
        else pending.resolve(message.result);
        return;
      }
      if (message.method === "Runtime.exceptionThrown" || message.method === "Log.entryAdded") {
        this.events.push(message);
      }
    });
  }

  async command(method, params = {}) {
    await this.opened;
    const id = this.nextId++;
    const promise = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method });
    });
    this.socket.send(JSON.stringify({ id, method, params }));
    return promise;
  }

  async evaluate(expression, { awaitPromise = false } = {}) {
    const response = await this.command("Runtime.evaluate", {
      expression,
      awaitPromise,
      returnByValue: true,
      userGesture: true,
    });
    if (response.exceptionDetails) {
      const description = response.result?.description || response.exceptionDetails.text || "Browser evaluation failed.";
      throw new Error(description);
    }
    return response.result?.value;
  }

  close() {
    try {
      this.socket.close();
    } catch (_error) {
      // Browser teardown is best effort.
    }
  }
}

async function runSmoke(args) {
  const chromePath = findChrome();
  const profile = await mkdtemp(path.join(tmpdir(), "agent-bounties-coinbase-smoke-"));
  const port = 9_300 + Math.floor(Math.random() * 500);
  const chrome = spawn(chromePath, [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--disable-extensions",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-allow-origins=*",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  let stderr = "";
  chrome.stderr.setEncoding("utf8");
  chrome.stderr.on("data", (chunk) => {
    stderr = `${stderr}${chunk}`.slice(-8_000);
  });

  let cdp = null;
  try {
    const version = await poll(
      () => jsonFetch(`http://127.0.0.1:${port}/json/version`).catch(() => null),
      15_000,
      "Chrome DevTools",
    );
    if (!version.webSocketDebuggerUrl) throw new Error("Chrome did not expose a DevTools WebSocket URL.");
    const target = await jsonFetch(
      `http://127.0.0.1:${port}/json/new?${encodeURIComponent(args.url)}`,
      { method: "PUT" },
    );
    cdp = new CdpConnection(target.webSocketDebuggerUrl);
    await cdp.command("Page.enable");
    await cdp.command("Runtime.enable");
    await cdp.command("Log.enable");

    await poll(
      () => cdp.evaluate("document.readyState === 'complete'"),
      args.timeoutMs,
      "the live page to load",
    );

    const adapter = await poll(
      () => cdp.evaluate(`(() => {
        const registry = window.AgentBountiesWalletAdapters;
        const entry = registry?.get?.("coinbase-embedded");
        const wallet = window.AgentBountiesCoinbaseEmbeddedWallet;
        const config = window.AgentBountiesWalletConfig?.providers?.coinbaseEmbedded;
        if (!entry || !wallet?.enabled || !config?.enabled) return null;
        return {
          adapterId: entry.id,
          providerName: entry.info?.name || null,
          embedded: entry.capabilities?.embedded === true,
          userCustody: entry.capabilities?.custody === "user",
          authMethodLinking: entry.capabilities?.authMethodLinking === true,
          eip3009: entry.capabilities?.eip3009 === true,
          gasSponsoredOnSupportedRelays: entry.capabilities?.gasSponsoredOnSupportedRelays === true,
          directTransactions: entry.capabilities?.directTransactions === true,
          configuredAuthMethods: Array.from(config.authMethods || []),
          projectConfigured: Boolean(config.projectId) && !String(config.projectId).startsWith("__"),
        };
      })()`),
      args.timeoutMs,
      "the Coinbase adapter to register",
    );

    const requiredMethods = ["email", "sms", "oauth:google", "oauth:apple", "oauth:x", "oauth:telegram"];
    const missingMethods = requiredMethods.filter((method) => !adapter.configuredAuthMethods.includes(method));
    if (!adapter.embedded || !adapter.userCustody || !adapter.authMethodLinking || !adapter.eip3009) {
      throw new Error("The live Coinbase adapter capabilities do not match the reviewed integration.");
    }
    if (!adapter.gasSponsoredOnSupportedRelays || adapter.directTransactions || !adapter.projectConfigured) {
      throw new Error("The live Coinbase adapter transaction boundary is incorrect.");
    }
    if (missingMethods.length) throw new Error(`The live adapter is missing authentication methods: ${missingMethods.join(", ")}`);

    await cdp.evaluate(`(() => {
      window.__agentBountiesCoinbaseSmoke = { started: true, rejected: false, error: null };
      window.AgentBountiesCoinbaseEmbeddedWallet.ensureAuthenticated().catch((error) => {
        window.__agentBountiesCoinbaseSmoke.rejected = true;
        window.__agentBountiesCoinbaseSmoke.error = String(error?.message || error || "unknown").slice(0, 240);
      });
      return true;
    })()`);

    const panel = await poll(
      async () => {
        const state = await cdp.evaluate(`(() => {
          const panel = document.querySelector(".wallet-auth-panel");
          const smoke = window.__agentBountiesCoinbaseSmoke || {};
          return {
            visible: Boolean(panel),
            rejected: Boolean(smoke.rejected),
            error: smoke.error || null,
            text: panel?.innerText?.replace(/\\s+/g, " ").trim().slice(0, 2_000) || "",
            buttons: panel ? Array.from(panel.querySelectorAll("button")).map((button) => ({
              text: button.innerText.replace(/\\s+/g, " ").trim(),
              ariaLabel: button.getAttribute("aria-label") || "",
              disabled: button.disabled,
            })) : [],
          };
        })()`);
        if (state.rejected && !state.visible) throw new Error(`Coinbase SDK rejected initialization: ${state.error || "unknown"}`);
        return state.visible ? state : null;
      },
      args.timeoutMs,
      "the maintained Coinbase sign-in interface",
    );

    for (const marker of [
      "Create or access your wallet",
      "Coinbase provides the non-custodial wallet",
      "Use the same sign-in method",
    ]) {
      if (!panel.text.includes(marker)) throw new Error(`The live sign-in panel is missing: ${marker}`);
    }
    const authButton = panel.buttons.find((button) => button.text && !button.ariaLabel.toLowerCase().includes("close"));
    if (!authButton || authButton.disabled) throw new Error("Coinbase's maintained authentication button did not render.");

    const providerConfig = await cdp.evaluate(`fetch(
      "https://api.cdp.coinbase.com/platform/v2/embedded-wallet-api/projects/" +
        encodeURIComponent(window.AgentBountiesWalletConfig.providers.coinbaseEmbedded.projectId) +
        "/config",
      { credentials: "include", headers: { Accept: "application/json" } },
    ).then(async (response) => ({
      ok: response.ok,
      status: response.status,
      contentType: response.headers.get("content-type") || "",
      jsonObject: response.ok && typeof (await response.json()) === "object",
    })).catch((error) => ({ ok: false, status: 0, error: String(error?.message || error).slice(0, 160) }))`, { awaitPromise: true });
    if (!providerConfig.ok || providerConfig.status !== 200 || !providerConfig.jsonObject) {
      throw new Error(`The live origin could not load its Coinbase project configuration: ${JSON.stringify(providerConfig)}`);
    }

    await cdp.evaluate(`(() => {
      document.querySelector(".wallet-auth-close")?.click();
      return true;
    })()`);
    await poll(
      () => cdp.evaluate("!document.querySelector('.wallet-auth-panel')"),
      5_000,
      "the authentication panel to close without submitting identity data",
    );

    const browserErrors = cdp.events.map((event) => {
      if (event.method === "Runtime.exceptionThrown") {
        return event.params?.exceptionDetails?.exception?.description || event.params?.exceptionDetails?.text || "runtime exception";
      }
      return event.params?.entry?.text || "log error";
    }).filter(Boolean).slice(0, 20);

    return {
      schema_version: "agent-bounties/coinbase-live-smoke-v1",
      success: true,
      url: args.url,
      adapter,
      signInPanel: {
        rendered: true,
        maintainedAuthButtonRendered: true,
        identitySubmitted: false,
        linkedMethodsReviewMarkerPresent: panel.text.includes("Use the same sign-in method"),
      },
      projectConfig: providerConfig,
      browserErrorCount: browserErrors.length,
      browserErrors,
      evidenceBoundary: "This proves the public project, origin, adapter registration, SDK initialization, and sign-in UI. It does not authenticate a user, create a wallet, sign a transaction, fund a bounty, or prove payment.",
    };
  } catch (error) {
    throw new Error(`${error.message || error}${stderr ? `\nChrome: ${stderr.slice(-2_000)}` : ""}`);
  } finally {
    cdp?.close();
    chrome.kill("SIGTERM");
    await Promise.race([
      new Promise((resolve) => chrome.once("exit", resolve)),
      delay(2_000),
    ]);
    if (!chrome.killed) chrome.kill("SIGKILL");
    await rm(profile, { recursive: true, force: true });
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  let report;
  try {
    report = await runSmoke(args);
  } catch (error) {
    report = {
      schema_version: "agent-bounties/coinbase-live-smoke-v1",
      success: false,
      url: args.url,
      error: String(error?.message || error).slice(0, 4_000),
      evidenceBoundary: "No authentication, wallet creation, signature, transaction, funding, or payment was attempted.",
    };
  }
  const rendered = `${JSON.stringify(report, null, 2)}\n`;
  process.stdout.write(rendered);
  if (args.output) {
    await writeFile(args.output, rendered, "utf8");
  }
  if (!report.success) process.exitCode = 1;
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
