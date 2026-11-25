"use strict";
// Shared ⓘ info tip: placement stays on screen, and the button label is translated.
const test = require("node:test"), assert = require("node:assert/strict");
const tip = require("../site/info-tip.js"), i18n = require("../site/i18n.js");
const rect = (left, top, width, height) => ({ left, top, width, height, right: left + width, bottom: top + height });
const el = (box) => ({ getBoundingClientRect: () => box });

test("the tip opens centred below the icon when it fits", () => {
  assert.deepEqual(tip.place(el(rect(500, 100, 24, 24)), el(rect(0, 0, 200, 60)), { width: 1440, height: 900 }), { left: 412, top: 130 });
});

test("the tip never runs off a 390px screen", () => {
  const viewport = { width: 390, height: 844 };
  assert.deepEqual(tip.place(el(rect(370, 100, 24, 24)), el(rect(0, 0, 240, 80)), viewport), { left: 142, top: 130 }, "near the right edge it shifts left");
  assert.deepEqual(tip.place(el(rect(0, 100, 24, 24)), el(rect(0, 0, 240, 80)), viewport), { left: 8, top: 130 }, "near the left edge it shifts right");
  assert.deepEqual(tip.place(el(rect(100, 100, 24, 24)), el(rect(0, 0, 600, 80)), viewport), { left: 8, top: 130 }, "an overly wide tip is clamped to the margins");
});

test("the tip flips above the icon near the bottom, and stays below when neither fits", () => {
  assert.deepEqual(tip.place(el(rect(100, 800, 24, 24)), el(rect(0, 0, 200, 80)), { width: 1440, height: 844 }), { left: 12, top: 714 });
  assert.deepEqual(tip.place(el(rect(100, 40, 24, 24)), el(rect(0, 0, 200, 900)), { width: 1440, height: 844 }), { left: 12, top: 70 });
});

test("the icon's accessible name has a Spanish translation", () => {
  assert.equal(tip.LABEL, "What is this?");
  assert.equal(i18n.translate(tip.LABEL), "¿Para qué sirve?");
});
