#!/usr/bin/env node
"use strict";
const fs = require("node:fs");
const path = require("node:path");
const { FEED_URL, assess } = require("../../site/collaborate/assessment.js");

const MAX_FEED_BYTES = 1000000;
const FETCH_TIMEOUT_MS = 10000;

async function readLiveFeed({ fetchImpl = fetch, timeoutMs = FETCH_TIMEOUT_MS } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  let reader;
  try {
    const response = await fetchImpl(FEED_URL, {
      signal: controller.signal,
      redirect: "error",
      headers: { accept: "application/json" }
    });
    reader = response.body?.getReader();
    if (!response.ok) throw new Error(`Public feed returned HTTP ${response.status}; no action taken.`);
    if (!reader) throw new Error("Public feed has no response body; no action taken.");
    const chunks = [];
    let bytes = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > MAX_FEED_BYTES) throw new Error("Public feed exceeds 1,000,000 bytes; no action taken.");
      chunks.push(Buffer.from(value));
    }
    return JSON.parse(Buffer.concat(chunks, bytes).toString("utf8"));
  } catch (error) {
    if (controller.signal.aborted) throw new Error("Public feed timed out after its request deadline; no action taken.");
    throw error;
  } finally {
    clearTimeout(timer);
    // Do not wait for remote cleanup before returning a failed assessment.
    if (reader) { reader.cancel().catch(() => {}); reader.releaseLock(); }
    controller.abort();
  }
}

async function main(args) {
  if (args.length > 1 || (args[0] && !["--live", "--help"].includes(args[0]))) throw new Error("Usage: node examples/free-discovery/assess.cjs [--live|--help]");
  if (args[0] === "--help") { console.log("Default: offline fixture, no network. --live: one public read-only GET, no redirects, 10-second deadline, 1,000,000-byte response limit; no wallet or credentials. Confirm network and service costs first. Output is JSON on stdout."); return; }
  let feed;
  const live = args[0] === "--live";
  if (live) {
    feed = await readLiveFeed();
  } else feed = JSON.parse(fs.readFileSync(path.join(__dirname, "fixture.json"), "utf8"));
  const result = assess(feed);
  result.source_kind = live ? "live_public_snapshot" : "offline_fixture_not_adoption";
  console.log(JSON.stringify(result, null, 2));
}
if (require.main === module) main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
module.exports = { MAX_FEED_BYTES, FETCH_TIMEOUT_MS, readLiveFeed, main };
