import {
  createCDPEmbeddedWallet,
  getCurrentUser,
  initialize,
  signInWithEmail,
  signInWithOAuth,
  signInWithSms,
  signOut,
  verifyEmailOTP,
  verifySmsOTP,
} from "@coinbase/cdp-core";
import { http } from "viem";
import { base } from "viem/chains";
import { accountFromUser, createAuthenticatedProvider, userRejected } from "./provider.js";

const runtime = window.AgentBountiesWalletRuntime;
const adapterConfig = runtime?.adapters?.coinbaseEmbedded;
const registry = window.AgentBountiesWalletAdapters;
const ADAPTER_ID = "coinbase-embedded";
const INFO = Object.freeze({
  uuid: crypto.randomUUID(),
  name: "Email or social wallet (Coinbase)",
  icon: `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="18" fill="#1652f0"/><path fill="white" d="M32 14c9.9 0 18 8.1 18 18s-8.1 18-18 18-18-8.1-18-18 8.1-18 18-18Zm0 8.5a9.5 9.5 0 1 0 0 19h8v-7H32a2.5 2.5 0 1 1 0-5h8v-7h-8Z"/></svg>')}`,
  rdns: "app.agentbounties.wallet.coinbase-embedded",
});

function publishStatus(status, details = {}) {
  window.dispatchEvent(new CustomEvent("agentbounties:embedded-wallet-status", {
    detail: Object.freeze({ adapter: ADAPTER_ID, status, ...details }),
  }));
}

function humanError(error) {
  const message = String(error?.message || error || "The wallet request failed.").trim();
  return message.slice(0, 320);
}

function assertConfiguration() {
  if (!runtime || runtime.schemaVersion !== "agent-bounties/wallet-runtime-v1") {
    throw new Error("Agent Bounties wallet runtime configuration is unavailable.");
  }
  if (!adapterConfig?.enabled || !String(adapterConfig.projectId || "").trim()) {
    throw new Error("Coinbase embedded wallet activation is incomplete.");
  }
  if (adapterConfig.accountType !== "eoa") {
    throw new Error("Coinbase embedded wallets must use an EOA until Agent Bounties supports contract signatures.");
  }
  if (runtime.network?.chainId !== base.id || runtime.network?.chainIdHex !== "0x2105") {
    throw new Error("Coinbase embedded wallet configuration must target Base mainnet.");
  }
}

let sdkPromise = null;
let cdpProvider = null;
let authPromise = null;
let authDeferred = null;
let dialog = null;
let currentFlow = null;

async function initializeSdk() {
  assertConfiguration();
  if (!sdkPromise) {
    sdkPromise = initialize({
      projectId: String(adapterConfig.projectId).trim(),
      disableAnalytics: adapterConfig.disableAnalytics !== false,
      ethereum: { createOnLogin: "eoa" },
    }).then(() => {
      publishStatus("ready", { accountType: "eoa", network: "base-mainnet" });
    }).catch((error) => {
      sdkPromise = null;
      publishStatus("error", { message: humanError(error) });
      throw error;
    });
  }
  await sdkPromise;
}

async function loadProvider() {
  await initializeSdk();
  if (!cdpProvider) {
    const wallet = createCDPEmbeddedWallet({
      chains: [base],
      transports: { [base.id]: http(runtime.network.rpcUrl) },
      announceProvider: false,
    });
    cdpProvider = wallet.provider;
  }
  return cdpProvider;
}

async function currentUser() {
  await initializeSdk();
  try {
    return await getCurrentUser();
  } catch (_error) {
    return null;
  }
}

async function authenticated() {
  return Boolean(accountFromUser(await currentUser()));
}

