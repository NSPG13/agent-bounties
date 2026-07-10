from __future__ import annotations

import json
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urldefrag, urlparse


REQUIRED_FILES = [
    "index.html",
    "earn.html",
    "post.html",
    "funding.html",
    "operator.html",
    "terms.html",
    "privacy.html",
    "refunds.html",
    "styles.css",
    "favicon.svg",
    "home.js",
    "autonomous.js",
    "protocol.json",
    "llms.txt",
    ".well-known/agent-bounties.json",
    ".nojekyll",
]

CORE_PAGES = ["index.html", "earn.html", "post.html", "funding.html", "operator.html"]
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")


class LinkParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.links: list[str] = []
        self.ids: set[str] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if values.get("id"):
            self.ids.add(values["id"] or "")
        for attr in ("href", "src"):
            if values.get(attr):
                self.links.append(values[attr] or "")


def fail(message: str) -> None:
    raise SystemExit(message)


def require_phrases(label: str, text: str, phrases: list[str]) -> None:
    for phrase in phrases:
        if phrase not in text:
            fail(f"{label} missing required phrase: {phrase}")


def check_internal_link(site_dir: Path, source: Path, link: str, ids: set[str]) -> None:
    target, fragment = urldefrag(link)
    parsed = urlparse(target)
    if parsed.scheme in {"http", "https", "mailto"}:
        return
    if target.startswith("#"):
        if fragment and fragment not in ids:
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
        if not ADDRESS.match(protocol.get("factory") or ""):
            fail("active protocol.json requires a factory address")
        if not ADDRESS.match(protocol.get("implementation") or ""):
            fail("active protocol.json requires an implementation address")
    else:
        if protocol.get("factory") is not None or protocol.get("implementation") is not None:
            fail("pending protocol.json must not advertise undeployed addresses")
    if deployment.get("protocol_version") != protocol.get("protocol_version"):
        fail("site and deployment manifests disagree on protocol version")
    if deployment.get("status") != protocol.get("status"):
        fail("site and deployment manifests disagree on deployment status")
    if deployment.get("factory", {}).get("contract") != protocol.get("factory"):
        fail("site and deployment manifests disagree on factory address")
    if deployment.get("policy", {}).get("operator_settlement_signer") is not False:
        fail("autonomous deployment must not configure a settlement operator")


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    site_dir = repo_root / "site"
    for relative in REQUIRED_FILES:
        if not (site_dir / relative).exists():
            fail(f"missing site file: {relative}")

    for html_file in sorted(site_dir.glob("*.html")):
        parser = LinkParser()
        text = html_file.read_text(encoding="utf-8")
        parser.feed(text)
        if "<title>" not in text or '<meta name="description"' not in text:
            fail(f"{html_file}: missing title or description meta")
        if '<link rel="icon" href="favicon.svg" type="image/svg+xml">' not in text:
            fail(f"{html_file}: missing project favicon")
        for link in parser.links:
            check_internal_link(site_dir, html_file, link, parser.ids)

    if (site_dir / "main.js").exists():
        fail("retired browser settlement bundle site/main.js must not exist")

    pages = {name: (site_dir / name).read_text(encoding="utf-8") for name in CORE_PAGES}
    javascript = (site_dir / "autonomous.js").read_text(encoding="utf-8")
    home_javascript = (site_dir / "home.js").read_text(encoding="utf-8")
    llms = (site_dir / "llms.txt").read_text(encoding="utf-8")
    discovery = json.loads((site_dir / ".well-known/agent-bounties.json").read_text(encoding="utf-8"))
    protocol = json.loads((site_dir / "protocol.json").read_text(encoding="utf-8"))
    deployment = json.loads((repo_root / "deployments" / "base-mainnet.json").read_text(encoding="utf-8"))
    check_protocol(protocol, deployment)

    for name, page in pages.items():
        require_phrases(name, page, ["Post your own bounty", "autonomous.js"])
        if "main.js" in page:
            fail(f"{name} still loads the retired browser settlement bundle")

    for name in ["earn.html", "post.html", "funding.html"]:
        require_phrases(name, pages[name], ["data-protocol-action", "disabled"])

    require_phrases(
        "autonomous.js",
        javascript,
        ["requireActiveProtocol", "No transaction was requested", "[data-protocol-action]"],
    )

    require_phrases("home.js", home_javascript, ["network-canvas", "requestAnimationFrame"])

    require_phrases(
        "index.html",
        pages["index.html"],
        [
            "AI agents earn",
            "Automatic settlement",
            "BountySettled",
            "share verified proof",
            "star and upvote",
        ],
    )
    require_phrases(
        "post.html",
        pages["post.html"],
        [
            "Sign and post bounty",
            "Create unfunded and open it for pooled funding",
            "Deterministic signed verifier",
            "AI judge quorum",
            "Benchmark JSON",
            "Evidence schema JSON",
            "How did you find Agent Bounties?",
        ],
    )
    require_phrases(
        "funding.html",
        pages["funding.html"],
        [
            "Pooled funding",
            "Sign and fund bounty",
            "FundingAdded",
            "BountyBecameClaimable",
            "transaction hash is not funding evidence",
        ],
    )
    require_phrases(
        "earn.html",
        pages["earn.html"],
        [
            "Make money with your AI",
            "Claimable bounties",
            "Submit evidence",
            "Artifact reference",
            "Evidence package JSON",
            "Only a confirmed BountySettled event",
            "star and upvote",
        ],
    )
    require_phrases(
        "operator.html",
        pages["operator.html"],
        [
            "No settlement operator",
            "Escrow #1 refunded",
            "retired contract holds zero USDC",
        ],
    )

    require_phrases(
        "autonomous.js",
        javascript,
        [
            "eth_signTypedData_v4",
            "wallet_sendCalls",
            "create_bounty",
            "eip3009_authorization",
            "/v1/base/autonomous-bounties/terms",
            "/v1/base/autonomous-bounties/creation-plan",
            "/v1/base/autonomous-bounties/contribution-plan",
            "/v1/base/autonomous-bounties/claim-plan",
            "/v1/base/autonomous-bounties/authorized-claim-plan",
            "/v1/base/autonomous-bounties/submission-plan",
            "contract_terms",
            "canonical_bounty_created",
            "bounty_became_claimable",
            "SHA-256",
            "A transaction hash alone is not funding evidence",
        ],
    )

    active_surface = "\n".join(pages.values()) + "\n" + javascript + "\n" + llms
    for retired in [
        "createEscrow",
        "EscrowReleased",
        "/v1/base/release-plan",
        "release(uint256,address[],uint256[],bytes32)",
        "0x150C6dFbCe7803cc7f634f59b0624e87349CEAce",
    ]:
        if retired in active_surface:
            fail(f"active site still advertises retired escrow behavior: {retired}")
    if "sk_live" in active_surface or "private_key" in active_surface.lower():
        fail("active site must not contain secret-looking payment material")

    require_phrases(
        "llms.txt",
        llms,
        [
            "Default CTA: Post your own bounty",
            "agent-bounties/autonomous-v1",
            "pending external review and deployment",
            "list_autonomous_bounties",
            "publish_autonomous_bounty_terms",
            "plan_autonomous_bounty_authorized_creation",
            "plan_autonomous_bounty_authorized_contribution",
            "plan_autonomous_bounty_authorized_claim",
            "list_autonomous_verification_jobs",
            "solver bond",
            "ai_judge_quorum",
            "at least two",
            "BountySettled",
            "How did you find Agent Bounties?",
            "star the repository and upvote the bounty",
            "more and higher-value funded bounties",
            "Stripe and PayPal are future convenience onramps",
        ],
    )

    if discovery.get("schema") != "https://agentbounties.org/schemas/discovery-manifest.v2.json":
        fail("static discovery manifest must use v2")
    if discovery.get("open_source") is not True:
        fail("static discovery manifest must advertise open_source=true")
    if discovery.get("default_cta", {}).get("label") != "Post your own bounty":
        fail("static discovery manifest has the wrong default CTA")
    manifest_protocol = discovery.get("protocol", {})
    if manifest_protocol.get("version") != protocol.get("protocol_version"):
        fail("static discovery manifest protocol version mismatch")
    if manifest_protocol.get("factory") != protocol.get("factory"):
        fail("static discovery manifest factory mismatch")
    if manifest_protocol.get("operator_settlement_signer") is not False:
        fail("static discovery manifest must not advertise a settlement operator")
    if manifest_protocol.get("payout_authority") != "confirmed canonical BountySettled event":
        fail("static discovery manifest must bind payout to BountySettled")
    tools = discovery.get("agent_tools", [])
    for tool in [
        "list_autonomous_bounties",
        "publish_autonomous_bounty_terms",
        "plan_autonomous_bounty_creation",
        "plan_autonomous_bounty_contribution",
        "plan_autonomous_bounty_claim",
        "plan_autonomous_bounty_authorized_claim",
        "list_autonomous_verification_jobs",
        "plan_autonomous_bounty_submission",
        "list_autonomous_bounty_events",
    ]:
        if tool not in tools:
            fail(f"static discovery manifest missing autonomous tool: {tool}")
    if any(tool in tools for tool in ["plan_base_funding", "plan_base_release", "plan_base_refund"]):
        fail("static discovery manifest advertises retired escrow tools")
    ai_mode = next(
        (item for item in discovery.get("verification_modes", []) if item.get("name") == "ai_judge_quorum"),
        None,
    )
    if not ai_mode or ai_mode.get("minimum_threshold") != 2:
        fail("AI judge mode must require at least two committed signers")
    actions = discovery.get("post_value_loop", {}).get("actions", [])
    if [item.get("kind") for item in actions] != [
        "share_verified_value",
        "tell_your_human",
        "star_upvote_repo",
        "post_own_bounty",
        "claim_next_bounty",
    ]:
        fail("static discovery manifest has an invalid post-value loop")
    if len(discovery.get("distribution_feedback", {}).get("questions", [])) != 4:
        fail("static discovery manifest must ask the four distribution questions")

    print("site check ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
