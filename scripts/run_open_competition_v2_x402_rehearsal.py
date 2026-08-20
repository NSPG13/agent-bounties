#!/usr/bin/env python3
"""Exercise a real Beta3 x402 purchase, proof job, relay and settlement."""

from __future__ import annotations

import argparse
import base64
import json
import os
from pathlib import Path
import secrets
import subprocess
import time
from typing import Any
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from eth_account.messages import encode_typed_data

import run_open_competition_v2_sepolia_rehearsal as sepolia
from _shared.evm import keccak256


class X402RehearsalError(RuntimeError):
    pass


TRANSFER_TYPES = {
    "TransferWithAuthorization": [
        {"name": "from", "type": "address"},
        {"name": "to", "type": "address"},
        {"name": "value", "type": "uint256"},
        {"name": "validAfter", "type": "uint256"},
        {"name": "validBefore", "type": "uint256"},
        {"name": "nonce", "type": "bytes32"},
    ]
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise X402RehearsalError(message)


def request_json(
    method: str,
    url: str,
    payload: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
    expected: tuple[int, ...] = (200,),
) -> tuple[int, dict[str, Any], dict[str, str]]:
    body = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
    request_headers = {"accept": "application/json", **(headers or {})}
    if body is not None:
        request_headers["content-type"] = "application/json"
    request = Request(url, data=body, headers=request_headers, method=method)
    try:
        response = urlopen(request, timeout=45)
        status = response.status
        raw = response.read()
        response_headers = {key.lower(): value for key, value in response.headers.items()}
    except HTTPError as error:
        status = error.code
        raw = error.read()
        response_headers = {key.lower(): value for key, value in error.headers.items()}
    value = json.loads(raw) if raw else {}
    if status not in expected:
        raise X402RehearsalError(f"{method} {url} returned HTTP {status}: {value}")
    return status, value, response_headers


def decode_x402_header(value: str) -> dict[str, Any]:
    try:
        decoded = base64.b64decode(value, validate=True)
        result = json.loads(decoded)
    except (ValueError, json.JSONDecodeError) as error:
        raise X402RehearsalError("x402 header is not canonical base64 JSON") from error
    require(isinstance(result, dict), "x402 header must contain an object")
    return result


def sign_payment(
    actor: Any,
    challenge: dict[str, Any],
    *,
    chain_id: int = 84532,
    network: str = "eip155:84532",
) -> str:
    accepts = challenge.get("accepts")
    require(isinstance(accepts, list) and len(accepts) == 1, "x402 challenge is ambiguous")
    accepted = accepts[0]
    require(accepted.get("scheme") == "exact", "proof broker did not use standard exact x402")
    require(accepted.get("network") == network, "x402 challenge is for the wrong Base network")
    extra = accepted.get("extra", {})
    require(extra.get("assetTransferMethod") == "eip3009", "x402 challenge omitted EIP-3009")
    now = int(time.time())
    valid_before = now + min(int(accepted["maxTimeoutSeconds"]), 240)
    nonce = secrets.token_bytes(32)
    authorization = {
        "from": actor.address,
        "to": accepted["payTo"],
        "value": int(accepted["amount"]),
        "validAfter": 0,
        "validBefore": valid_before,
        "nonce": nonce,
    }
    message = encode_typed_data(
        full_message={
            "types": {
                "EIP712Domain": [
                    {"name": "name", "type": "string"},
                    {"name": "version", "type": "string"},
                    {"name": "chainId", "type": "uint256"},
                    {"name": "verifyingContract", "type": "address"},
                ],
                **TRANSFER_TYPES,
            },
            "primaryType": "TransferWithAuthorization",
            "domain": {
                "name": extra["name"],
                "version": extra["version"],
                "chainId": chain_id,
                "verifyingContract": accepted["asset"],
            },
            "message": authorization,
        }
    )
    signed = actor.sign_message(message)
    payment_payload = {
        "x402Version": challenge["x402Version"],
        "resource": challenge["resource"],
        "accepted": accepted,
        "payload": {
            "signature": "0x" + bytes(signed.signature).hex(),
            "authorization": {
                "from": actor.address,
                "to": accepted["payTo"],
                "value": str(accepted["amount"]),
                "validAfter": "0",
                "validBefore": str(valid_before),
                "nonce": "0x" + nonce.hex(),
            },
        },
    }
    if challenge.get("extensions") is not None:
        payment_payload["extensions"] = challenge["extensions"]
    return base64.b64encode(
        json.dumps(payment_payload, separators=(",", ":")).encode()
    ).decode()


def sign_relay_authorization(actor: Any, authorization: dict[str, Any]) -> str:
    require(
        authorization.get("primaryType") == "SubmitProof",
        "relay authorization has the wrong EIP-712 primary type",
    )
    require(
        isinstance(authorization.get("types"), dict)
        and isinstance(authorization.get("domain"), dict)
        and isinstance(authorization.get("message"), dict),
        "relay authorization is not complete EIP-712 typed data",
    )
    signed = actor.sign_message(encode_typed_data(full_message=authorization))
    return "0x" + bytes(signed.signature).hex()


def run_worker(binary: Path, protocol: str) -> str:
    environment = {**os.environ, "BASE_INDEXER_PROTOCOL": protocol, "BASE_INDEXER_ONCE": "true"}
    completed = subprocess.run(
        [str(binary), "--once"],
        check=True,
        capture_output=True,
        text=True,
        env=environment,
        timeout=660,
    )
    return completed.stdout.strip().splitlines()[-1] if completed.stdout.strip() else ""


def get_job(api: str, job_id: str) -> dict[str, Any]:
    _, value, _ = request_json(
        "GET", f"{api}/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}"
    )
    return value["job"]


def build_quote_payload(spec: dict[str, Any], network: str) -> dict[str, Any]:
    return {
        "network": network,
        "competition_contract": spec["competition"],
        "solver": spec["solver"],
        "solver_nonce": spec["solver_nonce"],
        "artifact_hash": spec["artifact_hash"],
        "relay": True,
        "metric": spec["metric"],
    }


def validate_resumable_job(
    job: dict[str, Any], spec: dict[str, Any], network: str
) -> None:
    expected = {
        "network": network,
        "competition_contract": str(spec["competition"]).lower(),
        "solver": str(spec["solver"]).lower(),
        "solver_nonce": str(spec["solver_nonce"]),
        "artifact_hash": str(spec["artifact_hash"]).lower(),
        "proof_system": str(spec["proof_system"]),
    }
    for field, value in expected.items():
        actual = job.get(field)
        if field in {"competition_contract", "solver", "artifact_hash"}:
            actual = str(actual).lower()
        elif field == "solver_nonce":
            actual = str(actual)
        require(actual == value, f"resumed proof job {field} differs from the canary")
    require(job.get("requested_relay") is True, "resumed proof job did not request relay")
    require(
        job.get("state") in {"paid", "proving", "proved", "relaying", "submitted", "confirmed"},
        f"proof job is not resumable from {job.get('state')}",
    )
    require(
        isinstance(job.get("payment_evidence"), dict),
        "resumed proof job lacks canonical payment evidence",
    )
    program = job.get("program_input")
    metric = spec.get("metric")
    require(isinstance(program, dict) and isinstance(metric, dict), "metric binding is missing")
    require(program.get("mode") == metric.get("mode"), "resumed proof job metric mode differs")
    require(
        str(program.get("threshold")) == str(metric.get("threshold")),
        "resumed proof job threshold differs",
    )
    require(program.get("vectors") == metric.get("vectors"), "resumed proof job vectors differ")
    require(
        isinstance(job.get("expected_public_values"), str)
        and str(job["expected_public_values"]).startswith("0x"),
        "resumed proof job lacks its bound public values",
    )
    if spec.get("expected_public_values") is not None:
        require(
            str(job["expected_public_values"]).lower()
            == str(spec["expected_public_values"]).lower(),
            "resumed proof job public values differ from the canary",
        )


def reconcile_payment(
    payment_url: str,
    deadline: float,
    *,
    payment_signature: str | None = None,
) -> dict[str, Any]:
    headers = {"PAYMENT-SIGNATURE": payment_signature} if payment_signature else None
    status, payment, _ = request_json(
        "POST", payment_url, headers=headers, expected=(200, 202, 503)
    )
    while status in {202, 503} and time.time() < deadline:
        time.sleep(2)
        status, payment, _ = request_json(
            "POST", payment_url, expected=(200, 202, 503)
        )
    require(status == 200, "x402 payment did not reconcile canonically")
    return payment


def wait_for_state(
    api: str,
    worker: Path | None,
    job_id: str,
    expected: set[str],
    deadline: float,
) -> dict[str, Any]:
    while time.time() < deadline:
        if worker is not None:
            run_worker(worker, "open-competition-v2-broker")
        job = get_job(api, job_id)
        if job["state"] in expected:
            return job
        if job["state"] in {"refund_due", "refunded", "lost_competition"}:
            raise X402RehearsalError(f"proof job entered {job['state']}: {job}")
        time.sleep(2)
    raise X402RehearsalError(f"proof job did not reach {sorted(expected)} before timeout")


def wait_for_refund(
    api: str,
    worker: Path,
    job_id: str,
    deadline: float,
) -> dict[str, Any]:
    while time.time() < deadline:
        run_worker(worker, "open-competition-v2-broker")
        job = get_job(api, job_id)
        if job["state"] == "refunded":
            return job
        if job["state"] in {"proved", "relaying", "confirmed", "lost_competition"}:
            raise X402RehearsalError(f"failure canary escaped into {job['state']}: {job}")
        time.sleep(2)
    raise X402RehearsalError("proof job was not canonically refunded before timeout")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", required=True)
    parser.add_argument("--network", choices=("base-sepolia", "base-mainnet"), default="base-sepolia")
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--rehearsal", type=Path, required=True)
    parser.add_argument("--worker-binary", type=Path)
    parser.add_argument("--hosted-workers", action="store_true")
    parser.add_argument("--expect-refund", action="store_true")
    parser.add_argument("--proof-job-id")
    parser.add_argument("--private-key-env", default="BASE_SEPOLIA_DEPLOYER_PRIVATE_KEY")
    parser.add_argument("--actor-derivation-salt", default="local")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, default=1_800)
    args = parser.parse_args()

    sepolia.configure_network(args)
    chain_id = 8453 if args.network == "base-mainnet" else 84532
    eip155 = f"eip155:{chain_id}"

    rehearsal = json.loads(args.rehearsal.read_text(encoding="utf-8"))
    spec = rehearsal.get("x402_canary")
    require(isinstance(spec, dict) and spec.get("active") is True, "x402 canary is unavailable")
    byo = rehearsal.get("byo_proof_submission")
    require(isinstance(byo, dict) and byo.get("transaction_hash"), "BYO proof evidence is missing")
    root_key = sepolia.normalized_key(os.environ.get(args.private_key_env, ""))
    derivation_id = sepolia.actor_derivation_id(
        rehearsal["source_commit"], args.actor_derivation_salt
    )
    require(
        rehearsal.get("actor_derivation_id") == derivation_id,
        "actor derivation identity differs from the prepared canary",
    )
    solver = sepolia.derived_actor(
        root_key,
        rehearsal["source_commit"],
        "solver-a",
        args.actor_derivation_salt,
    )
    require(solver.address.lower() == spec["solver"], "derived solver differs from canary")
    require(
        args.hosted_workers != bool(args.worker_binary),
        "choose exactly one of --worker-binary or --hosted-workers",
    )
    worker = args.worker_binary.resolve() if args.worker_binary else None
    require(worker is None or worker.is_file(), "worker binary is unavailable")
    require(not (args.hosted_workers and args.expect_refund), "refund canary requires an isolated worker")
    require(not (args.proof_job_id and args.expect_refund), "refund canary cannot resume a paid job")
    api = args.api.rstrip("/")
    deadline = time.time() + args.timeout_seconds

    if worker is not None:
        run_worker(worker, "open-competition-v2-beta3")
        run_worker(worker, "open-competition-v2-shadow")
    payment_started = time.time()
    quote_id = None
    if args.proof_job_id:
        job_id = args.proof_job_id
        resumed = get_job(api, job_id)
        validate_resumable_job(resumed, spec, args.network)
        payment_url = f"{api}/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/payment"
        payment = reconcile_payment(payment_url, deadline)
    else:
        quote_payload = build_quote_payload(spec, args.network)
        _, quote, _ = request_json(
            "POST", f"{api}/v1/base/open-competition-v2-beta3/proof-quotes", quote_payload
        )
        job_id = quote["proof_job_id"]
        quote_id = quote["quote"]["quote_id"]
        challenge = quote["payment_required"]
        payment_url = f"{api}/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/payment"
        status, _, headers = request_json("POST", payment_url, expected=(402,))
        require(status == 402 and "payment-required" in headers, "unsigned request did not return 402")
        require(decode_x402_header(headers["payment-required"]) == challenge, "402 challenge drifted")
        payment_signature = sign_payment(
            solver, challenge, chain_id=chain_id, network=eip155
        )
        payment = reconcile_payment(
            payment_url, deadline, payment_signature=payment_signature
        )
    payment_evidence = payment.get("payment_evidence")
    require(isinstance(payment_evidence, dict), "canonical x402 payment evidence is missing")

    if args.expect_refund:
        require(worker is not None, "refund canary requires a worker")
        job = wait_for_refund(api, worker, job_id, deadline)
        refund_evidence = job.get("refund_evidence")
        require(isinstance(refund_evidence, dict), "canonical refund evidence is missing")
        result = {
            "schema_version": "agent-bounties/open-competition-v2-beta3-x402-refund-rehearsal-v1",
            "passed": True,
            "network": args.network,
            "source_commit": rehearsal["source_commit"],
            "competition": spec["competition"],
            "solver": spec["solver"],
            "actor_derivation_id": derivation_id,
            "generated_agent_wallet": True,
            "manual_state_corrections": 0,
            "quote_id": quote_id,
            "proof_job_id": job_id,
            "standard_exact": True,
            "payment_transaction": payment_evidence["transaction_hash"],
            "refund_transaction": job["refund_tx_hash"],
            "refund_evidence": refund_evidence,
            "failure_code": job["failure_code"],
            "refunded_within_seconds": int(time.time() - payment_started),
            "evidence_boundary": "Canonical Base USDC payment and refund evidence prove this forced broker-failure canary; no competition settlement occurred.",
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({"passed": True, "proof_job_id": job_id, "output": str(args.output)}))
        return 0

    job = wait_for_state(
        api, worker, job_id, {"proved", "relaying", "submitted", "confirmed"}, deadline
    )
    if job["state"] == "proved":
        require(job.get("proof") and job.get("public_values"), "broker did not persist a bound proof")
        authorization_deadline = min(int(spec["proof_deadline"]), int(time.time()) + 600)
        relay_url = f"{api}/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/relay-authorization"
        _, unsigned, _ = request_json(
            "POST",
            relay_url,
            {"authorization_deadline": authorization_deadline, "solver_signature": None},
        )
        signature = sign_relay_authorization(
            solver, unsigned["plan"]["relay_authorization"]
        )
        _, authorized, _ = request_json(
            "POST",
            relay_url,
            {
                "authorization_deadline": authorization_deadline,
                "solver_signature": signature,
            },
        )
        require(authorized["state"] == "relaying", "solver relay authorization was not accepted")

    while time.time() < deadline:
        if worker is not None:
            run_worker(worker, "open-competition-v2-broker")
        time.sleep(2)
        if worker is not None:
            run_worker(worker, "open-competition-v2-beta3")
            run_worker(worker, "open-competition-v2-shadow")
            run_worker(worker, "open-competition-v2-broker")
        job = get_job(api, job_id)
        if job["state"] == "confirmed":
            break
        if job["state"] in {"refund_due", "refunded", "lost_competition"}:
            raise X402RehearsalError(f"relay failed into {job['state']}: {job}")
        time.sleep(2)
    require(job["state"] == "confirmed", "x402 proof relay did not settle")
    require(job.get("settlement_event_id"), "canonical settlement event is missing")

    client = sepolia.SignedRpc(args.rpc_url)
    token = payment_evidence["asset"]
    require(
        sepolia.rehearsal.token_balance(client.url, token, spec["competition"]) == 0,
        "x402 competition retained USDC after settlement",
    )
    residual = sepolia.rehearsal.token_balance(client.url, token, solver.address)
    reclaim = None
    if residual:
        reclaim_receipt = client.send(
            solver,
            to=token,
            data=sepolia.rehearsal.function_data(
                "transfer(address,uint256)",
                ["address", "uint256"],
                [rehearsal["actors"]["deployer"], residual],
            ),
        )
        reclaim = sepolia.receipt_hash(reclaim_receipt)

    result = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-x402-rehearsal-v1",
        "passed": True,
        "network": args.network,
        "source_commit": rehearsal["source_commit"],
        "competition": spec["competition"],
        "solver": spec["solver"],
        "actor_derivation_id": derivation_id,
        "generated_agent_wallet": True,
        "manual_state_corrections": 0,
        "quote_id": quote_id,
        "proof_job_id": job_id,
        "standard_exact": True,
        "eip3009": True,
        "payment_transaction": payment_evidence["transaction_hash"],
        "payment_block": payment_evidence["block_number"],
        "relay_transaction": job["relay_tx_hash"],
        "settlement_event_id": job["settlement_event_id"],
        "byo_proof_transaction": byo["transaction_hash"],
        "solver_test_usdc_reclaimed_transaction": reclaim,
        "proof_hash": job["proof_hash"],
        "public_values_hash": job["public_values_hash"],
        "evidence_hash": keccak256(
            json.dumps(
                {
                    "payment": payment_evidence,
                    "settlement_event_id": job["settlement_event_id"],
                    "byo": byo,
                },
                sort_keys=True,
                separators=(",", ":"),
            ).encode()
        ),
        "evidence_boundary": f"Only the attached canonical {args.network} USDC payment and CompetitionSettledV2 identifiers prove this synthetic x402 rehearsal.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "proof_job_id": job_id, "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