function buildDialog() {
  if (dialog) return dialog;
  dialog = document.createElement("dialog");
  dialog.className = "embedded-wallet-dialog";
  dialog.setAttribute("aria-labelledby", "embedded-wallet-title");
  dialog.innerHTML = `
    <form method="dialog" class="embedded-wallet-shell" data-embedded-wallet-shell>
      <header class="embedded-wallet-head">
        <div>
          <p class="eyebrow">Your non-custodial Base wallet</p>
          <h2 id="embedded-wallet-title">Create or access your wallet</h2>
          <p>Use email, SMS, or a social account. No browser extension, seed phrase, or Base ETH is required.</p>
        </div>
        <button class="embedded-wallet-close" type="button" aria-label="Close wallet sign in" data-wallet-auth-close>×</button>
      </header>

      <dl class="embedded-wallet-assurance">
        <div><dt>Custody</dt><dd>You control the wallet</dd></div>
        <div><dt>Network</dt><dd>Base mainnet</dd></div>
        <div><dt>Gas</dt><dd>Sponsored by Agent Bounties</dd></div>
      </dl>

      <div class="embedded-wallet-tabs" role="tablist" aria-label="Wallet sign-in method">
        <button type="button" role="tab" aria-selected="true" data-wallet-auth-tab="email">Email</button>
        <button type="button" role="tab" aria-selected="false" data-wallet-auth-tab="sms">SMS</button>
        <button type="button" role="tab" aria-selected="false" data-wallet-auth-tab="social">Social</button>
      </div>

      <section class="embedded-wallet-panel" role="tabpanel" data-wallet-auth-panel="email">
        <label>Email address<input type="email" autocomplete="email" inputmode="email" data-wallet-auth-email></label>
        <div class="embedded-wallet-actions">
          <button class="embedded-wallet-action secondary" type="button" data-wallet-email-send>Send code</button>
          <button class="embedded-wallet-action" type="button" data-wallet-email-verify>Verify and create wallet</button>
        </div>
        <label>Six-digit code<input type="text" autocomplete="one-time-code" inputmode="numeric" maxlength="6" pattern="[0-9]{6}" data-wallet-email-otp></label>
      </section>

      <section class="embedded-wallet-panel" role="tabpanel" data-wallet-auth-panel="sms" hidden>
        <label>Phone number in international format<input type="tel" autocomplete="tel" inputmode="tel" placeholder="+5215555555555" data-wallet-auth-phone></label>
        <div class="embedded-wallet-actions">
          <button class="embedded-wallet-action secondary" type="button" data-wallet-sms-send>Send code</button>
          <button class="embedded-wallet-action" type="button" data-wallet-sms-verify>Verify and create wallet</button>
        </div>
        <label>Six-digit code<input type="text" autocomplete="one-time-code" inputmode="numeric" maxlength="6" pattern="[0-9]{6}" data-wallet-sms-otp></label>
        <p class="embedded-wallet-fine">SMS availability depends on Coinbase's supported countries and carriers. Email remains available when SMS is not.</p>
      </section>

      <section class="embedded-wallet-panel" role="tabpanel" data-wallet-auth-panel="social" hidden>
        <div class="embedded-wallet-social">
          <button type="button" data-wallet-oauth="google">Continue with Google</button>
          <button type="button" data-wallet-oauth="apple">Continue with Apple</button>
          <button type="button" data-wallet-oauth="x">Continue with X</button>
        </div>
        <p class="embedded-wallet-fine">Coinbase handles authentication and returns you to this exact Agent Bounties page. Your bounty action remains a separate approval.</p>
      </section>

      <output class="embedded-wallet-status" aria-live="polite" data-wallet-auth-status>No wallet has been created or connected yet.</output>
      <p class="embedded-wallet-fine">Coinbase supplies the wallet infrastructure. Agent Bounties never receives a private key, seed phrase, email code, or SMS code.</p>
    </form>`;
  document.body.append(dialog);

  const status = dialog.querySelector("[data-wallet-auth-status]");
  const buttons = Array.from(dialog.querySelectorAll("button"));

  function setStatus(message, tone = "") {
    status.textContent = message;
    status.dataset.tone = tone;
  }

  function setBusy(busy) {
    buttons.forEach((button) => {
      if (!button.matches("[data-wallet-auth-close]")) button.disabled = busy;
    });
  }

  function selectTab(name) {
    dialog.querySelectorAll("[data-wallet-auth-tab]").forEach((button) => {
      button.setAttribute("aria-selected", String(button.dataset.walletAuthTab === name));
    });
    dialog.querySelectorAll("[data-wallet-auth-panel]").forEach((panel) => {
      panel.hidden = panel.dataset.walletAuthPanel !== name;
    });
    currentFlow = null;
    setStatus("Choose a sign-in method. Nothing is posted, funded, or claimed by signing in.");
  }

  async function completeAuthentication(label) {
    let user = null;
    for (let attempt = 0; attempt < 8; attempt += 1) {
      user = await currentUser();
      if (accountFromUser(user)) break;
      await new Promise((resolve) => setTimeout(resolve, 350));
    }
    const account = accountFromUser(user);
    if (!account) throw new Error("Coinbase authenticated the user but did not return an EVM EOA.");
    await loadProvider();
    setStatus(`${label} Wallet ready: ${account.slice(0, 6)}…${account.slice(-4)}.`, "success");
    const deferred = authDeferred;
    authDeferred = null;
    authPromise = null;
    currentFlow = null;
    deferred?.resolve(account);
    dialog.close("connected");
    publishStatus("connected", { account });
    return account;
  }

  async function run(action) {
    setBusy(true);
    try {
      await action();
    } catch (error) {
      setStatus(humanError(error), "error");
    } finally {
      setBusy(false);
    }
  }

  dialog.querySelectorAll("[data-wallet-auth-tab]").forEach((button) => {
    button.addEventListener("click", () => selectTab(button.dataset.walletAuthTab));
  });

  dialog.querySelector("[data-wallet-auth-close]").addEventListener("click", () => dialog.close("cancel"));

  dialog.querySelector("[data-wallet-email-send]").addEventListener("click", () => run(async () => {
    if (await authenticated()) return completeAuthentication("Welcome back.");
    const email = String(dialog.querySelector("[data-wallet-auth-email]").value || "").trim();
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) throw new Error("Enter a valid email address.");
    const result = await signInWithEmail({ email });
    currentFlow = { kind: "email", flowId: result.flowId };
    setStatus("Coinbase sent a six-digit code. It expires after a short time.");
    dialog.querySelector("[data-wallet-email-otp]").focus();
  }));

  dialog.querySelector("[data-wallet-email-verify]").addEventListener("click", () => run(async () => {
    if (!currentFlow || currentFlow.kind !== "email") throw new Error("Send an email code first.");
    const otp = String(dialog.querySelector("[data-wallet-email-otp]").value || "").trim();
    if (!/^\d{6}$/.test(otp)) throw new Error("Enter the six-digit email code.");
    await verifyEmailOTP({ flowId: currentFlow.flowId, otp });
    await completeAuthentication("Email verified.");
  }));

  dialog.querySelector("[data-wallet-sms-send]").addEventListener("click", () => run(async () => {
    if (await authenticated()) return completeAuthentication("Welcome back.");
    const phoneNumber = String(dialog.querySelector("[data-wallet-auth-phone]").value || "").replace(/[\s()-]/g, "");
    if (!/^\+[1-9]\d{7,14}$/.test(phoneNumber)) throw new Error("Enter a phone number in international format, beginning with + and the country code.");
    const result = await signInWithSms({ phoneNumber });
    currentFlow = { kind: "sms", flowId: result.flowId };
    setStatus("Coinbase sent a six-digit SMS code. Carrier and country availability may vary.");
    dialog.querySelector("[data-wallet-sms-otp]").focus();
  }));

  dialog.querySelector("[data-wallet-sms-verify]").addEventListener("click", () => run(async () => {
    if (!currentFlow || currentFlow.kind !== "sms") throw new Error("Send an SMS code first.");
    const otp = String(dialog.querySelector("[data-wallet-sms-otp]").value || "").trim();
    if (!/^\d{6}$/.test(otp)) throw new Error("Enter the six-digit SMS code.");
    await verifySmsOTP({ flowId: currentFlow.flowId, otp });
    await completeAuthentication("Phone verified.");
  }));

  dialog.querySelectorAll("[data-wallet-oauth]").forEach((button) => {
    button.addEventListener("click", () => run(async () => {
      if (await authenticated()) return completeAuthentication("Welcome back.");
      const provider = button.dataset.walletOauth;
      if (!["google", "apple", "x"].includes(provider)) throw new Error("Unsupported social provider.");
      sessionStorage.setItem("agentbounties:embedded-wallet-return", location.href);
      setStatus(`Opening ${provider === "x" ? "X" : provider[0].toUpperCase() + provider.slice(1)}. You will return to this page.`, "success");
      await signInWithOAuth(provider);
    }));
  });

  dialog.addEventListener("close", () => {
    if (dialog.returnValue === "connected") return;
    const deferred = authDeferred;
    authDeferred = null;
    authPromise = null;
    currentFlow = null;
    deferred?.reject(userRejected());
  });

  dialog.AgentBountiesSetStatus = setStatus;
  return dialog;
}

