#!/usr/bin/env python3
"""Materialize the reviewed MoonPay integration payload used by this temporary branch."""

from __future__ import annotations

import base64
import io
import tarfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
PARTS = ROOT / "scripts/.moonpay-payload"
ALLOWED_PREFIXES = (
    "crates/mcp-server/src/moonpay.rs",
    "docs/moonpay-onramp.md",
    "scripts/apply-moonpay-integration.py",
    "scripts/check-moonpay-onramp.py",
    "site/moonpay-link.js",
    "site/moonpay-onramp.js",
    "site/onramp.css",
    "site/onramp.html",
)


def main() -> int:
    parts = sorted(PARTS.glob("part-*.txt"))
    if not parts:
        raise SystemExit("MoonPay payload parts are missing")
    encoded = "".join(part.read_text(encoding="utf-8").strip() for part in parts)
    archive = base64.b64decode(encoded, validate=True)
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as bundle:
        members = bundle.getmembers()
        names = {member.name for member in members}
        if names != set(ALLOWED_PREFIXES):
            raise SystemExit(f"MoonPay payload inventory changed: {sorted(names)}")
        for member in members:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or not member.isfile():
                raise SystemExit(f"Unsafe MoonPay payload member: {member.name}")
        bundle.extractall(ROOT, filter="data")
    print(f"Materialized {len(members)} reviewed MoonPay integration files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
