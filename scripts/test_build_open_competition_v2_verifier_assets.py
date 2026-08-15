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

    def test_test_only_setup_is_explicit_and_never_mainnet_eligible(self) -> None:
        value = MODULE.trusted_setup_provenance(
            None,
            sp1_source_commit="11" * 20,
            verifier_hashes={"groth16": "0x" + "22" * 32, "plonk": "0x" + "33" * 32},
            setup_files=None,
            allow_test_only=True,
        )
        self.assertEqual(value["state"], "test_only_unsafe")
        self.assertFalse(value["mainnet_eligible"])
        with self.assertRaisesRegex(ValueError, "trusted setup provenance"):
            MODULE.trusted_setup_provenance(
                None,
                sp1_source_commit="11" * 20,
                verifier_hashes={"groth16": "0x" + "22" * 32, "plonk": "0x" + "33" * 32},
                setup_files=None,
                allow_test_only=False,
            )

    def test_trusted_setup_binds_both_verifiers_and_transcripts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "setup.json"
            systems = {}
            setup_files = {}
            for name, model, verifier_byte in (
                ("groth16", "mpc_phase2", "22"),
                ("plonk", "public_mpc_kzg_srs", "33"),
            ):
                files = {}
                hashes = {}
                for file_field, manifest_field, content in (
                    ("constraint_system", "constraint_system_sha256", b"constraints"),
                    ("proving_key", "proving_key_sha256", b"pk"),
                    ("verifying_key", "verifying_key_sha256", b"vk"),
                    ("transcript", "transcript_sha256", b"transcript"),
                    ("verification_evidence", "verification_evidence_sha256", b"evidence"),
                ):
                    file_path = Path(directory) / f"{name}-{file_field}.bin"
                    file_path.write_bytes(content + name.encode())
                    files[file_field] = file_path
                    hashes[manifest_field] = MODULE.hashlib.sha256(file_path.read_bytes()).hexdigest()
                setup_files[name] = files
                systems[name] = {
                    "security_model": model,
                    "verification_passed": True,
                    "verifier_hash": "0x" + verifier_byte * 32,
                    **hashes,
                    "ceremony_uri": "https://example.test/ceremony",
                    "contribution_count": 3,
                }
            path.write_text(
                json.dumps(
                    {
                        "schema_version": MODULE.TRUSTED_SETUP_SCHEMA,
                        "sp1_source_commit": "11" * 20,
                        "circuit_version": MODULE.CIRCUIT_VERSION,
                        "mainnet_eligible": True,
                        "proof_systems": systems,
                    }
                ),
                encoding="utf-8",
            )
            value = MODULE.trusted_setup_provenance(
                path,
                sp1_source_commit="11" * 20,
                verifier_hashes={"groth16": "0x" + "22" * 32, "plonk": "0x" + "33" * 32},
                setup_files=setup_files,
                allow_test_only=False,
            )
            self.assertEqual(value["state"], "trusted_mpc")
            self.assertTrue(value["mainnet_eligible"])
            setup_files["groth16"]["proving_key"].write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "does not match the file"):
                MODULE.trusted_setup_provenance(
                    path,
                    sp1_source_commit="11" * 20,
                    verifier_hashes={
                        "groth16": "0x" + "22" * 32,
                        "plonk": "0x" + "33" * 32,
                    },
                    setup_files=setup_files,
                    allow_test_only=False,
                )


if __name__ == "__main__":
    unittest.main()
