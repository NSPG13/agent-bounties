#!/usr/bin/env python3
"""Settle passing leading-zero-work autonomous bounties without a relay comment.

The scheduled keeper runs in two stages, each in its own workflow job:

- ``discover`` reads the public verification-job feed, keeps submitted jobs on
  the deployed LeadingZeroWorkVerifier and mines a passing nonce from each
  job's committed values. It needs no secrets and no chain access.
- ``settle`` re-reads each bounty on-chain and relays ``verifyAndSettle``
  through the bounded relay, which first requires the module's own ``verify``
  to return pass. It holds the keeper key and the shared keeper lock.

The feed and the mined proofs are hints. The committed module and canonical
on-chain state are the only acceptance authority, and only a confirmed
``BountySettled`` event proves payment.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import sys
import time
import urllib.parse
import urllib.request
from typing import Any, Callable, Mapping, Sequence

import relay_autonomous_action as relay

SCHEMA = "agent-bounties/autonomous-auto-verify-v1"
DEFAULT_API = "https://api.agentbounties.app"
NETWORK = "base-mainnet"
MODULE = relay.LEADING_ZERO_WORK_VERIFIER_MODULE
ENGINE = "leading_zero_work_v1"
DIFFICULTY_BITS = 16
MAX_MINING_ATTEMPTS = 1 << 20
MAX_JOBS_PER_RUN = 5
MAX_ENVELOPES_BYTES = 4_096
# The settle job starts after discovery; skip jobs it could not reach in time.
MIN_REMAINING_SECONDS = 300
FEED_TIMEOUT_SECONDS = 30
MAX_FEED_BYTES = 8 * 1024 * 1024
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
OK_OUTCOMES = {"relayed", "already_applied", "skipped"}


class AutoVerifyError(RuntimeError):
    pass


def fetch_jobs(api_base: str, *, timeout: int = FEED_TIMEOUT_SECONDS) -> list[Any]:
    query = urllib.parse.urlencode({"network": NETWORK})
    request = urllib.request.Request(
        f"{api_base.rstrip('/')}/v1/base/autonomous-bounties/verification-jobs?{query}",
        headers={
            "Accept": "application/json",
            "User-Agent": "agent-bounties-auto-verify/1",
        },
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        if response.status != 200:
            raise AutoVerifyError(f"verification feed returned HTTP {response.status}")
        body = response.read(MAX_FEED_BYTES + 1)
    if len(body) > MAX_FEED_BYTES:
        raise AutoVerifyError("verification feed exceeds the size limit")
    value = json.loads(body.decode("utf-8"))
    if not isinstance(value, list):
        raise AutoVerifyError("verification feed must be an array of jobs")
    return value


def _bytes32(value: object) -> bytes:
    if not isinstance(value, str) or not BYTES32_RE.fullmatch(value):
        raise AutoVerifyError("expected a 0x-prefixed 32-byte hex value")
    return bytes.fromhex(value[2:])


def _address_word(value: object) -> bytes:
    if not isinstance(value, str) or not ADDRESS_RE.fullmatch(value):
        raise AutoVerifyError("expected a 0x-prefixed address")
    return bytes.fromhex(value[2:]).rjust(32, b"\0")


def _uint(value: object) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise AutoVerifyError("expected a non-negative integer")
    return value


def work_prefix(job: Mapping[str, Any]) -> bytes:
    """ABI-encode every LeadingZeroWorkVerifier.workHash input except the nonce."""

    evidence = job["submission_evidence"]
    terms = job["terms"]
    round_number = _uint(job["round"])
    if round_number >= 1 << 64:
        raise AutoVerifyError("round exceeds uint64")
    return b"".join(
        (
            _bytes32(job["bounty_id"]),
            round_number.to_bytes(32, "big"),
            _address_word(job["solver_wallet"]),
            _bytes32(evidence["artifact_hash"]),
            _bytes32(evidence["evidence_hash"]),
            _bytes32(terms["policy_hash"]),
        )
    )


def mine_nonce(
    prefix: bytes,
    *,
    difficulty_bits: int = DIFFICULTY_BITS,
    max_attempts: int = MAX_MINING_ATTEMPTS,
    hash_fn: Callable[[bytes], bytes] | None = None,
) -> int:
    if hash_fn is None:
        # Imported here so the keyed settle job needs only the standard library.
        from _shared.evm import keccak_bytes as hash_fn
    shift = 256 - difficulty_bits
    for nonce in range(max_attempts):
        digest = hash_fn(prefix + nonce.to_bytes(32, "big"))
        if int.from_bytes(digest, "big") >> shift == 0:
            return nonce
    raise AutoVerifyError(f"no passing nonce within {max_attempts} attempts")


def candidate_reason(job: object, now: int) -> str | None:
    """Return why a feed entry is not an auto-verification candidate, or None."""

    if not isinstance(job, dict):
        return "malformed job"
    if job.get("network") != NETWORK:
        return "network is not base-mainnet"
    if job.get("verification_mode") != "deterministic_module":
        return "not a deterministic-module job"
    module = job.get("verifier_module")
    if not isinstance(module, str) or module.lower() != MODULE:
        return "verifier module is not the leading-zero-work module"
    if job.get("threshold") != 1:
        return "deterministic verification requires threshold 1"
    terms = job.get("terms")
    document = terms.get("document") if isinstance(terms, dict) else None
    benchmark = document.get("benchmark") if isinstance(document, dict) else None
    if not isinstance(benchmark, dict):
        return "terms are missing a benchmark"
    if benchmark.get("engine") != ENGINE or benchmark.get("difficulty_bits") != DIFFICULTY_BITS:
        return "benchmark is not the 16-bit leading-zero-work engine"
    expires = job.get("verification_expires_at")
    if not isinstance(expires, int) or isinstance(expires, bool):
        return "verification deadline is unavailable"
    if expires <= now + MIN_REMAINING_SECONDS:
        return "verification window closes too soon"
    round_number = job.get("round")
    if not isinstance(round_number, int) or isinstance(round_number, bool) or round_number <= 0:
        return "round is invalid"
    contract = job.get("bounty_contract")
    if not isinstance(contract, str) or not ADDRESS_RE.fullmatch(contract):
        return "bounty contract is invalid"
    return None


def discover(
    jobs: Sequence[object],
    *,
    now: int,
    max_jobs: int = MAX_JOBS_PER_RUN,
    hash_fn: Callable[[bytes], bytes] | None = None,
) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    """Return settle envelopes for passing proofs, plus skipped feed entries."""

    candidates: list[dict[str, Any]] = []
    skipped: list[dict[str, object]] = []
    seen: set[str] = set()
    for job in jobs:
        reason = candidate_reason(job, now)
        contract = job.get("bounty_contract") if isinstance(job, dict) else None
        if reason is None and str(contract).lower() in seen:
            reason = "duplicate bounty contract"
        if reason is not None:
            if reason not in {
                "not a deterministic-module job",
                "verifier module is not the leading-zero-work module",
            }:
                skipped.append({"bounty_contract": str(contract), "reason": reason})
            continue
        assert isinstance(job, dict)
        seen.add(str(contract).lower())
        candidates.append(job)
    candidates.sort(key=lambda item: item["verification_expires_at"])
    envelopes: list[dict[str, object]] = []
    for job in candidates[:max_jobs]:
        try:
            nonce = mine_nonce(work_prefix(job), hash_fn=hash_fn)
        except (AutoVerifyError, KeyError, TypeError) as error:
            skipped.append({"bounty_contract": job["bounty_contract"], "reason": str(error)})
            continue
        envelopes.append(
            {
                "bounty_contract": relay.normalize_address(job["bounty_contract"]),
                "round": job["round"],
                "proof": "0x" + nonce.to_bytes(32, "big").hex(),
            }
        )
    for job in candidates[max_jobs:]:
        skipped.append(
            {"bounty_contract": job["bounty_contract"], "reason": "deferred to the next run"}
        )
    return envelopes, skipped


def parse_envelopes(text: str) -> list[dict[str, object]]:
    """Strictly rebuild relay settle envelopes from the discover job's output."""

    if len(text.encode("utf-8")) > MAX_ENVELOPES_BYTES:
        raise AutoVerifyError("envelope list exceeds the size limit")
    value = json.loads(text)
    if not isinstance(value, list) or len(value) > MAX_JOBS_PER_RUN:
        raise AutoVerifyError(f"envelopes must be a list of at most {MAX_JOBS_PER_RUN}")
    envelopes = []
    seen: set[str] = set()
    for item in value:
        if not isinstance(item, dict):
            raise AutoVerifyError("each envelope must be an object")
        relay.require_exact_keys(item, {"bounty_contract", "round", "proof"}, "auto envelope")
        envelope = relay.validate_envelope(
            {
                "schema": relay.SCHEMA,
                "action": "settle",
                "network": NETWORK,
                "bounty_contract": item["bounty_contract"],
                "round": item["round"],
                "proof": item["proof"],
            }
        )
        contract = str(envelope["bounty_contract"])
        if contract in seen:
            raise AutoVerifyError("envelopes must name distinct bounties")
        seen.add(contract)
        envelopes.append(envelope)
    return envelopes


