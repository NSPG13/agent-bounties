"""One explicit, read-only discovery check; never starts a scheduler or agent."""
from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import urllib.parse
import urllib.request

SOURCE = "https://api.agentbounties.app/v1/opportunities?network=base-mainnet&source_type=canonical_base&limit=300"
BOUNDARY = "Discovery only. No claim, payment, private account read or notification was sent. Later participation can require a bond, gas and other spending. Absence from this bounded view does not establish completion or cancellation."


def timestamp(value):
    result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if result.tzinfo is None:
        raise ValueError("timestamp needs a timezone")
    return result.astimezone(timezone.utc)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def validate_policy(policy):
    if not isinstance(policy, dict) or policy.get("schema_version") != "agent-bounties/return-policy-v1" or type(policy.get("enabled")) is not bool:
        raise ValueError("Use the versioned example policy with explicit enabled true/false.")
    allowed = {"schema_version", "enabled", "interval_minutes", "max_backoff_minutes", "quiet_hours_utc", "filters"}
    if set(policy) - allowed:
        raise ValueError("Unknown policy field; no setting was applied.")
    for key in ["interval_minutes", "max_backoff_minutes"]:
        if type(policy.get(key)) is not int or not 60 <= policy[key] <= 10080:
            raise ValueError(f"{key} must be 60..10080.")
    if policy["max_backoff_minutes"] < policy["interval_minutes"]:
        raise ValueError("Maximum backoff cannot be shorter than the interval.")
    quiet = policy.get("quiet_hours_utc")
    if quiet is not None and (not isinstance(quiet, list) or len(quiet) != 2 or any(type(h) is not int or not 0 <= h <= 23 for h in quiet) or quiet[0] == quiet[1]):
        raise ValueError("Quiet hours must be null or two different UTC hours.")
    filters = policy.get("filters")
    if not isinstance(filters, dict) or set(filters) - {"skills", "categories", "minimum_reward_base_units"}:
        raise ValueError("Use skills, categories and minimum_reward_base_units filters.")
    for key in ["skills", "categories"]:
        values = filters.get(key, [])
        if not isinstance(values, list) or len(values) > 20 or any(not isinstance(v, str) or not v.strip() or len(v) > 100 for v in values):
            raise ValueError(f"{key} must contain at most 20 nonempty strings.")
    minimum = filters.get("minimum_reward_base_units")
    if minimum is not None and (not isinstance(minimum, str) or not minimum.isascii() or not minimum.isdigit() or len(minimum) > 39):
        raise ValueError("Minimum reward must be null or an unsigned USDC base-unit string.")


def quiet_until(now, hours):
    if hours is None:
        return now
    start, end = hours
    quiet = start <= now.hour < end if start < end else now.hour >= start or now.hour < end
    if not quiet:
        return now
    end_time = now.replace(hour=end, minute=0, second=0, microsecond=0)
    return end_time if end_time > now else end_time + timedelta(days=1)


def valid_state(state):
    if not isinstance(state, dict) or state.get("schema_version") != "agent-bounties/return-state-v1":
        raise ValueError("Saved return state is invalid; keep it for recovery instead of resetting.")
    seen = state.get("seen")
    if not isinstance(seen, dict) or len(seen) > 1000 or any(not isinstance(k, str) or len(k) > 256 or not isinstance(v, str) or len(v) != 64 for k, v in seen.items()):
        raise ValueError("Saved change IDs are invalid; no request was made.")
    if type(state.get("failures")) is not int or not 0 <= state["failures"] <= 20:
        raise ValueError("Saved retry state is invalid.")
    if state.get("next_check_at") is not None:
        timestamp(state["next_check_at"])


def money(value):
    if not isinstance(value, dict):
        return None
    amount = value.get("amount")
    if isinstance(amount, str) and amount.isascii() and amount.isdigit() and len(amount) <= 39 and value.get("currency") == "USDC" and value.get("unit") == "base_units" and type(value.get("decimals")) is int and value["decimals"] == 6:
        return {"amount": amount, "currency": "USDC", "unit": "base_units", "decimals": 6}
    return None


def summary(item):
    identifier = item.get("opportunity_id")
    if not isinstance(identifier, str) or not identifier or len(identifier) > 256:
        raise ValueError("invalid opportunity identity")
    url = urllib.parse.urlsplit(item.get("public_url", ""))
    known_link = (url.hostname == "agentbounties.app"
                  or url.hostname == "github.com" and re.fullmatch(r"/[^/]+/[^/]+/issues/[1-9][0-9]*", url.path)
                  or url.hostname == "api.agentbounties.app" and url.path in {"/v1/base/autonomous-bounties/events", "/v1/base/open-competition-v1/events", "/v1/base/open-competition-v2-beta3/events"})
    if url.scheme != "https" or url.username or url.password or url.port not in (None, 443) or not known_link:
        raise ValueError("untrusted opportunity link")
    title = item.get("title")
    if not isinstance(title, str) or not title.strip():
        raise ValueError("missing opportunity title")
    costs = item.get("cash_economics") or {}
    return {"opportunity_id": identifier, "title": title[:200], "url": url.geturl(),
            "work_state": "claimable", "payment_state": "escrowed", "verification_ready": item.get("verification_ready") if type(item.get("verification_ready")) is bool else None,
            "terms_hash": item["terms_hash"][:128] if isinstance(item.get("terms_hash"), str) else None,
            "reward": money(item.get("reward")), "bond": money(item.get("bond")),
            "required_external_spend": money(costs.get("required_external_spend")) if isinstance(costs, dict) else None,
            "next_action": "Read the current terms, readiness and all costs before deciding whether to participate."}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise ValueError("discovery redirect refused")


