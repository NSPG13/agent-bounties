import importlib.util
from datetime import datetime, timezone
from pathlib import Path
import unittest
import urllib.parse


PATH = Path(__file__).with_name("diagnose_open_competition_v2_beta3_render.py")
SPEC = importlib.util.spec_from_file_location(
    "diagnose_open_competition_v2_beta3_render", PATH
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class FakeClient:
    def __init__(self):
        self.paths = []

    def resolve_service(self, spec):
        return {
            "id": f"srv-{spec.name.rsplit('-', 1)[-1]}",
            "name": spec.name,
            "ownerId": "tea-workspace",
        }

    def _read_with_retry(self, path):
        self.paths.append(path)
        return {
            "logs": [
                {
                    "message": (
                        "rpc failed at https://user:secret@example.invalid/v1 "
                        "for 0x" + "ab" * 32
                    )
                }
            ]
        }


class Beta3RenderDiagnosticTests(unittest.TestCase):
    def test_collect_is_read_only_scoped_and_redacted(self):
        client = FakeClient()
        result = MODULE.collect(
            client,
            now=datetime(2026, 8, 20, 6, 0, tzinfo=timezone.utc),
        )

        self.assertTrue(result["read_only"])
        self.assertTrue(result["secrets_redacted"])
        self.assertEqual(len(result["workers"]), 4)
        self.assertEqual(len(client.paths), 4)
        for path, worker in zip(client.paths, result["workers"]):
            query = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)
            self.assertEqual(query["ownerId"], ["tea-workspace"])
            self.assertEqual(query["resource"], [worker["service_id"]])
            self.assertEqual(query["type"], ["app"])
            excerpts = "\n".join(worker["runtime_logs"]["excerpts"])
            self.assertNotIn("secret", excerpts)
            self.assertNotIn("example.invalid", excerpts)
            self.assertNotIn("ab" * 32, excerpts)


if __name__ == "__main__":
    unittest.main()
