window.agentBountiesAnalyticsConfig = Object.freeze({
  googleMeasurementId: "",
  webMcpEnabled: true,
});

// Browser tools do not depend on analytics collection or analytics consent.
(() => {
  const base = new URL(".", document.currentScript.src);
  const script = document.createElement("script");
  script.src = new URL("webmcp.js?v=2", base).href;
  script.async = false;
  document.head.appendChild(script);
})();
