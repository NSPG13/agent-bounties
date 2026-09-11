/* Landing-page interactions. No model calls, account writes, or wallet actions. */
if (typeof document !== "undefined") {
  (() => {
    const form = document.querySelector("[data-home-task]");
    const input = document.querySelector("#home-task");
    if (!form || !input) return;
    const storageKey = "agent-bounties.home-task";
    try { input.value = sessionStorage.getItem(storageKey) || ""; } catch (_) { /* Input remains usable. */ }
    input.addEventListener("input", () => {
      try { sessionStorage.setItem(storageKey, input.value); } catch (_) { /* Submission reports unavailable storage. */ }
    });
    form.addEventListener("submit", event => {
      event.preventDefault();
      document.querySelector("#post-a-bounty")?.click();
    });
    document.querySelectorAll("[data-task-example]").forEach(button => button.addEventListener("click", () => {
      input.value = button.dataset.taskExample;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
      input.focus({ preventScroll: true });
    }));
    // A cycling placeholder never replaces user input and stops during editing.
    const examples = Array.from(new Set(Array.from(document.querySelectorAll("[data-task-example]"), button => button.dataset.taskExample)));
    let example = 0;
    const reduced = matchMedia("(prefers-reduced-motion: reduce)");
    const outcome = document.querySelector("[data-outcome-word]");
    const outcomes = ["Faster", "Cheaper", "Better"];
    let outcomeIndex = 0, outcomeAnimation;
    window.setInterval(() => {
      if (!outcome || document.hidden || reduced.matches || outcome.getBoundingClientRect().bottom < 0) return;
      outcomeIndex = (outcomeIndex + 1) % outcomes.length;
      outcome.textContent = outcomes[outcomeIndex];
      outcomeAnimation?.cancel();
      outcomeAnimation = outcome.animate?.([
        { opacity: 0, transform: "translateY(.3em)" },
        { opacity: 1, transform: "translateY(0)" },
      ], { duration: 420, easing: "cubic-bezier(.2,.8,.2,1)" });
    }, 2800);
    reduced.addEventListener("change", () => { if (reduced.matches) outcomeAnimation?.cancel(); });
    if (!reduced.matches && "IntersectionObserver" in window) {
      const reveal = new IntersectionObserver(entries => entries.forEach(entry => {
        if (!entry.isIntersecting) return;
        entry.target.dataset.enter = "visible";
        reveal.unobserve(entry.target);
      }), { threshold: .12 });
      document.querySelectorAll(".ab-process-step").forEach(step => { step.dataset.enter = "waiting"; reveal.observe(step); });
      reduced.addEventListener("change", () => {
        if (!reduced.matches) return;
        reveal.disconnect();
        document.querySelectorAll("[data-enter]").forEach(step => { step.dataset.enter = "visible"; });
      });
    }
    window.setInterval(() => {
      if (!examples.length || document.hidden || reduced.matches || input.value || document.activeElement === input) return;
      example = (example + 1) % examples.length;
      input.placeholder = examples[example];
    }, 5000);
    const deck = document.querySelector("[data-example-deck]");
    const cards = Array.from(deck.querySelectorAll(".ab-example-card"));
    let active = 0, pointerX = null;
    function show(direction) {
      active = (active + direction + cards.length) % cards.length;
      cards.forEach((card, index) => { card.dataset.slot = String((index - active + cards.length) % cards.length); card.setAttribute("aria-hidden", String(index !== active)); });
      document.querySelector("[data-deck-status]").textContent = `Example ${active + 1} of ${cards.length}`;
    }
    document.querySelector("[data-deck-prev]").addEventListener("click", () => show(-1));
    document.querySelector("[data-deck-next]").addEventListener("click", () => show(1));
    deck.addEventListener("keydown", event => {
      if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
      event.preventDefault(); show(event.key === "ArrowRight" ? 1 : -1);
    });
    deck.addEventListener("pointerdown", event => { pointerX = event.clientX; });
    deck.addEventListener("pointerup", event => { if (pointerX !== null && Math.abs(event.clientX - pointerX) > 45) show(event.clientX < pointerX ? 1 : -1); pointerX = null; });
    deck.addEventListener("pointercancel", () => { pointerX = null; });
    show(0);
  })();
}

