#!/usr/bin/env python3
"""Small follow-up edits for the PR #536 maintainer security patch."""

from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    value = file.read_text(encoding="utf-8")
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one exact match, found {count}: {old[:120]!r}")
    file.write_text(value.replace(old, new, 1), encoding="utf-8")


def main() -> None:
    replace_once(
        "scripts/standing_meta_v4_deploy.py",
        """def run(command: Sequence[str], *, cwd: Path, timeout: int = 300) -> str:
    completed = subprocess.run(
        list(command),
        cwd=cwd,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
""",
        """def run(command: Sequence[str], *, cwd: Path, timeout: int = 300) -> str:
    try:
        completed = subprocess.run(
            list(command),
            cwd=cwd,
            text=True,
            encoding="utf-8",
            errors="replace",
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        raise DeploymentError(f"command timed out: {redacted_command(command)}") from None
""",
    )
    replace_once(
        "scripts/test_standing_meta_v4_deploy.py",
        """import tempfile
import unittest
""",
        """import tempfile
import unittest
from unittest import mock
""",
    )
    replace_once(
        "scripts/test_standing_meta_v4_deploy.py",
        """    def test_networks_pin_official_vrf_configuration(self) -> None:
""",
        """    def test_timeout_errors_never_expose_signer_or_rpc_credentials(self) -> None:
        command = [
            "cast",
            "send",
            "--private-key",
            "0xsupersecret",
            "--rpc-url",
            "https://rpc.example/private-token",
        ]
        with mock.patch.object(
            MODULE.subprocess,
            "run",
            side_effect=MODULE.subprocess.TimeoutExpired(command, 1),
        ):
            with self.assertRaises(MODULE.DeploymentError) as raised:
                MODULE.run(command, cwd=Path("."), timeout=1)
        rendered = str(raised.exception)
        self.assertNotIn("supersecret", rendered)
        self.assertNotIn("private-token", rendered)
        self.assertEqual(rendered.count("[redacted]"), 2)

    def test_networks_pin_official_vrf_configuration(self) -> None:
""",
    )


if __name__ == "__main__":
    main()
