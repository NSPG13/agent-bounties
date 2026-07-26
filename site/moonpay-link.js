(() => {
  "use strict";

  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

  function onrampUrl(form) {
    const bountyContract = String(form.elements.bountyContract?.value || "").trim();
    const amount = String(form.elements.amount?.value || "").trim();
    if (!ADDRESS.test(bountyContract)) {
      throw new Error("Choose a canonical bounty before opening MoonPay.");
    }
    if (!/^\d+(?:\.\d{1,6})?$/.test(amount) || Number(amount) <= 0) {
      throw new Error("Enter the USDC amount you intend to add to this bounty.");
    }

    const target = new URL("onramp.html", location.href);
    target.searchParams.set("bountyContract", bountyContract);
    target.searchParams.set("amount", amount);
    const intent = new URLSearchParams(location.search).get("intent");
    if (UUID.test(intent || "")) target.searchParams.set("intent", intent);

    const returnUrl = new URL(location.href);
    returnUrl.hash = "fund-bounty-panel";
    target.searchParams.set("return", returnUrl.href);
    return target;
  }

  function output(message, tone = "") {
    const element = document.querySelector("[data-moonpay-link-output]");
    if (!element) return;
    element.textContent = message;
    element.dataset.tone = tone;
  }

  function initialize() {
    const form = document.getElementById("autonomous-fund-form");
    const link = document.querySelector("[data-moonpay-onramp-link]");
    if (!form || !link) return;

    link.addEventListener("click", (event) => {
      event.preventDefault();
      try {
        const target = onrampUrl(form);
        output(
          "Opening a separate wallet top-up step. Buying crypto will not fund the bounty until you return and approve the canonical contribution.",
          "pending",
        );
        location.assign(target.href);
      } catch (error) {
        output(error.message || String(error), "error");
      }
    });
  }

  initialize();
})();