def fetch_projection():
    request = urllib.request.Request(SOURCE, headers={"Accept": "application/json", "User-Agent": "agent-bounties-opt-in-return/1.0"})
    with urllib.request.build_opener(NoRedirect).open(request, timeout=15) as response:
        body = response.read(2 * 1024 * 1024 + 1)
        if response.status != 200 or len(body) > 2 * 1024 * 1024:
            raise ValueError("discovery response unavailable or too large")
        return json.loads(body)


def check_once(policy, state, fetch=fetch_projection, now=None):
    validate_policy(policy)
    valid_state(state)
    now = now or datetime.now(timezone.utc)
    result = {"status": "inactive", "source": SOURCE, "checked_at": now.isoformat(), "changes": [], "evidence_boundary": BOUNDARY, "untrusted_content": True}
    if not policy["enabled"]:
        return result, state
    allowed = quiet_until(now, policy.get("quiet_hours_utc"))
    due = timestamp(state["next_check_at"]) if state.get("next_check_at") else now
    if allowed > now or due > now:
        result.update(status="quiet" if allowed > now else "not_due", next_check_at=quiet_until(max(allowed, due), policy.get("quiet_hours_utc")).isoformat())
        return result, state
    updated = {**state, "seen": dict(state["seen"])}
    try:
        payload = fetch()
        if not isinstance(payload, dict) or payload.get("schema_version") != "agent-bounties/opportunity-projection-v1" or payload.get("network") != "base-mainnet" or payload.get("degraded") is not False:
            raise ValueError("projection unavailable")
        sources = payload.get("source_statuses")
        if not isinstance(sources, list) or not any(isinstance(s, dict) and s.get("source_type") == "canonical_base" and s.get("available") is True for s in sources):
            raise ValueError("canonical source unavailable")
        age = (now - timestamp(payload["generated_at"])).total_seconds()
        if not -120 <= age <= 900 or not isinstance(payload.get("items"), list) or len(payload["items"]) > 300:
            raise ValueError("stale or malformed projection")
        matches, invalid, identifiers = [], 0, set()
        filters = policy["filters"]
        for item in payload["items"]:
            if not isinstance(item, dict):
                raise ValueError("malformed opportunity")
            if item.get("network") != "base-mainnet" or item.get("source_type") != "canonical_base" or item.get("work_state") != "claimable" or item.get("payment_state") != "escrowed" or item.get("payment_committed") is not True:
                continue
            if any(wanted and not set(v.strip().casefold() for v in wanted).intersection(v.casefold() for v in item.get(key, []) if isinstance(v, str)) for key in ["skills", "categories"] if (wanted := filters.get(key, []))):
                continue
            try:
                entry = summary(item)
            except (TypeError, ValueError, AttributeError):
                invalid += 1
                continue
            if filters.get("minimum_reward_base_units") is not None and (entry["reward"] is None or int(entry["reward"]["amount"]) < int(filters["minimum_reward_base_units"])):
                continue
            identifier = entry["opportunity_id"]
            if identifier in identifiers:
                raise ValueError("duplicate opportunity identities")
            identifiers.add(identifier)
            if updated["seen"].get(identifier) != digest(entry):
                matches.append(entry)
        result.update(status="partial" if invalid else "changes" if matches else "unchanged", changes=matches[:20], pending_changes=max(0, len(matches)-20), invalid_items=invalid, projection_at=payload["generated_at"], result_limit=300)
        for entry in result["changes"]:
            identifier = entry["opportunity_id"]
            updated["seen"].pop(identifier, None)
            updated["seen"][identifier] = digest(entry)
        updated["seen"] = dict(list(updated["seen"].items())[-1000:])
        updated["failures"] = 0
    except (ValueError, TypeError, KeyError, AttributeError, OSError):
        updated["failures"] = min(20, state["failures"] + 1)
        result.update(status="unavailable", next_action="Keep previous work. Retry after next_check_at; no absence or payment conclusion is available.")
    delay = min(policy["max_backoff_minutes"], policy["interval_minutes"] * 2 ** updated["failures"])
    updated["next_check_at"] = quiet_until(now + timedelta(minutes=delay), policy.get("quiet_hours_utc")).isoformat()
    result["next_check_at"] = updated["next_check_at"]
    return result, updated


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--snapshot", type=Path, help="Use a local projection instead of making a request.")
    parser.add_argument("--stop", action="store_true", help="Set this policy inactive; starts no process.")
    args = parser.parse_args()
    paths = [p.resolve() for p in [args.config, args.state, args.snapshot] if p is not None]
    if len(paths) != len(set(paths)):
        parser.error("Config, state and snapshot must be different files.")
    policy = json.loads(args.config.read_text())
    validate_policy(policy)
    if args.stop:
        policy["enabled"] = False
        args.config.write_text(json.dumps(policy, indent=2) + "\n")
        print(json.dumps({"status": "inactive", "scheduler_started": False}))
        return
    lock = args.state.with_name(args.state.name + ".lock")
    with open(lock, "x", encoding="utf-8"):
        pass
    try:
        state = json.loads(args.state.read_text()) if args.state.exists() else {"schema_version": "agent-bounties/return-state-v1", "seen": {}, "failures": 0, "next_check_at": None}
        fetch = (lambda: json.loads(args.snapshot.read_text())) if args.snapshot else fetch_projection
        result, updated = check_once(policy, state, fetch)
        if updated != state:
            temporary = args.state.with_name(args.state.name + ".tmp")
            with open(temporary, "x", encoding="utf-8") as handle:
                json.dump(updated, handle, indent=2)
            os.replace(temporary, args.state)
    finally:
        lock.unlink()
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
