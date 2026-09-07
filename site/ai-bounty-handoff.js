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
  const webFallback = document.querySelector("[data-ai-web-fallback]");

  if (!panel || !log || !original || !promptPreview || !importInput) return;

  let currentIntent = "";
  let currentContext = null;
  let currentPrompt = "";

  function providerLinks(provider, prompt, context = null) {
    if (!Object.hasOwn(PROVIDERS, provider) || !String(prompt || "").trim()) return null;
    if (provider !== "chatgpt") return { webUrl: PROVIDERS[provider], desktopUrl: null };
    const browserUrl = new URL("https://agentbounties.app/post.html?from=webmcp");
    const parent = context?.meta_child?.parent_bounty_contract;
    if (parent != null) {
      if (!/^0x[0-9a-fA-F]{40}$/.test(parent)) throw new Error("The parent bounty address is invalid.");
      browserUrl.searchParams.set("parentBounty", parent.toLowerCase());
    }
    // OpenAI Learn uses this registered desktop route for a draft + browser tab.
    const desktop = new URL("codex://threads/new");
    desktop.searchParams.set("prompt", prompt);
    desktop.searchParams.set("browserUrl", browserUrl.href);
    const web = new URL(PROVIDERS.chatgpt);
    web.searchParams.set("prompt", prompt);
    return { desktopUrl: desktop.href, webUrl: web.href };
  }

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
      benchmark: raw.benchmark && typeof raw.benchmark === "object" && !Array.isArray(raw.benchmark)
        ? raw.benchmark
        : null,
      evidence_schema: raw.evidence_schema && typeof raw.evidence_schema === "object" && !Array.isArray(raw.evidence_schema)
        ? raw.evidence_schema
        : null,
      meta_child: raw.meta_child == null ? null : window.AgentBountiesMetaChild?.normalize(raw.meta_child) || (() => { throw new Error("The parent-child review module is unavailable."); })(),
      ...(raw.image_required === true
        ? {
            image_required: true,
            image: raw.image,
          }
        : {}),
    };
  }

  function promptFor(intent, context) {
    const data = {};
    if (intent) data.request = String(intent);
    if (context?.draft) {
      data.draft = {
        ...context.draft,
        solver_reward_usdc: context.solver_reward_usdc ?? context.draft.solver_reward_usdc,
        verifier_reward_usdc: context.verifier_reward_usdc ?? context.draft.verifier_reward_usdc,
        task_window_days: context.task_window_days ?? context.draft.task_window_days,
        source_url: context.source_url ?? context.draft.source_url,
        benchmark: context.benchmark ?? context.draft.benchmark,
        evidence_schema: context.evidence_schema ?? context.draft.evidence_schema,
        crowdfund: context.crowdfund ?? context.draft.crowdfund,
      };
    }
    if (context?.meta_child || context?.draft?.meta_child) data.meta_child = context.meta_child || context.draft.meta_child;
    return window.AgentBountiesPostingPrompt.build(data);
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
    try {
      if (!document.execCommand("copy")) throw new Error("Clipboard access is unavailable.");
    } finally {
      helper.remove();
    }
  }

  function show(intent, context = null) {
    currentIntent = boundedText(intent, "Bounty idea", 12_000);
    currentContext = context;
    currentPrompt = promptFor(currentIntent, currentContext);
    original.textContent = currentIntent;
    promptPreview.value = currentPrompt;
    if (webFallback) {
      webFallback.hidden = true;
      webFallback.removeAttribute("href");
    }
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
      let links;
      try {
        links = providerLinks(provider, currentPrompt, currentContext);
      } catch (error) {
        setImportStatus(error.message || "The desktop handoff could not be prepared.", "error");
        return;
      }
      if (!links) return;
      if (webFallback) {
        webFallback.href = links.webUrl;
        webFallback.hidden = !links.desktopUrl;
      }
      if (links.desktopUrl) {
        const anchor = document.createElement("a");
        anchor.href = links.desktopUrl;
        anchor.hidden = true;
        document.body.append(anchor);
        anchor.click();
        anchor.remove();
      } else {
        window.open(links.webUrl, "_blank", "noopener,noreferrer");
      }
      try {
        await copyText(currentPrompt);
        setImportStatus(links.desktopUrl
          ? "Desktop launch requested. Accept your browser's Open app prompt if shown. Your prompt is also copied; if the app does not open, use ChatGPT web or paste it manually. Nothing is sent automatically."
          : `Prompt copied. Requested ${button.dataset.providerLabel || provider} in a new tab; paste the prompt there.`, "success");
      } catch (_error) {
        setImportStatus("Launch requested, but clipboard access was unavailable. The exact prompt remains below for manual copying; the ChatGPT launch links also contain it.", "error");
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
    providerLinks,
    show,
  });
})();