def settle_one(
    client: relay.CastClient,
    envelope: Mapping[str, object],
    *,
    execute: bool,
    private_key: str | None,
) -> dict[str, object]:
    contract = str(envelope["bounty_contract"])
    state = relay.read_state(client, contract, block="latest")
    if state.factory != relay.FACTORY:
        return {
            "outcome": "skipped",
            "bounty_contract": contract,
            "reason": "bounty is not from the autonomous-v1 factory",
        }
    if state.verifier_module != MODULE:
        return {
            "outcome": "skipped",
            "bounty_contract": contract,
            "reason": "bounty does not commit the leading-zero-work module",
        }
    if state.status != relay.STATUS_SUBMITTED or state.round != envelope["round"]:
        if state.status == relay.STATUS_SETTLED:
            return {"outcome": "already_applied", "bounty_contract": contract}
        return {
            "outcome": "skipped",
            "bounty_contract": contract,
            "reason": (
                "bounty is no longer awaiting verification for this round; observed "
                f"{relay.status_name(state.status)} round {state.round}"
            ),
        }
    return relay.relay_envelope(
        client,
        envelope,
        source={"trigger": "auto_verify"},
        execute=execute,
        private_key=private_key,
        state_wait_seconds=0,
    )


def settle(
    client: relay.CastClient,
    envelopes: Sequence[Mapping[str, object]],
    *,
    execute: bool,
    private_key: str | None,
) -> list[dict[str, object]]:
    results: list[dict[str, object]] = []
    for envelope in envelopes:
        try:
            result = settle_one(client, envelope, execute=execute, private_key=private_key)
        except relay.RelayError as error:
            result = {
                "outcome": "retryable" if error.retryable else "failed",
                "bounty_contract": str(envelope["bounty_contract"]),
                "error": str(error),
                "error_code": error.code,
            }
            results.append(result)
            if error.retryable:
                # Keeper, RPC or budget trouble affects every remaining bounty.
                break
            continue
        results.append(result)
    return results


