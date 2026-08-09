(() => {
  "use strict";

  const NETWORK = "base-mainnet";
  const CHAIN_ID = "0x2105";
  const FIXED_PREIMAGES = Object.freeze({
    terms: "agent-bounties/open-competition-v1/public-leading-zero-work-v1",
    policy: "agent-bounties/open-competition-v1/first-valid-confirmed-reveal-v1",
    acceptance: "leading-zero-work-v1/difficulty-16/acceptance-v1",
  });
  const state = { account: null, api: "https://api.agentbounties.app", profile: null, provider: null };
  const by = (selector) => document.querySelector(selector);

  function output(message, tone = "") {
    const element = by("[data-create-output]");
    element.textContent = message;
    element.dataset.tone = tone;
  }

  function assertAddress(value, label) {
    const normalized = String(value || "").toLowerCase();
    if (!/^0x[0-9a-f]{40}$/.test(normalized)) throw new Error(`${label} is not a valid EVM address.`);
    return normalized;
  }

  async function json(url, options) {
    const response = await fetch(url, { cache: "no-store", ...options });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.message || body.error || `Request failed (${response.status}).`);
    return body;
  }

  function injectedMetaMask() {
    const providers = Array.isArray(window.ethereum && window.ethereum.providers)
      ? window.ethereum.providers
      : window.ethereum ? [window.ethereum] : [];
    return providers.find((provider) => provider && provider.isMetaMask) || null;
  }

  async function ensureNetwork() {
    const actual = String(await state.provider.request({ method: "eth_chainId" })).toLowerCase();
    if (actual !== CHAIN_ID) {
      await state.provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN_ID }] });
    }
    if (String(await state.provider.request({ method: "eth_chainId" })).toLowerCase() !== CHAIN_ID) {
      throw new Error("MetaMask did not switch to Base mainnet.");
    }
  }

  async function connect() {
    const provider = injectedMetaMask();
    if (!provider) throw new Error("MetaMask was not found in Brave.");
    const accounts = await provider.request({ method: "eth_requestAccounts" });
    state.provider = provider;
    state.account = assertAddress(accounts && accounts[0], "Connected wallet");
    await ensureNetwork();
    by("[data-creator-wallet]").textContent = state.account;
    updateButton();
    output("Wallet connected. Review the fixed verifier scope and bounded economics before creating the competition.");
  }

  function updateButton() {
    by("[data-create-competition-form] button[type='submit']").disabled = !(
      state.account && state.profile && state.profile.public_inventory_eligible === true
      && state.profile.deployment_state === "active_ready_to_earn"
    );
  }

  function usdcBaseUnits(value, label) {
    const amount = Number(value);
    if (!Number.isFinite(amount) || amount <= 0 || !/^\d+(?:\.\d{1,6})?$/.test(String(value))) {
      throw new Error(`${label} must be a positive USDC amount with at most six decimals.`);
    }
    return Math.round(amount * 1_000_000);
  }

  function commitment(preimage) {
    return window.AgentBountiesEvm.keccak256Hex(window.AgentBountiesEvm.textHex(preimage));
  }

  async function waitForReceipt(transactionHash) {
    for (let attempt = 0; attempt < 90; attempt += 1) {
      const receipt = await state.provider.request({ method: "eth_getTransactionReceipt", params: [transactionHash] });
      if (receipt) {
        if (String(receipt.status).toLowerCase() !== "0x1") throw new Error(`Transaction reverted: ${transactionHash}`);
        return receipt;
      }
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    throw new Error(`Timed out waiting for transaction receipt: ${transactionHash}`);
  }

  function walletTransaction(call) {
    const from = assertAddress(call.from || state.account, "Prepared sender");
    const to = assertAddress(call.to, "Prepared target");
    if (from !== state.account) throw new Error("Prepared sender does not match the connected wallet.");
    if (!/^0x(?:[0-9a-fA-F]{2})*$/.test(String(call.data || ""))) throw new Error("Prepared calldata is malformed.");
    return { from, to, data: call.data, value: `0x${BigInt(call.value_wei || 0).toString(16)}` };
  }

  async function sendWalletCalls(plan) {
    if (!plan.ready_to_broadcast || !plan.public_inventory_eligible || !Array.isArray(plan.wallet_calls) || plan.wallet_calls.length !== 2) {
      throw new Error("Creation plan is not an exact public approval-and-create sequence.");
    }
    await ensureNetwork();
    for (let index = 0; index < plan.wallet_calls.length; index += 1) {
      output(`Confirm wallet transaction ${index + 1} of ${plan.wallet_calls.length}. The first sets the exact USDC allowance; the second creates and funds the competition.`);
      const transactionHash = await state.provider.request({
        method: "eth_sendTransaction",
        params: [walletTransaction(plan.wallet_calls[index])],
      });
      await waitForReceipt(transactionHash);
    }
  }

  async function waitForCanonicalCreation(plan) {
    const url = `${state.api}/v1/base/open-competition-v1/events?network=${NETWORK}&bounty_id=${plan.bounty_id}`;
    for (let attempt = 0; attempt < 45; attempt += 1) {
      const body = await json(url);
      const kinds = new Set((body.events || []).map((event) => event.kind));
      if (kinds.has("canonical_competition_created") && kinds.has("competition_opened")) return url;
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    throw new Error("Transactions confirmed, but canonical creation and funding events are not indexed yet. Do not advertise the competition as open until they appear.");
  }

  async function createCompetition(event) {
    event.preventDefault();
    const form = event.currentTarget;
    if (!state.account || !state.profile) throw new Error("Connect the creator wallet first.");
    if (window.AgentBountiesLegal) {
      await window.AgentBountiesLegal.requireAcceptance({ action: "post_bounty", walletAddress: state.account, scope: document.body });
    }
    const solverReward = usdcBaseUnits(form.elements.solverReward.value, "Solver reward");
    const verifierReward = usdcBaseUnits(form.elements.verifierReward.value, "Verifier reward");
    const competitionWindow = Number(form.elements.competitionHours.value) * 3_600;
    const revealWindow = Number(form.elements.revealHours.value) * 3_600;
    const maxEntries = Number(form.elements.maxEntries.value);
    if (!Number.isInteger(competitionWindow) || competitionWindow < 3_600 || competitionWindow > 2_592_000) throw new Error("Competition window is out of range.");
    if (!Number.isInteger(revealWindow) || revealWindow < 3_600 || revealWindow > 86_400 || revealWindow > competitionWindow) throw new Error("Reveal window is out of range.");
    if (!Number.isInteger(maxEntries) || maxEntries < 2 || maxEntries > 64) throw new Error("Maximum entries must be from 2 to 64.");
    const now = Math.floor(Date.now() / 1_000);
    const request = {
      network: NETWORK,
      creator: state.account,
      creation_nonce: window.AgentBountiesEvm.randomBytes32(),
      initial_funding: solverReward + verifierReward,
      verifier_profile_id: state.profile.profile_id,
      params: {
        solver_reward: solverReward,
        verifier_reward: verifierReward,
        terms_hash: commitment(FIXED_PREIMAGES.terms),
        policy_hash: commitment(FIXED_PREIMAGES.policy),
        acceptance_criteria_hash: commitment(FIXED_PREIMAGES.acceptance),
        benchmark_hash: state.profile.benchmark_hash,
        evidence_schema_hash: state.profile.evidence_schema_hash,
        funding_deadline: now + 7 * 86_400,
        competition_window_seconds: competitionWindow,
        reveal_window_seconds: revealWindow,
        max_entries: maxEntries,
        verifier_reward_recipient: state.account,
      },
      funding_authorization: null,
    };
    const plan = await json(`${state.api}/v1/base/open-competition-v1/creation-preparation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(request),
    });
    if (String(plan.creator).toLowerCase() !== state.account || plan.verifier_profile_id !== state.profile.profile_id) {
      throw new Error("Creation plan identity does not match the approved request.");
    }
    await sendWalletCalls(plan);
    const eventsUrl = await waitForCanonicalCreation(plan);
    const competitionUrl = `competition.html?bountyContract=${encodeURIComponent(plan.predicted_bounty_contract)}&network=${NETWORK}&verifierProfileId=${encodeURIComponent(state.profile.profile_id)}`;
    by("[data-created-address]").textContent = plan.predicted_bounty_contract;
    by("[data-created-link]").href = competitionUrl;
    by("[data-created-events]").href = eventsUrl;
    by("[data-created-competition]").hidden = false;
    output("Canonical creation, funding, and competition-open events confirmed. The competition is now public and ready for eligible entries.", "ready");
  }

  function guard(handler) {
    return async (event) => {
      try { await handler(event); } catch (error) { output(error.message || "Competition creation failed.", "error"); }
    };
  }

  async function initialize() {
    const protocol = await json("protocol.json");
    state.api = String(protocol.api_base_url || state.api).replace(/\/$/, "");
    const catalog = await json(`${state.api}/v1/base/open-competition-v1/verifiers?network=${NETWORK}`);
    if (!Array.isArray(catalog.profiles) || catalog.profiles.length !== 1) throw new Error("The public verifier catalog is unavailable or ambiguous.");
    state.profile = catalog.profiles[0];
    by("[data-profile-name]").textContent = state.profile.display_name;
    const active = state.profile.public_inventory_eligible === true && state.profile.deployment_state === "active_ready_to_earn";
    by("[data-create-stage]").textContent = active
      ? "Public deterministic profile active."
      : "Open Competition creation is not active; no wallet transaction can be prepared.";
    by("[data-create-stage]").dataset.tone = active ? "ready" : "blocked";
    by("[data-connect-creator]").addEventListener("click", guard(connect));
    by("[data-create-competition-form]").addEventListener("submit", guard(createCompetition));
    updateButton();
  }

  initialize().catch((error) => {
    by("[data-create-stage]").textContent = error.message || "Open Competition is unavailable.";
    by("[data-create-stage]").dataset.tone = "blocked";
    output("Creation is disabled until the exact public verifier catalog is available.", "error");
  });
})();
