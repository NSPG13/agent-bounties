"""Packaging smoke: the package installs (editable dry-run + real wheel into a
fresh venv) and the installed console script speaks MCP over stdio."""
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
PKG_DIR = ROOT


def _pip_available() -> bool:
    try:
        proc = subprocess.run(
            [sys.executable, "-m", "pip", "--version"],
            capture_output=True, text=True, timeout=60,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return proc.returncode == 0


class PackagingInstallTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not _pip_available():
            raise unittest.SkipTest("pip is not available")

    def _venv_python(self) -> str:
        """Fresh venv for interpreters whose system pip is PEP-668 managed."""
        venv_dir = pathlib.Path(tempfile.mkdtemp(prefix="tma-dryrun-"))
        self.addCleanup(shutil.rmtree, venv_dir, ignore_errors=True)
        build = subprocess.run(
            [sys.executable, "-m", "venv", str(venv_dir)],
            capture_output=True, text=True, timeout=300,
        )
        if build.returncode != 0:
            self.skipTest(f"venv unavailable: {build.stderr[:200]}")
        bin_name = "Scripts" if os.name == "nt" else "bin"
        return str(venv_dir / bin_name / ("python.exe" if os.name == "nt" else "python"))

    def test_editable_dry_run_installs(self):
        # The exact command the maintainer review ran.
        proc = subprocess.run(
            [sys.executable, "-m", "pip", "install", "--dry-run", "-e", str(PKG_DIR)],
            capture_output=True, text=True, timeout=600,
        )
        if proc.returncode != 0 and "externally-managed-environment" in (proc.stdout + proc.stderr):
            # System python refuses installs; prove installability in a venv instead.
            proc = subprocess.run(
                [self._venv_python(), "-m", "pip", "install", "--dry-run", "-e", str(PKG_DIR)],
                capture_output=True, text=True, timeout=900,
            )
        self.assertEqual(proc.returncode, 0,
                         f"editable install failed:\n{proc.stdout}\n{proc.stderr}")

    def test_installed_console_script_speaks_mcp(self):
        venv_dir = pathlib.Path(tempfile.mkdtemp(prefix="tma-venv-"))
        self.addCleanup(shutil.rmtree, venv_dir, ignore_errors=True)
        build = subprocess.run(
            [sys.executable, "-m", "venv", str(venv_dir)],
            capture_output=True, text=True, timeout=300,
        )
        if build.returncode != 0:
            self.skipTest(f"venv unavailable: {build.stderr[:200]}")
        bin_name = "Scripts" if os.name == "nt" else "bin"
        venv_bin = venv_dir / bin_name
        pip_exe = venv_bin / ("pip.exe" if os.name == "nt" else "pip")
        install = subprocess.run(
            [str(pip_exe), "install", "--quiet", str(PKG_DIR)],
            capture_output=True, text=True, timeout=900,
        )
        self.assertEqual(install.returncode, 0,
                         f"wheel install failed:\n{install.stdout}\n{install.stderr}")
        script = venv_bin / ("taskmarket-mcp.exe" if os.name == "nt" else "taskmarket-mcp")
        self.assertTrue(script.exists(), f"console script missing in {sorted(p.name for p in venv_bin.iterdir())}")
        request = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {}}}
        proc = subprocess.run(
            [str(script)], input=json.dumps(request) + "\n",
            capture_output=True, text=True, timeout=120,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        response = json.loads(proc.stdout.strip().splitlines()[0])
        self.assertEqual(response["id"], 1)
        self.assertIn("serverInfo", response["result"])


if __name__ == "__main__":
    unittest.main()
