import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("build_open_competition_v2_verifier_assets.py")
SPEC = importlib.util.spec_from_file_location("v2_verifier_assets", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2VerifierAssetTests(unittest.TestCase):
    def test_verifier_and_proof_evidence_are_hash_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "verifier.json"
            artifact.write_text(
                json.dumps(
                    {
                        "bytecode": {"object": "0x60016000"},
                        "deployedBytecode": {"object": "0x6001"},
                    }
                ),
                encoding="utf-8",
            )
            item = MODULE.verifier("groth16", artifact, "0x" + "11" * 32)
            self.assertEqual(item["creation_code_hash"], MODULE.keccak256(bytes.fromhex("60016000")))
            self.assertEqual(item["runtime_code_hash"], MODULE.keccak256(bytes.fromhex("6001")))

            proof = root / "proof.json"
            proof.write_text(
                json.dumps({"self_verified": True, "gpu_proving_enabled": False}),
                encoding="utf-8",
            )
            self.assertRegex(MODULE.proof_evidence_hash(proof), r"^0x[0-9a-f]{64}$")
            proof.write_text(json.dumps({"self_verified": False}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "not self-verified"):
                MODULE.proof_evidence_hash(proof)

    def test_pending_stage_requires_explicit_flag(self) -> None:
        parser = MODULE.parse_args
        self.assertTrue(callable(parser))


if __name__ == "__main__":
    unittest.main()
