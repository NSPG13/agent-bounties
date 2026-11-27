"use strict";
// Every placeholder, aria-label, title and alt in the body of a page that offers Español
// has a Spanish entry in the dictionaries that page loads. i18n.js translates these
// attributes in place, including on textareas, whose typed text it never touches.
const test = require("node:test"), assert = require("node:assert/strict");
const fs = require("node:fs"), path = require("node:path"), vm = require("node:vm");
const site = path.resolve(__dirname, "../site");
// The language switch names each language in that language on purpose.
const KEEP_AS_WRITTEN = new Set(["English", "Español"]);
const decode = (value) => value.replace(/&#10;/g, "\n").replace(/&quot;/g, "\"").replace(/&#39;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

function pages(dir = site, found = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) { if (!["fixtures", "generated", "vendor", "assets"].includes(entry.name)) pages(full, found); }
    else if (entry.name.endsWith(".html")) found.push(full);
  }
  return found;
}

// Load i18n.js and the page's own dictionaries into one context, as the browser does.
function dictionaryFor(file, html) {
  const context = { module: { exports: {} } };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(site, "i18n.js"), "utf8"), context);
  const i18n = context.module.exports;
  for (const match of html.matchAll(/<script[^>]+src="([^"]*i18n\/es-[a-z-]+\.js)[^"]*"/g)) {
    context.AgentBountiesI18nQueue = undefined;
    vm.runInContext(fs.readFileSync(path.resolve(path.dirname(file), match[1].split("?")[0]), "utf8"), context);
    for (const group of [].concat(context.AgentBountiesI18nQueue || [])) i18n.extend(group.entries, group.patterns);
  }
  return i18n;
}

test("placeholders and accessible labels on every Español page have Spanish", () => {
  const missing = [];
  let checked = 0;
  for (const file of pages()) {
    const html = fs.readFileSync(file, "utf8");
    if (!/i18n\.js\?v=/.test(html)) continue;
    const i18n = dictionaryFor(file, html), body = html.split(/<body[^>]*>/)[1] || "";
    for (const [, name, raw] of body.matchAll(/\s(placeholder|aria-label|title|alt)="([^"]*)"/g)) {
      const value = decode(raw).replace(/\s+/g, " ").trim();
      if (!/[A-Za-z]{2}/.test(value) || KEEP_AS_WRITTEN.has(value)) continue;
      checked += 1;
      if (i18n.translate(value) === null) missing.push(`${path.relative(site, file)} ${name}="${value}"`);
    }
  }
  assert.ok(checked > 100, `the check found the pages' attributes (${checked})`);
  assert.deepEqual([...new Set(missing)], [], "add these to the page's i18n/es-*.js dictionary");
});

test("a multi-line placeholder keeps its line breaks in Spanish", () => {
  const html = fs.readFileSync(path.join(site, "post.html"), "utf8");
  const i18n = dictionaryFor(path.join(site, "post.html"), html);
  assert.equal(i18n.translate("One check per line, e.g.\nLanding page loads on mobile\nIncludes a contact form"),
    "Una comprobación por línea, p. ej.:\nLa página carga en el móvil\nIncluye un formulario de contacto");
});
