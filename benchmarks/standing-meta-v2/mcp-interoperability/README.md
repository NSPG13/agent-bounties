# MCP Interoperability Child Bounty

## Objective
Build an MCP (Model Context Protocol) interoperability bridge that enables two distinct MCP servers to exchange tool definitions and forward tool calls between each other.

## Requirements
1. Implement MCP client that connects to a remote MCP server via stdio
2. List and forward tool definitions from remote server to local server
3. Forward tool call requests and responses bidirectionally
4. Handle at least 3 tool types: query, action, resource
5. Self-contained Node.js module using @modelcontextprotocol/sdk
6. All tests pass against provided test suite

## Deliverable
- `src/mcp-interop/bridge.js` — MCP bridge implementation
- `src/mcp-interop/tool-forwarder.js` — tool definition/call forwarding
- `src/mcp-interop/mock-server.js` — mock MCP server for testing

## Validation
Run `node benchmarks/standing-meta-v2/mcp-interoperability/test.mjs` — all assertions must pass.

## Reward
1 USDC
