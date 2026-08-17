#!/usr/bin/env python3
"""Select one canonically claimable coding bounty from an inventory snapshot.

Reads an inventory JSON file and emits a single JSON object on stdout carrying an
`action` and exactly one `next_action` string.

Actions
-------
claim    one canonical, claimable, positive-margin bounty was selected
wait     the inventory is empty; nothing to act on
refresh  the inventory snapshot is stale and must be re-fetched before acting
skip     candidates exist but none are actionable (no margin, or exclusively claimed)

WALLET SAFETY: this module never reads, stores, or transmits key material, and it
never broadcasts a transaction. It emits an unsigned intent for an external,
operator-controlled signer.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

# An inventory older than this is not trustworthy for a claim decision: another
# solver may already hold an exclusive claim that the snapshot cannot show.
DEFAULT_STALENESS_SECONDS = 900

CODING_HINTS = (
    "code", "coding", "api", "cli", "mcp", "sdk", "integration", "environment",
    "checker", "failover", "harness", "benchmark", "agent", "tooling", "fix",
    "implement", "repair", "add", "deterministic", "software",
)

LIVE_CLAIM_STATES = ("in_progress", "submitted", "claimed", "exclusive")


def _units(obj):
    """Decode a {amount, decimals} money object into a float. Missing -> 0.0."""
    if not isinstance(obj, dict):
        return 0.0
    amount = obj.get("amount")
    if amount in (None, ""):
        return 0.0
    try:
        decimals = int(obj.get("decimals", 6))
        return int(amount) / (10 ** decimals)
    except (TypeError, ValueError):
        return 0.0


def _snapshot_age(inv):
    """Age of the snapshot in seconds, or None when no timestamp is present."""
    ts = inv.get("generated_at") or inv.get("snapshot_at") or inv.get("as_of")
    if isinstance(ts, (int, float)):
        return max(0.0, time.time() - float(ts))
    if isinstance(ts, str) and ts:
        cleaned = ts.replace("Z", "+00:00")
        try:
            import datetime as _dt

            parsed = _dt.datetime.fromisoformat(cleaned)
            if parsed.tzinfo is None:
                parsed = parsed.replace(tzinfo=_dt.timezone.utc)
            return max(0.0, time.time() - parsed.timestamp())
        except ValueError:
            return None
    age = inv.get("age_seconds") or inv.get("snapshot_age_seconds")
    if isinstance(age, (int, float)):
        return float(age)
    return None


def _is_coding(item):
    text = f"{item.get('title', '')} {item.get('goal', '')}".lower()
    cats = " ".join(item.get("categories") or []).lower()
    skills = " ".join(item.get("skills") or []).lower()
    blob = f"{text} {cats} {skills}"
    return any(hint in blob for hint in CODING_HINTS)


def _exclusively_claimed(item):
    """True when another solver holds a live exclusive claim we must not contest."""
    if item.get("exclusive_claimant") or item.get("active_claimant"):
        return True
    if item.get("claim_expired") is True or item.get("reclaimable") is True:
        return False
    expires = item.get("claim_expires_at")
    if isinstance(expires, (int, float)) and expires > 0:
        return expires > time.time()
    mode = str(item.get("competition_mode") or "").lower()
    state = str(item.get("work_state") or "").lower()
    if state in LIVE_CLAIM_STATES and "exclusive" in mode:
        return True
    return state in LIVE_CLAIM_STATES


def _margin(item):
    """Cash margin: explicit gross margin when present, else reward - external spend."""
    econ = item.get("cash_economics") or {}
    if "gross_cash_margin" in econ:
        return _units(econ.get("gross_cash_margin"))
    reward = _units(item.get("reward") or econ.get("solver_reward"))
    return reward - _units(econ.get("required_external_spend"))


def select(inv):
    items = inv.get("items") or inv.get("opportunities") or inv.get("bounties") or []

    if not items:
        return {
            "action": "wait",
            "reason": "inventory contains no opportunities",
            "next_action": (
                "Re-poll https://api.agentbounties.app/v1/opportunities and wait for "
                "newly funded canonical work before claiming."
            ),
            "selected": None,
        }

    age = _snapshot_age(inv)
    max_age = float(inv.get("staleness_seconds") or DEFAULT_STALENESS_SECONDS)
    if age is not None and age > max_age:
        return {
            "action": "refresh",
            "reason": f"inventory snapshot is {int(age)}s old (limit {int(max_age)}s)",
            "next_action": (
                "Re-fetch https://api.agentbounties.app/v1/opportunities to obtain a "
                "fresh snapshot; a stale inventory can hide a live exclusive claim."
            ),
            "selected": None,
        }

    candidates, skipped = [], []
    for item in items:
        title = item.get("title") or item.get("opportunity_id") or "(untitled)"
        state = str(item.get("work_state") or "").lower()

        if state in ("completed", "settled", "cancelled"):
            skipped.append((title, "already completed"))
            continue
        if not _is_coding(item):
            skipped.append((title, "not coding work"))
            continue
        if _exclusively_claimed(item):
            skipped.append((title, "exclusive claimant active"))
            continue
        margin = _margin(item)
        if margin <= 0:
            skipped.append((title, f"no positive margin ({margin:.2f})"))
            continue
        candidates.append((margin, item))

    if not candidates:
        detail = "; ".join(f"{t}: {r}" for t, r in skipped[:5]) or "no eligible items"
        return {
            "action": "skip",
            "reason": detail,
            "next_action": (
                "Skip this inventory: no canonical claimable coding bounty has positive "
                "margin and a free claim slot. Re-poll after the next funding round."
            ),
            "selected": None,
            "skipped": [{"title": t, "reason": r} for t, r in skipped],
        }

    # Highest margin first; lowest bond breaks ties (least capital at risk).
    candidates.sort(key=lambda pair: (-pair[0], _units(pair[1].get("bond"))))
    margin, best = candidates[0]
    contract = best.get("source_id") or best.get("bounty_contract") or ""

    return {
        "action": "claim",
        "reason": f"highest positive margin ({margin:.2f} USDC) with no exclusive claimant",
        "next_action": (
            "POST https://api.agentbounties.app/v1/base/autonomous-bounties/claim-plan "
            f'with {{"network":"base-mainnet","bounty_contract":"{contract}",'
            '"solver":"<PUBLIC_WALLET>"}, then approve the exact bond and call claim(). '
            "Sign externally; never expose key material."
        ),
        "selected": {
            "opportunity_id": best.get("opportunity_id"),
            "title": best.get("title"),
            "bounty_contract": contract,
            "reward_usdc": round(_units(best.get("reward")), 6),
            "bond_usdc": round(_units(best.get("bond")), 6),
            "margin_usdc": round(margin, 6),
        },
        "considered": len(items),
        "eligible": len(candidates),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="path to an inventory JSON file")
    args = parser.parse_args(argv)

    path = Path(args.input)
    if not path.is_file():
        print(json.dumps({"action": "refresh", "reason": f"missing inventory file: {path}",
                          "next_action": "Fetch a fresh inventory snapshot before selecting.",
                          "selected": None}))
        return 0

    try:
        inv = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        print(json.dumps({"action": "refresh", "reason": f"invalid inventory JSON: {error}",
                          "next_action": "Re-fetch the inventory; the snapshot is unparseable.",
                          "selected": None}))
        return 0

    print(json.dumps(select(inv), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
