#!/usr/bin/env python3
"""Idempotently create, fund, and reconcile four profitable routed-V3 parent bounties."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.request
from typing import Any, Mapping, Sequence

import durable_verifier_router_deploy as durable


ROOT = Path(__file__).resolve().parents[1]
CHAIN_ID = 8453
RPC_DEFAULT = "https://mainnet.base.org"
API_DEFAULT = "https://api.agentbounties.app"
DEPLOYMENT_PATH = ROOT / "deployments" / "durable-verifier-router-base-mainnet.json"
WALLET = durable.BOUNDED_WALLET
OWNER = durable.OWNER
KEEPER = durable.KEEPER
FACTORY = durable.CANONICAL_FACTORY
USDC = durable.NATIVE_USDC
TARGET = durable.PARENT_TARGET
TOTAL = durable.TOTAL_REPLACEMENT_FUNDING
UINT64_MAX = (1 << 64) - 1
ZERO_HASH = "0x" + "00" * 32
ISSUES = {
    333: {"lane": "CLI", "old": "0xfffecb0fcd36477c5f6ecec808f6f0cf53819562"},
    334: {"lane": "API", "old": "0xbe17ef2d154265ebe3142d7bda5e99610d571455"},
    335: {"lane": "MCP", "old": "0x43d42cb227d76588ab16693f14efd6cff851fa7a"},
    336: {"lane": "wallet UX", "old": "0xe8c1d3f046f3e4690bef59ba4abd5d02d2a6984b"},
}
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
UINT_RE = re.compile(r"^(?:0x[0-9a-fA-F]+|[0-9]+)")


class ActivationError(RuntimeError):
    pass


def redact_command(command: Sequence[str]) -> str:
    result: list[str] = []
    hide_next = False
    for item in command:
        if hide_next:
            result.append("***")
            hide_next = False
            continue
        result.append(item)
        if item in {"--private-key", "--rpc-url"}:
            hide_next = True
    return " ".join(result)


def run(command: Sequence[str], *, cwd: Path = ROOT, timeout: int = 300) -> str:
    completed = subprocess.run(
        list(command),
        cwd=cwd,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
    if completed.returncode != 0:
        raise ActivationError(
            f"command failed ({completed.returncode}): {redact_command(command)}\n{completed.stdout[-6000:]}"
        )
    return completed.stdout.strip()


def address(value: object, label: str) -> str:
    text = str(value).strip().lower()
    if not ADDRESS_RE.fullmatch(text):
        raise ActivationError(f"{label} is not an EVM address")
    return text


def bytes32(value: object, label: str) -> str:
    text = str(value).strip().lower()
    if not BYTES32_RE.fullmatch(text):
        raise ActivationError(f"{label} is not bytes32")
    return text


def parse_uint(value: object, label: str) -> int:
    match = UINT_RE.match(str(value).strip())
    if not match:
        raise ActivationError(f"{label} is not an unsigned integer")
    return int(match.group(0), 0)


def parse_bool(value: object, label: str) -> bool:
    text = str(value).strip().lower()
    if text in {"true", "1", "0x1", "0x01"}:
        return True
    if text in {"false", "0", "0x0", "0x00"}:
        return False
    raise ActivationError(f"{label} is not boolean")


def lines(value: str, expected: int, label: str) -> list[str]:
    result = [item.strip() for item in value.splitlines() if item.strip()]
    if len(result) != expected:
        raise ActivationError(f"{label} returned {len(result)} fields; expected {expected}")
    return result


def http_json(method: str, url: str, body: Mapping[str, object] | None = None) -> Any:
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={"content-type": "application/json", "user-agent": "agent-bounties-routed-v3-activation/1"},
    )
    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            raw = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise ActivationError(f"{method} {url} failed with HTTP {error.code}: {detail[:2000]}") from error
    try:
        return json.loads(raw)
    except json.JSONDecodeError as error:
        raise ActivationError(f"{method} {url} returned invalid JSON") from error


class Cast:
    def __init__(self, executable: str, rpc_url: str) -> None:
        self.executable = executable
        self.rpc_url = rpc_url

    def rpc(self, *args: str, timeout: int = 300) -> str:
        return run([self.executable, *args, "--rpc-url", self.rpc_url], timeout=timeout)

    def call(self, target: str, signature: str, *args: str) -> str:
        return self.rpc("call", target, signature, *args).strip()

    def code(self, target: str) -> str:
        return self.rpc("code", target).strip().lower()

    def chain_id(self) -> int:
        return parse_uint(self.rpc("chain-id"), "chain id")

    def send_data(self, target: str, data: str, private_key: str) -> dict[str, Any]:
        raw = self.rpc(
            "send", target, "--data", data, "--private-key", private_key, "--json", timeout=180
        )
        try:
            result = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ActivationError("cast send did not return JSON") from error
        tx_hash = result.get("transactionHash") or result.get("transaction_hash")
        if not isinstance(tx_hash, str) or not BYTES32_RE.fullmatch(tx_hash):
            raise ActivationError("cast send JSON is missing a transaction hash")
        receipt_raw = self.rpc("receipt", tx_hash, "--json", timeout=180)
        try:
            receipt = json.loads(receipt_raw)
        except json.JSONDecodeError as error:
            raise ActivationError("cast receipt did not return JSON") from error
        if receipt.get("status") not in {"0x1", "0x01", 1}:
            raise ActivationError(f"transaction reverted: {tx_hash}")
        result["receipt"] = receipt
        return result


def load_deployment() -> dict[str, Any]:
    if not DEPLOYMENT_PATH.exists():
        raise ActivationError("durable verifier-router deployment manifest is missing")
    try:
        deployment = json.loads(DEPLOYMENT_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ActivationError(f"invalid durable verifier-router deployment manifest: {error}") from error
    if deployment.get("schema") != "agent-bounties/durable-verifier-router-deployment-v1":
        raise ActivationError("durable verifier-router deployment schema mismatch")
    if deployment.get("network") != "base-mainnet" or deployment.get("chain_id") != CHAIN_ID:
        raise ActivationError("durable verifier-router deployment network mismatch")
    router = deployment.get("router")
    adapter = deployment.get("adapter")
    if not isinstance(router, dict) or not isinstance(adapter, dict):
        raise ActivationError("durable verifier-router deployment lacks router or adapter evidence")
    deployment["router_address"] = address(router.get("address"), "router address")
    deployment["router_runtime_code_hash"] = bytes32(router.get("runtime_code_hash"), "router code hash")
    deployment["policy_hash"] = bytes32(deployment.get("policy_hash"), "routed policy hash")
    deployment["adapter_address"] = address(adapter.get("address"), "adapter address")
    deployment["adapter_runtime_code_hash"] = bytes32(
        adapter.get("runtime_code_hash"), "adapter code hash"
    )
    return deployment


def policy_state(cast: Cast, deployment: Mapping[str, Any]) -> dict[str, Any]:
    if cast.chain_id() != CHAIN_ID:
        raise ActivationError("routed replacement activation is pinned to Base mainnet")
    router = str(deployment["router_address"])
    adapter = str(deployment["adapter_address"])
    policy_hash = str(deployment["policy_hash"])
    for label, target in {"router": router, "adapter": adapter, "wallet": WALLET}.items():
        if cast.code(target) in {"0x", "0x0"}:
            raise ActivationError(f"{label} runtime code is missing")
    if not parse_bool(cast.call(router, "isPolicyActive(bytes32)(bool)", policy_hash), "routed policy active"):
        raise ActivationError("routed policy is not active")

    record = lines(
        cast.call(
            router,
            "policies(bytes32)(address,bytes32,uint64,uint64,uint64,bool)",
            policy_hash,
        ),
        6,
        "router policy record",
    )
    if address(record[0], "router policy adapter") != adapter:
        raise ActivationError("router policy adapter mismatch")
    if bytes32(record[1], "router policy code hash") != deployment["adapter_runtime_code_hash"]:
        raise ActivationError("router policy runtime code hash mismatch")
    if parse_uint(record[4], "router activated at") == 0 or parse_bool(record[5], "router vetoed"):
        raise ActivationError("router policy is not safely active")

    policy = lines(
        cast.call(
            WALLET,
            "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32)",
        ),
        13,
        "bounded wallet policy",
    )
    now = parse_uint(cast.rpc("block", "latest", "--field", "timestamp"), "block timestamp")
    state = {
        "owner": address(cast.call(WALLET, "owner()(address)"), "wallet owner"),
        "delegate": address(policy[0], "policy delegate"),
        "valid_after": parse_uint(policy[1], "valid after"),
        "valid_until": parse_uint(policy[2], "valid until"),
        "period_seconds": parse_uint(policy[3], "period seconds"),
        "max_per_action": parse_uint(policy[4], "max per action"),
        "max_per_period": parse_uint(policy[5], "max per period"),
        "max_lifetime_spend": parse_uint(policy[6], "max lifetime spend"),
        "max_bounty_target": parse_uint(policy[7], "max bounty target"),
        "allowed_actions": parse_uint(policy[8], "allowed actions"),
        "allowed_verification_modes": parse_uint(policy[9], "allowed modes"),
        "deterministic_verifier": address(policy[10], "deterministic verifier"),
        "signed_quorum": bytes32(policy[11], "signed quorum hash"),
        "ai_quorum": bytes32(policy[12], "AI quorum hash"),
        "policy_hash": bytes32(cast.call(WALLET, "policyHash()(bytes32)"), "wallet policy hash"),
        "policy_version": parse_uint(cast.call(WALLET, "policyVersion()(uint64)"), "policy version"),
        "period_spent": parse_uint(cast.call(WALLET, "periodSpent()(uint256)"), "period spent"),
        "lifetime_spent": parse_uint(cast.call(WALLET, "lifetimeSpent()(uint256)"), "lifetime spent"),
        "wallet_balance": parse_uint(cast.call(USDC, "balanceOf(address)(uint256)", WALLET), "wallet balance"),
        "now": now,
    }
    expected = {
        "owner": OWNER,
        "delegate": KEEPER,
        "period_seconds": 86_400,
        "max_per_action": 5_000_000,
        "max_per_period": 10_000_000,
        "max_lifetime_spend": 89_000_000,
        "max_bounty_target": 5_000_000,
        "allowed_actions": 15,
        "allowed_verification_modes": 1,
        "deterministic_verifier": router,
        "signed_quorum": ZERO_HASH,
        "ai_quorum": ZERO_HASH,
        "valid_until": UINT64_MAX,
    }
    for key, wanted in expected.items():
        if state[key] != wanted:
            raise ActivationError(f"durable wallet policy {key} mismatch: expected {wanted}, got {state[key]}")
    if not state["valid_after"] <= now <= state["valid_until"]:
        raise ActivationError("durable wallet policy is not active")
    current_bucket = now // state["period_seconds"]
    observed_bucket = parse_uint(cast.call(WALLET, "periodBucket()(uint256)"), "period bucket")
    period_spent = state["period_spent"] if observed_bucket == current_bucket else 0
    if period_spent + TOTAL > state["max_per_period"]:
        raise ActivationError("remaining current-period budget is below 8.04 USDC")
    if state["wallet_balance"] < TOTAL:
        raise ActivationError("bounded wallet balance is below 8.04 USDC")
    if state["lifetime_spent"] + TOTAL > state["max_lifetime_spend"]:
        raise ActivationError("bounded wallet remaining lifetime budget is below 8.04 USDC")
    state["effective_period_spent"] = period_spent
    return state


def planner_manifest(path: Path, router: str) -> Path:
    source = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"
    manifest = json.loads(source.read_text(encoding="utf-8"))
    manifest["canonical"]["deterministic_verifier"] = router
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def event_kinds(api: str, bounty_id: str) -> set[str]:
    events = http_json(
        "GET",
        f"{api.rstrip('/')}/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id={bounty_id}",
    )
    if not isinstance(events, list):
        raise ActivationError("events endpoint returned a non-list")
    return {str(item.get("kind")) for item in events if isinstance(item, dict)}


def reconcile(api: str, contract: str, bounty_id: str, timeout_seconds: int = 180) -> dict[str, Any]:
    required = {"canonical_bounty_created", "funding_added", "bounty_became_claimable"}
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        kinds = event_kinds(api, bounty_id)
        if required.issubset(kinds):
            feed = http_json(
                "GET",
                f"{api.rstrip('/')}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false",
            )
            if not isinstance(feed, list):
                raise ActivationError("feed endpoint returned a non-list")
            item = next(
                (
                    candidate
                    for candidate in feed
                    if isinstance(candidate, dict)
                    and str(candidate.get("bounty_contract", "")).lower() == contract.lower()
                ),
                None,
            )
            if (
                item
                and item.get("status") == "claimable"
                and item.get("terms_valid") is True
                and item.get("verification_ready") is True
            ):
                return {"event_kinds": sorted(kinds), "feed_item": item}
        time.sleep(3)
    raise ActivationError(f"canonical activation did not reconcile for {contract}")


def issue_body(issue: int, lane: str, old: str, result: Mapping[str, object], deployment: Mapping[str, Any]) -> str:
    contract = str(result["contract"])
    transaction_hash = str(result["transaction_hash"])
    transaction_line = (
        f"- Creation and funding transaction: https://basescan.org/tx/{transaction_hash}"
        if BYTES32_RE.fullmatch(transaction_hash)
        else "- Creation and funding transaction: previously confirmed canonical creation"
    )
    return f"""## Goal

