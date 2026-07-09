#!/usr/bin/env python3
"""Deterministic read-only attestation for deployments/base-mainnet.json.

Compares on-chain Base mainnet escrow state against the checked-in manifest.
Does not sign, broadcast, fund, release, or mutate any state.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

from Crypto.Hash import keccak

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "deployments" / "base-mainnet.json"

SELECTORS = {
    "owner()": keccak.new(digest_bits=256, data=b"owner()").hexdigest()[:8],
    "settlementSigner()": keccak.new(digest_bits=256, data=b"settlementSigner()").hexdigest()[:8],
    "paused()": keccak.new(digest_bits=256, data=b"paused()").hexdigest()[:8],
    "nextEscrowId()": keccak.new(digest_bits=256, data=b"nextEscrowId()").hexdigest()[:8],
}

BOUNDARY = (
    "Passing deployment attestation does not prove hosted API or indexer health, "
    "and does not prove a bounty is funded, claimable, accepted, paid, or settled."
)


@dataclass
class CheckResult:
    name: str
    expected: Any
    observed: Any
    passed: bool
    note: str = ""


def redact_rpc_url(url: str) -> str:
    parsed = urlparse(url)
    if not parsed.username and not parsed.password:
        return url
    host = parsed.hostname or ""
    if parsed.port:
        host = f"{host}:{parsed.port}"
    scheme = parsed.scheme or "https"
    return f"{scheme}://***REDACTED***@{host}{parsed.path or ''}"


def normalize_address(value: str | None) -> str:
    if not value:
        return ""
    value = value.lower()
    if not value.startswith("0x"):
        value = f"0x{value}"
    return value


def is_address(value: str) -> bool:
    return bool(re.fullmatch(r"0x[0-9a-fA-F]{40}", value))


def keccak256_hex(data: bytes) -> str:
    digest = keccak.new(digest_bits=256, data=data).hexdigest()
    return f"0x{digest}"


def decode_address(result: str) -> str:
    if not result or result == "0x":
        return ""
    return normalize_address("0x" + result[-40:])


def decode_bool(result: str) -> bool:
    if not result or result == "0x":
        return False
    return int(result, 16) != 0


def decode_uint256(result: str) -> int:
    if not result or result == "0x":
        return 0
    return int(result, 16)


class RpcClient:
    def __init__(self, url: str, mock_fixture: dict[str, Any] | None = None) -> None:
        self.url = url
        self.mock_fixture = mock_fixture
        self._request_id = 0

    def call(self, method: str, params: list[Any]) -> Any:
        self._request_id += 1
        key = f"{method}:{json.dumps(params, separators=(',', ':'))}"
        if self.mock_fixture is not None:
            responses = self.mock_fixture.get("responses", {})
            if key not in responses:
                raise RuntimeError(f"mock fixture missing response for {key}")
            payload = responses[key]
            if "error" in payload:
                raise RuntimeError(payload["error"]["message"])
            return payload.get("result")

        body = json.dumps(
            {"jsonrpc": "2.0", "id": self._request_id, "method": method, "params": params}
        ).encode()
        request = urllib.request.Request(
            self.url,
            data=body,
            headers={
                "content-type": "application/json",
                "user-agent": "agent-bounties-base-deployment-attest/1.0",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.loads(response.read().decode())
        except urllib.error.URLError as exc:
            raise RuntimeError(f"RPC request failed: {exc}") from exc
        if "error" in payload:
            raise RuntimeError(payload["error"].get("message", "unknown RPC error"))
        return payload.get("result")

    def eth_call(self, contract: str, selector_name: str) -> str:
        selector = SELECTORS[selector_name]
        return self.call("eth_call", [{"to": contract, "data": f"0x{selector}"}, "latest"])


def load_manifest(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def add_check(checks: list[CheckResult], name: str, expected: Any, observed: Any, passed: bool, note: str = "") -> None:
    checks.append(CheckResult(name=name, expected=expected, observed=observed, passed=passed, note=note))


def run_attestation(
    manifest: dict[str, Any],
    rpc_url: str,
    mock_fixture: dict[str, Any] | None = None,
) -> dict[str, Any]:
    escrow = manifest["escrow"]
    contract = escrow["contract"]
    checks: list[CheckResult] = []
    client = RpcClient(rpc_url, mock_fixture=mock_fixture)

    chain_id_hex = client.call("eth_chainId", [])
    chain_id = int(chain_id_hex, 16)
    add_check(
        checks,
        "chain_id",
        manifest["chain_id"],
        chain_id,
        chain_id == manifest["chain_id"],
    )

    receipt = client.call("eth_getTransactionReceipt", [escrow["deployment_transaction"]])
    if receipt is None:
        add_check(checks, "deployment_receipt_present", True, False, False)
    else:
        add_check(checks, "deployment_receipt_present", True, True, True)
        status = receipt.get("status")
        add_check(checks, "deployment_receipt_status", "0x1", status, status == "0x1")
        block_number = int(receipt.get("blockNumber", "0x0"), 16)
        add_check(
            checks,
            "deployment_block",
            escrow["deployment_block"],
            block_number,
            block_number == escrow["deployment_block"],
        )
        contract_address = normalize_address(receipt.get("contractAddress"))
        add_check(
            checks,
            "deployment_contract_address",
            normalize_address(contract),
            contract_address,
            contract_address == normalize_address(contract),
        )

    runtime_code = client.call("eth_getCode", [contract, "latest"]) or "0x"
    bytecode = bytes.fromhex(runtime_code[2:]) if runtime_code not in ("0x", "") else b""
    add_check(checks, "runtime_bytecode_nonempty", "non-empty", len(bytecode), len(bytecode) > 0)
    observed_hash = keccak256_hex(bytecode) if bytecode else "0x"
    add_check(
        checks,
        "runtime_code_hash",
        escrow["runtime_code_hash"].lower(),
        observed_hash.lower(),
        observed_hash.lower() == escrow["runtime_code_hash"].lower(),
    )

    owner = decode_address(client.eth_call(contract, "owner()"))
    add_check(
        checks,
        "owner()",
        normalize_address(escrow["owner_at_deployment"]),
        owner,
        owner == normalize_address(escrow["owner_at_deployment"]),
    )

    signer = decode_address(client.eth_call(contract, "settlementSigner()"))
    add_check(
        checks,
        "settlementSigner()",
        normalize_address(escrow["settlement_signer_at_deployment"]),
        signer,
        signer == normalize_address(escrow["settlement_signer_at_deployment"]),
    )

    paused = decode_bool(client.eth_call(contract, "paused()"))
    add_check(
        checks,
        "paused()",
        False,
        paused,
        paused is False,
        note="paused=true blocks new funding readiness but is not proof of compromise",
    )

    next_escrow_id = decode_uint256(client.eth_call(contract, "nextEscrowId()"))
    add_check(
        checks,
        "nextEscrowId()",
        ">= 1",
        next_escrow_id,
        next_escrow_id >= 1,
        note="observed counter only; manifest does not pin an expected escrow count",
    )

    native_usdc = manifest["native_usdc"]
    add_check(
        checks,
        "native_usdc_format",
        "valid address",
        native_usdc,
        is_address(native_usdc),
    )

    verification = escrow.get("verification", {})
    overall_passed = all(check.passed for check in checks)
    return {
        "schema_version": 1,
        "network": manifest.get("network", "base-mainnet"),
        "manifest_path": "deployments/base-mainnet.json",
        "observation_timestamp_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "rpc_url_redacted": redact_rpc_url(rpc_url),
        "escrow_contract": contract,
        "checks": [
            {
                "name": check.name,
                "expected": check.expected,
                "observed": check.observed,
                "pass": check.passed,
                **({"note": check.note} if check.note else {}),
            }
            for check in checks
        ],
        "source_verification_reference": {
            "sourcify_url": verification.get("sourcify_url"),
            "blockscout_url": verification.get("blockscout_url"),
            "note": "Explorer records are reference metadata only; RPC checks are authoritative.",
        },
        "overall_result": "pass" if overall_passed else "fail",
        "boundary": BOUNDARY,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=DEFAULT_MANIFEST,
        help="Path to deployment manifest JSON",
    )
    parser.add_argument(
        "--rpc-url",
        default=None,
        help="Base JSON-RPC URL (defaults to manifest rpc_url)",
    )
    parser.add_argument(
        "--mock-fixture",
        type=Path,
        default=None,
        help="Offline mock-RPC fixture JSON for deterministic tests",
    )
    parser.add_argument(
        "--live",
        action="store_true",
        help="Use public/live RPC from manifest (opt-in production smoke)",
    )
    parser.add_argument("--json-out", type=Path, default=None, help="Write JSON report to file")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = load_manifest(args.manifest)

    mock_fixture: dict[str, Any] | None = None
    if args.mock_fixture is not None:
        mock_fixture = json.loads(args.mock_fixture.read_text(encoding="utf-8"))
        rpc_url = mock_fixture.get("rpc_url", manifest["rpc_url"])
    else:
        rpc_url = args.rpc_url or manifest["rpc_url"]
        if not args.live and args.rpc_url is None:
            print(
                "Refusing live RPC without --live or --mock-fixture. "
                "Use --live for production smoke or --mock-fixture for offline replay.",
                file=sys.stderr,
            )
            return 2

    try:
        report = run_attestation(manifest, rpc_url, mock_fixture=mock_fixture)
    except RuntimeError as exc:
        print(json.dumps({"overall_result": "fail", "error": str(exc), "boundary": BOUNDARY}, indent=2))
        return 1

    output = json.dumps(report, indent=2)
    print(output)
    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(output + "\n", encoding="utf-8")

    return 0 if report["overall_result"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
