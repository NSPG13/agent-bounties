# MCP protocol compatibility

The hosted Streamable HTTP endpoint at `/mcp` implements the modern MCP
`2026-07-28` wire contract and retains a separate legacy compatibility lane for
clients that still initialize with `2024-11-05`, `2025-03-26`, or
`2025-06-18`.

## Compatibility boundary

| Client request | Server behavior |
| --- | --- |
| `2026-07-28` per-request metadata | Strict modern validation and modern result shapes |
| Legacy `initialize` handshake | Negotiates a supported legacy version and preserves legacy result shapes |
| Modern `initialize` or `ping` | `404` with JSON-RPC `-32601`; those methods are not in the modern core |
| Modern `GET /mcp` or `DELETE /mcp` | `405`; modern Streamable HTTP uses one `POST` per request |

Modern and legacy MCP implementations are not directly interoperable. The
endpoint therefore detects the era from the modern protocol header/request
metadata or `server/discover`; it does not mix modern fields into legacy
responses.

## Modern request contract

Every modern request is a single JSON-RPC object. It includes:

- `MCP-Protocol-Version: 2026-07-28`;
- `Mcp-Method`, equal to the body `method`;
- `Mcp-Name` for `tools/call`, `resources/read`, and `prompts/get`, equal to the
  body name or URI after decoding the protocol's Base64 sentinel when used;
- `params._meta["io.modelcontextprotocol/protocolVersion"]`;
- `params._meta["io.modelcontextprotocol/clientCapabilities"]`;
- preferably `params._meta["io.modelcontextprotocol/clientInfo"]`.

Header/body mismatches return HTTP `400` and JSON-RPC error `-32020`.
Unsupported modern versions return HTTP `400`, error `-32022`, and the
supported/requested version data. An invalid `Origin`, when that header is
present, returns HTTP `403`.

Use `server/discover` before calling tools:

```http
POST /mcp
Accept: application/json, text/event-stream
Content-Type: application/json
MCP-Protocol-Version: 2026-07-28
Mcp-Method: server/discover

{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"example-client","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}}}
```

Successful modern results include `resultType: "complete"` and server identity
under `_meta["io.modelcontextprotocol/serverInfo"]`. Discovery, list, and read
results also include `ttlMs` and `cacheScope`. Catalogs are deterministic so a
client can safely reuse the cache hint.

The server advertises only tools and resources. It does not advertise Tasks,
subscriptions, prompts, or other optional extensions that it does not
implement.

## Origin configuration

Requests without an `Origin` are allowed for normal server-to-server MCP
clients. Exact first-party, ChatGPT, and loopback development origins are
allowed by default. `MCP_BASE_URL` adds its exact origin. Deployments with
additional browser clients can set a comma-separated exact allowlist:

```text
MCP_ALLOWED_ORIGINS=https://client.example,https://another.example
```

Do not use wildcard origins. This check is the endpoint's DNS-rebinding
boundary, not an authentication substitute.

## Verification

```powershell
cargo test -p mcp-server chatgpt_app::tests
cargo build -p mcp-server
python scripts/check-chatgpt-app-runtime.py
```

The runtime check calls modern `server/discover`, modern catalogs, a resource,
and a tool, plus legacy `initialize` and a legacy catalog through the real HTTP
endpoint.

The deployed production smoke performs modern discovery and legacy initialize
against the published endpoint.

Protocol references:

- [MCP 2026-07-28 release](https://github.com/modelcontextprotocol/modelcontextprotocol/releases/tag/2026-07-28)
- [2026-07-28 changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)
- [Versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)
- [`server/discover`](https://modelcontextprotocol.io/specification/2026-07-28/server/discover)
