from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-public-handoffs.py")
SPEC = importlib.util.spec_from_file_location("check_public_handoffs", SCRIPT)
assert SPEC and SPEC.loader
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class PublicHandoffCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for relative in CHECK.ADVERTISEMENT_SOURCES:
            source = self.root / relative
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "\n".join(f'"{CHECK.PUBLIC_ORIGIN}/{path}"' for path in CHECK.HANDOFF_BOUNDARIES),
                encoding="utf-8",
            )
        site = self.root / "site"
        site.mkdir()
        (site / "shared.js").write_text("", encoding="utf-8")
        for path, phrases in CHECK.HANDOFF_BOUNDARIES.items():
            (site / path).write_text(
                '<meta name="robots" content="noindex, nofollow"><script src="shared.js"></script>'
                + " ".join(phrases),
                encoding="utf-8",
            )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_complete_handoff_surface_passes(self) -> None:
        self.assertEqual(CHECK.check_local(self.root), [])

    def test_missing_advertised_page_fails(self) -> None:
        missing = self.root / "site" / "onramp.html"
        missing.unlink()
        errors = CHECK.check_local(self.root)
        self.assertTrue(any("onramp.html" in error and "no site file" in error for error in errors))

    def test_missing_evidence_boundary_fails(self) -> None:
        page = self.root / "site" / "success.html"
        page.write_text('<meta name="robots" content="noindex, nofollow">', encoding="utf-8")
        errors = CHECK.check_local(self.root)
        self.assertTrue(any("success.html" in error and "missing evidence boundary" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
