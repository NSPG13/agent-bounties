#!/usr/bin/env python3
"""Run one local test command with a disposable, loopback-only PostgreSQL database."""

from __future__ import annotations

import argparse
import json
import os
import secrets
import shutil
import subprocess
import time
import uuid
from urllib.parse import urlsplit


def validate_existing_fixture() -> None:
    url = urlsplit(os.getenv("DATABASE_URL", ""))
    if (
        os.getenv("AGENT_BOUNTIES_LOCAL_TEST_DATABASE") != "1"
        or url.scheme not in ("postgres", "postgresql")
        or url.hostname not in ("127.0.0.1", "localhost")
        or url.query
        or url.fragment
    ):
        raise RuntimeError("SDK fixture writes require the isolated local PostgreSQL test database")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validate-existing", action="store_true", help="check an inherited local fixture without starting a container")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.validate_existing:
        validate_existing_fixture()
        return 0
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("provide a test command after --")
    docker = shutil.which("docker") or shutil.which("docker.exe")
    if not docker:
        parser.error("Docker is required for the isolated PostgreSQL fixture")

    name = f"agent-bounties-test-{uuid.uuid4().hex[:12]}"
    password = secrets.token_hex(24)
    container_env = {**os.environ, "POSTGRES_PASSWORD": password}
    try:
        subprocess.run(
            [docker, "run", "--detach", "--rm", "--name", name,
             "--label", "app.agentbounties.local-smoke=true",
             "--publish", "127.0.0.1::5432", "--env", "POSTGRES_PASSWORD",
             "--env", "POSTGRES_USER=agent_bounties_test",
             "--env", "POSTGRES_DB=agent_bounties_test", "postgres:16"],
            env=container_env, check=True, stdout=subprocess.DEVNULL,
        )
        deadline = time.monotonic() + 60
        while True:
            ready = subprocess.run(
                [docker, "exec", name, "pg_isready", "-h", "127.0.0.1", "-U", "agent_bounties_test",
                 "-d", "agent_bounties_test"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            if ready.returncode == 0:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("Local PostgreSQL fixture did not become ready within 60 seconds")
            time.sleep(0.25)
        details = json.loads(subprocess.check_output([docker, "inspect", name], text=True))[0]
        binding = details["NetworkSettings"]["Ports"]["5432/tcp"][0]
        if binding["HostIp"] != "127.0.0.1":
            raise RuntimeError("PostgreSQL fixture was not bound exclusively to loopback")
        port = int(binding["HostPort"])
        database_url = f"postgres://agent_bounties_test:{password}@127.0.0.1:{port}/agent_bounties_test"
        fixture_env = {
            **os.environ,
            "DATABASE_URL": database_url,
            "AGENT_BOUNTIES_TEST_DATABASE_URL": database_url,
            "AGENT_BOUNTIES_LOCAL_TEST_DATABASE": "1",
            "AGENT_BOUNTIES_TEST_POSTGRES_CONTAINER": name,
            "API_UPSTREAM_BASE_URL": "",
            "ENABLE_BASE_TX_BROADCAST": "false",
            "ENABLE_X402_HOSTED_RELAY": "false",
            "ENABLE_STRIPE_LIVE_EXECUTION": "false",
            "ENABLE_STRIPE_PUBLIC_CHECKOUT": "false",
        }
        print("Running local fixture command with isolated PostgreSQL", flush=True)
        return subprocess.run(command, env=fixture_env).returncode
    finally:
        subprocess.run([docker, "rm", "--force", name], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    raise SystemExit(main())
