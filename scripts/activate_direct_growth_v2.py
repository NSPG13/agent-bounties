#!/usr/bin/env python3
"""Create, fund, and reconcile manifest-pinned direct coding bounties."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Mapping, Sequence


ROOT = Path(__file__).resolve().parents[1]
MANIFEST_PATH = ROOT / "bounties" / "autonomous-v1" / "direct-growth-v2-manifest.json"
DEFAULT_CONTRACT_MANIFEST = (
    ROOT / "deployments" / "bounded-agent-wallet-v2-base-mainnet.json"
)
RPC_DEFAULT = "https://mainnet.base.org"
API_DEFAULT = "https://api.agentbounties.app"
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
DIRECTORY_DIGEST_DOMAIN = b"agent-bounties/directory-v1\0"


class ActivationError(RuntimeError):
    pass


def redact(command: Sequence[str], output: str = "") -> str:
    rendered = output
    for index, value in enumerate(command[:-1]):
        if value in {"--private-key", "--rpc-url"}:
            rendered = rendered.replace(command[index + 1], "[redacted]")
    return rendered


def run(command: Sequence[str], *, timeout: int = 300) -> str:
    completed = subprocess.run(
        list(command),
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
    if completed.returncode != 0:
        safe_command = []
        hide = False
        for value in command:
            if hide:
                safe_command.append("[redacted]")
                hide = False
            else:
                safe_command.append(value)
                hide = value in {"--private-key", "--rpc-url"}
        raise ActivationError(
            f"command failed ({completed.returncode}): {' '.join(safe_command)}\n"
            f"{redact(command, completed.stdout)[-6000:]}"
        )
    return completed.stdout.strip()


def address(value: object, label: str) -> str:
    result = str(value).strip().lower()
    if not ADDRESS_RE.fullmatch(result):
        raise ActivationError(f"{label} is not an EVM address")
    return result


def bytes32(value: object, label: str) -> str:
    result = str(value).strip().lower()
    if not BYTES32_RE.fullmatch(result):
        raise ActivationError(f"{label} is not bytes32")
    return result


def benchmark_digest(subdirectory: str) -> str:
    root = ROOT / subdirectory
    if not root.is_dir():
        raise ActivationError(f"benchmark directory is missing: {subdirectory}")
    listed = run(
        [
            "git",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            subdirectory,
        ]
    )
    paths = sorted(
        line.strip().replace("\\", "/") for line in listed.splitlines() if line.strip()
    )
    if not paths:
        raise ActivationError(
            f"benchmark directory has no publishable files: {subdirectory}"
        )
    hasher = hashlib.sha256(DIRECTORY_DIGEST_DOMAIN)
    prefix = subdirectory.rstrip("/") + "/"
    for repository_path in paths:
        if not repository_path.startswith(prefix):
            raise ActivationError(
                f"benchmark file escaped its directory: {repository_path}"
            )
        relative = repository_path[len(prefix) :]
        path_bytes = relative.encode("utf-8")
        payload = (ROOT / repository_path).read_bytes()
        hasher.update(len(path_bytes).to_bytes(8, "big"))
        hasher.update(path_bytes)
        hasher.update(len(payload).to_bytes(8, "big"))
        hasher.update(payload)
    return f"sha256:{hasher.hexdigest()}"


def http_json(method: str, url: str, body: Mapping[str, object] | None = None) -> Any:
    payload = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    request = urllib.request.Request(
        url,
        data=payload,
        method=method,
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-direct-growth/2",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            raw = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise ActivationError(
            f"{method} {url} failed with HTTP {error.code}: {detail[:2000]}"
        ) from error
    try:
        return json.loads(raw)
    except json.JSONDecodeError as error:
        raise ActivationError(f"{method} {url} returned invalid JSON") from error


def rpc_chain_id(url: str) -> int:
    request = urllib.request.Request(
        url,
        data=b'{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}',
        method="POST",
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-direct-growth/2",
        },
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return int(str(payload.get("result", "")), 16)


def select_rpc_url(value: str) -> str:
    candidates = [
        candidate.strip() for candidate in value.split(",") if candidate.strip()
    ]
    for candidate in candidates:
        parsed = urllib.parse.urlparse(candidate)
        if parsed.scheme != "https" or not parsed.netloc:
            continue
        try:
            if rpc_chain_id(candidate) == 8453:
                return candidate
        except (OSError, ValueError, json.JSONDecodeError):
            continue
    raise ActivationError(
        "no configured HTTPS RPC endpoint returned Base chain ID 8453"
    )


class Cast:
    def __init__(self, executable: str, rpc_url: str) -> None:
        self.executable = executable
        self.rpc_url = rpc_url

    def rpc(self, *arguments: str, timeout: int = 300) -> str:
        return run(
            [self.executable, *arguments, "--rpc-url", self.rpc_url], timeout=timeout
        )

    def call(self, target: str, signature: str, *arguments: str) -> str:
        return self.rpc("call", target, signature, *arguments).strip()

    def send_data(self, target: str, data: str, private_key: str) -> str:
        raw = self.rpc(
            "send",
            target,
            "--data",
            data,
            "--private-key",
            private_key,
            "--json",
            timeout=180,
        )
        try:
            sent = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ActivationError("cast send did not return JSON") from error
        tx_hash = str(sent.get("transactionHash") or sent.get("transaction_hash") or "")
        bytes32(tx_hash, "transaction hash")
        receipt = json.loads(self.rpc("receipt", tx_hash, "--json", timeout=180))
        if receipt.get("status") not in {"0x1", "0x01", 1}:
            raise ActivationError(f"transaction reverted: {tx_hash}")
        return tx_hash


def cast_uint(value: str, label: str) -> int:
    match = re.match(r"^\s*(\d+)\b", value)
    if match is None:
        raise ActivationError(f"{label} did not return an unsigned integer")
    return int(match.group(1))


def latest_wallet_state(cast: Cast, manifest: Mapping[str, Any]) -> dict[str, int]:
    wallet = manifest["wallet"]
    return {
        "wallet_usdc_balance": cast_uint(
            cast.call(
                manifest["settlement_token"],
                "balanceOf(address)(uint256)",
                wallet,
            ),
            "wallet USDC balance",
        ),
        "lifetime_spent": cast_uint(
            cast.call(wallet, "lifetimeSpent()(uint256)"), "wallet lifetime spend"
        ),
        "period_spent": cast_uint(
            cast.call(wallet, "periodSpent()(uint256)"), "wallet period spend"
        ),
        "delegate_nonce": cast_uint(
            cast.call(wallet, "delegateNonce()(uint256)"), "wallet delegate nonce"
        ),
    }


def is_canonical(cast: Cast, factory: str, bounty: str) -> bool:
    result = cast.call(factory, "isCanonicalBounty(address)(bool)", bounty).lower()
    if result not in {"true", "false"}:
        raise ActivationError("canonical-bounty check did not return a boolean")
    return result == "true"


def wait_for_canonical(
    cast: Cast, factory: str, bounty: str, timeout_seconds: int
) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while True:
        if is_canonical(cast, factory, bounty):
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(2)


def create_if_missing(
    cast: Cast,
    factory: str,
    bounty: str,
    target: str,
    data: str,
    private_key: str,
    recovery_timeout: int,
) -> tuple[str | None, bool]:
    if is_canonical(cast, factory, bounty):
        return None, False
    try:
        return cast.send_data(target, data, private_key), True
    except ActivationError:
        # A provider may accept the broadcast and then fail while polling its
        # receipt. The deterministic contract is the authoritative postcondition.
        if wait_for_canonical(cast, factory, bounty, recovery_timeout):
            return None, True
        raise


def load_manifest(path: Path = MANIFEST_PATH) -> dict[str, Any]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ActivationError(f"invalid activation manifest: {error}") from error
    if manifest.get("schema") not in {
        "agent-bounties/direct-growth-activation-v2",
        "agent-bounties/direct-inventory-activation-v1",
    }:
        raise ActivationError("activation manifest schema mismatch")
    if manifest.get("network") != "base-mainnet" or manifest.get("chain_id") != 8453:
        raise ActivationError("activation manifest is not pinned to Base mainnet")
    for key in ("wallet", "owner", "delegate", "canonical_factory", "settlement_token"):
        manifest[key] = address(manifest.get(key), key)
    manifest["wallet_policy_hash"] = bytes32(
        manifest.get("wallet_policy_hash"), "policy hash"
    )
    manifest["verifier_set_hash"] = bytes32(
        manifest.get("verifier_set_hash"), "verifier set hash"
    )
    manifest["verifiers"] = [
        address(value, "verifier") for value in manifest.get("verifiers", [])
    ]
    threshold = int(manifest.get("threshold", 0))
    if not 1 <= threshold <= len(manifest["verifiers"]):
        raise ActivationError("direct coding tasks require a valid committed verifier quorum")
    if len(manifest["verifiers"]) > 8:
        raise ActivationError("direct coding tasks support at most eight verifier wallets")
    verifier_reward = int(manifest.get("verifier_reward", 0))
    solver_reward = int(manifest.get("solver_reward", 0))
    initial_funding = int(manifest.get("initial_funding", 0))
    if solver_reward <= 0 or verifier_reward <= 0:
        raise ActivationError("solver and verifier rewards must be positive")
    if verifier_reward % threshold:
        raise ActivationError("verifier reward must divide evenly across the quorum")
    if initial_funding != solver_reward + verifier_reward:
        raise ActivationError("initial funding must equal solver plus verifier rewards")
    tasks = manifest.get("tasks")
    if not isinstance(tasks, list) or not 1 <= len(tasks) <= 10:
        raise ActivationError("activation manifest must contain one to ten tasks")
    if int(manifest.get("task_count", len(tasks))) != len(tasks):
        raise ActivationError("task count does not match the activation manifest")
    issues = set()
    digests = set()
    for task in tasks:
        if not isinstance(task, dict) or not isinstance(task.get("issue"), int):
            raise ActivationError("task issue is invalid")
        issues.add(task["issue"])
        bytes32(task.get("creation_nonce"), "creation nonce")
        digest = str(task.get("benchmark_digest", ""))
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
            raise ActivationError(f"issue #{task['issue']} benchmark digest is invalid")
        digests.add(digest)
        benchmark_subdirectory = str(task.get("benchmark_subdirectory", ""))
        benchmark = ROOT / benchmark_subdirectory / "check.py"
        if not benchmark.is_file():
            raise ActivationError(f"issue #{task['issue']} benchmark is missing")
        observed_digest = benchmark_digest(benchmark_subdirectory)
        if observed_digest != digest:
            raise ActivationError(
                f"issue #{task['issue']} benchmark digest differs: {observed_digest}"
            )
    if len(issues) != len(tasks) or len(digests) != len(tasks):
        raise ActivationError("task issues and benchmark digests must be unique")
    expected_total = initial_funding * len(tasks)
    if expected_total != int(manifest["total_funding"]):
        raise ActivationError("activation total does not match task funding")
    contract_manifest = manifest.get("wallet_contract_manifest")
    if contract_manifest is not None:
        path_value = Path(str(contract_manifest))
        if path_value.is_absolute() or ".." in path_value.parts:
            raise ActivationError("wallet contract manifest must stay inside the repository")
        if not (ROOT / path_value).is_file():
            raise ActivationError("wallet contract manifest is missing")
    return manifest


def contract_manifest_path(manifest: Mapping[str, Any]) -> Path:
    value = manifest.get("wallet_contract_manifest")
    return DEFAULT_CONTRACT_MANIFEST if value is None else ROOT / str(value)


def terms_document(
    manifest: Mapping[str, Any], task: Mapping[str, Any], commit: str
) -> dict[str, Any]:
    issue = int(task["issue"])
    benchmark = {
        "engine": "sandboxed_regression_v1",
        "source": {
            "kind": "github_commit",
            "repository": "NSPG13/agent-bounties",
            "commit": commit,
            "subdirectory": task["benchmark_subdirectory"],
        },
        "runner_manifest": {
            "schema_version": "agent-bounties/regression-sandbox-v1",
            "image": manifest["runner_image"],
            "command": ["python", "/benchmark/check.py"],
            "workdir": "/workspace",
            "benchmark_digest": task["benchmark_digest"],
            "timeout_seconds": 120,
            "cpu_millis": 1000,
            "memory_bytes": 268435456,
            "pids_limit": 64,
            "max_output_bytes": 1048576,
            "tmpfs_bytes": 134217728,
            "max_source_bytes": 536870912,
            "max_source_files": 50000,
            "max_benchmark_bytes": 1048576,
            "max_benchmark_files": 100,
            "platform": "linux/amd64",
            "test_seed": 1,
        },
    }
    return {
        "schema_version": "agent-bounties/terms-v1",
        "contract_terms": {
            "protocol_version": "agent-bounties/autonomous-v1",
            "creator_wallet": manifest["wallet"],
            "network": manifest["network"],
            "settlement_token": manifest["settlement_token"],
            "solver_reward": {"amount": manifest["solver_reward"], "currency": "usdc"},
            "verifier_reward": {
                "amount": manifest["verifier_reward"],
                "currency": "usdc",
            },
            "claim_bond": {"amount": manifest["verifier_reward"], "currency": "usdc"},
            "initial_funding": {
                "amount": manifest["initial_funding"],
                "currency": "usdc",
            },
            "funding_deadline": manifest["funding_deadline"],
            "claim_window_seconds": manifest["claim_window_seconds"],
            "verification_window_seconds": manifest["verification_window_seconds"],
            "creation_nonce": task["creation_nonce"],
        },
        "title": task["title"],
        "goal": task["goal"],
        "acceptance_criteria": task["acceptance_criteria"],
        "benchmark": benchmark,
        "evidence_schema": {
            "type": "object",
            "required": [
                "repository",
                "commit",
                "test_command",
                "source_snapshot_digest",
                "discovery_source",
                "participation_reason",
                "improvement_feedback",
            ],
            "additionalProperties": True,
        },
        "verification_policy": {
            "mechanism": "signed_quorum",
            "engine": "sandboxed_regression_v1",
            "verifiers": manifest["verifiers"],
            "threshold": manifest["threshold"],
            "rubric": "The precommitted sandbox runs the exact benchmark against the submitted GitHub commit. The configured verifier quorum signs only the resulting deterministic pass or fail candidate.",
            "self_verification_forbidden": True,
        },
        "source_url": f"https://github.com/NSPG13/agent-bounties/issues/{issue}",
        "discovery_source": manifest.get(
            "discovery_source",
            "maintainer-seeded direct growth inventory after organic label-search feedback",
        ),
    }


def create_payload(
    document: Mapping[str, Any], published: Mapping[str, Any]
) -> dict[str, Any]:
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
        "verification_mode": "signed_quorum",
        "verifier_module": None,
        "verifier_reward_recipient": None,
        "verifiers": policy["verifiers"],
        "threshold": policy["threshold"],
        "initial_funding": terms["initial_funding"],
        "creation_nonce": terms["creation_nonce"],
    }


def inspect_wallet(
    args: argparse.Namespace, manifest: Mapping[str, Any], suffix: str
) -> dict[str, Any]:
    output = args.output_dir / f"wallet-{suffix}.json"
    run(
        [
            args.python,
            "scripts/inspect_bounded_agent_wallet.py",
            "--wallet",
            manifest["wallet"],
            "--manifest",
            str(contract_manifest_path(manifest)),
            "--rpc-url",
            args.rpc_url,
            "--expect-owner",
            manifest["owner"],
            "--expect-delegate",
            manifest["delegate"],
            "--expect-policy-hash",
            manifest["wallet_policy_hash"],
            "--output",
            str(output),
        ]
    )
    report = json.loads(output.read_text(encoding="utf-8"))
    if report.get("ready") is not True:
        raise ActivationError(f"bounded wallet is not ready: {report.get('failures')}")
    state = report["state"]
    if int(state["policy_version"]) != int(manifest["wallet_policy_version"]):
        raise ActivationError("bounded wallet policy version changed")
    if int(state["policy"]["max_per_action"]) < int(manifest["initial_funding"]):
        raise ActivationError("bounded wallet per-action cap is below task funding")
    if (
        state["policy"]["signed_quorum_verifier_set_hash"]
        != manifest["verifier_set_hash"]
    ):
        raise ActivationError("bounded wallet signed verifier set changed")
    return report


def event_kinds(api: str, bounty_id: str) -> set[str]:
    events = http_json(
        "GET",
        f"{api}/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id={bounty_id}",
    )
    if not isinstance(events, list):
        raise ActivationError("events endpoint returned a non-list")
    return {str(item.get("kind")) for item in events if isinstance(item, dict)}


def reconcile(
    api: str, contract: str, bounty_id: str, timeout_seconds: int
) -> dict[str, Any]:
    required = {"canonical_bounty_created", "funding_added", "bounty_became_claimable"}
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        kinds = event_kinds(api, bounty_id)
        if required.issubset(kinds):
            feed = http_json(
                "GET",
                f"{api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false",
            )
            if isinstance(feed, list):
                item = next(
                    (
                        value
                        for value in feed
                        if isinstance(value, dict)
                        and str(value.get("bounty_contract", "")).lower()
                        == contract.lower()
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
        time.sleep(4)
    raise ActivationError(f"canonical activation did not reconcile for {contract}")


def usdc(amount: object) -> str:
    return f"{int(amount) / 1_000_000:.2f}"


def issue_body(
    manifest: Mapping[str, Any],
    task: Mapping[str, Any],
    result: Mapping[str, Any],
    commit: str,
) -> str:
    criteria = "\n".join(f"- {value}" for value in task["acceptance_criteria"])
    issue = int(task["issue"])
    return f"""## Goal

