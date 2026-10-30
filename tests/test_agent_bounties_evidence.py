import importlib.util
import json
import os
import sys
import tempfile
import unittest
from unittest.mock import patch

HOOK_PATH = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", ".openhands", "hooks", "agent-bounties-evidence.py")
)
spec = importlib.util.spec_from_file_location("agent_bounties_evidence", HOOK_PATH)
evidence_hook = importlib.util.module_from_spec(spec)
sys.modules["agent_bounties_evidence"] = evidence_hook
spec.loader.exec_module(evidence_hook)


class TestAgentBountiesEvidenceHook(unittest.TestCase):
    def setUp(self):
        self.env_patcher = patch.dict(os.environ, {}, clear=True)
        self.env_patcher.start()

    def tearDown(self):
        self.env_patcher.stop()

    def test_unconfigured_state_source(self):
        code = evidence_hook.main()
        self.assertEqual(code, evidence_hook.ALLOW)

    def test_missing_state_file_blocks(self):
        os.environ["AGENT_BOUNTIES_STATE"] = "/nonexistent/path/state.json"
        code = evidence_hook.main()
        self.assertEqual(code, evidence_hook.BLOCK)

    def test_malformed_state_file_blocks(self):
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            f.write("invalid json{")
            f_path = f.name
        try:
            os.environ["AGENT_BOUNTIES_STATE"] = f_path
            code = evidence_hook.main()
            self.assertEqual(code, evidence_hook.BLOCK)
        finally:
            os.unlink(f_path)

    def test_no_active_claim_allows(self):
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            json.dump({"active": False}, f)
            f_path = f.name
        try:
            os.environ["AGENT_BOUNTIES_STATE"] = f_path
            code = evidence_hook.main()
            self.assertEqual(code, evidence_hook.ALLOW)
        finally:
            os.unlink(f_path)

    def test_active_claim_missing_evidence_blocks(self):
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            json.dump({
                "claim": {"active": True, "bounty_id": "123"},
                "evidence": {"repository": "owner/repo"}
            }, f)
            f_path = f.name
        try:
            os.environ["AGENT_BOUNTIES_STATE"] = f_path
            code = evidence_hook.main()
            self.assertEqual(code, evidence_hook.BLOCK)
        finally:
            os.unlink(f_path)

    def test_active_claim_all_evidence_allows(self):
        full_evidence = {
            "repository": "owner/repo",
            "commit": "abc1234",
            "test_command": "pytest",
            "source_snapshot_digest": "sha256:123",
            "discovery_source": "github",
            "participation_reason": "fix bug",
            "improvement_feedback": "none",
        }
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            json.dump({
                "claim": {"active": True, "bounty_id": "123"},
                "evidence": full_evidence
            }, f)
            f_path = f.name
        try:
            os.environ["AGENT_BOUNTIES_STATE"] = f_path
            code = evidence_hook.main()
            self.assertEqual(code, evidence_hook.ALLOW)
        finally:
            os.unlink(f_path)

    def test_settled_claim_allows(self):
        state = {
            "claim": {
                "active": True,
                "bounty_id": "123",
                "contract_address": "0x111",
                "round": "1",
                "solver": "0x222",
            },
            "settlement": {
                "canonical_event": {
                    "kind": "BountySettled",
                    "provenance": "canonical_live",
                    "tx_hash": "0xabc",
                    "log_key": "log_0",
                    "block_number": 100,
                    "network": "base-mainnet",
                    "bounty_id": "123",
                    "contract_address": "0x111",
                    "round": "1",
                    "solver": "0x222",
                }
            }
        }
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            json.dump(state, f)
            f_path = f.name
        try:
            os.environ["AGENT_BOUNTIES_STATE"] = f_path
            code = evidence_hook.main()
            self.assertEqual(code, evidence_hook.ALLOW)
        finally:
            os.unlink(f_path)

    def test_parse_state_cmd(self):
        argv = evidence_hook.parse_state_cmd('["python", "-m", "producer"]')
        self.assertEqual(argv, ["python", "-m", "producer"])

        argv_str = evidence_hook.parse_state_cmd('python -m producer')
        self.assertEqual(argv_str, ["python", "-m", "producer"])


if __name__ == "__main__":
    unittest.main()
