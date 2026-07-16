#!/usr/bin/env python3
"""Relay one bounded autonomous-v1 solver action from a GitHub issue comment.

The keeper is intentionally not a general transaction signer. It accepts only
three exact calls against low-value canonical Base-mainnet bounties using the
deployed deterministic verifier module.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from typing import Any, Mapping, Sequence


SCHEMA = "agent-bounties/autonomous-gas-relay-v1"
COMMAND = "/agent-bounty relay"
MARKER = "<!-- agent-bounties-autonomous-gas-relay -->"
REPOSITORY = "NSPG13/agent-bounties"
CHAIN_ID = 8453
RPC_URL = "https://mainnet.base.org"
FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
IMPLEMENTATION = "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
LEADING_ZERO_WORK_VERIFIER_MODULE = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e"
# Backwards-compatible name used by fixtures and downstream scripts.
VERIFIER_MODULE = LEADING_ZERO_WORK_VERIFIER_MODULE
ALLOWED_VERIFIER_MODULES = frozenset({LEADING_ZERO_WORK_VERIFIER_MODULE})
CLONE_CODEHASH = "0x6e7d6297e170d10e6484c9b72314bb0e2173cd967aa8e05231ee369dbde0c0a1"
ZERO_ADDRESS = "0x0000000000000000000000000000000000000000"
MAX_COMMENT_BYTES = 8_192
MAX_TARGET_MINOR = 5_000_000
MAX_BOND_MINOR = 500_000
MAX_AUTHORIZATION_WINDOW_SECONDS = 3_600
MAX_GAS_PRICE_WEI = 100_000_000
MAX_GAS_COST_WEI = 100_000_000_000_000
DEFAULT_STATE_WAIT_SECONDS = 120
DEFAULT_STATE_POLL_SECONDS = 3
DEFAULT_COMMAND_TIMEOUT_SECONDS = 30
DEFAULT_OPERATION_TIMEOUT_SECONDS = 240
GAS_CAPS = {"claim": 350_000, "submit": 250_000, "settle": 500_000}
STATUS_OPEN = 0
STATUS_CLAIMABLE = 1
STATUS_CLAIMED = 2
STATUS_SUBMITTED = 3
STATUS_SETTLED = 4
HEX_32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
HEX_SIGNATURE_RE = re.compile(r"^0x[0-9a-fA-F]{130}$")
PRIVATE_KEY_RE = re.compile(r"^(?:0x)?[0-9a-fA-F]{64}$")


class RelayError(RuntimeError):
    def __init__(
        self,
        message: str,
        *,
        code: str = "relay_refused",
        retryable: bool = False,
        details: Mapping[str, object] | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.retryable = retryable
        self.details = dict(details or {})


def normalize_private_key(value: str) -> str:
    normalized = value.strip()
    if not PRIVATE_KEY_RE.fullmatch(normalized):
        raise RelayError(
            "keeper signer unavailable: BASE_KEEPER_PRIVATE_KEY must be exactly "
            "one 32-byte hex value",
            code="keeper_configuration_invalid",
            retryable=True,
        )
    return normalized


def parse_uint(value: str) -> int:
    return int(value.strip().split()[0], 0)


def normalize_address(value: str) -> str:
    normalized = value.strip().lower()
    if len(normalized) != 42 or not normalized.startswith("0x"):
        raise RelayError(f"invalid EVM address: {value!r}")
    try:
        int(normalized[2:], 16)
    except ValueError as error:
        raise RelayError(f"invalid EVM address: {value!r}") from error
    return normalized


def require_bytes32(value: object, field: str, *, nonzero: bool = True) -> str:
    if not isinstance(value, str) or not HEX_32_RE.fullmatch(value):
        raise RelayError(f"{field} must be 0x-prefixed bytes32 hex")
    normalized = value.lower()
    if nonzero and normalized == "0x" + "00" * 32:
        raise RelayError(f"{field} must be nonzero")
    return normalized


def require_exact_keys(value: Mapping[str, object], expected: set[str], context: str) -> None:
    actual = set(value)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise RelayError(f"{context} keys mismatch: missing={missing}, extra={extra}")


@dataclass(frozen=True)
class RelaySource:
    repository: str
    issue_number: int
    comment_id: int
    comment_author: str
    labels: tuple[str, ...]


@dataclass(frozen=True)
class RelayEvent(RelaySource):
    envelope: dict[str, object]


def parse_event_source(path: pathlib.Path) -> tuple[RelaySource, str]:
    raw = path.read_bytes()
    if len(raw) > 2_000_000:
        raise RelayError("GitHub event payload is too large")
    event = json.loads(raw.decode("utf-8", errors="strict"))
    if not isinstance(event, dict):
        raise RelayError("GitHub event payload must be an object")
    repository = event.get("repository")
    issue = event.get("issue")
    comment = event.get("comment")
    if not isinstance(repository, dict) or not isinstance(issue, dict) or not isinstance(comment, dict):
        raise RelayError("issue_comment event is required")
    if issue.get("pull_request") is not None:
        raise RelayError("relay commands are accepted on bounty issues, not pull requests")
    full_name = str(repository.get("full_name") or "")
    if full_name != REPOSITORY:
        raise RelayError(f"relay is pinned to {REPOSITORY}")
    issue_number = issue.get("number")
    comment_id = comment.get("id")
    if not isinstance(issue_number, int) or issue_number <= 0:
        raise RelayError("issue number is missing")
    if not isinstance(comment_id, int) or comment_id <= 0:
        raise RelayError("comment id is missing")
    user = comment.get("user")
    author = str(user.get("login") or "") if isinstance(user, dict) else ""
    if not author:
        raise RelayError("comment author is missing")
    labels_value = issue.get("labels")
    labels = tuple(
        str(item.get("name"))
        for item in labels_value or []
        if isinstance(item, dict) and item.get("name")
    )
    body = str(comment.get("body") or "")
    return RelaySource(full_name, issue_number, comment_id, author, labels), body


def parse_event_request(source: RelaySource, body: str) -> RelayEvent:
    labels = source.labels
    if "funded-live" not in labels:
        raise RelayError("issue is not labeled funded-live")
    if "verification-unavailable" in labels or "legacy-canary" in labels:
        raise RelayError("relay is disabled for unavailable or legacy verification")
    envelope = parse_comment(body)
    return RelayEvent(
        source.repository,
        source.issue_number,
        source.comment_id,
        source.comment_author,
        source.labels,
        envelope,
    )


def parse_event(path: pathlib.Path) -> RelayEvent:
    source, body = parse_event_source(path)
    return parse_event_request(source, body)


def parse_comment(body: str) -> dict[str, object]:
    encoded = body.encode("utf-8", errors="strict")
    if len(encoded) > MAX_COMMENT_BYTES:
        raise RelayError("relay comment exceeds 8192 bytes")
    lines = body.strip().splitlines()
    if not lines or lines[0].strip() != COMMAND:
        raise RelayError(f"first line must be exactly {COMMAND}")
    payload = "\n".join(lines[1:]).strip()
    if payload.startswith("```json") and payload.endswith("```"):
        payload = payload[7:-3].strip()
    elif payload.startswith("```") and payload.endswith("```"):
        payload = payload[3:-3].strip()
    try:
        value = json.loads(payload)
    except json.JSONDecodeError as error:
        raise RelayError(f"relay envelope is not valid JSON: {error.msg}") from error
    if not isinstance(value, dict):
        raise RelayError("relay envelope must be a JSON object")
    try:
        return validate_envelope(value)
    except RelayError as error:
        error.details.setdefault("action", str(value.get("action") or "unknown"))
        error.details.setdefault(
            "bounty_contract", str(value.get("bounty_contract") or "unknown")
        )
        raise


def validate_envelope(value: dict[str, object]) -> dict[str, object]:
    action = value.get("action")
    if action not in GAS_CAPS:
        raise RelayError("action must be claim, submit, or settle")
    common = {"schema", "action", "network", "bounty_contract"}
    expected = {
        "claim": common | {"solver", "authorization"},
        "submit": common
        | {"solver", "round", "submission_hash", "evidence_hash", "deadline", "signature"},
        "settle": common | {"round", "proof"},
    }[str(action)]
    require_exact_keys(value, expected, "relay envelope")
    if value["schema"] != SCHEMA:
        raise RelayError(f"schema must be {SCHEMA}")
    if value["network"] != "base-mainnet":
        raise RelayError(
            f'network must be exactly "base-mainnet"; received {value["network"]!r}',
            details={
                "correction": (
                    'Set "network" to "base-mainnet" and post a new '
                    f'`{COMMAND}` command.'
                )
            },
        )
    value["bounty_contract"] = normalize_address(str(value["bounty_contract"]))
    if action == "claim":
        value["solver"] = normalize_address(str(value["solver"]))
        authorization = value["authorization"]
        if not isinstance(authorization, dict):
            raise RelayError("authorization must be an object")
        require_exact_keys(
            authorization,
            {"valid_after", "valid_before", "nonce", "v", "r", "s"},
            "authorization",
        )
        for field in ("valid_after", "valid_before", "v"):
            if not isinstance(authorization[field], int) or isinstance(authorization[field], bool):
                raise RelayError(f"authorization.{field} must be an integer")
        if authorization["valid_after"] != 0:
            raise RelayError("authorization.valid_after must be 0")
        if authorization["v"] not in (27, 28):
            raise RelayError("authorization.v must be 27 or 28")
        for field in ("nonce", "r", "s"):
            authorization[field] = require_bytes32(authorization[field], f"authorization.{field}")
    elif action == "submit":
        value["solver"] = normalize_address(str(value["solver"]))
        for field in ("round", "deadline"):
            if not isinstance(value[field], int) or isinstance(value[field], bool) or value[field] <= 0:
                raise RelayError(f"{field} must be a positive integer")
        value["submission_hash"] = require_bytes32(value["submission_hash"], "submission_hash")
        value["evidence_hash"] = require_bytes32(value["evidence_hash"], "evidence_hash")
        signature = value["signature"]
        if not isinstance(signature, str) or not HEX_SIGNATURE_RE.fullmatch(signature):
            raise RelayError("signature must be one 65-byte 0x-prefixed ECDSA signature")
        value["signature"] = signature.lower()
    else:
        if not isinstance(value["round"], int) or isinstance(value["round"], bool) or value["round"] <= 0:
            raise RelayError("round must be a positive integer")
        proof = value["proof"]
        if not isinstance(proof, str) or not HEX_32_RE.fullmatch(proof):
            raise RelayError("proof must be one ABI-encoded uint256 (32-byte hex)")
        value["proof"] = proof.lower()
    return value


@dataclass(frozen=True)
class BountyState:
    chain_id: int
    block_timestamp: int
    codehash: str
    canonical: bool
    factory_implementation: str
    factory: str
    settlement_token: str
    bounty_id: str
    creator: str
    solver_reward: int
    verifier_reward: int
    target_amount: int
    funded_amount: int
    status: int
    round: int
    solver: str
    claim_expires_at: int
    verification_expires_at: int
    active_claim_bond: int
    verification_mode: int
    verifier_module: str
    threshold: int
    policy_hash: str
    submission_hash: str
    evidence_hash: str


class CastClient:
    def __init__(
        self,
        cast_bin: str,
        rpc_url: str,
        block_tag: str = "finalized",
        command_timeout_seconds: int = DEFAULT_COMMAND_TIMEOUT_SECONDS,
        operation_timeout_seconds: int = DEFAULT_OPERATION_TIMEOUT_SECONDS,
    ) -> None:
        self.cast_bin = cast_bin
        self.rpc_url = rpc_url
        self.block_tag = block_tag
        self.command_timeout_seconds = command_timeout_seconds
        self.operation_timeout_seconds = operation_timeout_seconds
        self.operation_deadline = time.monotonic() + operation_timeout_seconds

    def remaining_command_timeout(self) -> float:
        remaining = self.operation_deadline - time.monotonic()
        if remaining <= 0:
            raise RelayError(
                f"relay operation exceeded its {self.operation_timeout_seconds}-second deadline",
                code="relay_operation_timeout",
                retryable=True,
            )
        return min(float(self.command_timeout_seconds), remaining)

    def run(self, *args: str, retry: bool = True) -> str:
        attempts = 3 if retry else 1
        message = "unknown cast failure"
        for attempt in range(attempts):
            timeout = self.remaining_command_timeout()
            try:
                completed = subprocess.run(
                    [self.cast_bin, *args],
                    check=False,
                    capture_output=True,
                    text=True,
                    encoding="utf-8",
                    timeout=timeout,
                )
            except subprocess.TimeoutExpired:
                message = f"cast operation timed out after {timeout:.1f} seconds"
                if self.operation_deadline - time.monotonic() <= 0:
                    raise RelayError(
                        "relay operation exceeded its "
                        f"{self.operation_timeout_seconds}-second deadline",
                        code="relay_operation_timeout",
                        retryable=True,
                    )
                if attempt + 1 < attempts:
                    time.sleep(2)
                continue
            if completed.returncode == 0:
                return completed.stdout.strip()
            message = completed.stderr.strip() or completed.stdout.strip()
            if attempt + 1 < attempts:
                time.sleep(2)
        raise RelayError(f"Base RPC/cast operation failed: {message}")

    def call(self, contract: str, signature: str, *args: str, block: str | None = None) -> str:
        command = ["call", contract, signature, *args]
        if block or self.block_tag:
            command.extend(["--block", block or self.block_tag])
        command.extend(["--rpc-url", self.rpc_url])
        return self.run(*command)

    def chain_id(self) -> int:
        return parse_uint(self.run("chain-id", "--rpc-url", self.rpc_url))

    def block_timestamp(self, block: str = "latest") -> int:
        return parse_uint(
            self.run("block", block, "--field", "timestamp", "--rpc-url", self.rpc_url)
        )

    def codehash(self, contract: str, block: str | None = None) -> str:
        code = self.run(
            "code",
            contract,
            "--block",
            block or self.block_tag,
            "--rpc-url",
            self.rpc_url,
        ).lower()
        if code == "0x":
            raise RelayError("bounty contract has no bytecode")
        return self.run("keccak", code).lower()

    def estimate(self, keeper: str, contract: str, signature: str, *args: str) -> int:
        return parse_uint(
            self.run(
                "estimate",
                contract,
                signature,
                *args,
                "--from",
                keeper,
                "--rpc-url",
                self.rpc_url,
                retry=False,
            )
        )

    def send(
        self,
        private_key: str,
        gas_limit: int,
        contract: str,
        signature: str,
        *args: str,
    ) -> dict[str, Any]:
        output = self.run(
            "send",
            contract,
            signature,
            *args,
            "--private-key",
            private_key,
            "--gas-limit",
            str(gas_limit),
            "--rpc-url",
            self.rpc_url,
            "--json",
            retry=False,
        )
        start = output.find("{")
        if start < 0:
            raise RelayError("cast send did not return a JSON receipt")
        receipt = json.loads(output[start:])
        if not isinstance(receipt, dict):
            raise RelayError("cast send receipt is not an object")
        return receipt

    def keeper_address(self, private_key: str) -> str:
        return normalize_address(
            self.run("wallet", "address", "--private-key", private_key)
        )

    def balance(self, account: str) -> int:
        return parse_uint(
            self.run("balance", account, "--block", "latest", "--rpc-url", self.rpc_url)
        )

    def gas_price(self) -> int:
        return parse_uint(self.run("gas-price", "--rpc-url", self.rpc_url))


def bool_value(value: str) -> bool:
    normalized = value.strip().lower()
    if normalized not in ("true", "false"):
        raise RelayError(f"expected bool RPC value, got {value!r}")
    return normalized == "true"


def preflight_keeper(
    client: CastClient, private_key: str | None
) -> tuple[str, str, int]:
    if not private_key:
        raise RelayError(
            "keeper signer unavailable: BASE_KEEPER_PRIVATE_KEY is required for execution",
            code="keeper_configuration_missing",
            retryable=True,
        )
    normalized_key = normalize_private_key(private_key)
    try:
        keeper = client.keeper_address(normalized_key)
        chain_id = client.chain_id()
        balance = client.balance(keeper)
    except RelayError as error:
        raise RelayError(
            f"keeper preflight unavailable: {error}",
            code="keeper_preflight_unavailable",
            retryable=True,
        ) from error
    if chain_id != CHAIN_ID:
        raise RelayError(
            f"keeper RPC is connected to chain {chain_id}; expected {CHAIN_ID}",
            code="keeper_chain_mismatch",
            retryable=True,
        )
    if balance < MAX_GAS_COST_WEI:
        raise RelayError(
            f"keeper {keeper} needs at least {MAX_GAS_COST_WEI} wei on Base for one bounded relay",
            code="keeper_balance_low",
            retryable=True,
        )
    return normalized_key, keeper, balance


def keeper_health(client: CastClient, private_key: str | None) -> dict[str, object]:
    _, keeper, balance = preflight_keeper(client, private_key)
    return {
        "schema": SCHEMA,
        "outcome": "healthy",
        "action": "health_check",
        "bounty_contract": "none",
        "keeper": keeper,
        "chain_id": CHAIN_ID,
        "keeper_balance_wei": balance,
        "minimum_balance_wei": MAX_GAS_COST_WEI,
    }


def read_state(client: CastClient, contract: str, block: str | None = None) -> BountyState:
    call = lambda signature, *args: client.call(contract, signature, *args, block=block)
    return BountyState(
        chain_id=client.chain_id(),
        block_timestamp=client.block_timestamp("latest" if block is None else block),
        codehash=client.codehash(contract, block),
        canonical=bool_value(
            client.call(FACTORY, "isCanonicalBounty(address)(bool)", contract, block=block)
        ),
        factory_implementation=normalize_address(
            client.call(FACTORY, "implementation()(address)", block=block)
        ),
        factory=normalize_address(call("factory()(address)")),
        settlement_token=normalize_address(call("settlementToken()(address)")),
        bounty_id=call("bountyId()(bytes32)").strip().lower(),
        creator=normalize_address(call("creator()(address)")),
        solver_reward=parse_uint(call("solverReward()(uint256)")),
        verifier_reward=parse_uint(call("verifierReward()(uint256)")),
        target_amount=parse_uint(call("targetAmount()(uint256)")),
        funded_amount=parse_uint(call("fundedAmount()(uint256)")),
        status=parse_uint(call("status()(uint8)")),
        round=parse_uint(call("round()(uint64)")),
        solver=normalize_address(call("solver()(address)")),
        claim_expires_at=parse_uint(call("claimExpiresAt()(uint64)")),
        verification_expires_at=parse_uint(call("verificationExpiresAt()(uint64)")),
        active_claim_bond=parse_uint(call("activeClaimBond()(uint256)")),
        verification_mode=parse_uint(call("verificationMode()(uint8)")),
        verifier_module=normalize_address(call("verifierModule()(address)")),
        threshold=parse_uint(call("threshold()(uint8)")),
        policy_hash=call("policyHash()(bytes32)").strip().lower(),
        submission_hash=call("submissionHash()(bytes32)").strip().lower(),
        evidence_hash=call("evidenceHash()(bytes32)").strip().lower(),
    )


def validate_common(state: BountyState, *, require_funded: bool = True) -> None:
    expected = {
        "chain_id": CHAIN_ID,
        "codehash": CLONE_CODEHASH,
        "canonical": True,
        "factory_implementation": IMPLEMENTATION,
        "factory": FACTORY,
        "settlement_token": USDC,
        "verification_mode": 0,
        "threshold": 1,
    }
    for field, wanted in expected.items():
        observed = getattr(state, field)
        if observed != wanted:
            raise RelayError(
                f"fail-closed canonical-state mismatch for {field}: expected {wanted}, got {observed}"
            )
    if state.verifier_module not in ALLOWED_VERIFIER_MODULES:
        raise RelayError(
            f"verifier module is not allowlisted for the bounded relay: {state.verifier_module}"
        )
    if state.target_amount <= 0 or state.target_amount > MAX_TARGET_MINOR:
        raise RelayError("bounty exceeds the public relay 5 USDC target cap")
    if state.verifier_reward <= 0 or state.verifier_reward > MAX_BOND_MINOR:
        raise RelayError("claim bond exceeds the public relay 0.5 USDC cap")
    if state.solver_reward + state.verifier_reward != state.target_amount:
        raise RelayError("bounty reward conservation is invalid")
    if require_funded and state.funded_amount != state.target_amount:
        raise RelayError("bounty is not fully funded")


def action_call(
    client: CastClient, event: RelayEvent, state: BountyState
) -> tuple[str, list[str]]:
    envelope = event.envelope
    action = str(envelope["action"])
    now = state.block_timestamp
    if action == "claim":
        solver = normalize_address(str(envelope["solver"]))
        if solver in (ZERO_ADDRESS, state.creator):
            raise RelayError("solver must be nonzero and independent from the creator")
        if state.status != STATUS_CLAIMABLE or state.solver != ZERO_ADDRESS:
            raise RelayError("bounty is not currently claimable")
        authorization = envelope["authorization"]
        assert isinstance(authorization, dict)
        valid_before = int(authorization["valid_before"])
        if valid_before <= now + 30:
            raise RelayError("claim authorization expires too soon")
        if valid_before > now + MAX_AUTHORIZATION_WINDOW_SECONDS:
            raise RelayError("claim authorization window exceeds one hour")
        return (
            "claimWithAuthorization(address,uint256,uint256,bytes32,uint8,bytes32,bytes32)",
            [
                solver,
                str(authorization["valid_after"]),
                str(valid_before),
                str(authorization["nonce"]),
                str(authorization["v"]),
                str(authorization["r"]),
                str(authorization["s"]),
            ],
        )
    if action == "submit":
        solver = normalize_address(str(envelope["solver"]))
        if state.status != STATUS_CLAIMED or state.solver != solver:
            raise RelayError("submission solver does not own the active claim")
        if int(envelope["round"]) != state.round:
            raise RelayError("submission round does not match the active round")
        deadline = int(envelope["deadline"])
        if deadline <= now + 30:
            raise RelayError("submission authorization expires too soon")
        if deadline > min(state.claim_expires_at, now + MAX_AUTHORIZATION_WINDOW_SECONDS):
            raise RelayError("submission authorization exceeds the active claim or one-hour window")
        return (
            "submitWithSignature(bytes32,bytes32,uint256,bytes)",
            [
                str(envelope["submission_hash"]),
                str(envelope["evidence_hash"]),
                str(deadline),
                str(envelope["signature"]),
            ],
        )
    if state.status != STATUS_SUBMITTED:
        raise RelayError("bounty is not awaiting deterministic verification")
    if int(envelope["round"]) != state.round:
        raise RelayError("settlement round does not match the active round")
    if state.verification_expires_at <= now + 30:
        raise RelayError("verification window expires too soon")
    proof = str(envelope["proof"])
    verification = client.call(
        state.verifier_module,
        "verify(bytes32,uint64,address,bytes32,bytes32,bytes32,bytes)(bool,bytes32)",
        state.bounty_id,
        str(state.round),
        state.solver,
        state.submission_hash,
        state.evidence_hash,
        state.policy_hash,
        proof,
        block="latest",
    ).split()
    if not verification or verification[0].lower() != "true":
        raise RelayError("deterministic verifier did not return pass; refusing rejection relay")
    return "verifyAndSettle(bytes)", [proof]


def validate_receipt(receipt: Mapping[str, object], contract: str) -> tuple[str, str]:
    status = receipt.get("status")
    if isinstance(status, str):
        status = int(status, 0)
    if status != 1:
        raise RelayError(f"relay transaction failed with receipt status {status!r}")
    transaction_hash = receipt.get("transactionHash") or receipt.get("transaction_hash")
    if not isinstance(transaction_hash, str) or not HEX_32_RE.fullmatch(transaction_hash):
        raise RelayError("relay receipt is missing a canonical transaction hash")
    to = receipt.get("to")
    if to is not None and normalize_address(str(to)) != contract:
        raise RelayError("relay receipt target does not match the bounty contract")
    block_number = receipt.get("blockNumber") or receipt.get("block_number")
    if isinstance(block_number, int) and block_number >= 0:
        block_tag = str(block_number)
    elif isinstance(block_number, str):
        parse_uint(block_number)
        block_tag = block_number
    else:
        raise RelayError("relay receipt is missing a canonical block number")
    return transaction_hash.lower(), block_tag


def validate_after(action: str, envelope: Mapping[str, object], before: BountyState, after: BountyState) -> None:
    validate_common(after, require_funded=action != "settle")
    if after.round != before.round + (1 if action == "claim" else 0):
        raise RelayError("post-transaction round mismatch")
    if action == "claim":
        if after.status != STATUS_CLAIMED or after.solver != normalize_address(str(envelope["solver"])):
            raise RelayError("confirmed claim post-state mismatch")
        if after.active_claim_bond != before.verifier_reward:
            raise RelayError("confirmed claim bond mismatch")
    elif action == "submit":
        if after.status != STATUS_SUBMITTED:
            raise RelayError("confirmed submission post-state mismatch")
        if after.submission_hash != envelope["submission_hash"] or after.evidence_hash != envelope["evidence_hash"]:
            raise RelayError("confirmed submission commitment mismatch")
    elif after.status != STATUS_SETTLED or after.funded_amount != 0:
        raise RelayError("confirmed settlement post-state mismatch")


def already_applied(action: str, envelope: Mapping[str, object], state: BountyState) -> bool:
    if action == "claim":
        return state.status in (STATUS_CLAIMED, STATUS_SUBMITTED, STATUS_SETTLED) and state.solver == normalize_address(str(envelope["solver"]))
    if action == "submit":
        return state.status in (STATUS_SUBMITTED, STATUS_SETTLED) and state.submission_hash == envelope["submission_hash"] and state.evidence_hash == envelope["evidence_hash"]
    return state.status == STATUS_SETTLED


def status_name(status: int) -> str:
    return {
        STATUS_OPEN: "open",
        STATUS_CLAIMABLE: "claimable",
        STATUS_CLAIMED: "claimed",
        STATUS_SUBMITTED: "submitted",
        STATUS_SETTLED: "settled",
    }.get(status, f"unknown-{status}")


def is_waitable_predecessor(
    action: str, envelope: Mapping[str, object], state: BountyState
) -> bool:
    if action == "claim":
        return state.status == STATUS_OPEN and state.solver == ZERO_ADDRESS
    if action == "submit":
        return (
            state.status == STATUS_CLAIMABLE
            and state.solver == ZERO_ADDRESS
            and state.round + 1 == int(envelope["round"])
        )
    return (
        state.status == STATUS_CLAIMED
        and state.round == int(envelope["round"])
        and state.solver != ZERO_ADDRESS
    )


def read_actionable_state(
    client: CastClient,
    event: RelayEvent,
    *,
    wait_seconds: int,
    poll_seconds: float,
    sleep_fn=time.sleep,
    clock=time.monotonic,
) -> tuple[BountyState, tuple[str, list[str]] | None, int]:
    action = str(event.envelope["action"])
    contract = normalize_address(str(event.envelope["bounty_contract"]))
    started = clock()
    attempts = 0
    while True:
        attempts += 1
        state = read_state(client, contract, block="latest")
        validate_common(state, require_funded=state.status != STATUS_SETTLED)
        if already_applied(action, event.envelope, state):
            return state, None, attempts
        try:
            call = action_call(client, event, state)
        except RelayError as error:
            if not is_waitable_predecessor(action, event.envelope, state):
                raise
            elapsed = max(0.0, clock() - started)
            if elapsed >= wait_seconds:
                observed = status_name(state.status)
                raise RelayError(
                    f"lifecycle state did not reach the {action} precondition within "
                    f"{wait_seconds} seconds; observed {observed} round {state.round}",
                    code="lifecycle_state_timeout",
                    retryable=True,
                    details={
                        "state_attempts": attempts,
                        "state_wait_seconds": wait_seconds,
                        "observed_status": observed,
                        "observed_round": state.round,
                    },
                ) from error
            remaining = wait_seconds - elapsed
            sleep_fn(min(max(poll_seconds, 0.01), remaining))
            continue
        return state, call, attempts


def write_json(path: pathlib.Path, value: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def render_comment(report: Mapping[str, object], comment_id: int) -> str:
    outcome = str(report.get("outcome") or "failed")
    action = str(report.get("action") or "unknown")
    contract = str(report.get("bounty_contract") or "unknown")
    lines = [
        MARKER,
        f"Source comment id: `{comment_id}`",
        f"### Autonomous gas relay: {outcome}",
        "",
        f"Action: `{action}`",
        f"Bounty contract: `{contract}`",
    ]
    if outcome == "processing":
        lines.extend(
            [
                "",
                "Request received. Keeper, chain, canonical contract, lifecycle state, proof, "
                "gas, and balance validation are in progress.",
                "This run has a bounded lifecycle wait of "
                f"{report.get('state_wait_seconds', 0)} seconds; "
                "this comment will be updated with a terminal result.",
            ]
        )
    if report.get("transaction_hash"):
        tx_hash = str(report["transaction_hash"])
        lines.extend([f"Transaction: https://basescan.org/tx/{tx_hash}"])
    if report.get("error"):
        lines.extend(["", f"Relay refused: {report['error']}"])
    if report.get("error_code"):
        lines.append(f"Failure code: `{report['error_code']}`")
    if report.get("correction"):
        lines.extend(["", f"Next step: {report['correction']}"])
    if report.get("workflow_disposition") == "handled_after_feedback":
        lines.append(
            "No transaction was broadcast. This refusal is handled only because the "
            "originating issue received this actionable feedback."
        )
    if report.get("retryable"):
        lines.append(
            "Retryable: yes. The same bounded request may be retried while its signature, "
            "round, and deadline remain valid; operator configuration errors require "
            "maintainer repair."
        )
    if report.get("observed_status"):
        lines.append(
            f"Observed lifecycle state: `{report['observed_status']}` "
            f"(round `{report.get('observed_round', 'unknown')}`)."
        )
    lines.extend(
        [
            "",
            "A signature, simulation, or transaction hash is not lifecycle or payment evidence. "
            "Wait for finalized canonical events; only `BountySettled` proves solver payment.",
            "",
        ]
    )
    return "\n".join(lines)


def publish_comment(event: RelaySource, comment: str, env: Mapping[str, str]) -> None:
    gh = shutil.which("gh") or shutil.which("gh.exe")
    if not gh:
        raise RelayError("gh is required to publish relay results")
    existing_raw = subprocess.check_output(
        [gh, "api", f"repos/{event.repository}/issues/{event.issue_number}/comments?per_page=100"],
        env=dict(env),
        text=True,
        encoding="utf-8",
        timeout=20,
    )
    existing = json.loads(existing_raw)
    source_marker = f"Source comment id: `{event.comment_id}`"
    bot_comment_id = None
    for item in existing if isinstance(existing, list) else []:
        body = str(item.get("body") or "") if isinstance(item, dict) else ""
        if MARKER in body and source_marker in body:
            bot_comment_id = item.get("id")
            break
    if bot_comment_id:
        command = [
            gh,
            "api",
            "--method",
            "PATCH",
            f"repos/{event.repository}/issues/comments/{bot_comment_id}",
            "--field",
            f"body={comment}",
        ]
    else:
        command = [
            gh,
            "api",
            "--method",
            "POST",
            f"repos/{event.repository}/issues/{event.issue_number}/comments",
            "--field",
            f"body={comment}",
        ]
    subprocess.run(
        command,
        env=dict(env),
        check=True,
        stdout=subprocess.DEVNULL,
        timeout=20,
    )


def relay(
    client: CastClient,
    event: RelayEvent,
    *,
    execute: bool,
    private_key: str | None,
    state_wait_seconds: int = DEFAULT_STATE_WAIT_SECONDS,
    state_poll_seconds: float = DEFAULT_STATE_POLL_SECONDS,
    sleep_fn=time.sleep,
    clock=time.monotonic,
) -> dict[str, object]:
    envelope = event.envelope
    action = str(envelope["action"])
    contract = normalize_address(str(envelope["bounty_contract"]))
    normalized_key: str | None = None
    keeper: str | None = None
    if execute or private_key:
        normalized_key, keeper, _ = preflight_keeper(client, private_key)
    state, call, state_attempts = read_actionable_state(
        client,
        event,
        wait_seconds=state_wait_seconds,
        poll_seconds=state_poll_seconds,
        sleep_fn=sleep_fn,
        clock=clock,
    )
    report: dict[str, object] = {
        "schema": SCHEMA,
        "outcome": "validated",
        "action": action,
        "bounty_contract": contract,
        "issue_number": event.issue_number,
        "source_comment_id": event.comment_id,
        "source_comment_author": event.comment_author,
        "execute_requested": execute,
        "lifecycle_block_tag": "latest",
        "state_attempts": state_attempts,
        "state_wait_seconds": state_wait_seconds,
        "before": asdict(state),
    }
    if keeper:
        report["keeper"] = keeper
    if call is None:
        report["outcome"] = "already_applied"
        return report
    signature, args = call
    if not normalized_key:
        report["call"] = {"function": signature, "args": args}
        return report
    assert keeper is not None
    gas_estimate = client.estimate(keeper, contract, signature, *args)
    gas_limit = gas_estimate * 125 // 100 + 10_000
    if gas_limit > GAS_CAPS[action]:
        raise RelayError(f"{action} gas limit exceeds the public relay cap")
    gas_price = client.gas_price()
    if gas_price > MAX_GAS_PRICE_WEI:
        raise RelayError("Base gas price exceeds the public relay ceiling")
    max_cost = gas_limit * gas_price
    if max_cost > MAX_GAS_COST_WEI:
        raise RelayError("estimated relay cost exceeds 0.0001 ETH")
    keeper_balance = client.balance(keeper)
    if keeper_balance < max_cost:
        raise RelayError(f"keeper {keeper} needs Base ETH for this bounded relay")
    report.update(
        {
            "keeper": keeper,
            "gas_estimate": gas_estimate,
            "gas_limit": gas_limit,
            "gas_price_wei": gas_price,
            "max_gas_cost_wei": max_cost,
            "keeper_balance_before_wei": keeper_balance,
            "call": {"function": signature, "args": args},
        }
    )
    if not execute:
        return report
    receipt = client.send(normalized_key, gas_limit, contract, signature, *args)
    transaction_hash, block_tag = validate_receipt(receipt, contract)
    after = read_state(client, contract, block=block_tag)
    validate_after(action, envelope, state, after)
    report.update(
        {
            "outcome": "relayed",
            "transaction_hash": transaction_hash,
            "basescan_url": f"https://basescan.org/tx/{transaction_hash}",
            "after": asdict(after),
        }
    )
    return report


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", type=pathlib.Path, default=os.environ.get("GITHUB_EVENT_PATH"))
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--publish", action="store_true")
    parser.add_argument("--health-check", action="store_true")
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", RPC_URL))
    parser.add_argument("--cast-bin", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--block-tag", choices=("finalized", "latest"), default="finalized")
    parser.add_argument(
        "--state-wait-seconds",
        type=int,
        default=int(os.environ.get("BASE_RELAY_STATE_WAIT_SECONDS", DEFAULT_STATE_WAIT_SECONDS)),
    )
    parser.add_argument(
        "--state-poll-seconds",
        type=float,
        default=float(os.environ.get("BASE_RELAY_STATE_POLL_SECONDS", DEFAULT_STATE_POLL_SECONDS)),
    )
    parser.add_argument(
        "--command-timeout-seconds",
        type=int,
        default=int(
            os.environ.get(
                "BASE_RELAY_COMMAND_TIMEOUT_SECONDS", DEFAULT_COMMAND_TIMEOUT_SECONDS
            )
        ),
    )
    parser.add_argument(
        "--operation-timeout-seconds",
        type=int,
        default=int(
            os.environ.get(
                "BASE_RELAY_OPERATION_TIMEOUT_SECONDS", DEFAULT_OPERATION_TIMEOUT_SECONDS
            )
        ),
    )
    parser.add_argument(
        "--report", type=pathlib.Path, default=pathlib.Path("target/autonomous-gas-relay.json")
    )
    parser.add_argument(
        "--comment", type=pathlib.Path, default=pathlib.Path("target/autonomous-gas-relay.md")
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if not 0 <= args.state_wait_seconds <= 300:
        print(
            "autonomous_gas_relay=failed error=state wait must be between 0 and 300 seconds",
            file=sys.stderr,
        )
        return 1
    if not 0.1 <= args.state_poll_seconds <= 30:
        print(
            "autonomous_gas_relay=failed error=state poll must be between 0.1 and 30 seconds",
            file=sys.stderr,
        )
        return 1
    if not 5 <= args.command_timeout_seconds <= 60:
        print(
            "autonomous_gas_relay=failed error=command timeout must be between 5 and 60 seconds",
            file=sys.stderr,
        )
        return 1
    if not 60 <= args.operation_timeout_seconds <= 480:
        print(
            "autonomous_gas_relay=failed error=operation timeout must be between 60 and 480 seconds",
            file=sys.stderr,
        )
        return 1
    if not args.event and not args.health_check:
        print("autonomous_gas_relay=failed error=GITHUB_EVENT_PATH is required", file=sys.stderr)
        return 1
    source: RelaySource | None = None
    event: RelayEvent | None = None
    request_refused = False
    try:
        if args.health_check:
            client = CastClient(
                args.cast_bin,
                args.rpc_url,
                args.block_tag,
                args.command_timeout_seconds,
                args.operation_timeout_seconds,
            )
            report = keeper_health(client, os.environ.get("BASE_KEEPER_PRIVATE_KEY"))
        else:
            source, body = parse_event_source(pathlib.Path(args.event))
            try:
                event = parse_event_request(source, body)
            except RelayError as error:
                request_refused = True
                report = {
                    "schema": SCHEMA,
                    "outcome": "refused",
                    "workflow_disposition": "handled_after_feedback",
                    "action": str(error.details.get("action") or "unknown"),
                    "bounty_contract": str(
                        error.details.get("bounty_contract") or "unknown"
                    ),
                    "issue_number": source.issue_number,
                    "source_comment_id": source.comment_id,
                    "source_comment_author": source.comment_author,
                    "error": str(error),
                    "error_code": "request_invalid",
                    "retryable": False,
                    "correction": str(
                        error.details.get("correction")
                        or f"Correct the envelope and post a new `{COMMAND}` command."
                    ),
                }
            else:
                client = CastClient(
                    args.cast_bin,
                    args.rpc_url,
                    args.block_tag,
                    args.command_timeout_seconds,
                    args.operation_timeout_seconds,
                )
                if args.publish:
                    processing = {
                        "schema": SCHEMA,
                        "outcome": "processing",
                        "action": str(event.envelope["action"]),
                        "bounty_contract": str(event.envelope["bounty_contract"]),
                        "state_wait_seconds": args.state_wait_seconds,
                    }
                    try:
                        publish_comment(
                            event,
                            render_comment(processing, event.comment_id),
                            os.environ,
                        )
                    except (
                        RelayError,
                        OSError,
                        subprocess.SubprocessError,
                        json.JSONDecodeError,
                    ) as error:
                        print(
                            "autonomous_gas_relay=warning "
                            f"error=unable to publish processing status: {error}",
                            file=sys.stderr,
                        )
                report = relay(
                    client,
                    event,
                    execute=args.execute,
                    private_key=os.environ.get("BASE_KEEPER_PRIVATE_KEY"),
                    state_wait_seconds=args.state_wait_seconds,
                    state_poll_seconds=args.state_poll_seconds,
                )
    except (RelayError, json.JSONDecodeError, OSError, ValueError) as error:
        retryable = isinstance(error, RelayError) and error.retryable
        report = {
            "schema": SCHEMA,
            "outcome": "retryable" if retryable else "failed",
            "action": str(event.envelope.get("action") if event else "unknown"),
            "bounty_contract": str(event.envelope.get("bounty_contract") if event else "unknown"),
            "error": str(error),
            "error_code": error.code if isinstance(error, RelayError) else "unexpected_input",
            "retryable": retryable,
        }
        if isinstance(error, RelayError):
            report.update(error.details)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    write_json(args.report, report)
    comment_id = source.comment_id if source else 0
    comment = render_comment(report, comment_id)
    args.comment.parent.mkdir(parents=True, exist_ok=True)
    args.comment.write_text(comment, encoding="utf-8")
    published = False
    if args.publish and source:
        try:
            publish_comment(source, comment, os.environ)
            published = True
        except (RelayError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
            print(f"autonomous_gas_relay=failed error=unable to publish result: {error}", file=sys.stderr)
            return 1
    print(
        f"autonomous_gas_relay={report['outcome']} action={report.get('action')} "
        f"contract={report.get('bounty_contract')}"
    )
    if request_refused:
        return 0 if published else 1
    return 0 if report["outcome"] in {"validated", "relayed", "already_applied", "healthy"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