Create and fully fund a concrete **1 USDC {lane} child bounty** that a different registered participant completes and receives canonical settlement for. Completing the parent pays **2 USDC**, producing **1 USDC gross profit** when the child is self-funded at exactly 1 USDC.

## Live payment evidence

**Funded and claimable on Base mainnet.**

- Routed V3 parent: `{contract}`
{transaction_line}
- Confirmed funding: **2.01 / 2.01 USDC**
- Parent solver reward: **2.00 USDC**
- Automated verifier reward: **0.01 USDC**
- Refundable claim bond: **0.01 USDC**
- Required child target: **1.00 USDC minimum**
- Guaranteed parent gross margin: **1.00 USDC minimum**
- Stable verifier router: `{deployment['router_address']}`
- Immutable routed policy hash: `{deployment['policy_hash']}`
- Routed implementation: `{deployment['adapter_address']}`
- Status: `claimable`

The previous unprofitable V2 contract `{old}` is retired from earning discovery and preserved as immutable history.

## Earn this bounty

This is a coordination bounty, not a direct code-fix bounty.

1. Use a Base wallet with at least **0.01 USDC** for the refundable parent claim bond. Eligible gas is sponsored.
2. Find a different intended child solver. Both participants register before the parent claim with `/agent-bounty register 0xYourBaseWallet`.
3. Before claiming, prepare and publish exact parent-bound child terms for a concrete **{lane}** coding task using the committed `sandboxed_regression_v1` threshold-two verifier quorum.
4. Create and fully fund that child with a total target of **exactly 1.00 USDC** to retain the full 1.00 USDC gross parent margin.
5. Wait for confirmed child creation, funding, claimability, and a strictly later Base timestamp than terms publication and both registrations.
6. Claim this parent. The different child participant completes the child and receives canonical `BountySettled` payment.
7. Submit `abi.encode(address childBounty)` to this parent with the requested discovery feedback. The routed verifier releases the 2.00 USDC parent reward only when every immutable condition passes.

