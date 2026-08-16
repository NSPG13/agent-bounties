import importlib.util
import hashlib
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("build_open_competition_v2_trusted_setup_manifest.py")
SPEC = importlib.util.spec_from_file_location("v2_setup_manifest", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class TrustedSetupManifestTests(unittest.TestCase):
    def fixture(self, root: Path, system: str) -> tuple[dict[str, Path], Path]:
        files = {}
        for field in ("constraint_system", "proving_key", "verifying_key", "transcript"):
            path = root / f"{system}-{field}.bin"
            path.write_bytes(f"{system}-{field}".encode())
            files[field] = path
        evidence = root / f"{system}-evidence.json"
        evidence.write_text(
            json.dumps(
                {
                    "schema_version": "agent-bounties/open-competition-v2-beta3-setup-verification-evidence-v1",
                    "proof_system": system,
                    "security_model": MODULE.MODELS[system],
                    "verification_passed": True,
                    "constraint_system_sha256": hashlib.sha256(files["constraint_system"].read_bytes()).hexdigest(),
                    "proving_key_sha256": hashlib.sha256(files["proving_key"].read_bytes()).hexdigest(),
                    "verifying_key_sha256": hashlib.sha256(files["verifying_key"].read_bytes()).hexdigest(),
                    "transcript_sha256": hashlib.sha256(files["transcript"].read_bytes()).hexdigest(),
                    "verifier_hash": "0x" + ("11" if system == "groth16" else "22") * 32,
                    "contribution_count": 3,
                    "ceremony_uri": f"https://example.test/{system}",
                }
            ),
            encoding="utf-8",
        )
        return files, evidence

    def test_system_record_binds_every_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files, evidence = self.fixture(root, "groth16")
            args = type("Args", (), {})()
            for name, path in files.items():
                setattr(args, f"groth16_{name}", path)
            args.groth16_verification_evidence = evidence
            args.groth16_verifier_hash = "0x" + "11" * 32
            value = MODULE.system_record(args, "groth16")
            self.assertEqual(value["contribution_count"], 3)
            files["proving_key"].write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "proving_key_sha256 mismatch"):
                MODULE.system_record(args, "groth16")

    def test_rejects_unverified_or_single_party_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files, evidence = self.fixture(root, "plonk")
            value = json.loads(evidence.read_text())
            value["contribution_count"] = 1
            evidence.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, "at least two contributions"):
                MODULE.verification_evidence(
                    evidence,
                    system="plonk",
                    constraint_hash=hashlib.sha256(files["constraint_system"].read_bytes()).hexdigest(),
                    proving_key_hash=hashlib.sha256(files["proving_key"].read_bytes()).hexdigest(),
                    verifying_key_hash=hashlib.sha256(files["verifying_key"].read_bytes()).hexdigest(),
                    transcript_hash=hashlib.sha256(files["transcript"].read_bytes()).hexdigest(),
                )


if __name__ == "__main__":
    unittest.main()
