"""Check deployed homepage semantics and the assets from the Pages revision."""
from __future__ import annotations

import argparse
import hashlib
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlencode, urljoin, urlsplit
from urllib.request import Request, urlopen


PUBLIC_ORIGIN = "https://agentbounties.app"
VOID_TAGS = set("area base br col embed hr img input link meta param source track wbr".split())
# Pages intentionally substitutes these public configuration files at deployment.
DEPLOYED_CONFIGS = {
    "phone-wallet-config.js": rb'(projectId: )"(?:|[a-fA-F0-9]{32})"',
    "wallet-config.js": rb'(const projectId = )"(?:__COINBASE_CDP_PROJECT_ID__|[A-Za-z0-9-]{8,128})"',
    "analytics-config.js": rb'(googleMeasurementId: )"(?:|G-[A-Z0-9]+)"',
}


class Page(HTMLParser):
    def __init__(self, body: str) -> None:
        super().__init__()
        self.nodes: list[tuple[str, dict[str, str | None], tuple[int, ...]]] = []
        self.stack: list[int] = []
        self.feed(body)

    def handle_starttag(self, tag, attrs):
        self.nodes.append((tag, dict(attrs), tuple(self.stack)))
        if tag not in VOID_TAGS:
            self.stack.append(len(self.nodes) - 1)

    def handle_endtag(self, tag):
        for offset in range(len(self.stack) - 1, -1, -1):
            if self.nodes[self.stack[offset]][0] == tag:
                del self.stack[offset:]
                return

    def matching(self, attribute: str, value: str | None = None) -> list[int]:
        return [i for i, (_, attrs, _) in enumerate(self.nodes)
                if attribute in attrs and (value is None or attrs[attribute] == value)]

    def one(self, attribute: str, value: str | None = None) -> int:
        matches = self.matching(attribute, value)
        require(len(matches) == 1, f"expected one {attribute}={value}, found {len(matches)}")
        return matches[0]

    def assets(self) -> list[tuple[str, str]]:
        assets = []
        for tag, attrs, _ in self.nodes:
            if tag == "script" and attrs.get("src"):
                assets.append(("script", attrs["src"]))
            if tag == "link" and "stylesheet" in (attrs.get("rel") or "").split():
                require(bool(attrs.get("href")), "stylesheet has no href")
                assets.append(("style", attrs["href"]))
        return assets


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def check_home(body: str) -> Page:
    page = Page(body)
    canonical = page.nodes[page.one("rel", "canonical")][1]
    require(canonical.get("href") == PUBLIC_ORIGIN + "/", "wrong canonical homepage")
    header = page.one("data-site-header")
    require(page.nodes[header][0] == "header", "site header is not a header element")
    require(any(tag == "a" and attrs.get("href") == "earn.html" and header in parents
                for tag, attrs, parents in page.nodes), "header has no browse-work link")
    form = page.one("data-home-task")
    tag, attrs, _ = page.nodes[form]
    require(tag == "form" and attrs.get("action") == "post.html"
            and attrs.get("method", "get").lower() == "get", "task form lost the posting fallback")
    task = page.nodes[page.one("name", "task")]
    require(task[0] == "textarea" and form in task[2], "task input is outside the posting form")
    button = page.nodes[page.one("id", "post-a-bounty")]
    require(button[0] == "button" and form in button[2] and "data-bounty-open" in button[1]
            and button[1].get("type") == "button", "primary posting control is not wired")
    dialog = page.one("id", button[1].get("aria-controls") or "")
    require(page.nodes[dialog][0] == "dialog" and "data-bounty-launcher" in page.nodes[dialog][1],
            "posting control does not target the assistant chooser")
    assistants = {attrs.get("data-bounty-assistant") for tag, attrs, parents in page.nodes
                  if tag == "button" and dialog in parents}
    require({"gpt", "claude", "cursor", "custom"} <= assistants, "assistant choices are missing")
    metric = page.nodes[page.one("data-market-volume")]
    require(header not in metric[2], "payout metric must stay outside the site header")
    sections = [i for i in metric[2] if page.nodes[i][0] == "section"
                and page.nodes[i][1].get("aria-label")]
    require(bool(sections), "payout metric is outside the labelled metrics section")
    section = sections[-1]
    require(any(page.nodes[i][0] == "main" for i in page.nodes[section][2]),
            "metrics section is outside the main content")
    for attribute in ("data-completed-bounties", "data-live-bounties"):
        require(section in page.nodes[page.one(attribute)][2], f"{attribute} is outside the metrics section")
    return page


def fetch(url: str) -> bytes:
    request = Request(url, headers={"Cache-Control": "no-cache", "User-Agent": "agent-bounties-site-canary/1"})
    with urlopen(request, timeout=20) as response:
        require(response.status == 200, f"{url}: expected HTTP 200")
        body = response.read(2 * 1024 * 1024 + 1)
        require(len(body) <= 2 * 1024 * 1024, f"{url}: response exceeds 2 MiB")
        return body


def asset_digest(path: str, body: bytes) -> bytes:
    if path in DEPLOYED_CONFIGS:
        body, count = re.subn(DEPLOYED_CONFIGS[path], rb'\1"<deployment-value>"', body)
        require(count == 1, f"invalid deployed configuration: {path}")
    return hashlib.sha256(body).digest()


def check_deployed(base_url: str, site: Path, nonce: str, get=fetch) -> int:
    def read(reference: str) -> bytes:
        url = urljoin(base_url.rstrip("/") + "/", reference)
        url += ("&" if "?" in url else "?") + urlencode({"verify": nonce})
        return get(url)

    expected = check_home((site / "index.html").read_text(encoding="utf-8"))
    actual = check_home(read("").decode("utf-8"))
    require(actual.assets() == expected.assets(), "homepage scripts/styles differ from the Pages revision")
    require({kind for kind, _ in expected.assets()} == {"script", "style"}, "homepage needs scripts and styles")
    for kind, reference in expected.assets():
        parsed = urlsplit(reference)
        require(not parsed.scheme and not parsed.netloc and not parsed.fragment,
                f"unexpected non-local {kind}: {reference}")
        target = (site / parsed.path.lstrip("/")).resolve()
        require(target.is_relative_to(site.resolve()) and target.is_file(), f"missing source asset: {reference}")
        deployed = read(reference)
        require(asset_digest(parsed.path, deployed) == asset_digest(parsed.path, target.read_bytes()),
                f"stale or incorrect deployed asset: {reference}")
    return len(expected.assets())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default=PUBLIC_ORIGIN)
    parser.add_argument("--nonce", default="local-canary")
    args = parser.parse_args()
    try:
        count = check_deployed(args.base_url, Path(__file__).resolve().parents[1] / "site", args.nonce)
    except (OSError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(f"public homepage canary passed: posting controls, metrics, {count} current assets")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
