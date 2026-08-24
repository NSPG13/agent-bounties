from __future__ import annotations

import json
import re
import xml.etree.ElementTree as ET
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urldefrag, urlparse


CANONICAL_PAGES = {
    "index.html": "https://agentbounties.app/",
    "earn.html": "https://agentbounties.app/earn.html",
    "competition.html": "https://agentbounties.app/competition.html",
    "about.html": "https://agentbounties.app/about.html",
    "blog/index.html": "https://agentbounties.app/blog/",
    "blog/agentic-economy-needs-a-market-for-work.html": "https://agentbounties.app/blog/agentic-economy-needs-a-market-for-work.html",
    "how-to-earn-money-with-my-ai-agent.html": "https://agentbounties.app/how-to-earn-money-with-my-ai-agent.html",
    "authorize.html": "https://agentbounties.app/authorize.html",
    "cancel.html": "https://agentbounties.app/cancel.html",
    "metrics.html": "https://agentbounties.app/metrics.html",
    "onramp.html": "https://agentbounties.app/onramp.html",
    "post.html": "https://agentbounties.app/post.html",
    "privacy.html": "https://agentbounties.app/privacy.html",
    "success.html": "https://agentbounties.app/success.html",
    "terms.html": "https://agentbounties.app/terms.html",
}
INDEXABLE_PAGES = {
    "about.html",
    "blog/agentic-economy-needs-a-market-for-work.html",
    "blog/index.html",
    "competition.html",
    "earn.html",
    "how-to-earn-money-with-my-ai-agent.html",
    "index.html",
    "metrics.html",
    "privacy.html",
    "terms.html",
}
REQUIRED_FILES = {
    ".nojekyll",
    ".well-known/agent-bounties.json",
    ".well-known/agent-card.json",
    ".well-known/x402.json",
    "agent/index.md",
    "analytics-config.js",
    "analytics.js",
    "competition.html",
    "competition.js",
    "earn.html",
    "about.css",
    "about.html",
    "blog/agentic-economy-needs-a-market-for-work.html",
    "blog/feed.xml",
    "blog/index.html",
    "blog/posts.json",
    "ai-bounty-handoff.css",
    "ai-bounty-handoff.js",
    "authorize.html",
    "authorize.js",
    "bounty-chat-controls.css",
    "bounty-chat-ui.js",
    "bounty-chat.css",
    "bounty-composer-v2.css",
    "bounty-composer-v2.js",
    "bounty-composer.css",
    "bounty-entry.js",
    "cancel.html",
    "evm.js",
    "favicon.svg",
    "generated/github-participation.json",
    "generated/public-metrics-policy.json",
    "index.html",
    "how-to-earn-money-with-my-ai-agent.html",
    "legal-consent.js",
    "llms.txt",
    "metrics.css",
    "metrics.html",
    "metrics.js",
    "marketplace.css",
    "marketplace.js",
    "moonpay-direct-fallback.js",
    "moonpay-onramp.js",
    "onramp.css",
    "onramp.html",
    "post.html",
    "privacy.html",
    "protocol.json",
    "robots.txt",
    "sitemap.xml",
    "solarpunk-home.js",
    "solarpunk.css",
    "simple-ux.css",
    "styles.css",
    "success.html",
    "terms.html",
    "guild-pages.css",
    "guild-shell.js",
    "wallet-adapters.css",
    "x402-test-vectors.json",
}
ALLOWED_UI_CODE = {
    "about.css",
    "ai-bounty-handoff.css",
    "ai-bounty-handoff.js",
    "analytics-config.js",
    "analytics.js",
    "authorize.js",
    "bounty-chat-controls.css",
    "bounty-chat-ui.js",
    "bounty-chat.css",
    "bounty-composer-v2.css",
    "bounty-composer-v2.js",
    "bounty-composer.css",
    "bounty-entry.js",
    "competition.js",
    "evm.js",
    "guild-pages.css",
    "guild-shell.js",
    "legal-consent.js",
    "metrics.css",
    "metrics.js",
    "marketplace.css",
    "marketplace.js",
    "moonpay-direct-fallback.js",
    "moonpay-onramp.js",
    "onramp.css",
    "simple-ux.css",
    "solarpunk-home.js",
    "solarpunk.css",
    "styles.css",
    "wallet-adapters.css",
}
EXPECTED_SCENE_ASSETS = {
    "assets/solarpunk/characters-helping.webp",
    "assets/solarpunk/characters-walking.webp",
    "assets/solarpunk/characters-wrestling.webp",
    "assets/solarpunk/scene-dawn-mobile.webp",
    "assets/solarpunk/scene-dawn.webp",
    "assets/solarpunk/scene-day-mobile.webp",
    "assets/solarpunk/scene-day.webp",
    "assets/solarpunk/scene-dusk-mobile.webp",
    "assets/solarpunk/scene-dusk.webp",
    "assets/solarpunk/scene-night-mobile.webp",
    "assets/solarpunk/scene-night.webp",
}
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")


class PageParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.links: list[str] = []
        self.ids: set[str] = set()
        self.h1_count = 0

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if values.get("id"):
            self.ids.add(values["id"] or "")
        if tag.lower() == "h1":
            self.h1_count += 1
        for attribute in ("href", "src", "data-src", "data-srcset"):
            if values.get(attribute):
                self.links.append(values[attribute] or "")


def fail(message: str) -> None:
    raise SystemExit(message)


def require_phrases(label: str, text: str, phrases: list[str]) -> None:
    for phrase in phrases:
        if phrase not in text:
            fail(f"{label} missing required phrase: {phrase}")


def json_file(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail(f"{path}: expected a JSON object")
    return value


def check_internal_link(site_dir: Path, source: Path, link: str, ids: set[str]) -> None:
    target, fragment = urldefrag(link)
    parsed = urlparse(target)
    if parsed.scheme in {"http", "https", "mailto"}:
        return
    if not target:
        if fragment and fragment not in ids:
            fail(f"{source}: missing local anchor {fragment}")
        return
    if target.startswith("/"):
        fail(f"{source}: root-relative link is not portable: {link}")
    path = parsed.path
    target_path = (source.parent / path).resolve()
    try:
        target_path.relative_to(site_dir.resolve())
    except ValueError:
        fail(f"{source}: link escapes site directory: {link}")
    if not target_path.exists():
        fail(f"{source}: missing linked file {link}")
    if fragment and target_path.suffix == ".html":
        target_parser = PageParser()
        target_parser.feed(target_path.read_text(encoding="utf-8"))
        if fragment not in target_parser.ids:
            fail(f"{source}: missing target anchor {link}")


def check_protocol(protocol: dict, deployment: dict) -> None:
    if protocol.get("protocol_version") != "agent-bounties/autonomous-v1":
        fail("protocol.json must identify autonomous-v1")
    if protocol.get("network") != "base-mainnet" or protocol.get("chain_id") != 8453:
        fail("protocol.json must target Base mainnet chain 8453")
    if protocol.get("native_usdc", "").lower() != "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913":
        fail("protocol.json must use Base native USDC")
    if protocol.get("status") not in {"pending_external_review_and_deployment", "active"}:
        fail("protocol.json has an unsupported status")
    if protocol.get("status") == "active":
        for field in ("factory", "implementation"):
            if not ADDRESS.match(protocol.get(field) or ""):
                fail(f"active protocol.json requires a valid {field} address")
    elif protocol.get("factory") is not None or protocol.get("implementation") is not None:
        fail("pending protocol.json must not advertise undeployed addresses")
    if deployment.get("protocol_version") != protocol.get("protocol_version"):
        fail("site and deployment manifests disagree on protocol version")
    if deployment.get("status") != protocol.get("status"):
        fail("site and deployment manifests disagree on deployment status")
    if deployment.get("factory", {}).get("contract") != protocol.get("factory"):
        fail("site and deployment manifests disagree on factory address")
    if deployment.get("policy", {}).get("operator_settlement_signer") is not False:
        fail("autonomous deployment must not configure a settlement operator")


def check_discovery(site_dir: Path, repo_root: Path, protocol: dict) -> None:
    discovery = json_file(site_dir / ".well-known" / "agent-bounties.json")
    schema = json_file(repo_root / "schemas" / "discovery-manifest.v2.json")
    unknown = set(discovery) - set(schema.get("properties", {}))
    missing = set(schema.get("required", [])) - set(discovery)
    if unknown or missing:
        fail(f"discovery schema mismatch: unknown={sorted(unknown)} missing={sorted(missing)}")
    if discovery.get("schema") != "https://agentbounties.app/schemas/discovery-manifest.v2.json":
        fail("discovery manifest must use schema v2")
    if discovery.get("open_source") is not True:
        fail("discovery manifest must advertise open_source=true")
    manifest_protocol = discovery.get("protocol", {})
    if manifest_protocol.get("version") != protocol.get("protocol_version"):
        fail("discovery and protocol versions disagree")
    if manifest_protocol.get("factory") != protocol.get("factory"):
        fail("discovery and protocol factory addresses disagree")
    if manifest_protocol.get("operator_settlement_signer") is not False:
        fail("discovery must not advertise a settlement operator")
    if manifest_protocol.get("payout_authority") != "confirmed canonical BountySettled event":
        fail("discovery must bind autonomous-v1 payment to BountySettled")
    boundary = "Only a confirmed canonical BountySettled or CompetitionSettledV2 event proves solver payment, depending on the protocol version."
    if boundary not in discovery.get("evidence_boundaries", []):
        fail("discovery must preserve the canonical payment boundary")
    tools = set(discovery.get("agent_tools", []))
    for tool in ("get_bounty_feed", "prepare_bounty_action", "get_bounty_action_status", "inspect_open_competition_v2"):
        if tool not in tools:
            fail(f"discovery manifest is missing agent tool {tool}")

    x402 = json_file(site_dir / ".well-known" / "x402.json")
    if x402.get("x402Version") != 2:
        fail("x402 discovery must use version 2")
    resources = {item.get("name"): item for item in x402.get("resources", [])}
    funding = resources.get("canonical-bounty-funding", {})
    if funding.get("scheme") != "agent-bounty-fund" or funding.get("genericExactCompatible") is not False:
        fail("x402 discovery must preserve the custom canonical funding scheme")
    if "FundingAdded" not in funding.get("settlement", ""):
        fail("x402 funding evidence must remain bound to FundingAdded")

    vectors = json_file(site_dir / "x402-test-vectors.json")
    if vectors.get("schema_version") != "agent-bounties/x402-test-vectors-v1":
        fail("x402 test vectors have the wrong schema")
    by_id = {item.get("id"): item for item in vectors.get("vectors", [])}
    for vector_id in ("pending_relay_is_not_funding", "confirmed_funding", "solver_payment_boundary"):
        if vector_id not in by_id:
            fail(f"x402 test vectors are missing {vector_id}")
    if by_id["pending_relay_is_not_funding"].get("expected", {}).get("funded") is not False:
        fail("pending relay must not count as funding")
    if by_id["confirmed_funding"].get("expected", {}).get("paid") is not False:
        fail("FundingAdded must not count as solver payment")
    if by_id["solver_payment_boundary"].get("input", {}).get("canonical_event") != "BountySettled":
        fail("x402 solver payment evidence must remain bound to BountySettled")

    llms = (site_dir / "llms.txt").read_text(encoding="utf-8")
    agent_markdown = (site_dir / "agent" / "index.md").read_text(encoding="utf-8")
    require_phrases(
        "llms.txt",
        llms,
        [
            "get_bounty_feed",
            "A2A Agent Card: https://api.agentbounties.app/.well-known/agent-card.json",
            "Only a confirmed canonical `BountySettled` or `CompetitionSettledV2` event",
        ],
    )
    require_phrases(
        "agent/index.md",
        agent_markdown,
        ["No computer use is required", "get_bounty_feed", "CompetitionSettledV2"],
    )


def check_a2a_card(site_dir: Path, repo_root: Path) -> None:
    site_card_path = site_dir / ".well-known" / "agent-card.json"
    api_card_path = repo_root / "crates" / "api" / "fixtures" / "agent-card.json"
    if site_card_path.read_bytes() != api_card_path.read_bytes():
        fail("website and API A2A Agent Cards must match byte-for-byte")
    card = json_file(site_card_path)
    interfaces = card.get("supportedInterfaces", [])
    if interfaces != [
        {
            "url": "https://api.agentbounties.app/a2a/v1",
            "protocolBinding": "HTTP+JSON",
            "protocolVersion": "1.0",
        }
    ]:
        fail("A2A Agent Card must advertise only the implemented HTTP+JSON 1.0 interface")
    if card.get("capabilities") != {
        "streaming": False,
        "pushNotifications": False,
        "extendedAgentCard": False,
    }:
        fail("A2A Agent Card capabilities must remain conservative and implementation-backed")
    expected_skills = {
        "discover-ready-to-earn-bounties",
        "explain-bounty-opportunity",
        "explain-agent-bounties-protocol",
        "explain-bounty-alerts",
    }
    skills = card.get("skills", [])
    if {skill.get("id") for skill in skills} != expected_skills:
        fail("A2A Agent Card skills must match the read-only implementation")
    if any(not skill.get("examples") or not skill.get("tags") for skill in skills):
        fail("every A2A skill must include examples and discovery tags")
    if "cannot claim work" not in card.get("description", ""):
        fail("A2A Agent Card must state its consequential-action boundary")


def check_blog(site_dir: Path) -> None:
    about = (site_dir / "about.html").read_text(encoding="utf-8")
    archive = (site_dir / "blog" / "index.html").read_text(encoding="utf-8")
    guide = (site_dir / "how-to-earn-money-with-my-ai-agent.html").read_text(encoding="utf-8")
    essay = (site_dir / "blog" / "agentic-economy-needs-a-market-for-work.html").read_text(encoding="utf-8")
    for label, text in (("about.html", about), ("blog/index.html", archive)):
        require_phrases(
            label,
            text,
            [
                "how-to-earn-money-with-my-ai-agent.html",
                "agentic-economy-needs-a-market-for-work.html",
            ],
        )
    require_phrases(
        "AI-agent earnings guide",
        guide,
        [
            '"@type": "BlogPosting"',
            '"@type": "FAQPage"',
            "Publisher disclosure:",
            "Contribution margin",
            "Where Agent Bounties fits",
            "makes no earnings promise",
        ],
    )
    require_phrases(
        "agentic economy essay",
        essay,
        [
            '"@type":"BlogPosting"',
            "Publisher disclosure:",
            "originally published on DEV",
            "Collaboration and competition should coexist",
            "Only a confirmed canonical <code>BountySettled</code> or <code>CompetitionSettledV2</code> event",
        ],
    )
    posts = json_file(site_dir / "blog" / "posts.json")
    expected_urls = {
        "https://agentbounties.app/how-to-earn-money-with-my-ai-agent.html",
        "https://agentbounties.app/blog/agentic-economy-needs-a-market-for-work.html",
    }
    if posts.get("version") != "https://jsonfeed.org/version/1.1":
        fail("blog JSON index must use JSON Feed 1.1")
    if {item.get("url") for item in posts.get("items", [])} != expected_urls:
        fail("blog JSON index must contain every canonical post exactly once")
    atom = ET.parse(site_dir / "blog" / "feed.xml").getroot()
    namespace = {"atom": "http://www.w3.org/2005/Atom"}
    atom_links = {
        item.attrib.get("href")
        for item in atom.findall("atom:entry/atom:link", namespace)
        if item.attrib.get("href")
    }
    if atom_links != expected_urls:
        fail("Atom feed must contain every canonical post exactly once")


def check_analytics(site_dir: Path, repo_root: Path) -> None:
    javascript = (site_dir / "analytics.js").read_text(encoding="utf-8")
    config = (site_dir / "analytics-config.js").read_text(encoding="utf-8")
    require_phrases(
        "analytics.js",
        javascript,
        [
            "https://api.agentbounties.app/v1/analytics/events",
            "navigator.globalPrivacyControl",
            "navigator.doNotTrack",
            'credentials: "omit"',
            'referrerPolicy: "no-referrer"',
            "page_path: window.location.pathname",
            "allow_google_signals: false",
            "allow_ad_personalization_signals: false",
            "data-google-analytics-consent",
        ],
    )
    for forbidden in ("document.cookie", "location.search.slice", "wallet_address", "user_agent", "ip_address"):
        if forbidden in javascript:
            fail(f"analytics.js must not collect or store {forbidden}")
    if not re.search(r'googleMeasurementId:\s*"(?:|G-[A-Z0-9]+)"', config):
        fail("analytics-config.js must contain an empty or valid GA4 measurement ID")
    workflow = (repo_root / ".github" / "workflows" / "pages.yml").read_text(encoding="utf-8")
    require_phrases("Pages analytics configuration", workflow, ["GA_MEASUREMENT_ID", "^G-[A-Z0-9]+$"])


def check_metrics(site_dir: Path) -> None:
    page = (site_dir / "metrics.html").read_text(encoding="utf-8")
    javascript = (site_dir / "metrics.js").read_text(encoding="utf-8")
    css = (site_dir / "metrics.css").read_text(encoding="utf-8")
    require_phrases(
        "metrics.html",
        page,
        [
            "External active identities",
            "Marketplace payout volume",
            "Mature claim-to-settlement",
            "Counts are external requests, not unique people, agents, clients, or sessions",
            "Verify every payout",
            "Policy-excluded value",
            'href="generated/public-metrics-policy.json"',
            "Only a confirmed canonical <code>BountySettled</code> or <code>CompetitionSettledV2</code> event proves solver payment, depending on the protocol version",
            '<a href="./">Home</a><a href="metrics.html" aria-current="page">Metrics</a>',
        ],
    )
    for removed in ("earn.html", "how-it-works.html"):
        if removed in page:
            fail(f"metrics.html still links to removed page {removed}")
    require_phrases(
        "metrics.js",
        javascript,
        [
            "marketplace_payout_volume",
            "lifetime_settled_rounds",
            "canonicalPayoutRows",
            "partitionCanonicalPayoutRows",
            "PUBLIC_METRICS_POLICY_URL",
            "visibilitychange",
            '"unavailable"',
            '"delayed"',
        ],
    )
    require_phrases(
        "metrics.css",
        css,
        ["@media (prefers-reduced-motion: reduce)", ".audit-table-shell:focus-visible"],
    )
    github = json_file(site_dir / "generated" / "github-participation.json")
    if github.get("schema_version") != "agent-bounties/github-participation-v1":
        fail("GitHub participation artifact has the wrong schema")
    if github.get("coverage", {}).get("raw_identifiers_included") is not False:
        fail("GitHub participation artifact must remain aggregate-only")
    for forbidden in ("html_url", "profile_url", "comment_text", "wallet_address"):
        if forbidden in json.dumps(github).lower():
            fail(f"GitHub participation artifact exposes forbidden field {forbidden}")


def check_homepage(site_dir: Path) -> None:
    page = (site_dir / "index.html").read_text(encoding="utf-8")
    javascript = (site_dir / "solarpunk-home.js").read_text(encoding="utf-8")
    css = (site_dir / "solarpunk.css").read_text(encoding="utf-8")
    require_phrases(
        "index.html",
        page,
        [
            "Rather pay for results than tokens?",
            "Let AI agents",
            "compete <em>&amp;</em> collaborate",
            "to solve your problems",
            "data-scene-plate=\"dawn\"",
            "data-scene-plate=\"day\"",
            "data-scene-plate=\"dusk\"",
            "data-scene-plate=\"night\"",
            "data-scene-canvas",
            "data-market-volume",
            "data-live-bounties",
            "data-completed-bounties",
            "metrics.html#payout-audit",
            "Only a confirmed canonical <code>BountySettled</code> or <code>CompetitionSettledV2</code> event proves solver payment.",
        ],
    )
    require_phrases(
        "index.html navigation",
        page,
        [
            '<a href="about.html">About us</a>',
            '<a href="about.html#blog">Blog</a>',
            '<a href="earn.html">Find bounties</a>',
        ],
    )
    hero_start = page.find('<section class="hero"')
    hero_end = page.find("</section>", hero_start)
    hero_action = page.find('<div class="hero-action">', hero_start)
    if hero_start < 0 or hero_end < 0 or not hero_start < hero_action < hero_end:
        fail("the homepage CTA must remain in the hero flow to prevent headline overlap")
    stylesheet_version = re.search(r'<link rel="stylesheet" href="solarpunk\.css\?v=(\d+)">', page)
    if not stylesheet_version or int(stylesheet_version.group(1)) < 14:
        fail("the homepage must load the flow-layout stylesheet through a cache-busted URL")
    header_start = page.find('<header class="scene-header">')
    header_end = page.find("</header>", header_start)
    market_volume = page.find("data-market-volume")
    metrics_start = page.find('<section class="market-proof"')
    metrics_end = page.find("</section>", metrics_start)
    if min(header_start, header_end, market_volume, metrics_start, metrics_end) < 0:
        fail("the homepage must retain its header and marketplace metric structure")
    if header_start < market_volume < header_end:
        fail("market volume must not be displayed in the navigation header")
    if not metrics_start < market_volume < metrics_end:
        fail("market volume must be displayed in the marketplace metrics panel")
    if 'class="metric-market"' not in page:
        fail("market volume must occupy the full-width third metric row")
    require_phrases(
        "index.html bounty assistant launcher",
        page,
        [
            'data-bounty-open',
            'aria-controls="bounty-launcher"',
            'data-bounty-launcher aria-labelledby="bounty-launcher-title"',
            'data-bounty-assistant="gpt"',
            'data-bounty-assistant="claude"',
            'data-bounty-assistant="cursor"',
            'data-bounty-assistant="custom"',
            'assets/solarpunk/provider-openai.svg',
            'assets/solarpunk/provider-claude.svg',
            'assets/solarpunk/provider-cursor.svg',
            "The prompt is never submitted automatically.",
        ],
    )
    for placeholder in ('class="assistant-mark" aria-hidden="true">G<', 'class="assistant-mark" aria-hidden="true">C<', '>⌁</span>'):
        if placeholder in page:
            fail("the bounty assistant chooser must use provider vector marks, not letter placeholders")
    require_phrases(
        "index.html login dialog",
        page,
        [
            'data-auth-open aria-haspopup="dialog"',
            'data-auth-dialog aria-labelledby="auth-title"',
            'data-auth-provider="google"',
            'data-auth-provider="microsoft"',
            'data-auth-provider="github"',
            'data-auth-provider="amazon"',
            "data-auth-session",
            "Create an account",
        ],
    )
    if "town hall" in page.lower():
        fail("homepage must not use the retired town-hall language")
    for removed in ("post.html", "how-it-works.html"):
        if removed in page:
            fail(f"homepage links to removed page {removed}")
    require_phrases(
        "solarpunk-home.js",
        javascript,
        [
            "sceneTime",
            "LOCAL_HOSTS",
            "ready_to_earn",
            "source_type=canonical_base",
            "payment_state=escrowed",
            "marketplace_payout_volume?.lifetime?.usdc",
            "lifetime_settled_rounds",
            "setInterval(refreshMetrics, 60000)",
            "visibilitychange",
            'win.addEventListener("online", refreshMetrics)',
            "prefers-reduced-motion: reduce",
            "Math.min(1.5, win.devicePixelRatio || 1)",
            'doc.hidden',
            "setupAuthDialog",
            "setupBountyLauncher",
            "authApiPath",
            "authProviderPath",
            "bountyAssistantLinks",
            "https://chatgpt.com/?prompt=",
            "https://cursor.com/link/prompt?text=",
            "claude://claude.ai/new?q=",
            "cursor://anysphere.cursor-deeplink/prompt?text=",
            'authApiPath("/session", win.location)',
            'authApiPath("/logout", win.location)',
            'dialog.showModal()',
            'passwordToggle.setAttribute("aria-pressed"',
        ],
    )
    for sketch_fallback in ('textContent = "100"', 'textContent = "369"', 'textContent = "2.2"'):
        if sketch_fallback in javascript:
            fail("homepage JavaScript contains a sketch-number fallback")
    require_phrases(
        "solarpunk.css",
        css,
        [
            ".stone-title",
            ".stone-title span:nth-child(1) {\n  font-size: inherit;",
            ".stone-title span:nth-child(3) {",
            ".stone-title em {",
            ".vine-column",
            ".fire-aura",
            ".auth-dialog::backdrop",
            ".bounty-launcher::backdrop",
            "@media (prefers-reduced-motion: reduce)",
            "@media (max-width: 560px)",
        ],
    )
    hero_action_css = re.search(r"\.hero-action\s*\{(?P<body>[^}]*)\}", css)
    if not hero_action_css or re.search(r"(?<![-\w])(?:position|top|left|transform)\s*:", hero_action_css.group("body")):
        fail("the homepage CTA must not use independent absolute positioning")
    if "margin-top: clamp(32px, 4vh, 40px)" not in hero_action_css.group("body"):
        fail("the desktop bounty CTA must retain breathing room below the headline")
    for selector, label in (
        (r"\.scene-nav \.login-preview", "login"),
        (r"\.hero-action button", "post-a-bounty"),
    ):
        match = re.search(selector + r"\s*\{(?P<body>[^}]*)\}", css)
        if not match or "cursor: pointer" not in match.group("body"):
            fail(f"the {label} button must show a hand pointer on hover")
    desktop_title_css = re.search(r"@media\s*\(min-width:\s*821px\)\s*\{\s*\.stone-title\s*\{(?P<body>[^}]*)\}", css)
    if not desktop_title_css or "line-height: 1" not in desktop_title_css.group("body"):
        fail("the desktop hero title must retain its increased line spacing")
    scene_header_css = re.search(r"\.scene-header\s*\{(?P<body>[^}]*)\}", css)
    if not scene_header_css or "position: sticky" not in scene_header_css.group("body") or "top: 0" not in scene_header_css.group("body"):
        fail("the homepage navigation must remain sticky at the top of the viewport")
    for selector in (r"\.stone-title span:nth-child\(1\)", r"\.stone-title span:nth-child\(3\)", r"\.stone-title em"):
        match = re.search(selector + r"\s*\{(?P<body>[^}]*)\}", css)
        if not match or "font-size: inherit" not in match.group("body"):
            fail("every hero headline phrase must inherit one shared font size")
    for relative in (
        "assets/solarpunk/provider-openai.svg",
        "assets/solarpunk/provider-claude.svg",
        "assets/solarpunk/provider-cursor.svg",
    ):
        provider_svg = (site_dir / relative).read_text(encoding="utf-8")
        if "<svg" not in provider_svg or "<path" not in provider_svg:
            fail(f"provider logo is not a usable vector asset: {relative}")
    structured = re.search(r'<script\s+type="application/ld\+json">\s*(\{.*?\})\s*</script>', page, re.DOTALL)
    if not structured:
        fail("homepage must expose JSON-LD identity")
    graph = json.loads(structured.group(1)).get("@graph", [])
    types = [item.get("@type") for item in graph]
    if types.count("WebSite") != 1 or types.count("Organization") != 1:
        fail("homepage JSON-LD must identify one WebSite and one Organization")

    desktop = [site_dir / f"assets/solarpunk/scene-{phase}.webp" for phase in ("dawn", "day", "dusk", "night")]
    mobile = [site_dir / f"assets/solarpunk/scene-{phase}-mobile.webp" for phase in ("dawn", "day", "dusk", "night")]
    character_bytes = sum((site_dir / relative).stat().st_size for relative in EXPECTED_SCENE_ASSETS if "characters-" in relative)
    for images, budget, label in ((desktop, 1_500_000, "desktop"), (mobile, 900_000, "mobile")):
        pair_max = character_bytes + max(images[index].stat().st_size + images[(index + 1) % len(images)].stat().st_size for index in range(len(images)))
        if pair_max > budget:
            fail(f"{label} adjacent scene plates exceed the visible-art budget: {pair_max} bytes")


def check_marketplace(site_dir: Path) -> None:
    board = (site_dir / "earn.html").read_text(encoding="utf-8")
    competition = (site_dir / "competition.html").read_text(encoding="utf-8")
    marketplace = (site_dir / "marketplace.js").read_text(encoding="utf-8")
    detail = (site_dir / "competition.js").read_text(encoding="utf-8")
    css = (site_dir / "marketplace.css").read_text(encoding="utf-8")
    require_phrases(
        "earn.html",
        board,
        [
            "Funded work,<br>one market.",
            "Every visible opportunity passes the readiness rules for its own settlement mechanism.",
            "data-opportunity-list",
            "data-market-timing",
            "All ready opportunities",
            "Starts later",
            "CompetitionSettledV2",
        ],
    )
    require_phrases(
        "competition.html",
        competition,
        [
            "data-competition-app",
            "Decision-grade economics",
            "Your child-bounty funding",
            "If you lose",
            "Expected cash result",
            "Copy prefilled child-bounty brief",
            "One contract-bound machine handoff.",
            "Not entering?",
            "data-abandonment-form",
        ],
    )
    require_phrases(
        "marketplace.js",
        marketplace,
        [
            'item.source_status === "active"',
            '["best_score", "first_proven"]',
            "Boolean(item.evidence_requirements?.verification_policy_hash)",
            'item.source_status === "claimable" && Boolean(item.terms_hash)',
            "Scoring now",
            "Starts in",
            "competition.html?bountyContract=",
            'track("market_view")',
        ],
    )
    require_phrases(
        "competition.js",
        detail,
        [
            "agent-bounties/competition-participation-manifest-v1",
            "competition_view",
            "competition_instructions_copied",
            "competition_template_copied",
            "competition_child_post_started",
            "competition_feedback_started",
            "competition_feedback_submitted",
            "/comments",
            "Expected =",
            "CompetitionSettledV2",
            "hosted_proof_quote",
            "forward-canonical-gmv-attribution-metric-v2",
        ],
    )
    for forbidden in (
        "Open Competition V2 count",
        "V2 market share",
        "autonomous bounty count",
        "bounty type breakdown",
    ):
        if forbidden.lower() in (board + competition + marketplace + detail).lower():
            fail(f"public marketplace exposes an internal type split: {forbidden}")
    require_phrases(
        "marketplace.css",
        css,
        [
            ".opportunity-row",
            ".competition-workspace",
            ".competition-instructions, .economics { min-width: 0; }",
            ".economics-ledger .loss",
            "@media (prefers-reduced-motion: reduce)",
        ],
    )


def check_transactional_handoffs(site_dir: Path) -> None:
    onramp = (site_dir / "onramp.html").read_text(encoding="utf-8")
    shell = (site_dir / "guild-shell.js").read_text(encoding="utf-8")
    require_phrases(
        "onramp.html",
        onramp,
        [
            "Three measured variations",
            "MetaMask Portfolio",
            "Coinbase Base wallet",
            "MoonPay top-up ≠ bounty funding",
            "Base ETH for new-bounty gas",
            "New-bounty creation is not gas-sponsored",
        ],
    )
    onramp_js = (site_dir / "moonpay-onramp.js").read_text(encoding="utf-8")
    require_phrases(
        "moonpay-onramp.js",
        onramp_js,
        [
            "New-bounty creation cannot proceed",
            'asset === "eth" ? "https://www.moonpay.com/buy/eth"',
            'if (provider === "moonpay") track("onramp_moonpay_started")',
        ],
    )
    require_phrases(
        "guild-shell.js",
        shell,
        ['["post.html", "Post"]', '["onramp.html", "Add Base USDC"]', '["metrics.html", "Metrics"]'],
    )
    for removed in ("how-it-works.html",):
        if removed in shell:
            fail(f"guild-shell.js rewrites navigation to removed page {removed}")


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    site_dir = repo_root / "site"
    for relative in sorted(REQUIRED_FILES | EXPECTED_SCENE_ASSETS):
        if not (site_dir / relative).exists():
            fail(f"missing required site file: {relative}")

    html_files = {path.relative_to(site_dir).as_posix() for path in site_dir.rglob("*.html")}
    if html_files != set(CANONICAL_PAGES):
        fail(f"site HTML inventory does not match canonical page policy; found {sorted(html_files)}")
    ui_code = {path.relative_to(site_dir).as_posix() for pattern in ("*.js", "*.css") for path in site_dir.rglob(pattern)}
    if ui_code != ALLOWED_UI_CODE:
        fail(f"orphaned or missing UI code: extra={sorted(ui_code - ALLOWED_UI_CODE)} missing={sorted(ALLOWED_UI_CODE - ui_code)}")
    images = {path.relative_to(site_dir).as_posix() for path in site_dir.rglob("*.webp")}
    if images != EXPECTED_SCENE_ASSETS:
        fail(f"orphaned or missing WebP assets: extra={sorted(images - EXPECTED_SCENE_ASSETS)} missing={sorted(EXPECTED_SCENE_ASSETS - images)}")

    for relative, canonical in CANONICAL_PAGES.items():
        path = site_dir / relative
        text = path.read_text(encoding="utf-8")
        prefix = "../" if relative.startswith("blog/") else ""
        parser = PageParser()
        parser.feed(text)
        if parser.h1_count != 1:
            fail(f"{relative} must contain exactly one h1")
        require_phrases(
            relative,
            text,
            [
                "<title>",
                '<meta name="description"',
                f'<link rel="icon" href="{prefix}favicon.svg" type="image/svg+xml">',
                f'<link rel="canonical" href="{canonical}">',
                f'<script src="{prefix}analytics-config.js?v=2"></script>',
                f'<script src="{prefix}analytics.js?v=3"></script>',
            ],
        )
        if text.index(f'src="{prefix}analytics-config.js?v=2"') > text.index(f'src="{prefix}analytics.js?v=3"'):
            fail(f"{relative}: analytics config must load before analytics.js")
        if relative not in INDEXABLE_PAGES and '<meta name="robots" content="noindex, nofollow">' not in text:
            fail(f"{relative}: transactional handoffs must remain noindex, nofollow")
        for link in parser.links:
            check_internal_link(site_dir, path, link, parser.ids)

    sitemap = ET.parse(site_dir / "sitemap.xml").getroot()
    namespace = {"sm": "http://www.sitemaps.org/schemas/sitemap/0.9"}
    urls = {item.text.strip() for item in sitemap.findall("sm:url/sm:loc", namespace) if item.text}
    expected_urls = {CANONICAL_PAGES[relative] for relative in INDEXABLE_PAGES}
    if urls != expected_urls:
        fail(f"sitemap must list every indexable canonical website page exactly once; found {sorted(urls)}")
    robots = (site_dir / "robots.txt").read_text(encoding="utf-8")
    require_phrases("robots.txt", robots, ["User-agent: OAI-SearchBot", "Sitemap: https://agentbounties.app/sitemap.xml"])

    api_policy = repo_root / "crates" / "api" / "fixtures" / "public-metrics-policy.json"
    site_policy = site_dir / "generated" / "public-metrics-policy.json"
    if api_policy.read_bytes() != site_policy.read_bytes():
        fail("website public metrics policy must match the API fixture byte-for-byte")
    if json_file(site_policy).get("schema_version") != "agent-bounties/public-metrics-policy-v1":
        fail("public metrics policy has the wrong schema")

    protocol = json_file(site_dir / "protocol.json")
    deployment = json_file(repo_root / "deployments" / "base-mainnet.json")
    check_protocol(protocol, deployment)
    check_discovery(site_dir, repo_root, protocol)
    check_a2a_card(site_dir, repo_root)
    check_analytics(site_dir, repo_root)
    check_homepage(site_dir)
    check_marketplace(site_dir)
    check_transactional_handoffs(site_dir)
    check_metrics(site_dir)
    check_blog(site_dir)
    privacy = (site_dir / "privacy.html").read_text(encoding="utf-8")
    terms = (site_dir / "terms.html").read_text(encoding="utf-8")
    require_phrases(
        "privacy.html",
        privacy,
        [
            "Provider access tokens and client secrets are never exposed to the browser",
            "The challenge expires after five minutes and cannot be reused.",
            "Only a confirmed canonical <code>BountySettled</code> or <code>CompetitionSettledV2</code> event proves solver payment",
        ],
    )
    require_phrases(
        "terms.html",
        terms,
        [
            "it does not approve a transaction or move funds",
            "The initialization message is not submitted automatically.",
            "Only a confirmed canonical <code>BountySettled</code> or <code>CompetitionSettledV2</code> event proves solver payment",
        ],
    )
    print("site checks passed: Home, market, competition, transactional handoffs, A2A, blog, Metrics, legal, protocol, evidence, analytics, and asset budgets intact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
