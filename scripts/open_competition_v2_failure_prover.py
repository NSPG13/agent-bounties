#!/usr/bin/env python3
"""Loopback-only prover rejection endpoint for the canonical refund canary."""

from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hmac
import json
import os


class Handler(BaseHTTPRequestHandler):
    server_version = "AgentBountiesFailureCanary/1"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def response(self, status: int, value: dict[str, object]) -> None:
        body = json.dumps(value, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.send_header("cache-control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        if self.path == "/health":
            self.response(200, {"status": "failure_canary_ready"})
        else:
            self.response(404, {"error": "not_found"})

    def do_POST(self) -> None:
        expected = f"Bearer {self.server.api_key}"
        if self.path != "/v1/prove" or not hmac.compare_digest(self.headers.get("authorization", ""), expected):
            self.response(404, {"error": "not_found"})
            return
        try:
            length = int(self.headers.get("content-length", "0"))
            payload = json.loads(self.rfile.read(length))
        except (ValueError, json.JSONDecodeError):
            self.response(400, {"error": "invalid_request"})
            return
        if payload.get("schema_version") != "agent-bounties/open-competition-v2-prover-request-v1":
            self.response(400, {"error": "schema_mismatch"})
            return
        self.response(422, {"error": "forced_release_canary_rejection"})


def main() -> int:
    api_key = os.environ.get("OPEN_COMPETITION_V2_PROVER_API_KEY", "")
    if len(api_key) < 32:
        raise SystemExit("OPEN_COMPETITION_V2_PROVER_API_KEY must contain at least 32 characters")
    server = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("PORT", "9071"))), Handler)
    server.api_key = api_key
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
