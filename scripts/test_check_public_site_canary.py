from __future__ import annotations

import importlib.util
import json
import re
import shutil
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import URLError
from urllib.parse import parse_qs, urlsplit


SCRIPT = Path(__file__).with_name("check-public-site-canary.py")
SPEC = importlib.util.spec_from_file_location("public_site_canary", SCRIPT)
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)
SITE = SCRIPT.parents[1] / "site"


class PublicSiteCanaryTests(unittest.TestCase):
    def setUp(self):
        self.home = (SITE / "index.html").read_text(encoding="utf-8")
        self.overrides = {}
        self.requests = []

    def deployed(self, url):
        self.requests.append(url)
        path = urlsplit(url).path.lstrip("/") or "index.html"
        return self.overrides.get(path, (SITE / path).read_bytes())

    def run_canary(self):
        return guard.check_deployed("https://agentizable.ai", SITE, "fixture", self.deployed)

    def test_current_homepage_and_every_referenced_asset_pass(self):
        self.assertGreater(self.run_canary(), 10)
        self.assertTrue(all(parse_qs(urlsplit(url).query)["verify"] == ["fixture"]
                            for url in self.requests))

    def test_current_homepage_replayed_over_local_http(self):
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                body = fixture.deployed(self.path)
                self.send_response(200)
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            self.assertGreater(guard.check_deployed(
                f"http://127.0.0.1:{server.server_port}", SITE, "http-fixture"), 10)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

    def test_semantic_regressions_fail_even_with_all_current_assets(self):
        mutations = {
            "old header": self.home.replace("data-site-header", "data-old-header"),
            "missing payout": self.home.replace("data-market-volume", "data-old-volume"),
            "broken dialog": self.home.replace('aria-controls="bounty-launcher"', 'aria-controls="missing-dialog"'),
            "wrong posting fallback": self.home.replace('action="post.html"', 'action="missing.html"'),
            "broken task handoff": self.home.replace('id="home-task"', 'id="missing-task"'),
            "disabled task handoff": self.home.replace('name="task"', 'name="task" disabled'),
            "visible task input": self.home.replace('name="task" type="hidden"', 'name="task" type="text"'),
            "task outside posting form": self.home.replace('<input id="home-task"', '</form><input id="home-task"'),
            "missing assistant": self.home.replace('data-bounty-assistant="custom"', 'data-old-assistant="custom"'),
            "wrong canonical": self.home.replace('<link rel="canonical" href="https://agentizable.ai/">',
                                                 '<link rel="canonical" href="https://example.invalid/">'),
        }
        for name, home in mutations.items():
            with self.subTest(name=name):
                self.overrides["index.html"] = home.encode()
                with self.assertRaises(ValueError):
                    self.run_canary()

    def test_header_keeps_workspace_and_leaves_browse_work_to_the_page(self):
        cases = {
            "header has no Workspace link": self.home.replace('class="ab-site-workspace" href="./workspace"', 'class="ab-site-workspace" href="./old-workspace"'),
            "Browse work must not return": self.home.replace("</nav>", '<a class="ab-site-board" href="earn.html">Browse work</a></nav>', 1),
            "no longer links the work board": self.home.replace('href="earn.html"', 'href="old-board.html"'),
        }
        for message, home in cases.items():
            with self.subTest(message=message):
                self.assertNotEqual(home, self.home)
                self.overrides["index.html"] = home.encode()
                with self.assertRaisesRegex(ValueError, message):
                    self.run_canary()

    def test_payout_metric_in_header_fails(self):
        home = self.home.replace("data-market-volume", "data-old-volume")
        home = home.replace("</header>", '<output data-market-volume>0</output></header>', 1)
        self.overrides["index.html"] = home.encode()
        with self.assertRaisesRegex(ValueError, "outside the site header"):
            self.run_canary()

    def test_old_stylesheet_or_missing_script_reference_fails(self):
        assets = guard.Page(self.home).assets()
        for kind in ("style", "script"):
            reference = next(ref for found, ref in assets if found == kind)
            with self.subTest(kind=kind):
                self.overrides["index.html"] = self.home.replace(reference, "missing." + kind).encode()
                with self.assertRaisesRegex(ValueError, "scripts/styles differ"):
                    self.run_canary()

    def test_stale_css_and_html_fallback_for_javascript_fail(self):
        assets = guard.Page(self.home).assets()
        for kind, body in (("style", b"/* stale build */"), ("script", b"<html>fallback</html>")):
            reference = next(ref for found, ref in assets if found == kind)
            with self.subTest(kind=kind):
                self.overrides = {urlsplit(reference).path: body}
                with self.assertRaisesRegex(ValueError, "stale or incorrect deployed asset"):
                    self.run_canary()

    def test_pages_configuration_substitution_is_allowed_but_missing_configs_fail(self):
        self.overrides = {
            "phone-wallet-config.js": (SITE / "phone-wallet-config.js").read_bytes().replace(
                b'projectId: ""', b'projectId: "11111111111111111111111111111111"'),
            "wallet-config.js": (SITE / "wallet-config.js").read_bytes().replace(
                b"__COINBASE_CDP_PROJECT_ID__", b"canary-fixture-project"),
            "analytics-config.js": (SITE / "analytics-config.js").read_bytes().replace(
                b'googleMeasurementId: ""', b'googleMeasurementId: "G-FIXTURE"'),
        }
        self.assertGreater(self.run_canary(), 10)
        for name in guard.DEPLOYED_CONFIGS:
            with self.subTest(name=name):
                valid = self.overrides[name]
                self.overrides[name] = b"<html>not the config</html>"
                with self.assertRaisesRegex(ValueError, "invalid deployed configuration"):
                    self.run_canary()
                self.overrides[name] = valid

    def test_deploy_substitution_cannot_hide_stale_configuration_logic(self):
        self.overrides["analytics-config.js"] = (SITE / "analytics-config.js").read_bytes().replace(
            b"webMcpEnabled: true", b"webMcpEnabled: false")
        with self.assertRaisesRegex(ValueError, "stale or incorrect deployed asset"):
            self.run_canary()

    def test_next_asset_version_comes_from_the_checked_out_page(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            for _, reference in guard.Page(self.home).assets():
                path = urlsplit(reference).path
                shutil.copyfile(SITE / path, site / path)
            next_home = self.home.replace("solarpunk.css?v=", "solarpunk.css?v=next-")
            (site / "index.html").write_text(next_home, encoding="utf-8")
            self.overrides["index.html"] = next_home.encode()
            self.assertGreater(guard.check_deployed("https://agentizable.ai", site, "fixture", self.deployed), 10)

    def beacon(self):
        return json.loads(guard.EDGE_PROFILE.read_text(encoding="utf-8"))["script"]

    def with_beacon(self, beacon=None):
        return self.home.replace("</body>", (self.beacon() if beacon is None else beacon) + "</body>")

    def test_reviewed_edge_beacon_or_eu_absence_preserves_all_23_assets(self):
        for home in (self.home, self.with_beacon()):
            with self.subTest(beacon_present=home != self.home):
                self.overrides["index.html"] = home.encode()
                self.assertEqual(self.run_canary(), 23)
        self.assertTrue(all(urlsplit(url).netloc == "agentizable.ai" for url in self.requests))

    def test_changed_edge_script_attributes_or_config_fail(self):
        beacon = self.beacon()
        changes = [
            ("https://static.", "http://static."),
            ("static.cloudflareinsights.com", "static.cloudflareinsights.com.evil.invalid"),
            ("beacon.min.js/v", "beacon.min.js/changed-v"),
            ('type="module"', 'type="text/javascript"'),
            ('crossorigin="anonymous"', 'crossorigin="use-credentials"'),
            ("sha512-UKNk", "sha512-XKNk"),
            ("8afca1c874594e8f95646fdabc16e928", "00000000000000000000000000000000"),
            ('"version":"2024.11.0"', '"version":"2025.11.0"'),
            ('"r":1', '"r":0'), ('"spa":2', '"spa":1'),
            ('"r":1', '"r":1,"r":1'),
            ("<script ", '<script async '),
            ("<script ", '<script onload="bad()" '),
            ('type="module"', 'type="module" type="module"'),
            ("</script>", "bad()</script>"),
            ("></script>", "/>"),
        ]
        for old, new in changes:
            with self.subTest(old=old, new=new):
                self.assertIn(old, beacon)
                self.overrides["index.html"] = self.with_beacon(beacon.replace(old, new, 1)).encode()
                with self.assertRaises(ValueError):
                    self.run_canary()

    def test_duplicate_edge_beacon_fails(self):
        self.overrides["index.html"] = self.with_beacon(self.beacon() * 2).encode()
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.run_canary()

    def test_edge_beacon_must_be_last_direct_body_child(self):
        cases = [
            self.home.replace("</head>", self.beacon() + "</head>"),
            self.home.replace("<footer", self.beacon() + "<footer", 1),
            self.with_beacon("<div>" + self.beacon()),
            self.with_beacon("<body>" + self.beacon()),
            self.home.replace("</body>", "</body>" + self.beacon()),
            self.with_beacon(self.beacon() + "<p>after</p>"),
            self.with_beacon(self.beacon() + "<!-- after -->"),
        ]
        for index, home in enumerate(cases):
            with self.subTest(index=index):
                self.overrides["index.html"] = home.encode()
                with self.assertRaises(ValueError):
                    self.run_canary()

    def test_valid_beacon_does_not_allow_extra_external_or_inline_script(self):
        for extra in ('<script src="https://evil.invalid/extra.js"></script>',
                      '<script>bad()</script>'):
            with self.subTest(extra=extra):
                self.overrides["index.html"] = self.with_beacon(extra + self.beacon()).encode()
                with self.assertRaises(ValueError):
                    self.run_canary()

    def test_valid_beacon_preserves_original_asset_order_and_bytes(self):
        home = self.with_beacon()
        theme = re.search(r"forest-theme\.js\?v=\d+", home).group(0)
        navigation = re.search(r"site-navigation\.js\?v=\d+", home).group(0)
        reordered = home.replace(theme, "__swap__").replace(navigation, theme).replace("__swap__", navigation)
        self.assertNotEqual(reordered, home)
        self.overrides["index.html"] = reordered.encode()
        with self.assertRaisesRegex(ValueError, "scripts/styles differ"):
            self.run_canary()
        self.overrides = {"index.html": home.encode(), "forest-theme.js": b"/* wrong version */"}
        with self.assertRaisesRegex(ValueError, "stale or incorrect deployed asset"):
            self.run_canary()

    def test_valid_beacon_preserves_original_script_attributes_and_inline_content(self):
        home = self.with_beacon()
        theme = re.search(r'src="forest-theme\.js\?v=\d+"', home).group(0)
        mutations = [home.replace(theme, theme + ' onload="bad()"'),
                     home.replace('"@context": "https://schema.org"', '"@context": "https://evil.invalid"')]
        for changed in mutations:
            self.assertNotEqual(changed, home)
            self.overrides["index.html"] = changed.encode()
            with self.assertRaisesRegex(ValueError, "script attributes/content differ"):
                self.run_canary()

    def test_beacon_profile_does_not_apply_to_other_origins(self):
        self.overrides["index.html"] = self.with_beacon().encode()
        with self.assertRaisesRegex(ValueError, "scripts/styles differ"):
            guard.check_deployed("https://d4191097.agent-bounties.pages.dev", SITE, "fixture", self.deployed)

    def test_unavailable_asset_fails(self):
        def unavailable(url):
            if urlsplit(url).path.endswith(".css"):
                raise URLError("fixture unavailable")
            return self.deployed(url)

        with self.assertRaises(URLError):
            guard.check_deployed("https://agentizable.ai", SITE, "fixture", unavailable)


if __name__ == "__main__":
    unittest.main()
