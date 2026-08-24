from __future__ import annotations

import json
import re
import xml.etree.ElementTree as ET
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urldefrag, urlparse


CANONICAL_PAGES = {
    "index.html": "https://agentbounties.app/",
    "metrics.html": "https://agentbounties.app/metrics.html",
    "privacy.html": "https://agentbounties.app/privacy.html",
    "terms.html": "https://agentbounties.app/terms.html",
}
REQUIRED_FILES = {
    ".nojekyll",
    ".well-known/agent-bounties.json",
    ".well-known/x402.json",
    "agent/index.md",
    "analytics-config.js",
    "analytics.js",
    "favicon.svg",
    "generated/github-participation.json",
    "generated/public-metrics-policy.json",
    "index.html",
    "llms.txt",
    "metrics.css",
    "metrics.html",
    "metrics.js",
    "privacy.html",
    "protocol.json",
    "robots.txt",
    "sitemap.xml",
    "solarpunk-home.js",
    "solarpunk.css",
    "styles.css",
    "terms.html",
    "guild-pages.css",
    "x402-test-vectors.json",
}
ALLOWED_UI_CODE = {
    "analytics-config.js",
    "analytics.js",
    "guild-pages.css",
    "metrics.css",
    "metrics.js",
    "solarpunk-home.js",
    "solarpunk.css",
    "styles.css",
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
    if discovery.get("schema") != "https://agentbounties.org/schemas/discovery-manifest.v2.json":
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
    require_phrases("llms.txt", llms, ["get_bounty_feed", "Only `BountySettled` proves payment."])
    require_phrases(
        "agent/index.md",
        agent_markdown,
        ["No computer use is required", "get_bounty_feed", "CompetitionSettledV2"],
    )
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
    if page.count("<button type=\"button\" disabled>") < 2:
        fail("unfinished About and Find controls must remain native disabled buttons")
    hero_start = page.find('<section class="hero"')
    hero_end = page.find("</section>", hero_start)
    hero_action = page.find('<div class="hero-action">', hero_start)
    if hero_start < 0 or hero_end < 0 or not hero_start < hero_action < hero_end:
        fail("the homepage CTA must remain in the hero flow to prevent headline overlap")
    stylesheet_version = re.search(r'<link rel="stylesheet" href="solarpunk\.css\?v=(\d+)">', page)
    if not stylesheet_version or int(stylesheet_version.group(1)) < 10:
        fail("the homepage must load the flow-layout stylesheet through a cache-busted URL")
    require_phrases(
        "index.html bounty assistant launcher",
        page,
        [
            'data-bounty-open aria-haspopup="dialog"',
            'data-bounty-launcher aria-labelledby="bounty-launcher-title"',
            'data-bounty-assistant="gpt"',
            'data-bounty-assistant="claude"',
            'data-bounty-assistant="cursor"',
            'data-bounty-assistant="custom"',
            "The prompt is never submitted automatically.",
        ],
    )
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
    for removed in ("earn.html", "post.html", "how-it-works.html"):
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
    for selector in (r"\.stone-title span:nth-child\(1\)", r"\.stone-title span:nth-child\(3\)", r"\.stone-title em"):
        match = re.search(selector + r"\s*\{(?P<body>[^}]*)\}", css)
        if not match or "font-size: inherit" not in match.group("body"):
            fail("every hero headline phrase must inherit one shared font size")
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


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    site_dir = repo_root / "site"
    for relative in sorted(REQUIRED_FILES | EXPECTED_SCENE_ASSETS):
        if not (site_dir / relative).exists():
            fail(f"missing required site file: {relative}")

    html_files = {path.relative_to(site_dir).as_posix() for path in site_dir.rglob("*.html")}
    if html_files != set(CANONICAL_PAGES):
        fail(f"site must expose only Home, Metrics, and required legal HTML; found {sorted(html_files)}")
    ui_code = {path.relative_to(site_dir).as_posix() for pattern in ("*.js", "*.css") for path in site_dir.rglob(pattern)}
    if ui_code != ALLOWED_UI_CODE:
        fail(f"orphaned or missing UI code: extra={sorted(ui_code - ALLOWED_UI_CODE)} missing={sorted(ALLOWED_UI_CODE - ui_code)}")
    images = {path.relative_to(site_dir).as_posix() for path in site_dir.rglob("*.webp")}
    if images != EXPECTED_SCENE_ASSETS:
        fail(f"orphaned or missing WebP assets: extra={sorted(images - EXPECTED_SCENE_ASSETS)} missing={sorted(EXPECTED_SCENE_ASSETS - images)}")

    for relative, canonical in CANONICAL_PAGES.items():
        path = site_dir / relative
        text = path.read_text(encoding="utf-8")
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
                '<link rel="icon" href="favicon.svg" type="image/svg+xml">',
                f'<link rel="canonical" href="{canonical}">',
                '<script src="analytics-config.js?v=2"></script>',
                '<script src="analytics.js?v=2"></script>',
            ],
        )
        if text.index('src="analytics-config.js?v=2"') > text.index('src="analytics.js?v=2"'):
            fail(f"{relative}: analytics config must load before analytics.js")
        for link in parser.links:
            check_internal_link(site_dir, path, link, parser.ids)

    sitemap = ET.parse(site_dir / "sitemap.xml").getroot()
    namespace = {"sm": "http://www.sitemaps.org/schemas/sitemap/0.9"}
    urls = {item.text.strip() for item in sitemap.findall("sm:url/sm:loc", namespace) if item.text}
    if urls != set(CANONICAL_PAGES.values()):
        fail(f"sitemap must list only Home, Metrics, and required legal pages; found {sorted(urls)}")
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
    check_analytics(site_dir, repo_root)
    check_homepage(site_dir)
    check_metrics(site_dir)
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
    print("site checks passed: Home + Metrics + required legal pages; protocol, evidence, analytics, and asset budgets intact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
