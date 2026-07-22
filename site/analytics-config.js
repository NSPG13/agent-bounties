window.agentBountiesAnalyticsConfig = Object.freeze({
  googleMeasurementId: "",
});

(() => {
  "use strict";

  const loadGuildShell = () => {
    const body = document.body;
    if (!body) return;

    const source = document.currentScript && document.currentScript.src
      ? document.currentScript.src
      : window.location.href;
    const base = new URL(".", source);

    if (body.classList.contains("guild-home")) {
      if (!document.querySelector('script[data-simple-home="true"]')) {
        const script = document.createElement("script");
        script.src = new URL("simple-home.js?v=1", base).href;
        script.dataset.simpleHome = "true";
        document.head.appendChild(script);
      }
      return;
    }

    body.classList.add("guild-interior");

    if (!document.querySelector('link[data-guild-pages="true"]')) {
      const stylesheet = document.createElement("link");
      stylesheet.rel = "stylesheet";
      stylesheet.href = new URL("guild-pages.css?v=20260721", base).href;
      stylesheet.dataset.guildPages = "true";
      document.head.appendChild(stylesheet);
    }

    if (!document.querySelector('script[data-guild-shell="true"]')) {
      const script = document.createElement("script");
      script.src = new URL("guild-shell.js?v=2", base).href;
      script.dataset.guildShell = "true";
      document.head.appendChild(script);
    }
  };

  if (document.body) loadGuildShell();
  else document.addEventListener("DOMContentLoaded", loadGuildShell, { once: true });
})();
