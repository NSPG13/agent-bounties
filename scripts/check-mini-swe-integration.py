#!/usr/bin/env python3
"""Smoke test for mini-SWE-agent integration."""

import json, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
errors = []

config = ROOT / "integrations/mini-swe-agent/config.toml"
if not config.exists():
    errors.append("config.toml missing")
else:
    c = config.read_text().lower()
    for p in ("claimable-live", "positive_margin", "exclusive", "evidence"):
        if p not in c:
            errors.append(f"config missing: {p}")

for name in ("multiple-claimable", "empty", "stale", "no-margin", "exclusive-claimant"):
    p = ROOT / f"integrations/mini-swe-agent/fixtures/{name}.json"
    if not p.exists():
        errors.append(f"fixture {name}.json missing")
    else:
        try:
            d = json.loads(p.read_text())
            if "action" not in d:
                errors.append(f"fixture {name} missing action")
        except:
            errors.append(f"fixture {name} invalid JSON")

readme = ROOT / "integrations/mini-swe-agent/README.md"
if not readme.exists():
    errors.append("README.md missing")

if errors:
    print("FAILED:"); [print(f"  - {e}") for e in errors]
    sys.exit(1)
print("mini-SWE-agent integration smoke test PASSED")
