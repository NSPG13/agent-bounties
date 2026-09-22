/* Never replace a user's selected wallet with a generic provider checkout. */
(() => {
  "use strict";
  const button = document.querySelector("[data-direct-moonpay]");
  // Keep this guard until live, wallet-bound checkout is enabled.
  button.disabled = true;
  button.setAttribute("aria-disabled", "true");
  button.removeAttribute("href");
})();
