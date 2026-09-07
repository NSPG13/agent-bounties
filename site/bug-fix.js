(function () {
  "use strict";
  // Use the shared first-party handoff to preserve source and privacy choices.
  document.querySelectorAll("[data-buyer-post]").forEach(function (link) {
    const target = new URL(link.getAttribute("href"), window.location.href);
    const analytics = window.agentBountiesAnalytics;
    if (analytics && typeof analytics.handoffUrl === "function") {
      link.href = analytics.handoffUrl(target.href);
    }
  });
})();
