from __future__ import annotations

import importlib.util
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
        return guard.check_deployed("https://agentbounties.app", SITE, "fixture", self.deployed)

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
            "missing assistant": self.home.replace('data-bounty-assistant="custom"', 'data-old-assistant="custom"'),
            "wrong canonical": self.home.replace('<link rel="canonical" href="https://agentbounties.app/">',
                                                 '<link rel="canonical" href="https://example.invalid/">'),
        }
        for name, home in mutations.items():
            with self.subTest(name=name):
                self.overrides["index.html"] = home.encode()
                with self.assertRaises(ValueError):
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
            self.assertGreater(guard.check_deployed("https://agentbounties.app", site, "fixture", self.deployed), 10)

    def test_unavailable_asset_fails(self):
        def unavailable(url):
            if urlsplit(url).path.endswith(".css"):
                raise URLError("fixture unavailable")
            return self.deployed(url)

        with self.assertRaises(URLError):
            guard.check_deployed("https://agentbounties.app", SITE, "fixture", unavailable)


if __name__ == "__main__":
    unittest.main()
