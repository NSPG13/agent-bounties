(() => {
  "use strict";

  const FALLBACK_POLICY = Object.freeze({
    schema_version: "agent-bounties/legal-policy-v1",
    terms_version: "2026-07-18",
    privacy_version: "2026-07-18",
    statement: "I meet the age requirement in the Terms and am authorized to use this wallet and perform this action. I understand that public and blockchain records may be permanent. I accept the posted task, verification, and settlement rules. I am responsible for legal compliance, taxes, content rights, agent authority, and wallet security. I agree to the Terms of Use and Privacy Policy.",
    terms_url: "terms.html",
    privacy_url: "privacy.html",
    supported_actions: [
      "post_bounty",
      "fund_bounty",
      "claim_bounty",
      "submit_result",
      "recover_funds",
      "activate_agent_budget",
      "update_agent_policy",
      "revoke_agent_policy",
    ],
  });

  const actionLabels = Object.freeze({
    post_bounty: "post this bounty",
    fund_bounty: "fund this bounty",
    claim_bounty: "claim this bounty",
    submit_result: "submit this result",
    recover_funds: "recover these funds",
    activate_agent_budget: "activate this agent budget",
    update_agent_policy: "update this agent policy",
    revoke_agent_policy: "revoke this agent policy",
  });

  const receipts = new Map();
  let policyPromise = null;
  let latestReceipt = null;

  async function sha256(value) {
    const bytes = new TextEncoder().encode(value);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return `sha256:${Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  async function loadPolicy() {
    if (policyPromise) return policyPromise;
    policyPromise = (async () => {
      try {
        const protocolResponse = await fetch("protocol.json", { cache: "no-store" });
        if (!protocolResponse.ok) throw new Error("Protocol configuration unavailable.");
        const protocol = await protocolResponse.json();
        const api = String(protocol.api_base_url || "").replace(/\/$/, "");
        const response = await fetch(`${api}/v1/legal/policy`, { cache: "no-store" });
        if (!response.ok) throw new Error(`Legal policy unavailable (${response.status}).`);
        const policy = await response.json();
        const expectedHash = await sha256(policy.statement);
        if (policy.schema_version !== FALLBACK_POLICY.schema_version
          || policy.statement_hash !== expectedHash
          || !Array.isArray(policy.supported_actions)) {
          throw new Error("The hosted legal policy failed integrity checks.");
        }
        return { ...policy, api, source: "hosted" };
      } catch (_error) {
        return {
          ...FALLBACK_POLICY,
          statement_hash: await sha256(FALLBACK_POLICY.statement),
          api: null,
          source: "bundled",
        };
      }
    })();
    return policyPromise;
  }

  function actionsFor(root) {
    return String(root.dataset.consentActions || "")
      .split(/\s+/)
      .map((value) => value.trim())
      .filter(Boolean);
  }

  function findRoot(action, scope = document) {
    const roots = [
      ...(scope.querySelectorAll ? scope.querySelectorAll("[data-legal-consent]") : []),
      ...document.querySelectorAll("[data-legal-consent]"),
    ];
    return roots.find((root, index) => roots.indexOf(root) === index && actionsFor(root).includes(action));
  }

  function createElement(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text) element.textContent = text;
    return element;
  }

  function renderRoot(root) {
    if (root.dataset.consentReady === "true") return;
    root.dataset.consentReady = "true";
    root.classList.add("legal-consent");
    const actions = actionsFor(root);
    const actionText = actions.length === 1 ? actionLabels[actions[0]] : "use this wallet action";

    const heading = createElement("div", "legal-consent-heading");
    heading.append(
      createElement("span", "legal-consent-step", "Before you continue"),
      createElement("h3", "", `Know what happens when you ${actionText}`),
    );

    const points = createElement("ul", "legal-consent-points");
    for (const text of [
      "Money: check the Base network, USDC amount, and destination in your wallet before approving.",
      "Public record: your wallet, bounty, evidence, and blockchain activity may be public and permanent.",
      "Payment: only the posted verifier and a confirmed BountySettled event prove that work was paid.",
      "Security: we never need your recovery phrase or private key.",
    ]) {
      points.append(createElement("li", "", text));
    }

    const label = createElement("label", "legal-consent-check");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.required = true;
    checkbox.dataset.legalConsentCheckbox = "";
    const agreement = createElement("span", "");
    agreement.append("I understand the points above. I am authorized to act and agree to the ");
    const terms = createElement("a", "", "Terms of Use");
    terms.href = "terms.html";
    terms.target = "_blank";
    terms.rel = "noopener";
    const privacy = createElement("a", "", "Privacy Policy");
    privacy.href = "privacy.html";
    privacy.target = "_blank";
    privacy.rel = "noopener";
    agreement.append(terms, " and ", privacy, ".");
    label.append(checkbox, agreement);

    const note = createElement(
      "p",
      "fine legal-consent-note",
      "The wallet prompt is your final review. Cancel it if the amount, network, or destination is wrong.",
    );
    const status = createElement("output", "fine legal-consent-status", "Agreement not yet accepted.");
    status.setAttribute("aria-live", "polite");
    status.dataset.legalConsentStatus = "";
    root.append(heading, points, label, note, status);

    checkbox.addEventListener("change", () => {
      if (!checkbox.checked) {
        for (const key of receipts.keys()) {
          if (actions.some((action) => key.endsWith(`:${action}`))) receipts.delete(key);
        }
        status.textContent = "Agreement not yet accepted.";
        status.dataset.tone = "";
      } else {
        status.textContent = "Ready. Connect the wallet and review its exact prompt.";
        status.dataset.tone = "pending";
      }
    });
  }

  function renderAll() {
    document.querySelectorAll("[data-legal-consent]").forEach(renderRoot);
  }

  async function recordAcceptance(policy, action, walletAddress) {
    const acceptedAt = new Date().toISOString();
    const payload = {
      terms_version: policy.terms_version,
      privacy_version: policy.privacy_version,
      action,
      wallet_address: walletAddress,
      statement_hash: policy.statement_hash,
      acceptance_method: "web_clickwrap",
      accepted_at: acceptedAt,
    };
    if (policy.api) {
      try {
        const response = await fetch(`${policy.api}/v1/legal/acceptances`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(payload),
        });
        if (response.ok) return { ...(await response.json()), durable: true };
        if (response.status === 400) {
          throw new Error("The legal policy changed. Reload this page and review it again.");
        }
      } catch (error) {
        if (error.message && error.message.includes("policy changed")) throw error;
      }
    }
    return {
      schema_version: "agent-bounties/legal-acceptance-v1",
      acceptance_id: `session:${crypto.randomUUID ? crypto.randomUUID() : Date.now()}`,
      ...payload,
      recorded_at: acceptedAt,
      durable: false,
    };
  }

  async function requireAcceptance({ action, walletAddress, scope = document }) {
    if (!actionLabels[action]) throw new Error(`Unsupported wallet action: ${action}.`);
    if (!/^0x[0-9a-fA-F]{40}$/.test(String(walletAddress || ""))) {
      throw new Error("Connect a valid Base wallet before accepting the terms.");
    }
    const root = findRoot(action, scope);
    if (!root) throw new Error("The required legal agreement is missing from this page.");
    renderRoot(root);
    const checkbox = root.querySelector("[data-legal-consent-checkbox]");
    const status = root.querySelector("[data-legal-consent-status]");
    if (!checkbox.checked) {
      status.textContent = "Check the agreement before the wallet is asked to sign.";
      status.dataset.tone = "error";
      root.scrollIntoView({ behavior: "smooth", block: "center" });
      checkbox.focus({ preventScroll: true });
      throw new Error("Read and accept the Terms and Privacy Policy before continuing.");
    }
    const policy = await loadPolicy();
    if (!policy.supported_actions.includes(action)) {
      throw new Error("This wallet action is not covered by the current hosted legal policy.");
    }
    const key = `${policy.terms_version}:${walletAddress.toLowerCase()}:${action}`;
    let receipt = receipts.get(key);
    if (!receipt) {
      receipt = await recordAcceptance(policy, action, walletAddress.toLowerCase());
      receipts.set(key, receipt);
    }
    latestReceipt = receipt;
    status.textContent = receipt.durable
      ? `Agreement recorded for ${actionLabels[action]}. Review the wallet prompt now.`
      : `Agreement accepted for ${actionLabels[action]}. Review the wallet prompt now.`;
    status.dataset.tone = "success";
    return receipt;
  }

  window.AgentBountiesLegal = Object.freeze({
    loadPolicy,
    requireAcceptance,
    latestReceipt: () => latestReceipt,
  });

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", renderAll);
  } else {
    renderAll();
  }
})();
