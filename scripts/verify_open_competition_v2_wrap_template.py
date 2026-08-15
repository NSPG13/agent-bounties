#!/usr/bin/env python3
"""Verify that the pinned SP1 wrap key and template proof are one hash-bound pair."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SCHEMA = "agent-bounties/sp1-wrap-template-v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(source_root: Path, expected_version: str) -> dict[str, str]:
    source_root = source_root.resolve()
    prover_root = source_root / "crates/prover"
    manifest_path = prover_root / "wrap-template-manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    circuit_version = (source_root / "SP1_CIRCUIT_VERSION").read_text(
        encoding="utf-8"
    ).strip()

    if manifest.get("schema") != SCHEMA:
        raise ValueError("wrap template manifest schema is not recognized")
    if circuit_version != expected_version:
        raise ValueError("SP1 circuit version does not match the release")
    if manifest.get("circuit_version") != circuit_version:
        raise ValueError("wrap template was generated for a different circuit version")

    files = {
        "wrap_vk_sha256": prover_root / "wrap_vk.bin",
        "wrapped_proof_sha256": prover_root / "wrapped_proof.bin",
    }
    for field, path in files.items():
        if manifest.get(field) != sha256(path):
            raise ValueError(f"{field} does not match the pinned binary")

    elf_hash = manifest.get("template_elf_sha256", "")
    if len(elf_hash) != 64 or any(character not in "0123456789abcdef" for character in elf_hash):
        raise ValueError("template ELF hash must be lowercase SHA-256")

    generator = prover_root / "scripts/regenerate_wrap_template.rs"
    generator_source = generator.read_text(encoding="utf-8")
    required_generator_fragments = (
        "expected_elf_sha256",
        "template ELF hash mismatch",
        "generated wrap template has a stale recursion-vkey root",
        "generated wrap template is not bound to the template guest vkey",
        "wrap-template-manifest.json",
    )
    if any(fragment not in generator_source for fragment in required_generator_fragments):
        raise ValueError("wrap template generator is not hash-bound")

    return {
        "schema": "agent-bounties/open-competition-v2-wrap-template-check-v1",
        "status": "hash_bound",
        "circuit_version": circuit_version,
        "template_elf_sha256": elf_hash,
        "wrap_vk_sha256": manifest["wrap_vk_sha256"],
        "wrapped_proof_sha256": manifest["wrapped_proof_sha256"],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source_root", type=Path)
    parser.add_argument("--expected-version", required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.source_root, args.expected_version), sort_keys=True))


if __name__ == "__main__":
    main()
