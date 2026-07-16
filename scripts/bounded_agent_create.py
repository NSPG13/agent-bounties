#!/usr/bin/env python3
"""Fail-closed validation for one bounded-wallet bounty creation."""

from __future__ import annotations

import re
from typing import Mapping

from inspect_bounded_agent_wallet import call, rpc, word_address, word_uint, words
from plan_bounded_agent_budget import calldata, require_address, require_bytes32


CREATE_PARAMS_TYPE = (
    "(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,"
    "uint8,address,address,uint8)"
)
CREATE_SIGNATURE = f"createBounty({CREATE_PARAMS_TYPE},address[],uint256,bytes32)"
CREATE_SELECTOR = "0x9d2e414c"
BOUNTY_ID_SIGNATURE = f"bountyIdFor(address,{CREATE_PARAMS_TYPE},address[],bytes32)(bytes32)"
PREDICT_SIGNATURE = f"predictBountyAddress(address,{CREATE_PARAMS_TYPE},address[],bytes32)(address)"
EXPECTED_IMPLEMENTATION = "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9"
ZERO_ADDRESS = "0x" + "00" * 20
ZERO_HASH = "0x" + "00" * 32
HEX_DATA = re.compile(r"^0x(?:[0-9a-fA-F]{2})+$")

PARAM_NAMES = (
    "solver_reward",
    "verifier_reward",
    "terms_hash",
    "policy_hash",
    "acceptance_criteria_hash",
    "benchmark_hash",
    "evidence_schema_hash",
    "funding_deadline",
    "claim_window_seconds",
    "verification_window_seconds",
    "verification_mode",
    "verifier_module",
    "verifier_reward_recipient",
    "threshold",
)


def fail(message: str) -> None:
    raise SystemExit(message)


