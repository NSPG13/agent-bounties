import importlib.util
import hashlib
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_open_competition_v2_groth16_ceremony.py")
RUNNER = Path(__file__).with_name("run_open_competition_v2_groth16_ceremony.sh")
SPEC = importlib.util.spec_from_file_location("v2_groth16_ceremony", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class Groth16CeremonyTests(unittest.TestCase):
    def test_runner_bundles_the_exact_constraint_system(self) -> None:
        source = RUNNER.read_text(encoding="utf-8")
        copy = 'cp --reflink=auto "$r1cs" "$output_dir/groth16_circuit.bin"'
        self.assertIn(copy, source)
        self.assertLess(source.index(copy), source.index("container init-phase1"))

    def test_inventory_rejects_ambiguous_or_invalid_entries(self) -> None:
        with self.assertRaisesRegex(ValueError, "ambiguous"):
            MODULE.inventory(
                {"inputs": [{"path": "a/b", "sha256": "11" * 32}]}, "inputs"
            )
        with self.assertRaisesRegex(ValueError, "digest"):
            MODULE.inventory({"inputs": [{"path": "a", "sha256": "bad"}]}, "inputs")
        self.assertEqual(
            MODULE.inventory({"inputs": [{"path": "a", "sha256": "11" * 32}]}, "inputs"),
            {"a": "11" * 32},
        )

    def test_verifies_exact_ordered_hash_chain_and_final_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            r1cs = root / "groth16_circuit.bin"
            r1cs.write_bytes(b"r1cs")
            for name in ("groth16_pk.bin", "groth16_vk.bin", "Groth16Verifier.sol"):
                (root / name).write_bytes(name.encode())

            def digest(name: str, value: str) -> dict[str, object]:
                return {"path": name, "sha256": value, "bytes": 1}

            def record(command: str, inputs: list, outputs: list, **extra: object) -> dict:
                return {
                    "schema_version": MODULE.COMMAND_SCHEMA,
                    "command": command,
                    "inputs": inputs,
                    "outputs": outputs,
                    "verified": True,
                    **extra,
                }

            h = {name: hashlib.sha256(name.encode()).hexdigest() for name in (
                "p1-init", "p1-1", "p1-2", "commons",
                "p2-init", "p2-1", "p2-2",
            )}
            r1cs_item = digest(r1cs.name, hashlib.sha256(r1cs.read_bytes()).hexdigest())
            records = [
                record("init-phase1", [r1cs_item], [digest("phase1-init.bin", h["p1-init"])]),
                record("contribute-phase1", [digest("phase1-init.bin", h["p1-init"])], [digest("phase1-1.bin", h["p1-1"])], contribution_id=1),
                record("contribute-phase1", [digest("phase1-1.bin", h["p1-1"])], [digest("phase1-2.bin", h["p1-2"])], contribution_id=2),
                record("verify-phase1", [r1cs_item, digest("phase1-1.bin", h["p1-1"]), digest("phase1-2.bin", h["p1-2"])], [digest("phase1-commons.bin", h["commons"])], beacon_hex="0x" + "11" * 32),
                record("init-phase2", [r1cs_item, digest("phase1-commons.bin", h["commons"])], [digest("phase2-init.bin", h["p2-init"])]),
                record("contribute-phase2", [digest("phase2-init.bin", h["p2-init"])], [digest("phase2-1.bin", h["p2-1"])], contribution_id=1),
                record("contribute-phase2", [digest("phase2-1.bin", h["p2-1"])], [digest("phase2-2.bin", h["p2-2"])], contribution_id=2),
                record("finalize", [r1cs_item, digest("phase1-commons.bin", h["commons"]), digest("phase2-1.bin", h["p2-1"]), digest("phase2-2.bin", h["p2-2"])], [
                    digest(name, hashlib.sha256((root / name).read_bytes()).hexdigest())
                    for name in ("groth16_pk.bin", "groth16_vk.bin", "Groth16Verifier.sol")
                ], beacon_hex="0x" + "22" * 32),
            ]
            transcript = {
                "schema_version": MODULE.TRANSCRIPT_SCHEMA,
                "records": records,
                "phase1_beacon": {"round": 1, "randomness": "11" * 32},
                "phase2_beacon": {"round": 2, "randomness": "22" * 32},
            }
            self.assertEqual(MODULE.verify(transcript, root, r1cs)[0], 2)
            records[6]["inputs"][0]["sha256"] = "ff" * 32
            with self.assertRaisesRegex(ValueError, "hash chain is broken"):
                MODULE.verify(transcript, root, r1cs)


if __name__ == "__main__":
    unittest.main()
