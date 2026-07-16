#!/usr/bin/env python3
"""Sponsor one exact bounded-wallet creation signed by its active delegate."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
from dataclasses import asdict, dataclass
from typing import Mapping, Sequence

from bounded_agent_create import (
    BOUNTY_ID_SIGNATURE,
    CREATE_SELECTOR,
    EXPECTED_IMPLEMENTATION,
    PREDICT_SIGNATURE,
    ZERO_ADDRESS,
    decode_create_calldata,
)
from relay_autonomous_action import (
    CHAIN_ID,
    CLONE_CODEHASH,
    FACTORY,
    MARKER,
    MAX_GAS_PRICE_WEI,
    REPOSITORY,
    USDC,
    CastClient,
    RelayError,
    RelayEvent,
    RelaySource,
    bool_value,
    normalize_address,
    parse_event_source,
    parse_uint,
    preflight_keeper,
    publish_comment,
    read_state,
    require_bytes32,
    require_exact_keys,
    validate_common,
    validate_receipt,
    write_json,
)


SCHEMA = "agent-bounties/bounded-wallet-relay-v1"
COMMAND = "/agent-bounty wallet-relay"
NETWORK = "base-mainnet"
RPC_URL = "https://mainnet.base.org"
WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
WALLET_FACTORY = "0x3840936351049aed639780a16845e6094c1f17f6"
WALLET_CODEHASH = "0xc663bed9b4097e22e5a18c0ecb662561bf45df1829e6412cdd0d8568d05ca1b6"
DELEGATE = "0xe98314df0e2f5657dd5ee3325f1e95f5a4249ef5"
POLICY_HASH = "0x2aed9973078480e80388bea6ed5662992a5a16698ef983a932f6a4ccc4bb158d"
VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e"
MAX_COMMENT_BYTES = 12_000
MAX_TARGET_MINOR = 5_000_000
MAX_VERIFIER_REWARD_MINOR = 500_000
MAX_SIGNATURE_WINDOW_SECONDS = 900
MAX_GAS_LIMIT = 1_500_000
MAX_GAS_COST_WEI = 200_000_000_000_000
SIGNATURE_RE = re.compile(r"^0x[0-9a-fA-F]{130}$")
DATA_RE = re.compile(r"^0x(?:[0-9a-fA-F]{2})+$")


@dataclass(frozen=True)
class WalletState:
    chain_id: int
    block_timestamp: int
    codehash: str
    registered: bool
    factory: str
    settlement_token: str
    deployment_factory: str
    owner: str
    delegate: str
    valid_after: int
    valid_until: int
    period_seconds: int
    max_per_action: int
    max_per_period: int
    max_lifetime_spend: int
    max_bounty_target: int
    allowed_actions: int
    allowed_verification_modes: int
    deterministic_verifier: str
    policy_hash: str
    policy_version: int
    delegate_nonce: int
    period_bucket: int
    period_spent: int
    lifetime_spent: int
    wallet_usdc_balance: int
    wallet_eth_balance: int
    revoked: bool


def parse_comment(body: str) -> dict[str, object]:
    if len(body.encode("utf-8", errors="strict")) > MAX_COMMENT_BYTES:
        raise RelayError("bounded-wallet relay comment is too large")
    lines = body.strip().splitlines()
    if not lines or lines[0].strip() != COMMAND:
        raise RelayError(f"first line must be exactly {COMMAND}")
    raw = "\n".join(lines[1:]).strip()
    if raw.startswith("```json") and raw.endswith("```"):
        raw = raw[7:-3].strip()
    elif raw.startswith("```") and raw.endswith("```"):
        raw = raw[3:-3].strip()
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise RelayError(f"bounded-wallet relay envelope is invalid JSON: {error.msg}") from error
    if not isinstance(value, dict):
        raise RelayError("bounded-wallet relay envelope must be an object")
    return validate_envelope(value)


def _positive_int(value: object, field: str, *, allow_zero: bool = False) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < (0 if allow_zero else 1):
        qualifier = "nonnegative" if allow_zero else "positive"
        raise RelayError(f"{field} must be a {qualifier} integer")
    return value


def validate_envelope(value: dict[str, object]) -> dict[str, object]:
    expected = {
        "schema",
        "network",
        "action",
        "issue_number",
        "wallet",
        "policy_hash",
        "policy_version",
        "nonce",
        "deadline",
        "payload",
        "payload_hash",
        "signature",
        "bounty_id",
        "predicted_bounty_contract",
    }
    require_exact_keys(value, expected, "bounded-wallet relay envelope")
    if value["schema"] != SCHEMA or value["network"] != NETWORK or value["action"] != "create":
        raise RelayError(f"schema, network, and action must be {SCHEMA}, {NETWORK}, and create")
    value["issue_number"] = _positive_int(value["issue_number"], "issue_number")
    value["policy_version"] = _positive_int(value["policy_version"], "policy_version")
    value["nonce"] = _positive_int(value["nonce"], "nonce", allow_zero=True)
    value["deadline"] = _positive_int(value["deadline"], "deadline")
    value["wallet"] = normalize_address(str(value["wallet"]))
    value["predicted_bounty_contract"] = normalize_address(str(value["predicted_bounty_contract"]))
    value["policy_hash"] = require_bytes32(value["policy_hash"], "policy_hash")
    value["payload_hash"] = require_bytes32(value["payload_hash"], "payload_hash")
    value["bounty_id"] = require_bytes32(value["bounty_id"], "bounty_id")
    payload = value["payload"]
    if not isinstance(payload, str) or not DATA_RE.fullmatch(payload):
        raise RelayError("payload must be nonempty 0x-prefixed byte hex")
    value["payload"] = payload.lower()
    signature = value["signature"]
    if not isinstance(signature, str) or not SIGNATURE_RE.fullmatch(signature):
        raise RelayError("signature must be one 65-byte ECDSA signature")
    value["signature"] = signature.lower()
    return value


def parse_event_request(source: RelaySource, body: str) -> RelayEvent:
    if "funding-needed" not in source.labels:
        raise RelayError("create relay requires the funding-needed label")
    if "verification-unavailable" in source.labels or "legacy-canary" in source.labels:
        raise RelayError("create relay is disabled for unavailable or legacy verification")
    envelope = parse_comment(body)
    if envelope["issue_number"] != source.issue_number:
        raise RelayError("relay envelope issue_number does not match the source issue")
    return RelayEvent(
        source.repository,
        source.issue_number,
        source.comment_id,
        source.comment_author,
        source.labels,
        envelope,
    )


def _lines(value: str, expected: int, label: str) -> list[str]:
    result = [line.strip() for line in value.splitlines() if line.strip()]
    if len(result) != expected:
        raise RelayError(f"{label} returned {len(result)} values; expected {expected}")
    return result


def read_wallet_state(client: CastClient, block: str | None = None) -> WalletState:
    tag = block or "latest"
    call = lambda signature, *args: client.call(WALLET, signature, *args, block=tag)
    policy = _lines(
        call(
            "policy()(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,"
            "address,bytes32,bytes32)"
        ),
        13,
        "wallet policy",
    )
    return WalletState(
        chain_id=client.chain_id(),
        block_timestamp=client.block_timestamp(tag),
        codehash=client.codehash(WALLET, tag),
        registered=bool_value(
            client.call(WALLET_FACTORY, "isFactoryWallet(address)(bool)", WALLET, block=tag)
        ),
        factory=normalize_address(call("factory()(address)")),
        settlement_token=normalize_address(call("settlementToken()(address)")),
        deployment_factory=normalize_address(call("deploymentFactory()(address)")),
        owner=normalize_address(call("owner()(address)")),
        delegate=normalize_address(policy[0]),
        valid_after=parse_uint(policy[1]),
        valid_until=parse_uint(policy[2]),
        period_seconds=parse_uint(policy[3]),
        max_per_action=parse_uint(policy[4]),
        max_per_period=parse_uint(policy[5]),
        max_lifetime_spend=parse_uint(policy[6]),
        max_bounty_target=parse_uint(policy[7]),
        allowed_actions=parse_uint(policy[8]),
        allowed_verification_modes=parse_uint(policy[9]),
        deterministic_verifier=normalize_address(policy[10]),
        policy_hash=call("policyHash()(bytes32)").strip().lower(),
        policy_version=parse_uint(call("policyVersion()(uint64)")),
        delegate_nonce=parse_uint(call("delegateNonce()(uint256)")),
        period_bucket=parse_uint(call("periodBucket()(uint256)")),
        period_spent=parse_uint(call("periodSpent()(uint256)")),
        lifetime_spent=parse_uint(call("lifetimeSpent()(uint256)")),
        wallet_usdc_balance=parse_uint(
            client.call(USDC, "balanceOf(address)(uint256)", WALLET, block=tag)
        ),
        wallet_eth_balance=client.balance(WALLET) if block is None else parse_uint(
            client.run("balance", WALLET, "--block", tag, "--rpc-url", client.rpc_url)
        ),
        revoked=bool_value(call("revoked()(bool)")),
    )


def validate_wallet(
    state: WalletState,
    envelope: Mapping[str, object],
    *,
    allow_next_nonce: bool = False,
) -> None:
    exact = {
        "chain_id": CHAIN_ID,
        "codehash": WALLET_CODEHASH,
        "registered": True,
        "factory": FACTORY,
        "settlement_token": USDC,
        "deployment_factory": WALLET_FACTORY,
        "delegate": DELEGATE,
        "deterministic_verifier": VERIFIER,
        "policy_hash": POLICY_HASH,
        "policy_version": int(envelope["policy_version"]),
        "allowed_actions": 15,
        "allowed_verification_modes": 1,
        "revoked": False,
    }
    for field, expected in exact.items():
        observed = getattr(state, field)
        if observed != expected:
            raise RelayError(f"bounded wallet {field} mismatch: expected {expected}, got {observed}")
    expected_nonce = int(envelope["nonce"])
    allowed_nonces = {expected_nonce, expected_nonce + 1} if allow_next_nonce else {expected_nonce}
    if state.delegate_nonce not in allowed_nonces:
        raise RelayError(
            f"bounded wallet delegate_nonce mismatch: expected {sorted(allowed_nonces)}, "
            f"got {state.delegate_nonce}"
        )
    if envelope["wallet"] != WALLET or envelope["policy_hash"] != POLICY_HASH:
        raise RelayError("relay is pinned to the activated bounded wallet and policy")
    if not state.valid_after <= state.block_timestamp <= state.valid_until:
        raise RelayError("bounded wallet policy is not active")
    if (
        state.period_seconds <= 0
        or state.max_per_action > MAX_TARGET_MINOR
        or state.max_bounty_target > MAX_TARGET_MINOR
        or state.max_per_period > state.max_lifetime_spend
    ):
        raise RelayError("bounded wallet spend policy exceeds public sponsor ceilings")


def _factory_word(client: CastClient, signature: str, *args: str, block: str = "latest") -> str:
    return client.call(FACTORY, signature, *args, block=block).strip().lower()


def validate_signed_creation(
    client: CastClient,
    envelope: Mapping[str, object],
    state: WalletState,
    *,
    allow_existing: bool = False,
) -> dict[str, object]:
    now = state.block_timestamp
    deadline = int(envelope["deadline"])
    if not allow_existing and (deadline < now or deadline > now + MAX_SIGNATURE_WINDOW_SECONDS):
        raise RelayError("delegate signature is expired or valid for more than fifteen minutes")
    if deadline > state.valid_until:
        raise RelayError("delegate signature outlives the bounded wallet policy")
    payload = str(envelope["payload"])
    payload_hash = client.run("keccak", payload).lower()
    if payload_hash != envelope["payload_hash"]:
        raise RelayError("payload hash does not match the signed payload")
    decoded = decode_create_calldata(f"{CREATE_SELECTOR}{payload[2:]}")
    params = decoded["params"]
    verifiers = decoded["verifiers"]
    assert isinstance(params, dict) and isinstance(verifiers, list)
    target = int(params["solver_reward"]) + int(params["verifier_reward"])
    initial = int(decoded["initial_funding"])
    if (
        int(params["solver_reward"]) <= 0
        or int(params["verifier_reward"]) <= 0
        or int(params["verifier_reward"]) > MAX_VERIFIER_REWARD_MINOR
        or target > min(MAX_TARGET_MINOR, state.max_bounty_target)
        or initial > target
    ):
        raise RelayError("creation economics exceed the public sponsor policy")
    if (
        int(params["verification_mode"]) != 0
        or params["verifier_module"] != state.deterministic_verifier
        or params["verifier_reward_recipient"] == ZERO_ADDRESS
        or int(params["threshold"]) != 1
        or verifiers
    ):
        raise RelayError("creation must use the exact deterministic verifier policy")
    if (
        int(params["funding_deadline"]) <= now
        or int(params["claim_window_seconds"]) <= 0
        or int(params["verification_window_seconds"]) <= 0
    ):
        raise RelayError("creation deadline or work windows are invalid")
    current_bucket = now // state.period_seconds
    period_spent = state.period_spent if current_bucket == state.period_bucket else 0
    if (
        initial > state.max_per_action
        or period_spent + initial > state.max_per_period
        or state.lifetime_spent + initial > state.max_lifetime_spend
        or initial > state.wallet_usdc_balance
    ):
        raise RelayError("creation exceeds the remaining bounded-wallet budget")
    bounty_id = _factory_word(
        client,
        BOUNTY_ID_SIGNATURE,
        WALLET,
        str(decoded["tuple_value"]),
        str(decoded["verifier_array"]),
        str(decoded["creation_nonce"]),
    )
    predicted = normalize_address(
        _factory_word(
            client,
            PREDICT_SIGNATURE,
            WALLET,
            str(decoded["tuple_value"]),
            str(decoded["verifier_array"]),
            str(decoded["creation_nonce"]),
        )
    )
    if bounty_id != envelope["bounty_id"] or predicted != envelope["predicted_bounty_contract"]:
        raise RelayError("factory bounty id or predicted address differs from the signed envelope")
    canonical = bool_value(
        client.call(FACTORY, "isCanonicalBounty(address)(bool)", predicted, block="latest")
    )
    code = client.run("code", predicted, "--block", "latest", "--rpc-url", client.rpc_url).lower()
    if allow_existing:
        if not canonical or code in {"0x", "0x0"}:
            raise RelayError("previously relayed bounty is not canonically deployed")
    elif canonical or code not in {"0x", "0x0"}:
        raise RelayError("predicted bounty address is already occupied or registered")
    digest = client.call(
        WALLET,
        "actionDigest(uint8,bytes32,uint256,uint256)(bytes32)",
        "0",
        payload_hash,
        str(envelope["nonce"]),
        str(deadline),
        block="latest",
    ).strip().lower()
    client.run(
        "wallet",
        "verify",
        "--address",
        state.delegate,
        digest,
        str(envelope["signature"]),
        "--no-hash",
        retry=False,
    )
    return {
        "decoded": decoded,
        "target": target,
        "initial_funding": initial,
        "bounty_id": bounty_id,
        "predicted": predicted,
        "payload_hash": payload_hash,
    }


def validate_created_bounty(
    client: CastClient,
    block: str,
    prepared: Mapping[str, object],
) -> dict[str, object]:
    predicted = str(prepared["predicted"])
    decoded = prepared["decoded"]
    assert isinstance(decoded, dict)
    params = decoded["params"]
    assert isinstance(params, dict)
    bounty = read_state(client, predicted, block=block)
    validate_common(bounty, require_funded=False)
    expected_status = 1 if prepared["initial_funding"] == prepared["target"] else 0
    expected = {
        "bounty_id": prepared["bounty_id"],
        "creator": WALLET,
        "solver_reward": int(params["solver_reward"]),
        "verifier_reward": int(params["verifier_reward"]),
        "target_amount": prepared["target"],
        "funded_amount": prepared["initial_funding"],
        "status": expected_status,
        "round": 0,
        "solver": ZERO_ADDRESS,
        "verification_mode": 0,
        "verifier_module": VERIFIER,
        "threshold": 1,
        "policy_hash": params["policy_hash"],
    }
    for field, wanted in expected.items():
        observed = getattr(bounty, field)
        if observed != wanted:
            raise RelayError(f"created bounty {field} mismatch: expected {wanted}, got {observed}")
    views = {
        "terms_hash": "termsHash()(bytes32)",
        "acceptance_criteria_hash": "acceptanceCriteriaHash()(bytes32)",
        "benchmark_hash": "benchmarkHash()(bytes32)",
        "evidence_schema_hash": "evidenceSchemaHash()(bytes32)",
        "funding_deadline": "fundingDeadline()(uint64)",
        "claim_window_seconds": "claimWindowSeconds()(uint64)",
        "verification_window_seconds": "verificationWindowSeconds()(uint64)",
        "verifier_reward_recipient": "verifierRewardRecipient()(address)",
    }
    for field, signature in views.items():
        raw = client.call(predicted, signature, block=block).strip()
        observed: object = normalize_address(raw) if field == "verifier_reward_recipient" else (
            parse_uint(raw) if field.endswith("seconds") or field == "funding_deadline" else raw.lower()
        )
        if observed != params[field]:
            raise RelayError(f"created bounty {field} does not match signed calldata")
    return asdict(bounty)


def relay(
    client: CastClient,
    event: RelayEvent,
    *,
    execute: bool,
    private_key: str | None,
) -> dict[str, object]:
    envelope = event.envelope
    before = read_wallet_state(client)
    already_relayed = before.delegate_nonce == int(envelope["nonce"]) + 1
    validate_wallet(before, envelope, allow_next_nonce=already_relayed)
    prepared = validate_signed_creation(
        client,
        envelope,
        before,
        allow_existing=already_relayed,
    )
    if already_relayed:
        bounty = validate_created_bounty(client, "latest", prepared)
        return {
            "schema": SCHEMA,
            "outcome": "already-relayed",
            "action": "create",
            "bounty_contract": prepared["predicted"],
            "bounty_id": prepared["bounty_id"],
            "issue_number": event.issue_number,
            "source_comment_id": event.comment_id,
            "source_comment_author": event.comment_author,
            "created_bounty": bounty,
            "claimable": int(prepared["initial_funding"]) == int(prepared["target"]),
            "agent_wallet_eth_required_wei": 0,
            "agent_wallet_eth_spent_wei": 0,
            "gas_payer": "previously-confirmed-relay",
            "after": asdict(before),
        }
    normalized_key, keeper, keeper_balance = preflight_keeper(client, private_key)
    call_signature = "executeWithSignature(uint8,bytes,uint256,uint256,bytes)"
    call_args = (
        "0",
        str(envelope["payload"]),
        str(envelope["nonce"]),
        str(envelope["deadline"]),
        str(envelope["signature"]),
    )
    gas_estimate = client.estimate(keeper, WALLET, call_signature, *call_args)
    gas_limit = gas_estimate * 125 // 100 + 10_000
    gas_price = client.gas_price()
    max_cost = gas_limit * gas_price
    if gas_limit > MAX_GAS_LIMIT:
        raise RelayError("bounded create gas exceeds the public sponsor limit")
    if gas_price > MAX_GAS_PRICE_WEI or max_cost > MAX_GAS_COST_WEI:
        raise RelayError("bounded create gas price or total cost exceeds the sponsor ceiling")
    if keeper_balance < max_cost:
        raise RelayError("keeper gas reserve is below the bounded create cost", retryable=True)
    report: dict[str, object] = {
        "schema": SCHEMA,
        "outcome": "validated",
        "action": "create",
        "bounty_contract": prepared["predicted"],
        "bounty_id": prepared["bounty_id"],
        "issue_number": event.issue_number,
        "source_comment_id": event.comment_id,
        "source_comment_author": event.comment_author,
        "keeper": keeper,
        "keeper_balance_before_wei": keeper_balance,
        "gas_estimate": gas_estimate,
        "gas_limit": gas_limit,
        "gas_price_wei": gas_price,
        "max_gas_cost_wei": max_cost,
        "agent_wallet_eth_required_wei": 0,
        "agent_wallet_eth_before_wei": before.wallet_eth_balance,
        "initial_funding_minor": prepared["initial_funding"],
        "before": asdict(before),
    }
    if not execute:
        return report
    receipt = client.send(normalized_key, gas_limit, WALLET, call_signature, *call_args)
    transaction_hash, block = validate_receipt(receipt, WALLET)
    after = read_wallet_state(client, block)
    if after.delegate_nonce != before.delegate_nonce + 1:
        raise RelayError("bounded wallet nonce did not advance exactly once")
    if after.lifetime_spent != before.lifetime_spent + int(prepared["initial_funding"]):
        raise RelayError("bounded wallet lifetime spend did not increase by initial funding")
    if after.wallet_usdc_balance != before.wallet_usdc_balance - int(prepared["initial_funding"]):
        raise RelayError("bounded wallet USDC balance does not reconcile with initial funding")
    if after.policy_hash != before.policy_hash or after.policy_version != before.policy_version:
        raise RelayError("bounded wallet policy changed during sponsored creation")
    allowance = parse_uint(
        client.call(USDC, "allowance(address,address)(uint256)", WALLET, FACTORY, block=block)
    )
    if allowance != 0:
        raise RelayError("bounded wallet factory allowance was not reset to zero")
    bounty = validate_created_bounty(client, block, prepared)
    report.update(
        {
            "outcome": "relayed",
            "transaction_hash": transaction_hash,
            "basescan_url": f"https://basescan.org/tx/{transaction_hash}",
            "after": asdict(after),
            "created_bounty": bounty,
            "claimable": int(prepared["initial_funding"]) == int(prepared["target"]),
            "agent_wallet_eth_spent_wei": 0,
            "gas_payer": keeper,
        }
    )
    return report


def render_comment(report: Mapping[str, object], comment_id: int) -> str:
    outcome = str(report.get("outcome") or "failed")
    lines = [
        MARKER,
        f"Source comment id: `{comment_id}`",
        f"### Bounded-wallet gas sponsor: {outcome}",
        "",
        f"Action: `create`",
        f"Bounty contract: `{report.get('bounty_contract', 'unknown')}`",
    ]
    if report.get("transaction_hash"):
        lines.append(f"Transaction: https://basescan.org/tx/{report['transaction_hash']}")
        lines.extend(
            [
                "",
                "Gas was sponsored by the capped keeper. The agent wallet required and spent `0 ETH`; "
                "its USDC changed only by the signed initial bounty funding.",
            ]
        )
    elif outcome == "already-relayed":
        lines.extend(
            [
                "",
                "The exact canonical bounty was already created by this signed nonce. "
                "No transaction was rebroadcast; current factory and bounty state were reconciled instead.",
            ]
        )
    if report.get("error"):
        lines.extend(["", f"Relay refused: {report['error']}"])
    if report.get("correction"):
        lines.extend(["", f"Next step: {report['correction']}"])
    lines.extend(
        [
            "",
            "A signature or transaction hash alone is not funding or payout evidence. Canonical factory, "
            "funding, claimability, and settlement events remain the evidence boundary.",
            "",
        ]
    )
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", type=pathlib.Path, default=os.environ.get("GITHUB_EVENT_PATH"))
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--publish", action="store_true")
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", RPC_URL))
    parser.add_argument("--cast-bin", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--report", type=pathlib.Path, default=pathlib.Path("target/bounded-wallet-relay.json"))
    parser.add_argument("--comment", type=pathlib.Path, default=pathlib.Path("target/bounded-wallet-relay.md"))
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if not args.event:
        print("bounded_wallet_relay=failed error=GITHUB_EVENT_PATH is required", file=sys.stderr)
        return 1
    source: RelaySource | None = None
    event: RelayEvent | None = None
    request_refused = False
    try:
        source, body = parse_event_source(pathlib.Path(args.event))
        try:
            event = parse_event_request(source, body)
        except RelayError as error:
            request_refused = True
            report: dict[str, object] = {
                "schema": SCHEMA,
                "outcome": "refused",
                "action": "create",
                "bounty_contract": "unknown",
                "issue_number": source.issue_number,
                "source_comment_id": source.comment_id,
                "error": str(error),
                "correction": f"Correct the envelope and post a fresh `{COMMAND}` command.",
            }
        else:
            client = CastClient(args.cast_bin, args.rpc_url, "finalized")
            report = relay(
                client,
                event,
                execute=args.execute,
                private_key=os.environ.get("BASE_KEEPER_PRIVATE_KEY"),
            )
    except (RelayError, json.JSONDecodeError, OSError, ValueError) as error:
        report = {
            "schema": SCHEMA,
            "outcome": "retryable" if isinstance(error, RelayError) and error.retryable else "failed",
            "action": "create",
            "bounty_contract": str(event.envelope.get("predicted_bounty_contract", "unknown")) if event else "unknown",
            "error": str(error),
            "retryable": isinstance(error, RelayError) and error.retryable,
        }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    write_json(args.report, report)
    comment = render_comment(report, source.comment_id if source else 0)
    args.comment.parent.mkdir(parents=True, exist_ok=True)
    args.comment.write_text(comment, encoding="utf-8")
    published = False
    if args.publish and source:
        try:
            publish_comment(source, comment, os.environ)
            published = True
        except (RelayError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
            print(f"bounded_wallet_relay=failed error=unable to publish result: {error}", file=sys.stderr)
            return 1
    print(
        f"bounded_wallet_relay={report['outcome']} contract={report.get('bounty_contract', 'unknown')}"
    )
    if request_refused:
        return 0 if published else 1
    return 0 if report["outcome"] in {"validated", "relayed", "already-relayed"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
