#!/usr/bin/env python3
"""Check that the live GitHub branch ruleset has not drifted from canonical.

The canonical configuration lives in ``.github/rulesets/main.json``. This script
is read-only: it authenticates through the ``gh`` CLI to fetch the live ruleset,
but it never writes to GitHub, never posts comments, and never touches payment
code. It performs two independent checks and exits non-zero if either fails:

1. Structural drift - compare the canonical ruleset to the live ruleset after
   removing only server-owned bookkeeping fields (numeric ids, timestamps, and
   the source links GitHub attaches when it returns a ruleset). Any remaining
   difference is real drift a maintainer must reconcile.
2. Semantic validation - confirm the ruleset still encodes the documented main
   branch protections. This runs against both the canonical file and the live
   ruleset, so a hand-edit to either side is caught.

Point it at a fixture with ``--live-fixture`` to run fully offline. Without a
fixture it calls ``gh api`` to read (never modify) the live ruleset.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
CANONICAL_PATH = REPO_ROOT / ".github" / "rulesets" / "main.json"

# Fields GitHub owns and attaches to a ruleset when it returns one. They carry
# no policy meaning, so ignore exactly these and nothing else when diffing.
SERVER_OWNED_TOP_LEVEL = frozenset(
    {
        "id",
        "node_id",
        "source",
        "source_type",
        "created_at",
        "updated_at",
        "_links",
        "current_user_can_bypass",
    }
)
SERVER_OWNED_RULE_FIELDS = frozenset(
    {
        "ruleset_source_type",
        "ruleset_source",
        "ruleset_id",
    }
)

# Semantic expectations documented in docs/main-branch-ruleset.md.
GITHUB_ACTIONS_INTEGRATION_ID = 15368
ADMIN_REPOSITORY_ROLE_ID = 5
REQUIRED_STATUS_CONTEXTS = ("full-check", "postgres-sync")
DEFAULT_BRANCH_TARGET = "~DEFAULT_BRANCH"


def normalize_ruleset(ruleset: dict[str, Any]) -> dict[str, Any]:
    """Return a copy of ``ruleset`` with only server-owned fields removed."""
    normalized = {
        key: value
        for key, value in ruleset.items()
        if key not in SERVER_OWNED_TOP_LEVEL
    }
    rules = normalized.get("rules")
    if isinstance(rules, list):
        normalized["rules"] = [_strip_rule(rule) for rule in rules]
    return normalized


def _strip_rule(rule: Any) -> Any:
    if not isinstance(rule, dict):
        return rule
    normalized = {
        key: value
        for key, value in rule.items()
        if key not in SERVER_OWNED_RULE_FIELDS
    }
    # GitHub materializes this API default even when the submitted canonical
    # JSON omits it. Preserve non-empty values because they change policy.
    if normalized.get("type") == "pull_request":
        parameters = normalized.get("parameters")
        if isinstance(parameters, dict):
            parameters = dict(parameters)
            parameters.setdefault("required_reviewers", [])
            normalized["parameters"] = parameters
    return normalized


def _canonical_form(value: Any) -> Any:
    """Recursively sort dict keys and lists so ordering never masquerades as drift."""
    if isinstance(value, dict):
        return {key: _canonical_form(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        items = [_canonical_form(item) for item in value]
        return sorted(items, key=lambda item: json.dumps(item, sort_keys=True))
    return value


def _diff(expected: Any, actual: Any, path: str, out: list[str]) -> None:
    if isinstance(expected, dict) and isinstance(actual, dict):
        for key in sorted(set(expected) | set(actual)):
            child = f"{path}.{key}" if path else key
            if key not in expected:
                out.append(f"{child}: present live but not in canonical ({actual[key]!r})")
            elif key not in actual:
                out.append(f"{child}: in canonical but missing live ({expected[key]!r})")
            else:
                _diff(expected[key], actual[key], child, out)
    elif isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            out.append(
                f"{path or 'rules'}: canonical has {len(expected)} entries, live has {len(actual)}"
            )
        for index, (exp_item, act_item) in enumerate(zip(expected, actual)):
            _diff(exp_item, act_item, f"{path}[{index}]", out)
    elif expected != actual:
        out.append(f"{path}: canonical {expected!r} != live {actual!r}")


def compare_rulesets(canonical: dict[str, Any], live: dict[str, Any]) -> list[str]:
    """Return human-readable drift entries, ignoring only server-owned fields."""
    expected = _canonical_form(normalize_ruleset(canonical))
    actual = _canonical_form(normalize_ruleset(live))
    differences: list[str] = []
    _diff(expected, actual, "", differences)
    return differences


def validate_semantics(ruleset: dict[str, Any]) -> list[str]:
    """Return the list of documented protections this ruleset fails to encode."""
    problems: list[str] = []

    if ruleset.get("target") != "branch":
        problems.append("target must be 'branch'")
    if ruleset.get("enforcement") != "active":
        problems.append("enforcement must be 'active'")

    ref_name = ruleset.get("conditions", {}).get("ref_name", {})
    include = ref_name.get("include", [])
    exclude = ref_name.get("exclude", [])
    if include != [DEFAULT_BRANCH_TARGET]:
        problems.append(
            f"ref_name.include must target only [{DEFAULT_BRANCH_TARGET!r}], found {include!r}"
        )
    if exclude:
        problems.append(f"ref_name.exclude must be empty, found {exclude!r}")

    bypass_actors = ruleset.get("bypass_actors", [])
    if len(bypass_actors) != 1:
        problems.append(f"expected exactly one bypass actor, found {len(bypass_actors)}")
    else:
        actor = bypass_actors[0]
        if actor.get("actor_type") != "RepositoryRole":
            problems.append("bypass actor must be a RepositoryRole")
        if actor.get("actor_id") != ADMIN_REPOSITORY_ROLE_ID:
            problems.append(
                f"bypass actor must be the admin repository role (id {ADMIN_REPOSITORY_ROLE_ID})"
            )
        if actor.get("bypass_mode") != "pull_request":
            problems.append("admin bypass must use pull_request mode only")

    rules_by_type: dict[str, dict[str, Any]] = {}
    for rule in ruleset.get("rules", []):
        if isinstance(rule, dict) and isinstance(rule.get("type"), str):
            rules_by_type.setdefault(rule["type"], rule)

    if "deletion" not in rules_by_type:
        problems.append("deletion protection rule is missing")
    if "non_fast_forward" not in rules_by_type:
        problems.append("non-fast-forward protection rule is missing")

    pull_request = rules_by_type.get("pull_request")
    if pull_request is None:
        problems.append("pull_request rule is missing")
    else:
        params = pull_request.get("parameters", {})
        if params.get("required_approving_review_count") != 1:
            problems.append("pull request rule must require exactly one approval")
        if params.get("require_last_push_approval") is not True:
            problems.append("pull request rule must require latest-push approval")
        if params.get("required_review_thread_resolution") is not True:
            problems.append("pull request rule must require review thread resolution")

    checks_rule = rules_by_type.get("required_status_checks")
    if checks_rule is None:
        problems.append("required_status_checks rule is missing")
    else:
        params = checks_rule.get("parameters", {})
        if params.get("strict_required_status_checks_policy") is not False:
            problems.append("status checks must use loose mode (strict policy must be false)")
        checks_by_context: dict[Any, dict[str, Any]] = {}
        for check in params.get("required_status_checks", []):
            if isinstance(check, dict):
                checks_by_context.setdefault(check.get("context"), check)
        for context in REQUIRED_STATUS_CONTEXTS:
            check = checks_by_context.get(context)
            if check is None:
                problems.append(f"required status check {context!r} is missing")
            elif check.get("integration_id") != GITHUB_ACTIONS_INTEGRATION_ID:
                problems.append(
                    f"required status check {context!r} must be bound to GitHub Actions "
                    f"integration {GITHUB_ACTIONS_INTEGRATION_ID}, found "
                    f"{check.get('integration_id')!r}"
                )

    return problems


def evaluate(canonical: dict[str, Any], live: dict[str, Any]) -> dict[str, list[str]]:
    """Run every offline check and return the collected findings."""
    return {
        "drift": compare_rulesets(canonical, live),
        "canonical_semantic_problems": validate_semantics(canonical),
        "live_semantic_problems": validate_semantics(live),
    }


def is_clean(result: dict[str, list[str]]) -> bool:
    return not any(result.values())


def _gh_json(args: list[str]) -> Any:
    completed = subprocess.run(
        ["gh", "api", *args], check=True, capture_output=True, text=True
    )
    return json.loads(completed.stdout)


def fetch_live_ruleset(repository: str, name: str) -> dict[str, Any]:
    """Read-only fetch of the live ruleset named ``name`` via the gh CLI."""
    listing = _gh_json([f"repos/{repository}/rulesets"])
    if not isinstance(listing, list):
        raise RuntimeError("unexpected rulesets listing response")
    match = next(
        (item for item in listing if isinstance(item, dict) and item.get("name") == name),
        None,
    )
    if match is None:
        raise RuntimeError(f"no live ruleset named {name!r} on {repository}")
    ruleset_id = match.get("id")
    if not isinstance(ruleset_id, int):
        raise RuntimeError("live ruleset is missing a numeric id")
    return _gh_json([f"repos/{repository}/rulesets/{ruleset_id}"])


def format_report(result: dict[str, list[str]]) -> str:
    lines: list[str] = []
    if result["drift"]:
        lines.append("Structural drift from canonical ruleset:")
        lines.extend(f"  - {entry}" for entry in result["drift"])
    if result["canonical_semantic_problems"]:
        lines.append("Canonical ruleset fails semantic validation:")
        lines.extend(f"  - {entry}" for entry in result["canonical_semantic_problems"])
    if result["live_semantic_problems"]:
        lines.append("Live ruleset fails semantic validation:")
        lines.extend(f"  - {entry}" for entry in result["live_semantic_problems"])
    if not lines:
        lines.append("Live ruleset matches canonical and satisfies every documented protection.")
    return "\n".join(lines)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", default="NSPG13/agent-bounties")
    parser.add_argument("--canonical", type=Path, default=CANONICAL_PATH)
    parser.add_argument(
        "--live-fixture",
        type=Path,
        help="Read the live ruleset from a JSON file instead of calling gh (offline).",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    canonical = json.loads(args.canonical.read_text(encoding="utf-8"))
    if args.live_fixture:
        live = json.loads(args.live_fixture.read_text(encoding="utf-8"))
    else:
        live = fetch_live_ruleset(args.repository, canonical["name"])

    result = evaluate(canonical, live)
    print(format_report(result))
    return 0 if is_clean(result) else 1


if __name__ == "__main__":
    raise SystemExit(main())
