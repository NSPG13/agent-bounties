import hashlib
import json
import re
from pathlib import Path


MIGRATION_NAME = re.compile(r"^\d{4}_[a-z0-9_]+\.sql$")


def normalized_sha256(path: Path) -> str:
    content = path.read_bytes().replace(b"\r\n", b"\n")
    return hashlib.sha256(content).hexdigest()


def main() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    migration_dir = repo_root / "migrations"
    manifest_path = migration_dir / "checksums.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("algorithm") != "sha256" or manifest.get("line_endings") != "lf":
        raise SystemExit("migration checksum manifest has unsupported settings")

    migrations = manifest.get("migrations")
    if not isinstance(migrations, dict) or not migrations:
        raise SystemExit("migration checksum manifest is empty")

    sql_files = sorted(path.name for path in migration_dir.glob("*.sql"))
    invalid_names = [name for name in sql_files if not MIGRATION_NAME.fullmatch(name)]
    if invalid_names:
        raise SystemExit(f"invalid migration filename(s): {', '.join(invalid_names)}")
    if sorted(migrations) != sql_files:
        missing = sorted(set(sql_files) - set(migrations))
        stale = sorted(set(migrations) - set(sql_files))
        raise SystemExit(
            "migration checksum manifest mismatch: "
            f"missing={missing or 'none'} stale={stale or 'none'}"
        )

    for name in sql_files:
        expected = migrations[name]
        actual = normalized_sha256(migration_dir / name)
        if actual != expected:
            raise SystemExit(
                f"immutable migration changed: {name}; add a new migration instead "
                f"(expected {expected}, got {actual})"
            )

    print(f"migration_history=ok files={len(sql_files)}")


if __name__ == "__main__":
    main()
