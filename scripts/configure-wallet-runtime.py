#!/usr/bin/env python3
"""Inject the public CDP Project ID into a staged wallet runtime configuration."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

PROJECT_ID = re.compile(r"^[A-Za-z0-9_-]{8,128}$")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", default="site/wallet-runtime-config.js")
    parser.add_argument("--project-id", default="")
    parser.add_argument("--require-enabled", action="store_true")
    args = parser.parse_args()

    path = Path(args.file)
    source = path.read_text(encoding="utf-8")
    project_id = args.project_id.strip()

    if source.count("enabled: false") != 1 or source.count('projectId: ""') != 1:
        raise SystemExit("wallet runtime activation placeholders were not found exactly once")
    if 'accountType: "eoa"' not in source or 'gasSponsorshipMode: "eip7702-cdp-paymaster"' not in source:
        raise SystemExit("wallet runtime lost its reviewed EOA and sponsored-gas configuration")

    if not project_id:
        if args.require_enabled:
            raise SystemExit("CDP_PROJECT_ID is required for this deployment")
        print("Coinbase embedded wallet remains fail-closed because CDP_PROJECT_ID is not configured")
        return 0
    if not PROJECT_ID.fullmatch(project_id):
        raise SystemExit("CDP_PROJECT_ID contains unsupported characters or length")

    updated = source.replace("enabled: false", "enabled: true", 1)
    updated = updated.replace('projectId: ""', f"projectId: {json.dumps(project_id)}", 1)
    if "enabled: false" in updated or 'projectId: ""' in updated:
        raise SystemExit("wallet runtime activation substitution was incomplete")
    path.write_text(updated, encoding="utf-8")
    print("Coinbase embedded wallet runtime activated for the staged site")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
