import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_open_competition_v2_plonk_setup.py")
SPEC = importlib.util.spec_from_file_location("v2_plonk_setup", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class PlonkSetupTests(unittest.TestCase):
    def test_requires_chain_and_kzg_success(self) -> None:
        log = "processing contribution 176\nsuccess ✅: all contributions are valid\nsuccess ✅: kzg sanity check with SRS"
        final, count = MODULE.contribution_count(log)
        self.assertEqual((final, count), (176, 2))
        for invalid in (
            "processing contribution 176\nsuccess ✅: all contributions are valid",
            "processing contribution 175\nsuccess ✅: all contributions are valid\nsuccess ✅: kzg sanity check with SRS",
        ):
            with self.assertRaises(ValueError):
                MODULE.contribution_count(invalid)

    def test_pins_safe_source_branches(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            build = root / "build.go"
            setup = root / "trusted_setup.go"
            build.write_text(
                'DownloadAndSaveAztecIgnitionSrs(174, srsFileName)\nif !strings.Contains(dataDir, "dev")',
                encoding="utf-8",
            )
            setup.write_text(
                'BaseURL:  "https://aztec-ignition.s3.amazonaws.com/"\n'
                'Ceremony: "MAIN IGNITION"\nif !next.Follows(&current)\n'
                'if !next.Follows(&current)\nsanityCheck(&srs)',
                encoding="utf-8",
            )
            value = MODULE.verify_source(build, setup)
            self.assertRegex(value["build_go_sha256"], r"^[0-9a-f]{64}$")
            setup.write_text(setup.read_text().replace("MAIN IGNITION", "TINY_TEST_5"))
            with self.assertRaisesRegex(ValueError, "requires 1 exact occurrences"):
                MODULE.verify_source(build, setup)

    def test_requires_both_chain_guards(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            build = root / "build.go"
            setup = root / "trusted_setup.go"
            build.write_text(
                'DownloadAndSaveAztecIgnitionSrs(174, srsFileName)\nif !strings.Contains(dataDir, "dev")',
                encoding="utf-8",
            )
            setup.write_text(
                'BaseURL:  "https://aztec-ignition.s3.amazonaws.com/"\n'
                'Ceremony: "MAIN IGNITION"\nif !next.Follows(&current)\nsanityCheck(&srs)',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "requires 2 exact occurrences"):
                MODULE.verify_source(build, setup)

    def test_replayed_srs_must_match_build_srs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = root / "expected.bin"
            replayed = root / "replayed.bin"
            expected.write_bytes(b"canonical-srs")
            replayed.write_bytes(b"canonical-srs")
            self.assertEqual(MODULE.require_matching_srs(expected, replayed), MODULE.sha256(expected))
            replayed.write_bytes(b"different-srs")
            with self.assertRaisesRegex(ValueError, "differs from the build SRS"):
                MODULE.require_matching_srs(expected, replayed)


if __name__ == "__main__":
    unittest.main()
