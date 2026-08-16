#!/usr/bin/env python3
"""Wait for the hosted Beta3 API to expose an exact, reconciled runtime."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen
from typing import Any


class RuntimeWaitError(RuntimeError):
    pass


IDENTITY_KEYS = (
    "protocol_version",
    "network",
    "source_commit",
    "repository_subject_hash",
    "factory_contract",
    "factory_runtime_code_hash",
    "implementation_contract",
    "implementation_runtime_code_hash",
    "settlement_token",
    "groth16_verifier",
    "groth16_verifier_hash",
    "groth16_verifier_runtime_code_hash",
    "plonk_verifier",
    "plonk_verifier_hash",
    "plonk_verifier_runtime_code_hash",
    "deployment_block",
    "release_hash",
    "beta_risk_hash",
)


def exact_runtime(response: dict[str, Any], expected: dict[str, Any], broker: bool, public: bool) -> bool:
    release = response.get("release")
    if not isinstance(release, dict) or not isinstance(response.get("indexer_agreement"), dict):
        return False
    if any(release.get(key) != expected.get(key) for key in IDENTITY_KEYS):
        return False
    return (
        release.get("proof_broker_enabled") is broker
        and release.get("public_creation_enabled") is public
        and response["indexer_agreement"].get("agrees") is True
        and response["indexer_agreement"].get("factory_contract", "").lower()
        == expected["factory_contract"].lower()
    )


def fetch(url: str) -> dict[str, Any]:
    separator = "&" if "?" in url else "?"
    request = Request(
        f"{url}{separator}{urlencode({'network': 'base-mainnet', '_probe': time.time_ns()})}",
        headers={"accept": "application/json", "cache-control": "no-store"},
    )
    with urlopen(request, timeout=20) as response:
        return json.loads(response.read())


def wait(url: str, expected: dict[str, Any], broker: bool, public: bool, timeout: float, poll: float) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        try:
            last = fetch(url)
            if exact_runtime(last, expected, broker, public):
                return last
        except (HTTPError, URLError, TimeoutError, json.JSONDecodeError):
            pass
        time.sleep(poll)
    raise RuntimeWaitError(f"hosted runtime did not reconcile before timeout; last={last}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="https://api.agentbounties.app/v1/base/open-competition-v2-beta3/release")
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--expect-broker", choices=("true", "false"), required=True)
    parser.add_argument("--expect-public", choices=("true", "false"), required=True)
    parser.add_argument("--timeout-seconds", type=float, default=900)
    parser.add_argument("--poll-seconds", type=float, default=10)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    expected = json.loads(args.runtime.read_text(encoding="utf-8"))
    result = wait(
        args.url,
        expected,
        args.expect_broker == "true",
        args.expect_public == "true",
        args.timeout_seconds,
        args.poll_seconds,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
