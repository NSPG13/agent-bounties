#!/usr/bin/env python3
"""Validate the single-product ChatGPT app artifact and hosted-execution boundary."""

from __future__ import annotations

import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CANONICAL_SCHEMA = (
    "https://developers.openai.com/plugins/schemas/"
    "chatgpt-app-submission.v1.json"
)
FULL_TOOLS = {
    "get_bounty_feed",
    "render_bounty_feed",
    "prepare_moonpay_onramp",
    "prepare_bounty_post",
    "prepare_bounty_action",
    "get_bounty_action_status",
    "compile_objective_with_cloud_agent",
    "list_bounty_comments",
    "add_bounty_comment",
    "create_share_bundle",
}
DIRECT_EXECUTION_TOOLS = {
    "fund_bounty_with_x402",
    "agent_native_claim",
    "prepare_autonomous_bounty_submission",
    "plan_autonomous_module_settlement",
    "plan_autonomous_attestation_settlement",
}
EXPECTED_ANNOTATIONS = {
    "get_bounty_feed": (True, False, False),
    "render_bounty_feed": (True, False, False),
    "prepare_moonpay_onramp": (True, False, False),
    "prepare_bounty_post": (False, True, True),
    "prepare_bounty_action": (False, False, False),
    "get_bounty_action_status": (False, False, False),
    "compile_objective_with_cloud_agent": (False, True, False),
    "list_bounty_comments": (True, False, False),
    "add_bounty_comment": (False, True, True),
    "create_share_bundle": (True, False, False),
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"chatgpt_submission_check=failed reason={message}")


