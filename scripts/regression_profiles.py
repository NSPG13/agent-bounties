"""The same reviewed execution profiles embedded by verifier-sdk."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

REGISTRY_PATH = Path(__file__).resolve().parents[1] / "crates/verifier-sdk/regression-profiles-v1.json"


def _unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate profile registry key")
        result[key] = value
    return result


def registry() -> dict[str, Any]:
    value = json.loads(REGISTRY_PATH.read_bytes(), object_pairs_hook=_unique)
    if value.get("schema") != "agent-bounties/regression-profiles-v1" or type(value.get("version")) is not int or value["version"] != 1:
        raise ValueError("unsupported regression profile registry")
    profiles = value.get("profiles")
    if not isinstance(profiles, list) or len({p["id"] for p in profiles}) != len(profiles):
        raise ValueError("invalid regression profile registry")
    return value


def registry_digest() -> str:
    return "sha256:" + hashlib.sha256(REGISTRY_PATH.read_bytes()).hexdigest()


def require_approved_profile(benchmark: dict[str, Any]) -> dict[str, Any]:
    source = dict(benchmark.get("source") or {})
    for key in ("repository", "commit"):
        if not isinstance(source.get(key), str):
            raise ValueError("immutable source tuple is unavailable")
        source[key] = source[key].lower()
    if benchmark.get("engine") != "sandboxed_regression_v1":
        raise ValueError("sandboxed regression benchmark engine is unavailable")
    profiles = registry()["profiles"]
    matching = [p for p in profiles if source in p["sources"] and p["runner_manifest"]["benchmark_digest"] == benchmark.get("runner_manifest", {}).get("benchmark_digest")]
    if not matching:
        raise ValueError("immutable source tuple and benchmark digest must be independently reconciled and approved")
    # JSON comparison also distinguishes true from 1, and 120.0 from 120.
    canonical = lambda value: json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)
    for profile in matching:
        if profile["status"] == "approved" and canonical(profile["runner_manifest"]) == canonical(benchmark.get("runner_manifest")):
            return profile
    raise ValueError("verification_profile_unapproved: Verification unavailable. The complete runner manifest must be independently reconciled and approved.")
