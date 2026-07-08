from __future__ import annotations

import json
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urldefrag, urlparse


REQUIRED_FILES = [
    "index.html",
    "funding.html",
    "terms.html",
    "privacy.html",
    "refunds.html",
    "success.html",
    "cancel.html",
    "styles.css",
    "main.js",
    "llms.txt",
    ".well-known/agent-bounties.json",
    ".nojekyll",
]


class LinkParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.links: list[tuple[str, str]] = []
        self.ids: set[str] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if "id" in values and values["id"]:
            self.ids.add(values["id"] or "")
        for attr in ("href", "src"):
            value = values.get(attr)
            if value:
                self.links.append((attr, value))


def fail(message: str) -> None:
    raise SystemExit(message)


def is_external(url: str) -> bool:
    parsed = urlparse(url)
    return parsed.scheme in {"http", "https", "mailto"}


def check_internal_link(site_dir: Path, source: Path, link: str, ids: set[str]) -> None:
    target, fragment = urldefrag(link)
    if is_external(target) or target.startswith("#"):
        if target.startswith("#") and fragment and fragment not in ids:
            fail(f"{source}: missing local anchor {fragment}")
        return
    if target.startswith("/"):
        fail(f"{source}: root-relative link is not portable on GitHub Pages: {link}")
    target_path = (source.parent / (target or source.name)).resolve()
    try:
        target_path.relative_to(site_dir.resolve())
    except ValueError:
        fail(f"{source}: link escapes site directory: {link}")
    if not target_path.exists():
        fail(f"{source}: missing linked file {link}")


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    site_dir = repo_root / "site"
    for relative in REQUIRED_FILES:
        if not (site_dir / relative).exists():
            fail(f"missing site file: {relative}")

    html_files = sorted(site_dir.glob("*.html"))
    for html_file in html_files:
        parser = LinkParser()
        text = html_file.read_text(encoding="utf-8")
        parser.feed(text)
        if "<title>" not in text or '<meta name="description"' not in text:
            fail(f"{html_file}: missing title or description meta")
        for _attr, link in parser.links:
            check_internal_link(site_dir, html_file, link, parser.ids)

    index = (site_dir / "index.html").read_text(encoding="utf-8")
    funding = (site_dir / "funding.html").read_text(encoding="utf-8")
    main_js = (site_dir / "main.js").read_text(encoding="utf-8")
    llms = (site_dir / "llms.txt").read_text(encoding="utf-8")
    discovery = json.loads((site_dir / ".well-known/agent-bounties.json").read_text())

    required_index_phrases = [
        "Stripe-hosted Checkout",
        "checkout.session.completed",
        "AI judges can route review, but cannot release funds",
        "Fund a bounty",
    ]
    for phrase in required_index_phrases:
        if phrase not in index:
            fail(f"index.html missing required phrase: {phrase}")

    required_funding_phrases = [
        "ENABLE_STRIPE_PUBLIC_CHECKOUT=true",
        "/v1/bounties/",
        "/v1/stripe/live/funding-intents/",
        "No payment credentials were collected here",
        "STRIPE_PAYMENT_METHOD_CONFIGURATION",
        "Check readiness",
        "/health",
        "Hosted API health",
        "/v1/readiness/live-money?network=base-mainnet",
        "stripe_payment_method_configuration_configured",
        "Prefilled funding request",
        "apiBaseUrl",
        "bountyId",
        "amountMinor",
        "funding_source",
        "Plan Base USDC escrow",
        "/v1/base/funding-plan",
        "createEscrow",
        "EscrowCreated",
    ]
    for phrase in required_funding_phrases:
        if phrase not in funding and phrase not in main_js:
            fail(f"funding page missing required phrase: {phrase}")

    if "sk_live" in index + funding + main_js:
        fail("site must not include secret-looking Stripe live keys")
    if "Stripe Checkout funding" not in llms or "PayPal-capable" not in llms:
        fail("llms.txt must orient agents to Stripe Checkout and PayPal-capable funding")
    if discovery.get("open_source") is not True:
        fail("static discovery manifest must advertise open_source=true")
    questions = discovery.get("distribution_feedback", {}).get("questions", [])
    if len(questions) < 4:
        fail("static discovery manifest must include distribution feedback questions")
    hosted_health = discovery.get("hosted_health", {})
    if (
        hosted_health.get("funding_page_action") != "check_health"
        or hosted_health.get("endpoint_template") != "{api_base_url}/health"
        or hosted_health.get("expected_body") != "ok"
    ):
        fail("static discovery manifest must advertise hosted health preflight")
    hosted_readiness = discovery.get("hosted_readiness", {})
    if (
        hosted_readiness.get("funding_page_action") != "check_readiness"
        or hosted_readiness.get("health_preflight") != "{api_base_url}/health"
        or "stripe_payment_method_configuration_configured"
        not in hosted_readiness.get("non_secret_fields", [])
    ):
        fail("static discovery manifest must advertise hosted readiness preflight fields")
    funding_handoff = discovery.get("funding_handoff", {})
    if (
        funding_handoff.get("page") != "https://nspg13.github.io/agent-bounties/funding.html"
        or "apiBaseUrl" not in funding_handoff.get("query_params", [])
        or "bountyId" not in funding_handoff.get("query_params", [])
        or "amountMinor" not in funding_handoff.get("query_params", [])
    ):
        fail("static discovery manifest must advertise public funding handoff query params")
    base_funding_handoff = discovery.get("base_funding_handoff", {})
    if (
        base_funding_handoff.get("endpoint_template") != "{api_base_url}/v1/base/funding-plan"
        or base_funding_handoff.get("supported_rail") != "BaseUsdc"
        or "escrowContract" not in base_funding_handoff.get("query_params", [])
        or "payer" not in base_funding_handoff.get("query_params", [])
        or "EscrowCreated" not in base_funding_handoff.get("settlement_authority", "")
    ):
        fail("static discovery manifest must advertise Base funding plan handoff")

    print("site check ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
