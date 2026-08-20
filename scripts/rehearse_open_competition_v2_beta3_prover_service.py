#!/usr/bin/env python3
"""Run a no-money proof through the repaired production prover service."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import time
from urllib.request import Request, urlopen
import uuid

from diagnose_open_competition_v2_beta3_prover import load_record


def post(endpoint: str, api_key: str, idempotency_key: str, payload: dict) -> dict:
    request = Request(
        endpoint,
        data=json.dumps(payload, separators=(",", ":")).encode(),
        headers={
            "authorization": f"Bearer {api_key}",
            "content-type": "application/json",
            "idempotency-key": idempotency_key,
        },
        method="POST",
    )
    with urlopen(request, timeout=45) as response:
        return json.loads(response.read())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--provider-job-id", required=True)
    parser.add_argument("--idempotency-key", required=True)
    parser.add_argument("--job-dir", type=Path, default=Path("/var/lib/agent-bounties-prover/jobs"))
    parser.add_argument("--endpoint", default="http://127.0.0.1:9070/v1/prove")
    parser.add_argument("--timeout-seconds", type=int, default=1_500)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    api_key = os.environ.get("OPEN_COMPETITION_V2_PROVER_API_KEY", "")
    if len(api_key) < 32:
        raise SystemExit("prover API key is unavailable")
    _, record = load_record(args.job_dir, args.provider_job_id)
    payload = dict(record["request"])
    payload["idempotency_key"] = args.idempotency_key
    payload["proof_job_id"] = str(uuid.uuid5(uuid.NAMESPACE_URL, args.idempotency_key))
    payload["proof_sla_deadline"] = int(time.time()) + args.timeout_seconds + 60
    deadline = time.time() + args.timeout_seconds
    response = post(args.endpoint, api_key, args.idempotency_key, payload)
    while response.get("status") == "pending" and time.time() < deadline:
        time.sleep(5)
        response = post(args.endpoint, api_key, args.idempotency_key, payload)
    proof = response.get("proof")
    public_values = response.get("public_values")
    passed = (
        response.get("status") == "proved"
        and isinstance(proof, str)
        and proof.startswith("0x")
        and isinstance(public_values, str)
        and public_values.lower() == payload["expected_public_values"].lower()
    )
    evidence = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-production-prover-rehearsal-v1",
        "passed": passed,
        "source_provider_job_id": args.provider_job_id,
        "rehearsal_provider_job_id": response.get("provider_job_id"),
        "proof_system": payload["proof_system"],
        "proof_sha256": hashlib.sha256(proof.encode()).hexdigest() if isinstance(proof, str) else None,
        "public_values_match": isinstance(public_values, str)
        and public_values.lower() == payload["expected_public_values"].lower(),
        "failure_code": response.get("failure_code"),
        "failure_message": response.get("failure_message"),
        "money_moved": False,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"passed": passed, "provider_job_id": response.get("provider_job_id")}))
    if not passed:
        raise SystemExit("production prover rehearsal failed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
