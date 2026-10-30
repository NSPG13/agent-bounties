#!/usr/bin/env python3
"""[DIRECT #684] fail-closed API test for stale domain origins (offline, exits 0)."""
import json, os, sys, unittest
from urllib.parse import urlparse

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "domain-origins.json")


def load():
    with open(FIXTURE, encoding="utf-8") as fh:
        return json.load(fh)


def origin_of(url):
    parts = urlparse(url)
    if parts.scheme != "https" or not parts.netloc:
        return ""
    return parts.netloc.lower()


def is_canonical_origin(origin, canonical):
    return origin in canonical


def render_public_link(template, bounty_id, canonical_website):
    """Generated links must never embed a retired legacy origin."""
    return template.format(origin=canonical_website, bounty_id=bounty_id)


class StaleDomainOriginTests(unittest.TestCase):
    def setUp(self):
        self.doc = load()
        self.canonical = set(self.doc["canonical_origins"])
        self.retired = set(self.doc["retired_origins"])

    def test_retired_and_canonical_sets_never_overlap(self):
        self.assertFalse(self.canonical & self.retired)

    def test_every_retired_origin_is_rejected(self):
        for origin in self.retired:
            self.assertFalse(is_canonical_origin(origin, self.canonical),
                             f"retired origin accepted: {origin}")

    def test_canonical_surfaces_are_accepted_where_required(self):
        for surface in self.doc["accepted_inputs"]:
            self.assertTrue(is_canonical_origin(origin_of(surface["url"]), self.canonical),
                            surface["url"])

    def test_non_canonical_input_is_rejected(self):
        for surface in self.doc["rejected_inputs"]:
            self.assertFalse(is_canonical_origin(origin_of(surface["url"]), self.canonical),
                             surface["url"])

    def test_generated_links_carry_only_canonical_origins(self):
        for case in self.doc["generated_outputs"]:
            link = render_public_link(case["template"], case["bounty_id"],
                                      self.doc["canonical_website"])
            self.assertEqual(link, case["expected"])
            self.assertEqual(origin_of(link), self.doc["canonical_website"])
            for dead in self.retired:
                self.assertNotIn(dead, link)

    def test_redirect_targets_are_canonical(self):
        for case in self.doc["redirects"]:
            self.assertIn(origin_of(case["target"]), self.canonical)
            for dead in self.retired:
                self.assertNotIn(dead, case["target"])

    def test_no_test_touches_dns_credentials_or_network(self):
        blob = json.dumps(self.doc).lower()
        for word in ("api_key", "secret", "wallet", "rpc", "dns_lookup"):
            self.assertNotIn(word, blob)

    def test_accepted_generated_and_redirect_urls_are_https(self):
        """Accepted / generated / redirect URLs must be HTTPS.
        Rejected inputs are allowed to be non-HTTPS -- that is one of the
        reasons they are rejected, so asserting HTTPS there would be wrong."""
        urls = ([s["url"] for s in self.doc["accepted_inputs"]]
                + [c["expected"] for c in self.doc["generated_outputs"]]
                + [c["target"] for c in self.doc["redirects"]])
        for url in urls:
            self.assertTrue(url.startswith("https://"), url)

    def test_scheme_is_part_of_the_rejection_reason(self):
        for surface in self.doc["rejected_inputs"]:
            url = surface["url"]
            if not url.startswith("https://"):
                self.assertFalse(is_canonical_origin(origin_of(url), self.canonical))


if __name__ == "__main__":
    rc = unittest.main(verbosity=2, exit=False).result
    sys.exit(0 if rc.wasSuccessful() else 1)
