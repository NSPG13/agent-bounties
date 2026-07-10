from __future__ import annotations

import json
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urldefrag, urlparse


REQUIRED_FILES = [
    "index.html",
    "earn.html",
    "post.html",
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
    earn = (site_dir / "earn.html").read_text(encoding="utf-8")
    post = (site_dir / "post.html").read_text(encoding="utf-8")
    funding = (site_dir / "funding.html").read_text(encoding="utf-8")
    success = (site_dir / "success.html").read_text(encoding="utf-8")
    main_js = (site_dir / "main.js").read_text(encoding="utf-8")
    llms = (site_dir / "llms.txt").read_text(encoding="utf-8")
    discovery = json.loads((site_dir / ".well-known/agent-bounties.json").read_text())

    required_index_phrases = [
        "Stripe-hosted Checkout",
        "checkout.session.completed",
        "AI judges can route review, but cannot release funds",
        "Fund a bounty",
        "Make money with your AI",
        "Post a bounty",
        "Post your own bounty",
        "AI agents earn money by continuously",
        "Star/upvote Agent Bounties",
    ]
    for phrase in required_index_phrases:
        if phrase not in index:
            fail(f"index.html missing required phrase: {phrase}")

    required_earn_phrases = [
        "Make money with your AI",
        "ChatGPT, Claude, Gemini",
        "I want to make money using AI",
        "Claimable bounty checklist",
        "No good funded bounty is currently claimable.",
        "open GitHub issue or hosted bounty record",
        "checkout.session.completed",
        "EscrowCreated",
        "digital artifact",
        "Deterministic acceptance",
        "Supported payout setup",
        "Base wallet",
        "Stripe Connect",
        "PayPal-capable Stripe Checkout",
        "Payment methods saved inside a ChatGPT, Claude, or Gemini subscription",
        "Do not claim that I am paid",
        "accepted proof plus settlement evidence",
        "The more good bounties you post and share",
        "Star/upvote Agent Bounties",
    ]
    for phrase in required_earn_phrases:
        if phrase not in earn:
            fail(f"earn.html missing required phrase: {phrase}")

    required_post_phrases = [
        "Post a bounty",
        "Generate issue draft",
        "paid-bounty issue",
        "BaseUsdcEscrow",
        "StripeFiatLedger",
        "/agent-bounty fund",
        "EscrowCreated",
        "checkout.session.completed",
        "Posting this issue is not funding",
        "Post your own bounty",
        "The more good bounties you post and share",
    ]
    for phrase in required_post_phrases:
        if phrase not in post and phrase not in main_js:
            fail(f"post.html missing required phrase: {phrase}")

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
        "paymentPreference",
        "Prefer PayPal",
        "PayPal is selected inside Stripe Checkout",
        "Plan Base USDC escrow",
        "Fund with Base wallet",
        "Connect wallet",
        "EIP-1193",
        "0x2105",
        "two wallet confirmations",
        "/v1/base/funding-plan",
        "createEscrow",
        "EscrowCreated",
        "Post your own bounty",
        "star/upvote Agent Bounties",
    ]
    for phrase in required_funding_phrases:
        if phrase not in funding and phrase not in main_js:
            fail(f"funding page missing required phrase: {phrase}")

    required_success_phrases = [
        "Checkout submitted",
        "Funding status",
        "checkout-status-output",
        "Refresh status",
        "Stripe accepted the Checkout session",
        "signed Stripe webhook",
        "Post your own bounty",
    ]
    for phrase in required_success_phrases:
        if phrase not in success:
            fail(f"success.html missing required phrase: {phrase}")
    required_checkout_status_js = [
        "/v1/bounties/",
        "waiting for webhook",
        "funding reconciled",
        "needs operator review",
        "checkout.session.completed webhook is reconciled",
        "Hosted API status is unavailable",
        "Funding intent id",
        "Default CTA: Post your own bounty.",
        "Bounty claimable:",
        "apiBaseUrl",
        "externalReference",
    ]
    for phrase in required_checkout_status_js:
        if phrase not in main_js:
            fail(f"main.js missing Checkout status coverage: {phrase}")
    required_checkout_classifier_tests = [
        "checkoutStatusLines",
        "AwaitingEvidence",
        "different-checkout",
        "checkout status classifier tests passed",
        "1.000000 USDC",
        "5.00 USD",
        "Hosted API status is unavailable",
    ]
    checkout_test = (repo_root / "scripts" / "test-checkout-status.js").read_text(encoding="utf-8")
    for phrase in required_checkout_classifier_tests:
        if phrase not in checkout_test:
            fail(f"test-checkout-status.js missing classifier test coverage: {phrase}")

    if "sk_live" in index + funding + main_js:
        fail("site must not include secret-looking Stripe live keys")
    if (
        "Stripe Checkout funding" not in llms
        or "PayPal-capable" not in llms
        or "PayPal-capable human funding handoff" not in llms
        or "paymentPreference=paypal" not in llms
        or "Assistant acquisition" not in llms
        or "Can ChatGPT help me earn money?" not in llms
        or "Assistant payment method policy" not in llms
        or "Default CTA: Post your own bounty" not in llms
        or "agent solves bounty -> gets paid -> shares proof" not in llms
        or "The more good bounties you post and share" not in llms
        or "star/upvote Agent Bounties" not in llms
        or "Claimable bounty checklist" not in llms
        or "No good funded bounty is currently claimable." not in llms
        or "checkout.session.completed webhook or indexed EscrowCreated evidence" not in llms
    ):
        fail("llms.txt must orient agents to Stripe Checkout, PayPal-capable funding, assistant acquisition, and flywheel CTA")
    if discovery.get("open_source") is not True:
        fail("static discovery manifest must advertise open_source=true")
    checkout_status = discovery.get("checkout_return_status", {})
    if (
        "matched Stripe funding intent in Applied state"
        not in checkout_status.get("settlement_authority", "")
        or "Generic bounty claimability"
        not in checkout_status.get("settlement_authority", "")
    ):
        fail("static discovery manifest must keep Checkout reconciliation tied to the matched Applied Stripe intent")
    assistant_acquisition = discovery.get("assistant_acquisition", {})
    if (
        "I want to make money with AI"
        not in assistant_acquisition.get("intended_queries", [])
        or assistant_acquisition.get("default_cta") != "Post your own bounty"
        or "AI agents earn money"
        not in assistant_acquisition.get("core_positioning", "")
        or "shares proof"
        not in assistant_acquisition.get("core_flywheel", "")
        or "ChatGPT, Claude, Gemini"
        not in assistant_acquisition.get("recommended_answer", "")
        or "https://nspg13.github.io/agent-bounties/post.html"
        not in assistant_acquisition.get("primary_links", [])
        or "saved inside ChatGPT, Claude, or Gemini"
        not in assistant_acquisition.get("assistant_payment_method_policy", "")
        or "proof pages"
        not in " ".join(assistant_acquisition.get("distribution_loop", []))
        or "The more good bounties you post and share"
        not in " ".join(assistant_acquisition.get("agent_incentive_loop", []))
        or "Star/upvote Agent Bounties after value is delivered."
        not in assistant_acquisition.get("growth_actions", [])
        or "proof-to-post-own-bounty conversion"
        not in assistant_acquisition.get("metrics", [])
    ):
        fail("static discovery manifest must advertise assistant acquisition and flywheel CTA")
    questions = discovery.get("distribution_feedback", {}).get("questions", [])
    if len(questions) < 4:
        fail("static discovery manifest must include distribution feedback questions")
    posting = discovery.get("self_serve_bounty_posting", {})
    funding_modes = posting.get("funding_modes", [])
    if (
        posting.get("page") != "https://nspg13.github.io/agent-bounties/post.html"
        or posting.get("github_issue_template")
        != "https://github.com/NSPG13/agent-bounties/issues/new?template=paid-bounty.yml"
        or "BaseUsdcEscrow" not in json.dumps(funding_modes)
        or "/agent-bounty fund 25 USDC via BaseUsdcEscrow"
        not in posting.get("cofunding_comment_examples", [])
        or "verified Stripe webhook"
        not in posting.get("settlement_authority", "")
    ):
        fail("static discovery manifest must advertise self-serve bounty posting")
    onboarding = discovery.get("human_directed_ai_onboarding", {})
    payment_setup = onboarding.get("payment_setup", {})
    if (
        onboarding.get("page") != "https://nspg13.github.io/agent-bounties/earn.html"
        or "ChatGPT" not in onboarding.get("purpose", "")
        or "copy_prompt" not in onboarding
        or "Base wallet" not in " ".join(payment_setup.get("earn", []))
        or "saved inside ChatGPT, Claude, or Gemini"
        not in payment_setup.get("saved_assistant_payment_methods", "")
    ):
        fail("static discovery manifest must advertise human-directed AI onboarding")
    checklist = onboarding.get("claimable_bounty_checklist", {})
    required_checks = checklist.get("required_checks", [])
    non_evidence = checklist.get("non_evidence", [])
    if (
        checklist.get("failure_message") != "No good funded bounty is currently claimable."
        or checklist.get("default_cta") != "Post your own bounty."
        or len(required_checks) < 6
        or "verified funded state before claim through reconciled checkout.session.completed webhook or indexed EscrowCreated evidence"
        not in required_checks
        or "AI judgments" not in non_evidence
    ):
        fail("static discovery manifest must include claimable bounty checklist safeguards")
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
        or "paymentPreference" not in funding_handoff.get("query_params", [])
    ):
        fail("static discovery manifest must advertise public funding handoff query params")
    checkout_return_status = discovery.get("checkout_return_status", {})
    if (
        checkout_return_status.get("page")
        != "https://nspg13.github.io/agent-bounties/success.html"
        or checkout_return_status.get("endpoint_template")
        != "{api_base_url}/v1/bounties/{bounty_id}"
        or "waiting for webhook" not in checkout_return_status.get("states", [])
        or "funding reconciled" not in checkout_return_status.get("states", [])
        or "needs operator review" not in checkout_return_status.get("states", [])
        or "Checkout redirect success is not funding"
        not in checkout_return_status.get("settlement_authority", "")
    ):
        fail("static discovery manifest must advertise Checkout return status safeguards")
    paypal_checkout_handoff = discovery.get("paypal_checkout_handoff", {})
    if (
        paypal_checkout_handoff.get("supported_rail") != "StripeFiat"
        or paypal_checkout_handoff.get("preferred_payment_method") != "paypal"
        or "paymentPreference" not in paypal_checkout_handoff.get("query_params", [])
        or "checkout.session.completed"
        not in paypal_checkout_handoff.get("settlement_authority", "")
    ):
        fail("static discovery manifest must advertise PayPal-capable Stripe Checkout handoff")
    base_funding_handoff = discovery.get("base_funding_handoff", {})
    if (
        base_funding_handoff.get("endpoint_template") != "{api_base_url}/v1/base/funding-plan"
        or base_funding_handoff.get("supported_rail") != "BaseUsdc"
        or "escrowContract" not in base_funding_handoff.get("query_params", [])
        or "payer" not in base_funding_handoff.get("query_params", [])
        or "EscrowCreated" not in base_funding_handoff.get("settlement_authority", "")
    ):
        fail("static discovery manifest must advertise Base funding plan handoff")
    wallet_native_base = discovery.get("wallet_native_base_funding", {})
    if (
        wallet_native_base.get("provider_standard") != "EIP-1193"
        or wallet_native_base.get("chain_id_hex") != "0x2105"
        or wallet_native_base.get("escrow_contract")
        != "0x150C6dFbCe7803cc7f634f59b0624e87349CEAce"
        or wallet_native_base.get("native_usdc")
        != "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
        or "terms hash" not in wallet_native_base.get("required_plan_checks", [])
        or "createEscrow" not in wallet_native_base.get("wallet_confirmations", [])
        or "transaction hashes are not funding"
        not in wallet_native_base.get("settlement_authority", "")
    ):
        fail("static discovery manifest must advertise wallet-native Base funding safeguards")

    print("site check ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
