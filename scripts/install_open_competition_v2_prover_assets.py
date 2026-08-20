#!/usr/bin/env python3
"""Install hash-verified Beta3 setup assets in SP1's versioned runtime layout."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile


SCHEMA = "agent-bounties/open-competition-v2-beta3-trusted-setup-v1"
SYSTEM_FILES = {
    "constraint_system_sha256": "{system}_circuit.bin",
    "proving_key_sha256": "{system}_pk.bin",
    "verifying_key_sha256": "{system}_vk.bin",
    "transcript_sha256": "transcript.json",
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_system(root: Path, manifest: dict, system: str) -> dict[str, str]:
    record = manifest["proof_systems"].get(system)
    if not isinstance(record, dict) or record.get("verification_passed") is not True:
        raise ValueError(f"{system} trusted setup is not verified")
    hashes: dict[str, str] = {}
    for field, template in SYSTEM_FILES.items():
        path = root / system / template.format(system=system)
        expected = record.get(field)
        actual = sha256(path)
        if not isinstance(expected, str) or actual != expected:
            raise ValueError(f"{system} {field} mismatch")
        hashes[path.name] = actual
    return hashes


def install(trusted_root: Path, install_root: Path, circuit_version: str) -> dict:
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,79}", circuit_version):
        raise ValueError("circuit version is invalid")
    manifest_path = trusted_root / "trusted-setup.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != SCHEMA or manifest.get("circuit_version") != circuit_version:
        raise ValueError("trusted setup manifest identity mismatch")

    installed: dict[str, dict] = {}
    for system in ("groth16", "plonk"):
        hashes = verify_system(trusted_root, manifest, system)
        base = install_root / system
        destination = base / circuit_version
        if destination.exists():
            if not (destination / ".complete").is_file():
                raise ValueError(f"existing {system} runtime assets are incomplete")
            for name, expected in hashes.items():
                if sha256(destination / name) != expected:
                    raise ValueError(f"existing {system} runtime asset hash mismatch")
        else:
            base.mkdir(parents=True, exist_ok=True)
            temporary = Path(tempfile.mkdtemp(prefix=f".{circuit_version}.", dir=base))
            try:
                for source in (trusted_root / system).iterdir():
                    target = temporary / source.name
                    shutil.copytree(source, target) if source.is_dir() else shutil.copy2(source, target)
                (temporary / ".complete").touch()
                for path in temporary.rglob("*"):
                    path.chmod(0o755 if path.is_dir() else 0o644)
                os.replace(temporary, destination)
            finally:
                if temporary.exists():
                    shutil.rmtree(temporary)
        installed[system] = {
            "base_path": str(base),
            "versioned_path": str(destination),
            "hashes": hashes,
        }
    return {
        "schema_version": "agent-bounties/open-competition-v2-beta3-prover-assets-v1",
        "passed": True,
        "circuit_version": circuit_version,
        "trusted_setup_manifest_sha256": sha256(manifest_path),
        "proof_systems": installed,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trusted-root", type=Path, required=True)
    parser.add_argument("--install-root", type=Path, required=True)
    parser.add_argument("--circuit-version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = install(args.trusted_root.resolve(), args.install_root.resolve(), args.circuit_version)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "install_root": str(args.install_root)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
