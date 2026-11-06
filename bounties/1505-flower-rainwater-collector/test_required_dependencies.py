"""Requested CAD generation must fail when its tool is unavailable."""
import builtins
import os
from pathlib import Path
import runpy
import sys
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


class RequiredDependencyTests(unittest.TestCase):
    def test_requested_render_does_not_use_stale_export(self):
        module = runpy.run_path(str(HERE / "test_geometry.py"))
        with patch.dict(os.environ, {}, clear=True), \
             patch("shutil.which", return_value=None), \
             patch.object(Path, "exists", return_value=False), \
             patch("subprocess.run") as execute:
            with self.assertRaisesRegex(SystemExit, "requires OpenSCAD"):
                module["render_stl"](HERE / "unused.stl")
            execute.assert_not_called()

    def test_requested_step_export_requires_ocp(self):
        module = runpy.run_path(str(HERE / "export_step.py"))
        original_import = builtins.__import__

        def unavailable(name, *args, **kwargs):
            if name == "OCP" or name.startswith("OCP."):
                raise ImportError("forced missing dependency")
            return original_import(name, *args, **kwargs)

        with patch("builtins.__import__", side_effect=unavailable), \
             patch.object(sys, "argv", ["export_step.py"]):
            self.assertEqual(module["main"](), 1)


if __name__ == "__main__":
    unittest.main()
