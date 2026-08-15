#!/usr/bin/env python3
"""Verify two isolated SP1 builds and emit a hash-bound metric release record."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SOURCE_FILES = (
    "crates/competition-metric-core/Cargo.toml",
    "crates/competition-metric-core/src/lib.rs",
    "programs/public-vector-metric-v1/program/Cargo.toml",
    "programs/public-vector-metric-v1/program/Cargo.lock",
    "programs/public-vector-metric-v1/program/src/main.rs",
)
JOURNAL_BYTES = 20 * 32
PROGRAM_VKEY_WORD = 9
SOURCE_HASH_WORD = 10
ELF_HASH_WORD = 11
JOURNAL_SCHEMA_WORD = 12
METRIC_PROGRAM_WORD = 13
EXPECTED_SP1_VERSION_PREFIX = "cargo-prove sp1 (f205eba "
EXPECTED_SP1_COMMIT = "f205ebada7f3bf35a71a28492ca8481aff3679ca"
IDENTITY_PATH = "programs/public-vector-metric-v1/release-identity.json"


def canonical_source_hash(root: Path) -> str:
    digest = hashlib.sha256()
    for relative in SOURCE_FILES:
        data = (root / relative).read_bytes().replace(b"\r\n", b"\n")
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(len(data)).encode("ascii"))
        digest.update(b"\0")
        digest.update(data)
    return "0x" + digest.hexdigest()


def read_evidence(path: Path) -> dict:
    lines = [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not any(line.startswith(EXPECTED_SP1_VERSION_PREFIX) for line in lines):
        raise ValueError(f"{path} did not use the pinned patched SP1 cargo-prove build")
    for line in reversed(lines):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and value.get("mode") == "execute":
            return value
    raise ValueError(f"{path} has no final SP1 execute evidence JSON object")


def bytes32(value: str, field: str) -> bytes:
    if not isinstance(value, str) or not value.startswith("0x") or len(value) != 66:
        raise ValueError(f"{field} must be bytes32 hex")
    try:
        decoded = bytes.fromhex(value[2:])
    except ValueError as error:
        raise ValueError(f"{field} must be bytes32 hex") from error
    if decoded == bytes(32):
        raise ValueError(f"{field} must be nonzero")
    return decoded


def journal(value: str) -> bytes:
    if not isinstance(value, str) or not value.startswith("0x"):
        raise ValueError("journal_hex must be 0x-prefixed hex")
    try:
        decoded = bytes.fromhex(value[2:])
    except ValueError as error:
        raise ValueError("journal_hex is malformed") from error
    if len(decoded) != JOURNAL_BYTES:
        raise ValueError(f"journal must be exactly {JOURNAL_BYTES} bytes")
    return decoded


def word(value: bytes, index: int) -> bytes:
    return value[index * 32 : (index + 1) * 32]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--first", type=Path)
    parser.add_argument("--second", type=Path)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path)
    parser.add_argument("--source-hash", action="store_true")
    args = parser.parse_args()

    if args.source_hash:
        print(canonical_source_hash(args.root))
        return 0
    if args.first is None or args.second is None or args.output is None:
        parser.error("--first, --second, and --output are required unless --source-hash is used")

    first = read_evidence(args.first)
    second = read_evidence(args.second)
    for field in ("program_vkey", "elf_keccak256", "elf_sha256", "journal_hex"):
        if first.get(field) != second.get(field):
            raise ValueError(f"isolated builds disagree on {field}")

    program_vkey = bytes32(first["program_vkey"], "program_vkey")
    elf_hash = bytes32(first["elf_keccak256"], "elf_keccak256")
    if len(first.get("elf_sha256", "")) != 64:
        raise ValueError("elf_sha256 must contain 64 lowercase hex digits")
    int(first["elf_sha256"], 16)
    public_values = journal(first["journal_hex"])
    source_hash_hex = canonical_source_hash(args.root)
    source_hash = bytes32(source_hash_hex, "source_hash")
    identity = json.loads((args.root / IDENTITY_PATH).read_text(encoding="utf-8"))
    expected_identity = {
        "program_vkey": first["program_vkey"],
        "source_hash": source_hash_hex,
        "elf_keccak256": first["elf_keccak256"],
        "elf_sha256": first["elf_sha256"],
    }
    for field, observed in expected_identity.items():
        if identity.get(field) != observed:
            raise ValueError(
                f"reproduced {field} does not match the committed metric release identity"
            )

    if word(public_values, PROGRAM_VKEY_WORD) != program_vkey:
        raise ValueError("journal program_vkey does not match the SP1 setup vkey")
    if word(public_values, SOURCE_HASH_WORD) != source_hash:
        raise ValueError("journal source_hash does not match the canonical source digest")
    if word(public_values, ELF_HASH_WORD) != elf_hash:
        raise ValueError("journal elf_hash does not match the built ELF Keccak-256")

    summary = {
        "schema": "agent-bounties/open-competition-v2-metric-review-evidence-v1",
        "profile_id": "public-vector-metric-v1",
        "sp1_release_line": "6.4.0-agent-bounties-sp1-safe-v2",
        "sp1_commit": EXPECTED_SP1_COMMIT,
        "program_vkey": first["program_vkey"],
        "source_hash": source_hash_hex,
        "elf_hash": first["elf_keccak256"],
        "elf_sha256": first["elf_sha256"],
        "journal_schema_hash": "0x" + word(public_values, JOURNAL_SCHEMA_WORD).hex(),
        "metric_program_hash": "0x" + word(public_values, METRIC_PROGRAM_WORD).hex(),
        "journal_hex": first["journal_hex"],
        "cycles": first.get("cycles"),
        "isolated_builds": 2,
    }
    review_hash = hashlib.sha256(
        json.dumps(summary, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).hexdigest()
    result = {
        **summary,
        "classification": "reviewed",
        "review_evidence_hash": "0x" + review_hash,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
