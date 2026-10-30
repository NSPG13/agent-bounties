from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("check_site", Path(__file__).with_name("check-site.py"))
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)


class SharedScriptVersionTests(unittest.TestCase):
    def check(self, pages, omit=()):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            versions = {"marketplace.js": 10, "wallet-link.js": 14, "marketplace-workflow.js": 16,
                        "submissions.js": 5, "review-deadline.js": 4, "analytics-config.js": 16}
            supplied = "".join(pages.values())
            defaults = "".join(f'<script src="{name}?v={version}"></script>' for name, version in versions.items()
                               if name not in supplied and name not in omit)
            (root / "defaults.html").write_text(defaults)
            (root / "analytics-config.js").write_text('new URL("webmcp.js?v=17", base);')
            for name, text in pages.items():
                (root / name).write_text(text)
            guard.check_shared_script_versions(root)

    def test_current_site_uses_one_fresh_key_per_shared_asset(self):
        guard.check_shared_script_versions(Path(__file__).resolve().parents[1] / 'site')

    def test_consistent_later_versions_are_allowed(self):
        self.check({'a.html': '<script src="marketplace.js?v=11"></script><script src="wallet-link.js?v=15"></script>',
                    'b.html': '<script src="../marketplace.js?v=11"></script>'})

    def test_pre_cutover_cache_keys_are_rejected(self):
        for asset, old in [('marketplace.js', 8), ('wallet-link.js', 12), ('marketplace-workflow.js', 12),
                           ('submissions.js', 3), ('review-deadline.js', 2), ('analytics-config.js', 13)]:
            with self.subTest(asset=asset), self.assertRaises(SystemExit):
                self.check({'a.html': f'<script src="{asset}?v={old}"></script>'})

    def test_old_dynamic_loader_key_is_rejected(self):
        with self.assertRaises(SystemExit):
            self.check({'analytics-config.js': 'new URL("webmcp.js?v=15", base);'})

    def test_missing_or_non_numeric_key_is_rejected(self):
        for suffix in ['', '?v=latest', '?v=14&v=12']:
            with self.subTest(suffix=suffix), self.assertRaises(SystemExit):
                self.check({'a.html': f'<script src="wallet-link.js{suffix}"></script>'})

    def test_two_valid_but_different_keys_are_rejected(self):
        with self.assertRaises(SystemExit):
            self.check({'a.html': '<script src="marketplace.js?v=10"></script><script src="wallet-link.js?v=14"></script>',
                        'b.html': '<script src="marketplace.js?v=11"></script>'})

    def test_higher_floor_does_not_pass_if_an_asset_disappears(self):
        with self.assertRaises(SystemExit):
            self.check({'a.html': '<script src="marketplace.js?v=10"></script>'}, omit=('wallet-link.js',))


if __name__ == '__main__':
    unittest.main()
