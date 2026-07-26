#!/usr/bin/env python3
"""Reject framework leakage and dependency drift in the vanilla Coinbase wallet bundle."""

from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "coinbase-wallet-adapter"
BUNDLE = ROOT / "site" / "coinbase-embedded-wallet.bundle.js"


def fail(message: str) -> None:
    raise SystemExit(message)


def main() -> int:
    package = json.loads((TOOL / "package.json").read_text(encoding="utf-8"))
    dependencies = package.get("dependencies", {})
    dev_dependencies = package.get("devDependencies", {})
    if dependencies.get("@coinbase/cdp-core") != "0.0.118":
        fail("@coinbase/cdp-core must remain exactly pinned")
    if dependencies.get("viem") != "2.55.8":
        fail("viem must remain exactly pinned")
    if dev_dependencies.get("esbuild") != "0.28.1":
        fail("esbuild must remain exactly pinned")
    if "react" in dependencies or "react" in dev_dependencies:
        fail("the vanilla adapter must not add React solely for an optional peer")
    if "--external:react" not in package.get("scripts", {}).get("build", ""):
        fail("the browser build must externalize Coinbase's optional React peer")

    lock = json.loads((TOOL / "package-lock.json").read_text(encoding="utf-8"))
    root_lock = lock.get("packages", {}).get("", {})
    if root_lock.get("dependencies") != dependencies:
        fail("package-lock direct dependencies do not match package.json")
    if root_lock.get("devDependencies") != dev_dependencies:
        fail("package-lock dev dependencies do not match package.json")

    bundle = BUNDLE.read_text(encoding="utf-8")
    if len(bundle.encode("utf-8")) < 20_000:
        fail("the Coinbase production bundle was not generated")
    if re.search(r"(?:__require|require)\([\"']react[\"']\)", bundle):
        fail("the generated vanilla bundle retained an unresolved React runtime dependency")
    if re.search(r"(?:from|import)\s*[\"']react[\"']", bundle):
        fail("the generated vanilla bundle retained a React module import")

    print("Coinbase vanilla browser bundle checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
