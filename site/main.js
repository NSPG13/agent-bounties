(function () {
  const canvas = document.getElementById("network-canvas");
  if (canvas) {
    const context = canvas.getContext("2d");
    const nodes = Array.from({ length: 44 }, (_, index) => ({
      x: Math.random(),
      y: Math.random(),
      vx: (Math.random() - 0.5) * 0.0007,
      vy: (Math.random() - 0.5) * 0.0007,
      r: index % 7 === 0 ? 2.6 : 1.6,
    }));

    function resize() {
      const scale = window.devicePixelRatio || 1;
      canvas.width = Math.floor(canvas.clientWidth * scale);
      canvas.height = Math.floor(canvas.clientHeight * scale);
      context.setTransform(scale, 0, 0, scale, 0, 0);
    }

    function draw() {
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      context.clearRect(0, 0, width, height);
      context.fillStyle = "#10191f";
      context.fillRect(0, 0, width, height);

      for (const node of nodes) {
        node.x += node.vx;
        node.y += node.vy;
        if (node.x < 0.04 || node.x > 0.96) node.vx *= -1;
        if (node.y < 0.06 || node.y > 0.94) node.vy *= -1;
      }

      for (let i = 0; i < nodes.length; i += 1) {
        for (let j = i + 1; j < nodes.length; j += 1) {
          const a = nodes[i];
          const b = nodes[j];
          const ax = a.x * width;
          const ay = a.y * height;
          const bx = b.x * width;
          const by = b.y * height;
          const distance = Math.hypot(ax - bx, ay - by);
          if (distance < 170) {
            context.strokeStyle = `rgba(141, 224, 203, ${0.2 - distance / 1000})`;
            context.lineWidth = 1;
            context.beginPath();
            context.moveTo(ax, ay);
            context.lineTo(bx, by);
            context.stroke();
          }
        }
      }

      for (const node of nodes) {
        const x = node.x * width;
        const y = node.y * height;
        context.beginPath();
        context.fillStyle = node.r > 2 ? "#f0f4c3" : "#8ee0cb";
        context.arc(x, y, node.r, 0, Math.PI * 2);
        context.fill();
      }

      requestAnimationFrame(draw);
    }

    window.addEventListener("resize", resize);
    resize();
    draw();
  }

  const form = document.getElementById("funding-form");
  const output = document.getElementById("funding-output");
  const readinessButton = document.getElementById("readiness-button");
  const readinessOutput = document.getElementById("readiness-output");
  if (!form || !output) return;

  function randomUuid() {
    if (window.crypto && typeof window.crypto.randomUUID === "function") {
      return window.crypto.randomUUID();
    }
    const bytes = new Uint8Array(16);
    window.crypto.getRandomValues(bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"));
    return `${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex.slice(6, 8).join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10).join("")}`;
  }

  const organizationField = form.elements.namedItem("organizationId");
  if (organizationField instanceof HTMLInputElement) {
    const storageKey = "agent-bounties-public-funder-id";
    let funderId = window.localStorage.getItem(storageKey);
    if (!funderId) {
      funderId = randomUuid();
      window.localStorage.setItem(storageKey, funderId);
    }
    if (!organizationField.value) {
      organizationField.value = funderId;
    }
  }

  function configuredLabel(value) {
    return value ? "ready" : "needs setup";
  }

  function formatReadiness(report) {
    const checks = Array.isArray(report.checks) ? report.checks : [];
    const methodConfig = report.stripe_payment_method_configuration_configured === true;
    const checkoutMethodCheck = checks.find(
      (check) => check && check.name === "Stripe Checkout payment-method configuration",
    );
    const webhookBoundary = Array.isArray(report.evidence_boundaries)
      && report.evidence_boundaries.some((boundary) =>
        String(boundary).includes("checkout.session.completed webhook")
      );

    return [
      `Network: ${report.network || "unknown"} (${report.network_chain_id || "unknown"})`,
      `Live-money gate: ${configuredLabel(report.live_money_ready === true)}`,
      `Stripe live execution: ${configuredLabel(report.stripe_live_mode_ready === true)}`,
      `Signed webhook evidence: ${configuredLabel(report.stripe_webhook_ready === true)}`,
      `Checkout method configuration: ${configuredLabel(methodConfig)}`,
      `PayPal-capable setup indicator: ${methodConfig ? "configured" : "not configured"}`,
      `Base mainnet escrow: ${configuredLabel(report.base_mainnet_ready === true)}`,
      `Webhook settlement boundary: ${webhookBoundary ? "present" : "missing"}`,
      checkoutMethodCheck && checkoutMethodCheck.detail
        ? `Method detail: ${checkoutMethodCheck.detail}`
        : "Method detail: no readiness detail returned",
      "This readiness check is informational. Funding still requires Stripe Checkout completion and a verified webhook.",
    ].join("\n");
  }

  if (readinessButton && readinessOutput) {
    readinessButton.addEventListener("click", async () => {
      const apiBaseUrlField = form.elements.namedItem("apiBaseUrl");
      const apiBaseUrl = apiBaseUrlField instanceof HTMLInputElement
        ? apiBaseUrlField.value.replace(/\/+$/, "")
        : "";
      if (!apiBaseUrl) {
        readinessOutput.textContent = "Enter a hosted API base URL before checking readiness.";
        return;
      }

      readinessOutput.textContent = "Checking hosted live-money readiness...";
      try {
        const response = await fetch(`${apiBaseUrl}/v1/readiness/live-money?network=base-mainnet`, {
          headers: { accept: "application/json" },
        });
        if (!response.ok) {
          throw new Error(`Readiness check failed with ${response.status}`);
        }
        readinessOutput.textContent = formatReadiness(await response.json());
      } catch (error) {
        readinessOutput.textContent = `${error.message}\n\nNo funding intent or Checkout Session was created. Confirm the hosted API URL, CORS settings, and live-money readiness endpoint.`;
      }
    });
  }

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    output.textContent = "Creating funding intent...";
    const data = new FormData(form);
    const apiBaseUrl = String(data.get("apiBaseUrl") || "").replace(/\/+$/, "");
    const bountyId = String(data.get("bountyId") || "").trim();
    const organizationId = String(data.get("organizationId") || "").trim();
    const amountMinor = Number(data.get("amountMinor"));
    const currency = String(data.get("currency") || "usd").trim().toLowerCase();
    const externalReference =
      String(data.get("externalReference") || "").trim() ||
      `checkout-funding-${Date.now()}`;

    try {
      const intentResponse = await fetch(`${apiBaseUrl}/v1/bounties/${bountyId}/funding-intents`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          bounty_id: bountyId,
          source_organization_id: organizationId,
          contributor_agent_id: null,
          amount_minor: amountMinor,
          currency,
          rail: "StripeFiat",
          external_reference: externalReference,
          stripe_success_url: `${window.location.origin}${window.location.pathname.replace(/funding\.html$/, "success.html")}`,
          stripe_cancel_url: `${window.location.origin}${window.location.pathname.replace(/funding\.html$/, "cancel.html")}`,
          base_escrow_contract: null,
          base_payer: null,
          base_token: null,
          base_network: null,
        }),
      });
      if (!intentResponse.ok) {
        throw new Error(`Funding intent failed with ${intentResponse.status}`);
      }
      const intentReport = await intentResponse.json();
      const fundingIntentId = intentReport.intent && intentReport.intent.id;
      if (!fundingIntentId) {
        throw new Error("Funding intent response did not include intent.id");
      }

      output.textContent = "Creating Stripe Checkout session...";
      const checkoutResponse = await fetch(
        `${apiBaseUrl}/v1/stripe/live/funding-intents/${fundingIntentId}/checkout-session`,
        { method: "POST" },
      );
      if (!checkoutResponse.ok) {
        throw new Error(`Checkout creation failed with ${checkoutResponse.status}`);
      }
      const checkout = await checkoutResponse.json();
      if (!checkout.url) {
        throw new Error("Checkout response did not include a Stripe URL");
      }
      output.textContent = `Checkout session created.\n\nOpen Stripe Checkout: ${checkout.url}`;
      window.location.assign(checkout.url);
    } catch (error) {
      output.textContent = `${error.message}\n\nNo payment credentials were collected here. Confirm the hosted API URL, bounty id, organization id, Stripe live settings, and ENABLE_STRIPE_PUBLIC_CHECKOUT=true.`;
    }
  });
})();
