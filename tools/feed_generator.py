"""
Agent Bounties Feed Generator
Produces standards-compliant RSS 2.0 and JSON Feed from deterministic fixtures.
Usage: python tools/feed_generator.py [--output-dir feeds] [--validate]

Future: ingest from canonical hosted funding feed (/.well-known/agent-bounties.json)
and GitHub issue inventory for live feeds.
"""
import json, os, sys, hashlib, argparse, xml.etree.ElementTree as ET
from datetime import datetime, timezone
from pathlib import Path

FEED_TITLE = "Agent Bounties -- Live Bounties Feed"
FEED_DESC = "Machine-readable bounty inventory for autonomous agents and operators"
# Placeholder URL -- replace with hosted canonical feed URL when live
FEED_URL = "https://agent-bounties.example.com"
BOUNTY_BASE = "https://github.com/NSPG13/agent-bounties/issues"


def rfc822(iso_str: str) -> str:
    """Convert ISO-8601 to RFC 822 date for RSS 2.0."""
    from email.utils import format_datetime
    dt = datetime.fromisoformat(iso_str.replace("Z", "+00:00"))
    return format_datetime(dt, usegmt=True)


def load_issues(path="feeds/fixtures/issues.json"):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def infer_state(issue):
    """State MUST be evidence-bound. Uses explicit '_bounty_state' field from
    verified canonical feed, not GitHub label heuristics. A GitHub 'funded' label
    alone does not make a bounty claimable without reconciled on-chain evidence."""
    return issue.get("_bounty_state", "seeking_funding")


def max_timestamp(issues):
    """Return max updated_at from all issues, or epoch if empty."""
    ts = [i.get("updated_at", "") for i in issues if i.get("updated_at")]
    return max(ts) if ts else "1970-01-01T00:00:00Z"


def build_rss(issues, last_modified: str):
    rss = ET.Element("rss", version="2.0")
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = FEED_TITLE
    ET.SubElement(channel, "link").text = FEED_URL
    ET.SubElement(channel, "description").text = FEED_DESC
    ET.SubElement(channel, "lastBuildDate").text = rfc822(last_modified)

    sorted_issues = sorted(
        issues,
        key=lambda i: (infer_state(i) == "claimable", i.get("updated_at", "")),
        reverse=True
    )
    for issue in sorted_issues:
        item = ET.SubElement(channel, "item")
        state = infer_state(issue)
        title = f"[{state.upper()}] {issue['title']}"
        ET.SubElement(item, "title").text = title
        ET.SubElement(item, "link").text = f"{BOUNTY_BASE}/{issue['number']}"
        guid = ET.SubElement(item, "guid")
        guid.text = f"agent-bounties-{issue['number']}"
        guid.set("isPermaLink", "false")
        desc = (issue.get("body", "") or "")[:500]
        ET.SubElement(item, "description").text = desc
        if issue.get("updated_at"):
            ET.SubElement(item, "pubDate").text = rfc822(issue["updated_at"])
        cat = ET.SubElement(item, "category")
        cat.text = state
    return ET.tostring(rss, encoding="unicode")


def build_json_feed(issues):
    sorted_issues = sorted(
        issues,
        key=lambda i: (infer_state(i) == "claimable", i.get("updated_at", "")),
        reverse=True
    )
    items = []
    for issue in sorted_issues:
        state = infer_state(issue)
        labels = sorted({l["name"] for l in issue.get("labels", [])} if issue.get("labels") else [])
        items.append({
            "id": f"agent-bounties-{issue['number']}",
            "url": f"{BOUNTY_BASE}/{issue['number']}",
            "title": f"[{state.upper()}] {issue['title']}",
            "content_text": (issue.get("body", "") or "")[:1000],
            "date_published": issue.get("created_at"),
            "date_modified": issue.get("updated_at"),
            "tags": labels,
            "_bounty_state": state,
        })
    return {
        "version": "https://jsonfeed.org/version/1.1",
        "title": FEED_TITLE,
        "home_page_url": FEED_URL,
        "feed_url": f"{FEED_URL}/feed.json",
        "description": FEED_DESC,
        "items": items,
    }


def compute_etag(content: str) -> str:
    return hashlib.sha256(content.encode()).hexdigest()[:16]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--output-dir", default="feeds")
    ap.add_argument("--fixtures", default="feeds/fixtures/issues.json")
    ap.add_argument("--validate", action="store_true")
    args = ap.parse_args()

    os.makedirs(args.output_dir, exist_ok=True)

    issues = load_issues(args.fixtures)
    last_modified = max_timestamp(issues)
    print(f"Loaded {len(issues)} issues (last modified: {last_modified})")

    rss = build_rss(issues, last_modified)
    jf = build_json_feed(issues)
    etag = compute_etag(rss)

    Path(args.output_dir, "bounties.rss").write_text(rss, encoding="utf-8")
    Path(args.output_dir, "bounties.json").write_text(
        json.dumps(jf, indent=2, ensure_ascii=False), encoding="utf-8"
    )

    manifest = {"etag": etag, "last_modified": last_modified, "item_count": len(issues)}
    Path(args.output_dir, "manifest.json").write_text(
        json.dumps(manifest, indent=2), encoding="utf-8"
    )

    print(f"RSS:  {len(rss)} bytes  |  ETag: {etag}")
    print(f"JSON: {len(json.dumps(jf))} bytes  |  Items: {len(jf['items'])}")

    if args.validate:
        validate_output(args.output_dir)
        verify_determinism(args.fixtures, args.output_dir)


def validate_output(d):
    errors = []
    try:
        ET.parse(os.path.join(d, "bounties.rss"))
        print("  RSS valid XML")
    except Exception as e:
        errors.append(f"RSS XML error: {e}")
    with open(os.path.join(d, "bounties.json"), encoding="utf-8") as f:
        jf = json.load(f)
    assert "version" in jf, "missing version"
    assert "items" in jf, "missing items"
    for item in jf["items"]:
        assert "id" in item, f"missing id in {item}"
        assert "url" in item, f"missing url in {item}"
        assert "_bounty_state" in item, f"missing state in {item.get('id')}"
        assert sorted(item["tags"]) == item["tags"], f"tags not sorted in {item.get('id')}"
    print(f"  JSON Feed valid ({len(jf['items'])} items, tags deterministic)")

    if errors:
        print("FAIL:", "\n".join(errors))
        sys.exit(1)
    print("  All validations passed")


def verify_determinism(fixture_path, output_dir):
    """Run generator twice, assert identical output."""
    import tempfile, subprocess
    tmp = tempfile.mkdtemp()
    try:
        exe = sys.executable
        subprocess.run([exe, "tools/feed_generator.py", "--fixtures", fixture_path,
                        "--output-dir", os.path.join(tmp, "run1")], check=True,
                       capture_output=True)
        subprocess.run([exe, "tools/feed_generator.py", "--fixtures", fixture_path,
                        "--output-dir", os.path.join(tmp, "run2")], check=True,
                       capture_output=True)
        r1 = Path(tmp, "run1", "bounties.rss").read_text()
        r2 = Path(tmp, "run2", "bounties.rss").read_text()
        assert r1 == r2, "RSS output is not deterministic"
        j1 = Path(tmp, "run1", "bounties.json").read_text()
        j2 = Path(tmp, "run2", "bounties.json").read_text()
        assert j1 == j2, "JSON output is not deterministic"
        print(f"  Determinism: PASS (identical RSS + JSON across runs)")
    finally:
        import shutil
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
