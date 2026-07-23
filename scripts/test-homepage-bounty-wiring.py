import json
from pathlib import Path


repo_root = Path(__file__).resolve().parents[1]
site_dir = repo_root / "site"
index_html = (site_dir / "index.html").read_text(encoding="utf-8")
objective_html = (site_dir / "objective.html").read_text(encoding="utf-8")
chat_ui = (site_dir / "bounty-chat-ui.js").read_text(encoding="utf-8")
ai_handoff = (site_dir / "ai-bounty-handoff.js").read_text(encoding="utf-8")
composer = (site_dir / "bounty-composer-v2.js").read_text(encoding="utf-8")
entry_js = (site_dir / "bounty-entry.js").read_text(encoding="utf-8")
guild_home_js = (site_dir / "guild-home.js").read_text(encoding="utf-8")
agent_html = (site_dir / "agent" / "index.html").read_text(encoding="utf-8")
agent_markdown = (site_dir / "agent" / "index.md").read_text(encoding="utf-8")
discovery = json.loads(
    (site_dir / ".well-known" / "agent-bounties.json").read_text(encoding="utf-8")
)


def require(label: str, text: str, marker: str) -> None:
    if marker not in text:
        raise SystemExit(f"{label} missing required marker: {marker}")


for retired in (
    "data-connect-wallet",
    "data-wallet-provider",
    'class="network-chip"',
    "Connect Wallet",
):
    if retired in index_html:
        raise SystemExit(f"homepage still exposes retired wallet-first navigation: {retired}")

for marker in (
    'data-primary-bounty-cta>Post a bounty</a>',
    'class="mode-switch"',
    'href="agent/"',
    'action="objective.html"',
    'name="source" value="home"',
    'name="autostart" value="1"',
    'name="goal"',
    'aria-label="Start a bounty with this problem"',
    'type="text/markdown" title="Agent mode (Markdown)"',
    'src="bounty-entry.js?v=1"',
):
    require("homepage", index_html, marker)

if index_html.index('src="bounty-entry.js?v=1"') > index_html.index('src="guild-home.js"'):
    raise SystemExit("homepage bounty entry helper must load before the form controller")

for marker in (
    'src="bounty-entry.js?v=1"',
    'src="ai-bounty-handoff.js?v=5"',
    'src="bounty-composer-v2.js?v=2"',
    'src="bounty-chat-ui.js?v=4"',
    'data-ai-handoff',
    'data-ai-draft-import',
):
    require("objective chat", objective_html, marker)

if not (
    objective_html.index('src="bounty-entry.js?v=1"')
    < objective_html.index('src="ai-bounty-handoff.js?v=5"')
    < objective_html.index('src="bounty-composer-v2.js?v=2"')
    < objective_html.index('src="bounty-chat-ui.js?v=4"')
):
    raise SystemExit("objective chat scripts are not loaded in entry/AI handoff/composer/UI order")

for marker in (
    "agent-bounties:homepage-bounty-intent",
    "window.sessionStorage.setItem",
    "window.sessionStorage.removeItem",
    "objective.html?source=home&autostart=1",
    "encodeURIComponent(message)",
):
    require("bounty entry helper", entry_js, marker)

for marker in ('event.key !== "Enter"', "search.requestSubmit()"):
    require("homepage Enter handler", guild_home_js, marker)

for marker in (
    "window.AgentBountyEntry.consume(params)",
    "entryIntent.autostart",
    "lastAssistantText = prompt.textContent.trim()",
    "form.requestSubmit()",
):
    require("chat autostart", chat_ui, marker)

for marker in (
    "https://mcp.agentbounties.app/mcp",
    "chatgpt.com",
    "claude.ai/new",
    "gemini.google.com/app",
    "prepare_bounty_post",
    "agent-bounties:prepared-draft",
    "No Agent Bounties model key is being used",
):
    require("user-owned AI handoff", ai_handoff, marker)

for marker in (
    "requestUserOwnedAi(value)",
    "agent-bounties:prepared-draft",
    "importPreparedDraft(event.detail)",
    'params.get("taskWindowDays")',
):
    require("local prepared-draft composer", composer, marker)

for marker in (
    "AGENT MODE · NO COMPUTER USE REQUIRED",
    "https://agentbounties.app/agent/index.md",
    "https://mcp.agentbounties.app/mcp",
    "https://api.agentbounties.app/api-docs/openapi.json",
    "https://agentbounties.app/schemas/discovery-manifest.v2.json",
    "Only <code>BountySettled</code> proves bounty payment",
):
    require("agent portal", agent_html, marker)

for marker in (
    "No computer use is required",
    "MCP transport:",
    "OpenAPI:",
    "CLI source:",
    "Only a confirmed canonical `BountySettled` event proves bounty payment",
):
    require("agent Markdown", agent_markdown, marker)

expected_endpoints = {
    "agent_mode": "https://agentbounties.app/agent/",
    "agent_mode_markdown": "https://agentbounties.app/agent/index.md",
    "openapi": "https://api.agentbounties.app/api-docs/openapi.json",
    "discovery_manifest_schema": "https://agentbounties.app/schemas/discovery-manifest.v2.json",
    "cli_source": "https://github.com/NSPG13/agent-bounties/tree/main/crates/cli",
    "user_ai_bounty_composer": "https://agentbounties.app/objective.html",
}
for key, expected in expected_endpoints.items():
    if discovery.get("endpoints", {}).get(key) != expected:
        raise SystemExit(f"discovery endpoint {key} does not route to {expected}")

print("homepage bounty entry and agent-mode wiring are valid")
