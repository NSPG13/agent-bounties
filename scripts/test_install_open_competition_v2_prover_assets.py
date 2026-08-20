import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import install_open_competition_v2_prover_assets as installer


class ProverAssetInstallerTests(unittest.TestCase):
    def fixture(self, root: Path) -> None:
        records = {}
        for system in ("groth16", "plonk"):
            directory = root / system
            directory.mkdir(parents=True)
            hashes = {}
            for field, template in installer.SYSTEM_FILES.items():
                path = directory / template.format(system=system)
                path.write_bytes(f"{system}:{field}".encode())
                hashes[field] = hashlib.sha256(path.read_bytes()).hexdigest()
            records[system] = {"verification_passed": True, **hashes}
        (root / "trusted-setup.json").write_text(
            json.dumps(
                {
                    "schema_version": installer.SCHEMA,
                    "circuit_version": "safe-v1",
                    "proof_systems": records,
                }
            ),
            encoding="utf-8",
        )

    def test_installs_the_version_below_each_sp1_base_path_idempotently(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            trusted = root / "trusted"
            trusted.mkdir()
            self.fixture(trusted)
            target = root / "runtime"
            first = installer.install(trusted, target, "safe-v1")
            second = installer.install(trusted, target, "safe-v1")
            for system in ("groth16", "plonk"):
                versioned = target / system / "safe-v1"
                self.assertTrue((versioned / ".complete").is_file())
                self.assertEqual(first["proof_systems"][system], second["proof_systems"][system])

    def test_rejects_a_mutated_setup_asset(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            trusted = Path(directory) / "trusted"
            trusted.mkdir()
            self.fixture(trusted)
            (trusted / "groth16" / "groth16_pk.bin").write_bytes(b"mutated")
            with self.assertRaisesRegex(ValueError, "proving_key_sha256 mismatch"):
                installer.install(trusted, Path(directory) / "runtime", "safe-v1")


if __name__ == "__main__":
    unittest.main()
