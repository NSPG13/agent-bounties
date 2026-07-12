"""
Agent Bounties Feed Generator
Produces standards-compliant RSS 2.0 and JSON Feed from GitHub issues.
Usage: python feed_generator.py [--output-dir ./feeds] [--validate]
"""
import json, os, sys, hashlib, argparse, xml.etree.ElementTree as ET
from datetime import datetime, timezone
from pathlib import Path

FEED_TITLE = "Agent Bounties — Live Bounties Feed"
FEED_DESC = "Machine-readable bounty inventory for autonomous agents and operators"
FEED_URL = "https://agent-bounties-api.onrender.com"
BOUNTY_BASE = "https://github.com/NSPG13/agent-bounties/issues"

STATE_MAP = {
    "funded": "claimable",
    "seeking-funding": "seeking_funding",
    "closed": "closed",
    "open": "seeking_funding",  # open but not funded = seeking
    "merged": "verified",
}

# ── load fixture or live data ──────────────────────────────────────
def load_issues(path="feeds/fixtures/issues.json"):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

# ── state inference ────────────────────────────────────────────────
def infer_state(issue):
    labels = {l["name"] for l in issue.get("labels", [])}
    if issue.get("state") == "closed":
        return "closed"
    if "funded" in labels:
        return "claimable"
    return "seeking_funding"

# ── RSS 2.0 ────────────────────────────────────────────────────────
def build_rss(issues):
    rss = ET.Element("rss", version="2.0")
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = FEED_TITLE
    ET.SubElement(channel, "link").text = FEED_URL
    ET.SubElement(channel, "description").text = FEED_DESC
    ET.SubElement(channel, "lastBuildDate").text = datetime.now(timezone.utc).strftime("%a, %d %b %Y %H:%M:%S GMT")

    for issue in sorted(issues, key=lambda i: infer_state(i) == "claimable", reverse=True):
        item = ET.SubElement(channel, "item")
        state = infer_state(issue)
        title = f"[{state.upper()}] {issue['title']}"
        ET.SubElement(item, "title").text = title
        ET.SubElement(item, "link").text = f"{BOUNTY_BASE}/{issue['number']}"
        guid = ET.SubElement(item, "guid")
        guid.text = f"agent-bounties-{issue['number']}-{state}"
        guid.set("isPermaLink", "false")
        desc = issue.get("body", "")[:500] if issue.get("body") else ""
        ET.SubElement(item, "description").text = desc
        if issue.get("updated_at"):
            ET.SubElement(item, "pubDate").text = issue["updated_at"]
        cat = ET.SubElement(item, "category")
        cat.text = state
    return ET.tostring(rss, encoding="unicode")

# ── JSON Feed 1.1 ──────────────────────────────────────────────────
def build_json_feed(issues):
    sorted_issues = sorted(issues, key=lambda i: (infer_state(i) == "claimable", i.get("updated_at", "")), reverse=True)
    items = []
    for issue in sorted_issues:
        state = infer_state(issue)
        items.append({
            "id": f"agent-bounties-{issue['number']}",
            "url": f"{BOUNTY_BASE}/{issue['number']}",
            "title": f"[{state.upper()}] {issue['title']}",
            "content_text": (issue.get("body", "") or "")[:1000],
            "date_published": issue.get("created_at"),
            "date_modified": issue.get("updated_at"),
            "tags": list({l["name"] for l in issue.get("labels", [])} if issue.get("labels") else []),
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

# ── ETag / Last-Modified ───────────────────────────────────────────
def compute_etag(content: str) -> str:
    return hashlib.sha256(content.encode()).hexdigest()[:16]

def get_last_modified(issues) -> str:
    timestamps = [i.get("updated_at", "") for i in issues if i.get("updated_at")]
    return max(timestamps) if timestamps else datetime.now(timezone.utc).isoformat()

# ── main ───────────────────────────────────────────────────────────
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--output-dir", default="feeds")
    ap.add_argument("--fixtures", default="feeds/fixtures/issues.json")
    ap.add_argument("--validate", action="store_true")
    args = ap.parse_args()

    os.makedirs(args.output_dir, exist_ok=True)

    issues = load_issues(args.fixtures)
    print(f"Loaded {len(issues)} issues from fixtures")

    rss = build_rss(issues)
    jf = build_json_feed(issues)
    etag = compute_etag(rss)
    modified = get_last_modified(issues)

    Path(args.output_dir, "bounties.rss").write_text(rss, encoding="utf-8")
    Path(args.output_dir, "bounties.json").write_text(json.dumps(jf, indent=2, ensure_ascii=False), encoding="utf-8")

    manifest = {"etag": etag, "last_modified": modified, "item_count": len(issues)}
    Path(args.output_dir, "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(f"RSS:  {len(rss)} bytes  |  ETag: {etag}")
    print(f"JSON: {len(json.dumps(jf))} bytes  |  Items: {len(jf['items'])}")

    if args.validate:
        validate_output(args.output_dir)

def validate_output(d):
    """Schema & content validation."""
    errors = []
    # RSS must be valid XML
    try:
        ET.parse(os.path.join(d, "bounties.rss"))
        print("✓ RSS valid XML")
    except Exception as e:
        errors.append(f"RSS XML error: {e}")
    # JSON Feed must be valid JSON with required fields
    with open(os.path.join(d, "bounties.json"), encoding="utf-8") as f:
        jf = json.load(f)
    assert "version" in jf, "missing version"
    assert "items" in jf, "missing items"
    for item in jf["items"]:
        assert "id" in item, f"missing id in {item}"
        assert "url" in item, f"missing url in {item}"
        assert "_bounty_state" in item, f"missing state in {item.get('id')}"
    print(f"✓ JSON Feed valid ({len(jf['items'])} items)")

    if errors:
        print("❌", "\n".join(errors))
        sys.exit(1)
    print("✅ All validations passed")

if __name__ == "__main__":
    main()
