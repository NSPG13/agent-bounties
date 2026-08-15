#!/usr/bin/env python3
"""Verify that every Beta2 prover graph is pinned to the patched SP1 fork."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import tomllib


ADVISORY = "GHSA-vj64-rjf3-w3v7"
SP1_REPOSITORY = "https://github.com/NSPG13/sp1"
SP1_COMMIT = "0b729d415bf024ae425b27e6a829bed7642bbe7f"
SP1_CIRCUIT_VERSION = "agent-bounties-sp1-safe-v1"
PATCHED_PACKAGES = ("p3-challenger", "p3-field")
EXPECTED_LOCKS = (
    Path("programs/public-vector-metric-v1/Cargo.lock"),
    Path("programs/public-vector-metric-v1/program/Cargo.lock"),
)
EXPECTED_MANIFESTS = (
    Path("programs/public-vector-metric-v1/Cargo.toml"),
    Path("programs/public-vector-metric-v1/program/Cargo.toml"),
)
IDENTITY_PATH = Path("programs/public-vector-metric-v1/release-identity.json")


def _exact_git_dependency(value: object, field: str) -> None:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be a table")
    if value.get("git") != SP1_REPOSITORY or value.get("rev") != SP1_COMMIT:
        raise ValueError(f"{field} must pin {SP1_REPOSITORY}@{SP1_COMMIT}")


def _verify_manifest(path: Path) -> None:
    document = tomllib.loads(path.read_text(encoding="utf-8"))
    dependencies = document.get("workspace", {}).get("dependencies", {})
    dependencies.update(document.get("dependencies", {}))
    sp1_dependencies = {
        name: value for name, value in dependencies.items() if name.startswith("sp1-")
    }
    if not sp1_dependencies:
        raise ValueError(f"{path} has no source-pinned SP1 dependency")
    for name, value in sp1_dependencies.items():
        _exact_git_dependency(value, f"{path}:{name}")

    patches = document.get("patch", {}).get("crates-io", {})
    for package in PATCHED_PACKAGES:
        _exact_git_dependency(patches.get(package), f"{path}:patch.{package}")


def _verify_lock(path: Path) -> dict[str, str]:
    document = tomllib.loads(path.read_text(encoding="utf-8"))
    result: dict[str, str] = {}
    expected_prefix = f"git+{SP1_REPOSITORY}?rev={SP1_COMMIT}#{SP1_COMMIT}"
    for package_name in PATCHED_PACKAGES:
        matches = [
            package
            for package in document.get("package", [])
            if package.get("name") == package_name
        ]
        if len(matches) != 1:
            raise ValueError(f"{path} must contain one {package_name} package")
        source = matches[0].get("source")
        if source != expected_prefix:
            raise ValueError(
                f"{path}:{package_name} must resolve only from the patched SP1 commit"
            )
        if "checksum" in matches[0]:
            raise ValueError(f"{path}:{package_name} unexpectedly retained a registry checksum")
        result[package_name] = source
    return result


def verify(root: Path) -> dict[str, object]:
    for relative in EXPECTED_MANIFESTS:
        _verify_manifest(root / relative)

    lock_sources = {
        str(relative).replace("\\", "/"): _verify_lock(root / relative)
        for relative in EXPECTED_LOCKS
    }

    identity = json.loads((root / IDENTITY_PATH).read_text(encoding="utf-8"))
    if identity.get("sp1_commit") != SP1_COMMIT:
        raise ValueError("metric release identity does not pin the patched SP1 commit")
    if identity.get("sp1_version") != "6.4.0-agent-bounties-sp1-safe-v1":
        raise ValueError("metric release identity does not pin the patched circuit version")

    return {
        "advisory": ADVISORY,
        "status": "patched_source_graph_pinned",
        "sp1_repository": SP1_REPOSITORY,
        "sp1_commit": SP1_COMMIT,
        "circuit_version": SP1_CIRCUIT_VERSION,
        "locks": lock_sources,
        "proof_assets_required_before_activation": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    print(json.dumps(verify(args.root.resolve()), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
