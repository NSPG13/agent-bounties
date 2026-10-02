window.agentBountiesAnalyticsConfig = Object.freeze({
  googleMeasurementId: "",
  webMcpEnabled: true,
});

// Browser tools do not depend on analytics collection or analytics consent.
(() => {
  const base = new URL(".", document.currentScript.src);
  function loadTools() {
    const script = document.createElement("script");
    script.src = new URL("webmcp.js?v=15", base).href;
    script.async = false;
    document.head.appendChild(script);
  }
  if (window.AgentBountiesWorkflow) loadTools();
  else {
    // Informational pages also offer browser tools; load their shared dependency
    // before exposing a tool registry. Failure leaves ordinary page use intact.
    const workflow = document.createElement("script");
    workflow.src = new URL("marketplace-workflow.js?v=11", base).href;
    workflow.async = false;
    workflow.onload = loadTools;
    document.head.appendChild(workflow);
  }
})();
