#!/usr/bin/env python3
"""Exercise the staging supervisor with real child processes and no credentials."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class CanaryProcessBoundary(unittest.TestCase):
    def test_readonly_children_do_not_inherit_signer_or_provider_secrets(self):
        with tempfile.TemporaryDirectory(prefix="posting-process-boundary-") as folder:
            root = Path(folder)
            stub = root / "indexer"
            stub.write_text("#!/usr/bin/python3\nimport os,json,time\nfrom pathlib import Path\n"
                            + "root=Path(" + repr(folder) + ")\n"
                            + "(root/(os.environ['BASE_INDEXER_PROTOCOL']+'.json')).write_text(json.dumps(dict(os.environ)))\n"
                            + "time.sleep(1)\n")
            stub.chmod(0o700)
            mcp = root / "mcp"
            mcp.write_text("#!/usr/bin/python3\nimport time\ntime.sleep(20)\n")
            mcp.chmod(0o700)
            script = (ROOT / "scripts/run-posting-mcp-canary.sh").read_text()
            script = script.replace("/usr/local/bin/posting-indexer", str(stub)).replace("/usr/local/bin/posting-mcp", str(mcp))
            supervisor = root / "supervisor.sh"
            supervisor.write_text(script)
            env = {**os.environ, "RENDER_SERVICE_ID": "srv-dau3p1favr4c73fij9jg", "DATABASE_URL": "postgres://fixture-only",
                   "CREATOR_OPEN_RELEASE_JSON": '{"network":"base-sepolia"}', "BASE_SEPOLIA_RPC_URL": "https://sepolia.base.org",
                   "PRIVATE_KEY": "must-not-inherit", "GITHUB_APP_PRIVATE_KEY": "must-not-inherit",
                   "GAS_SPONSOR_BUDGET_TOKEN": "must-not-inherit", "RESEND_API_KEY": "must-not-inherit"}
            result = subprocess.run(["bash", str(supervisor)], env=env, capture_output=True, timeout=5)
            self.assertNotEqual(result.returncode, 0, "A stopped worker must stop the container")
            for protocol in ("creator-open-v1", "autonomous-v1"):
                child = json.loads((root / (protocol + ".json")).read_text())
                for secret in ("PRIVATE_KEY", "GITHUB_APP_PRIVATE_KEY", "GAS_SPONSOR_BUDGET_TOKEN", "RESEND_API_KEY"):
                    self.assertNotIn(secret, child)
                self.assertEqual(child["DATABASE_URL"], "postgres://fixture-only")
            legacy = json.loads((root / "autonomous-v1.json").read_text())
            self.assertEqual(legacy["BASE_INDEXER_NETWORK"], "base-sepolia")
            self.assertEqual(legacy["BASE_INDEXER_FACTORY_CONTRACT"], "0xf699c4261709e67ad748c731ac0f7d014677e6e7")
            self.assertEqual(legacy["BASE_INDEXER_MAX_BLOCKS_PER_QUERY"], "1000")


if __name__ == "__main__":
    unittest.main()
