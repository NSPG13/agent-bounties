#!/usr/bin/env python3
"""Compare exact revisions using fresh OSV reports outside both untrusted trees."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

SCANNER_SHA256 = "160c6253e2922e9e0a6a9771c5c4c40a4b3c6090b34a3cdd5e80c96cc2e4218b"


class ScanError(RuntimeError):
    pass


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


def verify_scanner(binary: Path) -> None:
    if hashlib.sha256(binary.read_bytes()).hexdigest() != SCANNER_SHA256:
        raise ScanError("OSV binary does not match the reviewed v2.7.0 digest")


def findings(report: Path, tree: Path) -> tuple[set[tuple[str, ...]], set[str]]:
    if report.is_symlink() or not report.is_file():
        raise ScanError("Scanner did not produce a regular report")
    data = json.loads(report.read_text())
    if not isinstance(data, dict) or not isinstance(data.get("results"), list):
        raise ScanError("Invalid OSV report schema")
    result, sources = set(), set()
    for row in data["results"]:
        source = row["source"]
        path = Path(source["path"])
        if not path.is_absolute():
            path = tree / path
        try:
            relative = path.resolve().relative_to(tree.resolve()).as_posix()
        except ValueError as error:
            raise ScanError("Report contains a source outside its exact revision") from error
        sources.add(relative)
        if not isinstance(row["packages"], list):
            raise ScanError("Invalid package inventory")
        for entry in row["packages"]:
            package = entry["package"]
            identity = tuple(package.get(k, "") for k in ("ecosystem", "name", "version", "commit"))
            if not all(isinstance(v, str) for v in identity) or not any(identity):
                raise ScanError("Invalid package identity")
            vulns = entry.get("vulnerabilities", [])
            if not isinstance(vulns, list):
                raise ScanError("Invalid vulnerability inventory")
            for vuln in vulns:
                vid = vuln["id"]
                if not isinstance(vid, str) or not vid:
                    raise ScanError("Invalid advisory identity")
                result.add((relative, *identity, vid))
    return result, sources


def scan(binary: Path, tree: Path, report: Path, config: Path) -> set[tuple[str, ...]]:
    # Refuse stale, symlinked, or checkout-owned reports before starting the scanner.
    if report.exists() or report.is_symlink() or report.resolve().is_relative_to(tree.resolve()):
        raise ScanError("Report must be fresh and outside the checkout")
    command = [str(binary), "scan", "source", "--format=json", "--all-packages", "--all-vulns",
               "--no-ignore", "--no-call-analysis=go", "--no-call-analysis=rust", "--no-resolve",
               "--config=" + str(config), "--output-file=" + str(report), "--recursive", str(tree)]
    # No provider/wallet credentials or package-manager configuration are forwarded.
    env = {key: os.environ[key] for key in ("PATH", "HOME", "TMPDIR", "LANG") if key in os.environ}
    with report.with_suffix(".log").open("x") as log:
        code = subprocess.run(command, cwd=report.parent, env=env, stdout=log,
                              stderr=subprocess.STDOUT, timeout=900).returncode
    # OSV v2.7.0: 1 means findings; 127-130 mean incomplete/error, never clean.
    if code not in (0, 1):
        raise ScanError(f"Scanner failed with exit {code}; see {report.with_suffix('.log').name}")
    result, sources = findings(report, tree)
    if code == 1 and not result:
        raise ScanError("Scanner reported vulnerabilities without matching report evidence")
    expected = {p for p in git(tree, "ls-files").splitlines()
                if Path(p).name in ("Cargo.lock", "package-lock.json", "go.mod")
                or (Path(p).name.startswith("requirements") and p.endswith(".txt"))}
    if not expected or not expected.issubset(sources):
        raise ScanError("Missing dependency inventories: " + repr(sorted(expected - sources)))
    return result


def compare(repo: Path, base: str, head: str, binary: Path, output: Path) -> int:
    repo, binary, output = repo.resolve(), binary.resolve(), output.resolve()
    if output.is_relative_to(repo):
        raise ScanError("Evidence directory must be outside the repository")
    for revision in (base, head):
        if not re.fullmatch(r"[0-9a-f]{40}", revision) or git(repo, "rev-parse", revision + "^{commit}") != revision:
            raise ScanError("Expected an exact available commit SHA")
    verify_scanner(binary)
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    config = output / "empty-config.toml"
    config.write_text("")
    trees, sets = [], []
    try:
        for role, revision in (("base", base), ("head", head)):
            tree = output / (role + "-source")
            subprocess.run(["git", "-C", str(repo), "worktree", "add", "--detach", str(tree), revision],
                           check=True, stdout=subprocess.DEVNULL)
            trees.append(tree)
            if git(tree, "rev-parse", "HEAD") != revision:
                raise ScanError("Checkout revision mismatch")
            sets.append(scan(binary, tree, output / (role + ".json"), config))
        introduced = sorted(sets[1] - sets[0])
        summary = {"base": base, "head": head, "scanner_sha256": SCANNER_SHA256,
                   "base_findings": len(sets[0]), "head_findings": len(sets[1]),
                   "introduced": introduced, "passed": not introduced}
        (output / "comparison.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary))
        return 1 if introduced else 0
    finally:
        for tree in trees:
            subprocess.run(["git", "-C", str(repo), "worktree", "remove", "--force", str(tree)], check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--scanner", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        return compare(Path.cwd(), args.base, args.head, args.scanner, args.output)
    except (ScanError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print("Dependency comparison failed closed: " + str(error))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
