"use strict";

const assert = require("node:assert/strict");
const home = require("../site/solarpunk-home.js");
const mobile = require("../site/forest-home.js");

assert.equal(mobile.isMobileNavigator({ userAgentData: { mobile: true } }), true);
assert.equal(mobile.isMobileNavigator({ userAgent: "Mozilla/5.0 (Linux; Android 16; Pixel 9a)" }), true);
assert.equal(mobile.isMobileNavigator({ userAgent: "Mozilla/5.0 (X11; Linux x86_64)" }), false);
assert.equal(mobile.isAndroidNavigator({ userAgent: "Android 16" }), true);

const prompt = `${home.BOUNTY_POSTING_PROMPT}\n\nMy task: make the mobile AI handoff work.`;
const gptLinks = home.bountyAssistantLinks("gpt", prompt, mobile.WEBMCP_RETURN_URL);
const claudeLinks = home.bountyAssistantLinks("claude", prompt, mobile.WEBMCP_RETURN_URL);

const androidGpt = mobile.mobileLaunchUrl("gpt", gptLinks, { userAgent: "Android 16" });
assert.match(androidGpt, /^intent:\/\/chatgpt\.com\//);
assert.match(androidGpt, /package=com\.openai\.chatgpt/);
assert.doesNotMatch(androidGpt, /codex:/);
assert.equal(
  decodeURIComponent(androidGpt.match(/S\.browser_fallback_url=([^;]+);end$/)[1]),
  gptLinks.webUrl,
);

const androidClaude = mobile.mobileLaunchUrl("claude", claudeLinks, { userAgent: "Android 16" });
assert.match(androidClaude, /^intent:\/\/claude\.ai\/new/);
assert.match(androidClaude, /package=com\.anthropic\.claude/);
assert.doesNotMatch(androidClaude, /claude:/);
assert.equal(mobile.mobileLaunchUrl("gpt", gptLinks, { userAgent: "iPhone" }), gptLinks.webUrl);
assert.equal(
  mobile.mobileLaunchUrl("gpt", { webUrl: "https://evil.example/?prompt=x" }, { userAgent: "Android" }),
  null,
);

let dialog;
function assistantButton(provider) {
  const detail = { textContent: "Open a new chat" };
  return {
    dataset: { bountyAssistant: provider },
    attributes: {},
    detail,
    querySelector(selector) { return selector === "small" ? detail : null; },
    closest(selector) {
      if (selector === "[data-bounty-assistant]") return this;
      if (selector === "[data-bounty-launcher]") return dialog;
      return null;
    },
    setAttribute(name, value) { this.attributes[name] = value; },
    removeAttribute(name) { delete this.attributes[name]; },
  };
}

const promptPreview = { textContent: prompt };
const status = { textContent: "" };
const customActions = { hidden: false };
const webFallback = { hidden: true, href: "", textContent: "" };
const connectionHelp = { textContent: "Desktop connection help" };
const gptButton = assistantButton("gpt");
const claudeButton = assistantButton("claude");
const cursorButton = assistantButton("cursor");
const buttons = [gptButton, claudeButton, cursorButton];

dialog = {
  dataset: {},
  querySelectorAll(selector) { return selector === "[data-bounty-assistant]" ? buttons : []; },
  querySelector(selector) {
    return new Map([
      ["[data-bounty-prompt]", promptPreview],
      ["[data-bounty-launch-status]", status],
      ["[data-bounty-custom-actions]", customActions],
      ["[data-bounty-web-fallback]", webFallback],
      [".bounty-connection-help .bounty-launch-note", connectionHelp],
    ]).get(selector) || null;
  },
};

const handlers = {};
const launches = [];
const document = {
  querySelector(selector) { return selector === "[data-bounty-launcher]" ? dialog : null; },
  addEventListener(name, callback, options) { handlers[name] = { callback, options }; },
  createElement(tag) {
    assert.equal(tag, "a");
    return {
      href: "",
      hidden: false,
      click() { launches.push(this.href); },
      remove() {},
    };
  },
  body: { append() {} },
};
const window = {
  navigator: {
    userAgent: "Mozilla/5.0 (Linux; Android 16; Pixel 9a)",
    userAgentData: { mobile: true },
  },
  SolarpunkHome: home,
  agentBountiesAnalytics: {
    handoffUrl(value) {
      const url = new URL(value);
      url.searchParams.set("utm_source", "mobile-test");
      return url.href;
    },
  },
  location: { assign(value) { launches.push(value); } },
};

assert.equal(mobile.setupMobileAssistantHandoff(window, document), true);
assert.equal(handlers.click.options, true, "mobile interception must run during capture before the desktop handler");
assert.equal(gptButton.detail.textContent, "Open the mobile app");
assert.equal(claudeButton.detail.textContent, "Open the mobile app");
assert.equal(cursorButton.detail.textContent, "Desktop only");
assert.match(connectionHelp.textContent, /continue through WebMCP/);

let prevented = false;
let stopped = false;
handlers.click.callback({
  target: gptButton,
  preventDefault() { prevented = true; },
  stopImmediatePropagation() { stopped = true; },
});

assert.equal(prevented, true);
assert.equal(stopped, true);
assert.equal(launches.length, 1);
assert.match(launches[0], /^intent:\/\/chatgpt\.com\//);
assert.match(launches[0], /package=com\.openai\.chatgpt/);
assert.doesNotMatch(launches[0], /codex:/);
assert.equal(webFallback.hidden, false);
assert.equal(new URL(webFallback.href).origin, "https://chatgpt.com");
const prefilled = new URL(webFallback.href).searchParams.get("prompt");
assert.match(prefilled, /post\.html\?from=webmcp/);
assert.match(prefilled, /Discover WebMCP tools/);
assert.match(prefilled, /make the mobile AI handoff work/);
assert.equal(promptPreview.textContent, prefilled);
assert.match(status.textContent, /continue through WebMCP/);
assert.match(status.textContent, /Nothing is posted or funded until you approve it/);
assert.equal(customActions.hidden, true);
assert.equal(gptButton.attributes["aria-current"], "true");

assert.equal(
  mobile.setupMobileAssistantHandoff(
    { navigator: { userAgent: "Mozilla/5.0 (X11; Linux x86_64)", userAgentData: { mobile: false } } },
    { querySelector() { throw new Error("desktop must not inspect the mobile launcher"); } },
  ),
  false,
);

console.log("mobile AI selection opens the installed app with a prefilled WebMCP handoff and safe web fallback");
