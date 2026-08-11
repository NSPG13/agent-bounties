import importlib.util
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("open_competition_v2_proof_rehearsal.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_proof_rehearsal", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2ProofRehearsalTests(unittest.TestCase):
    def test_parameter_schema_has_only_static_fields(self) -> None:
        self.assertEqual(len(MODULE.PARAM_TYPES), 17)
        self.assertNotIn("bytes", MODULE.PARAM_TYPES)
        self.assertEqual(MODULE.PARAM_TYPES[6], "int256")

    def test_function_calldata_matches_erc20_selector(self) -> None:
        data = MODULE.function_data(
            "approve(address,uint256)",
            ["address", "uint256"],
            [MODULE.CREATOR, 7],
        )
        self.assertTrue(data.startswith("0x095ea7b3"))
        self.assertEqual(len(bytes.fromhex(data[2:])), 68)

    def test_canonical_settlement_topic_is_bytes32(self) -> None:
        self.assertRegex(MODULE.SETTLED_TOPIC, r"^0x[0-9a-f]{64}$")

    def test_posix_prover_receives_absolute_fixture_path(self) -> None:
        fixture = Path("target/open-competition-v2-proof-work/fixture.json")
        with mock.patch.object(MODULE.os, "name", "posix"):
            command = MODULE.prover_command(fixture, "groth16")
        self.assertTrue(Path(command[-2]).is_absolute())

    def test_python_reference_journal_matches_rust_golden_vector(self) -> None:
        fixture = json.loads(
            (MODULE.PROGRAM_ROOT / "fixtures/golden-v1.json").read_text(encoding="utf-8")
        )

    def test_prepared_fixture_digest_hashes_exact_cross_platform_bytes(self) -> None:
        fixture = json.loads(
            (MODULE.PROGRAM_ROOT / "fixtures/golden-v1.json").read_text(encoding="utf-8")
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            metadata = MODULE.write_fixture(root, "fixture", fixture)
            raw = (root / "fixture.json").read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), metadata["fixture_sha256"])
        self.assertNotIn(b"\r\n", raw)
        self.assertEqual(
            "0x" + MODULE.expected_journal(fixture).hex(),
            fixture["expected"]["journal_hex"],
        )

    def test_external_proof_artifact_is_bound_to_exact_fixture_and_release(self) -> None:
        fixture = json.loads(
            (MODULE.PROGRAM_ROOT / "fixtures/golden-v1.json").read_text(encoding="utf-8")
        )
        journal = MODULE.expected_journal(fixture)
        scope = fixture["scope"]
        bundle = {
            "metric_profile": {
                "program_vkey": "0x" + bytes(scope["program_vkey"]).hex(),
                "elf_hash": "0x" + bytes(scope["elf_hash"]).hex(),
                "elf_sha256": MODULE.release.ELF_SHA256,
            }
        }
        context = {
            "proofs": {
                "groth16_first": {"journal_sha256": hashlib.sha256(journal).hexdigest()}
            }
        }
        evidence = {
            "mode": "groth16",
            "program_vkey": bundle["metric_profile"]["program_vkey"],
            "elf_keccak256": bundle["metric_profile"]["elf_hash"],
            "elf_sha256": bundle["metric_profile"]["elf_sha256"],
            "proof_hex": "0x01",
            "journal_hex": "0x" + journal.hex(),
        }
        self.assertIs(
            MODULE.validate_proof_evidence(
                bundle, context, "groth16_first", fixture, evidence
            ),
            evidence,
        )
        evidence["journal_hex"] = "0x" + (journal[:-1] + bytes([journal[-1] ^ 1])).hex()
        with self.assertRaisesRegex(ValueError, "prepared fixture"):
            MODULE.validate_proof_evidence(
                bundle, context, "groth16_first", fixture, evidence
            )


if __name__ == "__main__":
    unittest.main()