A claim comment, signature request, transaction hash, or accepted submission is not payment. Only canonical `BountySettled` proves earnings.

**Post your own bounty:** https://agentbounties.app/objective.html#post

<!-- agent-bounties-github-metadata-v1 -->
## Automation metadata

### Goal
Create and fully fund a concrete 1 USDC {lane} child bounty that a different registered participant completes and receives canonical settlement for, leaving at least 1 USDC gross parent profit.

### Acceptance criteria
Satisfy every immutable routed standing-meta-v3 condition: pre-claim terms and registrations, a 1 USDC minimum parent-bound child, at least 1 USDC retained parent margin, the committed regression quorum, an independent child participant, canonical child settlement, and exact parent proof.

### Template
small-code-change

### Suggested amount
2 USDC

### Funding mode
AutonomousV1BaseUsdc

### Privacy
Public
"""


def activate(args: argparse.Namespace) -> dict[str, Any]:
    private_key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
    if not private_key:
        raise ActivationError("BASE_KEEPER_PRIVATE_KEY is required")
    cast = Cast(args.cast, args.rpc_url)
    keeper = address(run([args.cast, "wallet", "address", "--private-key", private_key]), "keeper")
    if keeper != KEEPER:
        raise ActivationError(f"keeper key resolves to {keeper}, expected {KEEPER}")
    deployment = load_deployment()
    before = policy_state(cast, deployment)
    manifest = planner_manifest(args.output_dir / "bounded-wallet-routed-v3-manifest.json", deployment["router_address"])
    documents = durable.materialize_terms(ROOT, deployment["router_address"])
    results: list[dict[str, Any]] = []

    for issue, config in ISSUES.items():
        document = documents[issue]
        published = http_json(
            "POST",
            f"{args.api.rstrip('/')}/v1/base/autonomous-bounties/terms",
            {"creator_wallet": WALLET, "document": document},
        )
        if not isinstance(published, dict):
            raise ActivationError(f"terms publication for issue #{issue} returned a non-object")
        if bytes32(published.get("policy_hash"), f"issue #{issue} policy hash") != deployment["policy_hash"]:
            raise ActivationError(f"issue #{issue} terms do not bind the deployed routed policy")
        create = durable.create_payload(document, published)
        plan = http_json(
            "POST",
            f"{args.api.rstrip('/')}/v1/base/autonomous-bounties/creation-plan",
            {"network": "base-mainnet", "create": create},
        )
        if not isinstance(plan, dict):
            raise ActivationError(f"creation plan for issue #{issue} returned a non-object")
        plan_path = args.output_dir / f"routed-v3-{issue}-creation-plan.json"
        plan_path.write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        action_path = args.output_dir / f"routed-v3-{issue}-bounded-action.json"
        run(
            [
                args.python,
                "scripts/plan_bounded_agent_action.py",
                "create",
                "--wallet",
                WALLET,
                "--creation-plan",
                str(plan_path),
                "--manifest",
                str(manifest),
                "--rpc-url",
                args.rpc_url,
                "--expect-owner",
                OWNER,
                "--expect-delegate",
                KEEPER,
                "--output",
                str(action_path),
            ]
        )
        action = json.loads(action_path.read_text(encoding="utf-8"))
        direct = action.get("direct_transaction")
        if not isinstance(direct, dict):
            raise ActivationError(f"issue #{issue} action plan lacks a direct transaction")
        predicted = address(plan.get("predicted_bounty_contract"), f"issue #{issue} predicted bounty")
        bounty_id = bytes32(plan.get("bounty_id"), f"issue #{issue} bounty id")
        canonical = parse_bool(
            cast.call(FACTORY, "isCanonicalBounty(address)(bool)", predicted),
            f"issue #{issue} canonical state",
        )
        if canonical:
            tx_hash = "already-canonical"
        else:
            sent = cast.send_data(
                address(direct.get("to"), "direct transaction target"),
                str(direct.get("data")),
                private_key,
            )
            tx_hash = str(sent.get("transactionHash") or sent.get("transaction_hash"))
        reconciled = reconcile(args.api, predicted, bounty_id)
        result = {
            "issue": issue,
            "lane": config["lane"],
            "old_contract": config["old"],
            "contract": predicted,
            "bounty_id": bounty_id,
            "transaction_hash": tx_hash,
            "terms_hash": bytes32(published.get("terms_hash"), f"issue #{issue} terms hash"),
            "policy_hash": deployment["policy_hash"],
            "reconciliation": reconciled,
        }
        (args.output_dir / f"routed-v3-{issue}-issue.md").write_text(
            issue_body(issue, str(config["lane"]), str(config["old"]), result, deployment),
            encoding="utf-8",
        )
        results.append(result)

    after = policy_state(cast, deployment)
    newly_created = sum(1 for item in results if item["transaction_hash"] != "already-canonical")
    expected_spend = newly_created * TARGET
    if before["lifetime_spent"] + expected_spend != after["lifetime_spent"]:
        raise ActivationError("wallet lifetime spend did not increase by the exact newly-created amount")
    if before["wallet_balance"] - expected_spend != after["wallet_balance"]:
        raise ActivationError("wallet balance did not decrease by the exact newly-created amount")
    return {
        "schema": "agent-bounties/routed-v3-replacement-activation-v1",
        "network": "base-mainnet",
        "wallet": WALLET,
        "router": deployment["router_address"],
        "policy_hash": deployment["policy_hash"],
        "adapter": deployment["adapter_address"],
        "wallet_balance_before": before["wallet_balance"],
        "wallet_balance_after": after["wallet_balance"],
        "lifetime_spent_before": before["lifetime_spent"],
        "lifetime_spent_after": after["lifetime_spent"],
        "new_spend": expected_spend,
        "results": results,
        "evidence_boundary": (
            "Confirmed canonical creation, FundingAdded, BountyBecameClaimable, valid terms, and verification readiness "
            "prove the routed replacement parents are live. Only BountySettled proves future solver payment."
        ),
    }


def markdown(report: Mapping[str, object]) -> str:
    results = report["results"]
    assert isinstance(results, list)
    lines = [
        "## Four profitable routed-V3 replacements activated",
        "",
        f"- Stable verifier router: `{report['router']}`",
        f"- Immutable routed policy: `{report['policy_hash']}`",
        f"- Routed implementation: `{report['adapter']}`",
        f"- Bounded wallet before: **{int(report['wallet_balance_before']) / 1_000_000:.6f} USDC**",
        f"- Bounded wallet after: **{int(report['wallet_balance_after']) / 1_000_000:.6f} USDC**",
        f"- New spend in this run: **{int(report['new_spend']) / 1_000_000:.6f} USDC**",
        "",
    ]
    for item in results:
        assert isinstance(item, dict)
        lines.append(
            f"- #{item['issue']} `{item['contract']}` — 2.00 USDC solver / 0.01 verifier / 0.01 refundable bond"
        )
    lines.extend(
        [
            "",
            "Each contract reconciled canonical creation, funding, claimability, valid terms, and verification readiness.",
            "Only a future canonical BountySettled event proves solver payment.",
        ]
    )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", RPC_DEFAULT))
    parser.add_argument("--api", default=os.environ.get("AGENT_BOUNTIES_API_URL", API_DEFAULT))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--python", default=os.environ.get("PYTHON_BIN", "python"))
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target" / "routed-v3-activation")
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "routed-v3-activation.json")
    parser.add_argument("--markdown-output", type=Path, default=ROOT / "target" / "routed-v3-activation.md")
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = activate(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.markdown_output.write_text(markdown(report), encoding="utf-8")
    print(
        json.dumps(
            {
                "activated": [item["issue"] for item in report["results"]],
                "wallet_balance_after": report["wallet_balance_after"],
                "output": str(args.output),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
