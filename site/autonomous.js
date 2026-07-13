(() => {
  "use strict";

  const state = {
    protocol: null,
    account: null,
    provider: null,
    providers: [],
  };

  const announcedProviders = [];

  const byId = (id) => document.getElementById(id);
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  function isWalletProvider(provider) {
    return Boolean(provider && typeof provider.request === "function");
  }

  function providerName(provider, info = {}) {
    if (info.name) return info.name;
    if (provider.isMetaMask) return "MetaMask";
    if (provider.isCoinbaseWallet) return "Coinbase Wallet";
    if (provider.isBraveWallet) return "Brave Wallet";
    return "Browser wallet";
  }

  function rememberProvider(event) {
    const detail = event && event.detail;
    if (!detail || !isWalletProvider(detail.provider)) return;
    if (!announcedProviders.some((item) => item.provider === detail.provider)) {
      announcedProviders.push(detail);
    }
  }

  window.addEventListener("eip6963:announceProvider", rememberProvider);

  function populateProviderSelectors() {
    document.querySelectorAll("[data-wallet-provider]").forEach((selector) => {
      const selectedProvider = state.provider;
      selector.replaceChildren(...state.providers.map((item, index) => {
        const option = document.createElement("option");
        option.value = String(index);
        option.textContent = providerName(item.provider, item.info);
        option.selected = item.provider === selectedProvider;
        return option;
      }));
      selector.disabled = state.providers.length === 0;
      if (state.providers.length === 0) {
        const option = document.createElement("option");
        option.textContent = "No browser wallet detected";
        selector.append(option);
      }
    });
  }

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(250);
    const candidates = [...announcedProviders];
    const injected = window.ethereum && Array.isArray(window.ethereum.providers)
      ? window.ethereum.providers
      : (window.ethereum ? [window.ethereum] : []);
    for (const provider of injected) {
      if (isWalletProvider(provider) && !candidates.some((item) => item.provider === provider)) {
        candidates.push({ provider, info: {} });
      }
    }
    state.providers = candidates;
    populateProviderSelectors();
    return state.providers;
  }

  function selectProvider(context = document) {
    const selector = (context.querySelector && context.querySelector("[data-wallet-provider]"))
      || document.querySelector("[data-wallet-provider]");
    const item = state.providers[Number.parseInt(selector && selector.value, 10)];
    if (!item) throw new Error("Unlock a browser wallet, reload, and select it here.");
    state.provider = item.provider;
    const index = String(state.providers.findIndex((provider) => provider.provider === item.provider));
    document.querySelectorAll("[data-wallet-provider]").forEach((candidate) => {
      candidate.value = index;
    });
    return item.provider;
  }

  function walletRequest(method, params = []) {
    const provider = state.provider || selectProvider();
    return provider.request({ method, params });
  }

  async function loadProtocol() {
    if (state.protocol) return state.protocol;
    const response = await fetch("protocol.json", { cache: "no-store" });
    if (!response.ok) throw new Error("Protocol configuration is unavailable.");
    state.protocol = await response.json();
    return state.protocol;
  }

  function requireActiveProtocol(protocol) {
    const address = /^0x[0-9a-fA-F]{40}$/;
    if (
      protocol.status !== "active" ||
      !address.test(protocol.factory || "") ||
      !address.test(protocol.implementation || "")
    ) {
      throw new Error("The autonomous protocol is pending review and deployment. No transaction was requested.");
    }
    return protocol;
  }

  function apiBase() {
    return state.protocol.api_base_url.replace(/\/$/, "");
  }

  async function requestJson(url, options = {}) {
    const response = await fetch(url, {
      ...options,
      headers: {
        "content-type": "application/json",
        ...(options.headers || {}),
      },
    });
    const text = await response.text();
    let body = null;
    if (text) {
      try {
        body = JSON.parse(text);
      } catch (_error) {
        body = text;
      }
    }
    if (!response.ok) {
      if (response.status === 503) {
        throw new Error("The autonomous protocol is not deployed on this network yet.");
      }
      throw new Error(
        typeof body === "string" ? body : body && body.error ? body.error : `Request failed (${response.status}).`,
      );
    }
    return body;
  }

  function output(element, lines, tone = "") {
    if (!element) return;
    element.textContent = Array.isArray(lines) ? lines.join("\n") : lines;
    element.dataset.tone = tone;
  }

  function randomBytes32() {
    const bytes = new Uint8Array(32);
    crypto.getRandomValues(bytes);
    return `0x${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function usdcMinor(value) {
    const amount = Number(value);
    if (!Number.isFinite(amount) || amount < 0 || amount > 9_000_000_000) {
      throw new Error("Enter a valid USDC amount.");
    }
    return Math.round(amount * 1_000_000);
  }

  function requiredAddress(value, label) {
    const address = value.trim();
    if (!/^0x[0-9a-fA-F]{40}$/.test(address)) throw new Error(`${label} must be an EVM address.`);
    return address;
  }

  function optionalAddress(value) {
    const address = value.trim();
    return address ? requiredAddress(address, "Address") : null;
  }

  function defaultVerification(protocol) {
    const config = protocol.default_verification;
    if (!config || config.mode !== "deterministic_module" || config.threshold !== 1) {
      throw new Error("The active protocol does not declare a safe deterministic default.");
    }
    const module = protocol.deterministic_modules && protocol.deterministic_modules[config.module_id];
    if (!module) throw new Error("The default deterministic verifier is unavailable.");
    return {
      ...config,
      contract: requiredAddress(module.contract || "", "Deterministic verifier module"),
    };
  }

  function configurePostVerification(form, protocol, account = null) {
    if (!form) return;
    const defaults = defaultVerification(protocol);
    const mode = form.elements.verificationMode.value;
    const deterministic = mode === "deterministic_module";
    const module = form.elements.verifierModule;
    const recipient = form.elements.verifierRewardRecipient;
    const verifiers = form.elements.verifiers;
    const threshold = form.elements.threshold;

    module.value = defaults.contract;
    module.readOnly = true;
    module.disabled = !deterministic;
    recipient.disabled = !deterministic;
    verifiers.disabled = deterministic;
    threshold.readOnly = deterministic;
    if (deterministic) {
      threshold.value = String(defaults.threshold);
      if (defaults.verifier_reward_recipient === "creator_wallet" && account && !recipient.value.trim()) {
        recipient.value = account;
      }
    }
  }

  function parseJson(value, label) {
    try {
      return JSON.parse(value);
    } catch (_error) {
      throw new Error(`${label} must be valid JSON.`);
    }
  }

  function splitLines(value) {
    return value
      .split(/\r?\n/)
      .map((line) => line.trim().replace(/^[-*]\s*/, ""))
      .filter(Boolean);
  }

  function splitAddresses(value) {
    return value
      .split(/[\s,]+/)
      .map((item) => item.trim())
      .filter(Boolean)
      .map((item) => requiredAddress(item, "Verifier"));
  }

  async function sha256Hex(value) {
    const bytes = new TextEncoder().encode(value);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return `0x${Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function canonicalJsonValue(value) {
    if (Array.isArray(value)) return value.map(canonicalJsonValue);
    if (value && typeof value === "object") {
      return Object.keys(value)
        .sort()
        .reduce((result, key) => {
          result[key] = canonicalJsonValue(value[key]);
          return result;
        }, {});
    }
    return value;
  }

  function canonicalJsonString(value) {
    return JSON.stringify(canonicalJsonValue(value));
  }

  async function connectWallet(context = document) {
    await discoverProviders();
    selectProvider(context);
    const protocol = await loadProtocol();
    const accounts = await walletRequest("eth_requestAccounts");
    if (!accounts || !accounts[0]) throw new Error("No wallet account was returned.");
    state.account = accounts[0];
    configurePostVerification(
      context.querySelector && context.querySelector("#autonomous-post-form")
        ? context.querySelector("#autonomous-post-form")
        : (context.id === "autonomous-post-form" ? context : byId("autonomous-post-form")),
      protocol,
      state.account,
    );
    const current = await walletRequest("eth_chainId");
    if (String(current).toLowerCase() !== protocol.chain_id_hex.toLowerCase()) {
      try {
        await walletRequest("wallet_switchEthereumChain", [{ chainId: protocol.chain_id_hex }]);
      } catch (error) {
        if (error && error.code === 4902) {
          await walletRequest("wallet_addEthereumChain", [
            {
              chainId: protocol.chain_id_hex,
              chainName: "Base",
              nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
              rpcUrls: ["https://mainnet.base.org"],
              blockExplorerUrls: [protocol.explorer_url],
            },
          ]);
        } else {
          throw error;
        }
      }
    }
    return state.account;
  }

  async function isContractAccount(account) {
    const code = await walletRequest("eth_getCode", [account, "latest"]);
    return code && code !== "0x" && code !== "0x0";
  }

  function signatureParts(signature) {
    const value = String(signature).replace(/^0x/, "");
    if (value.length !== 130) throw new Error("Wallet returned an invalid 65-byte signature.");
    return {
      r: `0x${value.slice(0, 64)}`,
      s: `0x${value.slice(64, 128)}`,
      v: Number.parseInt(value.slice(128, 130), 16),
    };
  }

  async function signTypedData(account, typedData) {
    const signature = await walletRequest("eth_signTypedData_v4", [account, JSON.stringify(typedData)]);
    return signatureParts(signature);
  }

  async function sendTransaction(transaction, from) {
    return walletRequest("eth_sendTransaction", [
      {
        from,
        to: transaction.to,
        data: transaction.data,
        value: "0x0",
      },
    ]);
  }

  async function waitReceipt(txHash, timeoutMs = 120_000) {
    const started = Date.now();
    while (Date.now() - started < timeoutMs) {
      const receipt = await walletRequest("eth_getTransactionReceipt", [txHash]);
      if (receipt) {
        if (receipt.status !== "0x1") throw new Error(`Transaction reverted: ${txHash}`);
        return receipt;
      }
      await sleep(1_500);
    }
    throw new Error(`Transaction confirmation timed out: ${txHash}`);
  }

  async function sendWalletCalls(calls, account, protocol) {
    try {
      const bundleId = await walletRequest("wallet_sendCalls", [
        {
          version: "2.0.0",
          chainId: protocol.chain_id_hex,
          from: account,
          calls: calls.map((call) => ({ to: call.to, data: call.data, value: "0x0" })),
        },
      ]);
      return { kind: "bundle", id: bundleId };
    } catch (_error) {
      const hashes = [];
      for (const call of calls) {
        const hash = await sendTransaction(call, account);
        await waitReceipt(hash);
        hashes.push(hash);
      }
      return { kind: "transactions", hashes };
    }
  }

  async function pollEvents(api, bountyId, expectedKinds, timeoutMs = 90_000) {
    const started = Date.now();
    while (Date.now() - started < timeoutMs) {
      const events = await requestJson(
        `${api}/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id=${encodeURIComponent(bountyId)}`,
      );
      if (expectedKinds.every((kind) => events.some((event) => event.kind === kind))) return events;
      await sleep(2_500);
    }
    return null;
  }

  async function canonicalBountyByContract(api, bountyContract) {
    const items = await requestJson(
      `${api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false`,
    );
    const item = items.find((candidate) =>
      candidate.bounty_contract.toLowerCase() === bountyContract.toLowerCase());
    if (!item) throw new Error("This contract is not indexed from the canonical factory.");
    if (!item.terms_valid) {
      throw new Error(`The indexed terms do not match this contract: ${item.validation_errors.join("; ")}`);
    }
    return item;
  }

  async function pollSubmission(api, bountyId, submissionHash, evidenceHash, timeoutMs = 90_000) {
    const started = Date.now();
    while (Date.now() - started < timeoutMs) {
      const events = await requestJson(
        `${api}/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id=${encodeURIComponent(bountyId)}`,
      );
      const submission = events
        .filter((event) => event.kind === "submission_added")
        .reverse()
        .find((event) =>
          String(event.data.submission_hash).toLowerCase() === submissionHash.toLowerCase()
          && String(event.data.evidence_hash).toLowerCase() === evidenceHash.toLowerCase());
      if (submission) return submission;
      await sleep(2_500);
    }
    return null;
  }

  function contractTerms(form, account, protocol) {
    const solverReward = usdcMinor(form.elements.solverReward.value);
    const verifierReward = usdcMinor(form.elements.verifierReward.value);
    const target = solverReward + verifierReward;
    return {
      protocol_version: protocol.protocol_version,
      creator_wallet: account,
      network: protocol.network,
      settlement_token: protocol.native_usdc,
      solver_reward: { amount: solverReward, currency: "usdc" },
      verifier_reward: { amount: verifierReward, currency: "usdc" },
      claim_bond: { amount: verifierReward, currency: "usdc" },
      initial_funding: {
        amount: form.elements.crowdfund.checked ? 0 : target,
        currency: "usdc",
      },
      funding_deadline:
        Math.floor(Date.now() / 1000) + Number(form.elements.fundingDays.value) * 86_400,
      claim_window_seconds: Number(form.elements.claimHours.value) * 3_600,
      verification_window_seconds: Number(form.elements.verificationHours.value) * 3_600,
      creation_nonce: randomBytes32(),
    };
  }

  function postPayload(form, terms, committed) {
    const mode = form.elements.verificationMode.value;
    const verifiers = splitAddresses(form.elements.verifiers.value);
    const threshold = Number(form.elements.threshold.value);
    const module = optionalAddress(form.elements.verifierModule.value);
    const verifierRecipient = optionalAddress(form.elements.verifierRewardRecipient.value);
    if (mode === "deterministic_module" && !module) {
      throw new Error("Deterministic mode requires a verifier module address.");
    }
    if (mode !== "deterministic_module" && verifiers.length === 0) {
      throw new Error("Quorum mode requires verifier wallet addresses.");
    }
    if (mode === "ai_judge_quorum" && threshold < 2) {
      throw new Error("AI judge settlement requires at least two matching verifier signatures.");
    }
    return {
      creator: committed.creator_wallet,
      solver_reward: committed.solver_reward,
      verifier_reward: committed.verifier_reward,
      terms_hash: terms.terms_hash,
      policy_hash: terms.policy_hash,
      acceptance_criteria_hash: terms.acceptance_criteria_hash,
      benchmark_hash: terms.benchmark_hash,
      evidence_schema_hash: terms.evidence_schema_hash,
      funding_deadline: committed.funding_deadline,
      claim_window_seconds: committed.claim_window_seconds,
      verification_window_seconds: committed.verification_window_seconds,
      verification_mode: mode,
      verifier_module: mode === "deterministic_module" ? module : null,
      verifier_reward_recipient: mode === "deterministic_module" ? verifierRecipient : null,
      verifiers: mode === "deterministic_module" ? [] : verifiers,
      threshold,
      initial_funding: committed.initial_funding,
      creation_nonce: committed.creation_nonce,
    };
  }

  function termsDocument(form, committed) {
    const mode = form.elements.verificationMode.value;
    const verifiers = splitAddresses(form.elements.verifiers.value);
    const threshold = Number(form.elements.threshold.value);
    const module = optionalAddress(form.elements.verifierModule.value);
    const verifierRecipient = optionalAddress(form.elements.verifierRewardRecipient.value);
    if (mode === "deterministic_module") {
      if (!module) throw new Error("Deterministic mode requires a verifier module address.");
      if (threshold !== 1) throw new Error("Deterministic mode requires threshold one.");
      if (Number(committed.verifier_reward.amount) > 0 && !verifierRecipient) {
        throw new Error("A paid deterministic verifier requires a reward recipient.");
      }
    } else {
      if (!verifiers.length || threshold < 1 || threshold > verifiers.length) {
        throw new Error("Quorum threshold must fit the verifier wallet set.");
      }
      if (new Set(verifiers.map((address) => address.toLowerCase())).size !== verifiers.length) {
        throw new Error("Verifier wallet addresses must be unique.");
      }
      if (mode === "ai_judge_quorum" && threshold < 2) {
        throw new Error("AI judge settlement requires at least two matching verifier signatures.");
      }
      if (Number(committed.verifier_reward.amount) % threshold !== 0) {
        throw new Error("Verifier reward must divide evenly across the threshold.");
      }
    }
    return {
      schema_version: "agent-bounties/terms-v1",
      contract_terms: committed,
      title: form.elements.title.value.trim(),
      goal: form.elements.goal.value.trim(),
      acceptance_criteria: splitLines(form.elements.acceptance.value),
      benchmark: parseJson(form.elements.benchmark.value, "Benchmark"),
      evidence_schema: parseJson(form.elements.evidenceSchema.value, "Evidence schema"),
      verification_policy: {
        mechanism: mode,
        verifier_module: mode === "deterministic_module" ? module : null,
        verifier_reward_recipient: mode === "deterministic_module" ? verifierRecipient : null,
        verifiers: mode === "deterministic_module" ? [] : verifiers,
        threshold,
        ai_provider: form.elements.aiProvider.value.trim() || null,
        ai_model: form.elements.aiModel.value.trim() || null,
        ai_model_version: form.elements.aiModelVersion.value.trim() || null,
        system_prompt: form.elements.systemPrompt.value.trim() || null,
        rubric: form.elements.rubric.value.trim() || null,
        decoding_parameters: parseJson(form.elements.decodingParameters.value, "Decoding parameters"),
      },
      source_url: form.elements.sourceUrl.value.trim() || null,
      discovery_source: form.elements.discoverySource.value.trim() || null,
    };
  }

  async function postBounty(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const result = byId("autonomous-post-output");
    try {
      const protocol = requireActiveProtocol(await loadProtocol());
      const account = await connectWallet(form);
      const api = apiBase(form);
      output(result, ["Publishing content-addressed terms...", `Creator: ${account}`]);
      const committed = contractTerms(form, account, protocol);
      const document = termsDocument(form, committed);
      const terms = await requestJson(`${api}/v1/base/autonomous-bounties/terms`, {
        method: "POST",
        body: JSON.stringify({ creator_wallet: account, document }),
      });
      const create = postPayload(form, terms, committed);
      const plan = await requestJson(`${api}/v1/base/autonomous-bounties/creation-plan`, {
        method: "POST",
        body: JSON.stringify({ network: "base-mainnet", create }),
      });
      output(result, [
        "Wallet confirmation required.",
        `Bounty: ${plan.predicted_bounty_contract}`,
        `Target: ${(Number(create.solver_reward.amount) + Number(create.verifier_reward.amount)) / 1_000_000} USDC`,
      ]);

      let txHash = null;
      if (Number(create.initial_funding.amount) === 0) {
        txHash = await sendTransaction(plan.create_bounty, account);
        await waitReceipt(txHash);
      } else if (!(await isContractAccount(account)) && plan.eip3009_authorization) {
        const signature = await signTypedData(account, plan.eip3009_authorization);
        const authorized = await requestJson(
          `${api}/v1/base/autonomous-bounties/authorized-creation-plan`,
          {
            method: "POST",
            body: JSON.stringify({
              network: "base-mainnet",
              create,
              signature,
              relayer: account,
            }),
          },
        );
        txHash = await sendTransaction(authorized.relay_transaction, account);
        await waitReceipt(txHash);
      } else {
        const sent = await sendWalletCalls(plan.wallet_calls, account, protocol);
        if (sent.kind === "transactions") txHash = sent.hashes[sent.hashes.length - 1];
      }

      output(result, [
        "Transaction confirmed. Waiting for indexed protocol evidence...",
        `Bounty id: ${plan.bounty_id}`,
        `Contract: ${plan.predicted_bounty_contract}`,
        txHash ? `Transaction: ${protocol.explorer_url}/tx/${txHash}` : "Wallet batch submitted.",
      ]);
      const expected = ["canonical_bounty_created"];
      if (Number(create.initial_funding.amount) === Number(create.solver_reward.amount) + Number(create.verifier_reward.amount)) {
        expected.push("bounty_became_claimable");
      }
      const events = await pollEvents(api, plan.bounty_id, expected);
      if (!events) {
        output(result, [
          "Transaction confirmed; indexer evidence is still pending.",
          `Bounty id: ${plan.bounty_id}`,
          "Do not describe it as funded until FundingAdded and BountyBecameClaimable appear.",
        ], "pending");
        return;
      }
      const claimable = events.some((item) => item.kind === "bounty_became_claimable");
      output(result, [
        claimable ? "Bounty is funded and claimable." : "Bounty contract is canonical and open for co-funding.",
        `Bounty id: ${plan.bounty_id}`,
        `Contract: ${plan.predicted_bounty_contract}`,
        "Default next step: Post your own bounty or share this one with solvers and funders.",
      ], "success");
    } catch (error) {
      output(result, error.message || String(error), "error");
    }
  }

  async function fundBounty(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const result = byId("autonomous-fund-output");
    try {
      const protocol = requireActiveProtocol(await loadProtocol());
      const account = await connectWallet(form);
      const api = apiBase(form);
      const contribution = {
        bounty_contract: requiredAddress(form.elements.bountyContract.value, "Bounty contract"),
        contributor: account,
        amount: { amount: usdcMinor(form.elements.amount.value), currency: "usdc" },
        authorization_nonce: randomBytes32(),
        authorization_valid_before: Math.floor(Date.now() / 1000) + 3_600,
      };
      const plan = await requestJson(`${api}/v1/base/autonomous-bounties/contribution-plan`, {
        method: "POST",
        body: JSON.stringify({ network: "base-mainnet", contribution }),
      });
      output(result, ["Wallet confirmation required.", `Contribution: ${form.elements.amount.value} USDC`]);
      let txHash = null;
      if (!(await isContractAccount(account)) && plan.eip3009_authorization) {
        const signature = await signTypedData(account, plan.eip3009_authorization);
        const authorized = await requestJson(
          `${api}/v1/base/autonomous-bounties/authorized-contribution-plan`,
          {
            method: "POST",
            body: JSON.stringify({ network: "base-mainnet", contribution, signature, relayer: account }),
          },
        );
        txHash = await sendTransaction(authorized.relay_transaction, account);
        await waitReceipt(txHash);
      } else {
        const sent = await sendWalletCalls(plan.wallet_calls, account, protocol);
        if (sent.kind === "transactions") txHash = sent.hashes[sent.hashes.length - 1];
      }
      output(result, [
        "Transaction confirmed. Funding evidence is waiting for the indexer.",
        txHash ? `${protocol.explorer_url}/tx/${txHash}` : "Wallet batch submitted.",
        "A transaction hash alone is not funding evidence.",
      ], "pending");
    } catch (error) {
      output(result, error.message || String(error), "error");
    }
  }

  async function submitBounty(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const result = byId("autonomous-submit-output");
    try {
      const protocol = requireActiveProtocol(await loadProtocol());
      const account = await connectWallet(form);
      const api = apiBase(form);
      const bountyContract = requiredAddress(form.elements.bountyContract.value, "Bounty contract");
      const artifact = form.elements.artifact.value.trim();
      const evidenceValue = parseJson(form.elements.evidence.value, "Evidence package");
      const evidence = canonicalJsonString(evidenceValue);
      if (!artifact) throw new Error("Artifact reference is required.");
      const bounty = await canonicalBountyByContract(api, bountyContract);
      if (bounty.status !== "claimed") throw new Error("This bounty is not currently claimed.");
      const submissionHash = await sha256Hex(artifact);
      const evidenceHash = await sha256Hex(evidence);
      output(result, [
        "Wallet confirmation required.",
        `Artifact SHA-256: ${submissionHash}`,
        `Evidence SHA-256: ${evidenceHash}`,
      ]);
      const plan = await requestJson(`${api}/v1/base/autonomous-bounties/submission-plan`, {
        method: "POST",
        body: JSON.stringify({
          network: "base-mainnet",
          bounty_contract: bountyContract,
          solver: account,
          submission_hash: submissionHash,
          evidence_hash: evidenceHash,
        }),
      });
      const hash = await sendTransaction(plan, account);
      await waitReceipt(hash);
      const submission = await pollSubmission(api, bounty.bounty_id, submissionHash, evidenceHash);
      if (!submission) {
        output(result, [
          "Submission transaction confirmed; indexed evidence is still pending.",
          `Transaction: ${protocol.explorer_url}/tx/${hash}`,
          "Keep the exact artifact and evidence package so their public preimages can be published after indexing.",
        ], "pending");
        return;
      }
      await requestJson(`${api}/v1/base/autonomous-bounties/submission-evidence`, {
        method: "POST",
        body: JSON.stringify({
          network: "base-mainnet",
          bounty_contract: bountyContract,
          bounty_id: bounty.bounty_id,
          round: Number(submission.data.round),
          solver_wallet: account,
          artifact_reference: artifact,
          evidence: evidenceValue,
        }),
      });
      output(result, [
        "Submission and public evidence are indexed.",
        `Transaction: ${protocol.explorer_url}/tx/${hash}`,
        `Round: ${submission.data.round}`,
        "Committed verifier agents can now evaluate and settle automatically.",
        "Only a confirmed BountySettled event proves payout.",
      ], "pending");
    } catch (error) {
      output(result, error.message || String(error), "error");
    }
  }

  function bountyRow(item, api) {
    const article = document.createElement("article");
    article.className = "bounty-row";
    const heading = document.createElement("h3");
    heading.textContent = item.terms ? item.terms.document.title : item.bounty_id;
    const detail = document.createElement("p");
    detail.textContent = `${(Number(item.solver_reward) + Number(item.timeout_bond_pool)) / 1_000_000} USDC current solver payout | ${Number(item.claim_bond) / 1_000_000} USDC solver bond | ${item.status}`;
    const goal = document.createElement("p");
    goal.className = "fine";
    goal.textContent = item.terms ? item.terms.document.goal : "Public terms are not available yet.";
    const actions = document.createElement("div");
    actions.className = "actions";
    const claim = document.createElement("button");
    claim.className = "button primary";
    claim.type = "button";
    claim.textContent = "Claim bounty";
    claim.disabled =
      state.protocol.status !== "active" || item.status !== "claimable" || !item.terms || !item.terms_valid;
    claim.addEventListener("click", async () => {
      const result = byId("claim-feed-output");
      try {
        const protocol = requireActiveProtocol(await loadProtocol());
        const account = await connectWallet(document);
        const authorizationNonce = randomBytes32();
        const authorizationValidBefore = Math.floor(Date.now() / 1000) + 3_600;
        const plan = await requestJson(`${api}/v1/base/autonomous-bounties/claim-plan`, {
          method: "POST",
          body: JSON.stringify({
            network: "base-mainnet",
            bounty_contract: item.bounty_contract,
            solver: account,
            authorization_nonce: authorizationNonce,
            authorization_valid_before: authorizationValidBefore,
          }),
        });
        output(result, [
          "Wallet confirmation required.",
          `Refundable solver bond: ${Number(plan.claim_bond) / 1_000_000} USDC`,
          "Acceptance or verifier timeout returns the bond. Rejection pays verifiers; no-submission timeout forfeits it into the completion bonus.",
        ]);
        let hash = null;
        if (!(await isContractAccount(account)) && plan.eip3009_authorization) {
          const signature = await signTypedData(account, plan.eip3009_authorization);
          const authorized = await requestJson(
            `${api}/v1/base/autonomous-bounties/authorized-claim-plan`,
            {
              method: "POST",
              body: JSON.stringify({
                network: "base-mainnet",
                bounty_contract: item.bounty_contract,
                solver: account,
                authorization_nonce: authorizationNonce,
                authorization_valid_before: authorizationValidBefore,
                signature,
                relayer: account,
              }),
            },
          );
          hash = await sendTransaction(authorized.relay_transaction, account);
          await waitReceipt(hash);
        } else {
          const sent = await sendWalletCalls(plan.wallet_calls, account, protocol);
          if (sent.kind === "transactions") hash = sent.hashes[sent.hashes.length - 1];
        }
        output(result, [
          "Claim transaction confirmed; waiting for BountyClaimed evidence.",
          hash ? `${protocol.explorer_url}/tx/${hash}` : "Wallet batch submitted.",
        ], "pending");
      } catch (error) {
        output(result, error.message || String(error), "error");
      }
    });
    const fund = document.createElement("a");
    fund.className = "button secondary";
    fund.href = `funding.html?bountyContract=${encodeURIComponent(item.bounty_contract)}`;
    fund.textContent = "Add funding";
    actions.append(claim, fund);
    article.append(heading, detail, goal, actions);
    return article;
  }

  async function loadClaimableFeed() {
    const container = byId("claimable-feed");
    if (!container) return;
    try {
      await loadProtocol();
      const api = state.protocol.api_base_url.replace(/\/$/, "");
      const items = await requestJson(
        `${api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=true`,
      );
      container.textContent = "";
      if (!items.length) {
        const empty = document.createElement("p");
        empty.textContent = "No funded bounty is currently claimable.";
        container.append(empty);
        return;
      }
      for (const item of items) container.append(bountyRow(item, api));
    } catch (error) {
      container.textContent = error.message || String(error);
    }
  }

  function prefillFunding() {
    const form = byId("autonomous-fund-form");
    if (!form) return;
    const params = new URLSearchParams(location.search);
    if (params.get("bountyContract")) form.elements.bountyContract.value = params.get("bountyContract");
    if (params.get("amount")) form.elements.amount.value = params.get("amount");
  }

  async function initialize() {
    const postForm = byId("autonomous-post-form");
    try {
      const protocol = await loadProtocol();
      configurePostVerification(postForm, protocol, state.account);
      document.querySelectorAll("[data-protocol-status]").forEach((element) => {
        element.textContent = protocol.status === "active" ? "Base mainnet active" : "Deployment pending review";
        element.dataset.tone = protocol.status === "active" ? "success" : "pending";
      });
      document.querySelectorAll("[data-protocol-action]").forEach((button) => {
        const active = protocol.status === "active";
        button.disabled = !active;
        button.title = active ? "" : "Pending external review and deployment";
      });
    } catch (_error) {
      // Individual actions surface configuration errors.
    }
    if (postForm) {
      postForm.addEventListener("submit", postBounty);
      postForm.elements.verificationMode.addEventListener("change", () => {
        try {
          configurePostVerification(postForm, state.protocol, state.account);
        } catch (error) {
          output(byId("autonomous-post-output"), error.message || String(error), "error");
        }
      });
    }
    const fundForm = byId("autonomous-fund-form");
    if (fundForm) fundForm.addEventListener("submit", fundBounty);
    const submitForm = byId("autonomous-submit-form");
    if (submitForm) submitForm.addEventListener("submit", submitBounty);
    document.querySelectorAll("[data-connect-wallet]").forEach((button) => {
      button.addEventListener("click", async () => {
        const target = byId(button.dataset.output);
        try {
          const account = await connectWallet(button.closest("form") || document);
          output(target, `Connected: ${account}`, "success");
        } catch (error) {
          output(target, error.message || String(error), "error");
        }
      });
    });
    document.querySelectorAll("[data-wallet-provider]").forEach((selector) => {
      selector.addEventListener("change", () => selectProvider(selector.closest("form") || document));
    });
    discoverProviders().catch(() => populateProviderSelectors());
    prefillFunding();
    loadClaimableFeed();
  }

  document.addEventListener("DOMContentLoaded", initialize);
})();
