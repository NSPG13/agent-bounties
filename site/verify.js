(() => {
  "use strict";

  const byId = (id) => document.getElementById(id);
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  const ADDRESS = /^0x[0-9a-f]{40}$/i;
  const TX_HASH = /^0x[0-9a-f]{64}$/i;
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const state = { protocol: null, intent: null, bounty: null, account: null, provider: null };

  function output(lines, tone = "") {
    const element = byId("verify-output");
    element.textContent = Array.isArray(lines) ? lines.join("\n") : lines;
    element.dataset.tone = tone;
  }

  async function requestJson(url, options = {}) {
    const acceptance = window.AgentBountiesLegal?.latestReceipt();
    const response = await fetch(url, {
      ...options,
      headers: {
        "content-type": "application/json",
        ...(acceptance ? { "x-agent-bounties-legal-acceptance": acceptance.acceptance_id } : {}),
        ...(options.headers || {}),
      },
    });
    const body = await response.json().catch(() => null);
    if (!response.ok) {
      throw new Error(body?.message || body?.error || `Request failed (${response.status}).`);
    }
    return body;
  }

  function intentId() {
    const value = new URLSearchParams(location.search).get("intent");
    if (!UUID.test(value || "")) throw new Error("This verification link is invalid.");
    return value;
  }

  function apiBase() {
    return state.protocol.api_base_url.replace(/\/$/, "");
  }

  async function load() {
    state.protocol = await requestJson("protocol.json", { cache: "no-store" });
    state.intent = await requestJson(`${apiBase()}/v1/chatgpt/action-intents/${intentId()}`);
    if (state.intent.action !== "verify") throw new Error("This intent is not a verification action.");
    if (!ADDRESS.test(state.intent.bounty_contract || "")) {
      throw new Error("The verification intent has no valid bounty contract.");
    }
    const feed = await requestJson(
      `${apiBase()}/v1/base/autonomous-bounties/feed?network=${encodeURIComponent(state.intent.network)}&claimable_only=false`,
    );
    state.bounty = feed.find((item) =>
      item.bounty_contract.toLowerCase() === state.intent.bounty_contract.toLowerCase());
    if (!state.bounty || state.bounty.status !== "submitted" || !state.bounty.terms_valid) {
      throw new Error("This bounty does not have one valid submitted round ready for verification.");
    }
    const form = byId("verify-form");
    form.elements.bountyContract.value = state.bounty.bounty_contract;
    form.elements.verificationMode.value = state.bounty.verification_mode;
    const deterministic = state.bounty.verification_mode === "deterministic_module";
    form.querySelector("[data-module-proof]").hidden = !deterministic;
    form.elements.proof.required = deterministic;
    form.querySelector("[data-attestations]").hidden = deterministic;
    form.elements.attestations.required = !deterministic;
    if (typeof state.intent.details?.proof === "string") {
      form.elements.proof.value = state.intent.details.proof;
    }
    if (Array.isArray(state.intent.details?.attestations)) {
      form.elements.attestations.value = JSON.stringify(state.intent.details.attestations, null, 2);
    }
    byId("verify-status").textContent = state.intent.status.replaceAll("_", " ");
    byId("verify-status").dataset.tone =
      state.intent.status === "confirmed" ? "success" : "pending";
    if (state.intent.status === "confirmed") {
      output([
        `Confirmed: ${state.intent.canonical_event_kind}.`,
        state.intent.paid
          ? "BountySettled confirms solver payment."
          : "SubmissionRejected confirms rejection; no solver payment occurred.",
        "Return to ChatGPT, refresh the card, and share the result.",
      ], "success");
    } else if (state.intent.status === "pending_confirmation") {
      output([
        `Transaction observed: ${state.intent.transaction_hash}.`,
        "Waiting for indexed BountySettled or SubmissionRejected. Do not submit again.",
      ], "pending");
    }
  }

  async function connect() {
    const provider = window.ethereum;
    if (!provider?.request) throw new Error("Install or unlock a browser wallet first.");
    state.provider = provider;
    const accounts = await provider.request({ method: "eth_requestAccounts" });
    if (!accounts?.[0] || !ADDRESS.test(accounts[0])) throw new Error("Wallet returned no valid account.");
    state.account = accounts[0].toLowerCase();
    const chainId = await provider.request({ method: "eth_chainId" });
    if (String(chainId).toLowerCase() !== state.protocol.chain_id_hex.toLowerCase()) {
      await provider.request({
        method: "wallet_switchEthereumChain",
        params: [{ chainId: state.protocol.chain_id_hex }],
      });
    }
    byId("verify-connect").textContent = `${state.account.slice(0, 6)}…${state.account.slice(-4)}`;
    byId("verify-submit").disabled = state.intent.status !== "review_required";
    output("Wallet connected. Review the proof or exact signed quorum before continuing.", "pending");
  }

  async function waitReceipt(hash) {
    const started = Date.now();
    while (Date.now() - started < 120_000) {
      const receipt = await state.provider.request({
        method: "eth_getTransactionReceipt",
        params: [hash],
      });
      if (receipt) {
        if (receipt.status !== "0x1") throw new Error(`Verification transaction reverted: ${hash}`);
        return receipt;
      }
      await sleep(1_500);
    }
    throw new Error(`Transaction confirmation timed out: ${hash}`);
  }

  async function submit(event) {
    event.preventDefault();
    try {
      if (!state.account) await connect();
      await window.AgentBountiesLegal.requireAcceptance({
        action: "verify_submission",
        walletAddress: state.account,
        scope: event.currentTarget,
      });
      const deterministic = state.bounty.verification_mode === "deterministic_module";
      let endpoint;
      let body;
      if (deterministic) {
        const proof = event.currentTarget.elements.proof.value.trim();
        if (!/^0x(?:[0-9a-f]{2})*$/i.test(proof)) {
          throw new Error("Deterministic proof must be even-length 0x-prefixed bytes.");
        }
        endpoint = "module-settlement-plan";
        body = {
          network: state.intent.network,
          bounty_contract: state.bounty.bounty_contract,
          caller: state.account,
          proof,
        };
      } else {
        let attestations;
        try {
          attestations = JSON.parse(event.currentTarget.elements.attestations.value);
        } catch (_error) {
          throw new Error("Signed attestation quorum must be valid JSON.");
        }
        if (!Array.isArray(attestations) || !attestations.length) {
          throw new Error("Provide the exact committed signed attestation quorum.");
        }
        endpoint = "attestation-settlement-plan";
        body = {
          network: state.intent.network,
          bounty_contract: state.bounty.bounty_contract,
          caller: state.account,
          attestations,
        };
      }
      output("The API validated the committed policy. Review the exact wallet transaction.", "pending");
      const plan = await requestJson(
        `${apiBase()}/v1/base/autonomous-bounties/${endpoint}`,
        { method: "POST", body: JSON.stringify(body) },
      );
      if (!ADDRESS.test(plan.to || "") || !/^0x[0-9a-f]*$/i.test(plan.data || "")) {
        throw new Error("The verification planner returned an invalid transaction.");
      }
      const hash = await state.provider.request({
        method: "eth_sendTransaction",
        params: [{ from: state.account, to: plan.to, data: plan.data, value: "0x0" }],
      });
      if (!TX_HASH.test(hash || "")) throw new Error("Wallet returned an invalid transaction hash.");
      await waitReceipt(hash);
      state.intent = await requestJson(
        `${apiBase()}/v1/chatgpt/action-intents/${state.intent.intent_id}/observations`,
        {
          method: "POST",
          body: JSON.stringify({
            transaction_hash: hash,
            bounty_contract: state.bounty.bounty_contract,
            bounty_id: state.bounty.bounty_id,
            actor_wallet: state.account,
          }),
        },
      );
      output([
        `Transaction confirmed: ${state.protocol.explorer_url}/tx/${hash}`,
        "Waiting for indexed BountySettled or SubmissionRejected. The receipt alone is not settlement.",
      ], "pending");
      for (let attempt = 0; attempt < 36; attempt += 1) {
        if (state.intent.status === "confirmed") break;
        await sleep(2_500);
        state.intent = await requestJson(
          `${apiBase()}/v1/chatgpt/action-intents/${state.intent.intent_id}`,
        );
      }
      if (state.intent.status !== "confirmed") {
        throw new Error("Canonical verification is still indexing. Refresh this same link; do not submit again.");
      }
      byId("verify-status").textContent = "confirmed";
      byId("verify-status").dataset.tone = "success";
      output([
        `Confirmed: ${state.intent.canonical_event_kind}.`,
        state.intent.paid
          ? "BountySettled confirms solver payment."
          : "SubmissionRejected confirms rejection; no solver payment occurred.",
        "Return to ChatGPT, refresh the card, and share the result.",
      ], "success");
      byId("verify-submit").disabled = true;
    } catch (error) {
      output(error.message || String(error), "error");
    }
  }

  byId("verify-connect").addEventListener("click", () => connect().catch((error) => {
    output(error.message || String(error), "error");
  }));
  byId("verify-form").addEventListener("submit", submit);
  load().catch((error) => {
    byId("verify-status").textContent = "unavailable";
    byId("verify-status").dataset.tone = "error";
    output(error.message || String(error), "error");
  });
})();
