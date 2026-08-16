#!/usr/bin/env python3
"""Fail on any untriaged high/medium Slither finding in V2 Beta3."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


SOURCE = "contracts/base-escrow/src/OpenCompetitionBountyV2Beta3.sol"
TRIAGED = {
    ("High", "arbitrary-send-erc20", "fundFromFactory(address,uint256,bytes32)"),
    ("Medium", "reentrancy-no-eth", "fund(uint256,bytes32)"),
    ("Medium", "reentrancy-no-eth", "fundFromFactory(address,uint256,bytes32)"),
    (
        "Medium",
        "reentrancy-no-eth",
        "fundWithAuthorization(address,uint256,bytes32,uint256,uint256,bytes32,uint8,bytes32,bytes32)",
    ),
}
FORBIDDEN_CHECKS = {
    "controlled-delegatecall",
    "delegatecall-loop",
    "msg-value-loop",
    "reentrancy-eth",
    "return-bomb",
    "suicidal",
    "uninitialized-state",
    "unprotected-upgrade",
}


def signature(detector: dict[str, Any]) -> str:
    elements = detector.get("elements") or []
    if not elements:
        return ""
    fields = elements[0].get("type_specific_fields") or {}
    return str(fields.get("signature") or "")


def source(detector: dict[str, Any]) -> str:
    elements = detector.get("elements") or []
    if not elements:
        return ""
    mapping = elements[0].get("source_mapping") or {}
    return str(mapping.get("filename_relative") or "").replace("\\", "/")


def verify(value: dict[str, Any]) -> dict[str, Any]:
    if value.get("success") is not True:
        raise ValueError("Slither report did not complete successfully")
    detectors = value.get("results", {}).get("detectors")
    if not isinstance(detectors, list):
        raise ValueError("Slither report omitted detectors")
    forbidden = sorted({str(item.get("check")) for item in detectors} & FORBIDDEN_CHECKS)
    if forbidden:
        raise ValueError(f"forbidden Slither checks found: {', '.join(forbidden)}")
    observed = {
        (str(item.get("impact")), str(item.get("check")), signature(item))
        for item in detectors
        if item.get("impact") in {"High", "Medium"} and source(item) == SOURCE
    }
    unexpected = sorted(observed - TRIAGED)
    missing = sorted(TRIAGED - observed)
    if unexpected:
        raise ValueError(f"untriaged high/medium Slither findings: {unexpected}")
    if missing:
        raise ValueError(
            "triage fingerprint changed; inspect rather than silently accepting the drift: "
            f"{missing}"
        )
    counts: dict[str, int] = {}
    for item in detectors:
        impact = str(item.get("impact"))
        counts[impact] = counts.get(impact, 0) + 1
    return {
        "passed": True,
        "triaged_high_medium": len(observed),
        "counts": counts,
        "evidence_boundary": "This confirms a stable maintainer triage fingerprint. It is not an independent security review.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = verify(json.loads(args.report.read_text(encoding="utf-8")))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
