#!/usr/bin/env python3
"""Build, independently sign, and relay closed solver-leaderboard awards."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any


CANDIDATE_SCHEMA = "agent-bounties/leaderboard-reward-candidate-v1"
ATTESTATION_SCHEMA = "agent-bounties/leaderboard-reward-attestation-v1"
MANIFEST_SCHEMA = "agent-bounties/leaderboard-reward-manifest-v1"
EVIDENCE_SCHEMA = "agent-bounties/leaderboard-reward-evidence-v1"
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")
SIGNATURE = re.compile(r"^0x[0-9a-f]{130}$")
ZERO_ADDRESS = "0x" + "0" * 40
BASE_MAINNET_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
FINALIZATION_DELAY = timedelta(hours=1)
PERIODS = {
    "daily": {"kind": 0, "reward": 3_000_000},
    "weekly": {"kind": 1, "reward": 26_000_000},
}


class PipelineError(RuntimeError):
    pass


class NoCandidate(RuntimeError):
    pass


def canonical_json(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def normalize_address(value: object, field: str) -> str:
    normalized = str(value or "").strip().lower()
    if not ADDRESS.fullmatch(normalized):
        raise PipelineError(f"{field} must be an EVM address")
    return normalized


def normalize_hash(value: object, field: str) -> str:
    normalized = str(value or "").strip().lower()
    if not HASH.fullmatch(normalized):
        raise PipelineError(f"{field} must be a bytes32 value")
    return normalized


def parse_utc(value: object, field: str) -> datetime:
    text = str(value or "")
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError as error:
        raise PipelineError(f"{field} must be RFC3339") from error
    if parsed.tzinfo is None:
        raise PipelineError(f"{field} must include a UTC offset")
    return parsed.astimezone(timezone.utc)


def run(command: list[str], *, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        env=env,
        timeout=300,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()[:800]
        raise PipelineError(f"command failed closed: {detail}")
    return completed.stdout.strip()


def fetch_json(url: str, timeout: float = 30) -> Any:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": "agent-bounties-leaderboard-finalizer/1",
        },
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        if response.status != 200:
            raise PipelineError(f"leaderboard returned HTTP {response.status}")
        return json.loads(response.read().decode("utf-8"))


def utc_text(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def period_references(now: datetime) -> dict[str, datetime]:
    now = now.astimezone(timezone.utc)
    today = now.replace(hour=0, minute=0, second=0, microsecond=0)
    this_week = today - timedelta(days=today.weekday())
    return {
        "daily": today - timedelta(seconds=1),
        "weekly": this_week - timedelta(seconds=1),
    }


def fetch_leaderboard(
    api_base: str, network: str, reference: datetime
) -> dict[str, Any]:
    query = urllib.parse.urlencode({"network": network, "at": utc_text(reference)})
    value = fetch_json(
        f"{api_base.rstrip('/')}/v1/base/autonomous-bounties/leaderboard?{query}"
    )
    if not isinstance(value, dict):
        raise PipelineError("leaderboard response must be an object")
    return value


def ranking_evidence(
    response: dict[str, Any], period_name: str, contract: str
) -> dict[str, Any]:
    if response.get("schema_version") != "agent-bounties/solver-leaderboard-v1":
        raise PipelineError("leaderboard schema is unsupported")
    network = str(response.get("network", ""))
    period_output = response.get(period_name)
    if not isinstance(period_output, dict):
        raise PipelineError(f"{period_name} leaderboard is missing")
    ranking = period_output.get("ranking")
    if not isinstance(ranking, dict):
        raise PipelineError(f"{period_name} ranking is missing")
    period = ranking.get("period")
    entries = ranking.get("entries")
    rules = ranking.get("rules")
    if not isinstance(period, dict) or not isinstance(entries, list) or not isinstance(rules, list):
        raise PipelineError(f"{period_name} ranking shape is invalid")
    if period.get("kind") != period_name:
        raise PipelineError(f"{period_name} period kind drifted")

    normalized_entries = []
    for entry in entries:
        if not isinstance(entry, dict):
            raise PipelineError("leaderboard entry must be an object")
        normalized_entries.append(
            {
                "rank": int(entry.get("rank", 0)),
                "solver_wallet": normalize_address(
                    entry.get("solver_wallet"), "solver wallet"
                ),
                "completed_bounties": int(entry.get("completed_bounties", 0)),
                "prize_eligible_bounties": int(
                    entry.get("prize_eligible_bounties", 0)
                ),
                "excluded_bounties": int(entry.get("excluded_bounties", 0)),
                "eligible_solver_rewards_usdc_base_units": str(
                    entry.get("eligible_solver_rewards_usdc_base_units", "")
                ),
                "last_eligible_settlement_at": entry.get(
                    "last_eligible_settlement_at"
                ),
                "exclusion_counts": entry.get("exclusion_counts", {}),
            }
        )

    leader = normalize_address(ranking.get("leader_wallet"), "leader wallet")
    return {
        "schema": EVIDENCE_SCHEMA,
        "network": network,
        "reward_contract": contract,
        "period": {
            "kind": period_name,
            "starts_at": utc_text(parse_utc(period.get("starts_at"), "period start")),
            "ends_at": utc_text(parse_utc(period.get("ends_at"), "period end")),
        },
        "minimum_solver_reward_usdc_base_units": str(
            ranking.get("minimum_solver_reward_usdc_base_units", "")
        ),
        "reward_usdc_base_units": str(ranking.get("reward_usdc_base_units", "")),
        "leader_wallet": leader,
        "entries": normalized_entries,
        "rules": [str(rule) for rule in rules],
    }


def build_candidate(
    response: dict[str, Any],
    period_name: str,
    contract: str,
    reference: datetime,
    now: datetime,
) -> dict[str, Any]:
    settings = PERIODS[period_name]
    contract = normalize_address(contract, "configured reward contract")
    pool = response.get("reward_pool")
    period_output = response.get(period_name)
    if not isinstance(pool, dict) or not isinstance(period_output, dict):
        raise PipelineError("reward pool or period output is missing")
    if normalize_address(pool.get("contract"), "API reward contract") != contract:
        raise PipelineError("API reward contract does not match the configured contract")
    if normalize_address(period_output.get("reward_contract"), "period reward contract") != contract:
        raise PipelineError("period reward contract does not match the configured contract")
    ranking = period_output.get("ranking")
    if not isinstance(ranking, dict):
        raise PipelineError("period ranking is missing")
    if ranking.get("leader_wallet") is None:
        raise NoCandidate("period has no eligible winner")

    evidence = ranking_evidence(response, period_name, contract)
    starts_at = parse_utc(evidence["period"]["starts_at"], "period start")
    ends_at = parse_utc(evidence["period"]["ends_at"], "period end")
    if now.astimezone(timezone.utc) < ends_at + FINALIZATION_DELAY:
        raise NoCandidate("period is not final")
    if int(evidence["reward_usdc_base_units"]) != settings["reward"]:
        raise PipelineError("reward amount drifted")
    if period_output.get("period_status") != "closed":
        raise PipelineError("finalized period is not closed")

    payout_status = str(period_output.get("reward_payout_status", ""))
    paid_wallet = period_output.get("reward_paid_wallet")
    if payout_status == "paid":
        raise NoCandidate("award is already paid")
    if payout_status == "paid_to_different_wallet" or paid_wallet is not None:
        raise PipelineError("award paid-winner evidence conflicts with the ranking")
    if payout_status == "payout_unverified":
        raise NoCandidate("paid-winner state is not verified")

    balance = int(pool.get("balance_usdc_base_units") or 0)
    if balance < settings["reward"]:
        raise NoCandidate("reward pool balance is insufficient")
    leader = evidence["leader_wallet"]
    leader_entries = [
        entry
        for entry in evidence["entries"]
        if entry["solver_wallet"] == leader and entry["prize_eligible_bounties"] > 0
    ]
    if len(leader_entries) != 1 or leader_entries[0]["rank"] != 1:
        raise PipelineError("leader is not one exact eligible rank-one entry")
    completions = leader_entries[0]["prize_eligible_bounties"]
    evidence_hash = "0x" + hashlib.sha256(canonical_json(evidence).encode()).hexdigest()
    starts_at_unix = int(starts_at.timestamp())
    return {
        "schema": CANDIDATE_SCHEMA,
        "candidate_id": hashlib.sha256(
            f"{contract}:{settings['kind']}:{starts_at_unix}".encode()
        ).hexdigest(),
        "network": str(response.get("network", "")),
        "reference_at": utc_text(reference),
        "reward_contract": contract,
        "period_name": period_name,
        "period_kind": settings["kind"],
        "starts_at": starts_at_unix,
        "ends_at": int(ends_at.timestamp()),
        "winner": leader,
        "eligible_completions": completions,
        "reward_usdc_base_units": settings["reward"],
        "evidence_hash": evidence_hash,
        "evidence": evidence,
    }


def current_candidate(
    args: argparse.Namespace, candidate: dict[str, Any]
) -> dict[str, Any]:
    reference = parse_utc(candidate.get("reference_at"), "candidate reference")
    response = fetch_leaderboard(args.api_base, args.network, reference)
    return build_candidate(
        response,
        str(candidate.get("period_name", "")),
        args.contract,
        reference,
        datetime.now(timezone.utc),
    )


def assert_candidate_unchanged(
    expected: dict[str, Any], current: dict[str, Any]
) -> None:
    if canonical_json(expected) != canonical_json(current):
        raise PipelineError("closed-period ranking changed; regenerate the candidate")


def contract_call(args: argparse.Namespace, signature: str, *values: str) -> str:
    output = run(
        [
            str(args.cast),
            "call",
            "--json",
            "--rpc-url",
            args.rpc_url,
            args.contract,
            signature,
            *values,
        ]
    )
    try:
        decoded = json.loads(output)
    except json.JSONDecodeError as error:
        raise PipelineError("cast call did not return JSON") from error
    if not isinstance(decoded, list) or len(decoded) != 1:
        raise PipelineError("cast call must return one value")
    value = decoded[0]
    if isinstance(value, bool):
        return str(value).lower()
    if not isinstance(value, (int, str)):
        raise PipelineError("cast call returned an unsupported value")
    return str(value).strip().lower()


def chain_int(value: str, field: str) -> int:
    try:
        parsed = int(value.strip(), 0)
    except ValueError as error:
        raise PipelineError(f"{field} is not an integer") from error
    if parsed < 0:
        raise PipelineError(f"{field} cannot be negative")
    return parsed


def validate_contract(args: argparse.Namespace) -> set[str]:
    token = normalize_address(
        contract_call(args, "settlementToken()(address)"), "settlement token"
    )
    expected_token = normalize_address(args.expected_token, "expected settlement token")
    if args.network != "base-mainnet" or token != expected_token:
        raise PipelineError("leaderboard contract is not pinned to Base mainnet USDC")
    return {
        normalize_address(contract_call(args, "signerA()(address)"), "signer A"),
        normalize_address(contract_call(args, "signerB()(address)"), "signer B"),
    }


def contract_period_starts(args: argparse.Namespace) -> dict[str, int]:
    return {
        "daily": chain_int(
            contract_call(args, "firstDailyStart()(uint64)"), "first daily start"
        ),
        "weekly": chain_int(
            contract_call(args, "firstWeeklyStart()(uint64)"), "first weekly start"
        ),
    }


def award_digest(args: argparse.Namespace, candidate: dict[str, Any]) -> str:
    return normalize_hash(
        contract_call(
            args,
            "awardDigest(uint8,uint64,address,uint32,bytes32)(bytes32)",
            str(candidate["period_kind"]),
            str(candidate["starts_at"]),
            candidate["winner"],
            str(candidate["eligible_completions"]),
            candidate["evidence_hash"],
        ),
        "award digest",
    )


def command_run(args: argparse.Namespace) -> None:
    now = datetime.now(timezone.utc)
    references = period_references(now)
    validate_contract(args)
    first_starts = contract_period_starts(args)
    cache: dict[str, dict[str, Any]] = {}
    candidates = []
    skipped = []
    args.output.mkdir(parents=True, exist_ok=True)
    for period_name, reference in references.items():
        key = utc_text(reference)
        if key not in cache:
            cache[key] = fetch_leaderboard(args.api_base, args.network, reference)
        response = cache[key]
        try:
            candidate = build_candidate(
                response, period_name, args.contract, reference, now
            )
        except NoCandidate as reason:
            skipped.append({"period": period_name, "reason": str(reason)})
            continue
        if candidate["starts_at"] < first_starts[period_name]:
            skipped.append({"period": period_name, "reason": "period predates program"})
            continue
        name = f"candidate-{candidate['candidate_id']}.json"
        write_json(args.output / name, candidate)
        candidates.append({"candidate_id": candidate["candidate_id"], "file": name})
    write_json(
        args.output / "manifest.json",
        {
            "schema": MANIFEST_SCHEMA,
            "network": args.network,
            "reward_contract": normalize_address(args.contract, "reward contract"),
            "candidates": candidates,
            "skipped": skipped,
        },
    )


def command_sign(args: argparse.Namespace) -> None:
    key = os.environ.get(args.private_key_env, "").strip()
    if not key:
        raise PipelineError(f"{args.private_key_env} is required")
    signer = normalize_address(
        run([str(args.cast), "wallet", "address", "--private-key", key]), "signer"
    )
    if signer != normalize_address(args.expected_signer, "expected signer"):
        raise PipelineError("private key does not match the expected signer")
    if signer not in validate_contract(args):
        raise PipelineError("signer is not committed by the reward contract")

    manifest = read_json(args.candidates / "manifest.json")
    if manifest.get("schema") != MANIFEST_SCHEMA:
        raise PipelineError("candidate manifest schema is invalid")
    args.output.mkdir(parents=True, exist_ok=True)
    attestations = []
    for entry in manifest.get("candidates", []):
        candidate = read_json(args.candidates / entry["file"])
        current = current_candidate(args, candidate)
        assert_candidate_unchanged(candidate, current)
        digest = award_digest(args, candidate)
        signature = run(
            [
                str(args.cast),
                "wallet",
                "sign",
                "--no-hash",
                "--private-key",
                key,
                digest,
            ]
        ).lower()
        if not SIGNATURE.fullmatch(signature):
            raise PipelineError("signer returned an invalid signature")
        name = f"attestation-{candidate['candidate_id']}.json"
        write_json(
            args.output / name,
            {
                "schema": ATTESTATION_SCHEMA,
                "candidate_id": candidate["candidate_id"],
                "signer": signer,
                "digest": digest,
                "signature": signature,
            },
        )
        attestations.append({"candidate_id": candidate["candidate_id"], "file": name})
    write_json(
        args.output / "manifest.json",
        {
            "schema": ATTESTATION_SCHEMA,
            "signer": signer,
            "attestations": attestations,
        },
    )


def command_relay(args: argparse.Namespace) -> None:
    keeper = os.environ.get(args.keeper_key_env, "").strip()
    if not keeper:
        raise PipelineError(f"{args.keeper_key_env} is required")
    expected_signers = {
        normalize_address(value, "expected signer") for value in args.expected_signer
    }
    if len(expected_signers) != 2 or validate_contract(args) != expected_signers:
        raise PipelineError("workflow signers do not match the reward contract")
    candidate_manifest = read_json(args.candidates / "manifest.json")
    by_signer: dict[str, dict[str, Path]] = {}
    for directory in args.attestations:
        manifest = read_json(directory / "manifest.json")
        signer = normalize_address(manifest.get("signer"), "attestation signer")
        if signer in by_signer:
            raise PipelineError("duplicate attestation signer")
        by_signer[signer] = {
            item["candidate_id"]: directory / item["file"]
            for item in manifest.get("attestations", [])
        }
    if set(by_signer) != expected_signers:
        raise PipelineError("attestation artifacts do not contain both committed signers")

    for entry in candidate_manifest.get("candidates", []):
        candidate = read_json(args.candidates / entry["file"])
        current = current_candidate(args, candidate)
        assert_candidate_unchanged(candidate, current)
        digest = award_digest(args, candidate)
        signed = []
        for signer in sorted(expected_signers):
            attestation = read_json(by_signer[signer][candidate["candidate_id"]])
            if (
                attestation.get("schema") != ATTESTATION_SCHEMA
                or attestation.get("candidate_id") != candidate["candidate_id"]
                or normalize_address(attestation.get("signer"), "attestation signer")
                != signer
                or normalize_hash(attestation.get("digest"), "attestation digest")
                != digest
                or not SIGNATURE.fullmatch(str(attestation.get("signature", "")))
            ):
                raise PipelineError("attestation does not match the exact candidate")
            signed.append(attestation["signature"])

        award_id = normalize_hash(
            contract_call(
                args,
                "awardId(uint8,uint64)(bytes32)",
                str(candidate["period_kind"]),
                str(candidate["starts_at"]),
            ),
            "award id",
        )
        paid = normalize_address(
            contract_call(args, "paidAwardWinner(bytes32)(address)", award_id),
            "paid award winner",
        )
        if paid == candidate["winner"]:
            print(f"already paid {candidate['candidate_id']} to {paid}")
            continue
        if paid != ZERO_ADDRESS:
            raise PipelineError("award is already paid to a different wallet")

        receipt = json.loads(
            run(
                [
                    str(args.cast),
                    "send",
                    "--json",
                    "--rpc-url",
                    args.rpc_url,
                    "--private-key",
                    keeper,
                    args.contract,
                    "pay(uint8,uint64,address,uint32,bytes32,bytes,bytes)",
                    str(candidate["period_kind"]),
                    str(candidate["starts_at"]),
                    candidate["winner"],
                    str(candidate["eligible_completions"]),
                    candidate["evidence_hash"],
                    signed[0],
                    signed[1],
                ]
            )
        )
        tx_hash = normalize_hash(receipt.get("transactionHash"), "transaction hash")
        if str(receipt.get("status", "")) not in {"0x1", "1"}:
            raise PipelineError("reward relay did not return a successful receipt")
        confirmed = normalize_address(
            contract_call(args, "paidAwardWinner(bytes32)(address)", award_id),
            "confirmed paid winner",
        )
        if confirmed != candidate["winner"]:
            raise PipelineError("successful receipt lacks the expected paid-winner state")
        print(f"paid {candidate['candidate_id']}: {tx_hash}")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    run_parser = commands.add_parser("run")
    run_parser.add_argument("--api-base", default="https://api.bountyboard.global")
    run_parser.add_argument("--network", default="base-mainnet")
    run_parser.add_argument("--rpc-url", required=True)
    run_parser.add_argument("--contract", required=True)
    run_parser.add_argument("--expected-token", default=BASE_MAINNET_USDC)
    run_parser.add_argument("--output", type=Path, required=True)
    run_parser.add_argument("--cast", type=Path, default=Path("cast"))
    run_parser.set_defaults(handler=command_run)

    sign_parser = commands.add_parser("sign")
    sign_parser.add_argument("--api-base", default="https://api.bountyboard.global")
    sign_parser.add_argument("--network", default="base-mainnet")
    sign_parser.add_argument("--rpc-url", required=True)
    sign_parser.add_argument("--contract", required=True)
    sign_parser.add_argument("--expected-token", default=BASE_MAINNET_USDC)
    sign_parser.add_argument("--candidates", type=Path, required=True)
    sign_parser.add_argument("--output", type=Path, required=True)
    sign_parser.add_argument("--cast", type=Path, default=Path("cast"))
    sign_parser.add_argument("--private-key-env", required=True)
    sign_parser.add_argument("--expected-signer", required=True)
    sign_parser.set_defaults(handler=command_sign)

    relay_parser = commands.add_parser("relay")
    relay_parser.add_argument("--api-base", default="https://api.bountyboard.global")
    relay_parser.add_argument("--network", default="base-mainnet")
    relay_parser.add_argument("--rpc-url", required=True)
    relay_parser.add_argument("--contract", required=True)
    relay_parser.add_argument("--expected-token", default=BASE_MAINNET_USDC)
    relay_parser.add_argument("--candidates", type=Path, required=True)
    relay_parser.add_argument("--attestations", type=Path, action="append", required=True)
    relay_parser.add_argument("--expected-signer", action="append", required=True)
    relay_parser.add_argument("--cast", type=Path, default=Path("cast"))
    relay_parser.add_argument("--keeper-key-env", default="BASE_KEEPER_PRIVATE_KEY")
    relay_parser.set_defaults(handler=command_relay)
    return root


def main() -> int:
    try:
        args = parser().parse_args()
        args.contract = normalize_address(args.contract, "reward contract")
        args.handler(args)
    except (PipelineError, NoCandidate, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"leaderboard reward pipeline failed: {error}", file=os.sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
