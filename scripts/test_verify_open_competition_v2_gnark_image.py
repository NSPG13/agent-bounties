from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify_open_competition_v2_gnark_image.py")
SPEC = importlib.util.spec_from_file_location("verify_v2_gnark_image", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class GnarkImageTests(unittest.TestCase):
    def test_current_image_build_is_exact(self) -> None:
        result = MODULE.verify()
        self.assertEqual(result["status"], "digest_pinned_local_build_with_trusted_setup")

    def test_unpinned_base_fails_closed(self) -> None:
        source = MODULE.DOCKERFILE.read_text(encoding="utf-8").replace(
            MODULE.EXPECTED_BASES[0], "golang:1.26", 1
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Dockerfile"
            path.write_text(source, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "exact digests"):
                MODULE.verify(path, MODULE.WORKFLOW, MODULE.CIRCUIT_BUILDER)

    def test_relative_circuit_mount_fails_closed(self) -> None:
        source = MODULE.CIRCUIT_BUILDER.read_text(encoding="utf-8").replace(
            'source_root="$(cd "$source_root" && pwd)"', "# relative checkout", 1
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build-circuits.sh"
            path.write_text(source, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "absolute"):
                MODULE.verify(MODULE.DOCKERFILE, MODULE.WORKFLOW, path)

    def test_release_rejects_single_party_setup_route(self) -> None:
        source = MODULE.WORKFLOW.read_text(encoding="utf-8")
        source += "\n# bash scripts/build_open_competition_v2_circuits.sh .sp1-safe\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "release.yml"
            path.write_text(source, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "single-party setup"):
                MODULE.verify(MODULE.DOCKERFILE, path, MODULE.CIRCUIT_BUILDER)

    def test_release_rejects_runtime_alias_pull(self) -> None:
        source = MODULE.WORKFLOW.read_text(encoding="utf-8")
        source += '\ndocker pull "$SP1_GNARK_RUNTIME_IMAGE"\n'
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "release.yml"
            path.write_text(source, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "must not pull"):
                MODULE.verify(MODULE.DOCKERFILE, path, MODULE.CIRCUIT_BUILDER)


if __name__ == "__main__":
    unittest.main()
