(() => {
  "use strict";

  const root = document.querySelector("[data-bounty-chat]");
  const log = document.querySelector("[data-conversation-log]");
  const prompt = document.querySelector("[data-assistant-prompt]");
  const form = document.getElementById("bounty-composer-form");
  const input = document.getElementById("bounty-composer-input");
  const status = document.querySelector("[data-composer-status]");
  const thinking = document.querySelector("[data-assistant-thinking]");
  const preview = document.getElementById("bounty-preview");
  const empty = document.querySelector("[data-draft-empty]");
  const draftState = document.querySelector("[data-draft-state]");
  const draftTitle = document.querySelector("[data-card-title]");
  const approve = document.querySelector("[data-approve-card]");
  const revise = document.querySelector("[data-revise-card]");
  const connect = document.querySelector("[data-open-funding]");
  const dialog = document.getElementById("funding-dialog");
  const cryptoMethod = document.querySelector("[data-payment-method='crypto']");

  if (!root || !log || !prompt || !form || !input || !preview) return;

  let lastAssistantText = "";
  let lastDraftTitle = "";
  let approvedAnnounced = false;

  function scrollConversation() {
    requestAnimationFrame(() => {
      log.scrollTop = log.scrollHeight;
    });
  }

  function message(role, text, kind = "") {
    const value = String(text || "").trim();
    if (!value) return;

    const article = document.createElement("article");
    article.className = "conversation-message";
    article.dataset.role = role;
    if (kind) article.dataset.kind = kind;

    const avatar = document.createElement("span");
    avatar.className = "conversation-avatar";
    avatar.textContent = role === "user" ? "YOU" : "AI";
    avatar.setAttribute("aria-hidden", "true");

    const bubble = document.createElement("div");
    bubble.className = "conversation-bubble";
    bubble.textContent = value;

    article.append(avatar, bubble);
    log.append(article);
    scrollConversation();
  }

  function syncPrompt() {
    const value = prompt.textContent.trim();
    if (!value || value === lastAssistantText) return;
    lastAssistantText = value;
    message("assistant", value);
    thinking.hidden = true;
  }

  function syncStatus() {
    const value = status.textContent.trim();
    const waiting = status.dataset.tone === "pending"
      && /AI|draft|turning|breaking|generating|preparing/i.test(value);
    thinking.hidden = !waiting;
    if (waiting) scrollConversation();
  }

  function syncDraft() {
    const ready = !preview.hidden;
    if (empty) empty.hidden = ready;
    if (!draftState) return;

    if (!ready) {
      draftState.textContent = "Building from the conversation";
      draftState.dataset.tone = "";
      return;
    }

    const title = draftTitle && draftTitle.textContent.trim();
    draftState.textContent = "Ready for your review";
    draftState.dataset.tone = "ready";
    if (title && title !== lastDraftTitle) {
      lastDraftTitle = title;
      approvedAnnounced = false;
      message("assistant", `I have a standardized bounty draft ready: “${title}”. Review it beside the conversation. Tell me what to change, or approve it when it is right.`, "draft");
    }
  }

  function syncApproval() {
    const isApproved = approve && approve.dataset.approved === "true";
    if (draftState && isApproved) {
      draftState.textContent = "Approved · not posted";
      draftState.dataset.tone = "approved";
    } else if (draftState && !preview.hidden) {
      draftState.textContent = "Ready for your review";
      draftState.dataset.tone = "ready";
    }

    if (connect) connect.textContent = isApproved ? "Connect wallet" : "Approve before connecting";
    if (isApproved && !approvedAnnounced) {
      approvedAnnounced = true;
      message("assistant", "The draft is approved. Connect a wallet when you are ready. Connecting still does not post or fund anything; the wallet will show the exact final request before you confirm it.", "draft");
    }
    if (!isApproved) approvedAnnounced = false;
  }

  form.addEventListener("submit", () => {
    const value = input.value.trim();
    if (value) message("user", value);
  }, true);

  new MutationObserver(syncPrompt).observe(prompt, {
    childList: true,
    characterData: true,
    subtree: true,
  });

  new MutationObserver(syncStatus).observe(status, {
    childList: true,
    characterData: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["data-tone"],
  });

  new MutationObserver(syncDraft).observe(preview, {
    attributes: true,
    attributeFilter: ["hidden"],
  });

  if (approve) {
    new MutationObserver(syncApproval).observe(approve, {
      attributes: true,
      attributeFilter: ["data-approved", "disabled"],
    });
    approve.addEventListener("click", () => setTimeout(syncApproval, 0));
  }

  if (revise) {
    revise.addEventListener("click", () => {
      approvedAnnounced = false;
      setTimeout(() => {
        document.querySelector(".conversation-panel")?.scrollIntoView({ behavior: "smooth", block: "start" });
        input.focus({ preventScroll: true });
      }, 0);
    });
  }

  if (connect) {
    connect.textContent = "Approve before connecting";
    connect.addEventListener("click", () => {
      setTimeout(() => {
        if (dialog && dialog.open && cryptoMethod) cryptoMethod.click();
      }, 0);
    });
  }

  const dialogObserver = dialog && new MutationObserver(() => {
    if (dialog.open && cryptoMethod) setTimeout(() => cryptoMethod.click(), 0);
  });
  if (dialogObserver) dialogObserver.observe(dialog, { attributes: true, attributeFilter: ["open"] });

  const supplied = new URLSearchParams(location.search).get("goal")
    || new URLSearchParams(location.search).get("draftObjective");
  if (supplied) input.value = supplied;

  syncPrompt();
  syncStatus();
  syncDraft();
  syncApproval();
  document.documentElement.dataset.bountyComposer = "chat-v1";
})();
