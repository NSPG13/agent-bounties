#!/usr/bin/env python3
"""Run and record application release checks without a hosted CI provider.

The record is review evidence, not a signature or authority to deploy/sign/pay.
Run only reviewed source on a trusted operator machine without production secrets.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "agent-bounties/application-release-gate-v1"
STEPS = (
    ("public-provenance", (sys.executable, "scripts/check_public_runtime_parity.py", "--published")),
    ("gate-contract", (sys.executable, "scripts/test_application_release_gate.py")),
    ("repository", ("bash", "scripts/check.sh")),
    ("postgres-sync", (sys.executable, "scripts/run-with-test-postgres.py", "--",
                       sys.executable, "scripts/application_release_gate.py", "postgres")),
    ("production-compose", ("bash", "scripts/check-production-compose.sh")),
)
POSTGRES = (
    ("db", "autonomous_canonicality::tests::", 1),
    ("worker", "autonomous_canonicality::tests::", 1),
    ("db", "terms_publication::tests::", 3),
    ("api", "terms_publication::tests::publication_http_wallet_ownership_and_exact_historical_replay", 1),
    ("api", "marketplace_readiness::tests::database_snapshots_finish_before_first_shared_head_postgres", 1),
    ("api", "marketplace_readiness::diagnostic_tests::observation_timeout_stages_postgres", 1),
    ("db", "timeout_revalidation::tests::", 1),
    ("db", "timeout_relay::tests::", 3),
    ("api", "timeout_revalidation::tests::", 3),
    ("api", "timeout_relay::tests::timeout_journal", 4),
    ("db", "managed_service_orders::tests::", 7),
    ("db", "managed_service_orders::metrics::postgres_tests::", 1),
    ("db", "managed_service_orders::stripe_tests::", 10),
    ("db", "managed_service_orders::stripe_execution_tests::", 6),
    ("db", "managed_service_orders::refunds::tests::", 7),
    ("db", "managed_service_orders::recovered::tests::", 6),
    ("db", "tests::autonomous_v2_events_round_trip_through_postgres", 1),
    ("db", "stripe_reconciliation::tests::", 7),
    ("service-runtime", "stripe_reconciliation::tests::", 3),
    ("service-runtime", "managed_checkout::tests::", 5),
    ("service-runtime", "managed_refund::tests::", 3),
    ("api", "managed_payments::tests::", 11),
    ("api", "tests::stripe_signed_webhook_is_durable_across_independent_api_states", 1),
    ("db", "tests::platform_metrics_query_separates_history_from_identity_and_cohort_boundaries", 1),
    ("api", "platform_payouts::tests::platform_payout_http_replays_real_database_selection", 1),
    ("db", "distribution_", 1),
    ("db", "site_posting_drafts", 1),
    ("worker", "open_competition_v2_shadow::tests::postgres_shadow_cache_certifies_full_sets_and_recovers_from_disagreement", 1),
    ("db", "tests::x402_relay_attempt_is_idempotent_and_lease_bounded", 1),
    ("api", "github_issue_api_sync_postgres", 1),
    ("db", "scoped_submission_history_reads_only_selected_bounty_postgres", 1),
    ("api", "opportunity_exact_unfunded_lookup_crosses_list_window_postgres", 1),
    ("db", "review_notifications::tests::", 21),
)


class GateError(RuntimeError):
    pass


def digest(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def source() -> dict[str, str]:
    if git("status", "--porcelain", "--untracked-files=normal"):
        raise GateError("Release checks require committed, clean source")
    return {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}"),
            "gate_sha256": digest(Path(__file__))}


def environment() -> dict[str, str]:
    # Do not forward deployment, email, wallet, operator or provider credentials.
    names = ("PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "LANG", "LC_ALL", "TERM",
             "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "CARGO_TARGET_DIR",
             "CARGO_BUILD_JOBS", "CARGO_PROFILE_DEV_DEBUG", "CARGO_PROFILE_TEST_DEBUG",
             "CARGO_INCREMENTAL", "PIP_CACHE_DIR", "VIRTUAL_ENV")
    return {key: os.environ[key] for key in names if key in os.environ}


def write_record(path: Path, value: dict) -> None:
    temp = path.with_suffix(".tmp")
    temp.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temp.replace(path)


def local_fixture_environment() -> dict[str, str]:
    # Hold both sockets together so the selected loopback ports are distinct.
    # Services still fail closed if another process claims a port after release.
    with socket.socket() as api, socket.socket() as mcp:
        api.bind(("127.0.0.1", 0))
        mcp.bind(("127.0.0.1", 0))
        return {
            "PRODUCTION_COMPOSE_API_PORT": str(api.getsockname()[1]),
            "PRODUCTION_COMPOSE_MCP_PORT": str(mcp.getsockname()[1]),
            "PRODUCTION_COMPOSE_PROJECT": "ab-release-" + uuid.uuid4().hex,
        }


def validate_record(record: dict, directory: Path, current: dict, now: dt.datetime) -> None:
    if record.get("schema") != SCHEMA or record.get("status") != "passed":
        raise GateError("Release gate did not pass")
    if record.get("source") != current:
        raise GateError("Release source or gate changed since verification")
    try:
        finished = dt.datetime.fromisoformat(record["finished_at"])
        age = (now - finished).total_seconds()
    except (KeyError, TypeError, ValueError) as error:
        raise GateError("Invalid evidence timestamp") from error
    if age < 0 or age > 86400:
        raise GateError("Release evidence must be no older than 24 hours")
    rows = record.get("steps", [])
    if not isinstance(rows, list) or [row.get("name") for row in rows] != [name for name, _ in STEPS]:
        raise GateError("Missing or unexpected release check")
    for row, (name, command) in zip(rows, STEPS):
        if row.get("exit_code") != 0 or row.get("command") != list(command):
            raise GateError(f"Required check did not pass: {name}")
        # Derive filenames from the fixed inventory, never from record input.
        log = directory / f"{name}.log"
        if not log.is_file() or log.is_symlink() or digest(log) != row.get("log_sha256"):
            raise GateError(f"Missing or changed check log: {name}")


def run_gate(directory: Path) -> int:
    identity = source()
    directory.mkdir(parents=True, exist_ok=False, mode=0o700)
    record = {"schema": SCHEMA, "source": identity, "status": "running",
              "started_at": dt.datetime.now(dt.timezone.utc).isoformat(), "steps": []}
    record_path = directory / "release.json"
    env = environment()
    write_record(record_path, record)
    for name, command in STEPS:
        fixture = local_fixture_environment()
        print(f"Running {name}; log: {directory / (name + '.log')}", flush=True)
        row = {"name": name, "command": list(command), "local_fixture": fixture}
        with (directory / f"{name}.log").open("w") as log:
            try:
                result = subprocess.run(command, cwd=ROOT, env={**env, **fixture}, stdout=log,
                                        stderr=subprocess.STDOUT, timeout=7200, check=False)
                row["exit_code"] = result.returncode
            except (OSError, subprocess.TimeoutExpired):
                row["exit_code"] = 124
                log.write("\nRelease check did not complete.\n")
        row["log_sha256"] = digest(directory / f"{name}.log")
        record["steps"].append(row)
        if row["exit_code"] != 0:
            record["status"] = "failed"
            write_record(record_path, record)
            return 1
        write_record(record_path, record)
    if source() != identity:
        raise GateError("Source changed during release checks")
    record["finished_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
    record["status"] = "passed"
    write_record(record_path, record)
    validate_record(record, directory, source(), dt.datetime.now(dt.timezone.utc))
    print(f"Release gate passed: {identity['commit']} tree={identity['tree']}")
    return 0


def postgres() -> int:
    from urllib.parse import urlsplit
    parsed = urlsplit(os.environ.get("AGENT_BOUNTIES_TEST_DATABASE_URL", ""))
    if (os.environ.get("AGENT_BOUNTIES_LOCAL_TEST_DATABASE") != "1"
            or parsed.scheme not in ("postgres", "postgresql")
            or parsed.hostname not in ("127.0.0.1", "localhost")
            or parsed.username != "agent_bounties_test" or parsed.path != "/agent_bounties_test"
            or parsed.query or parsed.fragment):
        raise GateError("Postgres checks require the isolated local fixture")
    for package, prefix, minimum in POSTGRES:
        result = subprocess.run(["cargo", "test", "-p", package, prefix, "--", "--ignored",
                                 "--test-threads=1"], cwd=ROOT, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
        print(result.stdout, end="", flush=True)
        passed = sum(map(int, re.findall(r"test result: ok\. (\d+) passed", result.stdout)))
        if result.returncode or passed < minimum:
            raise GateError(f"Postgres check failed or selected too few tests: {package} {prefix}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("run", "verify", "postgres"))
    parser.add_argument("--evidence-dir", type=Path)
    args = parser.parse_args()
    if args.operation == "postgres":
        return postgres()
    if args.evidence_dir is None:
        parser.error("--evidence-dir is required")
    directory = args.evidence_dir.resolve()
    if args.operation == "run":
        return run_gate(directory)
    record = json.loads((directory / "release.json").read_text())
    validate_record(record, directory, source(), dt.datetime.now(dt.timezone.utc))
    subprocess.run([sys.executable, "scripts/check_public_runtime_parity.py", "--published"], cwd=ROOT, check=True, env=environment())
    print(f"Verified release evidence for {record['source']['commit']}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (GateError, OSError, ValueError) as error:
        print(f"Release gate blocked: {error}", file=sys.stderr)
        raise SystemExit(1)
