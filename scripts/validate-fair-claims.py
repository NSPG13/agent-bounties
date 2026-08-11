#!/usr/bin/env python3
import os
import sys

REQUIRED_INVARIANTS = {
    "one_active_slot_invariant": [
        "1 active exclusive claim slot per solver address",
        "one_active_slot_invariant",
    ],
    "hour_scale_renewal_evidence": [
        "hour-scale durations",
        "progress evidence URIs",
        "hour_scale_renewal_evidence",
    ],
    "bond_rejection_solvency": [
        "100% rejection solvency",
        "bond_rejection_solvency",
    ],
    "precommitted_symmetric_appeals": [
        "precommitted symmetric appeal contracts",
        "precommitted_symmetric_appeals",
    ],
    "historical_bytecode_compatibility": [
        "historical V1/V2/V3/V4 on-chain bytecode",
        "BountySettled",
        "historical_bytecode_compatibility",
    ],
}

def validate_fair_claims_assessment(filepath="docs/FAIR_EXCLUSIVE_CLAIMS_ASSESSMENT.md"):
    if not os.path.exists(filepath):
        print(f"FAIL: Missing document {filepath}", file=sys.stderr)
        sys.exit(1)

    with open(filepath, "r", encoding="utf-8") as f:
        content = f.read()

    print(f"Validating {filepath} for #794 invariants...")

    missing = []
    for name, keywords in REQUIRED_INVARIANTS.items():
        matched = all(kw in content for kw in keywords)
        if matched:
            print(f"  [PASS] Invariant: {name}")
        else:
            print(f"  [FAIL] Missing required invariant keywords for: {name}", file=sys.stderr)
            missing.append(name)

    if missing:
        print(f"\nVALIDATION FAILED: {len(missing)} invariant(s) missing: {', '.join(missing)}", file=sys.stderr)
        sys.exit(1)

    print("\n[SUCCESS] All Fair Exclusive Claims (#794) invariants passed validation.")

if __name__ == "__main__":
    validate_fair_claims_assessment()
