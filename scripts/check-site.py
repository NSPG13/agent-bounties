from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urldefrag, urlparse


REQUIRED_FILES = [
    "index.html",
    "earn.html",
    "post.html",
    "funding.html",
    "x402.html",
    "x402-test-vectors.json",
    "prepare-agent.html",
    "agent-budget.html",
    "agent-budget.js",
    "operator.html",
    "recovery.html",
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
    ".well-known/x402.json",
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
    recovery_page = (site_dir / "recovery.html").read_text(encoding="utf-8")
    javascript = (site_dir / "autonomous.js").read_text(encoding="utf-8")
    home_javascript = (site_dir / "home.js").read_text(encoding="utf-8")
    llms = (site_dir / "llms.txt").read_text(encoding="utf-8")
    discovery = json.loads((site_dir / ".well-known/agent-bounties.json").read_text(encoding="utf-8"))
    x402_discovery = json.loads((site_dir / ".well-known/x402.json").read_text(encoding="utf-8"))
    x402_vectors = json.loads((site_dir / "x402-test-vectors.json").read_text(encoding="utf-8"))
    protocol = json.loads((site_dir / "protocol.json").read_text(encoding="utf-8"))
    deployment = json.loads((repo_root / "deployments" / "base-mainnet.json").read_text(encoding="utf-8"))
    bounded_deployment = json.loads(
        (repo_root / "deployments" / "bounded-agent-wallet-base-mainnet.json").read_text(encoding="utf-8")
    )
    standing_meta_deployment = json.loads(
        (repo_root / "deployments" / "standing-meta-v2-base-mainnet.json").read_text(encoding="utf-8")
    )
    bounded_page = (site_dir / "agent-budget.html").read_text(encoding="utf-8")
    bounded_javascript = (site_dir / "agent-budget.js").read_text(encoding="utf-8")
    pages_workflow = (repo_root / ".github" / "workflows" / "pages.yml").read_text(encoding="utf-8")
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
        [
            "requireActiveProtocol",
            "No transaction was requested",
            "[data-protocol-action]",
            "eth_requestAccounts",
        ],
    )

    require_phrases(
        "recovery.html",
        recovery_page,
        [
            'id="legacy-recovery-form"',
            "Cancel and recover 3 USDC",
            "0x786be3f994365fcd417a1b502a83300ea87d9b34",
            "0x481dfc6f45d43b89dfcc1a84fd6d9b5f73a6a0b9",
            "0x3195aebfc39a069bf1a4420951d0babc99b2b612",
            "Only the exact creator wallet and six pinned zero-value calls are accepted.",
            "autonomous.js",
        ],
    )
    require_phrases(
        "autonomous.js legacy recovery",
        javascript,
        [
            'creator: "0x884834e884d6e93462655a2820140ad03e6747bc"',
            'factory: "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"',
            'implementation: "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9"',
            'usdc: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"',
            'cancel: "0xea8a1af0"',
            'withdrawRefund: "0x110f8874"',
            "value.code !== expectedCloneRuntime()",
            "value.solver !== \"0x0000000000000000000000000000000000000000\" || value.bond !== 0n",
            "value.funded === 0n",
            "value.contribution === 0n",
            "value.balance === 0n",
            'value: "0x0"',
        ],
    )
    if "import wallet" in recovery_page.lower() or "private key" in recovery_page.lower():
        fail("legacy recovery must use connect-wallet onboarding only")

    public_wallet_surface = pages["earn.html"] + pages["post.html"] + pages["funding.html"]
    if "Connect wallet" not in public_wallet_surface:
        fail("public transaction pages must expose a connect-wallet flow")
    if "import wallet" in public_wallet_surface.lower():
        fail("public transaction pages must never expose wallet-import onboarding")
    if 'name="apiBaseUrl"' in public_wallet_surface:
        fail("public transaction pages must use the deployed API from protocol.json")

    require_phrases(
        "home.js",
        home_javascript,
        [
            "network-canvas",
            "requestAnimationFrame",
            "home-live-inventory",
            "/v1/opportunities",
            "Ready to earn",
            "Open opportunities",
            "Seeking funding",
            "In progress",
            "Recently paid",
            "payment_state",
            "payment_committed",
            "verification_ready",
            "Meta-bounty:",
            'timeZone: "UTC"',
            "end.getTime() - 1",
        ],
    )

    require_phrases(
        "index.html",
        pages["index.html"],
        [
            "AI agents earn",
            "3 USDC daily. 26 USDC weekly.",
            "BountySettled",
            "Share proof",
            "star the repository",
            "Funded work available now",
            "Each creator counts once",
            "Rank is not payment",
            "share verified proof",
            "star and upvote",
            "Opportunities across every payment state",
            "Open opportunity",
            "does not imply payment",
        ],
    )
    require_phrases(
        "post.html",
        pages["post.html"],
        [
            "Sign and post bounty",
            "Post with 0 USDC now and open it to funding later",
            "Fund on creation",
            "16-bit work-proof canary",
            "Verifier wallet quorum (advanced)",
            "AI judge quorum (advanced)",
            "Benchmark JSON (payout condition)",
            "Evidence record schema (hash-bound context)",
            "Choose it only when that proof is the payout condition",
            "How did you find Agent Bounties?",
            "Draft measurable terms",
            "cloud draft is advisory",
        ],
    )
    require_phrases(
        "earn.html unfunded discovery",
        pages["earn.html"],
        [
            "Unfunded bounties",
            "not claimable and promise no payment",
            "list_unfunded_bounties",
            "submit_unfunded_bounty_solution",
        ],
    )
    require_phrases(
        "funding.html",
        pages["funding.html"],
        [
            "Pooled funding",
            "Sign and fund bounty",
            "FundingAdded",
            "Stop only after that event",
            "transaction hash is not funding",
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
            "star the repository",
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
            "/v1/base/autonomous-bounties/claims",
            "/v1/cloud-agent/readiness",
            "/v1/cloud-agent/bounty-drafts",
            "request_bond_sponsorship",
            "wallet_signature",
            "canonical_event_id",
            "/v1/base/autonomous-bounties/submission-plan",
            "contract_terms",
            "canonical_bounty_created",
            "bounty_became_claimable",
            "SHA-256",
            "A transaction hash alone is not funding evidence",
            'params.get("amount")',
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
    if "/v1/base/autonomous-bounties/authorized-claim-plan" in javascript:
        fail("browser earning flow must use the hosted one-signature claim path")
    if "sk_live" in active_surface or "private_key" in active_surface.lower():
        fail("active site must not contain secret-looking payment material")

    require_phrases(
        "llms.txt",
        llms,
        [
            "Default CTA: Post your own bounty",
            "Do not skip steps",
            "get_solver_leaderboard",
            "Prize: 3 USDC",
            "Prize: 26 USDC",
            "prepare_agent_to_earn",
            "agent_native_claim",
            "fund_bounty_with_x402",
            "prepare_autonomous_bounty_submission",
            "list_autonomous_verification_jobs",
            "BountySettled",
            "draft_bounty_with_cloud_agent",
            "inventory-summary",
            "Inventory unavailable:",
        ],
    )

    if discovery.get("schema") != "https://agentbounties.org/schemas/discovery-manifest.v2.json":
        fail("static discovery manifest must use v2")
    if discovery.get("open_source") is not True:
        fail("static discovery manifest must advertise open_source=true")
    if discovery.get("default_cta", {}).get("label") != "Post your own bounty":
        fail("static discovery manifest has the wrong default CTA")
    live_inventory = discovery.get("live_inventory", {})
    if "claimable_only=true" not in live_inventory.get("claimable_feed", ""):
        fail("static discovery manifest must expose the canonical claimable feed")
    if "inventory-summary" not in live_inventory.get("summary", ""):
        fail("static discovery manifest must expose the live inventory summary")
    if "inventory-badge.svg" not in live_inventory.get("badge", ""):
        fail("static discovery manifest must expose the live inventory badge")
    if "not guaranteed profit" not in live_inventory.get("standing_meta_economics", ""):
        fail("static discovery manifest must disclose standing-meta economics")
    manifest_protocol = discovery.get("protocol", {})
    if manifest_protocol.get("version") != protocol.get("protocol_version"):
        fail("static discovery manifest protocol version mismatch")
    if manifest_protocol.get("factory") != protocol.get("factory"):
        fail("static discovery manifest factory mismatch")
    if manifest_protocol.get("implementation") != protocol.get("implementation"):
        fail("static discovery manifest implementation mismatch")
    if manifest_protocol.get("operator_settlement_signer") is not False:
        fail("static discovery manifest must not advertise a settlement operator")
    if manifest_protocol.get("payout_authority") != "confirmed canonical BountySettled event":
        fail("static discovery manifest must bind payout to BountySettled")
    default_verification = protocol.get("default_verification", {})
    if default_verification.get("mode") != "deterministic_module":
        fail("public posting must default to deterministic-module verification")
    if default_verification.get("module_id") not in protocol.get("deterministic_modules", {}):
        fail("default deterministic verifier must reference a deployed protocol module")
    if default_verification.get("verifier_reward_recipient") != "creator_wallet":
        fail("default deterministic verifier reward recipient must be the creator wallet")
    if default_verification.get("threshold") != 1:
        fail("default deterministic verifier threshold must be one")
    default_module = protocol["deterministic_modules"][default_verification["module_id"]]
    expected_work_benchmark = {
        "engine": "leading_zero_work_v1",
        "difficulty_bits": 16,
        "hash_function": "keccak256",
        "preimage_abi_types": [
            "bytes32",
            "uint64",
            "address",
            "bytes32",
            "bytes32",
            "bytes32",
            "uint256",
        ],
        "proof_encoding": "abi.encode(uint256 nonce)",
        "verifier_module": default_module.get("contract"),
        "reference_command": "cargo run -p cli -- autonomous-mine-work-proof",
    }
    if default_module.get("usage") != "protocol_canary_only":
        fail("default work verifier must be scoped to protocol canaries")
    if default_module.get("benchmark") != expected_work_benchmark:
        fail("default work verifier benchmark does not match its exact contract semantics")
    if '{"engine":"github_ci"' in pages["post.html"]:
        fail("public posting must not pair GitHub CI with the leading-zero work verifier")
    tools = discovery.get("agent_tools", [])
    for tool in [
        "list_autonomous_bounties",
        "publish_autonomous_bounty_terms",
        "plan_autonomous_canonical_child_terms",
        "prepare_standing_meta_v2_child",
        "plan_autonomous_bounty_creation",
        "plan_autonomous_bounty_contribution",
        "agent_native_claim",
        "plan_autonomous_bounty_claim",
        "plan_autonomous_bounty_authorized_claim",
        "list_autonomous_verification_jobs",
        "plan_autonomous_bounty_submission",
        "prepare_autonomous_bounty_submission",
        "plan_autonomous_bounty_submission_authorization",
        "relay_autonomous_action_via_github_comment",
        "fund_bounty_with_x402",
        "list_autonomous_bounty_events",
    ]:
        if tool not in tools:
            fail(f"static discovery manifest missing autonomous tool: {tool}")
    if any(tool in tools for tool in ["plan_base_funding", "plan_base_release", "plan_base_refund"]):
        fail("static discovery manifest advertises retired escrow tools")
    modes = {mode.get("name"): mode for mode in discovery.get("verification_modes", [])}
    deterministic_mode = modes.get("deterministic_module", {})
    if deterministic_mode.get("default_for_new_bounties") is not True:
        fail("discovery must default new bounties to deterministic verification")
    expected_module = protocol["deterministic_modules"]["leading_zero_work_v1"]["contract"]
    if deterministic_mode.get("default_module") != expected_module:
        fail("discovery default verifier module does not match protocol status")
    for advanced_mode in ("signed_quorum", "ai_judge_quorum"):
        if modes.get(advanced_mode, {}).get("default_for_new_bounties") is not False:
            fail(f"advanced verifier mode must not be a posting default: {advanced_mode}")
    funding = discovery.get("funding", {})
    if "wallet_signature" not in funding.get("gas_sponsorship", ""):
        fail("static discovery manifest must describe native claim signature replay")
    if funding.get("default_verification") != "deterministic_module":
        fail("discovery funding policy has the wrong verification default")
    if funding.get("default_verifier_module") != expected_module:
        fail("discovery funding policy has the wrong default verifier module")
    if "/agent-bounty relay" not in funding.get("gas_sponsorship", ""):
        fail("discovery funding policy does not advertise bounded gas sponsorship")
    x402_funding = funding.get("x402", {})
    if x402_funding.get("version") != 2 or x402_funding.get("scheme") != "agent-bounty-fund":
        fail("discovery funding policy must advertise x402 v2 agent-bounty-fund")
    if "FundingAdded" not in x402_funding.get("settlement_boundary", ""):
        fail("x402 funding policy must bind evidence to FundingAdded")
    if discovery.get("endpoints", {}).get("x402_discovery") != "https://api.bountyboard.global/.well-known/x402.json":
        fail("static discovery manifest has the wrong x402 discovery endpoint")
    if x402_discovery.get("x402Version") != 2:
        fail("static x402 discovery must use version 2")
    resources = {item.get("name"): item for item in x402_discovery.get("resources", [])}
    canonical_funding = resources.get("canonical-bounty-funding", {})
    if canonical_funding.get("scheme") != "agent-bounty-fund":
        fail("static x402 discovery must use the canonical funding scheme")
    if canonical_funding.get("genericExactCompatible") is not False:
        fail("static x402 discovery must reject generic exact bounty funding")
    if "FundingAdded" not in canonical_funding.get("settlement", ""):
        fail("static x402 discovery must bind funding state to FundingAdded")
    if x402_discovery.get("mpp", {}).get("status") != "planned":
        fail("static x402 discovery must keep MPP behind the planned adapter boundary")
    x402_docs = x402_discovery.get("documentation", {})
    if x402_docs.get("compatibility") != "https://bountyboard.global/x402.html":
        fail("static x402 discovery must publish the compatibility page")
    if x402_docs.get("testVectors") != "https://bountyboard.global/x402-test-vectors.json":
        fail("static x402 discovery must publish deterministic test vectors")
    if x402_vectors.get("schema_version") != "agent-bounties/x402-test-vectors-v1":
        fail("x402 test vectors have the wrong schema")
    if x402_vectors.get("scheme") != "agent-bounty-fund":
        fail("x402 vectors must exercise the custom funding scheme")
    vectors = {item.get("id"): item for item in x402_vectors.get("vectors", [])}
    for vector_id in [
        "valid_custom_bounty_funding",
        "reject_standard_exact_direct_transfer",
        "pending_relay_is_not_funding",
        "confirmed_funding",
        "solver_payment_boundary",
    ]:
        if vector_id not in vectors:
            fail(f"missing x402 test vector: {vector_id}")
    if vectors["pending_relay_is_not_funding"].get("expected", {}).get("funded") is not False:
        fail("pending x402 relay vector must remain non-evidence")
    if vectors["confirmed_funding"].get("expected", {}).get("paid") is not False:
        fail("FundingAdded vector must not claim solver payment")
    if vectors["solver_payment_boundary"].get("input", {}).get("canonical_event") != "BountySettled":
        fail("x402 payout vector must bind payment to BountySettled")
    x402_page = (site_dir / "x402.html").read_text(encoding="utf-8")
    require_phrases(
        "x402.html",
        x402_page,
        [
            "Agent Bounties x402 compatibility",
            "agent-bounty-fund",
            "not the standard <code>exact</code>",
            "FundingAdded",
            "BountySettled",
            "x402-test-vectors.json",
            "Post your own bounty",
        ],
    )
    prepare_agent_page = (site_dir / "prepare-agent.html").read_text(encoding="utf-8")
    require_phrases(
        "prepare-agent.html",
        prepare_agent_page,
        [
            "Prepare an agent to earn",
            "/v1/base/agent-wallet/readiness",
            "prepare_agent_to_earn",
            "allowed_chain_ids",
            "human_approval_policy",
            "Never send a private key",
            "agent-budget.html",
            "Post your own bounty",
        ],
    )
    require_phrases(
        "agent-budget.html",
        bounded_page,
        [
            "Authorize an agent budget",
            "Connect wallet",
            "Agent delegate address",
            "Initial funding, USDC",
            "Lifetime gross spend, USDC",
            "two-wallet sandboxed-regression quorum only",
            "Owner escape hatch",
            "Review policy update",
            "Update policy",
            "not independently audited",
            "Post your own bounty",
            "agent-budget.js",
        ],
    )
    if "import wallet" in bounded_page.lower() or any(
        marker in bounded_page.lower()
        for marker in ['name="private', 'name="seed', 'name="mnemonic', 'type="password"']
    ):
        fail("agent budget activation must use connect-wallet onboarding only")
    require_phrases(
        "agent-budget.js",
        bounded_javascript,
        [
            'CHAIN_ID = "0x2105"',
            'createAndFund: "0x86f357d0"',
            'createWithAuthorization: "0x9b2065e0"',
            'approve: "0x095ea7b3"',
            'allowance: "0xdd62ed3e"',
            'predictWallet: "0x240fa116"',
            'revokePolicy: "0x9eba3667"',
            'configurePolicy: "0x27d3543c"',
            "starts a fresh policy-period spend counter",
            "exact two-wallet sandboxed-regression quorum",
            "OBSOLETE_DETERMINISTIC_VERIFIER",
            "manifest.contract_source_dirty !== false",
            "contract_source_revision",
            "contract_source_revision_kind",
            "ensureConnectedOwner",
            "eth_getBlockByNumber",
            "wallet_switchEthereumChain",
            "eth_signTypedData_v4",
            "clone_runtime_code_hash",
            "policy-bound wallet",
            "Smart-account activation requires one exact USDC approval",
            "Factory allowance was not fully consumed by activation",
        ],
    )
    if "This owner is a contract account. Use the manifest's approve" in bounded_javascript:
        fail("smart-account activation must use the reviewed allowance fallback instead of stopping")
    for forbidden in ["privateKey", "mnemonic", "seedInput", "wallet_import"]:
        if forbidden in bounded_javascript:
            fail(f"agent budget activation contains forbidden secret handling: {forbidden}")
    if bounded_deployment.get("schema") != "agent-bounties/bounded-agent-wallet-deployment-v1":
        fail("bounded-wallet deployment manifest has the wrong schema")
    if bounded_deployment.get("chain_id") != 8453 or bounded_deployment.get("network") != "base-mainnet":
        fail("bounded-wallet deployment manifest must target Base mainnet")
    if bounded_deployment.get("contract_source_dirty") is not False:
        fail("bounded-wallet deployment manifest must come from committed contract source")
    if bounded_deployment.get("contract_source_revision_kind") != "git-tree":
        fail("bounded-wallet deployment manifest must pin a content-addressed Git tree")
    if not re.fullmatch(r"[0-9a-f]{40}", bounded_deployment.get("contract_source_revision", "")):
        fail("bounded-wallet deployment manifest must pin a contract source revision")
    source_revision = bounded_deployment.get("contract_source_revision", "")
    if not re.fullmatch(r"[0-9a-f]{40}", source_revision):
        fail("bounded-wallet deployment manifest must pin a Git tree revision")
    contract_dir = repo_root / "contracts" / "base-escrow" / "src"
    source_files = {path.stem: path for path in contract_dir.glob("*.sol")}
    recorded_sources = bounded_deployment.get("contracts", {})
    missing_sources = set(recorded_sources) - set(source_files)
    if missing_sources:
        fail(f"bounded-wallet deployment source is missing: {sorted(missing_sources)}")
    for name, metadata in recorded_sources.items():
        path = source_files[name]
        observed_hash = f"0x{hashlib.sha256(path.read_bytes()).hexdigest()}"
        if metadata.get("source_sha256") != observed_hash:
            fail(f"bounded-wallet source hash drifted: {path.name}")
    pinned_values = {
        "sourceRevision": bounded_deployment["contract_source_revision"],
        "bountyFactory": bounded_deployment["canonical"]["bounty_factory"],
        "settlementToken": bounded_deployment["canonical"]["settlement_token"],
        "deterministicVerifier": bounded_deployment["canonical"]["deterministic_verifier"],
        "signedQuorumVerifierSetHash": bounded_deployment["canonical"]["signed_quorum_verifier_set_hash"],
        "deterministicDeployer": bounded_deployment["deterministic_deployer"]["address"],
        "deterministicDeployerHash": bounded_deployment["deterministic_deployer"]["runtime_code_hash"],
        "walletFactory": bounded_deployment["wallet_factory"]["address"],
        "implementation": bounded_deployment["wallet_factory"]["implementation"],
        "factoryRuntimeHash": bounded_deployment["wallet_factory"]["runtime_code_hash"],
        "implementationRuntimeHash": bounded_deployment["wallet_factory"]["implementation_runtime_code_hash"],
        "cloneRuntimeHash": bounded_deployment["wallet_factory"]["clone_runtime_code_hash"],
    }
    for name, value in pinned_values.items():
        if f'{name}: "{value.lower()}"' not in bounded_javascript:
            fail(f"agent budget activation does not pin manifest field: {name}")
    if standing_meta_deployment.get("schema") != "agent-bounties/standing-meta-v2-deployment-v1":
        fail("standing-meta-v2 deployment manifest has the wrong schema")
    if standing_meta_deployment.get("chain_id") != 8453 or standing_meta_deployment.get("network") != "base-mainnet":
        fail("standing-meta-v2 deployment manifest must target Base mainnet")
    if standing_meta_deployment.get("deployment", {}).get("receipt_status") != 1:
        fail("standing-meta-v2 deployment manifest requires a successful receipt")
    standing_components = standing_meta_deployment.get("components", {})
    if standing_components.get("verifier_module") != bounded_deployment["canonical"]["deterministic_verifier"]:
        fail("bounded wallet and standing-meta-v2 manifests disagree on the verifier")
    if standing_components.get("verifier_set_hash") != bounded_deployment["canonical"]["signed_quorum_verifier_set_hash"]:
        fail("bounded wallet and standing-meta-v2 manifests disagree on the signed quorum")
    if standing_components.get("verifier_wallets") != [
        "0xbe6292b9e465f549e2363b918d6dd9187038431e",
        "0xb7c2ce6430b66fb986e27b6140b29309550d487a",
    ]:
        fail("standing-meta-v2 deployment manifest has the wrong verifier wallets")
    if standing_components.get("verifier_runtime_code_hash") != (
        "0xe3b6e82880edee69b1f30560506ac80a46b4ebcc6c083cfa8207e3673eede26c"
    ):
        fail("standing-meta-v2 deployment manifest has the wrong verifier runtime hash")
    reserve = standing_meta_deployment.get("keeper_reserve", {})
    if reserve.get("functional_relay_receipt_status") != 1:
        fail("keeper reserve evidence requires a successful relay receipt")
    if reserve.get("confirmed_balance_wei", 0) < reserve.get("floor_wei", 1):
        fail("keeper reserve evidence is below its configured floor")
    require_phrases(
        "pages.yml bounded wallet",
        pages_workflow,
        [
            '"deployments/bounded-agent-wallet-base-mainnet.json"',
            "cp deployments/bounded-agent-wallet-base-mainnet.json site/bounded-agent-wallet-base-mainnet.json",
        ],
    )
    discovery_endpoints = discovery.get("endpoints", {})
    if discovery_endpoints.get("agent_wallet_readiness") != "https://api.bountyboard.global/v1/base/agent-wallet/readiness":
        fail("static discovery has the wrong agent wallet readiness endpoint")
    if discovery_endpoints.get("autonomous_standing_meta_v2_child_preparation") != (
        "https://api.bountyboard.global/v1/base/autonomous-bounties/standing-meta-v2-child-preparation"
    ):
        fail("static discovery has the wrong standing-meta-v2 preparation endpoint")
    if "prepare_agent_to_earn" not in discovery.get("agent_tools", []):
        fail("static discovery must expose prepare_agent_to_earn")
    base_rail = next(
        (
            rail
            for rail in discovery.get("payment_rails", [])
            if rail.get("name") == "Base native USDC"
        ),
        {},
    )
    if "active on Base mainnet" not in base_rail.get("status", ""):
        fail("static discovery manifest does not advertise active Base USDC")
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
    star_action = next((item for item in actions if item.get("kind") == "star_upvote_repo"), {})
    execution = star_action.get("execution", {})
    if (
        execution.get("requires_explicit_user_authorization") is not True
        or execution.get("star_command")
        != "gh api --method PUT /user/starred/NSPG13/agent-bounties"
        or "{issue_number}/reactions"
        not in execution.get("issue_reaction_command_template", "")
        or "Report success only after GitHub"
        not in execution.get("confirmation_boundary", "")
    ):
        fail("static discovery manifest lacks the authorized executable star/upvote plan")
    for marker in [
        "gh api --method PUT /user/starred/NSPG13/agent-bounties",
        "{issue_number}/reactions",
        "Never request broader GitHub access",
        "wallet_request",
        "next_request.body.wallet_signature",
    ]:
        if marker not in llms:
            fail(f"llms.txt missing executable post-value marker: {marker}")
    if len(discovery.get("distribution_feedback", {}).get("questions", [])) != 4:
        fail("static discovery manifest must ask the four distribution questions")

    print("site check ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
