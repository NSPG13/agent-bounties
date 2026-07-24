(() => {
  "use strict";

  const MCP_URL = "https://mcp.agentbounties.app/mcp";
  const PROVIDERS = {
    chatgpt: "https://chatgpt.com/",
    claude: "https://claude.ai/new",
    gemini: "https://gemini.google.com/app",
  };
  const MAX_TOTAL_USDC = 2_000_000;

  const panel = document.querySelector("[data-ai-handoff]");
  const log = document.querySelector("[data-conversation-log]");
  const original = document.querySelector("[data-ai-original]");
  const promptPreview = document.querySelector("[data-ai-prompt]");
  const importInput = document.querySelector("[data-ai-draft-import]");
  const importStatus = document.querySelector("[data-ai-import-status]");
  const composerStatus = document.querySelector("[data-composer-status]");

  if (!panel || !log || !original || !promptPreview || !importInput) return;

  let currentIntent = "";
  let currentContext = null;
  let currentPrompt = "";

  function boundedText(value, label, maximum) {
    const text = String(value || "").trim();
    if (!text) throw new Error(`${label} is required.`);
    if ([...text].length > maximum) throw new Error(`${label} must be ${maximum} characters or fewer.`);
    return text;
  }

  function parseUsdc(value, label) {
    const text = String(value ?? "").trim();
    if (!/^\d+(?:\.\d{1,6})?$/.test(text)) {
      throw new Error(`${label} must be a positive USDC amount with no more than 6 decimal places.`);
    }
    const amount = Number(text);
    if (!Number.isFinite(amount) || amount <= 0 || amount > 1_000_000) {
      throw new Error(`${label} must be greater than 0 and no more than 1,000,000 USDC.`);
    }
    return text;
  }

  function stripCodeFence(value) {
    const text = String(value || "").trim();
    const match = text.match(/^```(?:json)?\s*([\s\S]*?)\s*```$/i);
    return match ? match[1] : text;
  }

  function parseDraft(value) {
    const raw = typeof value === "string" ? JSON.parse(stripCodeFence(value)) : value;
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) throw new Error("The AI response must be one JSON object.");

    const criteria = Array.isArray(raw.acceptance_criteria)
      ? raw.acceptance_criteria.map((item) => boundedText(item, "Each completion check", 1_000))
      : [];
    if (!criteria.length || criteria.length > 20) throw new Error("Add between 1 and 20 measurable completion checks.");

    const solver = parseUsdc(raw.solver_reward_usdc, "Solver reward");
    const verifier = parseUsdc(raw.verifier_reward_usdc, "Verifier reward");
    if (Number(solver) + Number(verifier) > MAX_TOTAL_USDC) throw new Error("The combined reward is too large.");

    if (!Object.prototype.hasOwnProperty.call(raw, "task_window_days")) {
      if (Object.prototype.hasOwnProperty.call(raw, "deadline_days")) {
        throw new Error("Use task_window_days instead of deadline_days so the work window is imported exactly.");
      }
      throw new Error("task_window_days is required so Agent Bounties does not guess or change the work window.");
    }
    const days = Number(raw.task_window_days);
    if (!Number.isInteger(days) || days < 1 || days > 30) throw new Error("task_window_days must be a whole number from 1 to 30.");

    const sourceUrl = raw.source_url == null || String(raw.source_url).trim() === ""
      ? null
      : String(raw.source_url).trim();
    if (sourceUrl) {
      let parsed;
      try { parsed = new URL(sourceUrl); } catch (_error) { throw new Error("source_url must be a public HTTPS URL or null."); }
      if (parsed.protocol !== "https:" || !parsed.hostname) throw new Error("source_url must be a public HTTPS URL or null.");
    }

    return {
      schema: "agent-bounties/ai-prepared-draft-v1",
      title: boundedText(raw.title, "Title", 200),
      goal: boundedText(raw.goal, "Goal", 4_000),
      acceptance_criteria: criteria,
      solver_reward_usdc: solver,
      verifier_reward_usdc: verifier,
      task_window_days: days,
      source_url: sourceUrl,
      crowdfund: Boolean(raw.crowdfund),
      discovery_source: boundedText(raw.discovery_source || "User-owned AI assistant", "Discovery source", 500),
    };
  }

  function promptFor(intent, context) {
    const revision = context?.draft
      ? `\n\nCURRENT DRAFT TO REVISE:\n${JSON.stringify({
          title: context.draft.title,
          goal: context.draft.goal,
          acceptance_criteria: context.draft.acceptance_criteria,
          solver_reward_usdc: context.solver_reward_usdc,
          verifier_reward_usdc: context.verifier_reward_usdc,
          task_window_days: context.task_window_days,
        }, null, 2)}\n\nREQUESTED CHANGE:\n${intent}`
      : `\n\nWHAT I WANT DONE:\n${intent}`;

    return `Help me prepare a public Agent Bounties bounty using the context you already have about me and this request.${revision}

If the Agent Bounties MCP connector is available, call prepare_bounty_post after clarifying only details that materially affect the public terms. The MCP endpoint is ${MCP_URL}.

If the connector is not available, ask concise clarifying questions and then return ONLY one JSON object in this exact shape so I can paste it back into Agent Bounties:
{
  "title": "concise public title",
  "goal": "specific public outcome",
  "acceptance_criteria": ["binary or measurable check"],
  "solver_reward_usdc": "2.00",
  "verifier_reward_usdc": "0.10",
  "task_window_days": 30,
  "source_url": null,
  "crowdfund": false,
  "discovery_source": "AI provider and account used"
}

Constraints: title <= 200 characters; goal <= 4000; 1-20 acceptance criteria, each <= 1000; rewards are positive USDC decimals with at most 6 places; task_window_days is 1-30; source_url is HTTPS or null. Do not claim that anything is posted, created, funded, signed, or paid. I must review the draft and explicitly approve the wallet transaction on Agent Bounties.`;
  }

  function setImportStatus(message, tone = "") {
    if (!importStatus) return;
    importStatus.textContent = message || "";
    importStatus.dataset.tone = tone;
  }

  async function copyText(text) {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return;
    }
    const helper = document.createElement("textarea");
    helper.value = text;
    helper.setAttribute("readonly", "");
    helper.style.position = "fixed";
    helper.style.opacity = "0";
    document.body.append(helper);
    helper.select();
    document.execCommand("copy");
    helper.remove();
  }

  function show(intent, context = null) {
    currentIntent = boundedText(intent, "Bounty idea", 12_000);
    currentContext = context;
    currentPrompt = promptFor(currentIntent, currentContext);
    original.textContent = currentIntent;
    promptPreview.value = currentPrompt;
    panel.hidden = false;
    log.append(panel);
    requestAnimationFrame(() => {
      const previousScrollBehavior = log.style.scrollBehavior;
      log.style.scrollBehavior = "auto";
      log.scrollTop = 0;
      requestAnimationFrame(() => { log.style.scrollBehavior = previousScrollBehavior; });
    });
    document.documentElement.dataset.aiInterface = "user-owned";
    if (composerStatus) {
      composerStatus.textContent = "No Agent Bounties model key is being used. Continue in your AI account, then return with its prepared draft.";
      composerStatus.dataset.tone = "success";
    }
    return currentPrompt;
  }

  for (const button of panel.querySelectorAll("[data-ai-provider]")) {
    button.addEventListener("click", async () => {
      const provider = button.dataset.aiProvider;
      const destination = PROVIDERS[provider];
      if (!destination || !currentPrompt) return;
      const providerTab = window.open(destination, "_blank", "noopener,noreferrer");
      try {
        await copyText(currentPrompt);
        setImportStatus(`${providerTab ? "Prompt copied." : "Prompt copied, but your browser blocked the new tab."} Paste it into ${button.dataset.providerLabel || provider}.`, "success");
      } catch (_error) {
        setImportStatus("The prompt could not be copied automatically. Copy it from the expandable prompt below.", "error");
      }
    });
  }

  panel.querySelector("[data-copy-mcp]")?.addEventListener("click", async () => {
    try {
      await copyText(MCP_URL);
      setImportStatus("MCP endpoint copied.", "success");
    } catch (_error) {
      setImportStatus(`Copy this MCP endpoint: ${MCP_URL}`, "error");
    }
  });

  panel.querySelector("[data-copy-ai-prompt]")?.addEventListener("click", async () => {
    try {
      await copyText(currentPrompt);
      setImportStatus("AI prompt copied.", "success");
    } catch (_error) {
      setImportStatus("Select and copy the prompt manually.", "error");
    }
  });

  panel.querySelector("[data-import-ai-draft]")?.addEventListener("click", () => {
    try {
      const draft = parseDraft(importInput.value);
      setImportStatus("Draft imported locally. Review every field before approving it.", "success");
      panel.hidden = true;
      window.dispatchEvent(new CustomEvent("agent-bounties:prepared-draft", { detail: draft }));
    } catch (error) {
      setImportStatus(error.message || String(error), "error");
    }
  });

  window.addEventListener("agent-bounties:request-ai-handoff", (event) => {
    try {
      show(event.detail?.intent, event.detail?.context || null);
    } catch (error) {
      setImportStatus(error.message || String(error), "error");
    }
  });

  window.AgentBountyAI = Object.freeze({
    mcpUrl: MCP_URL,
    parseDraft,
    promptFor,
    show,
  });
})();