def require_exact_keys(value: Mapping[str, object], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        fail(
            f"{label} keys mismatch: missing={sorted(expected - actual)}, "
            f"extra={sorted(actual - expected)}"
        )


def require_uint(value: object, label: str) -> int:
    if isinstance(value, bool):
        fail(f"{label} must be an unsigned integer")
    if isinstance(value, int):
        result = value
    elif isinstance(value, str) and value.isdigit():
        result = int(value)
    else:
        fail(f"{label} must be an unsigned integer")
    if result < 0 or result >= 1 << 256:
        fail(f"{label} is outside uint256")
    return result


def _address_word(word: str, label: str, *, allow_zero: bool = False) -> str:
    if len(word) != 64 or word[:24] != "0" * 24:
        fail(f"{label} has noncanonical address padding")
    address = require_address(f"0x{word[24:]}", label) if int(word[24:], 16) else ZERO_ADDRESS
    if address == ZERO_ADDRESS and not allow_zero:
        fail(f"{label} cannot be zero")
    return address


def _hash_word(word: str, label: str) -> str:
    value = require_bytes32(f"0x{word}", label)
    if value == ZERO_HASH:
        fail(f"{label} cannot be zero")
    return value


def _tuple_value(params: Mapping[str, object]) -> str:
    ordered = [
        str(params["solver_reward"]),
        str(params["verifier_reward"]),
        str(params["terms_hash"]),
        str(params["policy_hash"]),
        str(params["acceptance_criteria_hash"]),
        str(params["benchmark_hash"]),
        str(params["evidence_schema_hash"]),
        str(params["funding_deadline"]),
        str(params["claim_window_seconds"]),
        str(params["verification_window_seconds"]),
        str(params["verification_mode"]),
        str(params["verifier_module"]),
        str(params["verifier_reward_recipient"]),
        str(params["threshold"]),
    ]
    return f"({','.join(ordered)})"


def decode_create_calldata(data: object) -> dict[str, object]:
    if not isinstance(data, str) or not HEX_DATA.fullmatch(data):
        fail("create calldata must be nonempty 0x-prefixed byte hex")
    normalized = data.lower()
    if not normalized.startswith(CREATE_SELECTOR):
        fail("create calldata selector is not canonical createBounty")
    raw = normalized[10:]
    if len(raw) % 64:
        fail("create calldata arguments are not ABI word aligned")
    encoded_words = [raw[index : index + 64] for index in range(0, len(raw), 64)]
    if len(encoded_words) < 18:
        fail("create calldata is truncated")
    if int(encoded_words[14], 16) != 17 * 32:
        fail("create calldata has a noncanonical verifier-array offset")
    verifier_count = int(encoded_words[17], 16)
    if verifier_count > 8:
        fail("create calldata contains more than eight verifiers")
    if len(encoded_words) != 18 + verifier_count:
        fail("create calldata has trailing or truncated verifier data")

    params: dict[str, object] = {
        "solver_reward": int(encoded_words[0], 16),
        "verifier_reward": int(encoded_words[1], 16),
        "terms_hash": _hash_word(encoded_words[2], "terms hash"),
        "policy_hash": _hash_word(encoded_words[3], "policy hash"),
        "acceptance_criteria_hash": _hash_word(encoded_words[4], "acceptance criteria hash"),
        "benchmark_hash": _hash_word(encoded_words[5], "benchmark hash"),
        "evidence_schema_hash": _hash_word(encoded_words[6], "evidence schema hash"),
        "funding_deadline": int(encoded_words[7], 16),
        "claim_window_seconds": int(encoded_words[8], 16),
        "verification_window_seconds": int(encoded_words[9], 16),
        "verification_mode": int(encoded_words[10], 16),
        "verifier_module": _address_word(encoded_words[11], "verifier module", allow_zero=True),
        "verifier_reward_recipient": _address_word(
            encoded_words[12], "verifier reward recipient", allow_zero=True
        ),
        "threshold": int(encoded_words[13], 16),
    }
    for field in ("funding_deadline", "claim_window_seconds", "verification_window_seconds"):
        if int(params[field]) >= 1 << 64:
            fail(f"{field.replace('_', ' ')} is outside uint64")
    for field in ("verification_mode", "threshold"):
        if int(params[field]) >= 1 << 8:
            fail(f"{field.replace('_', ' ')} is outside uint8")
    creation_nonce = _hash_word(encoded_words[16], "creation nonce")
    verifiers = [
        _address_word(encoded_words[18 + index], f"verifier {index}")
        for index in range(verifier_count)
    ]
    tuple_value = _tuple_value(params)
    verifier_array = f"[{','.join(verifiers)}]"
    initial_funding = int(encoded_words[15], 16)
    expected = calldata(
        CREATE_SIGNATURE,
        tuple_value,
        verifier_array,
        str(initial_funding),
        creation_nonce,
    )
    if expected != normalized:
        fail("create calldata is not the canonical ABI encoding")
    return {
        "params": params,
        "verifiers": verifiers,
        "initial_funding": initial_funding,
        "creation_nonce": creation_nonce,
        "tuple_value": tuple_value,
        "verifier_array": verifier_array,
        "payload": f"0x{raw}",
        "direct_data": normalized,
    }


def normalize_intent(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        fail(f"{label} must be an object")
    require_exact_keys(value, {"from", "to", "value_wei", "data", "function"}, label)
    sender = require_address(str(value.get("from", "")), f"{label} sender")
    target = require_address(str(value.get("to", "")), f"{label} target")
    data = value.get("data")
    if not isinstance(data, str) or not HEX_DATA.fullmatch(data):
        fail(f"{label} calldata is invalid")
    function = value.get("function")
    if not isinstance(function, str) or not function:
        fail(f"{label} function is invalid")
    return {
        "from": sender,
        "to": target,
        "value_wei": require_uint(value.get("value_wei"), f"{label} value"),
        "data": data.lower(),
        "function": function,
    }


def _one_word(
    rpc_url: str,
    target: str,
    signature: str,
    block: str,
    arguments: tuple[str, ...] = (),
) -> str:
    result = words(call(rpc_url, target, signature, block, arguments))
    if len(result) != 1:
        fail(f"{signature} returned an unexpected shape")
    return result[0]


def _validate_spend(report: Mapping[str, object], amount: int) -> None:
    state = report["state"]
    assert isinstance(state, dict)
    policy = state["policy"]
    safe_block = report["safe_block"]
    assert isinstance(policy, dict) and isinstance(safe_block, dict)
    if amount > int(policy["max_per_action"]):
        fail("creation exceeds the per-action cap")
    current_bucket = int(safe_block["timestamp"]) // int(policy["period_seconds"])
    period_spent = int(state["period_spent"]) if current_bucket == int(state["period_bucket"]) else 0
    if period_spent + amount > int(policy["max_per_period"]):
        fail("creation exceeds the remaining period cap")
    if int(state["lifetime_spent"]) + amount > int(policy["max_lifetime_spend"]):
        fail("creation exceeds the remaining lifetime cap")
    if int(state["wallet_usdc_balance"]) < amount:
        fail("wallet USDC balance is below initial funding")


def validate_creation_plan(
    plan: object,
    wallet: str,
    manifest: Mapping[str, object],
    report: Mapping[str, object],
    rpc_url: str,
    block: str,
) -> dict[str, object]:
    if not isinstance(plan, dict):
        fail("creation plan must be a JSON object")
    require_exact_keys(
        plan,
        {
            "protocol_version",
            "network",
            "factory_contract",
            "implementation_contract",
            "bounty_id",
            "predicted_bounty_contract",
            "approve",
            "create_bounty",
            "wallet_calls",
            "supports_single_wallet_batch",
            "eip3009_authorization",
            "evidence_boundary",
        },
        "creation plan",
    )
    if plan["protocol_version"] != "agent-bounties/autonomous-v1":
        fail("creation plan protocol version is unsupported")
    network = plan["network"]
    if not isinstance(network, dict):
        fail("creation plan network is missing")
    require_exact_keys(
        network,
        {"name", "chain_id", "rpc_url_env", "native_usdc_token_address"},
        "creation plan network",
    )
    canonical = manifest.get("canonical")
    if not isinstance(canonical, dict):
        fail("bounded-wallet manifest canonical bindings are missing")
    factory = require_address(str(canonical.get("bounty_factory", "")), "bounty factory")
    token = require_address(str(canonical.get("settlement_token", "")), "settlement token")
    if (
        network["name"] != "Base"
        or require_uint(network["chain_id"], "network chain id") != 8453
        or network["rpc_url_env"] != "BASE_MAINNET_RPC_URL"
        or require_address(str(network["native_usdc_token_address"]), "network USDC") != token
    ):
        fail("creation plan network binding is not canonical Base mainnet USDC")
    if require_address(str(plan["factory_contract"]), "plan factory") != factory:
        fail("creation plan factory does not match the wallet policy")
    live_implementation = word_address(
        _one_word(rpc_url, factory, "implementation()(address)", block)
    )
    implementation = require_address(str(plan["implementation_contract"]), "plan implementation")
    if live_implementation != EXPECTED_IMPLEMENTATION or implementation != live_implementation:
        fail("creation plan implementation does not match the canonical factory")

    create_intent = normalize_intent(plan["create_bounty"], "create transaction")
    if (
        create_intent["from"] != wallet
        or create_intent["to"] != factory
        or create_intent["value_wei"] != 0
        or create_intent["function"] != CREATE_SIGNATURE
    ):
        fail("creation plan contains an unexpected creator, factory, value, or function")
    decoded = decode_create_calldata(create_intent["data"])
    params = decoded["params"]
    assert isinstance(params, dict)
    verifiers = decoded["verifiers"]
    assert isinstance(verifiers, list)
    initial_funding = int(decoded["initial_funding"])
    target = int(params["solver_reward"]) + int(params["verifier_reward"])
    if int(params["solver_reward"]) <= 0 or int(params["verifier_reward"]) <= 0:
        fail("solver and verifier rewards must be positive")
    if target >= 1 << 64 or initial_funding > target:
        fail("creation target or initial funding is invalid")
    if (
        int(params["funding_deadline"]) <= int(report["safe_block"]["timestamp"])
        or int(params["claim_window_seconds"]) <= 0
        or int(params["verification_window_seconds"]) <= 0
    ):
        fail("creation deadline and work windows must be positive and live")
    state = report["state"]
    assert isinstance(state, dict)
    policy = state["policy"]
    assert isinstance(policy, dict)
    if not int(policy["allowed_actions"]) & 1:
        fail("create action is not enabled by the wallet policy")
    mode = int(params["verification_mode"])
    if not int(policy["allowed_verification_modes"]) & (1 << mode):
        fail("creation verification mode is not allowed")
    if (
        mode != 0
        or params["verifier_module"] != policy["deterministic_verifier_module"]
        or params["verifier_reward_recipient"] == ZERO_ADDRESS
        or int(params["threshold"]) != 1
        or verifiers
    ):
        fail("bounded creation requires the exact deterministic verifier policy")
    if target > int(policy["max_bounty_target"]):
        fail("creation target exceeds the wallet policy")
    _validate_spend(report, initial_funding)

    expected_create = calldata(
        CREATE_SIGNATURE,
        str(decoded["tuple_value"]),
        str(decoded["verifier_array"]),
        str(initial_funding),
        str(decoded["creation_nonce"]),
    )
    if create_intent["data"] != expected_create:
        fail("creation plan calldata changed after decoding")
    approve = plan["approve"]
    expected_calls: list[dict[str, object]] = []
    if initial_funding == 0:
        if approve is not None or plan["eip3009_authorization"] is not None:
            fail("zero-funded creation plan must not contain funding authorization")
    else:
        approve_intent = normalize_intent(approve, "approval transaction")
        expected_approve = {
            "from": wallet,
            "to": token,
            "value_wei": 0,
            "data": calldata("approve(address,uint256)", factory, str(initial_funding)),
            "function": "approve(address,uint256)",
        }
        if approve_intent != expected_approve:
            fail("creation plan approval is not the exact factory allowance")
        if not isinstance(plan["eip3009_authorization"], dict):
            fail("funded creation plan is missing its standard authorization metadata")
        expected_calls.append(expected_approve)
    expected_calls.append(create_intent)
    wallet_calls = plan["wallet_calls"]
    if not isinstance(wallet_calls, list):
        fail("creation plan wallet calls must be an array")
    normalized_calls = [normalize_intent(item, f"wallet call {index}") for index, item in enumerate(wallet_calls)]
    if normalized_calls != expected_calls or plan["supports_single_wallet_batch"] is not True:
        fail("creation plan wallet call sequence is not canonical")

    tuple_value = str(decoded["tuple_value"])
    verifier_array = str(decoded["verifier_array"])
    creation_nonce = str(decoded["creation_nonce"])
    bounty_id = f"0x{_one_word(rpc_url, factory, BOUNTY_ID_SIGNATURE, block, (wallet, tuple_value, verifier_array, creation_nonce))}"
    predicted = word_address(
        _one_word(rpc_url, factory, PREDICT_SIGNATURE, block, (wallet, tuple_value, verifier_array, creation_nonce))
    )
    if require_bytes32(str(plan["bounty_id"]), "plan bounty id") != bounty_id:
        fail("creation plan bounty id does not match the canonical factory")
    if require_address(str(plan["predicted_bounty_contract"]), "predicted bounty") != predicted:
        fail("creation plan predicted address does not match the canonical factory")
    registered = bool(
        word_uint(_one_word(rpc_url, factory, "isCanonicalBounty(address)(bool)", block, (predicted,)))
    )
    code = rpc(rpc_url, "eth_getCode", [predicted, block], 70)
    if registered or not isinstance(code, str) or code.lower() not in {"0x", "0x0"}:
        fail("predicted bounty address is already occupied or registered")

    summary = {
        "factory": factory,
        "implementation": implementation,
        "creator": wallet,
        "bounty_id": bounty_id,
        "predicted_bounty_contract": predicted,
        "not_deployed": True,
        **params,
        "verifiers": verifiers,
        "initial_funding": initial_funding,
        "target_amount": target,
        "creation_nonce": creation_nonce,
    }
    return {
        "summary": summary,
        "payload": decoded["payload"],
        "direct_data": decoded["direct_data"],
        "spend": initial_funding,
        "bounty": predicted,
    }