{task["goal"]}

## Funded payment contract

**Funded and claimable on Base mainnet.**

- Contract: `{result["contract"]}`
- Contract explorer: https://basescan.org/address/{result["contract"]}
- Creation/funding transaction: https://basescan.org/tx/{result["transaction_hash"]}
- Confirmed funding: **{usdc(manifest["initial_funding"])} / {usdc(manifest["initial_funding"])} USDC**
- Solver reward: **{usdc(manifest["solver_reward"])} USDC**
- Automated verifier reward and refundable claim bond: **{usdc(manifest["verifier_reward"])} USDC**
- Verification: `sandboxed_regression_v1`, {int(manifest["threshold"])} of {len(manifest["verifiers"])} precommitted automated signers
- Immutable benchmark: https://github.com/NSPG13/agent-bounties/tree/{commit}/{task["benchmark_subdirectory"]}

## Acceptance criteria

{criteria}

## Earn it

1. Comment `/claim #{issue} wallet: 0xYOUR_PUBLIC_BASE_ADDRESS`.
2. Sign the returned bounded claim request. Never share a private key or seed phrase.
3. Wait for the canonical claim state, then implement only this issue and open a focused PR.
4. Run the focused checks and submit the requested repository, commit, command, snapshot digest, and discovery feedback evidence.
5. The precommitted sandbox verifies the submitted commit. A passing result settles automatically; only canonical `BountySettled` proves payment.

