#!/usr/bin/env node
"use strict";
const fs = require("node:fs");
const path = require("node:path");
const { FEED_URL, assess } = require("../../site/collaborate/assessment.js");

async function main(args) {
  if (args.length > 1 || (args[0] && !["--live", "--help"].includes(args[0]))) throw new Error("Usage: node examples/free-discovery/assess.cjs [--live|--help]");
  if (args[0] === "--help") { console.log("Default: offline fixture, no network. --live: one public read-only GET, no wallet or credentials. Output is JSON on stdout."); return; }
  let feed;
  const live = args[0] === "--live";
  if (live) {
    const response = await fetch(FEED_URL, { signal: AbortSignal.timeout(10000), headers: { accept: "application/json" } });
    if (!response.ok) throw new Error(`Public feed returned HTTP ${response.status}; no action taken.`);
    const body = await response.text();
    if (body.length > 1000000) throw new Error("Unexpectedly large public feed; no action taken.");
    feed = JSON.parse(body);
  } else feed = JSON.parse(fs.readFileSync(path.join(__dirname, "fixture.json"), "utf8"));
  const result = assess(feed);
  result.source_kind = live ? "live_public_snapshot" : "offline_fixture_not_adoption";
  console.log(JSON.stringify(result, null, 2));
}
main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
