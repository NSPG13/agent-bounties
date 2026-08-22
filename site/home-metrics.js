(function (root, factory) {
  "use strict";
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesHomeMetrics = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  function nonNegativeInteger(value) {
    const number = Number(value);
    return Number.isSafeInteger(number) && number >= 0 ? number : null;
  }

  function lifetimeExternalActiveIdentities(platform, github) {
    const platformStatus = String(platform?.coverage?.status || "").toLowerCase();
    const githubStatus = String(github?.coverage?.status || "").toLowerCase();
    if (["partial", "unavailable", ""].includes(platformStatus)
      || ["partial", "unavailable", ""].includes(githubStatus)) return null;
    const platformIdentities = nonNegativeInteger(platform?.platform_active_identities?.lifetime);
    const githubIdentities = nonNegativeInteger(github?.periods?.lifetime?.active_identities);
    if (platformIdentities === null || githubIdentities === null) return null;
    return {
      total: platformIdentities + githubIdentities,
      platform: platformIdentities,
      github: githubIdentities,
    };
  }

  function render(document, platform, github) {
    const metric = lifetimeExternalActiveIdentities(platform, github);
    document.querySelectorAll("[data-external-active-identities]").forEach((output) => {
      if (!metric) {
        output.textContent = "--";
        output.removeAttribute("data-loaded");
        output.title = "Lifetime external active identities are unavailable because a required source is incomplete.";
        return;
      }
      output.textContent = metric.total.toLocaleString("en-US");
      output.dataset.loaded = "true";
      output.title = `${metric.total.toLocaleString("en-US")} lifetime external active identities: ${metric.platform.toLocaleString("en-US")} platform-namespaced plus ${metric.github.toLocaleString("en-US")} GitHub-namespaced. Namespaces are not deduplicated into unique people.`;
    });
    return metric;
  }

  return { lifetimeExternalActiveIdentities, render };
});
