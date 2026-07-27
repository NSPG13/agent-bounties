import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const uuid = require("uuid");
const JaysonBrowserClient = require("jayson/lib/client/browser");

const namespace = "6ba7b811-9dad-11d1-80b4-00c04fd430c8";
assert.throws(
  () => uuid.v5("agent-bounties", namespace, new Uint8Array(8), 4),
  RangeError,
  "the locked uuid override must include the v5 output-buffer bounds fix",
);

const client = new JaysonBrowserClient(() => {
  throw new Error("the compatibility check must not make a network request");
});
const request = client.request("agent_bounties_dependency_probe", []);
assert.equal(request.jsonrpc, "2.0");
assert.equal(request.method, "agent_bounties_dependency_probe");
assert.match(
  request.id,
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
  "Jayson must continue generating UUIDv4 request identifiers through uuid@11",
);

console.log("Coinbase transitive dependency compatibility checks passed");
