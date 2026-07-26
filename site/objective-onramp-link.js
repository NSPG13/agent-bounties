(() => {
  "use strict";

  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  const link = document.querySelector("[data-objective-onramp-link]");
  const required = document.querySelector("[data-wallet-required]");
  if (!link || !required) return;

  function requiredAmount() {
    const match = String(required.textContent || "").replace(/,/g, "").match(/\d+(?:\.\d{1,6})?/);
    if (!match || Number(match[0]) <= 0) return "20.00";
    return Number(match[0]).toFixed(2);
  }

  function update() {
    const target = new URL("onramp.html", location.href);
    target.searchParams.set("amount", requiredAmount());
    const returnUrl = new URL(location.href);
    returnUrl.hash = "post";
    target.searchParams.set("return", returnUrl.href);
    const intent = new URLSearchParams(location.search).get("intent");
    if (UUID.test(intent || "")) target.searchParams.set("intent", intent);
    link.href = target.href;
  }

  new MutationObserver(update).observe(required, {
    childList: true,
    characterData: true,
    subtree: true,
  });
  update();
})();
