(function () {
  const paidBountyIssueTemplateUrl = "https://github.com/NSPG13/agent-bounties/issues/new?template=paid-bounty.yml";

  const canvas = document.getElementById("network-canvas");
  if (canvas) {
    const context = canvas.getContext("2d");
    const nodes = Array.from({ length: 44 }, (_, index) => ({
      x: Math.random(),
      y: Math.random(),
      vx: (Math.random() - 0.5) * 0.0007,
      vy: (Math.random() - 0.5) * 0.0007,
      r: index % 7 === 0 ? 2.6 : 1.6,
    }));

    function resize() {
      const scale = window.devicePixelRatio || 1;
      canvas.width = Math.floor(canvas.clientWidth * scale);
      canvas.height = Math.floor(canvas.clientHeight * scale);
      context.setTransform(scale, 0, 0, scale, 0, 0);
    }

    function draw() {
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      context.clearRect(0, 0, width, height);
      context.fillStyle = "#10191f";
      context.fillRect(0, 0, width, height);

      for (const node of nodes) {
        node.x += node.vx;
        node.y += node.vy;
        if (node.x < 0.04 || node.x > 0.96) node.vx *= -1;
        if (node.y < 0.06 || node.y > 0.94) node.vy *= -1;
      }

      for (let i = 0; i < nodes.length; i += 1) {
        for (let j = i + 1; j < nodes.length; j += 1) {
          const a = nodes[i];
          const b = nodes[j];
          const ax = a.x * width;
          const ay = a.y * height;
          const bx = b.x * width;
          const by = b.y * height;
          const distance = Math.hypot(ax - bx, ay - by);
          if (distance < 170) {
            context.strokeStyle = `rgba(141, 224, 203, ${0.2 - distance / 1000})`;
            context.lineWidth = 1;
            context.beginPath();
            context.moveTo(ax, ay);
            context.lineTo(bx, by);
            context.stroke();
          }
        }
      }

      for (const node of nodes) {
        const x = node.x * width;
        const y = node.y * height;
        context.beginPath();
        context.fillStyle = node.r > 2 ? "#f0f4c3" : "#8ee0cb";
        context.arc(x, y, node.r, 0, Math.PI * 2);
        context.fill();
      }

      requestAnimationFrame(draw);
    }

    window.addEventListener("resize", resize);
    resize();
    draw();
  }

  const postForm = document.getElementById("post-bounty-form");
  const postOutput = document.getElementById("post-bounty-output");
  if (postForm && postOutput) {
    function postFormValue(data, name) {
      return String(data.get(name) || "").trim();
    }

    function suggestedFundingCommand(amount, fundingMode) {
      const normalized = amount || "<amount>";
      if (fundingMode === "StripeFiatLedger") {
        return `/agent-bounty fund ${normalized.replace(/USDC/gi, "USD")} via StripeFiatLedger`;
      }
      if (fundingMode === "Simulated") {
        return "Simulated funding is local-only; do not advertise this as real payout.";
      }
      return `/agent-bounty fund ${normalized.replace(/USD(?!C)/gi, "USDC")} via BaseUsdcEscrow`;
    }

    postForm.addEventListener("submit", (event) => {
      event.preventDefault();
      const data = new FormData(postForm);
      const title = postFormValue(data, "postTitle");
      const goal = postFormValue(data, "postGoal");
      const acceptance = postFormValue(data, "postAcceptance");
      const template = postFormValue(data, "postTemplate") || "small-code-change";
      const amount = postFormValue(data, "postAmount");
      const funding = postFormValue(data, "postFunding") || "BaseUsdcEscrow";
      const privacy = postFormValue(data, "postPrivacy") || "Public";
      const cofunding = postFormValue(data, "postCofunding")
        || `Supporters can add funds by commenting \`${suggestedFundingCommand(amount, funding)}\`.`;
      const discovery = postFormValue(data, "postDiscovery");
      const issueTitle = title.startsWith("[bounty]:") ? title : `[bounty]: ${title}`;
      const issueUrl = `${paidBountyIssueTemplateUrl}&title=${encodeURIComponent(issueTitle)}`;
      const issueBody = [
        "### Goal",
        goal,
        "",
        "### Acceptance criteria",
        acceptance,
        "",
        "### Template",
        template,
        "",
        "### Suggested amount",
        amount,
        "",
        "### Funding mode",
        funding,
        "",
        "### Co-funding note",
        cofunding,
        "",
        "### Discovery feedback",
        discovery,
        "",
        "### Privacy",
        privacy,
      ].join("\n");
      const fundingCommand = suggestedFundingCommand(amount, funding);
      const boundary = "Posting this issue is not funding. Real funding still requires verified Stripe webhook reconciliation or indexed Base escrow log reconciliation.";
      postOutput.textContent = [
        `Open the paid-bounty issue template: ${issueUrl}`,
        "",
        "Paste this draft into the issue fields:",
        issueBody,
        "",
        `Suggested co-funding comment after the issue exists: ${fundingCommand}`,
        "",
        boundary,
      ].join("\n");
    });
  }

  const form = document.getElementById("funding-form");
  const output = document.getElementById("funding-output");
  const prefillOutput = document.getElementById("prefill-output");
  const readinessButton = document.getElementById("readiness-button");
  const readinessOutput = document.getElementById("readiness-output");
  const baseForm = document.getElementById("base-plan-form");
  const baseOutput = document.getElementById("base-plan-output");
  const baseWalletForm = document.getElementById("base-wallet-form");
  const baseWalletConnect = document.getElementById("base-wallet-connect");
  const baseWalletOutput = document.getElementById("base-wallet-output");
  const checkoutStatusOutput = document.getElementById("checkout-status-output");
  const checkoutStatusRefresh = document.getElementById("checkout-status-refresh");

  const baseMainnetWalletConfig = {
    network: "base-mainnet",
    chainId: 8453,
    chainIdHex: "0x2105",
    escrowContract: "0x150C6dFbCe7803cc7f634f59b0624e87349CEAce",
    nativeUsdc: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
  };

  function normalizedText(value) {
    return String(value || "").trim();
  }

  function firstCheckoutStatusParam(searchParams, names) {
    for (const name of names) {
      const value = searchParams.get(name);
      if (value && value.trim()) {
        return value.trim();
      }
    }
    return "";
  }

  function displayMoney(money) {
    if (!money || typeof money.amount !== "number") {
      return "unknown";
    }
    const currency = String(money.currency || "").trim().toLowerCase();
    const exponent = {
      usd: 2,
      eur: 2,
      gbp: 2,
      usdc: 6,
    }[currency];
    if (typeof exponent !== "number") {
      return `${money.amount} minor units${currency ? ` ${currency.toUpperCase()}` : ""}`;
    }
    const minor = Math.trunc(money.amount);
    const sign = minor < 0 ? "-" : "";
    const absolute = Math.abs(minor);
    const base = 10 ** exponent;
    const whole = Math.trunc(absolute / base);
    if (exponent === 0) {
      return `${sign}${whole} ${currency.toUpperCase()}`;
    }
    const fractional = String(absolute % base).padStart(exponent, "0");
    return `${sign}${whole}.${fractional} ${currency.toUpperCase()}`;
  }

  function normalizeAddress(value) {
    const normalized = normalizedText(value).toLowerCase();
    return /^0x[0-9a-f]{40}$/.test(normalized) ? normalized : "";
  }

  function sameAddress(left, right) {
    const leftAddress = normalizeAddress(left);
    return leftAddress !== "" && leftAddress === normalizeAddress(right);
  }

  function normalizeChainId(value) {
    if (typeof value === "number") {
      return value;
    }
    const text = normalizedText(value).toLowerCase();
    if (text.startsWith("0x")) {
      return Number.parseInt(text, 16);
    }
    return Number.parseInt(text, 10);
  }

  function sameMoney(left, right) {
    return left
      && right
      && left.amount === right.amount
      && normalizedText(left.currency).toLowerCase() === normalizedText(right.currency).toLowerCase();
  }

  function baseFundingTargetAmount(bounty) {
    const targets = Array.isArray(bounty && bounty.funding_targets)
      ? bounty.funding_targets
      : [];
    const baseTarget = targets.find((target) => target && target.rail === "BaseUsdc");
    return baseTarget && baseTarget.amount ? baseTarget.amount : bounty && bounty.amount;
  }

  function quantityHex(value) {
    if (typeof value === "string" && value.startsWith("0x")) {
      return value;
    }
    const numeric = typeof value === "bigint" ? value : BigInt(value || 0);
    return `0x${numeric.toString(16)}`;
  }

  function evmTransactionRequest(intent, fallbackFrom) {
    return {
      from: intent.from || fallbackFrom,
      to: intent.to,
      value: quantityHex(intent.value_wei || 0),
      data: intent.data,
    };
  }

  function baseWalletPlanValidationIssues(plan, context) {
    const issues = [];
    const connectedAddress = context && context.connectedAddress;
    const expectedBountyId = context && context.bountyId;
    const hostedBounty = context && context.hostedBounty;
    const bounty = plan && plan.bounty ? plan.bounty : {};
    const create = plan && plan.create ? plan.create : {};
    const funding = plan && plan.funding ? plan.funding : {};
    const network = plan && plan.network ? plan.network : funding.network || {};
    const approve = funding.approve || {};
    const createEscrow = funding.create_escrow || {};
    const configured = baseMainnetWalletConfig;

    if (normalizeChainId(network.chain_id) !== configured.chainId) {
      issues.push("funding plan is not for Base mainnet chain 8453");
    }
    if (expectedBountyId && bounty.id !== expectedBountyId) {
      issues.push("funding plan bounty id does not match the displayed bounty");
    }
    if (expectedBountyId && create.bounty_id !== expectedBountyId) {
      issues.push("funding plan createEscrow bounty id does not match the displayed bounty");
    }
    if (!sameAddress(connectedAddress, create.payer)) {
      issues.push("funding plan payer does not match the connected wallet");
    }
    if (!sameAddress(connectedAddress, approve.from) || !sameAddress(connectedAddress, createEscrow.from)) {
      issues.push("transaction sender does not match the connected wallet");
    }
    if (!sameAddress(create.token, configured.nativeUsdc) || !sameAddress(approve.to, configured.nativeUsdc)) {
      issues.push("funding plan token is not native USDC on Base mainnet");
    }
    if (!sameAddress(createEscrow.to, configured.escrowContract)) {
      issues.push("funding plan escrow target does not match the verified Base mainnet deployment");
    }
    if (!normalizedText(approve.data) || !normalizedText(createEscrow.data)) {
      issues.push("funding plan is missing approval or escrow calldata");
    }
    if (approve.function !== "approve(address,uint256)") {
      issues.push("approval transaction is not USDC approve(address,uint256)");
    }
    if (createEscrow.function !== "createEscrow(bytes32,address,uint256,bytes32)") {
      issues.push("escrow transaction is not createEscrow(bytes32,address,uint256,bytes32)");
    }
    if (bounty.terms_hash && create.terms_hash && bounty.terms_hash !== create.terms_hash) {
      issues.push("funding plan terms hash does not match the bounty terms hash");
    }
    if (hostedBounty) {
      if (hostedBounty.id && hostedBounty.id !== bounty.id) {
        issues.push("funding plan bounty id does not match hosted status readback");
      }
      if (hostedBounty.terms_hash && create.terms_hash && hostedBounty.terms_hash !== create.terms_hash) {
        issues.push("funding plan terms hash does not match hosted status readback");
      }
      if (!sameMoney(baseFundingTargetAmount(hostedBounty), create.amount)) {
        issues.push("funding plan amount does not match hosted Base funding target");
      }
    }
    if (!sameMoney(baseFundingTargetAmount(bounty), create.amount)) {
      issues.push("funding plan amount does not match the bounty Base funding target");
    }

    return issues;
  }

  function validateBaseWalletPlan(plan, context) {
    const issues = baseWalletPlanValidationIssues(plan, context);
    if (issues.length > 0) {
      throw new Error(`Base wallet funding plan rejected: ${issues.join("; ")}`);
    }
    return plan;
  }

  function baseWalletFundingStatusModel(report) {
    const summary = report && report.funding_summary ? report.funding_summary : {};
    const partitions = Array.isArray(summary.partitions) ? summary.partitions : [];
    const basePartition = partitions.find((partition) => partition && partition.rail === "BaseUsdc");
    const escrows = Array.isArray(report && report.escrows) ? report.escrows : [];
    const escrowCount = basePartition && typeof basePartition.escrow_count === "number"
      ? basePartition.escrow_count
      : escrows.length;
    const baseClaimable = basePartition ? basePartition.claimable === true : summary.claimable === true && escrowCount > 0;
    const reconciled = baseClaimable && escrowCount > 0;
    return {
      basePartition,
      bounty: report && report.bounty ? report.bounty : {},
      escrowCount,
      heading: reconciled ? "funding reconciled" : "waiting for confirmations",
      reconciled,
      summary,
    };
  }

  function baseWalletStatusLines(report) {
    const state = baseWalletFundingStatusModel(report);
    const applied = state.basePartition && state.basePartition.confirmed
      ? state.basePartition.confirmed
      : state.summary.applied;
    const remaining = state.basePartition && state.basePartition.remaining
      ? state.basePartition.remaining
      : state.summary.remaining;
    const lines = [
      `State: ${state.heading}`,
      `Bounty status: ${state.bounty.status || "unknown"}`,
      `Base applied funding: ${displayMoney(applied)}`,
      `Base remaining funding: ${displayMoney(remaining)}`,
      `Indexed Base escrows: ${state.escrowCount}`,
      `Bounty claimable from Base evidence: ${state.reconciled ? "yes" : "no"}`,
    ];
    if (state.reconciled) {
      lines.push("Base funding is reconciled only because hosted status reports matching indexed EscrowCreated evidence.");
      lines.push("Default CTA: Post your own bounty.");
    } else {
      lines.push("Wallet transactions or transaction hashes are not funding evidence. Keep polling hosted status until indexed EscrowCreated evidence is reconciled.");
    }
    return lines.join("\n");
  }

  async function providerRequest(provider, method, params) {
    if (!provider || typeof provider.request !== "function") {
      throw new Error("No injected EVM wallet provider found.");
    }
    return provider.request({ method, params: params || [] });
  }

  async function connectBaseWallet(provider) {
    const accounts = await providerRequest(provider, "eth_requestAccounts");
    const address = Array.isArray(accounts) && accounts.length > 0 ? normalizeAddress(accounts[0]) : "";
    if (!address) {
      throw new Error("Wallet did not return a usable account address.");
    }
    let chainId = normalizedText(await providerRequest(provider, "eth_chainId")).toLowerCase();
    if (chainId !== baseMainnetWalletConfig.chainIdHex) {
      await providerRequest(provider, "wallet_switchEthereumChain", [{ chainId: baseMainnetWalletConfig.chainIdHex }]);
      chainId = normalizedText(await providerRequest(provider, "eth_chainId")).toLowerCase();
    }
    if (chainId !== baseMainnetWalletConfig.chainIdHex) {
      throw new Error("Wallet is not connected to Base mainnet.");
    }
    return { address, chainId };
  }

  async function readHostedBountyStatus(fetchImpl, apiBaseUrl, bountyId) {
    const response = await fetchImpl(`${apiBaseUrl}/v1/bounties/${bountyId}`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) {
      throw new Error(`Hosted bounty status failed with ${response.status}`);
    }
    return response.json();
  }

  async function requestBaseFundingPlan(fetchImpl, apiBaseUrl, request) {
    const response = await fetchImpl(`${apiBaseUrl}/v1/base/funding-plan`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify(request),
    });
    if (!response.ok) {
      throw new Error(`Base funding plan failed with ${response.status}`);
    }
    return response.json();
  }

  async function fundBaseWalletBounty(options) {
    const fetchImpl = options.fetchImpl || window.fetch.bind(window);
    const provider = options.provider;
    const apiBaseUrl = normalizedText(options.apiBaseUrl).replace(/\/+$/, "");
    const bountyId = normalizedText(options.bountyId);
    const connectedAddress = normalizeAddress(options.connectedAddress);
    const onState = typeof options.onState === "function" ? options.onState : () => {};

    if (!apiBaseUrl || !bountyId || !connectedAddress) {
      throw new Error("Hosted API URL, bounty id, and connected wallet are required.");
    }

    const chainId = normalizedText(await providerRequest(provider, "eth_chainId")).toLowerCase();
    if (chainId !== baseMainnetWalletConfig.chainIdHex) {
      throw new Error("Wallet must stay on Base mainnet before funding.");
    }

    onState("Reading hosted bounty status...");
    const statusBefore = await readHostedBountyStatus(fetchImpl, apiBaseUrl, bountyId);
    onState("Requesting Base mainnet funding plan...");
    const plan = await requestBaseFundingPlan(fetchImpl, apiBaseUrl, {
      bounty_id: bountyId,
      escrow_contract: baseMainnetWalletConfig.escrowContract,
      payer: connectedAddress,
      token: baseMainnetWalletConfig.nativeUsdc,
      network: baseMainnetWalletConfig.network,
    });
    validateBaseWalletPlan(plan, {
      bountyId,
      connectedAddress,
      hostedBounty: statusBefore.bounty,
    });

    onState("Request wallet confirmation 1 of 2: USDC approval.");
    const approveHash = await providerRequest(provider, "eth_sendTransaction", [
      evmTransactionRequest(plan.funding.approve, connectedAddress),
    ]);
    onState("Approval submitted. Request wallet confirmation 2 of 2: create escrow.");
    const escrowHash = await providerRequest(provider, "eth_sendTransaction", [
      evmTransactionRequest(plan.funding.create_escrow, connectedAddress),
    ]);

    let receipt = null;
    try {
      receipt = await providerRequest(provider, "eth_getTransactionReceipt", [escrowHash]);
    } catch (_error) {
      receipt = null;
    }
    if (receipt && receipt.status === "0x0") {
      return {
        approveHash,
        escrowHash,
        heading: "needs operator review",
        lines: [
          "State: needs operator review",
          `Approval transaction: ${approveHash}`,
          `Escrow transaction: ${escrowHash}`,
          "The wallet/provider reports the escrow transaction reverted. No retry was attempted.",
        ].join("\n"),
      };
    }

    onState("Escrow submitted. Reading hosted status for indexed EscrowCreated evidence...");
    const statusAfter = await readHostedBountyStatus(fetchImpl, apiBaseUrl, bountyId);
    return {
      approveHash,
      escrowHash,
      heading: baseWalletFundingStatusModel(statusAfter).heading,
      lines: [
        `Approval transaction: ${approveHash}`,
        `Escrow transaction: ${escrowHash}`,
        "",
        baseWalletStatusLines(statusAfter),
      ].join("\n"),
    };
  }

  function stripeFundingIntents(report) {
    return Array.isArray(report.funding_intents)
      ? report.funding_intents.filter((intent) => intent && intent.rail === "StripeFiat")
      : [];
  }

  function matchingStripeFundingIntent(report, lookup) {
    const intents = stripeFundingIntents(report);
    if (lookup.fundingIntentId) {
      return intents.find((intent) => intent.id === lookup.fundingIntentId) || null;
    }
    if (lookup.externalReference) {
      return intents.find((intent) => intent.external_reference === lookup.externalReference) || null;
    }
    return intents.length === 1 ? intents[0] : null;
  }

  function checkoutStatusModel(report, lookup) {
    const bounty = report.bounty || {};
    const summary = report.funding_summary || {};
    const intent = matchingStripeFundingIntent(report, lookup);
    const status = intent && intent.status;
    const claimable = summary.claimable === true;
    const appliedMinor = summary.applied && typeof summary.applied.amount === "number"
      ? summary.applied.amount
      : 0;
    const heading = status === "Applied"
      ? "funding reconciled"
      : status === "Rejected"
        ? "needs operator review"
        : "waiting for webhook";
    return {
      appliedMinor,
      bounty,
      claimable,
      heading,
      intent,
      status,
      summary,
    };
  }

  function checkoutStatusLines(report, lookup) {
    const state = checkoutStatusModel(report, lookup);
    const bounty = state.bounty;
    const summary = state.summary;
    const intent = state.intent;
    const status = state.status;
    const claimable = state.claimable;
    const lines = [
      `State: ${state.heading}`,
      `Bounty status: ${bounty.status || "unknown"}`,
      `Bounty id: ${lookup.bountyId || bounty.id || "unknown"}`,
      `Applied funding: ${displayMoney(summary.applied)}`,
      `Remaining funding: ${displayMoney(summary.remaining)}`,
      `Bounty claimable: ${claimable ? "yes" : "no"}`,
    ];

    if (intent) {
      lines.push(`Funding intent id: ${intent.id}`);
      lines.push(`Funding intent status: ${status || "unknown"}`);
      lines.push(`External reference: ${intent.external_reference || "not set"}`);
    } else {
      lines.push(`Funding intent: not identified${lookup.externalReference ? ` for external reference ${lookup.externalReference}` : ""}`);
    }

    if (status === "Applied") {
      lines.push("Funding is reconciled only because the matching Stripe funding intent reports applied checkout.session.completed webhook evidence.");
      lines.push("Default CTA: Post your own bounty.");
    } else if (status === "Rejected") {
      lines.push("The hosted API rejected this funding intent. Contact the operator with the funding intent id if available.");
    } else if (state.appliedMinor > 0 || claimable) {
      lines.push("Some bounty funding or claimability exists, but this return page still waits for the matching Checkout funding intent to show Applied webhook evidence.");
    } else {
      lines.push("Checkout returned, but funding is still pending until the signed checkout.session.completed webhook is reconciled.");
      lines.push("Refresh this page after a few seconds. If it stays pending, the hosted operator should check webhook delivery.");
    }

    return lines.join("\n");
  }

  function checkoutUnavailableStatusLines(message) {
    return [
      "State: needs operator review",
      message,
      "Hosted API status is unavailable, so this page cannot show funding as reconciled.",
      "Refresh later or ask the operator to inspect webhook delivery for the funding intent.",
    ].join("\n");
  }

  window.AgentBountiesCheckoutStatus = {
    checkoutStatusLines,
    checkoutStatusModel,
    checkoutUnavailableStatusLines,
    displayMoney,
    matchingStripeFundingIntent,
  };

  async function refreshCheckoutStatus() {
    if (!checkoutStatusOutput) return;
    const params = new URLSearchParams(window.location.search);
    const lookup = {
      apiBaseUrl: firstCheckoutStatusParam(params, ["apiBaseUrl", "api_base_url", "api"]),
      bountyId: firstCheckoutStatusParam(params, ["bountyId", "bounty_id"]),
      fundingIntentId: firstCheckoutStatusParam(params, ["fundingIntentId", "funding_intent_id"]),
      externalReference: firstCheckoutStatusParam(params, ["externalReference", "external_reference"]),
    };
    if (!lookup.apiBaseUrl || !lookup.bountyId) {
      checkoutStatusOutput.textContent = [
        "State: Checkout returned",
        "Missing hosted API or bounty id in the return link, so this page cannot verify funding.",
        "A Checkout redirect is not funding evidence. Open the funding page or hosted bounty status to check reconciliation.",
      ].join("\n");
      return;
    }

    const apiBaseUrl = lookup.apiBaseUrl.replace(/\/+$/, "");
    checkoutStatusOutput.textContent = "Checkout returned. Reading hosted bounty status...";
    try {
      const response = await fetch(`${apiBaseUrl}/v1/bounties/${lookup.bountyId}`, {
        headers: { accept: "application/json" },
      });
      if (!response.ok) {
        throw new Error(`Hosted bounty status returned ${response.status}`);
      }
      checkoutStatusOutput.textContent = checkoutStatusLines(await response.json(), lookup);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      checkoutStatusOutput.textContent = checkoutUnavailableStatusLines(message);
    }
  }

  if (checkoutStatusOutput) {
    if (checkoutStatusRefresh) {
      checkoutStatusRefresh.addEventListener("click", refreshCheckoutStatus);
    }
    refreshCheckoutStatus();
  }

  window.AgentBountiesBaseWallet = {
    baseMainnetWalletConfig,
    baseWalletFundingStatusModel,
    baseWalletPlanValidationIssues,
    baseWalletStatusLines,
    connectBaseWallet,
    evmTransactionRequest,
    fundBaseWalletBounty,
    normalizeAddress,
    validateBaseWalletPlan,
  };

  if (!form || !output) return;

  function randomUuid() {
    if (window.crypto && typeof window.crypto.randomUUID === "function") {
      return window.crypto.randomUUID();
    }
    const bytes = new Uint8Array(16);
    window.crypto.getRandomValues(bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"));
    return `${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex.slice(6, 8).join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10).join("")}`;
  }

  function firstQueryParam(searchParams, names) {
    for (const name of names) {
      const value = searchParams.get(name);
      if (value && value.trim()) {
        return value.trim();
      }
    }
    return "";
  }

  function setInputValue(name, value) {
    const field = form.elements.namedItem(name);
    if ((field instanceof HTMLInputElement || field instanceof HTMLSelectElement) && value) {
      field.value = value;
      return true;
    }
    return false;
  }

  function setNamedInputValue(targetForm, name, value) {
    const field = targetForm && targetForm.elements.namedItem(name);
    if (field instanceof HTMLInputElement && value) {
      field.value = value;
      return true;
    }
    return false;
  }

  function normalizePaymentPreference(value) {
    const normalized = String(value || "").trim().toLowerCase();
    if (normalized === "paypal" || normalized === "pay_pal") {
      return "paypal";
    }
    if (normalized === "auto") {
      return "auto";
    }
    return "";
  }

  const organizationField = form.elements.namedItem("organizationId");
  if (organizationField instanceof HTMLInputElement) {
    const storageKey = "agent-bounties-public-funder-id";
    let funderId = window.localStorage.getItem(storageKey);
    if (!funderId) {
      funderId = randomUuid();
      window.localStorage.setItem(storageKey, funderId);
    }
    if (!organizationField.value) {
      organizationField.value = funderId;
    }
  }

  const query = new URLSearchParams(window.location.search);
  const prefillValues = {
    apiBaseUrl: firstQueryParam(query, ["apiBaseUrl", "api_base_url", "api"]),
    bountyId: firstQueryParam(query, ["bountyId", "bounty_id"]),
    organizationId: firstQueryParam(query, ["organizationId", "organization_id", "sourceOrganizationId", "source_organization_id"]),
    amountMinor: firstQueryParam(query, ["amountMinor", "amount_minor"]),
    currency: firstQueryParam(query, ["currency"]),
    externalReference: firstQueryParam(query, ["externalReference", "external_reference"]),
    source: firstQueryParam(query, ["source", "funding_source"]),
    rail: firstQueryParam(query, ["rail"]),
    paymentPreference: normalizePaymentPreference(firstQueryParam(query, ["paymentPreference", "payment_preference", "preferredPaymentMethod", "preferred_payment_method"])),
    network: firstQueryParam(query, ["network", "baseNetwork", "base_network"]),
    escrowContract: firstQueryParam(query, ["escrowContract", "escrow_contract", "baseEscrowContract", "base_escrow_contract"]),
    payer: firstQueryParam(query, ["payer", "basePayer", "base_payer"]),
    token: firstQueryParam(query, ["token", "baseToken", "base_token"]),
  };

  const prefilledFields = [
    setInputValue("apiBaseUrl", prefillValues.apiBaseUrl) && "hosted API",
    setInputValue("bountyId", prefillValues.bountyId) && "bounty id",
    setInputValue("organizationId", prefillValues.organizationId) && "funder ledger id",
    setInputValue("amountMinor", prefillValues.amountMinor) && "amount",
    setInputValue("currency", prefillValues.currency.toLowerCase()) && "currency",
    setInputValue("paymentPreference", prefillValues.paymentPreference) && "checkout preference",
    setInputValue("externalReference", prefillValues.externalReference) && "external reference",
  ].filter(Boolean);
  const fundingSource = prefillValues.source || "funding-page";
  form.dataset.fundingSource = fundingSource;
  form.dataset.fundingRail = prefillValues.rail || "StripeFiat";
  form.dataset.paymentPreference = prefillValues.paymentPreference || "auto";
  if (baseForm) {
    setNamedInputValue(baseForm, "baseApiBaseUrl", prefillValues.apiBaseUrl);
    setNamedInputValue(baseForm, "baseBountyId", prefillValues.bountyId);
    setNamedInputValue(baseForm, "baseNetwork", prefillValues.network);
    setNamedInputValue(baseForm, "baseEscrowContract", prefillValues.escrowContract);
    setNamedInputValue(baseForm, "basePayer", prefillValues.payer);
    setNamedInputValue(baseForm, "baseToken", prefillValues.token);
  }
  if (baseWalletForm) {
    setNamedInputValue(baseWalletForm, "walletApiBaseUrl", prefillValues.apiBaseUrl);
    setNamedInputValue(baseWalletForm, "walletBountyId", prefillValues.bountyId);
  }
  if (prefillOutput) {
    if (prefilledFields.length > 0) {
      const railNotice = prefillValues.rail && prefillValues.rail !== "StripeFiat"
        ? `\nRail warning: this page creates StripeFiat Checkout only; use the API/Base funding plan for ${prefillValues.rail}.`
        : "";
      const preferenceNotice = form.dataset.paymentPreference === "paypal"
        ? "\nPayment preference: PayPal requested. Stripe Checkout may show PayPal only if the hosted Stripe account, location, browser, currency, and payment-method configuration support it."
        : "";
      prefillOutput.textContent = `Prefilled funding request from ${fundingSource}: ${prefilledFields.join(", ")}.${railNotice}${preferenceNotice}\nReview the values and readiness before opening Checkout. Query parameters are UI defaults only; funding still requires a verified Stripe webhook.`;
    } else {
      prefillOutput.textContent = "Open this page from a public bounty funding link to prefill the hosted API, bounty, amount, and source.";
    }
  }

  function configuredLabel(value) {
    return value ? "ready" : "needs setup";
  }

  async function checkHostedHealth(apiBaseUrl) {
    const response = await fetch(`${apiBaseUrl}/health`, {
      headers: { accept: "text/plain" },
    });
    if (!response.ok) {
      throw new Error(`Hosted API health check failed with ${response.status}`);
    }
    const body = (await response.text()).trim();
    if (body !== "ok") {
      throw new Error("Hosted API health check did not return ok");
    }
  }

  function formatReadiness(report) {
    const checks = Array.isArray(report.checks) ? report.checks : [];
    const methodConfig = report.stripe_payment_method_configuration_configured === true;
    const checkoutMethodCheck = checks.find(
      (check) => check && check.name === "Stripe Checkout payment-method configuration",
    );
    const webhookBoundary = Array.isArray(report.evidence_boundaries)
      && report.evidence_boundaries.some((boundary) =>
        String(boundary).includes("checkout.session.completed webhook")
      );

    return [
      "Hosted API health: ok",
      `Network: ${report.network || "unknown"} (${report.network_chain_id || "unknown"})`,
      `Live-money gate: ${configuredLabel(report.live_money_ready === true)}`,
      `Stripe live execution: ${configuredLabel(report.stripe_live_mode_ready === true)}`,
      `Signed webhook evidence: ${configuredLabel(report.stripe_webhook_ready === true)}`,
      `Checkout method configuration: ${configuredLabel(methodConfig)}`,
      `PayPal-capable setup indicator: ${methodConfig ? "configured" : "not configured"}`,
      "PayPal availability is decided inside Stripe Checkout by account eligibility, Dashboard setup, currency, location, browser, and payment-method configuration.",
      `Base mainnet escrow: ${configuredLabel(report.base_mainnet_ready === true)}`,
      `Webhook settlement boundary: ${webhookBoundary ? "present" : "missing"}`,
      checkoutMethodCheck && checkoutMethodCheck.detail
        ? `Method detail: ${checkoutMethodCheck.detail}`
        : "Method detail: no readiness detail returned",
      "This readiness check is informational. Funding still requires Stripe Checkout completion and a verified webhook.",
    ].join("\n");
  }

  if (readinessButton && readinessOutput) {
    readinessButton.addEventListener("click", async () => {
      const apiBaseUrlField = form.elements.namedItem("apiBaseUrl");
      const apiBaseUrl = apiBaseUrlField instanceof HTMLInputElement
        ? apiBaseUrlField.value.replace(/\/+$/, "")
        : "";
      if (!apiBaseUrl) {
        readinessOutput.textContent = "Enter a hosted API base URL before checking readiness.";
        return;
      }

      readinessOutput.textContent = "Checking hosted API health...";
      try {
        await checkHostedHealth(apiBaseUrl);
        readinessOutput.textContent = "Hosted API health is ok. Checking live-money readiness...";
        const response = await fetch(`${apiBaseUrl}/v1/readiness/live-money?network=base-mainnet`, {
          headers: { accept: "application/json" },
        });
        if (!response.ok) {
          throw new Error(`Readiness check failed with ${response.status}`);
        }
        readinessOutput.textContent = formatReadiness(await response.json());
      } catch (error) {
        readinessOutput.textContent = `${error.message}\n\nNo funding intent or Checkout Session was created. Confirm the hosted API URL, CORS settings, /health endpoint, and live-money readiness endpoint.`;
      }
    });
  }

  if (baseForm && baseOutput) {
    baseForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const data = new FormData(baseForm);
      const apiBaseUrl = String(data.get("baseApiBaseUrl") || "").replace(/\/+$/, "");
      const bountyId = String(data.get("baseBountyId") || "").trim();
      const network = String(data.get("baseNetwork") || "base-sepolia").trim();
      const escrowContract = String(data.get("baseEscrowContract") || "").trim();
      const payer = String(data.get("basePayer") || "").trim();
      const token = String(data.get("baseToken") || "").trim();

      baseOutput.textContent = "Checking hosted API health...";
      try {
        await checkHostedHealth(apiBaseUrl);
        baseOutput.textContent = "Hosted API health is ok. Planning unsigned Base funding transactions...";
        const response = await fetch(`${apiBaseUrl}/v1/base/funding-plan`, {
          method: "POST",
          headers: { "content-type": "application/json", accept: "application/json" },
          body: JSON.stringify({
            bounty_id: bountyId,
            escrow_contract: escrowContract,
            payer,
            token,
            network,
          }),
        });
        if (!response.ok) {
          throw new Error(`Base funding plan failed with ${response.status}`);
        }
        const plan = await response.json();
        const summary = {
          network: plan.network && plan.network.name,
          chain_id: plan.network && plan.network.chain_id,
          bounty_id: plan.bounty && plan.bounty.id,
          amount: plan.create && plan.create.amount,
          approve: plan.funding && plan.funding.approve,
          create_escrow: plan.funding && plan.funding.create_escrow,
        };
        baseOutput.textContent = `${JSON.stringify(summary, null, 2)}\n\nSign and broadcast these transactions from the payer wallet outside this site. This plan is not funding; the bounty is funded only after an indexed EscrowCreated log is reconciled.`;
      } catch (error) {
        baseOutput.textContent = `${error.message}\n\nNo transaction was signed or broadcast. Confirm the hosted API URL, bounty id, escrow contract, payer wallet, token, network, and that the bounty is Base USDC funding-ready.`;
      }
    });
  }

  let baseWalletConnection = null;
  if (baseWalletForm && baseWalletOutput) {
    if (baseWalletConnect) {
      baseWalletConnect.addEventListener("click", async () => {
        baseWalletOutput.textContent = "Requesting wallet account and Base mainnet network...";
        try {
          baseWalletConnection = await connectBaseWallet(window.ethereum);
          baseWalletOutput.textContent = [
            "State: wallet connected",
            `Connected address: ${baseWalletConnection.address}`,
            "Network: Base mainnet (8453)",
            "No transaction has been planned, signed, or broadcast.",
          ].join("\n");
        } catch (error) {
          baseWalletConnection = null;
          baseWalletOutput.textContent = `${error.message}\n\nNo transaction was signed. Connect a wallet that supports EIP-1193 and Base mainnet.`;
        }
      });
    }

    baseWalletForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const data = new FormData(baseWalletForm);
      const apiBaseUrl = String(data.get("walletApiBaseUrl") || "").replace(/\/+$/, "");
      const bountyId = String(data.get("walletBountyId") || "").trim();
      if (!baseWalletConnection) {
        baseWalletOutput.textContent = "Connect a Base mainnet wallet before funding. No transaction was signed.";
        return;
      }

      try {
        const result = await fundBaseWalletBounty({
          apiBaseUrl,
          bountyId,
          connectedAddress: baseWalletConnection.address,
          fetchImpl: window.fetch.bind(window),
          provider: window.ethereum,
          onState(message) {
            baseWalletOutput.textContent = message;
          },
        });
        baseWalletOutput.textContent = result.lines;
      } catch (error) {
        baseWalletOutput.textContent = `${error.message}\n\nNo retry loop was started. If a wallet prompt was rejected or a transaction reverted, inspect the wallet and hosted bounty status before trying again. Transaction hashes are not reconciled funding.`;
      }
    });
  }

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    output.textContent = "Creating funding intent...";
    const data = new FormData(form);
    const apiBaseUrl = String(data.get("apiBaseUrl") || "").replace(/\/+$/, "");
    const bountyId = String(data.get("bountyId") || "").trim();
    const organizationId = String(data.get("organizationId") || "").trim();
    const amountMinor = Number(data.get("amountMinor"));
    const currency = String(data.get("currency") || "usd").trim().toLowerCase();
    const source = form.dataset.fundingSource || "funding-page";
    const paymentPreference = normalizePaymentPreference(data.get("paymentPreference")) || "auto";
    form.dataset.paymentPreference = paymentPreference;
    const externalReference =
      String(data.get("externalReference") || "").trim() ||
      `${source}-${paymentPreference === "paypal" ? "paypal-" : ""}checkout-${Date.now()}`;
    const pageBase = `${window.location.origin}${window.location.pathname.replace(/funding\.html$/, "")}`;
    const returnParams = new URLSearchParams({
      apiBaseUrl,
      bountyId,
      source,
      paymentPreference,
      externalReference,
    });
    const returnQuery = `?${returnParams.toString()}`;

    try {
      if (paymentPreference === "paypal") {
        output.textContent = "Creating Stripe Checkout for PayPal-capable funding. Select PayPal inside Stripe Checkout if it appears.";
      }
      const intentResponse = await fetch(`${apiBaseUrl}/v1/bounties/${bountyId}/funding-intents`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          bounty_id: bountyId,
          source_organization_id: organizationId,
          contributor_agent_id: null,
          amount_minor: amountMinor,
          currency,
          rail: "StripeFiat",
          external_reference: externalReference,
          stripe_success_url: `${pageBase}success.html${returnQuery}`,
          stripe_cancel_url: `${pageBase}cancel.html${returnQuery}`,
          base_escrow_contract: null,
          base_payer: null,
          base_token: null,
          base_network: null,
        }),
      });
      if (!intentResponse.ok) {
        throw new Error(`Funding intent failed with ${intentResponse.status}`);
      }
      const intentReport = await intentResponse.json();
      const fundingIntentId = intentReport.intent && intentReport.intent.id;
      if (!fundingIntentId) {
        throw new Error("Funding intent response did not include intent.id");
      }

      output.textContent = "Creating Stripe Checkout session...";
      const checkoutResponse = await fetch(
        `${apiBaseUrl}/v1/stripe/live/funding-intents/${fundingIntentId}/checkout-session`,
        { method: "POST" },
      );
      if (!checkoutResponse.ok) {
        throw new Error(`Checkout creation failed with ${checkoutResponse.status}`);
      }
      const checkout = await checkoutResponse.json();
      if (!checkout.url) {
        throw new Error("Checkout response did not include a Stripe URL");
      }
      output.textContent = `Checkout session created.\n\nOpen Stripe Checkout: ${checkout.url}`;
      window.location.assign(checkout.url);
    } catch (error) {
      output.textContent = `${error.message}\n\nNo payment credentials were collected here. Confirm the hosted API URL, bounty id, organization id, Stripe live settings, and ENABLE_STRIPE_PUBLIC_CHECKOUT=true.`;
    }
  });
})();
