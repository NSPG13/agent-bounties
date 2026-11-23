"use strict";
const test = require("node:test"), assert = require("node:assert/strict");
const i18n = require("../site/i18n.js");
const win = (url, saved) => ({ location: { href: url }, localStorage: { getItem: () => saved } });

test("?lang wins, then the saved choice, then English", () => {
  assert.deepEqual(i18n.resolve(win("https://agentizable.ai/?lang=es", "en")), { lang: "es", persist: true });
  assert.deepEqual(i18n.resolve(win("https://agentizable.ai/?lang=en", "es")), { lang: "en", persist: true });
  assert.deepEqual(i18n.resolve(win("https://agentizable.ai/", "es")), { lang: "es", persist: false });
  assert.deepEqual(i18n.resolve(win("https://agentizable.ai/", null)), { lang: "en", persist: false });
  assert.deepEqual(i18n.resolve(win("https://agentizable.ai/?lang=fr", "de")), { lang: "en", persist: false }, "unknown values fall back to English");
});

test("exact phrases and templated text translate; unknown text is left alone", () => {
  assert.equal(i18n.translate("  Post   a bounty "), "Publicar una recompensa");
  assert.equal(i18n.translate("Example 2 of 3"), "Ejemplo 2 de 3");
  assert.equal(i18n.translate("Show all (12)"), "Mostrar todo (12)");
  assert.equal(i18n.translate("Saved drafts (4)"), "Borradores guardados (4)");
  assert.equal(i18n.translate("Next: Review the current state."), "Siguiente: Revisa el estado actual.");
  assert.equal(i18n.translate("Unlisted text (3)"), null);
  assert.equal(i18n.translate("BountySettled"), null, "protocol names stay as they are");
  assert.equal(i18n.translate("Workspace"), null, "the Workspace keeps its product name");
});

test("the dictionary is well formed", () => {
  for (const [en, es] of Object.entries(i18n.ES)) {
    assert.equal(en, en.replace(/\s+/g, " ").trim(), `normalized key: ${en}`);
    assert.ok(typeof es === "string" && es.trim(), `Spanish text for: ${en}`);
    assert.notEqual(es, en, `actually translated: ${en}`);
  }
});

test("the board group translates the live board summary, with and without optional parts", () => {
  globalThis.AgentBountiesI18nQueue = undefined;
  require("../site/i18n/es-board.js");
  for (const group of [].concat(globalThis.AgentBountiesI18nQueue)) i18n.extend(group.entries, group.patterns);
  assert.equal(
    i18n.translate("1 open now (1 direct tasks · 0 competitions · 0 child-funding tasks) · 0 scoring closed · 0 upcoming · 1 funded · refreshed 2:00:00 AM"),
    "1 abiertas ahora (1 tareas directas · 0 competencias · 0 tareas de financiamiento derivado) · 0 con puntuación cerrada · 0 próximas · 1 financiadas · actualizado 2:00:00 AM");
  assert.equal(
    i18n.translate("3 confirmed open now (2 direct tasks · 1 competitions · 0 child-funding tasks) · 1 scoring closed · 0 upcoming · 6 funded · Timing unavailable for 2; open-now total unconfirmed · 1 past deadline"),
    "3 confirmadas abiertas ahora (2 tareas directas · 1 competencias · 0 tareas de financiamiento derivado) · 1 con puntuación cerrada · 0 próximas · 6 financiadas · Horario no disponible para 2; total abierto ahora sin confirmar · 1 con plazo vencido");
});

test("the posting group translates the review card's templated lines", () => {
  globalThis.AgentBountiesI18nQueue = undefined;
  require("../site/i18n/es-post.js");
  for (const group of [].concat(globalThis.AgentBountiesI18nQueue)) i18n.extend(group.entries, group.patterns);
  assert.equal(i18n.translate("10.00 USDC (9.00 solver + 1.00 creator-review reserve)"), "10.00 USDC (9.00 para quien resuelva + 1.00 de reserva para tu revisión)");
  assert.equal(i18n.translate("10.00 USDC (9.00 solver + 1.00 verifier reward)"), "10.00 USDC (9.00 para quien resuelva + 1.00 de recompensa de verificación)");
  assert.equal(i18n.translate("3 checks"), "3 verificaciones");
  assert.equal(i18n.translate("1 check"), "1 verificación");
  assert.equal(i18n.translate("Saturday, November 28 at 7:26 PM · 5d 0h remaining"), "Saturday, November 28 at 7:26 PM · quedan 5 d 0 h");
  assert.match(i18n.translate("Bounty budget — rewards: 10.00 USDC + platform fee: 0.00 USDC. Gas sponsorship is checked before signing. Posting pauses if it is unavailable; you will not be asked to pay gas."), /^Presupuesto de la recompensa — recompensas: 10\.00 USDC \+ comisión de la plataforma: 0\.00 USDC\./);
});
