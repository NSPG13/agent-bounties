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
    footer = (ROOT / "scripts/templates/site-footer.html").read_text(encoding="utf-8").strip()
    changed = []
    for path in sorted((ROOT / "site").rglob("*.html")):
        relative = path.relative_to(ROOT / "site")
        # check-site.py separately requires this diagnostic to remain isolated
        # from account/journey handlers and to load only its canary script.
        if relative.as_posix() == "posting-draft-canary.html":
            continue
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
        # Render shared assets once, last in the head so page-local legacy
        # styles cannot override the shared shell or selected color theme.
        result = re.sub(r'\s*<(?:link\b[^>]*href|script\b[^>]*src)="' + re.escape(prefix) + r'(?:site-navigation\.css|site-navigation\.js|forest-ui\.css|forest-theme\.js|forest-hall\.css)\?v=\d+"[^>]*>(?:</script>)?', '', result)
        home_atmosphere = '    <link rel="stylesheet" href="forest-hall.css?v=8">\n' if relative.as_posix() == "index.html" else ""
        assets = f'''    <link rel="stylesheet" href="{prefix}site-navigation.css?v=4">
    <link rel="stylesheet" href="{prefix}forest-ui.css?v=2">
{home_atmosphere}    <script src="{prefix}forest-theme.js?v=1"></script>
    <script src="{prefix}site-navigation.js?v=4" defer></script>
'''
        result = re.sub(r"[ \t]*</head>", lambda _: assets + "  </head>", result)
        footer_block = "<!-- shared-footer:start -->\n" + footer.replace("{{root}}", prefix or "./").replace("{{prefix}}", prefix).replace("{{login_href}}", login_href).replace("{{login_attributes}}", login_attributes) + "\n<!-- shared-footer:end -->"
        existing_footer = r'<!-- shared-footer:start -->.*?<!-- shared-footer:end -->'
        if re.search(existing_footer, result, re.S):
            result = re.sub(existing_footer, lambda _: footer_block, result, flags=re.S)
        elif re.search(r'<footer\b[^>]*class="(?:about-footer|guild-shell-footer|market-footer|legal-footer|scene-footer|install-footer)"[^>]*>.*?</footer>', result, re.S):
            result = re.sub(r'<footer\b[^>]*class="(?:about-footer|guild-shell-footer|market-footer|legal-footer|scene-footer|install-footer)"[^>]*>.*?</footer>', lambda _: footer_block, result, count=1, flags=re.S)
        else:
            result = result.replace('</body>', footer_block + '\n  </body>')
        if result != source:
            changed.append(relative.as_posix())
            if not check:
                path.write_text(result, encoding="utf-8", newline="\n")
    if check and changed:
        raise SystemExit("Navigation is out of sync: " + ", ".join(changed) + ". Run python scripts/sync-site-navigation.py")
    print(f"Shared navigation {'checked' if check else 'updated'} on {len(list((ROOT / 'site').rglob('*.html')))} pages.")


if __name__ == "__main__":
    sync("--check" in sys.argv)
