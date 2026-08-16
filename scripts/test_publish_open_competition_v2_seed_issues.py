import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


PATH = Path(__file__).with_name("publish_open_competition_v2_seed_issues.py")
SPEC = importlib.util.spec_from_file_location("publish_open_competition_v2_seed_issues", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class SeedIssuePublisherTests(unittest.TestCase):
    def fixture(self, root: Path):
        documents = []
        for index in range(5):
            seed_id = f"seed-{index}"
            competition = "0x" + f"{index + 1:02x}" * 20
            body_path = root / f"{seed_id}.md"
            body_path.write_text(f"<!-- beta3-seed:{seed_id}:{competition} -->\n", encoding="utf-8")
            documents.append({
                "seed_id": seed_id,
                "title": f"Seed {index}",
                "body_path": str(body_path),
                "labels": ["bounty", "funded-live"],
                "competition": competition,
            })
        index_path = root / "index.json"
        index_path.write_text(json.dumps(documents), encoding="utf-8")
        return index_path

    def test_creates_each_missing_issue_once(self):
        with tempfile.TemporaryDirectory() as temporary:
            index_path = self.fixture(Path(temporary))
            created = []

            def fake_request(method, url, token, body=None):
                created.append(body)
                number = len(created)
                return {"number": number, "html_url": f"https://github.test/issues/{number}", "body": body["body"]}

            with patch.object(MODULE, "existing_issues", return_value=[]), patch.object(MODULE, "request_json", side_effect=fake_request):
                result = MODULE.publish(index_path, "owner/repo", "token")
            self.assertTrue(result["passed"])
            self.assertEqual(len(result["issues"]), 5)
            self.assertTrue(all(issue["created"] for issue in result["issues"]))

    def test_reuses_exact_open_issue(self):
        with tempfile.TemporaryDirectory() as temporary:
            index_path = self.fixture(Path(temporary))
            documents = json.loads(index_path.read_text(encoding="utf-8"))
            body = Path(documents[0]["body_path"]).read_text(encoding="utf-8")
            existing = [{"number": 9, "html_url": "https://github.test/issues/9", "body": body, "state": "open"}]
            with patch.object(MODULE, "existing_issues", return_value=existing), patch.object(MODULE, "request_json") as request:
                result = MODULE.publish(index_path, "owner/repo", "token")
            self.assertFalse(result["issues"][0]["created"])
            self.assertEqual(request.call_count, 4)


if __name__ == "__main__":
    unittest.main()
