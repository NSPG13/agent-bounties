#!/usr/bin/env python3
"""Register a GitHub participant identity for the standing-meta protocol."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


SCHEMA = "agent-bounties/participant-registration-v1"
COMMAND = re.compile(r"^/agent-bounty register (0x[0-9a-fA-F]{40})\s*$")
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")


class RegistrationError(RuntimeError):
    def __init__(self, message: str, evidence: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.evidence = evidence or {}


@dataclass(frozen=True)
class RegistrationRequest:
    repository: str
    issue_number: int
    github_login: str
    github_user_id: int
    wallet: str


def parse_event(value: object, expected_repository: str) -> RegistrationRequest:
    if not isinstance(value, dict) or value.get("action") not in {"created", "edited"}:
        raise RegistrationError("only created or edited issue comments can register participants")
    repository = value.get("repository")
    comment = value.get("comment")
    issue = value.get("issue")
    sender = value.get("sender")
    if not all(isinstance(item, dict) for item in (repository, comment, issue, sender)):
        raise RegistrationError("GitHub event is missing registration scope")
    full_name = str(repository.get("full_name", ""))
    if full_name.lower() != expected_repository.lower():
        raise RegistrationError("registration event belongs to a different repository")
    if issue.get("pull_request") is not None:
        raise RegistrationError("participant registration must be requested on an issue")
    match = COMMAND.fullmatch(str(comment.get("body", "")))
    if match is None:
        raise RegistrationError("registration command must be `/agent-bounty register 0xWallet`")
    issue_number = issue.get("number")
    user_id = sender.get("id")
    login = str(sender.get("login", ""))
    if (
        not isinstance(issue_number, int)
        or issue_number <= 0
        or not isinstance(user_id, int)
        or user_id <= 0
        or not re.fullmatch(r"[A-Za-z0-9-]{1,39}", login)
    ):
        raise RegistrationError("GitHub participant identity is invalid")
    return RegistrationRequest(
        repository=full_name,
        issue_number=issue_number,
        github_login=login,
        github_user_id=user_id,
        wallet=match.group(1).lower(),
    )


def run(command: list[str]) -> str:
    completed = subprocess.run(command, capture_output=True, text=True, timeout=120, check=False)
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()[:600]
        raise RegistrationError(f"participant transaction failed closed: {detail}")
    return completed.stdout.strip()


def registration_cutoff(
    record: object,
    participant_id: str,
    source_hash: str,
    valid_until: int,
) -> int:
    if not isinstance(record, list) or len(record) != 4:
        raise RegistrationError("participant registry returned an invalid record")
    stored_participant, stored_source, registered_at, stored_valid_until = record
    if (
        str(stored_participant).lower() != participant_id
        or str(stored_source).lower() != source_hash
        or not isinstance(registered_at, int)
        or registered_at <= 0
        or stored_valid_until != valid_until
        or registered_at >= stored_valid_until
    ):
        raise RegistrationError("participant registry stored a different identity or validity window")
    return registered_at + 1


def validate_eligibility(value: object, participant_id: str, source_hash: str) -> None:
    if (
        not isinstance(value, list)
        or len(value) != 3
        or str(value[0]).lower() != participant_id
        or str(value[1]).lower() != source_hash
        or value[2] is not True
    ):
        raise RegistrationError("participant registry did not confirm eligibility")


def register(args: argparse.Namespace, request: RegistrationRequest) -> dict[str, Any]:
    registry = str(args.registry).lower()
    if not ADDRESS.fullmatch(registry):
        raise RegistrationError("participant registry address is unavailable")
    attester_key = os.environ.get("PARTICIPANT_ATTESTER_PRIVATE_KEY", "").strip()
    keeper_key = os.environ.get("BASE_KEEPER_PRIVATE_KEY", "").strip()
    if not attester_key or not keeper_key:
        raise RegistrationError("participant attester and keeper capabilities are required")
    cast = str(args.cast)
    if run([cast, "chain-id", "--rpc-url", args.rpc_url]) != "8453":
        raise RegistrationError("participant registration is pinned to Base mainnet")
    attester = run([cast, "wallet", "address", "--private-key", attester_key]).lower()
    configured = run([cast, "call", "--rpc-url", args.rpc_url, registry, "attester()(address)"]).lower()
    if attester != configured:
        raise RegistrationError("attester key does not match the immutable registry")
    participant_id = run(
        [cast, "keccak", f"agent-bounties/github-user-v1:{request.github_user_id}"]
    ).lower()
    source_hash = run([cast, "keccak", "agent-bounties/github-user-id"]).lower()
    nonce = run(
        [cast, "call", "--rpc-url", args.rpc_url, registry, "nonces(address)(uint256)", request.wallet]
    )
    valid_until = int(time.time()) + 30 * 24 * 60 * 60
    digest = run(
        [
            cast,
            "call",
            "--rpc-url",
            args.rpc_url,
            registry,
            "attestationDigest(address,bytes32,bytes32,uint64,uint256)(bytes32)",
            request.wallet,
            participant_id,
            source_hash,
            str(valid_until),
            nonce,
        ]
    ).lower()
    signature = run(
        [cast, "wallet", "sign", "--no-hash", "--private-key", attester_key, digest]
    ).lower()
    receipt = json.loads(
        run(
            [
                cast,
                "send",
                "--json",
                "--rpc-url",
                args.rpc_url,
                "--private-key",
                keeper_key,
                registry,
                "register(address,bytes32,bytes32,uint64,bytes)",
                request.wallet,
                participant_id,
                source_hash,
                str(valid_until),
                signature,
            ]
        )
    )
    transaction_hash = str(receipt.get("transactionHash", "")).lower()
    if not HASH.fullmatch(transaction_hash) or str(receipt.get("status", "")) not in {"1", "0x1"}:
        raise RegistrationError("participant registration did not return a successful receipt")
    receipt_evidence = {
        "repository": request.repository,
        "issue_number": request.issue_number,
        "github_login": request.github_login,
        "github_user_id": request.github_user_id,
        "wallet": request.wallet,
        "participant_id": participant_id,
        "source_hash": source_hash,
        "registry": registry,
        "valid_until": valid_until,
        "transaction_hash": transaction_hash,
    }
    last_error: RegistrationError | ValueError | None = None
    for attempt in range(6):
        try:
            record = json.loads(
                run(
                    [
                        cast,
                        "call",
                        "--json",
                        "--rpc-url",
                        args.rpc_url,
                        registry,
                        "participants(address)(bytes32,bytes32,uint64,uint64)",
                        request.wallet,
                    ]
                )
            )
            cutoff = registration_cutoff(record, participant_id, source_hash, valid_until)
            eligibility = json.loads(
                run(
                    [
                        cast,
                        "call",
                        "--json",
                        "--rpc-url",
                        args.rpc_url,
                        registry,
                        "eligibleAt(address,uint64)(bytes32,bytes32,bool)",
                        request.wallet,
                        str(cutoff),
                    ]
                )
            )
            validate_eligibility(eligibility, participant_id, source_hash)
            break
        except (RegistrationError, ValueError) as error:
            last_error = error
            if attempt < 5:
                time.sleep(1)
    else:
        assert last_error is not None
        raise RegistrationError(str(last_error), receipt_evidence) from last_error
    return {
        "schema": SCHEMA,
        "success": True,
        **receipt_evidence,
        "eligibility_cutoff": cutoff,
    }


def markdown(evidence: dict[str, Any]) -> str:
    if evidence.get("success"):
        return (
            f"Participant registration confirmed for `@{evidence['github_login']}` and "
            f"`{evidence['wallet']}`.\n\n"
            f"Base transaction: `https://basescan.org/tx/{evidence['transaction_hash']}`\n\n"
            "This source-scoped identity is used only to enforce independent participation in "
            "standing meta-bounties. It is not payment, claim, completion, or payout evidence."
        )
    receipt = evidence.get("transaction_hash")
    if receipt:
        return (
            "Participant registration transaction succeeded, but the confirmation read failed closed. "
            "Do not resend the command until a maintainer checks the stored record.\n\n"
            f"Base transaction: `https://basescan.org/tx/{receipt}`\n\n"
            f"Confirmation error: {evidence.get('error', 'unknown error')}"
        )
    return (
        "Participant registration was not completed. The request failed closed: "
        f"{evidence.get('error', 'unknown error')}\n\n"
        "Use exactly `/agent-bounty register 0xYourBaseWallet` and retry after correcting the stated issue."
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", type=Path, required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--registry", required=True)
    parser.add_argument("--rpc-url", default="https://mainnet.base.org")
    parser.add_argument("--cast", type=Path, default=Path("cast"))
    parser.add_argument("--json-out", type=Path, required=True)
    parser.add_argument("--md-out", type=Path, required=True)
    args = parser.parse_args()
    evidence: dict[str, Any] = {"schema": SCHEMA, "success": False, "error": None}
    try:
        request = parse_event(json.loads(args.event.read_text(encoding="utf-8")), args.repository)
        evidence = register(args, request)
    except (RegistrationError, OSError, ValueError, json.JSONDecodeError) as error:
        if isinstance(error, RegistrationError):
            evidence.update(error.evidence)
        evidence["error"] = str(error)[:600]
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.md_out.write_text(markdown(evidence) + "\n", encoding="utf-8")
    return 0 if evidence.get("success") else 1


if __name__ == "__main__":
    raise SystemExit(main())
