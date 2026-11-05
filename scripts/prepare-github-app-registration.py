#!/usr/bin/env python3
"""Prepare a read-only GitHub App registration; creation requires browser review.

Uses GitHub's documented manifest flow. Credentials returned by GitHub are kept
in a new mode-0600 local file, never printed or committed. This helper does not
install the App, enable a receiver, grant repository access, or send messages.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import html
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import re
import secrets
import time
from urllib.parse import parse_qs, urlsplit
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--port", type=int, default=18703)
    args = parser.parse_args()
    directory = args.directory.resolve()
    repository = Path(__file__).resolve().parents[1]
    if directory == repository or repository in directory.parents:
        parser.error("Store registration credentials outside the repository")
    directory.mkdir(parents=True, mode=0o700, exist_ok=True)
    os.chmod(directory, 0o700)
    credentials = directory / "github-app-credentials.json"
    if credentials.exists():
        parser.error("Registration credentials already exist; inspect the existing App instead")
    state = secrets.token_urlsafe(32)
    started = time.monotonic()
    result = {"complete": False, "exchanging": False}
    origin = f"http://127.0.0.1:{args.port}"
    manifest = {
        "name": "Agentizable Staging",
        "description": "Private issue-to-draft rehearsal for Agentizable.ai. No repository writes or payment authority.",
        "url": "https://agentizable.ai",
        "hook_attributes": {
            "url": "https://agent-bounties-posting-api-canary.onrender.com/v1/github-app/webhook",
            "active": True,
        },
        "redirect_url": origin + "/callback",
        "public": False,
        "default_permissions": {"issues": "read", "metadata": "read"},
        "default_events": ["issues", "issue_comment"],
        "request_oauth_on_install": False,
    }
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    form = ("<!doctype html><meta charset=utf-8><title>Review GitHub staging App</title>"
            "<h1>Agentizable Staging</h1><p>Private App owned by NSPG13. Read-only Issues and Metadata. "
            "No repository writes, payments, or production credentials.</p>"
            "<p>Continue to GitHub to review the registration. No App is created by this button.</p>"
            f'<form action="https://github.com/settings/apps/new?state={state}" method="post">'
            f'<input type="hidden" name="manifest" value="{html.escape(json.dumps(manifest), quote=True)}">'
            '<button type="submit">Review registration on GitHub</button></form>').encode()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass  # Never log callback codes, URL parameters, or provider credentials.

        def reply(self, status, body, location=None):
            self.send_response(status)
            self.send_header("Cache-Control", "no-store")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Security-Policy", "default-src 'none'; form-action https://github.com; frame-ancestors 'none'")
            if location:
                self.send_header("Location", location)
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.headers.get("Host") != f"127.0.0.1:{args.port}":
                return self.reply(400, b"Invalid host")
            parsed = urlsplit(self.path)
            if parsed.path == "/" and not parsed.query:
                return self.reply(200, form)
            if parsed.path == "/complete" and result["complete"]:
                return self.reply(200, b"<h1>Registration captured</h1><p>Credentials are saved locally. The App has not been installed on a repository.</p>")
            if parsed.path != "/callback" or result["complete"] or result["exchanging"]:
                return self.reply(404, b"Not found")
            try:
                query = parse_qs(parsed.query, strict_parsing=True, max_num_fields=2)
            except ValueError:
                return self.reply(400, b"Malformed registration callback")
            if (set(query) != {"state", "code"} or any(len(v) != 1 for v in query.values())
                    or not secrets.compare_digest(query["state"][0], state)
                    or not re.fullmatch(r"[A-Za-z0-9_-]{20,200}", query["code"][0])
                    or time.monotonic() - started > 3600):
                return self.reply(403, b"Invalid or expired registration callback")
            result["exchanging"] = True
            try:
                request = urllib.request.Request(
                    "https://api.github.com/app-manifests/" + query["code"][0] + "/conversions",
                    data=b"", method="POST", headers={"Accept": "application/vnd.github+json",
                    "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "agent-bounties-app-setup"})
                with urllib.request.build_opener(NoRedirect()).open(request, timeout=20) as response:
                    raw = response.read(1_000_001)
                if len(raw) > 1_000_000:
                    raise ValueError("Oversized registration response")
                value = json.loads(raw)
                if (value.get("owner", {}).get("login") != "NSPG13"
                        or not isinstance(value.get("id"), int)
                        or not re.fullmatch(r"[a-z0-9-]{1,100}", value.get("slug", ""))
                        or not value.get("webhook_secret") or not value.get("pem")):
                    raise ValueError("Unexpected registration identity")
                fd = os.open(credentials, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(fd, "w") as output:
                    json.dump(value, output)
                public = {"id": value["id"], "slug": value["slug"], "owner": "NSPG13",
                          "captured_at": datetime.now(timezone.utc).isoformat(), "installed": False}
                (directory / "registration.json").write_text(json.dumps(public, indent=2) + "\n")
                result["complete"] = True
                self.reply(303, b"", "/complete")
                print(json.dumps(public), flush=True)
            except Exception:
                # Do not expose upstream bodies or callback URLs in errors.
                self.reply(502, b"Registration exchange failed. Preserve this tab and inspect the existing App before retrying.")

    server = HTTPServer(("127.0.0.1", args.port), Handler)
    print(f"Review-only registration page: {origin}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
