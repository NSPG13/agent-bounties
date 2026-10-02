"""Offline return-policy behavior, with no live requests or notification delivery."""
from datetime import datetime, timedelta, timezone
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("opportunity_return", ROOT / "tools/opportunity_return.py")
watch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(watch)
NOW = datetime(2026, 10, 2, 12, tzinfo=timezone.utc)


def policy():
    result = json.loads((ROOT / "tools/opportunity-return.example.json").read_text())
    result["enabled"] = True
    result["quiet_hours_utc"] = None
    return result


def state():
    return {"schema_version": "agent-bounties/return-state-v1", "seen": {}, "failures": 0, "next_check_at": None}


def amount(value):
    return {"amount": value, "currency": "USDC", "unit": "base_units", "decimals": 6}


def projection():
    return {"schema_version": "agent-bounties/opportunity-projection-v1", "network": "base-mainnet", "degraded": False,
            "generated_at": NOW.isoformat(), "source_statuses": [{"source_type": "canonical_base", "available": True}],
            "items": [{"opportunity_id": "canonical:base:example", "network": "base-mainnet", "source_type": "canonical_base", "work_state": "claimable", "payment_state": "escrowed", "payment_committed": True,
                       "title": "A useful Rust task", "public_url": "https://github.com/NSPG13/agent-bounties/issues/1", "skills": ["Rust"], "categories": ["engineering"], "reward": amount("9007199254740993"), "terms_hash": "terms1", "verification_ready": True}]}


