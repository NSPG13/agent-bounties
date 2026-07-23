#!/usr/bin/env python3
"""Create and reconcile the four profitable standing-meta-v3 replacement parents."""

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


ROOT = Path(__file__).resolve().parents[1]
CHAIN_ID = 8453
RPC_DEFAULT = "https://mainnet.base.org"
API_DEFAULT = "https://api.agentbounties.app"
WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
KEEPER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
USDC = "0x833589fcd6edb6e08f4c7c32D4f71b54bdA02913".lower()
V3 = "0x8e3d799d3d2cf52112e5be4ce48f105379462077"
ZERO_HASH = "0x" + "00" * 32
TARGET = 2_010_000
TOTAL = 8_040_000
POLICY_SECONDS = 7_200
ISSUES = {
    333: {
        "lane": "cli",
        "terms": "bounties/autonomous-v1/v3-333.json",
        "old": "0xfffecb0fcd36477c5f6ecec808f6f0cf53819562",
    },
    334: {
        "lane": "api",
        "terms": "bounties/autonomous-v1/v3-334.json",
        "old": "0xbe17ef2d154265ebe3142d7bda5e99610d571455",
    },
    335: {
        "lane": "mcp",
        "terms": "bounties/autonomous-v1/v3-335.json",
        "old": "0x43d42cb227d76588ab16693f14efd6cff851fa7a",
    },
    336: {
        "lane": "wallet UX",
        "terms": "bounties/autonomous-v1/v3-336.json",
        "old": "0xe8c1d3f046f3e4690bef59ba4abd5d02d2a6984b",
    },
}
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
UINT_RE = re.compile(r"^(?:0x[0-9a-fA-F]+|[0-9]+)")


class ActivationError(RuntimeError):
    pass


def run(command: Sequence[str], *, cwd: Path = ROOT, timeout: int = 300) -> str:
    completed = subprocess.run(
        list(command), cwd=cwd, text=True, encoding="utf-8", errors="replace",
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, check=False,
    )
    if completed.returncode != 0:
        raise ActivationError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n{completed.stdout[-6000:]}"
        )
    return completed.stdout.strip()


def parse_uint(value: object, label: str) -> int:
    match = UINT_RE.match(str(value).strip())
    if not match:
        raise ActivationError(f"{label} is not an unsigned integer")
    return int(match.group(0), 0)


def address(value: object, label: str) -> str:
    text = str(value).strip().lower()
    if not ADDRESS_RE.fullmatch(text):
        raise ActivationError(f"{label} is not an EVM address")
    return text


def lines(value: str, expected: int, label: str) -> list[str]:
    result = [item.strip() for item in value.splitlines() if item.strip()]
    if len(result) != expected:
        raise ActivationError(f"{label} returned {len(result)} fields; expected {expected}")
    return result


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
        if not isinstance(tx_hash, str):
            raise ActivationError("cast send JSON is missing transaction hash")
        receipt_raw = self.rpc("receipt", tx_hash, "--json", timeout=180)
        receipt = json.loads(receipt_raw)
        if receipt.get("status") not in {"0x1", "0x01", 1}:
            raise ActivationError(f"transaction reverted: {tx_hash}")
        result["receipt"] = receipt
        return result


