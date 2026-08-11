#!/usr/bin/env python3
"""Keep the known SP1 transcript advisory isolated from mainnet activation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import tomllib


ADVISORY = "GHSA-vj64-rjf3-w3v7"
SP1_COMMIT = "8252c2905ce32964df68248117015c61ebb854db"
PACKAGE_VERSION = "0.4.3-succinct"
PACKAGE_CHECKSUM = "b6a908924d43e4cfb93fb41c8346cac211b70314385a9037e9241f5b7f3eaf77"
EXPECTED_LOCKS = {
    Path("programs/public-vector-metric-v1/Cargo.lock"),
    Path("programs/public-vector-metric-v1/program/Cargo.lock"),
}
EXPECTED_MANIFESTS = {
    Path("programs/public-vector-metric-v1/Cargo.toml"),
    Path("programs/public-vector-metric-v1/program/Cargo.toml"),
}


def verify(root: Path) -> dict[str, object]:
    observed_locks: set[Path] = set()
    for lock in root.rglob("Cargo.lock"):
        relative = lock.relative_to(root)
        document = tomllib.loads(lock.read_text(encoding="utf-8"))
        packages = [
            package
            for package in document.get("package", [])
            if package.get("name") == "p3-challenger"
        ]
        if not packages:
            continue
        observed_locks.add(relative)
        if len(packages) != 1:
            raise ValueError(f"{relative} must contain one quarantined p3-challenger")
        package = packages[0]
        if package.get("version") != PACKAGE_VERSION:
            raise ValueError(f"{relative} changed the quarantined p3-challenger version")
        if package.get("checksum") != PACKAGE_CHECKSUM:
            raise ValueError(f"{relative} changed the quarantined p3-challenger checksum")
    if observed_locks != EXPECTED_LOCKS:
        raise ValueError(
            "p3-challenger must occur only in the two isolated V2 metric lockfiles"
        )

    pin = f'rev = "{SP1_COMMIT}"'
    for relative in EXPECTED_MANIFESTS:
        text = (root / relative).read_text(encoding="utf-8")
        if pin not in text:
            raise ValueError(f"{relative} is not pinned to the quarantined SP1 commit")

    gates_path = root / "deployments/open-competition-v2-beta1-release-gates.json"
    gates = json.loads(gates_path.read_text(encoding="utf-8"))
    if gates.get("mainnet_creation_enabled") is not False:
        raise ValueError(f"{ADVISORY} blocks V2 mainnet creation")
    if gates.get("gates", {}).get("critical_and_high_findings_resolved") is not False:
        raise ValueError(f"{ADVISORY} must remain an unresolved high-severity finding")
    if gates.get("evidence", {}).get("critical_and_high_findings_resolved") is not None:
        raise ValueError(f"{ADVISORY} cannot have resolved-finding evidence")

    return {
        "advisory": ADVISORY,
        "status": "quarantined_unresolved_high",
        "sp1_commit": SP1_COMMIT,
        "package": f"p3-challenger@{PACKAGE_VERSION}",
        "lockfiles": sorted(str(path).replace("\\", "/") for path in observed_locks),
        "mainnet_creation_enabled": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    print(json.dumps(verify(args.root.resolve()), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
