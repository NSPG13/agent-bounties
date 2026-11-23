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
