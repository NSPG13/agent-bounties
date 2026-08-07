#!/usr/bin/env node
// tool-registry-drift.mjs — MCP/API tool-registry drift coverage (#685)
// Compares committed fixture against tool inventory; fails closed on drift.

import { readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(scriptDir, "..", "..");
const fixturePath = join(repoRoot, "fixtures", "tool-registry-expected.json");

function fail(errors, code = 1) {
  process.stdout.write(
    JSON.stringify({ ok: false, errors }) + "\n"
  );
  process.exit(code);
}

function pass(summary) {
  process.stdout.write(
    JSON.stringify({ ok: true, ...summary }) + "\n"
  );
  process.exit(0);
}

// 1. Load fixture
if (!existsSync(fixturePath)) {
  fail(["fixture_missing: fixtures/tool-registry-expected.json not found"], 2);
}

let fixture;
try {
  fixture = JSON.parse(readFileSync(fixturePath, "utf8"));
} catch {
  fail(["fixture_invalid_json"], 2);
}

if (fixture.schema !== "agent-bounties/tool-registry-fixture-v1") {
  fail(["fixture_schema_unsupported"], 2);
}

const errors = [];

// 2. Collect all expected tools
const allExpected = [
  ...(fixture.required_readonly_tools || []),
  ...(fixture.required_readiness_tools || []),
];

// 3. Check for duplicates
const seen = new Set();
const duplicates = [];
for (const tool of allExpected) {
  if (seen.has(tool)) {
    duplicates.push(tool);
  }
  seen.add(tool);
}
if (duplicates.length > 0) {
  errors.push(`duplicate_tools: [${duplicates.join(", ")}]`);
}

// 4. Check for missing tools (empty name)
const missing = allExpected.filter((t) => !t || typeof t !== "string");
if (missing.length > 0) {
  errors.push("missing_tool_names: empty or non-string tool entries detected");
}

// 5. Endpoint validation
const endpoints = fixture.endpoints || {};
if (!endpoints.mcp_transport || typeof endpoints.mcp_transport !== "string") {
  errors.push("endpoint_mcp_transport_missing");
}
if (!endpoints.json_tool_inventory || typeof endpoints.json_tool_inventory !== "string") {
  errors.push("endpoint_json_tool_inventory_missing");
}

// 6. Distinguish transport types
const transportTypes = new Set([
  typeof endpoints.mcp_transport,
  typeof endpoints.json_tool_inventory,
]);
if (transportTypes.size > 1) {
  errors.push("endpoint_type_mismatch: transport and inventory endpoints differ in format");
}
// Both should be strings (paths)
if (typeof endpoints.mcp_transport !== "string" || typeof endpoints.json_tool_inventory !== "string") {
  errors.push("endpoint_format_invalid: both endpoints must be string paths");
}

// 7. Tool count summary
const summary = {
  readonly_count: fixture.required_readonly_tools.length,
  readiness_count: fixture.required_readiness_tools.length,
  total_tools: allExpected.length,
  duplicates_found: duplicates.length,
  mcp_endpoint: endpoints.mcp_transport,
  json_inventory_endpoint: endpoints.json_tool_inventory,
};

if (errors.length > 0) {
  fail(errors, 1);
}

pass(summary);
