from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
SCRIPT = SCRIPT_DIR / "check-render-blueprint.py"
SPEC = importlib.util.spec_from_file_location("check_render_blueprint", SCRIPT)
assert SPEC and SPEC.loader
blueprint = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(blueprint)


class DockerCompileTimeAssetTests(unittest.TestCase):
    def create_repository(self, root: Path, *, copy_fixtures: bool) -> None:
        source = root / "crates" / "api" / "src" / "main.rs"
        source.parent.mkdir(parents=True)
        source.write_text(
            'const CARD: &str = include_str!("../../../fixtures/card.json");\n'
            '#[cfg(test)]\nmod tests {\n'
            '    const TEST_ONLY: &str = include_str!("../../../bounties/test.json");\n'
            '}\n',
            encoding="utf-8",
        )
        fixtures = root / "fixtures"
        fixtures.mkdir()
        (fixtures / "card.json").write_text("{}\n", encoding="utf-8")
        dockerfile = "COPY crates ./crates\n"
        if copy_fixtures:
            dockerfile += "COPY fixtures ./fixtures\n"
        dockerfile += "COPY --from=builder /tmp/service /usr/local/bin/service\n"
        (root / "Dockerfile").write_text(dockerfile, encoding="utf-8")
        workflow = root / ".github" / "workflows" / "containers.yml"
        workflow.parent.mkdir(parents=True)
        workflow.write_text(
            'pull_request:\n  paths:\n    - "crates/**"\n'
            + ('    - "fixtures/**"\n' if copy_fixtures else "")
            + 'push:\n  paths:\n    - "crates/**"\n'
            + ('    - "fixtures/**"\n' if copy_fixtures else ""),
            encoding="utf-8",
        )

    def test_missing_compile_time_asset_root_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, copy_fixtures=False)
            with self.assertRaisesRegex(
                SystemExit,
                r"Dockerfile must copy fixtures/.*compiles fixtures/card.json",
            ):
                blueprint.require_docker_compile_time_assets(root)

    def test_copied_asset_and_path_filters_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, copy_fixtures=True)
            blueprint.require_docker_compile_time_assets(root)


if __name__ == "__main__":
    unittest.main()
