#!/usr/bin/env python3
"""Seed one short-lived synthetic wallet session in an owned local test container."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import uuid


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--container", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    docker = shutil.which("docker") or shutil.which("docker.exe")
    if not docker:
        parser.error("Docker is required for local wallet fixture setup")
    details = json.loads(subprocess.check_output([docker, "inspect", args.container], text=True))[0]
    labels = details["Config"].get("Labels") or {}
    if labels.get("app.agentbounties.local-smoke") != "true":
        parser.error("wallet fixture setup requires an explicitly labelled local smoke container")
    env = dict(value.split("=", 1) for value in details["Config"]["Env"] if "=" in value)
    challenge_id = str(uuid.uuid4())
    token = "abws_" + secrets.token_hex(32)
    digest = hashlib.sha256(token.encode()).hexdigest()
    # Only generated UUID/hex literals enter this SQL. Wallet-signature exchange
    # is covered separately; this fixture tests HTTP authorization and ownership.
    sql = f"""
BEGIN;
INSERT INTO security_wallet_challenges(id,domain,chain_id,address,message,created_at,expires_at,consumed_at)
VALUES ('{challenge_id}','localhost',8453,'0x2222222222222222222222222222222222222222',
'Synthetic local fixture; wallet signature exchange tested separately',now(),now()+interval '5 minutes',now());
INSERT INTO security_wallet_sessions(token_hash,challenge_id,chain_id,address,created_at,expires_at)
VALUES ('{digest}','{challenge_id}',8453,'0x2222222222222222222222222222222222222222',now(),now()+interval '15 minutes');
COMMIT;
"""
    subprocess.run(
        [docker, "exec", "--interactive", args.container, "psql", "-v", "ON_ERROR_STOP=1",
         "-U", env["POSTGRES_USER"], "-d", env["POSTGRES_DB"]],
        input=sql, text=True, check=True, stdout=subprocess.DEVNULL,
    )
    path = Path(args.output)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w") as output:
        output.write(token)


if __name__ == "__main__":
    main()
