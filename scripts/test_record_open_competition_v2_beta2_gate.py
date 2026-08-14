import copy
import unittest

import record_open_competition_v2_beta2_gate as recorder


def manifest() -> dict:
    gates = {name: False for name in recorder.release.REQUIRED_GATE_NAMES}
    return {
        "beta_risk_preimage": "agent-bounties/open-competition-v2-beta2/risk/test",
        "gates": gates,
        "evidence": {name: None for name in gates},
    }


class GateRecorderTests(unittest.TestCase):
    def test_records_hash_bound_non_owner_evidence(self) -> None:
        value = manifest()
        updated = recorder.record_gate(
            value,
            gate="repository_gate_complete",
            source_commit="11" * 20,
            subject_hash="0x" + "22" * 32,
            evidence_bytes=b"passed\n",
            uri="https://github.com/NSPG13/agent-bounties/actions/runs/1",
            owner_risk_hash=None,
        )
        self.assertTrue(updated["gates"]["repository_gate_complete"])
        self.assertRegex(
            updated["evidence"]["repository_gate_complete"]["evidence_hash"],
            r"^0x[0-9a-f]{64}$",
        )

    def test_owner_gate_requires_exact_risk_acknowledgement(self) -> None:
        value = manifest()
        arguments = dict(
            gate="owner_mainnet_deployment_approved",
            source_commit="11" * 20,
            subject_hash="0x" + "22" * 32,
            evidence_bytes=b"owner approval\n",
            uri="https://github.com/NSPG13/agent-bounties/issues/888",
        )
        with self.assertRaisesRegex(RuntimeError, "risk hash"):
            recorder.record_gate(copy.deepcopy(value), owner_risk_hash=None, **arguments)
        risk_hash = recorder.release.keccak256(value["beta_risk_preimage"].encode())
        updated = recorder.record_gate(value, owner_risk_hash=risk_hash, **arguments)
        self.assertTrue(updated["gates"]["owner_mainnet_deployment_approved"])


if __name__ == "__main__":
    unittest.main()