def main() -> None:
    artifact = json.loads(
        (ROOT / "chatgpt-app-submission.json").read_text(encoding="utf-8")
    )
    tools = artifact.get("tools", {})
    server = (ROOT / "crates" / "mcp-server" / "src" / "chatgpt_app.rs").read_text(
        encoding="utf-8"
    )
    dossier = (ROOT / "docs" / "chatgpt-app-submission.md").read_text(
        encoding="utf-8"
    )
    advertised_match = re.search(
        r"const CHATGPT_ADVERTISED_TOOL_NAMES:.*?=\s*&\[(.*?)\];",
        server,
        re.DOTALL,
    )
    require(advertised_match is not None, "code ChatGPT allowlist is missing")
    code_tools = set(re.findall(r'"([a-z][a-z0-9_]+)"', advertised_match.group(1)))
    dossier_table = dossier.split("## Full tool surface", 1)[1].split(
        "Every tool declares", 1
    )[0]
    dossier_tools = set(re.findall(r"\| `([a-z][a-z0-9_]+)` \|", dossier_table))
    require(artifact.get("$schema") == CANONICAL_SCHEMA, "official schema URL drifted")
    require(artifact.get("schema_version") == 1, "schema_version must equal 1")
    require(set(tools) == FULL_TOOLS, "artifact must equal the ten-tool full product")
    require(code_tools == FULL_TOOLS, "code allowlist must equal the ten-tool artifact")
    require(dossier_tools == FULL_TOOLS, "release dossier must equal the ten-tool artifact")
    require(
        "list_autonomous_bounties" not in code_tools,
        "compatibility alias leaked into ChatGPT discovery",
    )
    compatibility_match = re.search(
        r"const CHATGPT_COMPATIBILITY_TOOL_NAMES:.*?=\s*&\[(.*?)\];",
        server,
        re.DOTALL,
    )
    require(compatibility_match is not None, "compatibility allowlist is missing")
    compatibility_tools = set(
        re.findall(r'"([a-z][a-z0-9_]+)"', compatibility_match.group(1))
    )
    require(
        compatibility_tools == {"list_autonomous_bounties"}
        and '"list_autonomous_bounties" =>' in server,
        "cached ChatGPT compatibility dispatch drifted",
    )
    require(
        "`list_autonomous_bounties` remains callable" in dossier
        and "list_autonomous_bounties" in artifact["app_info"]["description"],
        "compatibility boundary is missing from the dossier or artifact",
    )
    require(
        not DIRECT_EXECUTION_TOOLS.intersection(tools),
        "lower-level wallet or settlement execution tool leaked into ChatGPT",
    )
    require(len(artifact.get("test_cases", [])) == 5, "exactly five positive tests are required")
    require(
        len(artifact.get("negative_test_cases", [])) == 3,
        "exactly three negative tests are required",
    )
    require(
        artifact["app_info"]["display_name"] == "Agent Bounties",
        "single-product listing name drifted",
    )
    require(
        artifact["release_status"]["product_profile"] == "full_hosted_execution"
        and artifact["release_status"]["public_and_developer_parity"] is True,
        "artifact must declare one full public/developer product profile",
    )
    require(
        artifact["release_status"]["directory_submission"]
        == "blocked_pending_written_openai_approval_or_policy_change",
        "current Plugin Directory policy blocker must remain explicit",
    )

    for name, expected in EXPECTED_ANNOTATIONS.items():
        annotations = tools[name]["annotations"]
        actual = (
            annotations["readOnlyHint"],
            annotations["openWorldHint"],
            annotations["destructiveHint"],
        )
        require(actual == expected, f"{name} annotations drifted: {actual} != {expected}")

    widget = (ROOT / "site" / "chatgpt-bounty-feed-widget.html").read_text(
        encoding="utf-8"
    )
    require(
        'bridgeNotify("ui/message", message)' in widget
        and "openai()?.sendFollowUpMessage" in widget,
        "widget actions must continue in ChatGPT through the standard bridge and "
        "the documented ChatGPT compatibility helper",
    )
    widget_lower = widget.lower()
    require(
        all(
            element not in widget_lower
            for element in ("<input", "<textarea", "<select", "<form")
        ),
        "conversation-first widget must not expose fields or forms",
    )
    button_actions = set(re.findall(r'data-action="([^"]+)"', widget))
    require(
        button_actions == {"post-bounty", "comment", "share", "solve"}
        and widget.count("<button") == 4,
        f"widget buttons drifted from the approved four actions: {button_actions}",
    )
    for visible_label in (">Post bounty<", ">Comment<", ">Share<", ">Solve<"):
        require(visible_label in widget, f"widget lost {visible_label}")
    for forbidden_label in (
        ">Compete<",
        ">Fund<",
        ">Complete<",
        ">Verify<",
        ">Refresh<",
        ">Break down<",
    ):
        require(
            forbidden_label not in widget,
            f"widget exposed an unapproved button: {forbidden_label}",
        )
    for outdated_term in (
        "live quest feed",
        "guild companion",
        "share this quest step",
        "explore the quest",
        "open for competition",
        "ready to compete",
    ):
        require(
            outdated_term not in widget_lower,
            f"minimal widget must not contain outdated visual copy: {outdated_term}",
        )
    require(
        'class="project-thumb"' in widget
        and "Live bounties" in widget
        and "ChatGPT gathers details conversationally" in widget,
        "widget must preserve the branded conversation-first live-feed presentation",
    )
    require(
        'callTool("get_bounty_feed"' in widget,
        "read-only widget must load the live projection through the host bridge",
    )
    require(
        "generate one unique bounty image using my ChatGPT account" in widget
        and "prepare_bounty_post" in widget
        and "must not generate a replacement" in widget,
        "Post bounty conversation must preserve the user-owned ChatGPT image flow",
    )
    composer = (ROOT / "site" / "bounty-composer-v2.js").read_text(
        encoding="utf-8"
    )
    chat_css = (ROOT / "site" / "bounty-chat.css").read_text(encoding="utf-8")
    require(
        "enableChatgptHandoffReview" in composer
        and 'params.get("from") === "chatgpt-app"' in composer
        and 'inputWrap.hidden = true' in composer
        and 'ui.revise.hidden = true' in composer
        and "chatgpt-handoff-review" in chat_css,
        "ChatGPT post handoff must show a read-only review card without a second composer",
    )
    for mutating_tool in FULL_TOOLS - {"get_bounty_feed", "render_bounty_feed"}:
        require(
            f'callTool("{mutating_tool}"' not in widget,
            f"widget must leave {mutating_tool} to the confirmed conversation flow",
        )

    preview = (ROOT / "site" / "chatgpt-bounty-card-preview.html").read_text(
        encoding="utf-8"
    )
    for brand_color in (
        "#020b08",
        "#07140f",
        "#091710",
        "#b9ef37",
        "#18d9ac",
        "#e8bd26",
        "#f6f7ef",
        "#b9c0b8",
    ):
        require(
            brand_color in widget and brand_color in preview,
            f"widget and share card must use website brand color {brand_color}",
        )
    for blue in (
        "#2563eb",
        "#1d4ed8",
        "#60a5fa",
        "#eff6ff",
        "#bfdbfe",
        "#93c5fd",
        "#172554",
        "#1e40af",
    ):
        require(
            blue not in widget_lower and blue not in preview.lower(),
            f"unauthorized blue remains in the ChatGPT UI: {blue}",
        )

    require(
        "CHATGPT_APP_PUBLIC_REVIEW_MODE" not in server,
        "reduced public-review environment switch must be removed",
    )
    require(
        '"prepare_moonpay_onramp",' in server
        and "build_moonpay_onramp_handoff" in server
        and "moonpay_onramp_output_schema" in server,
        "bounded MoonPay MCP handoff contract is incomplete",
    )
    require(
        '"prepare_bounty_post",' in server
        and '"openai/fileParams"' in server
        and '"bounty_image"' in server
        and "chatgpt_user_generated" in server
        and "put_bounty_image_asset" in server,
        "ChatGPT-account bounty image handoff contract is incomplete",
    )
    require(
        '"checkout_created": false' in server
        and '"purchase_completed": false' in server
        and '"bounty_funded": false' in server
        and "FundingAdded" in server,
        "MoonPay handoff must fail closed across purchase and funding evidence",
    )
    require(
        '"prepare_bounty_action" =>' in server and "without_action_details" in server,
        "hosted bounty lifecycle must minimize MCP responses",
    )
    require(
        '"app_mode": {"type": "string", "enum": ["full", "sandbox"]}' in server,
        "runtime output schema must expose only full and fixture-only profiles",
    )
    require(
        "PokÃ©mon-card-style" not in server
        and "pokemon-card-style" not in server.lower(),
        "third-party card-style branding leaked into model-readable metadata",
    )

    main_server = (ROOT / "crates" / "mcp-server" / "src" / "main.rs").read_text(
        encoding="utf-8"
    )
    require(
        '"/.well-known/openai-apps-challenge"' in main_server,
        "OpenAI domain challenge route is missing",
    )
    require(
        '"/chatgpt/bounty-card-preview"' in main_server,
        "first-party bounty-card preview route is missing",
    )
    require(
        '"/public/bounty-images/:sha256"' in main_server,
        "content-addressed bounty image route is missing",
    )
    require(
        '"/v1/onramps/moonpay/checkout"' in main_server,
        "hosted MoonPay checkout-preparation route is missing",
    )

    preview = (ROOT / "site" / "chatgpt-bounty-card-preview.html").read_text(
        encoding="utf-8"
    )
    require("Download PNG" in preview, "card preview must require an explicit download click")
    require(
        "No wallet, signature, social post, purchase, or payment authorization occurs"
        in preview,
        "card preview must disclose its non-transactional boundary",
    )

    privacy = (ROOT / "site" / "privacy.html").read_text(encoding="utf-8")
    require("ChatGPT hosted action intents" in privacy, "intent privacy disclosure is missing")
    require(
        "deleted within 24 hours after expiry" in privacy,
        "intent retention disclosure is missing",
    )
    require(
        "public and developer-installed experiences use the same hosted-action flow"
        in privacy,
        "privacy policy must disclose public/developer parity",
    )
    require("MoonPay handles the purchase" in privacy, "MoonPay provider boundary is missing")
    require(
        "Agent Bounties does not use its own OpenAI API key" in privacy
        and "ChatGPT-generated bounty images" in privacy,
        "ChatGPT-account image privacy disclosure is missing",
    )

    submission_doc = (ROOT / "docs" / "chatgpt-app-submission.md").read_text(
        encoding="utf-8"
    )
    require(
        "Directory policy status: blocked" in submission_doc,
        "release documentation must not misrepresent directory eligibility",
    )

    print(
        "chatgpt_submission_check=ok "
        f"tools={len(tools)} positive_tests={len(artifact['test_cases'])} "
        f"negative_tests={len(artifact['negative_test_cases'])} "
        "profile=full_hosted_execution directory_policy=blocked"
    )


if __name__ == "__main__":
    main()
