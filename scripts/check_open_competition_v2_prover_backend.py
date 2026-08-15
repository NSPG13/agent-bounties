#!/usr/bin/env python3
"""Fail closed unless an Open Competition V2 proof backend can satisfy its proof system."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
from typing import Mapping


CPU_MINIMUM_GIB = {"groth16": 16, "plonk": 64}
NETWORK_PRIVATE_KEY_ENV = "NETWORK_PRIVATE_KEY"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--proof-system", choices=tuple(CPU_MINIMUM_GIB), required=True)
    parser.add_argument("--backend", choices=("cpu", "network"), required=True)
    parser.add_argument("--runner", type=Path)
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def _positive_limit(path: Path) -> int | None:
    try:
        raw = path.read_text(encoding="utf-8").strip()
    except OSError:
        return None
    if raw in {"", "max"}:
        return None
    try:
        value = int(raw)
    except ValueError:
        return None
    return value if value > 0 else None


def available_memory_bytes() -> int | None:
    candidates: list[int] = []
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal:"):
                candidates.append(int(line.split()[1]) * 1024)
                break
    except (OSError, ValueError, IndexError):
        pass
    for path in (
        Path("/sys/fs/cgroup/memory.max"),
        Path("/sys/fs/cgroup/memory/memory.limit_in_bytes"),
    ):
        limit = _positive_limit(path)
        if limit is not None:
            candidates.append(limit)
    return min(candidates) if candidates else None


def runner_capabilities(runner: Path) -> dict[str, object]:
    if not runner.is_file():
        raise ValueError(f"prover runner does not exist: {runner}")
    process = subprocess.run(
        [str(runner.resolve()), "--capabilities"],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if process.returncode != 0:
        raise ValueError(
            f"prover runner capability check exited {process.returncode}"
        )
    try:
        value = json.loads(process.stdout)
    except json.JSONDecodeError as error:
        raise ValueError("prover runner returned invalid capability JSON") from error
    if not isinstance(value, dict) or value.get("schema_version") != (
        "agent-bounties/open-competition-v2-prover-capabilities-v1"
    ):
        raise ValueError("prover runner capability schema mismatch")
    backends = value.get("backends")
    if not isinstance(backends, list) or not all(
        isinstance(item, str) for item in backends
    ):
        raise ValueError("prover runner capability backends are invalid")
    return value


def inspect(
    proof_system: str,
    backend: str,
    *,
    environ: Mapping[str, str] = os.environ,
    memory_bytes: int | None = None,
    capabilities: Mapping[str, object] | None = None,
) -> dict[str, object]:
    required_gib = CPU_MINIMUM_GIB[proof_system] if backend == "cpu" else None
    observed_gib = (
        round(memory_bytes / (1024**3), 2) if memory_bytes is not None else None
    )
    blockers: list[str] = []
    if backend == "cpu":
        if memory_bytes is None:
            blockers.append("V2_PROVER_MEMORY_UNKNOWN")
        elif memory_bytes < CPU_MINIMUM_GIB[proof_system] * 1024**3:
            blockers.append("V2_PROVER_MEMORY_INSUFFICIENT")
    else:
        if not environ.get(NETWORK_PRIVATE_KEY_ENV, "").strip():
            blockers.append("V2_PROVER_NETWORK_KEY_MISSING")
        backends = capabilities.get("backends") if capabilities else None
        if not isinstance(backends, list) or "network" not in backends:
            blockers.append("V2_PROVER_RUNNER_LACKS_NETWORK")
    return {
        "schema_version": "agent-bounties/open-competition-v2-prover-readiness-v1",
        "ready": not blockers,
        "proof_system": proof_system,
        "backend": backend,
        "required_memory_gib": required_gib,
        "observed_memory_gib": observed_gib,
        "network_key_configured": bool(
            environ.get(NETWORK_PRIVATE_KEY_ENV, "").strip()
        ),
        "runner_backends": capabilities.get("backends", []) if capabilities else [],
        "blockers": blockers,
        "next_action": (
            "run_prover"
            if not blockers
            else "configure SP1 Prover Network or a CPU runner meeting the published memory requirement"
        ),
    }


def main() -> int:
    args = parse_args()
    capabilities = runner_capabilities(args.runner) if args.runner else None
    report = inspect(
        args.proof_system,
        args.backend,
        memory_bytes=available_memory_bytes(),
        capabilities=capabilities,
    )
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0 if report["ready"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
