import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import deploy_open_competition_v2_beta3 as deploy


def fixture() -> dict:
    subject = "0x" + "11" * 32
    gates = {name: True for name in deploy.release.PRELAUNCH_GATE_NAMES}
    evidence = {
        name: {
            "subject_hash": subject,
            "source_commit": "22" * 20,
            "evidence_hash": "0x" + "33" * 32,
            "uri": "https://example.test/evidence",
        }
        for name in deploy.release.PRELAUNCH_GATE_NAMES
    }
    components = ("groth16_verifier", "plonk_verifier", "factory")
    transactions = [
        {
            "component": name,
            "from_nonce": 4 + index,
            "predicted_address": f"0x{index + 1:040x}",
            "data": "0x01",
        }
        for index, name in enumerate(components)
    ]
    value = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-release-bundle-v1",
        "protocol_version": "agent-bounties/open-competition-v2-beta3",
        "network": "base-mainnet",
        "chain_id": 8453,
        "source_commit": "22" * 20,
        "deployer": "0x1111111111111111111111111111111111111111",
        "repository_subject": {"hash": subject},
        "activation": {"mainnet_signing_allowed": True},
        "release_gates": {
            "prelaunch_complete": True,
            "gates": gates,
            "evidence": evidence,
        },
        "deployment_transactions": transactions,
    }
    for transaction in transactions:
        value[transaction["component"]] = {"address": transaction["predicted_address"]}
    return value


class DeploymentValidationTests(unittest.TestCase):
    def test_accepts_exact_approved_bundle(self) -> None:
        deploy.validate_bundle(fixture(), "0x1111111111111111111111111111111111111111")

    def test_rejects_false_gate(self) -> None:
        value = fixture()
        value["release_gates"]["gates"][deploy.release.PRELAUNCH_GATE_NAMES[0]] = False
        with self.assertRaisesRegex(RuntimeError, "prelaunch gate is false"):
            deploy.validate_bundle(value, value["deployer"])

    def test_rejects_wrong_signer_or_nonce_order(self) -> None:
        value = fixture()
        with self.assertRaisesRegex(RuntimeError, "signer differs"):
            deploy.validate_bundle(value, "0x2222222222222222222222222222222222222222")
        value["deployment_transactions"][1]["from_nonce"] = 9
        with self.assertRaisesRegex(RuntimeError, "nonces are not contiguous"):
            deploy.validate_bundle(value, value["deployer"])

    def test_resume_evidence_must_match_the_exact_release(self) -> None:
        value = fixture()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "deployment.json"
            evidence = deploy.initial_evidence(value)
            output.write_text(json.dumps(evidence), encoding="utf-8")
            self.assertEqual(deploy.load_evidence(value, output), evidence)
            value["repository_subject"]["hash"] = "0x" + "ff" * 32
            with self.assertRaisesRegex(RuntimeError, "another release"):
                deploy.load_evidence(value, output)

    def test_safe_runtime_check_uses_one_canonical_safe_block(self) -> None:
        client = object.__new__(deploy.SignedRpc)
        client.wait_safe = mock.Mock(return_value={"number": "0x2a"})
        client.code_hash = mock.Mock(return_value="0xexact")

        addresses = ("0x" + "12" * 20, "0x" + "34" * 20)
        client.require_safe_code_hashes(
            {address: "0xexact" for address in addresses}, 41
        )

        client.wait_safe.assert_called_once_with(41)
        self.assertEqual(
            client.code_hash.call_args_list,
            [mock.call(address, "0x2a") for address in addresses],
        )


if __name__ == "__main__":
    unittest.main()
