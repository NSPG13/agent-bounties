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
        manifest = {"name": "MAIN IGNITION", "participants": [{}] * 176}
        log = "success ✅: all contributions are valid\nsuccess ✅: kzg sanity check with SRS"
        final, count = MODULE.contribution_count(log, manifest)
        self.assertEqual((final, count), (176, 2))
        for invalid_log, invalid_manifest in (
            ("success ✅: all contributions are valid", manifest),
            (log, {"name": "MAIN IGNITION", "participants": [{}] * 175}),
            ("processing contribution 176\n" + log, manifest),
            (log, {"name": "TINY_TEST_5", "participants": [{}] * 176}),
        ):
            with self.assertRaises(ValueError):
                MODULE.contribution_count(invalid_log, invalid_manifest)

    def test_matches_manifest_entries_after_initial_pair(self) -> None:
        manifest = {"name": "MAIN IGNITION", "participants": [{}] * 178}
        log = (
            "processing contribution 177\nprocessing contribution 178\n"
            "success ✅: all contributions are valid\n"
            "success ✅: kzg sanity check with SRS"
        )
        self.assertEqual(MODULE.contribution_count(log, manifest), (178, 4))

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
