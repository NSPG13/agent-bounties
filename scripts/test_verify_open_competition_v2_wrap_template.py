from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_open_competition_v2_wrap_template.py")
SPEC = importlib.util.spec_from_file_location("verify_v2_wrap_template", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class WrapTemplateTests(unittest.TestCase):
    def fixture(self, root: Path) -> None:
        prover = root / "crates/prover"
        (prover / "scripts").mkdir(parents=True)
        (root / "SP1_CIRCUIT_VERSION").write_text("safe-v2\n", encoding="utf-8")
        (prover / "wrap_vk.bin").write_bytes(b"vk")
        (prover / "wrapped_proof.bin").write_bytes(b"proof")
        (prover / "scripts/regenerate_wrap_template.rs").write_text(
            " ".join(
                (
                    "expected_elf_sha256",
                    "template ELF hash mismatch",
                    "generated wrap template has a stale recursion-vkey root",
                    "generated wrap template is not bound to the template guest vkey",
                    "wrap-template-manifest.json",
                )
            ),
            encoding="utf-8",
        )
        manifest = {
            "schema": MODULE.SCHEMA,
            "circuit_version": "safe-v2",
            "template_elf_sha256": "a" * 64,
            "wrap_vk_sha256": MODULE.sha256(prover / "wrap_vk.bin"),
            "wrapped_proof_sha256": MODULE.sha256(prover / "wrapped_proof.bin"),
        }
        (prover / "wrap-template-manifest.json").write_text(
            json.dumps(manifest), encoding="utf-8"
        )

    def test_hash_bound_pair_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            result = MODULE.verify(root, "safe-v2")
            self.assertEqual(result["status"], "hash_bound")

    def test_proof_replacement_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            (root / "crates/prover/wrapped_proof.bin").write_bytes(b"stale")
            with self.assertRaisesRegex(ValueError, "wrapped_proof_sha256"):
                MODULE.verify(root, "safe-v2")

    def test_circuit_version_mismatch_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with self.assertRaisesRegex(ValueError, "circuit version"):
                MODULE.verify(root, "safe-v3")


if __name__ == "__main__":
    unittest.main()
