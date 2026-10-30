#!/usr/bin/env python3
"""Check static social cards and their local assets for every sitemap page.

This verifies crawlable metadata, not a platform's cached or rendered preview.
Use --live-image after publishing to verify the public image bytes separately.
"""

import argparse
import hashlib
import json
from html.parser import HTMLParser
from pathlib import Path
import struct
from urllib.parse import unquote, urlsplit
from urllib.request import Request, urlopen
import xml.etree.ElementTree as ET


class Head(HTMLParser):
    def __init__(self):
        super().__init__()
        self.meta = {}
        self.canonicals = []
        self.in_head = False

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "head":
            self.in_head = True
        if not self.in_head:
            return
        if tag == "meta":
            key = attrs.get("property") or attrs.get("name")
            self.meta.setdefault(key, []).append(attrs.get("content", ""))
        if tag == "link" and attrs.get("rel") == "canonical":
            self.canonicals.append(attrs.get("href", ""))

    def handle_endtag(self, tag):
        if tag == "head":
            self.in_head = False


def webp_dimensions(data):
    if data[:4] != b"RIFF" or data[8:12] != b"WEBP":
        raise ValueError("not a WebP image")
    position = 12
    while position + 8 <= len(data):
        kind = data[position:position + 4]
        length = struct.unpack_from("<I", data, position + 4)[0]
        payload = data[position + 8:position + 8 + length]
        if len(payload) != length:
            raise ValueError("truncated WebP chunk")
        if kind == b"VP8X" and length >= 10:
            return (1 + int.from_bytes(payload[4:7], "little"),
                    1 + int.from_bytes(payload[7:10], "little"))
        if kind == b"VP8 " and length >= 10 and payload[3:6] == b"\x9d\x01\x2a":
            width, height = struct.unpack_from("<HH", payload, 6)
            return width & 0x3FFF, height & 0x3FFF
        if kind == b"VP8L" and length >= 5 and payload[0] == 0x2F:
            bits = int.from_bytes(payload[1:5], "little")
            return 1 + (bits & 0x3FFF), 1 + ((bits >> 14) & 0x3FFF)
        position += 8 + length + (length % 2)
    raise ValueError("WebP dimensions unavailable")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--live-image", action="store_true")
    args = parser.parse_args()
    site = (args.root / "site").resolve()
    urls = [node.text for node in ET.parse(site / "sitemap.xml").iter()
            if node.tag.endswith("}loc")]
    required = ("og:type", "og:site_name", "og:title", "og:description", "og:url",
                "og:image", "og:image:type", "og:image:width", "og:image:height",
                "og:image:alt", "twitter:card", "twitter:title", "twitter:description",
                "twitter:image", "twitter:image:alt")
    errors, images = [], {}
    if not urls:
        errors.append("sitemap has no URLs to check")
    for url in urls:
        parts = urlsplit(url)
        relative = unquote(parts.path).lstrip("/")
        if not relative or relative.endswith("/"):
            relative += "index.html"
        elif not relative.endswith(".html"):
            relative += ".html"
        path = (site / relative).resolve()
        if not path.is_relative_to(site):
            errors.append(f"{url}: sitemap path escapes site")
            continue
        head = Head()
        try:
            head.feed(path.read_text())
        except OSError as exc:
            errors.append(f"{url}: {exc}")
            continue
        for key in required:
            values = head.meta.get(key, [])
            if len(values) != 1 or not values[0].strip():
                errors.append(f"{relative}: expected one nonempty {key}")
        if any(len(head.meta.get(key, [])) != 1 for key in required):
            continue
        meta = {key: values[0] for key, values in head.meta.items() if len(values) == 1}
        if "noindex" in meta.get("robots", "").lower():
            errors.append(f"{relative}: noindex document is listed in sitemap")
        if head.canonicals != [url] or meta["og:url"] != url:
            errors.append(f"{relative}: sitemap, canonical and og:url must agree")
        if meta["og:site_name"] != "Agentizable.ai":
            errors.append(f"{relative}: unexpected site brand")
        if meta["twitter:card"] != "summary_large_image":
            errors.append(f"{relative}: expected large image card")
        for field in ("title", "description", "image"):
            if meta[f"twitter:{field}"] != meta[f"og:{field}"]:
                errors.append(f"{relative}: inconsistent {field} across cards")
        if meta["twitter:image:alt"] != meta["og:image:alt"]:
            errors.append(f"{relative}: inconsistent image alternative text")
        image_url = meta["og:image"]
        image_parts = urlsplit(image_url)
        image_path = (site / unquote(image_parts.path).lstrip("/")).resolve()
        if (image_parts.scheme != "https" or image_parts.netloc != "agentizable.ai"
                or image_parts.query or image_parts.fragment or not image_path.is_relative_to(site)):
            errors.append(f"{relative}: image must be a clean first-party HTTPS URL")
            continue
        try:
            data = image_path.read_bytes()
            width, height = webp_dimensions(data)
            if (meta["og:image:width"], meta["og:image:height"]) != (str(width), str(height)):
                errors.append(f"{relative}: declared dimensions do not match image")
            if meta["og:image:type"] != "image/webp":
                errors.append(f"{relative}: declared MIME does not match image")
            if width < 600 or height < 315 or len(data) >= 5_000_000:
                errors.append(f"{relative}: image is too small or exceeds 5 MB")
            images[image_url] = {"width": width, "height": height, "bytes": len(data),
                                 "sha256": hashlib.sha256(data).hexdigest()}
        except (OSError, ValueError, struct.error) as exc:
            errors.append(f"{relative}: invalid image: {exc}")
    if args.live_image:
        for url, expected in images.items():
            try:
                request = Request(url, headers={"User-Agent": "Agentizable-Social-Preview-Check/1.0"})
                with urlopen(request, timeout=20) as response:
                    data = response.read(5_000_001)
                    if response.status != 200 or response.headers.get_content_type() != "image/webp":
                        errors.append(f"{url}: public image status or MIME mismatch")
                    if hashlib.sha256(data).hexdigest() != expected["sha256"]:
                        errors.append(f"{url}: public image bytes differ from local asset")
            except Exception as exc:
                errors.append(f"{url}: public fetch failed: {exc}")
    print(json.dumps({"status": "fail" if errors else "pass", "pages_checked": len(urls),
                      "images": images, "live_image_checked": args.live_image,
                      "platform_rendering_verified": False, "errors": errors}, indent=2))
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
