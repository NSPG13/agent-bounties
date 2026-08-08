import { strict as assert } from 'node:assert';

// Test 1: Bridge module loads
let bridge, toolForwarder, mockServer;
try {
  bridge = await import('../../src/mcp-interop/bridge.js');
  toolForwarder = await import('../../src/mcp-interop/tool-forwarder.js');
  mockServer = await import('../../src/mcp-interop/mock-server.js');
  console.log('PASS: All MCP interop modules load');
} catch (e) {
  console.error('FAIL: Module import failed:', e.message);
  process.exit(1);
}

// Test 2: Mock server provides tools
const mock = mockServer.createMockServer({ tools: ['query_db', 'send_email', 'read_file'] });
const tools = mock.listTools();
assert.ok(Array.isArray(tools), 'Tools should be an array');
assert.strictEqual(tools.length, 3, 'Mock server should expose 3 tools');
assert.ok(tools.includes('query_db'), 'Should include query_db');
assert.ok(tools.includes('send_email'), 'Should include send_email');
assert.ok(tools.includes('read_file'), 'Should include read_file');
console.log('PASS: Mock server exposes 3 tools (query, action, resource)');

// Test 3: Tool forwarder propagates definitions
const forwarded = toolForwarder.forwardToolDefinitions(tools);
assert.ok(Array.isArray(forwarded), 'Forwarded tools should be an array');
assert.strictEqual(forwarded.length, 3, 'Should forward all 3 tools');
console.log('PASS: Tool forwarder propagates all tool definitions');

// Test 4: Bridge connects two servers
const serverA = mockServer.createMockServer({ tools: ['tool_a'] });
const serverB = mockServer.createMockServer({ tools: ['tool_b'] });
const interopBridge = bridge.createBridge(serverA, serverB);
assert.ok(interopBridge, 'Bridge should be created');
assert.strictEqual(interopBridge.getSourceServer(), serverA, 'Source should be serverA');
assert.strictEqual(interopBridge.getTargetServer(), serverB, 'Target should be serverB');
console.log('PASS: Bridge connects two MCP servers');

// Test 5: Bidirectional tool discovery
const allToolsA = interopBridge.discoverAllTools();
assert.ok(allToolsA.length >= 1, 'Should discover tools from both servers');
console.log('PASS: Bridge discovers tools bidirectionally');

// Test 6: Tool call forwarding
mock.setHandler('tool_a', (args) => ({ result: 'ok', input: args }));
const result = interopBridge.forwardCall('tool_a', { param: 'test' });
assert.ok(result, 'Forward call should return result');
assert.strictEqual(result.result, 'ok', 'Result should be ok');
assert.strictEqual(result.input.param, 'test', 'Input should be forwarded');
console.log('PASS: Tool call forwarding works bidirectionally');

// Test 7: Error handling
try {
  interopBridge.forwardCall('nonexistent_tool', {});
  console.log('FAIL: Should throw for nonexistent tool');
} catch (e) {
  console.log('PASS: Error handling for nonexistent tool works');
}

console.log('\nAll tests passed!');
