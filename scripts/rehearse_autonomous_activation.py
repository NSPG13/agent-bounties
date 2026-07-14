#!/usr/bin/env python3
"""Replay an unsigned autonomous-v1 activation bundle on a Base mainnet fork."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from typing import Any
from urllib.error import URLError
from urllib.request import Request, urlopen

from Crypto.Hash import keccak


MAX_ACTIVATION_FUNDING_MINOR = 8_040_000


def rpc(url: str, method: str, params: list[Any], request_id: int = 1) -> Any:
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(url, data=payload, headers={"content-type": "application/json"})
    try:
        with urlopen(request, timeout=30) as response:
            body = json.load(response)
    except URLError as error:
        raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
    if body.get("error"):
        raise RuntimeError(f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}")
    return body.get("result")


def selector(signature: str) -> str:
    digest = keccak.new(digest_bits=256)
    digest.update(signature.encode("ascii"))
    return digest.hexdigest()[:8]


def address_word(address: str) -> str:
    raw = address.removeprefix("0x")
    if len(raw) != 40 or any(character not in "0123456789abcdefABCDEF" for character in raw):
        raise ValueError(f"invalid EVM address: {address}")
    return raw.lower().rjust(64, "0")


def uint256_argument(calldata: str, index: int) -> int:
    raw = calldata.removeprefix("0x")
    start = 8 + index * 64
    end = start + 64
    if index < 0 or len(raw) < end:
        raise ValueError(f"calldata is missing uint256 argument {index}")
    return int(raw[start:end], 16)


def call(url: str, contract: str, signature: str, arguments: str = "") -> str:
    return rpc(
        url,
        "eth_call",
        [{"to": contract, "data": f"0x{selector(signature)}{arguments}"}, "latest"],
    )


def uint_result(value: str) -> int:
    return int(value, 16)


def address_result(value: str) -> str:
    return f"0x{value.removeprefix('0x')[-40:]}".lower()


def wait_receipt(url: str, transaction_hash: str, timeout_seconds: float = 45) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        receipt = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
        if receipt:
            if receipt.get("status") != "0x1":
                raise RuntimeError(f"fork transaction reverted: {transaction_hash}")
            return receipt
        time.sleep(0.1)
    raise TimeoutError(f"timed out waiting for fork transaction {transaction_hash}")


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def find_anvil(repo: Path, configured: str | None) -> str:
    candidates = [configured, shutil.which("anvil")]
    if os.name == "nt":
        candidates.append(str(repo / ".tools" / "foundry" / "anvil.exe"))
    else:
        candidates.append(str(repo / ".tools" / "foundry" / "anvil"))
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return candidate
    raise FileNotFoundError("anvil was not found; install Foundry or pass --anvil")


def wait_ready(url: str, process: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"anvil exited before readiness with code {process.returncode}")
        try:
            if uint_result(rpc(url, "eth_chainId", [])) == 8453:
                return
        except (RuntimeError, URLError):
            pass
        time.sleep(0.25)
    raise TimeoutError("anvil Base fork did not become ready")


def validate_bundle(bundle: dict[str, Any]) -> None:
    if bundle.get("schema_version") != "agent-bounties/autonomous-activation-bundle-v1":
        raise ValueError("unsupported activation bundle schema")
    if bundle.get("network") != "base-mainnet" or bundle.get("chain_id") != 8453:
        raise ValueError("activation bundle is not for Base mainnet")
    deployment = bundle["deployment"]
    if deployment.get("to") is not None or deployment.get("value_wei") != 0:
        raise ValueError("factory deployment must be a zero-value contract creation")
    calls = bundle["creation_batch"]["wallet_calls"]
    if len(calls) != len(bundle["bounties"]) + 1:
        raise ValueError("activation batch must contain one approval and one call per bounty")
    creations = bundle["creation_batch"]["creations"]
    if len(creations) != len(bundle["bounties"]):
        raise ValueError("activation batch bounty and creation counts differ")
    total_initial_funding = int(bundle["creation_batch"]["total_initial_funding"])
    observed_total = 0
    for creation in creations:
        initial_funding = int(creation["eip3009_authorization"]["message"]["value"])
        target = uint256_argument(creation["create_bounty"]["data"], 0) + uint256_argument(
            creation["create_bounty"]["data"], 1
        )
        if initial_funding <= 0 or initial_funding > target:
            raise ValueError("activation bounty initial funding exceeds its target")
        observed_total += initial_funding
    if (
        total_initial_funding <= 0
        or total_initial_funding > MAX_ACTIVATION_FUNDING_MINOR
        or observed_total != total_initial_funding
    ):
        raise ValueError("activation batch initial funding total is inconsistent")


def run_portable_creation_plan(
    repo: Path,
    temp: Path,
    local_url: str,
    bundle: dict[str, Any],
    deploy_hash: str | None,
    deploy_receipt: dict[str, Any] | None,
) -> dict[str, Any]:
    deployment = bundle["deployment"]
    active_manifest = json.loads((repo / "deployments/base-mainnet.json").read_text(encoding="utf-8"))
    if deploy_hash is not None and deploy_receipt is not None:
        active_manifest["status"] = "active"
        active_manifest["factory"].update(
            {
                "contract": deployment["expected_factory"],
                "implementation": deployment["expected_implementation"],
                "deployment_transaction": deploy_hash,
                "deployment_block": uint_result(deploy_receipt["blockNumber"]),
                "deployer": deployment["from"],
                "runtime_code_hash": deployment["factory_runtime_code_hash"],
                "implementation_runtime_code_hash": deployment[
                    "implementation_runtime_code_hash"
                ],
            }
        )
    manifest_path = temp / "active-base-mainnet.json"
    first_bounty = bundle["bounties"][0]
    plan_path = temp / f"portable-bounty-{first_bounty['issue']}.json"
    manifest_path.write_text(
        json.dumps(active_manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    command = [
        "cargo",
        "run",
        "--quiet",
        "-p",
        "cli",
        "--",
        "autonomous-bounty-plan",
        "--terms-file",
        first_bounty["document"],
        "--deployment-file",
        str(manifest_path),
        "--rpc-url",
        local_url,
        "--output",
        str(plan_path),
    ]
    completed = subprocess.run(
        command,
        cwd=repo,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"portable creation planner failed on Base fork: {detail}")
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    expected_bounty = first_bounty
    expected_creation = next(
        item
        for item in bundle["creation_batch"]["creations"]
        if item["bounty_id"].lower() == expected_bounty["bounty_id"].lower()
    )
    if (
        plan.get("schema_version")
        != "agent-bounties/autonomous-portable-creation-plan-v1"
        or plan["safe_chain_observation"]["factory_contract"].lower()
        != deployment["expected_factory"].lower()
        or plan["safe_chain_observation"]["factory_runtime_code_hash"].lower()
        != deployment["factory_runtime_code_hash"].lower()
        or plan["safe_chain_observation"]["implementation_runtime_code_hash"].lower()
        != deployment["implementation_runtime_code_hash"].lower()
        or plan["creation_plan"]["bounty_id"].lower()
        != expected_bounty["bounty_id"].lower()
        or plan["creation_plan"]["predicted_bounty_contract"].lower()
        != expected_bounty["predicted_bounty_contract"].lower()
        or plan["creation_plan"]["create_bounty"]["data"].lower()
        != expected_creation["create_bounty"]["data"].lower()
        or len(plan["wallet_request"]["params"][0]["calls"]) != 2
    ):
        raise RuntimeError("portable creation planner output drifted from activation bundle")
    return {
        "schema_version": plan["schema_version"],
        "issue": expected_bounty["issue"],
        "safe_block_number": plan["safe_chain_observation"]["safe_block_number"],
        "safe_block_hash": plan["safe_chain_observation"]["safe_block_hash"],
        "bounty_id": plan["creation_plan"]["bounty_id"],
        "predicted_bounty_contract": plan["creation_plan"][
            "predicted_bounty_contract"
        ],
        "wallet_call_count": len(plan["wallet_request"]["params"][0]["calls"]),
        "evidence_boundary": "Unsigned planner output and fork state are rehearsal evidence only.",
    }


def deploy_or_verify_module(
    local_url: str,
    creator: str,
    bundle_path: Path,
) -> dict[str, Any]:
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    deployment = bundle.get("deployment", {})
    if (
        bundle.get("schema_version")
        != "agent-bounties/canonical-child-verifier-deployment-v1"
        or bundle.get("network") != "base-mainnet"
        or bundle.get("chain_id") != 8453
        or deployment.get("from", "").lower() != creator.lower()
        or deployment.get("to") is not None
        or deployment.get("value_wei") != 0
    ):
        raise ValueError("canonical child verifier deployment bundle is invalid")

    expected = deployment["expected_contract"].lower()
    code = rpc(local_url, "eth_getCode", [expected, "latest"]).lower()
    transaction_hash = None
    gas_used = 0
    if code == "0x":
        nonce = uint_result(rpc(local_url, "eth_getTransactionCount", [creator, "latest"]))
        if nonce != deployment["deployer_nonce"]:
            raise RuntimeError(
                f"verifier deployer nonce drift: bundle={deployment['deployer_nonce']} fork={nonce}"
            )
        transaction_hash = rpc(
            local_url,
            "eth_sendTransaction",
            [{"from": creator, "data": deployment["data"], "value": "0x0"}],
        )
        receipt = wait_receipt(local_url, transaction_hash)
        if receipt.get("contractAddress", "").lower() != expected:
            raise RuntimeError("canonical child verifier deployed to an unexpected address")
        gas_used = uint_result(receipt["gasUsed"])
        code = rpc(local_url, "eth_getCode", [expected, "latest"]).lower()

    if code != deployment["expected_runtime_code"].lower():
        raise RuntimeError("canonical child verifier runtime bytecode mismatch")
    if address_result(call(local_url, expected, "canonicalFactory()")) != bundle[
        "canonical_factory"
    ].lower():
        raise RuntimeError("canonical child verifier factory getter mismatch")
    if address_result(call(local_url, expected, "settlementToken()")) != bundle[
        "settlement_token"
    ].lower():
        raise RuntimeError("canonical child verifier token getter mismatch")
    if call(local_url, expected, "ACCEPTANCE_CRITERIA_HASH()").lower() != bundle[
        "acceptance_criteria_hash"
    ].lower():
        raise RuntimeError("canonical child verifier criteria getter mismatch")
    return {
        "contract": expected,
        "transaction": transaction_hash,
        "gas_used": gas_used,
        "runtime_code_hash": deployment["runtime_code_hash"],
    }


def rehearse(
    repo: Path,
    bundle_path: Path,
    fork_url: str,
    anvil_path: str | None,
    expect_existing_factory: bool = False,
    verifier_deployment_path: Path | None = None,
    fork_block_number: int | None = None,
) -> dict[str, Any]:
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    validate_bundle(bundle)
    deployment = bundle["deployment"]
    creator = deployment["from"]
    port = free_port()
    local_url = f"http://127.0.0.1:{port}"
    anvil = find_anvil(repo, anvil_path)
    with tempfile.TemporaryDirectory(prefix="agent-bounties-activation-") as temp_name:
        temp = Path(temp_name)
        stdout_path = temp / "anvil.stdout.log"
        stderr_path = temp / "anvil.stderr.log"
        creationflags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            anvil_command = [
                anvil,
                "--fork-url",
                fork_url,
                "--port",
                str(port),
                "--chain-id",
                "8453",
                "--silent",
            ]
            if fork_block_number is not None:
                anvil_command.extend(["--fork-block-number", str(fork_block_number)])
            process = subprocess.Popen(
                anvil_command,
                cwd=repo,
                stdout=stdout,
                stderr=stderr,
                creationflags=creationflags,
            )
            try:
                wait_ready(local_url, process)
                fork_block = uint_result(rpc(local_url, "eth_blockNumber", []))
                nonce = uint_result(rpc(local_url, "eth_getTransactionCount", [creator, "latest"]))
                factory_code = rpc(
                    local_url, "eth_getCode", [deployment["expected_factory"], "latest"]
                )
                factory_exists = factory_code != "0x"
                if expect_existing_factory and not factory_exists:
                    raise RuntimeError("canonical factory is missing from the Base fork")
                if not expect_existing_factory and factory_exists:
                    raise RuntimeError("predicted factory address already contains code")
                if not factory_exists and nonce != deployment["deployer_nonce"]:
                    raise RuntimeError(
                        f"deployer nonce drift: bundle={deployment['deployer_nonce']} fork={nonce}"
                    )
                wallet_eth = uint_result(rpc(local_url, "eth_getBalance", [creator, "latest"]))
                wallet_usdc = uint_result(
                    call(
                        local_url,
                        deployment["settlement_token"],
                        "balanceOf(address)",
                        address_word(creator),
                    )
                )
                total_funding = int(bundle["creation_batch"]["total_initial_funding"])
                bounty_count = len(bundle["bounties"])
                if bounty_count == 0:
                    raise RuntimeError("activation manifest has no bounties")
                if wallet_usdc < total_funding:
                    raise RuntimeError(
                        f"forked wallet has {wallet_usdc} USDC minor units; {total_funding} required"
                    )

                rpc(local_url, "anvil_impersonateAccount", [creator])
                rpc(local_url, "anvil_setBalance", [creator, "0xde0b6b3a7640000"])
                deploy_hash = None
                deploy_receipt = None
                deployment_gas_used = 0
                observed_factory = deployment["expected_factory"].lower()
                if not factory_exists:
                    deploy_hash = rpc(
                        local_url,
                        "eth_sendTransaction",
                        [{"from": creator, "data": deployment["data"], "value": "0x0"}],
                    )
                    deploy_receipt = wait_receipt(local_url, deploy_hash)
                    observed_factory = deploy_receipt.get("contractAddress", "").lower()
                    if observed_factory != deployment["expected_factory"].lower():
                        raise RuntimeError(
                            f"factory address mismatch: expected {deployment['expected_factory']} observed {observed_factory}"
                        )
                    deployment_gas_used = uint_result(deploy_receipt["gasUsed"])
                observed_implementation = address_result(
                    call(local_url, observed_factory, "implementation()")
                )
                if observed_implementation != deployment["expected_implementation"].lower():
                    raise RuntimeError("factory implementation address mismatch")
                observed_token = address_result(call(local_url, observed_factory, "settlementToken()"))
                if observed_token != deployment["settlement_token"].lower():
                    raise RuntimeError("factory settlement token mismatch")
                verifier_summary = None
                if verifier_deployment_path is not None:
                    verifier_summary = deploy_or_verify_module(
                        local_url, creator, verifier_deployment_path
                    )
                if factory_exists:
                    # Anvil cannot produce a reliable historical eth_getProof for every
                    # pre-existing Base account after local fork mutations. The active
                    # deployment's safe-block proof remains a separate production gate.
                    first_bounty = bundle["bounties"][0]
                    first_creation = bundle["creation_batch"]["creations"][0]
                    portable_plan = {
                        "schema_version": "agent-bounties/autonomous-activation-bundle-v1",
                        "issue": first_bounty["issue"],
                        "bounty_id": first_creation["bounty_id"],
                        "predicted_bounty_contract": first_creation[
                            "predicted_bounty_contract"
                        ],
                        "wallet_call_count": len(first_creation["wallet_calls"]),
                        "evidence_boundary": "The checked-in Rust-generated batch is replayed below. Production safe-block factory attestation is verified separately because Anvil historical account proofs are incomplete after local fork mutation.",
                    }
                else:
                    # Anvil models safe/finalized tags behind latest. Advance the local fork so
                    # the rehearsed deployment is visible through the production proof boundary.
                    rpc(local_url, "anvil_mine", ["0x40"])
                    portable_plan = run_portable_creation_plan(
                        repo,
                        temp,
                        local_url,
                        bundle,
                        deploy_hash,
                        deploy_receipt,
                    )

                transaction_summaries = []
                for transaction in bundle["creation_batch"]["wallet_calls"]:
                    transaction_hash = rpc(
                        local_url,
                        "eth_sendTransaction",
                        [
                            {
                                "from": creator,
                                "to": transaction["to"],
                                "data": transaction["data"],
                                "value": "0x0",
                            }
                        ],
                    )
                    receipt = wait_receipt(local_url, transaction_hash)
                    transaction_summaries.append(
                        {
                            "function": transaction["function"],
                            "gas_used": uint_result(receipt["gasUsed"]),
                            "log_count": len(receipt.get("logs", [])),
                        }
                    )

                bounty_summaries = []
                for index, bounty in enumerate(bundle["bounties"]):
                    contract = bounty["predicted_bounty_contract"]
                    creation = bundle["creation_batch"]["creations"][index]
                    expected_initial_funding = int(
                        creation["eip3009_authorization"]["message"]["value"]
                    )
                    create_data = creation["create_bounty"]["data"]
                    expected_target = uint256_argument(create_data, 0) + uint256_argument(
                        create_data, 1
                    )
                    expected_status = 1 if expected_initial_funding == expected_target else 0
                    canonical = uint_result(
                        call(local_url, observed_factory, "isCanonicalBounty(address)", address_word(contract))
                    )
                    bounty_id = call(local_url, contract, "bountyId()").lower()
                    funded = uint_result(call(local_url, contract, "fundedAmount()"))
                    target = uint_result(call(local_url, contract, "targetAmount()"))
                    status = uint_result(call(local_url, contract, "status()"))
                    verifier_module = address_result(call(local_url, contract, "verifierModule()"))
                    token_balance = uint_result(
                        call(
                            local_url,
                            deployment["settlement_token"],
                            "balanceOf(address)",
                            address_word(contract),
                        )
                    )
                    if (
                        canonical != 1
                        or bounty_id != bounty["bounty_id"].lower()
                        or funded != expected_initial_funding
                        or target != expected_target
                        or status != expected_status
                        or token_balance != expected_initial_funding
                        or (
                            verifier_summary is not None
                            and verifier_module != verifier_summary["contract"]
                        )
                    ):
                        raise RuntimeError(
                            f"issue {bounty['issue']} did not reach its committed funding state"
                        )
                    bounty_summaries.append(
                        {
                            "issue": bounty["issue"],
                            "contract": contract,
                            "bounty_id": bounty_id,
                            "funded": funded,
                            "target": target,
                            "status": "claimable" if expected_status == 1 else "seeking_funding",
                            "verifier_module": verifier_module,
                        }
                    )
                final_wallet_usdc = uint_result(
                    call(
                        local_url,
                        deployment["settlement_token"],
                        "balanceOf(address)",
                        address_word(creator),
                    )
                )
                if final_wallet_usdc != wallet_usdc - total_funding:
                    raise RuntimeError("forked creator USDC conservation check failed")
                return {
                    "result": "pass",
                    "network": "base-mainnet-fork",
                    "fork_block": fork_block,
                    "deployer_nonce": nonce,
                    "factory": observed_factory,
                    "implementation": observed_implementation,
                    "factory_source": "existing" if factory_exists else "rehearsed_deployment",
                    "deployment_gas_used": deployment_gas_used,
                    "verifier": verifier_summary,
                    "portable_creation_plan": portable_plan,
                    "transactions": transaction_summaries,
                    "bounties": bounty_summaries,
                    "creator_usdc_before": wallet_usdc,
                    "creator_usdc_after": final_wallet_usdc,
                    "creator_eth_before_wei": wallet_eth,
                    "evidence_boundary": "Fork transaction hashes are rehearsal evidence only and do not prove mainnet funding or payout.",
                }
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bundle",
        default="deployments/base-mainnet-activation.json",
        help="unsigned activation bundle generated by the Rust CLI",
    )
    parser.add_argument(
        "--fork-url",
        default=os.environ.get("BASE_MAINNET_RPC_URL", "https://mainnet.base.org"),
    )
    parser.add_argument("--anvil", help="path to the anvil executable")
    parser.add_argument(
        "--fork-block-number",
        type=int,
        help="optional exact Base block for a historical deployment rehearsal",
    )
    parser.add_argument(
        "--expect-existing-factory",
        action="store_true",
        help="require and reuse the attested canonical factory on the fork",
    )
    parser.add_argument(
        "--verifier-deployment",
        help="optional exact verifier deployment bundle to replay before bounty creation",
    )
    parser.add_argument("--output", help="optional JSON evidence output path")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    try:
        result = rehearse(
            repo,
            repo / args.bundle,
            args.fork_url,
            args.anvil,
            expect_existing_factory=args.expect_existing_factory,
            verifier_deployment_path=(repo / args.verifier_deployment)
            if args.verifier_deployment
            else None,
            fork_block_number=args.fork_block_number,
        )
    except Exception as error:  # noqa: BLE001 - CLI must emit one actionable failure.
        print(json.dumps({"result": "fail", "error": str(error)}, indent=2), file=sys.stderr)
        return 1
    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        output = repo / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered, encoding="utf-8")
        print(f"activation_rehearsal={output}")
    else:
        print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
