#!/usr/bin/env python3
"""Plan one policy-bound Base USDC agent budget without handling private keys."""

from __future__ import annotations

import argparse
import json
import re
import secrets
import shutil
import subprocess
import time
from decimal import Decimal, InvalidOperation
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"
POLICY_TYPE = (
    "(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32)"
)
ZERO_HASH = "0x" + "00" * 32
CLONE_PREFIX = "3d602d80600a3d3981f3" "363d3d373d3d3d363d73"
CLONE_SUFFIX = "5af43d82803e903d91602b57fd5bf3"
EXPECTED_NETWORK = "base-mainnet"
EXPECTED_CHAIN_ID = 8453
EXPECTED_CANONICAL = {
    "bounty_factory": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
    "settlement_token": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    "deterministic_verifier": "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e",
}
EXPECTED_CREATE2_DEPLOYER = "0x4e59b44847b379578588920ca78fbf26c0b4956c"
EXPECTED_CREATE2_DEPLOYER_HASH = "0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989"


def executable(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    candidate = ROOT / ".tools" / "foundry" / f"{name}.exe"
    if candidate.exists():
        return str(candidate)
    raise SystemExit(f"{name} is required; install Foundry or use .tools/foundry")


CAST = executable("cast")


def cast(*args: str, input_text: str | None = None) -> str:
    result = subprocess.run(
        [CAST, *args],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        input=input_text,
    )
    return result.stdout.strip()


def require_address(value: str, label: str) -> str:
    normalized = value.strip().lower()
    if len(normalized) != 42 or not normalized.startswith("0x"):
        raise SystemExit(f"{label} must be a 20-byte EVM address")
    try:
        bytes.fromhex(normalized[2:])
    except ValueError as error:
        raise SystemExit(f"{label} must be a 20-byte EVM address") from error
    if normalized == "0x" + "00" * 20:
        raise SystemExit(f"{label} cannot be zero")
    return normalized


def require_bytes32(value: str, label: str) -> str:
    normalized = value.strip().lower()
    if len(normalized) != 66 or not normalized.startswith("0x"):
        raise SystemExit(f"{label} must be 32 bytes")
    try:
        bytes.fromhex(normalized[2:])
    except ValueError as error:
        raise SystemExit(f"{label} must be 32 bytes") from error
    return normalized


def validate_manifest(manifest: dict) -> dict:
    if manifest.get("schema") != "agent-bounties/bounded-agent-wallet-deployment-v1":
        raise SystemExit("bounded-wallet manifest schema is unsupported")
    if manifest.get("network") != EXPECTED_NETWORK or manifest.get("chain_id") != EXPECTED_CHAIN_ID:
        raise SystemExit("bounded-wallet manifest must target Base mainnet")
    if manifest.get("contract_source_dirty") is not False:
        raise SystemExit("bounded-wallet manifest was generated from dirty contract inputs")
    if manifest.get("contract_source_revision_kind") != "git-tree":
        raise SystemExit("bounded-wallet manifest must use a content-addressed Git tree revision")
    if not re.fullmatch(r"[0-9a-f]{40}", str(manifest.get("contract_source_revision", ""))):
        raise SystemExit("bounded-wallet manifest does not pin a source revision")
    canonical = manifest.get("canonical") or {}
    for name, expected in EXPECTED_CANONICAL.items():
        if require_address(str(canonical.get(name, "")), name) != expected:
            raise SystemExit(f"bounded-wallet manifest has an unexpected canonical {name}")
    deployer = manifest.get("deterministic_deployer") or {}
    if require_address(str(deployer.get("address", "")), "deterministic deployer") != EXPECTED_CREATE2_DEPLOYER:
        raise SystemExit("bounded-wallet manifest has an unexpected deterministic deployer")
    if require_bytes32(str(deployer.get("runtime_code_hash", "")), "deployer runtime hash") != EXPECTED_CREATE2_DEPLOYER_HASH:
        raise SystemExit("bounded-wallet manifest has an unexpected deterministic deployer runtime")
    wallet_factory = manifest.get("wallet_factory") or {}
    require_address(str(wallet_factory.get("address", "")), "wallet factory")
    require_address(str(wallet_factory.get("implementation", "")), "wallet implementation")
    for name in ("salt", "init_code_hash", "runtime_code_hash", "implementation_runtime_code_hash", "clone_runtime_code_hash"):
        require_bytes32(str(wallet_factory.get(name, "")), name.replace("_", " "))
    transaction = str(wallet_factory.get("deployment_transaction", "")).lower()
    if not re.fullmatch(r"0x(?:[0-9a-f]{2})+", transaction):
        raise SystemExit("bounded-wallet manifest deployment transaction is invalid")
    return manifest


def usdc_units(value: str, label: str) -> int:
    try:
        amount = Decimal(value)
    except InvalidOperation as error:
        raise SystemExit(f"{label} must be a decimal USDC amount") from error
    units = amount * Decimal(1_000_000)
    if amount <= 0 or units != units.to_integral_value():
        raise SystemExit(f"{label} must be positive with at most six decimals")
    return int(units)


def keccak_hex(value: str) -> str:
    return cast("keccak", input_text=value).lower()


def encode(signature: str, *args: str) -> str:
    return cast("abi-encode", signature, *args).lower()


def calldata(signature: str, *args: str) -> str:
    return cast("calldata", signature, *args).lower()


def policy_tuple(policy: dict) -> str:
    return (
        f"({policy['delegate']},{policy['valid_after']},{policy['valid_until']},"
        f"{policy['period_seconds']},{policy['max_per_action']},{policy['max_per_period']},"
        f"{policy['max_lifetime_spend']},{policy['max_bounty_target']},"
        f"{policy['allowed_actions']},{policy['allowed_verification_modes']},"
        f"{policy['deterministic_verifier_module']},{policy['signed_quorum_verifier_set_hash']},"
        f"{policy['ai_judge_verifier_set_hash']})"
    )


def predicted_wallet(factory: str, implementation: str, owner: str, user_salt: str, policy_hash: str) -> tuple[str, str]:
    effective_preimage = encode("f(address,bytes32,bytes32)", owner, user_salt, policy_hash)
    effective_salt = keccak_hex(effective_preimage)
    init_code = f"0x{CLONE_PREFIX}{implementation[2:]}{CLONE_SUFFIX}"
    init_code_hash = keccak_hex(init_code)
    address = cast(
        "create2",
        "--deployer",
        factory,
        "--salt",
        effective_salt,
        "--init-code-hash",
        init_code_hash,
    ).splitlines()[0].lower()
    return address, effective_salt


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--owner", required=True)
    parser.add_argument("--delegate", required=True)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--initial-funding-usdc", default="89")
    parser.add_argument("--max-per-action-usdc", default="5")
    parser.add_argument("--max-per-period-usdc", default="10")
    parser.add_argument("--max-lifetime-usdc", default="89")
    parser.add_argument("--max-bounty-target-usdc", default="5")
    parser.add_argument("--period-seconds", type=int, default=86_400)
    parser.add_argument("--valid-after", type=int)
    parser.add_argument("--valid-until", type=int)
    parser.add_argument("--user-salt")
    parser.add_argument("--authorization-nonce")
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "bounded-agent-budget-plan.json")
    args = parser.parse_args()

    manifest = validate_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    owner = require_address(args.owner, "owner")
    delegate = require_address(args.delegate, "delegate")
    wallet_factory = require_address(manifest["wallet_factory"]["address"], "wallet factory")
    implementation = require_address(manifest["wallet_factory"]["implementation"], "implementation")
    usdc = require_address(manifest["canonical"]["settlement_token"], "settlement token")
    verifier = require_address(manifest["canonical"]["deterministic_verifier"], "verifier")
    now = int(time.time())
    valid_after = args.valid_after if args.valid_after is not None else now
    valid_until = args.valid_until if args.valid_until is not None else now + 30 * 86_400
    if valid_after < 0 or valid_until <= valid_after or valid_until > now + 366 * 86_400:
        raise SystemExit("policy validity must be positive, ordered, and no longer than 366 days from now")
    if args.period_seconds <= 0 or args.period_seconds > 30 * 86_400:
        raise SystemExit("period-seconds must be between 1 and 2592000")

    initial_funding = usdc_units(args.initial_funding_usdc, "initial funding")
    max_per_action = usdc_units(args.max_per_action_usdc, "per-action cap")
    max_per_period = usdc_units(args.max_per_period_usdc, "period cap")
    max_lifetime = usdc_units(args.max_lifetime_usdc, "lifetime cap")
    max_bounty_target = usdc_units(args.max_bounty_target_usdc, "bounty target cap")
    if initial_funding > max_lifetime:
        raise SystemExit("initial funding cannot exceed the policy lifetime cap")
    if max_per_action > max_per_period or max_per_period > max_lifetime:
        raise SystemExit("caps must satisfy per-action <= per-period <= lifetime")

    policy = {
        "delegate": delegate,
        "valid_after": valid_after,
        "valid_until": valid_until,
        "period_seconds": args.period_seconds,
        "max_per_action": max_per_action,
        "max_per_period": max_per_period,
        "max_lifetime_spend": max_lifetime,
        "max_bounty_target": max_bounty_target,
        "allowed_actions": 15,
        "allowed_verification_modes": 1,
        "deterministic_verifier_module": verifier,
        "signed_quorum_verifier_set_hash": ZERO_HASH,
        "ai_judge_verifier_set_hash": ZERO_HASH,
    }
    policy_value = policy_tuple(policy)
    policy_hash = keccak_hex(encode(f"f({POLICY_TYPE})", policy_value))
    user_salt = require_bytes32(args.user_salt, "user salt") if args.user_salt else f"0x{secrets.token_hex(32)}"
    wallet, effective_salt = predicted_wallet(wallet_factory, implementation, owner, user_salt, policy_hash)
    authorization_nonce = (
        require_bytes32(args.authorization_nonce, "authorization nonce")
        if args.authorization_nonce
        else f"0x{secrets.token_hex(32)}"
    )
    authorization_valid_after = max(0, now - 1)
    authorization_valid_before = now + 1_800
    call_signature = f"createWalletWithAuthorization(address,{POLICY_TYPE},bytes32,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)"
    fallback_signature = f"createWalletAndFund({POLICY_TYPE},bytes32,uint256)"
    plan = {
        "schema": "agent-bounties/bounded-agent-budget-plan-v1",
        "network": manifest["network"],
        "chain_id": manifest["chain_id"],
        "owner": owner,
        "delegate": delegate,
        "wallet_factory": wallet_factory,
        "predicted_wallet": wallet,
        "user_salt": user_salt,
        "effective_salt": effective_salt,
        "policy": policy,
        "policy_hash": policy_hash,
        "initial_funding": str(initial_funding),
        "authorization_typed_data": {
            "types": {
                "EIP712Domain": [
                    {"name": "name", "type": "string"},
                    {"name": "version", "type": "string"},
                    {"name": "chainId", "type": "uint256"},
                    {"name": "verifyingContract", "type": "address"},
                ],
                "TransferWithAuthorization": [
                    {"name": "from", "type": "address"},
                    {"name": "to", "type": "address"},
                    {"name": "value", "type": "uint256"},
                    {"name": "validAfter", "type": "uint256"},
                    {"name": "validBefore", "type": "uint256"},
                    {"name": "nonce", "type": "bytes32"},
                ],
            },
            "primaryType": "TransferWithAuthorization",
            "domain": {"name": "USD Coin", "version": "2", "chainId": manifest["chain_id"], "verifyingContract": usdc},
            "message": {
                "from": owner,
                "to": wallet,
                "value": str(initial_funding),
                "validAfter": str(authorization_valid_after),
                "validBefore": str(authorization_valid_before),
                "nonce": authorization_nonce,
            },
        },
        "relay_call": {
            "to": wallet_factory,
            "function": call_signature,
            "arguments_before_signature": [
                owner,
                policy_value,
                user_salt,
                str(initial_funding),
                str(authorization_valid_after),
                str(authorization_valid_before),
                authorization_nonce,
            ],
            "signature_tail": ["v", "r", "s"],
        },
        "direct_owner_fallback": {
            "approval": {
                "to": usdc,
                "data": calldata("approve(address,uint256)", wallet_factory, str(initial_funding)),
            },
            "create_and_fund": {
                "to": wallet_factory,
                "data": calldata(fallback_signature, policy_value, user_salt, str(initial_funding)),
            },
        },
        "owner_controls": {
            "revoke": {"to": wallet, "data": calldata("revokePolicy()")},
            "withdraw_all_usdc_template": {
                "to": wallet,
                "function": "withdrawToken(address,address,uint256)",
                "arguments": [usdc, owner, "CURRENT_WALLET_USDC_BALANCE"],
            },
        },
        "factory_deployment": manifest["wallet_factory"],
        "evidence_boundary": (
            "This plan contains no private key and moves no value. The owner signs only after comparing every "
            "policy field and the predicted USDC destination. A signature or transaction hash is not funding; "
            "confirm factory registration, wallet state, and token balance on-chain."
        ),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