How did you find this bounty, what made it worth attempting, and what should be easier next time?

**Post your own bounty:** https://agentbounties.app/post.html

<!-- agent-bounties-github-metadata-v1 -->
## Automation metadata

### Goal
{task["goal"]}

### Acceptance criteria
Pass the immutable benchmark and satisfy every criterion above.

### Template
small-code-change

### Suggested amount
{usdc(manifest["solver_reward"])} USDC

### Funding mode
AutonomousV1BaseUsdc

### Privacy
Public
"""


def activate(args: argparse.Namespace) -> dict[str, Any]:
    manifest = load_manifest(args.manifest)
    commit = args.commit.strip().lower()
    if not COMMIT_RE.fullmatch(commit):
        raise ActivationError("--commit must be the exact 40-character merged commit")
    private_key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
    if not private_key:
        raise ActivationError("BASE_KEEPER_PRIVATE_KEY is required")
    cast = Cast(args.cast, args.rpc_url)
    delegate = address(
        run([args.cast, "wallet", "address", "--private-key", private_key]), "delegate"
    )
    if delegate != manifest["delegate"]:
        raise ActivationError(
            f"delegate key resolves to {delegate}, expected {manifest['delegate']}"
        )

    inspect_wallet(args, manifest, "before")
    before = latest_wallet_state(cast, manifest)
    (args.output_dir / "wallet-before-latest.json").write_text(
        json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    results: list[dict[str, Any]] = []
    new_spend = 0
    for task in manifest["tasks"]:
        issue = int(task["issue"])
        document = terms_document(manifest, task, commit)
        published = http_json(
            "POST",
            f"{args.api}/v1/base/autonomous-bounties/terms",
            {"creator_wallet": manifest["wallet"], "document": document},
        )
        if not isinstance(published, dict):
            raise ActivationError(
                f"issue #{issue} terms publication returned a non-object"
            )
        plan = http_json(
            "POST",
            f"{args.api}/v1/base/autonomous-bounties/creation-plan",
            {
                "network": manifest["network"],
                "create": create_payload(document, published),
            },
        )
        if not isinstance(plan, dict):
            raise ActivationError(f"issue #{issue} creation plan returned a non-object")
        plan_path = args.output_dir / f"issue-{issue}-creation-plan.json"
        plan_path.write_text(
            json.dumps(plan, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        action_path = args.output_dir / f"issue-{issue}-bounded-action.json"
        run(
            [
                args.python,
                "scripts/plan_bounded_agent_action.py",
                "create",
                "--wallet",
                manifest["wallet"],
                "--creation-plan",
                str(plan_path),
                "--manifest",
                str(contract_manifest_path(manifest)),
                "--rpc-url",
                args.rpc_url,
                "--expect-owner",
                manifest["owner"],
                "--expect-delegate",
                manifest["delegate"],
                "--expect-policy-hash",
                manifest["wallet_policy_hash"],
                "--output",
                str(action_path),
            ]
        )
        action = json.loads(action_path.read_text(encoding="utf-8"))
        direct = action["direct_transaction"]
        predicted = address(plan.get("predicted_bounty_contract"), "predicted bounty")
        bounty_id = bytes32(plan.get("bounty_id"), "bounty id")
        tx_hash, created_now = create_if_missing(
            cast,
            manifest["canonical_factory"],
            predicted,
            address(direct.get("to"), "direct transaction target"),
            str(direct.get("data")),
            private_key,
            args.broadcast_recovery_timeout,
        )
        if created_now:
            new_spend += int(manifest["initial_funding"])
        reconciled = reconcile(args.api, predicted, bounty_id, args.reconcile_timeout)
        feed_events = reconciled["feed_item"].get("events") or []
        feed_tx = next(
            (
                str(event.get("tx_hash", ""))
                for event in feed_events
                if isinstance(event, dict)
                and event.get("kind") == "canonical_bounty_created"
            ),
            "",
        )
        if tx_hash is None:
            tx_hash = feed_tx
        bytes32(tx_hash, "creation transaction hash")
        result = {
            "issue": issue,
            "slug": task["slug"],
            "contract": predicted,
            "bounty_id": bounty_id,
            "transaction_hash": tx_hash,
            "terms_hash": bytes32(published.get("terms_hash"), "terms hash"),
            "reconciliation": reconciled,
        }
        (args.output_dir / f"issue-{issue}.md").write_text(
            issue_body(manifest, task, result, commit), encoding="utf-8"
        )
        results.append(result)

    inspect_wallet(args, manifest, "after")
    after = latest_wallet_state(cast, manifest)
    (args.output_dir / "wallet-after-latest.json").write_text(
        json.dumps(after, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    if (
        int(after["lifetime_spent"]) - int(before["lifetime_spent"])
        != new_spend
    ):
        raise ActivationError(
            "bounded wallet lifetime-spend delta does not match newly created bounties"
        )
    if (
        int(before["wallet_usdc_balance"])
        - int(after["wallet_usdc_balance"])
        != new_spend
    ):
        raise ActivationError(
            "bounded wallet balance delta does not match newly created bounties"
        )
    return {
        "schema": "agent-bounties/direct-growth-activation-result-v2",
        "network": manifest["network"],
        "commit": commit,
        "wallet": manifest["wallet"],
        "new_spend": new_spend,
        "total_funding": int(manifest["total_funding"]),
        "wallet_balance_before": int(before["wallet_usdc_balance"]),
        "wallet_balance_after": int(after["wallet_usdc_balance"]),
        "solver_reward": int(manifest["solver_reward"]),
        "verifier_reward": int(manifest["verifier_reward"]),
        "results": results,
        "complete": len(results) == len(manifest["tasks"]),
        "evidence_boundary": "Canonical creation, funding, claimability, valid terms, and verification readiness are reconciled. Only a future BountySettled event proves solver payment.",
    }


def markdown(report: Mapping[str, Any]) -> str:
    lines = [
        f"## {len(report['results'])} direct coding bounties activated",
        "",
        f"- Canonical funding in this batch: **{int(report['total_funding']) / 1_000_000:.2f} USDC**",
        f"- Newly spent by this run: **{int(report['new_spend']) / 1_000_000:.2f} USDC**",
        f"- Bounded wallet remaining: **{int(report['wallet_balance_after']) / 1_000_000:.2f} USDC**",
        "",
    ]
    for item in report["results"]:
        solver = usdc(report["solver_reward"])
        verifier = usdc(report["verifier_reward"])
        lines.append(
            f"- #{item['issue']} `{item['contract']}` - {solver} USDC solver / {verifier} USDC verifier"
        )
    lines.extend(
        [
            "",
            "Every item reconciled canonical creation, FundingAdded, BountyBecameClaimable, valid terms, and verifier readiness.",
            "Only a future canonical BountySettled event proves solver payment.",
        ]
    )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=MANIFEST_PATH)
    parser.add_argument("--commit", default=os.environ.get("GITHUB_SHA", ""))
    parser.add_argument(
        "--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", RPC_DEFAULT)
    )
    parser.add_argument("--api", default=API_DEFAULT.rstrip("/"))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--python", default=os.environ.get("PYTHON_BIN", "python"))
    parser.add_argument("--reconcile-timeout", type=int, default=300)
    parser.add_argument("--broadcast-recovery-timeout", type=int, default=45)
    parser.add_argument(
        "--output-dir", type=Path, default=ROOT / "target" / "direct-growth-v2"
    )
    parser.add_argument(
        "--output", type=Path, default=ROOT / "target" / "direct-growth-v2.json"
    )
    parser.add_argument(
        "--markdown-output", type=Path, default=ROOT / "target" / "direct-growth-v2.md"
    )
    args = parser.parse_args()
    args.api = args.api.rstrip("/")
    args.rpc_url = select_rpc_url(args.rpc_url)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = activate(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    args.markdown_output.write_text(markdown(report), encoding="utf-8")
    print(
        json.dumps(
            {"complete": report["complete"], "new_spend": report["new_spend"]},
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
