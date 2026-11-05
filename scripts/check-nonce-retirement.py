#!/usr/bin/env python3
"""Exercise nonce retirement on a disposable local EVM and PostgreSQL only."""
from pathlib import Path
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
TEST = "nonce_retirement::local_rehearsal::"


def main() -> int:
    if os.environ.get("AGENT_BOUNTIES_LOCAL_TEST_DATABASE") != "1":
        return subprocess.call([sys.executable, "scripts/run-with-test-postgres.py", "--",
                                sys.executable, "scripts/check-nonce-retirement.py"], cwd=ROOT)
    anvil, cargo = shutil.which("anvil"), shutil.which("cargo")
    if not anvil or not cargo:
        raise RuntimeError("Anvil and Cargo are required; no remote-chain fallback exists")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    url = f"http://127.0.0.1:{port}"
    # Anvil's well-known development keys stay out of test output.
    with tempfile.TemporaryFile() as log:
        node = subprocess.Popen([anvil, "--host", "127.0.0.1", "--port", str(port),
                                 "--chain-id", "8453", "--no-mining"], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if node.poll() is not None:
                    raise RuntimeError("Disposable Anvil exited during startup")
                try:
                    req = urllib.request.Request(url, data=json.dumps({"jsonrpc":"2.0", "id":1,
                        "method":"eth_chainId", "params":[]}).encode(), headers={"Content-Type":"application/json"})
                    with urllib.request.urlopen(req, timeout=1) as response:
                        if json.load(response).get("result") == "0x2105":
                            break
                except (OSError, ValueError):
                    pass
                time.sleep(0.1)
            else:
                raise RuntimeError("Disposable Anvil did not become ready")
            environment = {**os.environ, "RETIREMENT_LOCAL_ANVIL_URL": url,
                           "AGENT_BOUNTIES_TEST_DATABASE_URL": os.environ["DATABASE_URL"]}
            result = subprocess.run([cargo, "test", "-p", "api", TEST, "--", "--ignored",
                                     "--test-threads=1", "--nocapture"], cwd=ROOT, env=environment,
                                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            print(result.stdout, end="")
            if result.returncode or not re.search(r"test result: ok\. 2 passed", result.stdout):
                raise RuntimeError("Nonce retirement local-chain rehearsal did not pass")
        finally:
            node.terminate()
            try:
                node.wait(timeout=5)
            except subprocess.TimeoutExpired:
                node.kill()
                node.wait()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