def http_json(method: str, url: str, body: Mapping[str, object] | None = None) -> Any:
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={"content-type": "application/json", "user-agent": "agent-bounties-v3-activation/1"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise ActivationError(f"{method} {url} failed with HTTP {error.code}: {detail[:2000]}") from error
    try:
        return json.loads(raw)
    except json.JSONDecodeError as error:
        raise ActivationError(f"{method} {url} returned invalid JSON") from error


def policy_state(cast: Cast) -> dict[str, Any]:
    if cast.chain_id() != CHAIN_ID:
        raise ActivationError("replacement activation is pinned to Base mainnet")
    if cast.code(V3) in {"0x", "0x0"}:
        raise ActivationError("V3 verifier is not deployed")
    policy = lines(
        cast.call(
            WALLET,
            "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32)",
        ),
        13,
        "wallet policy",
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
        "signed_quorum": policy[11].strip().lower(),
        "ai_quorum": policy[12].strip().lower(),
        "policy_hash": cast.call(WALLET, "policyHash()(bytes32)").strip().lower(),
        "policy_version": parse_uint(cast.call(WALLET, "policyVersion()(uint64)"), "policy version"),
        "period_spent": parse_uint(cast.call(WALLET, "periodSpent()(uint256)"), "period spent"),
        "lifetime_spent": parse_uint(cast.call(WALLET, "lifetimeSpent()(uint256)"), "lifetime spent"),
        "wallet_balance": parse_uint(cast.call(USDC, "balanceOf(address)(uint256)", WALLET), "wallet balance"),
        "now": now,
    }
    expected = {
        "owner": OWNER,
        "delegate": KEEPER,
        "period_seconds": POLICY_SECONDS,
        "max_per_action": TARGET,
        "max_per_period": TOTAL,
        "max_bounty_target": TARGET,
        "allowed_actions": 1,
        "allowed_verification_modes": 1,
        "deterministic_verifier": V3,
        "signed_quorum": ZERO_HASH,
        "ai_quorum": ZERO_HASH,
    }
    for key, wanted in expected.items():
        if state[key] != wanted:
            raise ActivationError(f"temporary policy {key} mismatch: expected {wanted}, got {state[key]}")
    if not state["valid_after"] <= now <= state["valid_until"]:
        raise ActivationError("temporary migration policy is not active")
    if state["wallet_balance"] < TOTAL:
        raise ActivationError("bounded wallet balance is below 8.04 USDC")
    if state["lifetime_spent"] + TOTAL > state["max_lifetime_spend"]:
        raise ActivationError("bounded wallet remaining lifetime budget is below 8.04 USDC")
    return state


def migration_manifest(path: Path) -> Path:
    source = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"
    manifest = json.loads(source.read_text(encoding="utf-8"))
    manifest["canonical"]["deterministic_verifier"] = V3
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def create_payload(document: dict[str, Any], published: Mapping[str, object]) -> dict[str, object]:
    terms = document["contract_terms"]
    policy = document["verification_policy"]
    return {
        "creator": terms["creator_wallet"],
        "solver_reward": terms["solver_reward"],
        "verifier_reward": terms["verifier_reward"],
        "terms_hash": published["terms_hash"],
        "policy_hash": published["policy_hash"],
        "acceptance_criteria_hash": published["acceptance_criteria_hash"],
        "benchmark_hash": published["benchmark_hash"],
        "evidence_schema_hash": published["evidence_schema_hash"],
        "funding_deadline": terms["funding_deadline"],
        "claim_window_seconds": terms["claim_window_seconds"],
        "verification_window_seconds": terms["verification_window_seconds"],
        "verification_mode": "deterministic_module",
        "verifier_module": policy["verifier_module"],
        "verifier_reward_recipient": policy["verifier_reward_recipient"],
        "verifiers": [],
        "threshold": 1,
        "initial_funding": terms["initial_funding"],
        "creation_nonce": terms["creation_nonce"],
    }


def event_kinds(api: str, bounty_id: str) -> set[str]:
    events = http_json(
        "GET",
        f"{api}/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id={bounty_id}",
    )
    if not isinstance(events, list):
        raise ActivationError("events endpoint returned a non-list")
    return {str(item.get("kind")) for item in events if isinstance(item, dict)}


def reconcile(api: str, contract: str, bounty_id: str, timeout_seconds: int = 150) -> dict[str, Any]:
    required = {"canonical_bounty_created", "funding_added", "bounty_became_claimable"}
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        kinds = event_kinds(api, bounty_id)
        if required.issubset(kinds):
            feed = http_json(
                "GET",
                f"{api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false",
            )
            if not isinstance(feed, list):
                raise ActivationError("feed endpoint returned a non-list")
            item = next(
                (
                    candidate for candidate in feed
                    if isinstance(candidate, dict)
                    and str(candidate.get("bounty_contract", "")).lower() == contract.lower()
                ),
                None,
            )
            if item and item.get("status") == "claimable" and item.get("terms_valid") is True \
                    and item.get("verification_ready") is True:
                return {"event_kinds": sorted(kinds), "feed_item": item}
        time.sleep(3)
    raise ActivationError(f"canonical activation did not reconcile for {contract}")


def issue_body(issue: int, lane: str, old: str, result: Mapping[str, object]) -> str:
    contract = str(result["contract"])
    tx_hash = str(result["transaction_hash"])
    return f"""## Goal

Create and fully fund a concrete **1 USDC {lane} child bounty** that a different registered participant completes and receives canonical settlement for. Completing the parent pays **2 USDC**, producing **1 USDC gross profit** when the child is self-funded at exactly 1 USDC.

## Live payment evidence

**Funded and claimable on Base mainnet.**

- V3 contract: `{contract}`
- Creation and funding transaction: `https://basescan.org/tx/{tx_hash}`
- Confirmed funding: **2.01 / 2.01 USDC**
- Parent solver reward: **2.00 USDC**
- Automated verifier reward: **0.01 USDC**
- Refundable claim bond: **0.01 USDC**
- Required child target: **1.00 USDC minimum**
- Guaranteed parent gross margin: **1.00 USDC minimum**
- Status: `claimable`
- Verifier: profitable standing-meta-v3 `{V3}`

The previous unprofitable V2 contract `{old}` is retired from earning discovery and preserved as immutable history.

## Earn this bounty

This is a coordination bounty, not a direct code-fix bounty.

1. Use a Base wallet with at least **0.01 USDC** for the refundable parent claim bond. Eligible gas is sponsored.
2. Find a different intended child solver. Both participants register before the parent claim with `/agent-bounty register 0xYourBaseWallet`.
3. Before claiming, prepare and publish exact parent-bound child terms for a concrete **{lane}** coding task using the committed `sandboxed_regression_v1` threshold-two verifier quorum.
4. Create and fully fund that child with a total target of **exactly 1.00 USDC** to retain the full 1.00 USDC gross parent margin.
5. Wait for confirmed child creation, funding, claimability, and a strictly later Base timestamp than terms publication and both registrations.
6. Claim this parent. The different child participant completes the child and receives canonical `BountySettled` payment.
7. Submit `abi.encode(address childBounty)` to this parent with the requested discovery feedback. The V3 verifier releases the 2.00 USDC parent reward only when every immutable condition passes.

A claim comment, signature request, transaction hash, or accepted submission is not payment. Only canonical `BountySettled` proves earnings.

**Post your own bounty:** https://agentbounties.app/post.html

<!-- agent-bounties-github-metadata-v1 -->
## Automation metadata

### Goal
Create and fully fund a concrete 1 USDC {lane} child bounty that a different registered participant completes and receives canonical settlement for, leaving at least 1 USDC gross parent profit.

### Acceptance criteria
Satisfy every immutable profitable standing-meta-v3 condition: pre-claim terms and registrations, a 1 USDC minimum parent-bound child, at least 1 USDC retained parent margin, the committed regression quorum, an independent child participant, canonical child settlement, and exact parent proof.

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
    before = policy_state(cast)
    manifest = migration_manifest(args.output_dir / "bounded-wallet-v3-migration-manifest.json")
    results: list[dict[str, Any]] = []

    for issue, config in ISSUES.items():
        document = json.loads((ROOT / str(config["terms"])).read_text(encoding="utf-8"))
        published = http_json(
            "POST",
            f"{args.api}/v1/base/autonomous-bounties/terms",
            {"creator_wallet": WALLET, "document": document},
        )
        if not isinstance(published, dict):
            raise ActivationError("terms publication returned a non-object")
        create = create_payload(document, published)
        plan = http_json(
            "POST",
            f"{args.api}/v1/base/autonomous-bounties/creation-plan",
            {"network": "base-mainnet", "create": create},
        )
        if not isinstance(plan, dict):
            raise ActivationError("creation plan returned a non-object")
        plan_path = args.output_dir / f"v3-{issue}-creation-plan.json"
        plan_path.write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        action_path = args.output_dir / f"v3-{issue}-bounded-action.json"
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
        direct = action["direct_transaction"]
        predicted = address(plan["predicted_bounty_contract"], "predicted bounty")
        bounty_id = str(plan["bounty_id"]).lower()
        canonical = str(cast.call(FACTORY, "isCanonicalBounty(address)(bool)", predicted)).strip().lower()
        if canonical == "true":
            tx_hash = "already-canonical"
        else:
            sent = cast.send_data(address(direct["to"], "direct transaction target"), str(direct["data"]), private_key)
            tx_hash = str(sent.get("transactionHash") or sent.get("transaction_hash"))
        reconciled = reconcile(args.api, predicted, bounty_id)
        result = {
            "issue": issue,
            "lane": config["lane"],
            "old_contract": config["old"],
            "contract": predicted,
            "bounty_id": bounty_id,
            "transaction_hash": tx_hash,
            "terms_hash": published["terms_hash"],
            "reconciliation": reconciled,
        }
        (args.output_dir / f"v3-{issue}-issue.md").write_text(
            issue_body(issue, str(config["lane"]), str(config["old"]), result), encoding="utf-8"
        )
        results.append(result)

    after = policy_state(cast)
    if before["lifetime_spent"] + TOTAL != after["lifetime_spent"]:
        raise ActivationError("wallet lifetime spend did not increase by exactly 8.04 USDC")
    if before["wallet_balance"] - TOTAL != after["wallet_balance"]:
        raise ActivationError("wallet balance did not decrease by exactly 8.04 USDC")
    report = {
        "schema": "agent-bounties/standing-meta-v3-replacement-activation-v1",
        "network": "base-mainnet",
        "wallet": WALLET,
        "wallet_balance_before": before["wallet_balance"],
        "wallet_balance_after": after["wallet_balance"],
        "lifetime_spent_before": before["lifetime_spent"],
        "lifetime_spent_after": after["lifetime_spent"],
        "results": results,
        "evidence_boundary": (
            "Confirmed canonical creation, FundingAdded, BountyBecameClaimable, valid terms, and verification readiness "
            "prove the replacement parents are live. Only BountySettled proves future solver payment."
        ),
    }
    return report


def markdown(report: Mapping[str, object]) -> str:
    results = report["results"]
    assert isinstance(results, list)
    lines = [
        "## Four profitable V3 replacements activated",
        "",
        f"- Bounded wallet before: **{int(report['wallet_balance_before']) / 1_000_000:.6f} USDC**",
        f"- Bounded wallet after: **{int(report['wallet_balance_after']) / 1_000_000:.6f} USDC**",
        f"- Exact migration spend: **{TOTAL / 1_000_000:.6f} USDC**",
        "",
    ]
    for item in results:
        assert isinstance(item, dict)
        lines.append(
            f"- #{item['issue']} `{item['contract']}` — 2.00 USDC solver / 0.01 verifier / 0.01 refundable bond"
        )
    lines.extend([
        "",
        "Each contract reconciled canonical creation, funding, claimability, valid terms, and verification readiness.",
        "Only a future canonical BountySettled event proves solver payment.",
    ])
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", RPC_DEFAULT))
    parser.add_argument("--api", default=API_DEFAULT)
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--python", default=os.environ.get("PYTHON_BIN", "python"))
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target" / "standing-meta-v3-activation")
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "standing-meta-v3-activation.json")
    parser.add_argument("--markdown-output", type=Path, default=ROOT / "target" / "standing-meta-v3-activation.md")
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = activate(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.markdown_output.write_text(markdown(report), encoding="utf-8")
    print(json.dumps({
        "activated": [item["issue"] for item in report["results"]],
        "wallet_balance_after": report["wallet_balance_after"],
        "output": str(args.output),
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
