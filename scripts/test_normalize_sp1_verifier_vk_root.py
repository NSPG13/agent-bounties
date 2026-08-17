from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("normalize_sp1_verifier_vk_root.py")
SPEC = importlib.util.spec_from_file_location("normalize_sp1_vk_root", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def verifier(root: str) -> str:
    return f"""
contract SP1Verifier {{
    function VK_ROOT() public pure returns (bytes32) {{
        return 0x{root};
    }}
}}
"""


class NormalizeVkRootTests(unittest.TestCase):
    def write(self, directory: str, name: str, root: str) -> Path:
        path = Path(directory) / name
        path.write_text(verifier(root), encoding="utf-8")
        return path

    def test_removes_only_redundant_zero_byte(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            expected = "2f" * 32
            groth = self.write(directory, "groth.sol", "00" + expected)
            plonk = self.write(directory, "plonk.sol", "00" + expected)
            result = MODULE.normalize_all([groth, plonk])
            self.assertEqual(result["vk_root"], "0x" + expected)
            self.assertIn("return 0x" + expected, groth.read_text(encoding="utf-8"))

    def test_is_idempotent_for_canonical_bytes32(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            expected = "ab" * 32
            path = self.write(directory, "verifier.sol", expected)
            self.assertEqual(MODULE.normalize(path), "0x" + expected)
            self.assertEqual(MODULE.normalize(path), "0x" + expected)

    def test_nonzero_extra_byte_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(directory, "verifier.sol", "01" + "ab" * 32)
            with self.assertRaisesRegex(ValueError, "32 bytes"):
                MODULE.normalize(path)

    def test_proof_system_root_mismatch_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            groth = self.write(directory, "groth.sol", "11" * 32)
            plonk = self.write(directory, "plonk.sol", "22" * 32)
            with self.assertRaisesRegex(ValueError, "VK roots differ"):
                MODULE.normalize_all([groth, plonk])


if __name__ == "__main__":
    unittest.main()
