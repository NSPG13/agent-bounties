(() => {
  "use strict";

  const STORAGE_KEY = "agent-bounties:homepage-bounty-intent";
  const CHAT_ROUTE = "objective.html?source=home&autostart=1";

  function normalize(value) {
    return String(value || "").trim();
  }

  function start(rawMessage) {
    const message = normalize(rawMessage);
    if (!message) return false;

    let stored = false;
    try {
      window.sessionStorage.setItem(STORAGE_KEY, message);
      stored = true;
    } catch (_) {
      stored = false;
    }

    const destination = stored
      ? CHAT_ROUTE
      : `${CHAT_ROUTE}&goal=${encodeURIComponent(message)}`;
    window.location.assign(destination);
    return true;
  }

  function consume(search = window.location.search) {
    const params = search instanceof URLSearchParams
      ? search
      : new URLSearchParams(search || "");
    if (params.get("autostart") !== "1") {
      return Object.freeze({ message: "", autostart: false });
    }

    let message = "";
    if (params.get("source") === "home") {
      try {
        message = normalize(window.sessionStorage.getItem(STORAGE_KEY));
        window.sessionStorage.removeItem(STORAGE_KEY);
      } catch (_) {
        message = "";
      }
    }

    if (!message) {
      message = normalize(
        params.get("goal") || params.get("draftObjective") || params.get("objective"),
      );
    }

    return Object.freeze({ message, autostart: Boolean(message) });
  }

  window.AgentBountyEntry = Object.freeze({ STORAGE_KEY, start, consume });
})();
