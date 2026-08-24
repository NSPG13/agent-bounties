#!/usr/bin/env python3
"""Publish exact scoring metadata for a canonically activated reward cohort."""

from __future__ import annotations

import argparse
from copy import deepcopy
from datetime import datetime
import json
from pathlib import Path
import re
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REGISTRY_SCHEMA = "agent-bounties/open-competition-v2-public-metadata-v1"
COHORT_SCHEMA = "agent-bounties/open-competition-v2-forward-gmv-reward-cohort-v1"
BUNDLE_SCHEMA = "agent-bounties/open-competition-v2-reward-policy-rotation-v1"
RESULT_SCHEMA = "agent-bounties/open-competition-v2-reward-execution-v1"
SOURCE_COMMIT = "b600500a0ba25babe5bf9d262472ef4f701b480a"
SOURCE_URL = (
    f"https://github.com/NSPG13/agent-bounties/blob/{SOURCE_COMMIT}/ops/"
    "open-competition-v2-forward-gmv-reward-cohort-v1.json"
)
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")


class MetadataSyncError(ValueError):
    pass


def require_object(value: object, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise MetadataSyncError(f"{field} must be an object")
    return value


def require_list(value: object, field: str) -> list[Any]:
    if not isinstance(value, list):
        raise MetadataSyncError(f"{field} must be an array")
    return value


def scoring_timestamp(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.endswith("Z"):
        raise MetadataSyncError(f"{field} must be an exact UTC timestamp")
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise MetadataSyncError(f"{field} is malformed") from error
    return value


def synchronize(
    registry_value: object,
    cohort_value: object,
    bundle_value: object,
    result_value: object,
) -> dict[str, Any]:
    registry = require_object(registry_value, "registry")
    cohort = require_object(cohort_value, "cohort")
    bundle = require_object(bundle_value, "bundle")
    result = require_object(result_value, "result")
    if registry.get("schema_version") != REGISTRY_SCHEMA:
        raise MetadataSyncError("public registry schema is invalid")
    if cohort.get("schema_version") != COHORT_SCHEMA:
        raise MetadataSyncError("reward cohort schema is invalid")
    if bundle.get("schema_version") != BUNDLE_SCHEMA:
        raise MetadataSyncError("reviewed reward bundle schema is invalid")
    if result.get("schema_version") != RESULT_SCHEMA:
        raise MetadataSyncError("canonical execution result schema is invalid")
    if result.get("status") != "canonically_activated":
        raise MetadataSyncError("reward cohort is not canonically activated")
    for field in ("network", "factory_contract"):
        expected = str(registry.get(field) or "").lower()
        if not expected or str(cohort.get(field) or "").lower() != expected:
            raise MetadataSyncError(f"{field} disagrees across public inputs")
    if str(bundle.get("network") or "").lower() != str(registry["network"]).lower():
        raise MetadataSyncError("bundle network disagrees with the registry")

    candidates = require_list(cohort.get("candidates"), "cohort.candidates")
    creations = require_list(bundle.get("creations"), "bundle.creations")
    state = require_object(result.get("state"), "result.state")
    observed = require_list(state.get("creations"), "result.state.creations")
    if not candidates or len(candidates) != len(creations) or len(observed) != len(candidates):
        raise MetadataSyncError("cohort, bundle, and canonical result cardinality disagree")
    if state.get("used_count") != len(candidates) or state.get("active_count") != len(candidates):
        raise MetadataSyncError("canonical result does not contain the full active cohort")

    published = deepcopy(registry)
    entries = require_list(published.get("competitions"), "registry.competitions")
    by_seed = {str(item.get("seed_id") or ""): item for item in entries if isinstance(item, dict)}
    by_competition = {
        str(item.get("competition") or "").lower(): item
        for item in entries
        if isinstance(item, dict)
    }
    by_bounty = {
        str(item.get("bounty_id") or "").lower(): item
        for item in entries
        if isinstance(item, dict)
    }

    for index, (candidate_value, creation_value, observed_value) in enumerate(
        zip(candidates, creations, observed, strict=True)
    ):
        candidate = require_object(candidate_value, f"cohort.candidates[{index}]")
        creation = require_object(creation_value, f"bundle.creations[{index}]")
        live = require_object(observed_value, f"result.state.creations[{index}]")
        candidate_id = str(candidate.get("candidate_id") or "")
        if not candidate_id or any(
            value.get("candidate_id") != candidate_id for value in (creation, live)
        ):
            raise MetadataSyncError(f"candidate order or identity disagrees at index {index}")
        competition = str(creation.get("predicted_competition") or "").lower()
        bounty_id = str(creation.get("bounty_id") or "").lower()
        if not ADDRESS.fullmatch(competition) or not HASH.fullmatch(bounty_id):
            raise MetadataSyncError(f"candidate {candidate_id} has malformed contract identity")
        if (
            str(live.get("competition") or "").lower() != competition
            or live.get("used") is not True
            or live.get("approved") is not True
            or live.get("status") != 1
        ):
            raise MetadataSyncError(f"candidate {candidate_id} is not canonically active")
        title = str(candidate.get("title") or "").strip()
        summary = str(candidate.get("summary") or "").strip()
        epoch = require_object(candidate.get("epoch"), f"candidate {candidate_id}.epoch")
        starts_at = scoring_timestamp(epoch.get("starts_at"), f"candidate {candidate_id}.starts_at")
        ends_at = scoring_timestamp(epoch.get("ends_at"), f"candidate {candidate_id}.ends_at")
        minimum_score = epoch.get("minimum_score_base_units")
        if not title or not summary or not isinstance(minimum_score, int) or minimum_score <= 0:
            raise MetadataSyncError(f"candidate {candidate_id} public terms are incomplete")
        if datetime.fromisoformat(starts_at.replace("Z", "+00:00")) >= datetime.fromisoformat(
            ends_at.replace("Z", "+00:00")
        ):
            raise MetadataSyncError(f"candidate {candidate_id} scoring window is invalid")
        entry = {
            "seed_id": candidate_id,
            "bounty_id": bounty_id,
            "competition": competition,
            "title": title,
            "summary": summary,
            "source_url": SOURCE_URL,
            "epoch_starts_at": starts_at,
            "epoch_ends_at": ends_at,
            "minimum_score_base_units": str(minimum_score),
        }
        collisions = [
            existing
            for existing in (
                by_seed.get(candidate_id),
                by_competition.get(competition),
                by_bounty.get(bounty_id),
            )
            if existing is not None
        ]
        if collisions:
            if any(existing != entry for existing in collisions):
                raise MetadataSyncError(f"candidate {candidate_id} conflicts with public metadata")
            continue
        entries.append(entry)
        by_seed[candidate_id] = entry
        by_competition[competition] = entry
        by_bounty[bounty_id] = entry
    return published


def load(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--registry",
        type=Path,
        default=ROOT / "ops" / "open-competition-v2-public-metadata-v1.json",
    )
    parser.add_argument(
        "--cohort",
        type=Path,
        default=ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json",
    )
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("--json-out", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        published = synchronize(
            load(args.registry), load(args.cohort), load(args.bundle), load(args.result)
        )
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(published, indent=2) + "\n", encoding="utf-8")
    except (OSError, json.JSONDecodeError, MetadataSyncError) as error:
        print(json.dumps({"status": "blocked", "error": str(error)}))
        return 2
    print(
        json.dumps(
            {
                "status": "published",
                "competitions": len(published["competitions"]),
                "output": str(args.json_out),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
