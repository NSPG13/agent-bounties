/* Card payment page: apply the appearance chosen on the site before the first paint.
   Runs blocking in <head>. It reads only the theme key; it writes nothing, adds no
   listeners and makes no requests, so it never sees quote, order or account data. */
(function () {
  'use strict';
  // Only Light and Dark exist. Without a saved one (or with an older "auto"), follow the
  // system preference; the main site saves that choice on its next visit.
  let choice = null;
  try { const saved = window.localStorage.getItem('agent-bounties.theme'); if (saved === 'light' || saved === 'dark') choice = saved; } catch (_) { /* Storage can be disabled. */ }
  if (!choice) { try { choice = window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'; } catch (_) { choice = 'dark'; } }
  document.documentElement.dataset.theme = choice;
})();