async function openAuthentication() {
  await initializeSdk();
  const existing = accountFromUser(await currentUser());
  if (existing) return existing;
  if (authPromise) return authPromise;
  const authDialog = buildDialog();
  authPromise = new Promise((resolve, reject) => {
    authDeferred = { resolve, reject };
  });
  authDialog.returnValue = "";
  authDialog.AgentBountiesSetStatus("Choose email, SMS, or social login. Creating a wallet does not authorize any bounty action.");
  authDialog.showModal();
  return authPromise;
}

async function signOutEmbeddedWallet() {
  await initializeSdk();
  await signOut();
  publishStatus("signed_out");
}

function inactiveReason() {
  if (!adapterConfig?.enabled) return "CDP_PROJECT_ID has not been configured for the deployed site.";
  if (!adapterConfig?.projectId) return "Coinbase embedded wallet project ID is missing.";
  return null;
}

if (!registry) {
  publishStatus("disabled", { message: "Wallet adapter registry loaded after the Coinbase adapter." });
} else if (inactiveReason()) {
  publishStatus("disabled", { message: inactiveReason() });
  window.AgentBountiesCoinbaseWallet = Object.freeze({
    schemaVersion: "agent-bounties/coinbase-embedded-wallet-v1",
    enabled: false,
    reason: inactiveReason(),
  });
} else {
  const provider = createAuthenticatedProvider({
    loadProvider,
    ensureAuthenticated: openAuthentication,
    isAuthenticated: authenticated,
    chainIdHex: runtime.network.chainIdHex,
  });

  registry.register({
    id: ADAPTER_ID,
    info: INFO,
    provider,
    capabilities: {
      embedded: true,
      nonCustodial: true,
      accountType: "eoa",
      network: "base-mainnet",
      authMethods: [...adapterConfig.authMethods],
      gasSponsored: runtime.gasSponsored === true,
      requiresBrowserExtension: false,
      requiresSeedPhrase: false,
    },
    open: openAuthentication,
  });

  window.AgentBountiesCoinbaseWallet = Object.freeze({
    schemaVersion: "agent-bounties/coinbase-embedded-wallet-v1",
    enabled: true,
    provider,
    open: openAuthentication,
    signOut: signOutEmbeddedWallet,
    currentUser,
  });

  void initializeSdk().then(async () => {
    const account = accountFromUser(await currentUser());
    publishStatus(account ? "authenticated" : "ready", account ? { account } : {});
  }).catch(() => {});
}