class ReturnPolicyTests(unittest.TestCase):
    def test_inactive_quiet_and_not_due_make_no_request(self):
        def forbidden():
            self.fail("must not request")
        p = policy(); p["enabled"] = False
        self.assertEqual(watch.check_once(p, state(), forbidden, NOW)[0]["status"], "inactive")
        p["enabled"] = True; p["quiet_hours_utc"] = [22, 7]
        result, _ = watch.check_once(p, state(), forbidden, NOW.replace(hour=23))
        self.assertEqual(result["status"], "quiet")
        self.assertEqual(result["next_check_at"], "2026-10-03T07:00:00+00:00")
        s = state(); s["next_check_at"] = (NOW + timedelta(hours=2)).isoformat()
        self.assertEqual(watch.check_once(p, s, forbidden, NOW)[0]["status"], "not_due")

    def test_exact_cost_filter_changes_and_cross_session_deduplication(self):
        p = policy(); p["filters"] = {"skills": ["rust"], "categories": ["ENGINEERING"], "minimum_reward_base_units": "9007199254740993"}
        doc = projection()
        first, saved = watch.check_once(p, state(), lambda: doc, NOW)
        self.assertEqual(len(first["changes"]), 1)
        self.assertEqual(first["changes"][0]["reward"]["amount"], "9007199254740993")
        self.assertIsNone(first["changes"][0]["bond"])
        self.assertIsNone(first["changes"][0]["required_external_spend"])
        saved = json.loads(json.dumps(saved)); later = NOW + timedelta(hours=6)
        doc["generated_at"] = later.isoformat()
        second, saved = watch.check_once(p, saved, lambda: doc, later)
        self.assertEqual(second["status"], "unchanged")
        doc["items"][0]["terms_hash"] = "terms2"; later += timedelta(hours=6); doc["generated_at"] = later.isoformat()
        self.assertEqual(len(watch.check_once(p, saved, lambda: doc, later)[0]["changes"]), 1)
        p["filters"]["minimum_reward_base_units"] = "9007199254740994"
        self.assertEqual(watch.check_once(p, state(), lambda: doc, later)[0]["changes"], [])

    def test_failure_retains_seen_work_and_backs_off_with_cap(self):
        s = state(); s["seen"] = {"preserve": "a"*64}
        def failed():
            raise OSError("do not return sensitive remote errors")
        now = NOW
        for minutes in [720, 1440, 1440]:
            result, s = watch.check_once(policy(), s, failed, now)
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(s["seen"], {"preserve": "a"*64})
            self.assertEqual(watch.timestamp(result["next_check_at"])-now, timedelta(minutes=minutes))
            self.assertNotIn("sensitive", json.dumps(result))
            now = watch.timestamp(result["next_check_at"])

    def test_degraded_stale_duplicate_and_missing_sources_are_not_empty_success(self):
        fixtures = []
        for key, value in [("degraded", True), ("source_statuses", []), ("generated_at", (NOW-timedelta(hours=1)).isoformat()), ("items", None)]:
            doc = projection(); doc[key] = value; fixtures.append(doc)
        doc = projection(); doc["items"] *= 2; fixtures.append(doc)
        for doc in fixtures:
            result, saved = watch.check_once(policy(), state(), lambda: doc, NOW)
            self.assertEqual(result["status"], "unavailable")
            self.assertEqual(saved["seen"], {})

    def test_unsafe_links_and_noncommitted_rewards_do_not_route_agents(self):
        for url in ["https://outside.example", "https://agentbounties.app.evil.example/", "https://name:password@agentbounties.app/", "javascript:alert(1)"]:
            doc = projection(); doc["items"][0]["public_url"] = url
            result, _ = watch.check_once(policy(), state(), lambda: doc, NOW)
            self.assertEqual(result["changes"], []); self.assertEqual(result["invalid_items"], 1)
            self.assertEqual(result["status"], "partial")
        doc = projection(); doc["items"][0]["payment_committed"] = False
        self.assertEqual(watch.check_once(policy(), state(), lambda: doc, NOW)[0]["changes"], [])

    def test_direct_event_links_and_other_project_issues_remain_discoverable(self):
        for url in ["https://api.agentbounties.app/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id=0x123", "https://github.com/external-builder/project/issues/7"]:
            doc = projection(); doc["items"][0]["public_url"] = url
            result, _ = watch.check_once(policy(), state(), lambda: doc, NOW)
            self.assertEqual(result["status"], "changes"); self.assertEqual(result["changes"][0]["url"], url)

    def test_bounded_batches_do_not_silently_mark_unsent_changes_seen(self):
        doc = projection(); doc["items"] = [{**doc["items"][0], "opportunity_id": str(i)} for i in range(23)]
        first, saved = watch.check_once(policy(), state(), lambda: doc, NOW)
        self.assertEqual(len(first["changes"]), 20); self.assertEqual(first["pending_changes"], 3)
        later = NOW + timedelta(hours=6); doc["generated_at"] = later.isoformat()
        second, _ = watch.check_once(policy(), saved, lambda: doc, later)
        self.assertEqual(len(second["changes"]), 3)

    def test_invalid_policy_and_state_stop_before_network(self):
        for key, value in [("enabled", "yes"), ("interval_minutes", True), ("quiet_hours_utc", [7,7]), ("filters", {"minimum_reward_base_units": 1})]:
            p = policy(); p[key] = value
            with self.assertRaises(ValueError): watch.check_once(p, state(), lambda: self.fail("network"), NOW)
        with self.assertRaises(ValueError): watch.check_once(policy(), {}, lambda: self.fail("network"), NOW)

    def test_cli_stop_and_existing_lock_prevent_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            config, checkpoint = Path(tmp)/"policy.json", Path(tmp)/"state.json"
            config.write_text(json.dumps(policy()))
            command = [sys.executable, str(ROOT/"tools/opportunity_return.py"), "--config", str(config), "--state", str(checkpoint)]
            stopped = subprocess.run(command+["--stop"], check=True, capture_output=True, text=True)
            self.assertEqual(json.loads(stopped.stdout)["status"], "inactive")
            self.assertFalse(json.loads(config.read_text())["enabled"])
            self.assertFalse(checkpoint.exists())
            lock = checkpoint.with_name(checkpoint.name+".lock"); lock.write_text("owned")
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
            self.assertEqual(lock.read_text(), "owned")

    def test_cli_snapshot_persists_and_restores_the_same_change_ids(self):
        with tempfile.TemporaryDirectory() as tmp:
            config, checkpoint, snapshot = [Path(tmp)/name for name in ["policy.json", "state.json", "snapshot.json"]]
            config.write_text(json.dumps(policy()))
            doc = projection(); doc["generated_at"] = datetime.now(timezone.utc).isoformat()
            snapshot.write_text(json.dumps(doc))
            command = [sys.executable, str(ROOT/"tools/opportunity_return.py"), "--config", str(config), "--state", str(checkpoint), "--snapshot", str(snapshot)]
            first = subprocess.run(command, check=True, capture_output=True, text=True)
            self.assertEqual(len(json.loads(first.stdout)["changes"]), 1)
            self.assertEqual(len(json.loads(checkpoint.read_text())["seen"]), 1)
            self.assertEqual(json.loads(subprocess.run(command, check=True, capture_output=True, text=True).stdout)["status"], "not_due")
            self.assertFalse(checkpoint.with_name(checkpoint.name+".lock").exists())


if __name__ == "__main__":
    unittest.main()
