from __future__ import annotations

import argparse
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen


PUBLIC_ORIGIN = "https://agentbounties.app"
ADVERTISEMENT_SOURCES = (
    "crates/api/src/main.rs",
    "crates/mcp-server/src/chatgpt_app.rs",
)
HANDOFF_BOUNDARIES = {
    "authorize.html": ("Canonical evidence required", "BountySettled"),
    "cancel.html": ("not canonical funding evidence",),
    "onramp.html": ("FundingAdded", "MoonPay top-up ≠ bounty funding"),
    "post.html": ("canonical creation and funding events",),
    "success.html": ("does not prove funding", "FundingAdded"),
}
URL_PATTERN = re.compile(r"https://agentbounties\.app/(?P<path>[A-Za-z0-9_./-]+\.html)")


class AssetParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.assets: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        for attribute in ("src", "href"):
            value = values.get(attribute)
            if value:
                self.assets.append(value)


def advertised_paths(repo_root: Path) -> set[str]:
    paths: set[str] = set()
    for relative in ADVERTISEMENT_SOURCES:
        source = repo_root / relative
        if not source.is_file():
            raise AssertionError(f"missing advertisement source: {relative}")
        paths.update(match.group("path") for match in URL_PATTERN.finditer(source.read_text(encoding="utf-8")))
    return paths


def _check_local_assets(site_dir: Path, page: Path, body: str) -> list[str]:
    errors: list[str] = []
    parser = AssetParser()
    parser.feed(body)
    for reference in parser.assets:
        parsed = urlparse(reference)
        if parsed.scheme or reference.startswith("//") or reference.startswith("#"):
            continue
        relative = parsed.path
        if not relative or relative.endswith("/"):
            continue
        target = (page.parent / relative).resolve()
        try:
            target.relative_to(site_dir.resolve())
        except ValueError:
            errors.append(f"{page.name}: asset escapes site directory: {reference}")
            continue
        if not target.exists():
            errors.append(f"{page.name}: missing linked file: {reference}")
    return errors


def check_local(repo_root: Path) -> list[str]:
    site_dir = repo_root / "site"
    advertised = advertised_paths(repo_root)
    errors: list[str] = []
    missing_advertisements = set(HANDOFF_BOUNDARIES) - advertised
    if missing_advertisements:
        errors.append(f"handoff URLs are no longer advertised: {sorted(missing_advertisements)}")
    for path in sorted(advertised):
        page = site_dir / path
        if not page.is_file():
            errors.append(f"advertised public URL has no site file: {PUBLIC_ORIGIN}/{path}")
    for path, phrases in HANDOFF_BOUNDARIES.items():
        page = site_dir / path
        if not page.is_file():
            continue
        body = page.read_text(encoding="utf-8")
        for phrase in phrases:
            if phrase not in body:
                errors.append(f"{path}: missing evidence boundary: {phrase}")
        if '<meta name="robots" content="noindex, nofollow">' not in body:
            errors.append(f"{path}: transactional handoff must remain noindex, nofollow")
        errors.extend(_check_local_assets(site_dir, page, body))
    return errors


def check_remote(base_url: str) -> list[str]:
    errors: list[str] = []
    base = base_url.rstrip("/")
    for path, phrases in HANDOFF_BOUNDARIES.items():
        url = f"{base}/{path}"
        request = Request(url, headers={"User-Agent": "agent-bounties-public-handoff-check/1"})
        try:
            with urlopen(request, timeout=20) as response:
                body = response.read().decode("utf-8", errors="replace")
                if response.status != 200:
                    errors.append(f"{url}: expected 200, received {response.status}")
                    continue
        except (HTTPError, URLError, TimeoutError) as error:
            errors.append(f"{url}: unavailable: {error}")
            continue
        for phrase in phrases:
            if phrase not in body:
                errors.append(f"{url}: deployed page is missing evidence boundary: {phrase}")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Verify advertised Agent Bounties public handoffs.")
    parser.add_argument("--base-url", help="Also verify the deployed pages at this origin.")
    args = parser.parse_args(argv)
    repo_root = Path(__file__).resolve().parents[1]
    try:
        errors = check_local(repo_root)
    except AssertionError as error:
        errors = [str(error)]
    if args.base_url:
        errors.extend(check_remote(args.base_url))
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1
    scope = "local source"
    if args.base_url:
        scope += f" and {args.base_url.rstrip('/')}"
    print(f"public handoff checks passed: {scope}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
