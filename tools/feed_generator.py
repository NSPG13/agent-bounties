"""Snapshot and validate Agent Bounties's live opportunity feeds.

The API owns projection and feed rendering. This utility intentionally does not
rebuild bounty state from fixtures, GitHub labels, or another datastore.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path


FEEDS = {
    "rss": ("v1/opportunities/feed.rss", "bounties.rss"),
    "atom": ("v1/opportunities/feed.atom", "bounties.atom"),
    "json": ("v1/opportunities/feed.json", "bounties.json"),
}
ATOM_NS = "{http://www.w3.org/2005/Atom}"


def normalized_api_base(value: str) -> str:
    parsed = urllib.parse.urlparse(value.strip())
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise ValueError("api base URL must use http or https and include a host")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("api base URL cannot include credentials, query, or fragment")
    if parsed.scheme == "http" and parsed.hostname not in {"127.0.0.1", "localhost", "::1"}:
        raise ValueError("non-local API snapshots require https")
    return value.strip().rstrip("/")


def fetch(base_url: str, route: str) -> tuple[bytes, dict[str, str]]:
    url = f"{base_url}/{route}"
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "bountyboard-feed-validator/1.0"},
    )
    with urllib.request.urlopen(request, timeout=15) as response:
        if response.status != 200:
            raise RuntimeError(f"{url} returned HTTP {response.status}")
        return response.read(), {key.lower(): value for key, value in response.headers.items()}


def validate_rss(body: bytes) -> list[str]:
    root = ET.fromstring(body)
    if root.tag != "rss" or root.attrib.get("version") != "2.0":
        raise ValueError("RSS response is not RSS 2.0")
    channel = root.find("channel")
    if channel is None:
        raise ValueError("RSS response is missing channel")
    return [guid.text or "" for guid in channel.findall("item/guid")]


def validate_atom(body: bytes) -> list[str]:
    root = ET.fromstring(body)
    if root.tag != f"{ATOM_NS}feed":
        raise ValueError("Atom response is not an Atom 1.0 feed")
    values = []
    for entry in root.findall(f"{ATOM_NS}entry"):
        identifier = entry.findtext(f"{ATOM_NS}id", default="")
        values.append(identifier.removeprefix("urn:bountyboard:"))
    return values


def validate_json_feed(body: bytes) -> tuple[list[str], dict]:
    document = json.loads(body)
    if document.get("version") != "https://jsonfeed.org/version/1.1":
        raise ValueError("JSON response is not JSON Feed 1.1")
    items = document.get("items")
    if not isinstance(items, list):
        raise ValueError("JSON Feed is missing items")
    identifiers = []
    for item in items:
        identifiers.append(item["id"])
        extension = item.get("_bountyboard")
        if not isinstance(extension, dict):
            raise ValueError(f"{item['id']} is missing _bountyboard state")
        for key in ("source_type", "work_state", "payment_state", "payment_committed"):
            if key not in extension:
                raise ValueError(f"{item['id']} is missing _bountyboard.{key}")
        if extension["payment_state"] == "none" and extension["payment_committed"] is not False:
            raise ValueError(f"{item['id']} marks payment_state=none as committed")
    return identifiers, document


def validate_documents(documents: dict[str, bytes]) -> dict:
    rss_ids = validate_rss(documents["rss"])
    atom_ids = validate_atom(documents["atom"])
    json_ids, json_feed = validate_json_feed(documents["json"])
    if rss_ids != json_ids or atom_ids != json_ids:
        raise ValueError("RSS, Atom, and JSON Feed item order or identifiers differ")
    return json_feed


def write_snapshot(api_base_url: str, output_dir: Path) -> dict:
    output_dir.mkdir(parents=True, exist_ok=True)
    documents: dict[str, bytes] = {}
    response_headers: dict[str, dict[str, str]] = {}
    for name, (route, filename) in FEEDS.items():
        body, headers = fetch(api_base_url, route)
        documents[name] = body
        response_headers[name] = headers
        (output_dir / filename).write_bytes(body)

    json_feed = validate_documents(documents)
    manifest = {
        "source_api": api_base_url,
        "item_count": len(json_feed["items"]),
        "last_modified": response_headers["json"].get("last-modified"),
        "documents": {
            name: {
                "route": route,
                "filename": filename,
                "sha256": hashlib.sha256(documents[name]).hexdigest(),
                "etag": response_headers[name].get("etag"),
            }
            for name, (route, filename) in FEEDS.items()
        },
    }
    (output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--api-base-url", default="http://127.0.0.1:8080")
    parser.add_argument("--output-dir", default="feeds/proof")
    args = parser.parse_args()
    api_base_url = normalized_api_base(args.api_base_url)
    manifest = write_snapshot(api_base_url, Path(args.output_dir))
    print(
        f"feed_snapshot=ok items={manifest['item_count']} "
        f"source={manifest['source_api']} output={args.output_dir}"
    )


if __name__ == "__main__":
    main()
