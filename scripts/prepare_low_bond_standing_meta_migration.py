#!/usr/bin/env python3
"""Prepare exact low-bond replacement terms for standing-meta issues 333-336.

This script never publishes terms, signs, broadcasts, cancels, refunds, funds, or
claims a bounty. It preserves the original immutable terms and writes replacement
candidates plus a manifest for operator review.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
from typing import Any

MIGRATION_ISSUE = "https://github.com/NSPG13/agent-bounties/issues/527"
CREATOR_WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
OLD_ECONOMICS = (900_000, 100_000, 100_000, 1_000_000)
NEW_ECONOMICS = (990_000, 10_000, 10_000, 1_000_000)
REPLACEMENTS = {
    333: "0xfffecb0fcd36477c5f6ecec808f6f0cf53819562",
    334: "0xbe17ef2d154265ebe3142d7bda5e99610d571455",
    335: "0x43d42cb227d76588ab16693f14efd6cff851fa7a",
    336: "0xe8c1d3f046f3e4690bef59ba4abd5d02d2a6984b",
}


class MigrationError(RuntimeError):
    pass


def canonical_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def terms_hash(value: dict[str, Any]) -> str:
    return "0x" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


def replacement_nonce(issue_number: int) -> str:
    seed = f"agent-bounties:standing-meta-low-bond:{issue_number}:migration-527:v1"
    return "0x" + hashlib.sha256(seed.encode("utf-8")).hexdigest()


def amount(document: dict[str, Any], field: str) -> int:
    value = document["contract_terms"][field]
    if value.get("currency") != "usdc" or not isinstance(value.get("amount"), int):
        raise MigrationError(f"{field} must be integer native-USDC base units")
    return value["amount"]


def load_original(repo_root: Path, issue_number: int) -> dict[str, Any]:
    path = repo_root / "bounties" / "autonomous-v1" / f"{issue_number}.json"
    document = json.loads(path.read_text(encoding="utf-8"))
    terms = document.get("contract_terms") or {}
    observed = (
        amount(document, "solver_reward"),
        amount(document, "verifier_reward"),
        amount(document, "claim_bond"),
        amount(document, "initial_funding"),
    )
    if observed != OLD_ECONOMICS:
        raise MigrationError(
            f"#{issue_number} economics drift: expected {OLD_ECONOMICS}, got {observed}"
        )
    if str(terms.get("creator_wallet", "")).lower() != CREATOR_WALLET:
        raise MigrationError(f"#{issue_number} creator wallet drift")
    expected_source = f"https://github.com/NSPG13/agent-bounties/issues/{issue_number}"
    if document.get("source_url") != expected_source:
        raise MigrationError(f"#{issue_number} source URL drift")
    if document.get("benchmark", {}).get("engine") != "standing_meta_v2_parent":
        raise MigrationError(f"#{issue_number} is not a standing-meta-v2 parent")
    return document


def prepare(repo_root: Path, output_dir: Path) -> dict[str, Any]:
    output_dir.mkdir(parents=True, exist_ok=True)
    entries: list[dict[str, Any]] = []

    for issue_number, old_contract in REPLACEMENTS.items():
        document = copy.deepcopy(load_original(repo_root, issue_number))
        terms = document["contract_terms"]
        solver, verifier, bond, initial = NEW_ECONOMICS
        terms["solver_reward"]["amount"] = solver
        terms["verifier_reward"]["amount"] = verifier
        terms["claim_bond"]["amount"] = bond
        terms["initial_funding"]["amount"] = initial
        terms["creation_nonce"] = replacement_nonce(issue_number)
        document["discovery_source"] = (
            "maintainer-seeded standing-meta-v2 inventory; low-bond migration #527"
        )

        if verifier != bond:
            raise MigrationError("claim bond must equal verifier reward")
        if solver + verifier != initial:
            raise MigrationError("solver plus verifier reward must equal initial funding")

        target = output_dir / f"{issue_number}.json"
        target.write_text(
            json.dumps(document, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        entries.append(
            {
                "source_issue_number": issue_number,
                "source_issue_url": document["source_url"],
                "old_contract": old_contract,
                "replacement_terms_path": target.as_posix(),
                "replacement_terms_hash": terms_hash(document),
                "creation_nonce": terms["creation_nonce"],
                "solver_reward_base_units": solver,
                "verifier_reward_base_units": verifier,
                "claim_bond_base_units": bond,
                "initial_funding_base_units": initial,
            }
        )

    manifest = {
        "schema_version": "agent-bounties/standing-meta-low-bond-migration-v1",
        "migration_issue": MIGRATION_ISSUE,
        "network": "base-mainnet",
        "creator_wallet": CREATOR_WALLET,
        "reserved_contracts_csv": ",".join(REPLACEMENTS.values()),
        "replacements": entries,
        "required_confirmation_events": [
            "CanonicalBountyCreated",
            "FundingAdded",
            "BountyBecameClaimable",
        ],
        "old_contract_recovery_events": ["BountyCancelled", "RefundWithdrawn"],
        "evidence_boundary": (
            "Generated terms and plans are not publication, funding, cancellation, "
            "refund, claim, settlement, or payment evidence."
        ),
    }
    (output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("target/standing-meta-low-bond-migration"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    manifest = prepare(args.repo_root.resolve(), args.output_dir.resolve())
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
