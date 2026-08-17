#!/usr/bin/env python3
"""Canonicalize SP1's zero-prefixed BN254 VK root for Solidity bytes32."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


VK_ROOT = re.compile(
    r"(function\s+VK_ROOT\(\)\s+public\s+pure\s+returns\s+\(bytes32\)\s*\{\s*"
    r"return\s+0x)([0-9a-fA-F]+)(;\s*\})",
    re.MULTILINE,
)


def normalize(path: Path) -> str:
    source = path.read_text(encoding="utf-8")
    matches = list(VK_ROOT.finditer(source))
    if len(matches) != 1:
        raise ValueError(f"{path}: expected exactly one bytes32 VK_ROOT function")

    encoded = matches[0].group(2).lower()
    if len(encoded) == 64:
        canonical = encoded
    elif len(encoded) == 66 and encoded.startswith("00"):
        canonical = encoded[2:]
        if int(encoded, 16) != int(canonical, 16):
            raise ValueError(f"{path}: VK_ROOT normalization changed its value")
    else:
        raise ValueError(
            f"{path}: VK_ROOT must be 32 bytes or one zero-prefixed 33-byte value"
        )

    rewritten = VK_ROOT.sub(
        lambda match: f"{match.group(1)}{canonical}{match.group(3)}", source, count=1
    )
    if len(canonical) != 64 or not re.fullmatch(r"[0-9a-f]{64}", canonical):
        raise ValueError(f"{path}: canonical VK_ROOT is not bytes32")
    path.write_text(rewritten, encoding="utf-8")
    return f"0x{canonical}"


def normalize_all(paths: list[Path]) -> dict[str, object]:
    if not paths:
        raise ValueError("at least one verifier source is required")
    roots = {str(path): normalize(path) for path in paths}
    if len(set(roots.values())) != 1:
        raise ValueError("Groth16 and PLONK verifier VK roots differ")
    return {
        "schema_version": "agent-bounties/sp1-solidity-vk-root-normalization-v1",
        "status": "canonical_bytes32",
        "vk_root": next(iter(roots.values())),
        "sources": roots,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("sources", type=Path, nargs="+")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    evidence = normalize_all(args.sources)
    rendered = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")


if __name__ == "__main__":
    main()
