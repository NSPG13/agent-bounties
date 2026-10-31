#!/usr/bin/env python3
"""Require the shipped runtime's complete build inputs to exist in public main.

Local comparisons aid review. Only --published fetches the fixed public repository
and is acceptable as deployment evidence. No credential or git filter is used.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
from regression_verifier_source_guard import _compile_time_inputs, _rust_tokens

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = "https://github.com/NSPG13/agent-bounties.git"
PREFIXES = ("crates/", "migrations/", "contracts/", "programs/", ".cargo/")
FILES = {"Cargo.toml", "Cargo.lock", "Dockerfile", "rust-toolchain.toml", "rust-toolchain",
         "scripts/check_public_runtime_parity.py", "scripts/regression_verifier_source_guard.py"}


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(root), *args], text=True, timeout=120).strip()


def inventory(root: Path) -> dict[str, str]:
    root = root.resolve()
    names = git(root, "ls-files", "-z").split("\0")
    paths = {root / name for name in names if name in FILES or name.startswith(PREFIXES)}
    if root / "Cargo.toml" not in paths or root / "crates/api/src/main.rs" not in paths:
        raise ValueError("complete tracked runtime source is required")
    # Only workspace crates are compiled into the hosted binaries. Standalone
    # SP1 programs embed locally generated ELFs and are source-compared above.
    paths.update(_compile_time_inputs(root, {p for p in paths if p.is_relative_to(root / "crates")}))
    # include! can import Rust source outside the workspace crate directories.
    # Resolve literal imports transitively and reject environment-generated paths.
    pending = [p for p in paths if p.suffix == ".rs" and p.is_relative_to(root / "crates")]
    scanned = set()
    while pending:
        source = pending.pop()
        if source in scanned:
            continue
        scanned.add(source)
        tokens = _rust_tokens(source.read_text(), source.relative_to(root).as_posix())
        for index, token in enumerate(tokens):
            if token != ("ident", "include") or tokens[index + 1:index + 3] != [("bang", "!"), ("lparen", "(")]:
                continue
            args = tokens[index + 3:index + 6]
            if len(args) < 2 or args[0][0] != "string" or args[1][0] not in ("rparen", "comma") or "\\" in args[0][1]:
                raise ValueError("nonliteral include! cannot be certified")
            included = source.parent / args[0][1]
            resolved = included.resolve(strict=True)
            if root not in resolved.parents or included.is_symlink():
                raise ValueError("escaping or linked include! input")
            paths.add(resolved)
            paths.update(_compile_time_inputs(root, {resolved}))
            pending.append(resolved)
    result = {}
    for path in sorted(paths):
        if any(parent.is_symlink() for parent in (path, *path.parents) if parent != root) or not path.is_file() or root not in path.resolve().parents:
            raise ValueError("nonregular or escaping public build input")
        result[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def compare(runtime: Path, public: Path) -> dict:
    local, remote = inventory(runtime), inventory(public)
    missing = sorted(local.keys() - remote.keys())
    extra = sorted(remote.keys() - local.keys())
    changed = sorted(p for p in local.keys() & remote.keys() if local[p] != remote[p])
    if missing or extra or changed:
        raise ValueError(json.dumps({"missing_public": missing, "public_only": extra, "different": changed}))
    return {"schema": "agentizable/public-runtime-provenance-v1", "file_count": len(local),
            "source_sha256": hashlib.sha256(json.dumps(local, sort_keys=True, separators=(",", ":")).encode()).hexdigest()}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument("--public-root", type=Path)
    selection.add_argument("--published", action="store_true")
    args = parser.parse_args()
    if args.public_root:
        print(json.dumps({**compare(args.root, args.public_root), "published": False}))
        return
    if git(args.root, "status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("published provenance requires a clean committed runtime")
    with tempfile.TemporaryDirectory(prefix="agentizable-public-source-") as temp:
        public = Path(temp)
        subprocess.run(["git", "init", "--quiet", str(public)], check=True, timeout=30)
        subprocess.run(["git", "-C", str(public), "-c", "credential.helper=", "fetch", "--quiet", "--depth=1", PUBLIC, "refs/heads/main"], check=True, timeout=120)
        commit = git(public, "rev-parse", "FETCH_HEAD")
        if not re.fullmatch(r"[0-9a-f]{40}", commit):
            raise ValueError("invalid public revision")
        subprocess.run(["git", "-C", str(public), "-c", "core.hooksPath=/dev/null", "checkout", "--quiet", "--detach", commit], check=True, timeout=30)
        print(json.dumps({**compare(args.root, public), "published": True, "public_repository": PUBLIC,
                          "public_commit": commit, "runtime_commit": git(args.root, "rev-parse", "HEAD")}))


if __name__ == "__main__":
    main()
