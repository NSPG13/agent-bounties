(() => {
  "use strict";

  const params = new URLSearchParams(location.search);
  const network = params.get("network") || "base-mainnet";
  const bounty = String(params.get("bountyContract") || "").toLowerCase();
  const profileId = params.get("verifierProfileId");
  const state = {
    account: null,
    api: "https://api.agentbounties.app",
    canonical: null,
    envelope: null,
    profile: null,
    provider: null,
  };

  const by = (selector) => document.querySelector(selector);
  const setText = (selector, value) => {
    const element = by(selector);
    if (element) element.textContent = value;
  };

  function output(message, tone = "") {
    const element = by("[data-competition-output]");
    element.textContent = message;
    element.dataset.tone = tone;
  }

  function assertAddress(value, label) {
    if (!/^0x[0-9a-f]{40}$/.test(value)) throw new Error(`${label} is not a valid EVM address.`);
    return value;
  }

  function formatUsdc(baseUnits) {
    const amount = Number(baseUnits || 0) / 1_000_000;
    return `${amount.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 6 })} USDC`;
  }

  function formatTimestamp(value) {
    if (!Number(value)) return "Not opened";
    return new Date(Number(value) * 1000).toLocaleString();
  }

  async function json(url, options) {
    const response = await fetch(url, { cache: "no-store", ...options });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.message || body.error || `Request failed (${response.status}).`);
    return body;
  }

  async function loadApi() {
    const protocol = await json("protocol.json");
    state.api = String(protocol.api_base_url || state.api).replace(/\/$/, "");
  }

  async function loadCatalog() {
    const catalog = await json(`${state.api}/v1/base/open-competition-v1/verifiers?network=${encodeURIComponent(network)}`);
    const selected = profileId
      ? catalog.profiles.find((profile) => profile.profile_id === profileId)
      : catalog.profiles[0];
    if (!selected) throw new Error("No approved verifier profile is available for this network.");
    state.profile = selected;
  }

  async function loadCanonicalState() {
    assertAddress(bounty, "Bounty contract");
    const query = new URLSearchParams({
      network,
      bounty_contract: bounty,
      verifier_profile_id: state.profile.profile_id,
    });
    if (state.account) query.set("solver", state.account);
    state.canonical = await json(`${state.api}/v1/base/open-competition-v1/state?${query}`);
    renderCanonicalState();
  }

  function renderCanonicalState() {
    const observed = state.canonical;
    const ready = observed.public_inventory_eligible === true;
    const stage = by("[data-competition-stage]");
    stage.textContent = ready
      ? "Canonical checks passed. Entry preparation is active."
      : `${String(observed.deployment_state || "source_only_not_ready_to_earn").replaceAll("_", " ")}. New entries fail closed.`;
    stage.dataset.tone = ready ? "ready" : "blocked";
    setText("[data-competition-name]", "Open competition");
    setText("[data-state-status]", String(observed.status_name || "unknown"));
    setText("[data-state-reward]", formatUsdc(observed.solver_reward || observed.target_amount));
    setText("[data-state-bond]", formatUsdc(observed.entry_bond || observed.verifier_reward));
    setText("[data-state-capacity]", `${observed.entry_count} of ${observed.max_entries} entries used`);
    setText("[data-state-deadline]", formatTimestamp(observed.competition_ends_at));
    setText("[data-state-reveal]", `${Number(observed.reveal_window_seconds || 0).toLocaleString()} seconds`);
    setText("[data-state-verifier]", `${state.profile.display_name} · ${state.profile.verifier_address}`);
    setText("[data-state-block]", `${observed.safe_block_number} · ${observed.safe_block_hash}`);
    updateButtons();
  }

  function injectedMetaMask() {
    const providers = Array.isArray(window.ethereum && window.ethereum.providers)
      ? window.ethereum.providers
      : window.ethereum ? [window.ethereum] : [];
    return providers.find((provider) => provider && provider.isMetaMask) || null;
  }

  async function connect() {
    const provider = injectedMetaMask();
    if (!provider) throw new Error("MetaMask was not found in Brave.");
    const accounts = await provider.request({ method: "eth_requestAccounts" });
    const account = String(accounts && accounts[0] || "").toLowerCase();
    assertAddress(account, "Connected wallet");
    state.provider = provider;
    state.account = account;
    setText("[data-competition-wallet]", account);
    await loadCanonicalState();
    output("Wallet connected. Generate and download a recovery envelope before preparing an entry.");
  }

  function updateButtons() {
    const ready = Boolean(state.account && state.canonical && state.canonical.public_inventory_eligible);
    by("[data-generate-commitment]").disabled = !ready;
    by("[data-competition-commit-form] button[type='submit']").disabled = !(ready && state.envelope);
    by("[data-competition-reveal-form] button[type='submit']").disabled = !ready;
  }

  function createEnvelope() {
    if (!state.account || !state.canonical) throw new Error("Connect the solver wallet first.");
    const form = by("[data-competition-commit-form]");
    const submissionHash = window.AgentBountiesEvm.bytes32Word(form.elements.submissionHash.value, "Submission hash");
    const evidenceHash = window.AgentBountiesEvm.bytes32Word(form.elements.evidenceHash.value, "Evidence hash");
    const salt = window.AgentBountiesEvm.randomBytes32();
    const chainId = network === "base-mainnet" ? 8453 : network === "base-sepolia" ? 84532 : 0;
    if (!chainId) throw new Error("Unsupported Base network.");
    const domain = window.AgentBountiesEvm.keccak256Hex(
      window.AgentBountiesEvm.textHex("agent-bounties/open-competition-v1-solution"),
    );
    const encoded = `0x${domain.slice(2)}${window.AgentBountiesEvm.uint256Word(chainId)}${window.AgentBountiesEvm.addressWord(bounty)}${window.AgentBountiesEvm.addressWord(state.account)}${submissionHash}${evidenceHash}${salt.slice(2)}`;
    const commitment = window.AgentBountiesEvm.keccak256Hex(encoded);
    state.envelope = {
      schema_version: "agent-bounties/open-competition-v1-commitment-v1",
      network,
      chain_id: chainId,
      bounty,
      solver: state.account,
      submission_hash: `0x${submissionHash}`,
      evidence_hash: `0x${evidenceHash}`,
      salt,
      commitment,
      committed_block: null,
      reveal_deadline: null,
      evidence_boundary: "This recovery envelope contains the secret salt. Store it locally and send only commitment during entry preparation.",
    };
    const blob = new Blob([`${JSON.stringify(state.envelope, null, 2)}\n`], { type: "application/json" });
    const link = by("[data-download-envelope]");
    link.href = URL.createObjectURL(blob);
    link.download = `open-competition-${bounty.slice(2, 10)}-${commitment.slice(2, 10)}.json`;
    by("[data-competition-recovery]").hidden = false;
    setText("[data-public-commitment]", commitment);
    updateButtons();
    output("Recovery envelope generated locally. Download and back it up before preparing the entry.", "ready");
  }

  async function prepareCommit(event) {
    event.preventDefault();
    if (!state.envelope) throw new Error("Generate and download the recovery envelope first.");
    const plan = await json(`${state.api}/v1/base/open-competition-v1/commit-preparation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        network,
        bounty_contract: bounty,
        solver: state.account,
        commitment: state.envelope.commitment,
      }),
    });
    output(plan.allowed
      ? "Entry preparation passed. Review the exact bond approval and commitment transaction in your wallet workflow."
      : `Entry blocked: ${plan.blocker || "canonical readiness failed"}`,
    plan.allowed ? "ready" : "error");
  }

  async function prepareReveal(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const file = form.elements.envelope.files[0];
    if (!file) throw new Error("Choose the recovery envelope file.");
    const envelope = JSON.parse(await file.text());
    const plan = await json(`${state.api}/v1/base/open-competition-v1/reveal-preparation`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        network,
        bounty_contract: bounty,
        solver: state.account,
        commitment_envelope: envelope,
        proof: form.elements.proof.value,
      }),
    });
    output(plan.allowed
      ? "Reveal preparation passed. Review the exact reveal calldata before signing."
      : `Reveal blocked: ${plan.blocker || "canonical readiness failed"}`,
    plan.allowed ? "ready" : "error");
  }

  function guard(handler) {
    return async (event) => {
      try {
        await handler(event);
      } catch (error) {
        output(error.message || "Competition action failed.", "error");
      }
    };
  }

  async function initialize() {
    by("[data-connect-competition-wallet]").addEventListener("click", guard(connect));
    by("[data-generate-commitment]").addEventListener("click", guard(createEnvelope));
    by("[data-competition-commit-form]").addEventListener("submit", guard(prepareCommit));
    by("[data-competition-reveal-form]").addEventListener("submit", guard(prepareReveal));
    if (!/^base-(?:mainnet|sepolia)$/.test(network)) throw new Error("Unsupported Base network.");
    await loadApi();
    await loadCatalog();
    await loadCanonicalState();
  }

  initialize().catch((error) => {
    const stage = by("[data-competition-stage]");
    stage.textContent = error.message || "Competition state is unavailable.";
    stage.dataset.tone = "blocked";
    output("New entry preparation is disabled until canonical state is available.", "error");
  });
})();
