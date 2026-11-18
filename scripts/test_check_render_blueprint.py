from __future__ import annotations

import copy
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

import yaml


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
            'on:\n  pull_request:\n    paths:\n      - "crates/**"\n'
            + ('      - "fixtures/**"\n' if copy_fixtures else "")
            + '  push:\n    branches: [main]\n    paths:\n      - "crates/**"\n'
            + ('      - "fixtures/**"\n' if copy_fixtures else ""),
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

    def test_unfiltered_main_pushes_cover_all_runtime_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, copy_fixtures=True)
            workflow = root / ".github/workflows/containers.yml"
            original = workflow.read_text().split("  push:\n")[0]
            workflow.write_text(original + "  push:\n    branches: [main]\n")
            blueprint.require_docker_compile_time_assets(root)
            for exclusions in ('paths-ignore: ["fixtures/**"]', 'branches: [other]'):
                workflow.write_text(original + "  push:\n    " + exclusions + "\n")
                with self.subTest(exclusions=exclusions), self.assertRaises(SystemExit):
                    blueprint.require_docker_compile_time_assets(root)
            workflow.write_text(original.replace('      - "fixtures/**"\n', '') + "  push:\n    branches: [main]\n")
            with self.assertRaisesRegex(SystemExit, "fixtures/.*pull_request"):
                blueprint.require_docker_compile_time_assets(root)

    def test_negated_and_malformed_filters_cannot_disable_runtime_coverage(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, copy_fixtures=True)
            workflow = root / ".github/workflows/containers.yml"
            for event in ("push", "pull_request"):
                other = "pull_request" if event == "push" else "push"
                prefix = f"on:\n  {other}:\n    branches: [main]\n  {event}:\n"
                workflow.write_text(prefix + '    branches: [main]\n    paths: ["**"]\n')
                blueprint.require_docker_compile_time_assets(root)
                for filters in (
                    'branches: [main, "!main"]',
                    'branches: ["!other", main]',
                    'paths: ["**", "!crates/**", "!fixtures/**"]',
                    'paths: ["crates/**", "fixtures/**", "!fixtures/**"]',
                    'branches: [other]',
                    'branches: main',
                    'branches: []',
                    'paths: "**"',
                    'paths: []',
                    'paths: null',
                ):
                    with self.subTest(event=event, filters=filters):
                        workflow.write_text(prefix + "    " + filters + "\n")
                        with self.assertRaises(SystemExit):
                            blueprint.require_docker_compile_time_assets(root)
            for filters in ('tags: ["v*"]', 'tags-ignore: ["v*"]'):
                workflow.write_text("on:\n  pull_request:\n  push:\n    " + filters + "\n")
                with self.subTest(filters=filters), self.assertRaises(SystemExit):
                    blueprint.require_docker_compile_time_assets(root)


class RecoveryBlueprintTests(unittest.TestCase):
    def setUp(self) -> None:
        root = SCRIPT_DIR.parent
        self.text = (root / "render.yaml").read_text()
        self.manifest = json.loads((root / "ops/public-runtime-services.json").read_text())
        self.document = yaml.safe_load(self.text)

    def check_document(self, document: object) -> None:
        blueprint.require_recovery_blueprint(yaml.safe_dump(document), self.manifest)

    def test_current_recovery_blueprint_preserves_environment_scope(self) -> None:
        self.check_document(self.document)

    def test_old_services_and_unreviewed_resource_creation_are_rejected(self) -> None:
        for index, name in ((0, "agent-bounties-api"), (1, "agent-bounties-base-indexer")):
            with self.subTest(name=name):
                candidate = copy.deepcopy(self.document)
                candidate["services"][index]["name"] = name
                with self.assertRaises(SystemExit):
                    self.check_document(candidate)
        candidate = copy.deepcopy(self.document)
        candidate["services"].append(copy.deepcopy(candidate["services"][1]))
        with self.assertRaises(SystemExit):
            self.check_document(candidate)

    def test_sync_cannot_attach_credentials_or_mutate_domains_or_databases(self) -> None:
        for field, value in (
            ("envVars", [{"fromGroup": "agent-bounties-base"}]),
            ("domains", ["api.agentizable.ai"]),
            ("dockerCommand", "/usr/local/bin/worker"),
        ):
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.document)
                candidate["services"][0][field] = value
                with self.assertRaises(SystemExit):
                    self.check_document(candidate)
        for field in ("envVarGroups", "databases", "projects"):
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.document)
                candidate[field] = []
                with self.assertRaises(SystemExit):
                    self.check_document(candidate)

    def test_release_cannot_bypass_controller_or_reader_financial_hold(self) -> None:
        for index, key, value in (
            (0, "branch", "codex/clean-service-recovery-20261110"),
            (0, "autoDeployTrigger", "commit"),
            (0, "autoDeployTrigger", "checksPass"),
            (1, "autoDeployTrigger", "checksPass"),
            (1, "dockerfilePath", "./Dockerfile"),
            (0, "preDeployCommand", ""),
            (0, "preDeployCommand", "/usr/local/bin/check-recovery-revision b0f01679d69464d91df0170a66d81e3661a25657"),
        ):
            with self.subTest(index=index, key=key, value=value):
                candidate = copy.deepcopy(self.document)
                candidate["services"][index][key] = value
                with self.assertRaises(SystemExit):
                    self.check_document(candidate)

    def test_manifest_identity_drift_and_missing_hold_are_rejected(self) -> None:
        for mutate in (
            lambda doc: doc.pop("transition_hold"),
            lambda doc: doc.update(workspace_id="tea-other"),
            lambda doc: doc.update(branch="recovery"),
            lambda doc: doc["services"][0].update(id="srv-d982ftu7r5hc73ar4stg"),
            lambda doc: doc["services"].append(copy.deepcopy(doc["services"][0])),
        ):
            manifest = copy.deepcopy(self.manifest)
            mutate(manifest)
            with self.assertRaises(SystemExit):
                blueprint.require_recovery_blueprint(self.text, manifest)

    def test_invalid_yaml_and_nonobjects_are_rejected(self) -> None:
        for text in ("services: [", "null", "[]", "services: null"):
            with self.subTest(text=text), self.assertRaises(SystemExit):
                blueprint.require_recovery_blueprint(text, self.manifest)


if __name__ == "__main__":
    unittest.main()
