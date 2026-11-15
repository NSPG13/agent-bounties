/* Card payment page: apply the appearance chosen on the site before the first paint.
   Runs blocking in <head>. It reads only the theme key; it writes nothing, adds no
   listeners and makes no requests, so it never sees quote, order or account data. */
(function () {
  'use strict';
  let choice = 'dark';
  try { const saved = window.localStorage.getItem('agent-bounties.theme'); if (saved === 'light' || saved === 'dark' || saved === 'auto') choice = saved; } catch (_) { /* Storage can be disabled. */ }
  let light = choice === 'light';
  try { if (choice === 'auto') light = window.matchMedia('(prefers-color-scheme: light)').matches; } catch (_) { /* Keep dark. */ }
  document.documentElement.dataset.theme = light ? 'light' : 'dark';
})();
