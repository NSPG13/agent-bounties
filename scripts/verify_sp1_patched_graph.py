#!/usr/bin/env python3
"""Verify that every Beta2 prover graph is pinned to the patched SP1 fork."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import tomllib


ADVISORY = "GHSA-vj64-rjf3-w3v7"
SP1_REPOSITORY = "https://github.com/NSPG13/sp1"
SP1_COMMIT = "caf43bb80fab6745347fda83bb428cb08a463f8d"
SP1_CIRCUIT_VERSION = "agent-bounties-sp1-safe-v4"
PATCHED_PACKAGES = ("p3-challenger",)
P3_FIELD_VERSION = "0.4.3-succinct"
P3_FIELD_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
P3_FIELD_CHECKSUM = "3dc75969ca3ac847f43e632ab979d59ff7a68f9eac8dbf8edcbba47fc2e1d3aa"
GUEST_RUST_MIN_VERSION = "1.94"
GUEST_RUST_TOOLCHAIN_VERSION = "1.94.0-dev"
HOST_RUST_TOOLCHAIN_VERSION = "1.96.1"
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
    package = document.get("workspace", {}).get("package", {}) or document.get("package", {})
    if package.get("rust-version") != GUEST_RUST_MIN_VERSION:
        raise ValueError(
            f"{path} must pin SP1 guest rust-version {GUEST_RUST_MIN_VERSION}"
        )
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
    if "p3-field" in patches:
        raise ValueError(f"{path}:patch.p3-field must not replace the canonical registry package")


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
    field_matches = [
        package
        for package in document.get("package", [])
        if package.get("name") == "p3-field"
    ]
    if len(field_matches) != 1:
        raise ValueError(f"{path} must contain one canonical p3-field package")
    field = field_matches[0]
    if (
        field.get("version") != P3_FIELD_VERSION
        or field.get("source") != P3_FIELD_SOURCE
        or field.get("checksum") != P3_FIELD_CHECKSUM
    ):
        raise ValueError(
            f"{path}:p3-field must be canonical {P3_FIELD_VERSION} with the pinned checksum"
        )
    result["p3-field"] = field["source"]
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
    if identity.get("sp1_version") != "6.4.0-agent-bounties-sp1-safe-v4":
        raise ValueError("metric release identity does not pin the patched circuit version")
    if identity.get("rust_version") != HOST_RUST_TOOLCHAIN_VERSION:
        raise ValueError("metric release identity does not pin the host Rust toolchain")
    if identity.get("sp1_guest_rust_version") != GUEST_RUST_TOOLCHAIN_VERSION:
        raise ValueError("metric release identity does not pin the SP1 guest Rust toolchain")

    return {
        "advisory": ADVISORY,
        "status": "patched_source_graph_pinned",
        "sp1_repository": SP1_REPOSITORY,
        "sp1_commit": SP1_COMMIT,
        "circuit_version": SP1_CIRCUIT_VERSION,
        "host_rust_toolchain": HOST_RUST_TOOLCHAIN_VERSION,
        "sp1_guest_rust_toolchain": GUEST_RUST_TOOLCHAIN_VERSION,
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