(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesMobileAIHandoff = api;
  if (root && root.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const WEBMCP_RETURN_URL = "https://agentbounties.app/post.html?from=webmcp";
  const MOBILE_PROVIDERS = Object.freeze({
    gpt: Object.freeze({ label: "ChatGPT", hostname: "chatgpt.com", androidPackage: "com.openai.chatgpt" }),
    claude: Object.freeze({ label: "Claude", hostname: "claude.ai", androidPackage: "com.anthropic.claude" }),
  });

  function isMobileNavigator(navigatorLike = {}) {
    if (typeof navigatorLike.userAgentData?.mobile === "boolean") return navigatorLike.userAgentData.mobile;
    const userAgent = String(navigatorLike.userAgent || "");
    return /Android|iPhone|iPad|iPod/i.test(userAgent)
      || (navigatorLike.platform === "MacIntel" && Number(navigatorLike.maxTouchPoints) > 1);
  }

  function isAndroidNavigator(navigatorLike = {}) {
    return /Android/i.test(String(navigatorLike.userAgent || ""));
  }

  function trustedWebUrl(provider, links) {
    const config = MOBILE_PROVIDERS[provider];
    if (!config) return null;
    try {
      const url = new URL(String(links?.webUrl || ""));
      if (url.protocol !== "https:" || url.hostname !== config.hostname || url.username || url.password) return null;
      return url.href;
    } catch (_error) {
      return null;
    }
  }

  function androidIntentUrl(webUrl, packageName) {
    const url = new URL(webUrl);
    if (url.protocol !== "https:" || !/^[a-z][a-z0-9_.]*$/i.test(String(packageName || ""))) {
      throw new Error("A trusted HTTPS app link and Android package are required.");
    }
    return `intent://${url.host}${url.pathname}${url.search}#Intent;scheme=https;package=${packageName};S.browser_fallback_url=${encodeURIComponent(url.href)};end`;
  }

  function mobileLaunchUrl(provider, links, navigatorLike = {}) {
    const config = MOBILE_PROVIDERS[provider];
    const webUrl = trustedWebUrl(provider, links);
    if (!config || !webUrl) return null;
    return isAndroidNavigator(navigatorLike)
      ? androidIntentUrl(webUrl, config.androidPackage)
      : webUrl;
  }

  function exactPrompt(provider, links, fallback) {
    const name = provider === "gpt" ? "prompt" : "q";
    try { return new URL(links.webUrl).searchParams.get(name) || fallback; }
    catch (_error) { return fallback; }
  }

  function reviewDestination(win) {
    try {
      const candidate = win.agentBountiesAnalytics?.handoffUrl?.(WEBMCP_RETURN_URL) || WEBMCP_RETURN_URL;
      const url = new URL(candidate);
      if (url.origin === "https://agentbounties.app" && url.pathname === "/post.html") return url.href;
    } catch (_error) { /* The canonical first-party handoff remains available. */ }
    return WEBMCP_RETURN_URL;
  }

  function updateMobileCopy(dialog) {
    for (const button of dialog.querySelectorAll("[data-bounty-assistant]")) {
      const key = String(button.dataset.bountyAssistant || "").toLowerCase();
      const detail = button.querySelector("small");
      if (!detail) continue;
      if (MOBILE_PROVIDERS[key]) detail.textContent = "Open the mobile app";
      else if (key === "cursor") detail.textContent = "Desktop only";
    }
    const help = dialog.querySelector(".bounty-connection-help .bounty-launch-note");
    if (help) {
      help.textContent = "ChatGPT and Claude open their mobile app with the posting message ready. Send it so the assistant can open Agent Bounties and continue through WebMCP. If the app is unavailable, the same link opens its web version.";
    }
  }

  function setupMobileAssistantHandoff(win, doc) {
    if (!isMobileNavigator(win.navigator)) return false;
    const dialog = doc.querySelector("[data-bounty-launcher]");
    if (!dialog) return false;
    if (dialog.dataset.mobileAiHandoff === "ready") return true;

    const assistantButtons = Array.from(dialog.querySelectorAll("[data-bounty-assistant]"));
    const promptPreview = dialog.querySelector("[data-bounty-prompt]");
    const status = dialog.querySelector("[data-bounty-launch-status]");
    const customActions = dialog.querySelector("[data-bounty-custom-actions]");
    const webFallback = dialog.querySelector("[data-bounty-web-fallback]");

    updateMobileCopy(dialog);
    dialog.dataset.mobileAiHandoff = "ready";

    doc.addEventListener("click", (event) => {
      const button = event.target?.closest?.("[data-bounty-assistant]");
      if (!button || button.closest?.("[data-bounty-launcher]") !== dialog) return;
      const provider = String(button.dataset.bountyAssistant || "").toLowerCase();
      if (provider !== "cursor" && !MOBILE_PROVIDERS[provider]) return;

      const prompt = String(promptPreview?.textContent || "").trim();
      const links = win.SolarpunkHome?.bountyAssistantLinks?.(
        provider,
        prompt,
        reviewDestination(win),
      );
      if (!prompt || !links) return;

      event.preventDefault?.();
      event.stopImmediatePropagation?.();
      assistantButtons.forEach((item) => item.removeAttribute("aria-current"));
      button.setAttribute("aria-current", "true");

      if (provider === "cursor") {
        if (customActions) customActions.hidden = false;
        if (webFallback) {
          webFallback.hidden = true;
          webFallback.removeAttribute?.("href");
        }
        if (status) status.textContent = "Cursor is desktop-only. Choose ChatGPT or Claude on this phone, or copy the instructions to continue later in Cursor.";
        return;
      }

      const config = MOBILE_PROVIDERS[provider];
      const launchUrl = mobileLaunchUrl(provider, links, win.navigator);
      if (!launchUrl) return;
      if (customActions) customActions.hidden = true;
      if (promptPreview) promptPreview.textContent = exactPrompt(provider, links, prompt);
      if (webFallback) {
        webFallback.href = links.webUrl;
        webFallback.textContent = `Open ${config.label} web instead`;
        webFallback.hidden = false;
      }
      if (status) {
        status.textContent = `Opening ${config.label} with your posting message ready. Send it so ${config.label} can open Agent Bounties and continue through WebMCP. Nothing is posted or funded until you approve it.`;
      }

      try {
        const anchor = doc.createElement("a");
        anchor.href = launchUrl;
        anchor.hidden = true;
        doc.body.append(anchor);
        anchor.click();
        anchor.remove();
      } catch (_error) {
        win.location.assign(links.webUrl);
      }
    }, true);
    return true;
  }

  function start(win, doc) {
    return setupMobileAssistantHandoff(win, doc);
  }

  return Object.freeze({
    WEBMCP_RETURN_URL,
    androidIntentUrl,
    isAndroidNavigator,
    isMobileNavigator,
    mobileLaunchUrl,
    reviewDestination,
    setupMobileAssistantHandoff,
    start,
    trustedWebUrl,
  });
});
