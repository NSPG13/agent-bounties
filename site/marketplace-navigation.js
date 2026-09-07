(function () {
  "use strict";
  // A real click on Post a bounty starts fresh only after the previous posting
  // is confirmed. Pending operations and unfinished drafts remain recoverable.
  document.querySelectorAll("[data-new-bounty]").forEach((link) => link.addEventListener("click", (event) => {
    if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    try {
      const flow = window.AgentBountiesWorkflow;
      if (flow.createPostingJournal(window).load()?.phase === "funding_confirmed") flow.createClient(window).start({ role: "post", new_task: true });
    } catch (error) {
      event.preventDefault();
      const status = document.querySelector("[data-navigation-status]");
      if (status) { status.hidden = false; status.textContent = error.message; }
    }
  }));
})();