def write_json(path: pathlib.Path, value: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_github_output(path: pathlib.Path, envelopes: Sequence[Mapping[str, object]]) -> None:
    compact = json.dumps(list(envelopes), separators=(",", ":"), sort_keys=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(f"count={len(envelopes)}\n")
        handle.write(f"envelopes={compact}\n")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="stage", required=True)
    discover_parser = sub.add_parser("discover", help="find jobs and mine proofs; no secrets")
    discover_parser.add_argument("--api-base", default=os.environ.get("API_BASE_URL", DEFAULT_API))
    discover_parser.add_argument("--github-output", type=pathlib.Path)
    discover_parser.add_argument(
        "--report", type=pathlib.Path, default=pathlib.Path("target/autonomous-auto-verify-discover.json")
    )
    settle_parser = sub.add_parser("settle", help="validate on-chain and relay passing proofs")
    settle_parser.add_argument("--envelopes-env", default="AUTO_VERIFY_ENVELOPES")
    settle_parser.add_argument("--execute", action="store_true")
    settle_parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", relay.RPC_URL))
    settle_parser.add_argument("--cast-bin", default="cast")
    settle_parser.add_argument(
        "--report", type=pathlib.Path, default=pathlib.Path("target/autonomous-auto-verify.json")
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.stage == "discover":
        try:
            jobs = fetch_jobs(args.api_base)
            envelopes, skipped = discover(jobs, now=int(time.time()))
        except (AutoVerifyError, OSError, ValueError) as error:
            write_json(args.report, {"schema": SCHEMA, "stage": "discover", "outcome": "failed", "error": str(error)})
            print(f"autonomous_auto_verify=failed stage=discover error={error}", file=sys.stderr)
            return 1
        report = {
            "schema": SCHEMA,
            "stage": "discover",
            "outcome": "discovered",
            "feed_jobs": len(jobs),
            "envelopes": envelopes,
            "skipped": skipped,
        }
        write_json(args.report, report)
        if args.github_output:
            write_github_output(args.github_output, envelopes)
        print(f"autonomous_auto_verify=discovered jobs={len(jobs)} candidates={len(envelopes)}")
        return 0

    try:
        envelopes = parse_envelopes(os.environ.get(args.envelopes_env, "[]"))
        client = relay.CastClient(args.cast_bin, args.rpc_url)
        results = settle(
            client,
            envelopes,
            execute=args.execute,
            private_key=os.environ.get("BASE_KEEPER_PRIVATE_KEY"),
        )
    except (AutoVerifyError, relay.RelayError, OSError, ValueError) as error:
        write_json(args.report, {"schema": SCHEMA, "stage": "settle", "outcome": "failed", "error": str(error)})
        print(f"autonomous_auto_verify=failed stage=settle error={error}", file=sys.stderr)
        return 1
    expected = OK_OUTCOMES | ({"validated"} if not args.execute else set())
    outcome = "settled" if all(item.get("outcome") in expected for item in results) else "failed"
    write_json(args.report, {"schema": SCHEMA, "stage": "settle", "outcome": outcome, "results": results})
    relayed = sum(1 for item in results if item.get("outcome") == "relayed")
    print(f"autonomous_auto_verify={outcome} candidates={len(envelopes)} relayed={relayed}")
    return 0 if outcome == "settled" else 1


if __name__ == "__main__":
    raise SystemExit(main())
