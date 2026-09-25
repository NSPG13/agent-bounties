#!/usr/bin/env python3
"""Deterministic smoke check for the canonical A2A 1.0 Agent Card.

Runs offline against committed artifacts: the card fixture, the binding doc,
the quickstart pointer and the two host implementations.
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
failures = []


def fail(msg):
    failures.append(msg)


card_path = ROOT / "fixtures" / "a2a-agent-card.json"
if not card_path.exists():
    fail("missing fixtures/a2a-agent-card.json")
    card = {}
else:
    card = json.loads(card_path.read_text())

for field in ("name", "description", "version", "supportedInterfaces", "skills"):
    if not card.get(field):
        fail(f"Agent Card missing required field: {field}")

ifaces = card.get("supportedInterfaces") or []
if ifaces:
    iface = ifaces[0]
    for field in ("url", "protocolVersion", "protocolBinding"):
        if not iface.get(field):
            fail(f"supported interface missing {field}")
    if iface.get("protocolVersion") != "1.0":
        fail("supported interface must declare protocolVersion 1.0")
    if "agentbounties.app/docs/a2a-direct-api-binding-v1" not in iface.get("protocolBinding", ""):
        fail("protocolBinding must point at docs/a2a-direct-api-binding-v1.md")

required_skills = {
    "discover-funded-work",
    "plan-bounty-claim",
    "submit-bounty-evidence",
    "check-bounty-settlement",
    "post-bounty",
}
skill_ids = {s.get("id") for s in card.get("skills") or []}
missing = required_skills - skill_ids
if missing:
    fail(f"Agent Card missing skills: {sorted(missing)}")

blob = json.dumps(card).lower()
if "jsonrpc" in blob or "message/send" in blob:
    fail("Agent Card must not advertise unsupported A2A HTTP+JSON transports")

doc = ROOT / "docs" / "a2a-direct-api-binding-v1.md"
if not doc.exists():
    fail("missing docs/a2a-direct-api-binding-v1.md")
else:
    text = doc.read_text().lower()
    for phrase in ("not a2a http+json", "canonical", "bountysettled"):
        if phrase not in text:
            fail(f"binding doc missing {phrase}")

for rel, needle in (
    ("crates/api/src/main.rs", "well-known/agent-card.json"),
    ("crates/web-public/src/lib.rs", "well-known/agent-card.json"),
    ("docs/agent-quickstart.md", "well-known/agent-card.json"),
):
    p = ROOT / rel
    if not p.exists():
        fail(f"missing {rel}")
    elif needle not in p.read_text():
        fail(f"{rel} does not reference the well-known Agent Card path")

for rel in ("crates/api/src/main.rs", "crates/web-public/src/lib.rs"):
    p = ROOT / rel
    if not p.exists():
        continue
    low = p.read_text().lower()
    for token in ("agent_card", "etag", "cache-control"):
        if token not in low:
            fail(f"{rel} lacks {token}")

if failures:
    for f in failures:
        print(f"FAIL: {f}")
    sys.exit(1)

print("a2a agent card smoke: OK")
