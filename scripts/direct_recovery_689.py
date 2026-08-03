#!/usr/bin/env python3
"""Execute the exact, disclosed #689 direct-bounty recovery lane."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
from typing import Any, Mapping, Sequence
from urllib.parse import urlparse


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "ops" / "recovery" / "direct-mainnet-689.json"
DEFAULT_RPC = "https://mainnet.base.org"
MANIFEST_SCHEMA = "agent-bounties/direct-recovery-689-v1"
CANDIDATE_SCHEMA = "agent-bounties/direct-recovery-candidate-v1"
ATTESTATION_SCHEMA = "agent-bounties/direct-recovery-attestation-v1"
EVIDENCE_SCHEMA = "agent-bounties/direct-recovery-evidence-v1"
EXACT_CONTRACTS = {
    "0xc13ccf6c6a03b53f836d433c5e628f06bbc1dbf4",
    "0xad4532e45d371ff5b5c40ebbf0c20687ed9e6fc4",
    "0xf2e47a253988e98f535ab60f4b9bd7f8975c1263",
    "0x2afb91d160200fac4b91e6134b2cc9d9bff86f42",
    "0xc710d54d192ffb0b84cd6e051754ab70acf1130c",
}
EXACT_NOTICE_URL = "https://github.com/NSPG13/agent-bounties/issues/689"
EXACT_ACCEPTED_REVISION = "d58fa377e9081f963f9a380b11a29f51fa57421a"
EXACT_ACCEPTED_PULL_REQUEST_URL = "https://github.com/NSPG13/agent-bounties/pull/702"
EXACT_CHECK_RUN_URL = (
    "https://github.com/NSPG13/agent-bounties/actions/runs/30441776495"
)
EXACT_SETTLEMENT_TOKEN = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
EXACT_CREATOR = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
EXACT_OPERATOR_SOLVER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
EXACT_RETURN_RECIPIENT = "0x884834e884d6e93462655a2820140ad03e6747bc"
EXACT_VERIFIERS = (
    "0xbe6292b9e465f549e2363b918d6dd9187038431e",
    "0xb7c2ce6430b66fb986e27b6140b29309550d487a",
)
EXACT_VERIFIER_SET_HASH = (
    "0x2c5a10915ca1fb99d4a11e2222b4f32b986b4e0f5599f55d70e9c8f9725a28cd"
)
EXACT_REPOSITORY_CHECK = ["python", "scripts/check.py", "--platform", "posix"]
ACCEPTANCE_ENV_EXCLUSIONS = frozenset(
    {
        "BASE_MAINNET_RPC_URL",
        "BASE_KEEPER_PRIVATE_KEY",
        "RECOVERY_VERIFIER_KEY",
        "EXPECTED_SIGNER",
        "REVISION",
        "PULL_REQUEST_URL",
        "CHECK_RUN_URL",
        "GH_TOKEN",
    }
)
EXACT_ISSUE_CHECKS = {
    634: [
        "cargo",
        "test",
        "-p",
        "mcp-server",
        "mounted_public_inventory_tool_is_callable_and_fails_closed",
    ],
    635: ["python", "scripts/test_profitable_inventory_contract.py", "-v"],
    636: [
        "cargo",
        "test",
        "-p",
        "api",
        "canonical_cash_economics_cover_direct_standing_meta_and_unprofitable",
    ],
    637: ["python", "scripts/test_activate_routed_v3_replacements.py", "-v"],
    638: [
        "cargo",
        "test",
        "-p",
        "chain-base",
        "builds_content_addressed_autonomous_terms_commitments",
    ],
}
EXACT_CANDIDATE_HASHES = {
    634: {
        "submission_hash": "0x031a83cf6f0c310c1a2af303607e00fedf44b64bee89e595e5ba3dcdb0b0586e",
        "evidence_hash": "0x754efb520ba132d1bf3efd4e0710ab8c7431df7e7b1061224807460bdda3a957",
        "response_hash": "0x67d0eb91be2f50fa77046f681a84b08c639d1fe2fabbfa5d3aae64ba2e0487a3",
    },
    635: {
        "submission_hash": "0x031a83cf6f0c310c1a2af303607e00fedf44b64bee89e595e5ba3dcdb0b0586e",
        "evidence_hash": "0x20f5bfab3381ffac163b092bf667555b2d4f936f8cf85625acaae0011888ed5b",
        "response_hash": "0xe22eb3f6af9ad94e25d02cc81ddcdf6e78f9f17b5da3e99756cbc47bed036736",
    },
    636: {
        "submission_hash": "0x031a83cf6f0c310c1a2af303607e00fedf44b64bee89e595e5ba3dcdb0b0586e",
        "evidence_hash": "0x8a8b96f0d446ee6ee1adb5a7415ac95933bb07354dc71467745e78bb79cec116",
        "response_hash": "0x566d42356dde733852343508e600244e0b68618248e668da205ce3a5455b854c",
    },
    637: {
        "submission_hash": "0x031a83cf6f0c310c1a2af303607e00fedf44b64bee89e595e5ba3dcdb0b0586e",
        "evidence_hash": "0x1377ca2fd812ac65c62bf38f31f02e49b49cd9f5609f4181f7a0bd01113ba150",
        "response_hash": "0xa6bc6f63d245fb3ea143ace30af396fbe560c01fc297b72d9550f35c0f720a60",
    },
    638: {
        "submission_hash": "0x031a83cf6f0c310c1a2af303607e00fedf44b64bee89e595e5ba3dcdb0b0586e",
        "evidence_hash": "0x2b297e98398e12bb41c72f07b4d83b53e735c080b1e74cafe5d9dbbd59f0ebf5",
        "response_hash": "0x735df8048a3fc6fc6650888928208717b268dcb86bfdea73ba355640c08468d0",
    },
}
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
UINT = re.compile(r"^(?:0x[0-9a-fA-F]+|[0-9]+)")
RPC_RETRY_MARKERS = (
    "http error 403",
    "http error 408",
    "http error 429",
    "http error 500",
    "http error 502",
    "http error 503",
    "http error 504",
    "archive requests require",
    "connection",
    "max retries exceeded",
    "rate limit",
    "temporarily unavailable",
    "timed out",
    "timeout",
    "usage limit",
)


class RecoveryError(RuntimeError):
    pass


def canonical_json(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def sha256_text(value: str) -> str:
    return "0x" + hashlib.sha256(value.encode("utf-8")).hexdigest()


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def address(value: object, field: str) -> str:
    normalized = str(value or "").strip().lower()
    if not ADDRESS.fullmatch(normalized):
        raise RecoveryError(f"{field} must be a lowercase EVM address")
    return normalized


def bytes32(value: object, field: str) -> str:
    normalized = str(value or "").strip().lower()
    if not HASH.fullmatch(normalized):
        raise RecoveryError(f"{field} must be bytes32")
    return normalized


def uint(value: object, field: str) -> int:
    match = UINT.match(str(value).strip())
    if not match:
        raise RecoveryError(f"{field} must be an unsigned integer")
    return int(match.group(0), 0)


def require_https(value: object, field: str) -> str:
    text = str(value or "").strip()
    if not text.startswith("https://") or len(text) > 2_000:
        raise RecoveryError(f"{field} must be a bounded HTTPS URL")
    return text


def rpc_urls(value: object) -> list[str]:
    values = [item.strip() for item in str(value or "").split(",") if item.strip()]
    if not values or len(values) > 8:
        raise RecoveryError("rpc-url must contain between one and eight endpoints")
    endpoints: list[str] = []
    for item in values:
        parsed = urlparse(item)
        if (
            parsed.scheme != "https"
            or not parsed.netloc
            or parsed.username
            or parsed.password
            or parsed.fragment
        ):
            raise RecoveryError("every RPC endpoint must be a credential-free HTTPS URL")
        if item not in endpoints:
            endpoints.append(item)
    return endpoints


def require_pull_request_url(value: object) -> str:
    text = require_https(value, "pull_request_url")
    if not re.fullmatch(
        r"https://github\.com/NSPG13/agent-bounties/pull/[1-9][0-9]*",
        text,
    ):
        raise RecoveryError(
            "pull_request_url must identify the reviewed Agent Bounties PR"
        )
    return text


def redact(command: Sequence[str]) -> str:
    result: list[str] = []
    hide = False
    for item in command:
        if hide:
            result.append("***")
            hide = False
            continue
        result.append(item)
        hide = item in {"--private-key", "--rpc-url"}
    return " ".join(result)


def run(
    command: Sequence[str],
    *,
    cwd: Path = ROOT,
    timeout: int = 1_800,
    env: Mapping[str, str] | None = None,
) -> str:
    completed = subprocess.run(
        list(command),
        cwd=cwd,
        env=env,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
    if completed.returncode != 0:
        raise RecoveryError(
            f"command failed ({completed.returncode}): {redact(command)}\n"
            f"{completed.stdout[-6_000:]}"
        )
    return completed.stdout.strip()


def acceptance_environment() -> dict[str, str]:
    return {
        name: value
        for name, value in os.environ.items()
        if name not in ACCEPTANCE_ENV_EXCLUSIONS
    }


def load_manifest(path: Path) -> dict[str, Any]:
    value = read_json(path)
    if not isinstance(value, dict) or value.get("schema") != MANIFEST_SCHEMA:
        raise RecoveryError("recovery manifest schema is invalid")
    if value.get("network") != "base-mainnet" or value.get("chain_id") != 8453:
        raise RecoveryError("recovery manifest is not pinned to Base mainnet")
    if require_https(value.get("notice_url"), "notice_url") != EXACT_NOTICE_URL:
        raise RecoveryError("recovery notice URL differs from public issue #689")
    for field in (
        "settlement_token",
        "creator",
        "operator_solver",
        "return_recipient",
    ):
        value[field] = address(value.get(field), field)
    value["verifier_set_hash"] = bytes32(
        value.get("verifier_set_hash"), "verifier_set_hash"
    )
    verifiers = [address(item, "verifier") for item in value.get("verifiers", [])]
    if len(verifiers) != 2 or len(set(verifiers)) != 2:
        raise RecoveryError("manifest must contain two distinct verifiers")
    value["verifiers"] = verifiers
    pinned = {
        "settlement_token": EXACT_SETTLEMENT_TOKEN,
        "creator": EXACT_CREATOR,
        "operator_solver": EXACT_OPERATOR_SOLVER,
        "return_recipient": EXACT_RETURN_RECIPIENT,
        "verifier_set_hash": EXACT_VERIFIER_SET_HASH,
    }
    for field, expected in pinned.items():
        if value[field] != expected:
            raise RecoveryError(f"{field} differs from the public recovery scope")
    if tuple(verifiers) != EXACT_VERIFIERS:
        raise RecoveryError("verifier order differs from the public recovery scope")
    if value.get("required_repository_command") != EXACT_REPOSITORY_CHECK:
        raise RecoveryError("required repository check differs from the reviewed gate")
    bounties = value.get("bounties")
    if not isinstance(bounties, list) or len(bounties) != 5:
        raise RecoveryError("manifest must contain the five disclosed direct bounties")
    observed = set()
    issues = set()
    for item in bounties:
        if not isinstance(item, dict):
            raise RecoveryError("bounty manifest entry must be an object")
        item["contract"] = address(item.get("contract"), "bounty contract")
        observed.add(item["contract"])
        issue = uint(item.get("issue"), "issue")
        issues.add(issue)
        for field in (
            "bounty_id",
            "terms_hash",
            "policy_hash",
            "acceptance_criteria_hash",
            "benchmark_hash",
            "evidence_schema_hash",
        ):
            item[field] = bytes32(item.get(field), field)
        check = item.get("check")
        if (
            not isinstance(check, list)
            or not check
            or not all(isinstance(part, str) and part for part in check)
        ):
            raise RecoveryError(f"issue #{issue} check is invalid")
        if check != EXACT_ISSUE_CHECKS.get(issue):
            raise RecoveryError(f"issue #{issue} check differs from the public criteria")
    if observed != EXACT_CONTRACTS or issues != {634, 635, 636, 637, 638}:
        raise RecoveryError("manifest allowlist differs from public notice #689")
    if value.get("metrics_classification") != "operator_recovery_excluded":
        raise RecoveryError("recovery metrics exclusion is missing")
    exact_economics = {
        "threshold": 2,
        "solver_reward_per_bounty": 1_990_000,
        "verifier_reward_per_bounty": 10_000,
        "claim_bond_per_bounty": 10_000,
    }
    for field, expected in exact_economics.items():
        if uint(value.get(field), field) != expected:
            raise RecoveryError(f"{field} differs from the public recovery economics")
    if uint(value.get("initial_solver_bond_total"), "initial_solver_bond_total") != 50_000:
        raise RecoveryError("initial recovery bond total must be exactly 0.05 USDC")
    if uint(value.get("exact_return_amount"), "exact_return_amount") != 10_000_000:
        raise RecoveryError("return amount must be exactly 10 USDC")
    return value


class Cast:
    def __init__(self, executable: str, rpc_url: str) -> None:
        self.executable = executable
        self.rpc_urls = rpc_urls(rpc_url)
        self._rpc_cursor = 0
        self._next_nonces: dict[str, int] = {}

    def rpc(self, *args: str, timeout: int = 300) -> str:
        errors: list[str] = []
        attempts = max(2, len(self.rpc_urls) * 2)
        for _ in range(attempts):
            endpoint = self.rpc_urls[self._rpc_cursor % len(self.rpc_urls)]
            self._rpc_cursor += 1
            try:
                return run(
                    [self.executable, *args, "--rpc-url", endpoint],
                    timeout=timeout,
                )
            except RecoveryError as error:
                message = str(error)
                if not self._retryable_rpc_error(message):
                    raise
                errors.append(message[-1_000:])
                time.sleep(0.25)
        raise RecoveryError(
            "all configured RPC endpoints failed with retryable errors:\n"
            + "\n---\n".join(errors[-len(self.rpc_urls) :])
        )

    @staticmethod
    def _retryable_rpc_error(message: str) -> bool:
        lowered = message.lower()
        return any(marker in lowered for marker in RPC_RETRY_MARKERS)

    def _publish(self, raw_transaction: str, tx_hash: str) -> None:
        errors: list[str] = []
        for endpoint in self.rpc_urls:
            try:
                observed = run(
                    [
                        self.executable,
                        "publish",
                        "--async",
                        raw_transaction,
                        "--rpc-url",
                        endpoint,
                    ],
                    timeout=60,
                ).lower()
                if observed != tx_hash:
                    raise RecoveryError("published transaction hash differs from signed bytes")
                return
            except RecoveryError as error:
                message = str(error)
                lowered = message.lower()
                if "already known" in lowered or "nonce too low" in lowered:
                    return
                if not self._retryable_rpc_error(message):
                    raise
                errors.append(message[-1_000:])
        raise RecoveryError(
            "raw transaction was not published by any configured endpoint:\n"
            + "\n---\n".join(errors)
        )

    def _receipt(self, tx_hash: str, raw_transaction: str, timeout: int = 300) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        last_republish = 0.0
        while time.monotonic() < deadline:
            for endpoint in self.rpc_urls:
                try:
                    raw = run(
                        [
                            self.executable,
                            "receipt",
                            "--json",
                            "--async",
                            tx_hash,
                            "--rpc-url",
                            endpoint,
                        ],
                        timeout=30,
                    )
                    if not raw or raw == "null":
                        continue
                    receipt = json.loads(raw)
                    if isinstance(receipt, dict) and receipt.get("blockNumber"):
                        return receipt
                except json.JSONDecodeError:
                    continue
                except RecoveryError as error:
                    message = str(error).lower()
                    if (
                        "not found" not in message
                        and "null" not in message
                        and not self._retryable_rpc_error(message)
                    ):
                        raise
            now = time.monotonic()
            if now - last_republish >= 15:
                self._publish(raw_transaction, tx_hash)
                last_republish = now
            time.sleep(2)
        raise RecoveryError(f"transaction receipt timed out: {tx_hash}")

    def call(self, target: str, signature: str, *args: str) -> str:
        return self.rpc("call", target, signature, *args).strip()

    def code(self, target: str) -> str:
        return self.rpc("code", target).strip().lower()

    def chain_id(self) -> int:
        return uint(self.rpc("chain-id"), "chain id")

    def wallet_address(self, key: str) -> str:
        return address(
            run([self.executable, "wallet", "address", "--private-key", key]),
            "private-key address",
        )

    def balance(self, token: str, account: str) -> int:
        return uint(
            self.call(token, "balanceOf(address)(uint256)", account),
            "token balance",
        )

    def native_balance(self, account: str) -> int:
        return uint(self.rpc("balance", account), "native balance")

    def next_nonce(self, key: str) -> int:
        if key not in self._next_nonces:
            signer = self.wallet_address(key)
            latest = uint(
                self.rpc("nonce", signer, "--block", "latest"),
                "latest keeper nonce",
            )
            pending = uint(
                self.rpc("nonce", signer, "--block", "pending"),
                "pending keeper nonce",
            )
            if pending != latest:
                raise RecoveryError(
                    "keeper has another pending transaction; retry after it confirms or drops"
                )
            self._next_nonces[key] = latest
        return self._next_nonces[key]

    def send(self, key: str, target: str, signature: str, *args: str) -> str:
        nonce = self.next_nonce(key)
        raw_transaction = self.rpc(
            "mktx",
            "--private-key",
            key,
            "--nonce",
            str(nonce),
            target,
            signature,
            *args,
            timeout=300,
        ).strip()
        if not re.fullmatch(r"0x[0-9a-fA-F]+", raw_transaction):
            raise RecoveryError("cast mktx did not return signed transaction bytes")
        tx_hash = run([self.executable, "keccak", raw_transaction]).strip().lower()
        if not HASH.fullmatch(tx_hash):
            raise RecoveryError("signed transaction hash is invalid")
        self._publish(raw_transaction, tx_hash)
        receipt = self._receipt(tx_hash, raw_transaction)
        status = str(receipt.get("status", ""))
        if status not in {"0x1", "0x01", "1"}:
            raise RecoveryError("transaction did not return a successful receipt")
        self._next_nonces[key] = nonce + 1
        return tx_hash

    def sign_digest(self, key: str, digest: str) -> str:
        signature = run(
            [
                self.executable,
                "wallet",
                "sign",
                "--no-hash",
                "--private-key",
                key,
                digest,
            ]
        ).strip().lower()
        if not re.fullmatch(r"0x[0-9a-f]{130}", signature):
            raise RecoveryError("cast returned an invalid signature")
        return signature


def audit_bounty(
    cast: Cast,
    manifest: Mapping[str, Any],
    bounty: Mapping[str, Any],
) -> dict[str, Any]:
    contract = str(bounty["contract"])
    if cast.code(contract) in {"0x", "0x0"}:
        raise RecoveryError(f"issue #{bounty['issue']} contract has no bytecode")
    exact = {
        "creator": address(cast.call(contract, "creator()(address)"), "creator"),
        "settlement_token": address(
            cast.call(contract, "settlementToken()(address)"), "settlement token"
        ),
        "bounty_id": bytes32(cast.call(contract, "bountyId()(bytes32)"), "bounty id"),
        "terms_hash": bytes32(cast.call(contract, "termsHash()(bytes32)"), "terms hash"),
        "policy_hash": bytes32(
            cast.call(contract, "policyHash()(bytes32)"), "policy hash"
        ),
        "acceptance_criteria_hash": bytes32(
            cast.call(contract, "acceptanceCriteriaHash()(bytes32)"),
            "acceptance criteria hash",
        ),
        "benchmark_hash": bytes32(
            cast.call(contract, "benchmarkHash()(bytes32)"), "benchmark hash"
        ),
        "evidence_schema_hash": bytes32(
            cast.call(contract, "evidenceSchemaHash()(bytes32)"),
            "evidence schema hash",
        ),
        "verifier_set_hash": bytes32(
            cast.call(contract, "verifierSetHash()(bytes32)"), "verifier set hash"
        ),
    }
    expected = {
        "creator": manifest["creator"],
        "settlement_token": manifest["settlement_token"],
        "bounty_id": bounty["bounty_id"],
        "terms_hash": bounty["terms_hash"],
        "policy_hash": bounty["policy_hash"],
        "acceptance_criteria_hash": bounty["acceptance_criteria_hash"],
        "benchmark_hash": bounty["benchmark_hash"],
        "evidence_schema_hash": bounty["evidence_schema_hash"],
        "verifier_set_hash": manifest["verifier_set_hash"],
    }
    for field, wanted in expected.items():
        if exact[field] != wanted:
            raise RecoveryError(
                f"issue #{bounty['issue']} {field} mismatch: {exact[field]} != {wanted}"
            )
    values = {
        "status": uint(cast.call(contract, "status()(uint8)"), "status"),
        "solver_reward": uint(
            cast.call(contract, "solverReward()(uint256)"), "solver reward"
        ),
        "verifier_reward": uint(
            cast.call(contract, "verifierReward()(uint256)"), "verifier reward"
        ),
        "target_amount": uint(
            cast.call(contract, "targetAmount()(uint256)"), "target amount"
        ),
        "funded_amount": uint(
            cast.call(contract, "fundedAmount()(uint256)"), "funded amount"
        ),
        "threshold": uint(cast.call(contract, "threshold()(uint8)"), "threshold"),
        "round": uint(cast.call(contract, "round()(uint64)"), "round"),
        "active_claim_bond": uint(
            cast.call(contract, "activeClaimBond()(uint256)"), "active claim bond"
        ),
        "solver": address(cast.call(contract, "solver()(address)"), "solver"),
        "submission_hash": bytes32(
            cast.call(contract, "submissionHash()(bytes32)"), "submission hash"
        ),
        "evidence_hash": bytes32(
            cast.call(contract, "evidenceHash()(bytes32)"), "evidence hash"
        ),
        "verification_expires_at": uint(
            cast.call(contract, "verificationExpiresAt()(uint64)"),
            "verification expiry",
        ),
    }
    wanted_values = {
        "solver_reward": manifest["solver_reward_per_bounty"],
        "verifier_reward": manifest["verifier_reward_per_bounty"],
        "target_amount": 2_000_000,
        "threshold": manifest["threshold"],
    }
    for field, wanted in wanted_values.items():
        if values[field] != wanted:
            raise RecoveryError(f"issue #{bounty['issue']} {field} drifted")
    verifier_output = cast.call(contract, "verifiers()(address[])")
    verifiers = [item.lower() for item in re.findall(r"0x[0-9a-fA-F]{40}", verifier_output)]
    if verifiers != list(manifest["verifiers"]):
        raise RecoveryError(f"issue #{bounty['issue']} verifier order drifted")
    if values["status"] == 1 and values["funded_amount"] != 2_000_000:
        raise RecoveryError(f"issue #{bounty['issue']} is not fully funded")
    return {**exact, **values, "contract": contract, "verifiers": verifiers}


def current_revision(expected: str | None = None) -> str:
    revision = run(["git", "rev-parse", "HEAD"]).lower()
    if not GIT_SHA.fullmatch(revision):
        raise RecoveryError("current Git revision is invalid")
    if expected and revision != expected.lower():
        raise RecoveryError(
            f"recovery revision is stale: expected {expected.lower()}, got {revision}"
        )
    return revision


def run_acceptance_checks(manifest: Mapping[str, Any]) -> dict[int, dict[str, Any]]:
    commands = [manifest["required_repository_command"]] + [
        item["check"] for item in manifest["bounties"]
    ]
    cache: dict[str, dict[str, Any]] = {}
    by_issue: dict[int, dict[str, Any]] = {}
    check_env = acceptance_environment()
    for index, command in enumerate(commands):
        key = canonical_json(command)
        if key not in cache:
            run(command, env=check_env)
            cache[key] = {
                "command": command,
                "exit_code": 0,
            }
        if index > 0:
            by_issue[int(manifest["bounties"][index - 1]["issue"])] = cache[key]
    repository_check = cache[canonical_json(manifest["required_repository_command"])]
    for value in by_issue.values():
        value["repository_check"] = repository_check
    return by_issue


def build_candidate(
    manifest: Mapping[str, Any],
    bounty: Mapping[str, Any],
    revision: str,
    pull_request_url: str,
    check_run_urls: Sequence[str],
    check: Mapping[str, Any],
) -> dict[str, Any]:
    artifact_reference = (
        f"https://github.com/NSPG13/agent-bounties/commit/{revision}"
    )
    evidence = {
        "schema": EVIDENCE_SCHEMA,
        "classification": manifest["metrics_classification"],
        "notice_url": manifest["notice_url"],
        "issue_url": f"https://github.com/NSPG13/agent-bounties/issues/{bounty['issue']}",
        "repository": "NSPG13/agent-bounties",
        "commit_sha": revision,
        "pull_request_url": pull_request_url,
        "check_run_urls": list(check_run_urls),
        "artifact_digest": sha256_text(
            canonical_json(
                {
                    "revision": revision,
                    "issue": bounty["issue"],
                    "check": check,
                }
            )
        ),
        "operator_solver": manifest["operator_solver"],
        "metrics_credit": False,
        "reputation_credit": False,
        "leaderboard_credit": False,
        "organic_completion": False,
        "check": check,
    }
    response = {
        "classification": manifest["metrics_classification"],
        "notice_url": manifest["notice_url"],
        "issue": bounty["issue"],
        "contract": bounty["contract"],
        "revision": revision,
        "verdict": "passed",
        "artifact_digest": evidence["artifact_digest"],
    }
    return {
        "schema": CANDIDATE_SCHEMA,
        "issue": bounty["issue"],
        "contract": bounty["contract"],
        "bounty_id": bounty["bounty_id"],
        "revision": revision,
        "artifact_reference": artifact_reference,
        "submission_hash": sha256_text(artifact_reference),
        "evidence": evidence,
        "evidence_hash": sha256_text(canonical_json(evidence)),
        "response_hash": sha256_text(canonical_json(response)),
        "verdict": "passed",
    }


def validate_candidate_hashes(candidate: Mapping[str, Any]) -> None:
    issue = uint(candidate.get("issue"), "candidate issue")
    expected = EXACT_CANDIDATE_HASHES.get(issue)
    if expected is None:
        raise RecoveryError("candidate issue is outside the exact recovery set")
    for field, value in expected.items():
        if candidate.get(field) != value:
            raise RecoveryError(
                f"issue #{issue} {field} differs from the already-submitted evidence"
            )


def load_candidates(path: Path, manifest: Mapping[str, Any]) -> dict[int, dict[str, Any]]:
    summary = read_json(path / "manifest.json")
    if (
        not isinstance(summary, dict)
        or summary.get("schema") != CANDIDATE_SCHEMA
        or summary.get("classification") != manifest["metrics_classification"]
    ):
        raise RecoveryError("candidate manifest is invalid")
    candidates: dict[int, dict[str, Any]] = {}
    expected_by_issue = {
        int(item["issue"]): item for item in manifest["bounties"]
    }
    for entry in summary.get("candidates", []):
        candidate = read_json(path / str(entry["file"]))
        issue = uint(candidate.get("issue"), "candidate issue")
        if candidate.get("schema") != CANDIDATE_SCHEMA or issue in candidates:
            raise RecoveryError("candidate artifact is invalid or duplicated")
        validate_candidate_hashes(candidate)
        expected = expected_by_issue.get(issue)
        evidence = candidate.get("evidence")
        if (
            expected is None
            or candidate.get("contract") != expected["contract"]
            or candidate.get("bounty_id") != expected["bounty_id"]
            or candidate.get("revision") != EXACT_ACCEPTED_REVISION
            or summary.get("revision") != EXACT_ACCEPTED_REVISION
            or not isinstance(evidence, dict)
            or evidence.get("classification") != manifest["metrics_classification"]
            or evidence.get("notice_url") != manifest["notice_url"]
            or evidence.get("operator_solver") != manifest["operator_solver"]
            or evidence.get("metrics_credit") is not False
            or evidence.get("reputation_credit") is not False
            or evidence.get("leaderboard_credit") is not False
            or evidence.get("organic_completion") is not False
        ):
            raise RecoveryError("candidate artifact differs from the exact recovery scope")
        candidates[issue] = candidate
    if set(candidates) != {634, 635, 636, 637, 638}:
        raise RecoveryError("candidate artifact does not cover the exact recovery set")
    return candidates


def key_from_env(name: str) -> str:
    key = os.environ.get(name, "").strip()
    if not re.fullmatch(r"0x[0-9a-fA-F]{64}", key):
        raise RecoveryError(f"{name} is required and must be a private key")
    return key


def expire_stale_submissions(
    cast: Cast,
    manifest: Mapping[str, Any],
    states: dict[int, dict[str, Any]],
    key: str,
    now: int,
) -> dict[int, str]:
    transactions: dict[int, str] = {}
    for bounty in manifest["bounties"]:
        issue = int(bounty["issue"])
        state = states[issue]
        if (
            state["status"] != 3
            or state["verification_expires_at"] >= now
        ):
            continue
        transaction = cast.send(key, bounty["contract"], "expireSubmission()")
        refreshed = audit_bounty(cast, manifest, bounty)
        if (
            refreshed["status"] != 1
            or refreshed["solver"] != "0x0000000000000000000000000000000000000000"
            or refreshed["active_claim_bond"] != 0
            or refreshed["submission_hash"] != "0x" + "00" * 32
            or refreshed["evidence_hash"] != "0x" + "00" * 32
        ):
            raise RecoveryError(
                f"issue #{issue} expired submission did not reset to a clean claimable round"
            )
        states[issue] = refreshed
        transactions[issue] = transaction
    return transactions


def command_audit(args: argparse.Namespace) -> None:
    manifest = load_manifest(args.manifest)
    cast = Cast(args.cast, args.rpc_url)
    if cast.chain_id() != manifest["chain_id"]:
        raise RecoveryError("RPC is not Base mainnet")
    observations = [
        {"issue": bounty["issue"], **audit_bounty(cast, manifest, bounty)}
        for bounty in manifest["bounties"]
    ]
    report = {
        "schema": "agent-bounties/direct-recovery-audit-v1",
        "notice_url": manifest["notice_url"],
        "classification": manifest["metrics_classification"],
        "observations": observations,
    }
    if args.output:
        write_json(args.output, report)
    print(json.dumps(report, indent=2, sort_keys=True))


def command_prepare(args: argparse.Namespace) -> None:
    manifest = load_manifest(args.manifest)
    execution_revision = current_revision(args.revision)
    execution_pull_request_url = require_pull_request_url(args.pull_request_url)
    check_run_urls = [
        require_https(value, "check_run_url") for value in args.check_run_url
    ]
    if check_run_urls != [EXACT_CHECK_RUN_URL]:
        raise RecoveryError("check_run_url differs from the original submitted evidence")
    checks = run_acceptance_checks(manifest)
    key = key_from_env(args.keeper_key_env)
    cast = Cast(args.cast, args.rpc_url)
    if cast.chain_id() != manifest["chain_id"]:
        raise RecoveryError("RPC is not Base mainnet")
    keeper = cast.wallet_address(key)
    if keeper != manifest["operator_solver"]:
        raise RecoveryError("keeper key does not match the disclosed operator solver")
    states = {
        int(bounty["issue"]): audit_bounty(cast, manifest, bounty)
        for bounty in manifest["bounties"]
    }
    expiration_transactions = expire_stale_submissions(
        cast,
        manifest,
        states,
        key,
        int(time.time()),
    )
    claimable = [
        bounty
        for bounty in manifest["bounties"]
        if states[int(bounty["issue"])]["status"] == 1
    ]
    active = [
        bounty
        for bounty in manifest["bounties"]
        if states[int(bounty["issue"])]["status"] != 4
    ]
    required_bond = len(claimable) * manifest["claim_bond_per_bounty"]
    if cast.balance(manifest["settlement_token"], keeper) < required_bond:
        raise RecoveryError(
            f"operator solver requires at least {required_bond} USDC base units "
            "for the still-claimable bonds"
        )
    if active and cast.native_balance(keeper) < args.minimum_native_balance:
        raise RecoveryError("operator solver Base ETH reserve is below the recovery minimum")

    args.output.mkdir(parents=True, exist_ok=True)
    entries = []
    for bounty in manifest["bounties"]:
        issue = int(bounty["issue"])
        candidate = build_candidate(
            manifest,
            bounty,
            EXACT_ACCEPTED_REVISION,
            EXACT_ACCEPTED_PULL_REQUEST_URL,
            check_run_urls,
            checks[issue],
        )
        validate_candidate_hashes(candidate)
        state = states[issue]
        transactions: dict[str, str] = {}
        if issue in expiration_transactions:
            transactions["expire_submission"] = expiration_transactions[issue]
        if state["status"] == 1:
            transactions["approve"] = cast.send(
                key,
                manifest["settlement_token"],
                "approve(address,uint256)(bool)",
                bounty["contract"],
                str(manifest["claim_bond_per_bounty"]),
            )
            transactions["claim"] = cast.send(key, bounty["contract"], "claim()")
            state = audit_bounty(cast, manifest, bounty)
        if state["status"] == 2:
            if state["solver"] != keeper:
                raise RecoveryError(f"issue #{issue} is claimed by another solver")
            transactions["submit"] = cast.send(
                key,
                bounty["contract"],
                "submit(bytes32,bytes32)",
                candidate["submission_hash"],
                candidate["evidence_hash"],
            )
            state = audit_bounty(cast, manifest, bounty)
        if state["status"] not in {3, 4}:
            raise RecoveryError(f"issue #{issue} did not reach submitted or settled")
        if state["solver"] != keeper:
            raise RecoveryError(f"issue #{issue} solver differs from disclosed operator")
        if (
            state["submission_hash"] != candidate["submission_hash"]
            or state["evidence_hash"] != candidate["evidence_hash"]
        ):
            raise RecoveryError(f"issue #{issue} submitted hashes differ from evidence")
        candidate["transactions"] = transactions
        candidate["round"] = state["round"]
        candidate["verification_expires_at"] = state["verification_expires_at"]
        name = f"candidate-{issue}.json"
        write_json(args.output / name, candidate)
        entries.append({"issue": issue, "file": name})
    write_json(
        args.output / "manifest.json",
        {
            "schema": CANDIDATE_SCHEMA,
            "classification": manifest["metrics_classification"],
            "notice_url": manifest["notice_url"],
            "revision": EXACT_ACCEPTED_REVISION,
            "execution_revision": execution_revision,
            "execution_pull_request_url": execution_pull_request_url,
            "candidates": entries,
        },
    )


def command_sign(args: argparse.Namespace) -> None:
    manifest = load_manifest(args.manifest)
    candidates = load_candidates(args.candidates, manifest)
    execution_revision = current_revision(args.revision)
    checks = run_acceptance_checks(manifest)
    expected = address(args.expected_signer, "expected signer")
    if expected not in manifest["verifiers"]:
        raise RecoveryError("expected signer is outside the immutable verifier set")
    key = key_from_env(args.private_key_env)
    cast = Cast(args.cast, args.rpc_url)
    if cast.chain_id() != manifest["chain_id"]:
        raise RecoveryError("RPC is not Base mainnet")
    if cast.wallet_address(key) != expected:
        raise RecoveryError("signer key does not match expected signer")
    args.output.mkdir(parents=True, exist_ok=True)
    entries = []
    for bounty in manifest["bounties"]:
        issue = int(bounty["issue"])
        candidate = candidates[issue]
        if candidate.get("revision") != EXACT_ACCEPTED_REVISION:
            raise RecoveryError("candidate accepted-work revision is stale")
        if candidate.get("evidence", {}).get("check") != checks[issue]:
            raise RecoveryError(f"issue #{issue} independent check evidence drifted")
        state = audit_bounty(cast, manifest, bounty)
        if state["status"] not in {3, 4}:
            raise RecoveryError(f"issue #{issue} is not submitted or settled")
        if (
            state["submission_hash"] != candidate["submission_hash"]
            or state["evidence_hash"] != candidate["evidence_hash"]
        ):
            raise RecoveryError(f"issue #{issue} on-chain evidence drifted")
        now = int(time.time())
        if state["status"] == 4:
            deadline = now + 3_600
        else:
            deadline = min(now + 3_600, state["verification_expires_at"])
            if deadline <= now + 120:
                raise RecoveryError(f"issue #{issue} verification deadline is too close")
        digest = bytes32(
            cast.call(
                bounty["contract"],
                "attestationDigest(address,bool,bytes32,uint256)(bytes32)",
                expected,
                "true",
                candidate["response_hash"],
                str(deadline),
            ),
            "attestation digest",
        )
        attestation = {
            "schema": ATTESTATION_SCHEMA,
            "classification": manifest["metrics_classification"],
            "issue": issue,
            "contract": bounty["contract"],
            "revision": execution_revision,
            "accepted_work_revision": EXACT_ACCEPTED_REVISION,
            "verifier": expected,
            "passed": True,
            "response_hash": candidate["response_hash"],
            "deadline": deadline,
            "signature": cast.sign_digest(key, digest),
        }
        name = f"attestation-{issue}.json"
        write_json(args.output / name, attestation)
        entries.append({"issue": issue, "file": name})
    write_json(
        args.output / "manifest.json",
        {
            "schema": ATTESTATION_SCHEMA,
            "classification": manifest["metrics_classification"],
            "revision": execution_revision,
            "accepted_work_revision": EXACT_ACCEPTED_REVISION,
            "signer": expected,
            "attestations": entries,
        },
    )


def load_attestations(
    paths: Sequence[Path],
    manifest: Mapping[str, Any],
) -> dict[int, list[dict[str, Any]]]:
    by_issue: dict[int, list[dict[str, Any]]] = {
        issue: [] for issue in (634, 635, 636, 637, 638)
    }
    signers = set()
    for path in paths:
        summary = read_json(path / "manifest.json")
        if (
            summary.get("schema") != ATTESTATION_SCHEMA
            or summary.get("classification") != manifest["metrics_classification"]
            or summary.get("accepted_work_revision") != EXACT_ACCEPTED_REVISION
        ):
            raise RecoveryError("attestation manifest is invalid")
        signer = address(summary.get("signer"), "attestation signer")
        signers.add(signer)
        for entry in summary.get("attestations", []):
            item = read_json(path / str(entry["file"]))
            issue = uint(item.get("issue"), "attestation issue")
            if (
                issue not in by_issue
                or item.get("schema") != ATTESTATION_SCHEMA
                or item.get("classification") != manifest["metrics_classification"]
                or address(item.get("verifier"), "verifier") != signer
                or item.get("revision") != summary.get("revision")
                or item.get("accepted_work_revision") != EXACT_ACCEPTED_REVISION
            ):
                raise RecoveryError("attestation artifact is invalid")
            by_issue[issue].append(item)
    if signers != set(manifest["verifiers"]):
        raise RecoveryError("attestations do not contain the exact verifier set")
    if any(len(items) != manifest["threshold"] for items in by_issue.values()):
        raise RecoveryError("every recovery candidate requires the exact threshold")
    return by_issue


def command_relay(args: argparse.Namespace) -> None:
    manifest = load_manifest(args.manifest)
    candidates = load_candidates(args.candidates, manifest)
    attestations = load_attestations(args.attestations, manifest)
    execution_revision = current_revision(args.revision)
    checks = run_acceptance_checks(manifest)
    key = key_from_env(args.keeper_key_env)
    cast = Cast(args.cast, args.rpc_url)
    if cast.chain_id() != manifest["chain_id"]:
        raise RecoveryError("RPC is not Base mainnet")
    keeper = cast.wallet_address(key)
    if keeper != manifest["operator_solver"]:
        raise RecoveryError("keeper key does not match the disclosed operator solver")
    token = manifest["settlement_token"]
    states = {
        int(bounty["issue"]): audit_bounty(cast, manifest, bounty)
        for bounty in manifest["bounties"]
    }
    if any(state["status"] not in {3, 4} for state in states.values()):
        raise RecoveryError("every recovery contract must be submitted or already settled")
    already_settled = sum(state["status"] == 4 for state in states.values())
    keeper_before = cast.balance(token, keeper)
    expected_keeper_before = already_settled * 2_000_000
    if keeper_before != expected_keeper_before:
        raise RecoveryError(
            "operator balance does not match resumable partial-settlement accounting"
        )
    verifier_before = {
        verifier: cast.balance(token, verifier) for verifier in manifest["verifiers"]
    }
    settlements = []
    for bounty in manifest["bounties"]:
        issue = int(bounty["issue"])
        candidate = candidates[issue]
        if candidate.get("revision") != EXACT_ACCEPTED_REVISION:
            raise RecoveryError("candidate accepted-work revision is stale")
        if candidate.get("evidence", {}).get("check") != checks[issue]:
            raise RecoveryError(f"issue #{issue} relay check evidence drifted")
        state = states[issue]
        if state["status"] == 4:
            settlements.append(
                {
                    "issue": issue,
                    "contract": bounty["contract"],
                    "tx": None,
                    "state": "already_settled_before_relay",
                }
            )
            continue
        if state["status"] != 3:
            raise RecoveryError(f"issue #{issue} is not submitted")
        items = sorted(attestations[issue], key=lambda item: item["verifier"])
        first = items[0]
        for item in items:
            if (
                item["revision"] != execution_revision
                or item.get("accepted_work_revision") != EXACT_ACCEPTED_REVISION
                or item["contract"] != bounty["contract"]
                or item["passed"] is not True
                or item["response_hash"] != candidate["response_hash"]
                or item["response_hash"] != first["response_hash"]
            ):
                raise RecoveryError(f"issue #{issue} attestations disagree")
        tuple_values = ",".join(
            f"({item['verifier']},true,{item['response_hash']},{item['deadline']},{item['signature']})"
            for item in items
        )
        transaction = cast.send(
            key,
            bounty["contract"],
            "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])",
            f"[{tuple_values}]",
        )
        after = audit_bounty(cast, manifest, bounty)
        if after["status"] != 4:
            raise RecoveryError(f"issue #{issue} settlement did not finalize")
        settlements.append(
            {
                "issue": issue,
                "contract": bounty["contract"],
                "tx": transaction,
                "state": "settled_by_relay",
            }
        )

    keeper_after_settlement = cast.balance(token, keeper)
    newly_settled = len(manifest["bounties"]) - already_settled
    expected_solver_increase = newly_settled * 2_000_000
    if keeper_after_settlement - keeper_before != expected_solver_increase:
        raise RecoveryError("solver recovery receipts differ from exact per-bounty accounting")
    if keeper_after_settlement != manifest["exact_return_amount"]:
        raise RecoveryError("operator balance does not equal the exact 10 USDC return")
    for verifier in manifest["verifiers"]:
        increase = cast.balance(token, verifier) - verifier_before[verifier]
        if increase != newly_settled * 5_000:
            raise RecoveryError("verifier recovery receipt differs from exact per-bounty accounting")

    recipient = manifest["return_recipient"]
    recipient_before = cast.balance(token, recipient)
    transfer_tx = cast.send(
        key,
        token,
        "transfer(address,uint256)(bool)",
        recipient,
        str(manifest["exact_return_amount"]),
    )
    recipient_after = cast.balance(token, recipient)
    if recipient_after - recipient_before != manifest["exact_return_amount"]:
        raise RecoveryError("return transfer did not increase recipient by exactly 10 USDC")
    evidence = {
        "schema": EVIDENCE_SCHEMA,
        "classification": manifest["metrics_classification"],
        "notice_url": manifest["notice_url"],
        "revision": execution_revision,
        "accepted_work_revision": EXACT_ACCEPTED_REVISION,
        "operator_solver": keeper,
        "return_recipient": recipient,
        "return_amount": manifest["exact_return_amount"],
        "settlements": settlements,
        "return_transfer_tx": transfer_tx,
        "metrics_credit": False,
        "reputation_credit": False,
        "leaderboard_credit": False,
        "organic_completion": False,
        "evidence_boundary": (
            "These are disclosed operator recovery settlements, not organic paid loops, "
            "external solver activity, adoption, retention, revenue, reputation, or leaderboard evidence."
        ),
    }
    write_json(args.output, evidence)
    print(json.dumps(evidence, indent=2, sort_keys=True))


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    root.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", DEFAULT_RPC))
    root.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    commands = root.add_subparsers(dest="command", required=True)

    audit = commands.add_parser("audit")
    audit.add_argument("--output", type=Path)
    audit.set_defaults(handler=command_audit)

    prepare = commands.add_parser("prepare")
    prepare.add_argument("--revision", required=True)
    prepare.add_argument("--pull-request-url", required=True)
    prepare.add_argument("--check-run-url", action="append", default=[])
    prepare.add_argument("--output", type=Path, required=True)
    prepare.add_argument("--keeper-key-env", default="BASE_KEEPER_PRIVATE_KEY")
    prepare.add_argument("--minimum-native-balance", type=int, default=100_000_000_000_000)
    prepare.set_defaults(handler=command_prepare)

    sign = commands.add_parser("sign")
    sign.add_argument("--revision", required=True)
    sign.add_argument("--candidates", type=Path, required=True)
    sign.add_argument("--output", type=Path, required=True)
    sign.add_argument("--private-key-env", required=True)
    sign.add_argument("--expected-signer", required=True)
    sign.set_defaults(handler=command_sign)

    relay = commands.add_parser("relay")
    relay.add_argument("--revision", required=True)
    relay.add_argument("--candidates", type=Path, required=True)
    relay.add_argument("--attestations", type=Path, action="append", required=True)
    relay.add_argument("--output", type=Path, required=True)
    relay.add_argument("--keeper-key-env", default="BASE_KEEPER_PRIVATE_KEY")
    relay.set_defaults(handler=command_relay)
    return root


def main() -> int:
    try:
        args = parser().parse_args()
        args.handler(args)
    except (RecoveryError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"direct recovery #689 failed: {error}", file=os.sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
