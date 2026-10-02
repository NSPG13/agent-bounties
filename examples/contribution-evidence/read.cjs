"use strict";
const fs = require("node:fs");
const { validate } = require("../../site/collaborate/recognition.js");
const URL = "https://agentbounties.app/collaborate/recognition.json";
const LIMIT = 1000000;
function profile(value) {
  if (value !== undefined && (typeof value !== "string" || !/^https:\/\/github\.com\/[A-Za-z0-9][A-Za-z0-9-]{0,38}$/.test(value))) throw Error("Invalid public profile filter.");
  return value;
}
function consume(payload, { source = "caller-supplied snapshot", profileURL, now = Date.now() } = {}) {
  profile(profileURL);
  const data = validate(payload, now);
  const records = data.records.filter(row => !profileURL || row.profile_url.toLowerCase() === profileURL.toLowerCase());
  return {
    schema_version: "agent-bounties/contribution-consumer-v1",
    state: !data.records.length ? "empty_registry" : !records.length ? "no_match" : "available",
    source, observed_at: new Date(now).toISOString(), registry_updated_at: data.updated_at,
    registry_record_count: data.records.length, matched_count: records.length,
    profile_filter: profileURL ?? null, records,
    identity_assertions_verified_by_client: false, payment_state: "not_assessed",
    limitations: "Opt-in artifact records, not a census or lifetime work history. Shape validation does not verify consent, identity or review truth. Inspect public source evidence before relying on claims. No wallet or account authority is granted."
  };
}
async function readLive({ fetchImpl = globalThis.fetch, profileURL, now = Date.now() } = {}) {
  profile(profileURL);
  const response = await fetchImpl(URL, {
    method: "GET", headers: { accept: "application/json" }, credentials: "omit",
    redirect: "error", cache: "no-store", signal: AbortSignal.timeout(10000)
  });
  if (!response.ok || !/^application\/json(?:\s*;|$)/i.test(response.headers.get("content-type") ?? "") || !response.body) throw Error("Registry unavailable.");
  const reader = response.body.getReader(), chunks = [];
  let size = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read(); if (done) break;
      size += value.byteLength;
      if (size > LIMIT) { await reader.cancel(); throw Error("Registry exceeds the size limit."); }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  const raw = new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks));
  return consume(JSON.parse(raw), { source: URL, profileURL, now });
}
function readFile(filename, options = {}) {
  const stat = fs.statSync(filename);
  if (!stat.isFile() || stat.size > LIMIT) throw Error("Invalid registry file.");
  const bytes = fs.readFileSync(filename);
  if (bytes.byteLength > LIMIT) throw Error("Registry exceeds the size limit.");
  return consume(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)), { ...options, source: "local snapshot" });
}
async function main(args) {
  let live = false, filename, profileURL;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--live" && !live && !filename) live = true;
    else if (args[i] === "--file" && !live && !filename && args[i + 1] && !args[i + 1].startsWith("--")) filename = args[++i];
    else if (args[i] === "--profile" && !profileURL && args[i + 1]) profileURL = profile(args[++i]);
    else throw Error("Invalid arguments.");
  }
  if (!live && !filename) throw Error("Choose --file PATH or --live.");
  return live ? readLive({ profileURL }) : readFile(filename, { profileURL });
}
if (require.main === module) main(process.argv.slice(2)).then(
  result => console.log(JSON.stringify(result, null, 2)),
  () => { console.log(JSON.stringify({ schema_version: "agent-bounties/contribution-consumer-v1", state: "unavailable", records: null, error: "Read failed or input was invalid. Use --file PATH or --live, optionally --profile https://github.com/HANDLE. Unavailable does not mean zero participation." }, null, 2)); process.exitCode = 1; }
);
module.exports = { URL, LIMIT, consume, readLive, readFile, main };
