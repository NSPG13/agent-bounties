#!/usr/bin/env python3
"""Run the durable security regressions against an isolated local PostgreSQL fixture."""

from __future__ import annotations

import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parents[1]
SUITES = (
    ("db", "autonomous_canonicality::tests::", 1),
    ("worker", "autonomous_canonicality::tests::", 1),
    ("api", "nonce_retirement::tests::", 5),
    ("api", "nonce_retirement::retained_dispatch::tests::", 5),
    ("db", "timeout_relay::tests::", 3),
    ("api", "timeout_relay::tests::timeout_journal", 4),
    ("api", "tests::bounty_discovery_regressions::", 1),
    ("mcp-server", "tests::bounty_discovery_regressions::", 1),
    ("db", "verifier_checkpoints::tests::", 1),
    ("db", "evidence_retention::tests::", 1),
    ("db", "verification::tests::", 2),
    ("db", "review_notifications::tests::broad_reminders", 1),
    ("api", "tests::broad_verification_regressions::", 1),
    ("db", "auth::tests::", 1),
    ("db", "security_writes::tests::", 3),
    ("db", "owned_resources::tests::", 2),
    ("db", "claim_security::tests::", 4),
    ("service-runtime", "auth::durable_tests::", 1),
    ("api", "security_auth::tests::", 1),
    ("api", "tests::security_regressions::", 2),
    ("api", "tests::owned_resource_regressions::", 1),
    ("api", "tests::claim_relay_regressions::", 2),
    ("api", "marketplace_readiness::tests::database_snapshots_finish_before_first_shared_head_postgres", 1),
    ("api", "opportunity_source_tests::noncanonical_http_sources_never_contact_rpc_with_live_reader_rows_postgres", 1),
    ("api", "autonomous_projection_tests::autonomous_readers_keep_factory_health_identity_and_boundaries_separate_postgres", 1),
)


def main() -> int:
    if os.environ.get("AGENT_BOUNTIES_LOCAL_TEST_DATABASE") != "1":
        return subprocess.call(
            [sys.executable, "scripts/run-with-test-postgres.py", "--", sys.executable,
             "scripts/check-security-postgres.py"], cwd=ROOT,
        )
    database = os.environ.get("DATABASE_URL", "")
    parsed = urlsplit(database)
    if (parsed.scheme not in ("postgres", "postgresql")
            or parsed.hostname not in ("127.0.0.1", "localhost")
            or parsed.username != "agent_bounties_test"
            or parsed.path != "/agent_bounties_test" or parsed.query or parsed.fragment):
        raise RuntimeError("Security regressions require the isolated local fixture")
    environment = {**os.environ, "SECURITY_AUTH_TEST_DATABASE_URL": database,
                   "AGENT_BOUNTIES_TEST_DATABASE_URL": database}
    cargo = shutil.which("cargo") or shutil.which("cargo.exe") or "cargo"
    for package, prefix, minimum in SUITES:
        command = [cargo, "test", "-p", package, prefix, "--", "--ignored", "--test-threads=1"]
        if prefix.startswith(("opportunity_source_tests::", "autonomous_projection_tests::")):
            # This route-level fixture writes the configured factory identity;
            # isolate it from the rest of the suite, including on assertion failure.
            command = [sys.executable, "scripts/run-with-test-postgres.py", "--", *command]
        print(f"Running durable security suite: {package} {prefix}", flush=True)
        result = subprocess.run(command, cwd=ROOT, env=environment, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(result.stdout, end="", flush=True)
        if result.returncode:
            return result.returncode
        passed = sum(int(count) for count in re.findall(r"test result: ok\. (\d+) passed", result.stdout))
        if passed < minimum:
            raise RuntimeError(f"Missing security tests: {package} {prefix} ran {passed}, expected at least {minimum}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
