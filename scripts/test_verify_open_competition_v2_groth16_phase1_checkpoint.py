import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_open_competition_v2_groth16_phase1_checkpoint.py")
SPEC = importlib.util.spec_from_file_location("v2_groth16_phase1_checkpoint", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


class Groth16Phase1CheckpointTests(unittest.TestCase):
    def fixture(self, root: Path) -> tuple[Path, tuple[str, str, str, str, str]]:
        values = {
            "circuit.bin": b"r1cs",
            "phase1-1.bin": b"first",
            "phase1-2.bin": b"second",
            "phase1-commons.bin": b"commons",
        }
        for name, value in values.items():
            (root / name).write_bytes(value)
        hashes = {name: digest(value) for name, value in values.items()}
        beacon = "0x" + "12" * 32
        record = {
            "schema_version": MODULE.COMMAND_SCHEMA,
            "command": "verify-phase1",
            "verified": True,
            "inputs": [
                {"path": name, "sha256": hashes[name]}
                for name in ("circuit.bin", "phase1-1.bin", "phase1-2.bin")
            ],
            "outputs": [{"path": "phase1-commons.bin", "sha256": hashes["phase1-commons.bin"]}],
            "beacon_hex": beacon,
        }
        (root / "05-phase1-verify.json").write_text(json.dumps(record), encoding="utf-8")
        (root / "phase1-beacon.json").write_text(
            json.dumps({"round": 10, "randomness": beacon[2:]}), encoding="utf-8"
        )
        return root / "circuit.bin", (
            hashes["circuit.bin"],
            hashes["phase1-1.bin"],
            hashes["phase1-2.bin"],
            hashes["phase1-commons.bin"],
            beacon,
        )

    def test_accepts_exact_checkpoint(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            r1cs, expected = self.fixture(root)
            MODULE.verify_checkpoint(root, r1cs, *expected)

    def test_rejects_file_record_and_beacon_drift(self) -> None:
        for mutate in ("file", "record", "beacon"):
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                r1cs, expected = self.fixture(root)
                if mutate == "file":
                    (root / "phase1-commons.bin").write_bytes(b"changed")
                elif mutate == "record":
                    record = json.loads((root / "05-phase1-verify.json").read_text())
                    record["verified"] = False
                    (root / "05-phase1-verify.json").write_text(json.dumps(record))
                else:
                    (root / "phase1-beacon.json").write_text(
                        json.dumps({"round": 10, "randomness": "34" * 32})
                    )
                with self.assertRaises(ValueError):
                    MODULE.verify_checkpoint(root, r1cs, *expected)


if __name__ == "__main__":
    unittest.main()
