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
  const draftState = document.querySelector("[data-draft-state]");
  const approve = document.querySelector("[data-approve-card]");
  const revise = document.querySelector("[data-revise-card]");
  const connect = document.querySelector("[data-open-funding]");
  const dialog = document.getElementById("funding-dialog");
  const cryptoMethod = document.querySelector("[data-payment-method='crypto']");

  if (!root || !log || !prompt || !form || !input || !preview) return;

  let lastAssistantText = "";
  let approvedAnnounced = false;
  let draftSyncQueued = false;

  function scrollConversation() {
    requestAnimationFrame(() => {
      log.scrollTop = log.scrollHeight;
    });
  }

  function addMessage(role, text) {
    const value = String(text || "").trim();
    if (!value) return;

    const article = document.createElement("article");
    article.className = "conversation-message";
    article.dataset.role = role;

    const avatar = document.createElement("span");
    avatar.className = "conversation-avatar";
    avatar.textContent = role === "user" ? "You" : "AI";
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
    addMessage("assistant", value);
    thinking.hidden = true;
  }

  function syncStatus() {
    const value = status.textContent.trim();
    const waiting = status.dataset.tone === "pending"
      && /AI|draft|turning|breaking|generating|preparing/i.test(value);
    thinking.hidden = !waiting;
    if (waiting) scrollConversation();
  }

  function setDraftState(text, tone) {
    if (!draftState) return;
    if (draftState.textContent !== text) draftState.textContent = text;
    if (draftState.dataset.tone !== tone) draftState.dataset.tone = tone;
  }

  function syncDraft() {
    draftSyncQueued = false;
    if (preview.hidden) return;
    if (approve?.dataset.approved !== "true") setDraftState("Draft · not posted", "ready");
    if (preview.parentElement !== log || preview !== log.lastElementChild) log.append(preview);
    scrollConversation();
  }

  function scheduleDraftSync() {
    if (draftSyncQueued) return;
    draftSyncQueued = true;
    requestAnimationFrame(syncDraft);
  }

  function syncApproval() {
    const isApproved = approve?.dataset.approved === "true";
    setDraftState(isApproved ? "Approved · not posted" : "Draft · not posted", isApproved ? "approved" : "ready");

    if (isApproved && !approvedAnnounced) {
      approvedAnnounced = true;
      addMessage("assistant", "Draft approved. Connect your wallet when you are ready.");
    }
    if (!isApproved) approvedAnnounced = false;
  }

  function prepareNaturalRevision(value) {
    if (preview.hidden || !revise) return;
    const alreadyRevising = /what should the ai change/i.test(prompt.textContent);
    if (alreadyRevising) return;
    revise.click();
    input.value = value;
  }

  function fitInput() {
    input.style.height = "auto";
    input.style.height = `${Math.min(input.scrollHeight, 160)}px`;
  }

  form.addEventListener("submit", () => {
    const value = input.value.trim();
    if (!value) return;
    prepareNaturalRevision(value);
    addMessage("user", value);
    requestAnimationFrame(() => {
      input.style.height = "auto";
    });
  }, true);

  input.addEventListener("input", fitInput);
  input.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    event.preventDefault();
    form.requestSubmit();
  });

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

  new MutationObserver(scheduleDraftSync).observe(preview, {
    attributes: true,
    attributeFilter: ["hidden"],
    childList: true,
    characterData: true,
    subtree: true,
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
      setTimeout(() => input.focus({ preventScroll: true }), 0);
    });
  }

  if (connect) {
    connect.addEventListener("click", () => {
      setTimeout(() => {
        if (dialog?.open && cryptoMethod) cryptoMethod.click();
      }, 0);
    });
  }

  if (dialog) {
    new MutationObserver(() => {
      if (dialog.open && cryptoMethod) setTimeout(() => cryptoMethod.click(), 0);
    }).observe(dialog, { attributes: true, attributeFilter: ["open"] });
  }

  const params = new URLSearchParams(location.search);
  const supplied = params.get("goal") || params.get("draftObjective") || params.get("objective");
  if (supplied) {
    input.value = supplied;
    fitInput();
  }

  syncPrompt();
  syncStatus();
  scheduleDraftSync();
  syncApproval();
  document.documentElement.dataset.bountyComposer = "chat-v2";
})();
