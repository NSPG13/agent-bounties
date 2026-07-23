from __future__ import annotations

import argparse
import importlib.util
import json
import math
from pathlib import Path
from typing import Any


EVALUATOR_PATH = Path(__file__).with_name("evaluate_objective_compiler.py")
SPEC = importlib.util.spec_from_file_location("objective_compiler_eval", EVALUATOR_PATH)
evaluation = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(evaluation)

RATES_PER_MILLION = {
    "gpt-5.5": {"input": 5.0, "cached": 0.5, "output": 30.0},
    "gpt-5.6-sol": {"input": 5.0, "cached": 0.5, "output": 30.0},
    "gpt-5.6-terra": {"input": 2.5, "cached": 0.25, "output": 15.0},
    "gpt-5.6-luna": {"input": 1.0, "cached": 0.1, "output": 6.0},
}


def percentile(values: list[int], proportion: float) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * proportion) - 1)]


def score_raw_result(raw: dict[str, Any], cases: dict[str, dict[str, Any]]) -> dict[str, Any]:
    model = raw["model"]
    rates = RATES_PER_MILLION.get(model)
    if rates is None:
        raise ValueError(f"missing price for {model}")

    passed = 0
    coverage_values: list[float] = []
    failures = []
    durations = []
    totals = {
        "input_tokens": 0,
        "cached_input_tokens": 0,
        "output_tokens": 0,
        "reasoning_tokens": 0,
        "total_tokens": 0,
        "provider_calls": 0,
    }
    for result in raw["results"]:
        durations.append(int(result["duration_ms"]))
        for usage in result.get("usage", []):
            totals["provider_calls"] += 1
            for key in (
                "input_tokens",
                "cached_input_tokens",
                "output_tokens",
                "reasoning_tokens",
                "total_tokens",
            ):
                totals[key] += int(usage.get(key, 0))
        if result.get("error"):
            failures.append({"name": result["name"], "run": result["run"], "error": result["error"]})
            continue
        try:
            metrics = evaluation.validate_plan(
                result["plan"], cases[result["name"]], model
            )
            passed += 1
            coverage_values.append(float(metrics["keyword_coverage"]))
        except (KeyError, TypeError, ValueError) as error:
            failures.append({"name": result["name"], "run": result["run"], "error": str(error)})

    uncached = max(0, totals["input_tokens"] - totals["cached_input_tokens"])
    cost = (
        uncached * rates["input"]
        + totals["cached_input_tokens"] * rates["cached"]
        + totals["output_tokens"] * rates["output"]
    ) / 1_000_000
    attempted = len(raw["results"])
    coverage = sum(coverage_values) / len(coverage_values) if coverage_values else 0.0
    return {
        "model": model,
        "reasoning_effort": raw["reasoning_effort"],
        "attempted": attempted,
        "passed": passed,
        "pass_rate": round(passed / attempted, 4) if attempted else 0.0,
        "keyword_coverage": round(coverage, 4),
        "estimated_cost_usd": round(cost, 6),
        "average_cost_per_case_usd": round(cost / attempted, 6) if attempted else 0.0,
        "p50_latency_ms": percentile(durations, 0.5),
        "p95_latency_ms": percentile(durations, 0.95),
        "tokens": totals,
        "failures": failures,
    }


def select_candidate(candidates: list[dict[str, Any]], minimum_coverage: float) -> dict[str, Any] | None:
    eligible = [
        item
        for item in candidates
        if item["pass_rate"] == 1.0 and item["keyword_coverage"] >= minimum_coverage
    ]
    if not eligible:
        return None
    best_coverage = max(item["keyword_coverage"] for item in eligible)
    quality_band = [item for item in eligible if item["keyword_coverage"] >= best_coverage - 0.05]
    return min(
        quality_band,
        key=lambda item: (
            item["average_cost_per_case_usd"],
            item["p95_latency_ms"],
            -item["keyword_coverage"],
        ),
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Score cloud-agent model benchmark outputs.")
    parser.add_argument("--input-dir", required=True)
    parser.add_argument(
        "--corpus",
        default="benchmarks/openai-build-week/objective-compiler-corpus.json",
    )
    parser.add_argument("--output", default="cloud-agent-model-benchmark.json")
    parser.add_argument("--minimum-keyword-coverage", type=float, default=0.75)
    args = parser.parse_args()

    corpus = json.loads(Path(args.corpus).read_text(encoding="utf-8"))
    cases = {case["name"]: case for case in corpus["cases"]}
    candidates = [
        score_raw_result(json.loads(path.read_text(encoding="utf-8")), cases)
        for path in sorted(Path(args.input_dir).glob("*.json"))
    ]
    selected = select_candidate(candidates, args.minimum_keyword_coverage)
    report = {
        "schema_version": "agent-bounties/cloud-model-benchmark-v1",
        "pricing_basis": "OpenAI standard API text-token prices observed 2026-07-20; output_tokens already include reasoning tokens",
        "selection_rule": "All cases pass, keyword coverage is within 0.05 of the best eligible candidate, then lowest measured cost and latency win.",
        "minimum_keyword_coverage": args.minimum_keyword_coverage,
        "selected": (
            {"model": selected["model"], "reasoning_effort": selected["reasoning_effort"]}
            if selected
            else None
        ),
        "candidates": candidates,
    }
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0 if selected else 1


if __name__ == "__main__":
    raise SystemExit(main())
