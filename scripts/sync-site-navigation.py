"""Render one static, no-JavaScript-required primary navigation on every site page.

Edit scripts/templates/site-navigation.html, then run this script. --check is
also called by check-site.py so a new page or hand-edited header cannot drift.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
START, END = "<!-- shared-navigation:start -->", "<!-- shared-navigation:end -->"
PATTERN = re.compile(re.escape(START) + r".*?" + re.escape(END), re.S)


def sync(check=False):
    template = (ROOT / "scripts/templates/site-navigation.html").read_text(encoding="utf-8").strip()
    changed = []
    for path in sorted((ROOT / "site").rglob("*.html")):
        relative = path.relative_to(ROOT / "site")
        prefix = "../" * (len(relative.parts) - 1)
        markup = template.replace("{{root}}", prefix or "./").replace("{{prefix}}", prefix)
        if relative.as_posix() == "index.html":
            login_href = "#login"
            login_attributes = ' data-auth-open aria-haspopup="dialog" aria-controls="auth-dialog" aria-expanded="false"'
        elif relative.as_posix() == "post.html":
            login_href = "?postReturn=1#login"
            login_attributes = " data-post-auth-start"
        else:
            login_href = "#login"
            login_attributes = ""
        markup = markup.replace("{{login_href}}", login_href).replace("{{login_attributes}}", login_attributes)
        block = START + "\n" + markup + "\n" + END
        source = path.read_text(encoding="utf-8")
        if PATTERN.search(source):
            result = PATTERN.sub(lambda _: block, source)
        else:
            # Initial migration only. A workflow toolbar is local to its page.
            header = re.search(r'<header class="(?:scene-header|about-header|topbar|site-header|market-header|install-header|legal-header)"[^>]*>.*?</header>', source, re.S)
            result = source[:header.start()] + block + source[header.end():] if header else re.sub(r'(<body\b[^>]*>)', lambda m: m[0] + "\n    " + block, source, count=1)
        css = f'<link rel="stylesheet" href="{prefix}site-navigation.css?v=1">'
        js = f'<script src="{prefix}site-navigation.js?v=1" defer></script>'
        if css not in result:
            result = result.replace("  </head>", "    " + css + "\n    " + js + "\n  </head>")
        if result != source:
            changed.append(relative.as_posix())
            if not check:
                path.write_text(result, encoding="utf-8", newline="\n")
    if check and changed:
        raise SystemExit("Navigation is out of sync: " + ", ".join(changed) + ". Run python scripts/sync-site-navigation.py")
    print(f"Shared navigation {'checked' if check else 'updated'} on {len(list((ROOT / 'site').rglob('*.html')))} pages.")


if __name__ == "__main__":
    sync("--check" in sys.argv)
