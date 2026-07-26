#!/usr/bin/env python3
"""Inject public wallet-provider deployment configuration into static browser assets."""

from __future__ import annotations

import argparse
import os
import re
from pathlib import Path

PLACEHOLDER = "__COINBASE_CDP_PROJECT_ID__"
CANONICAL_REPOSITORY = "NSPG13/agent-bounties"
CANONICAL_PROJECT_ID = "9dfed88a-0b37-47e8-b867-96f1dfd0d4ee"
PROJECT_ID = re.compile(r"^[A-Za-z0-9_-]{8,128}$")


def configured_default() -> str:
    supplied = os.getenv("COINBASE_CDP_PROJECT_ID", "").strip()
    if supplied:
        return supplied
    if os.getenv("GITHUB_ACTIONS", "").lower() == "true" and os.getenv("GITHUB_REPOSITORY") == CANONICAL_REPOSITORY:
        return CANONICAL_PROJECT_ID
    return ""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--site-root", default="site")
    parser.add_argument("--coinbase-project-id", default=configured_default())
    parser.add_argument("--allow-disabled", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    site_root = Path(args.site_root).resolve()
    config_path = site_root / "wallet-config.js"
    if not config_path.exists():
        raise SystemExit(f"wallet provider config is missing: {config_path}")

    project_id = args.coinbase_project_id.strip()
    if not project_id:
        if args.allow_disabled:
            print("Coinbase embedded wallet remains disabled: no COINBASE_CDP_PROJECT_ID was supplied")
            return 0
        raise SystemExit("COINBASE_CDP_PROJECT_ID is required for a production wallet build")
    if not PROJECT_ID.fullmatch(project_id):
        raise SystemExit("COINBASE_CDP_PROJECT_ID contains unsupported characters or length")

    source = config_path.read_text(encoding="utf-8")
    occurrences = source.count(PLACEHOLDER)
    if occurrences != 1:
        raise SystemExit(f"expected exactly one Coinbase project-id placeholder, found {occurrences}")
    configured = source.replace(PLACEHOLDER, project_id)
    forbidden = ("CDP_API_KEY_SECRET", "CDP_WALLET_SECRET", "privateKey", "seedPhrase")
    if any(term in configured for term in forbidden):
        raise SystemExit("server-side wallet secret material must never enter the static config")
    config_path.write_text(configured, encoding="utf-8")
    print(f"Configured Coinbase embedded wallets for project {project_id[:4]}…{project_id[-4:]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
