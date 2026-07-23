(function () {
  "use strict";
  const destination = document.documentElement.dataset.destination;
  if (!destination) return;
  const target = new URL(destination, window.location.origin);
  if (window.location.search) target.search = window.location.search;
  if (window.location.hash) target.hash = window.location.hash;
  window.location.replace(target.href);
})();
