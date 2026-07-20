from __future__ import annotations

import argparse
import html
import json
import os
import re
import shutil
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path
from urllib.parse import urlencode, urlparse


OPPORTUNITY_SCHEMA = "agent-bounties/opportunity-projection-v1"
CLAIM_FUNNEL_SCHEMA = "agent-bounties/claim-funnel-v2"
NETWORK = "base-mainnet"
MAX_INPUT_BYTES = 25 * 1024 * 1024
MAX_SNAPSHOT_AGE = timedelta(minutes=20)
MAX_SOURCE_SKEW = timedelta(minutes=10)
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
HOME_CLAIMABLE_LIMIT = 5
HOME_SETTLED_LIMIT = 5
EARN_CLAIMABLE_LIMIT = 20


class SnapshotError(ValueError):
    pass


def parse_timestamp(value: object, label: str) -> datetime:
    if not isinstance(value, str) or not value.strip():
        raise SnapshotError(f"{label} must be an ISO-8601 timestamp")
    normalized = value.strip().replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError as error:
        raise SnapshotError(f"{label} must be an ISO-8601 timestamp") from error
    if parsed.tzinfo is None:
        raise SnapshotError(f"{label} must include a timezone")
    return parsed.astimezone(timezone.utc)


def load_json(path: Path, label: str) -> object:
    try:
        size = path.stat().st_size
    except OSError as error:
        raise SnapshotError(f"{label} is unavailable: {path}") from error
    if size <= 0 or size > MAX_INPUT_BYTES:
        raise SnapshotError(f"{label} must be between 1 byte and {MAX_INPUT_BYTES} bytes")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SnapshotError(f"{label} is not valid UTF-8 JSON") from error


def required_text(value: object, label: str, maximum: int) -> str:
    if not isinstance(value, str):
        raise SnapshotError(f"{label} must be text")
    compact = " ".join(value.split())
    if not compact or len(compact) > maximum:
        raise SnapshotError(f"{label} must contain 1 through {maximum} characters")
    return compact


def required_https_url(value: object, label: str) -> str:
    text = required_text(value, label, 2_048)
    parsed = urlparse(text)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise SnapshotError(f"{label} must be a public HTTPS URL")
    return text


def required_address(value: object, label: str) -> str:
    if not isinstance(value, str) or not ADDRESS.fullmatch(value):
        raise SnapshotError(f"{label} must be a 20-byte EVM address")
    return value.lower()


def base_units(value: object, label: str, *, positive: bool = False) -> int:
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        raise SnapshotError(f"{label} must be an integer base-unit amount")
    try:
        parsed = int(value)
    except (TypeError, ValueError) as error:
        raise SnapshotError(f"{label} must be an integer base-unit amount") from error
    if str(parsed) != str(value).strip() or parsed < 0 or (positive and parsed <= 0):
        boundary = "positive" if positive else "non-negative"
        raise SnapshotError(f"{label} must be a {boundary} integer base-unit amount")
    return parsed


def money_amount(value: object, label: str, *, positive: bool = False) -> int:
    if not isinstance(value, dict):
        raise SnapshotError(f"{label} must be a money object")
    if value.get("currency") != "USDC" or value.get("unit") != "base_units" or value.get("decimals") != 6:
        raise SnapshotError(f"{label} must be six-decimal USDC base units")
    return base_units(value.get("amount"), f"{label}.amount", positive=positive)


def format_usdc(amount: int) -> str:
    try:
        value = Decimal(amount) / Decimal(1_000_000)
    except (InvalidOperation, TypeError) as error:
        raise SnapshotError("USDC amount is invalid") from error
    rendered = f"{value:.6f}".rstrip("0").rstrip(".")
    return rendered if "." in rendered else f"{rendered}.00"


def human_timestamp(value: datetime) -> str:
    return value.strftime("%d %b %Y, %H:%M UTC")


def validate_projection(payload: object, now: datetime, fixture_mode: bool) -> tuple[dict, datetime, list[dict], list[dict]]:
    if not isinstance(payload, dict):
        raise SnapshotError("opportunity projection must be a JSON object")
    if payload.get("schema_version") != OPPORTUNITY_SCHEMA:
        raise SnapshotError(f"opportunity projection must use {OPPORTUNITY_SCHEMA}")
    if payload.get("network") != NETWORK:
        raise SnapshotError(f"opportunity projection must target {NETWORK}")
    generated_at = parse_timestamp(payload.get("generated_at"), "opportunity projection generated_at")
    age = now - generated_at
    if not fixture_mode and (age < -MAX_SOURCE_SKEW or age > MAX_SNAPSHOT_AGE):
        raise SnapshotError("opportunity projection is outside the production freshness window")

    statuses = payload.get("source_statuses")
    if not isinstance(statuses, list):
        raise SnapshotError("opportunity projection source_statuses must be an array")
    canonical_statuses = [item for item in statuses if isinstance(item, dict) and item.get("source_type") == "canonical_base"]
    if len(canonical_statuses) != 1 or canonical_statuses[0].get("available") is not True:
        raise SnapshotError("canonical Base opportunity source must be available")
    if "Only confirmed canonical BountySettled" not in str(payload.get("evidence_boundary", "")):
        raise SnapshotError("opportunity projection is missing its canonical payment boundary")

    items = payload.get("items")
    if not isinstance(items, list):
        raise SnapshotError("opportunity projection items must be an array")
    canonical_item_count = canonical_statuses[0].get("item_count")
    if (
        isinstance(canonical_item_count, bool)
        or not isinstance(canonical_item_count, int)
        or canonical_item_count < 0
    ):
        raise SnapshotError("canonical Base source item_count must be a non-negative integer")
    projected_canonical_count = sum(
        1
        for item in items
        if isinstance(item, dict) and item.get("source_type") == "canonical_base"
    )
    if projected_canonical_count != canonical_item_count:
        raise SnapshotError(
            "opportunity projection does not contain the complete canonical Base source set "
            f"(source_count={canonical_item_count}, projected_count={projected_canonical_count})"
        )
    claimable: list[dict] = []
    settled: list[dict] = []
    seen_contracts: set[str] = set()
    for index, item in enumerate(items):
        if not isinstance(item, dict) or item.get("source_type") != "canonical_base":
            continue
        contract = required_address(item.get("source_id"), f"opportunities.items[{index}].source_id")
        if contract in seen_contracts:
            raise SnapshotError(f"opportunity projection repeats canonical contract {contract}")
        seen_contracts.add(contract)
        is_claimable = (
            item.get("source_status") == "claimable"
            and item.get("work_state") == "claimable"
            and item.get("payment_state") == "escrowed"
            and item.get("payment_committed") is True
            and item.get("verification_ready") is True
        )
        is_settled = (
            item.get("source_status") == "paid"
            and item.get("work_state") == "completed"
            and item.get("payment_state") == "paid"
            and item.get("payment_committed") is True
        )
        if not is_claimable and not is_settled:
            continue
        try:
            item["_snapshot_contract"] = contract
            item["_snapshot_title"] = required_text(item.get("title"), f"opportunities.items[{index}].title", 180)
            item["_snapshot_goal"] = required_text(item.get("goal"), f"opportunities.items[{index}].goal", 1_200)
            item["_snapshot_source_url"] = required_https_url(
                item.get("source_url"), f"opportunities.items[{index}].source_url"
            )
            item["_snapshot_reward"] = money_amount(
                item.get("reward"), f"opportunities.items[{index}].reward", positive=True
            )
            item["_snapshot_updated_at"] = parse_timestamp(
                item.get("updated_at"), f"opportunities.items[{index}].updated_at"
            )
        except SnapshotError:
            if is_settled:
                continue
            raise
        if is_claimable:
            item["_snapshot_bond"] = money_amount(
                item.get("bond"), f"opportunities.items[{index}].bond", positive=True
            )
            item["_snapshot_timeout_bonus"] = money_amount(
                item.get("completion_bonus"),
                f"opportunities.items[{index}].completion_bonus",
            )
            item["_snapshot_funded"] = money_amount(
                item.get("funded_amount"),
                f"opportunities.items[{index}].funded_amount",
            )
            item["_snapshot_target"] = money_amount(
                item.get("funding_target"),
                f"opportunities.items[{index}].funding_target",
                positive=True,
            )
            if item["_snapshot_funded"] < item["_snapshot_target"]:
                raise SnapshotError(
                    f"opportunity projection contract {contract} is not fully funded"
                )
            claimable.append(item)
        elif is_settled:
            proofs = item.get("proof_urls")
            if not isinstance(proofs, list) or not proofs:
                continue
            try:
                item["_snapshot_proof_url"] = required_https_url(
                    proofs[0], f"opportunities.items[{index}].proof_urls[0]"
                )
            except SnapshotError:
                continue
            settled.append(item)

    settled.sort(key=lambda item: (item["_snapshot_updated_at"], item["_snapshot_contract"]), reverse=True)
    return payload, generated_at, claimable, settled


def validate_claim_funnel(payload: object, projection_at: datetime, now: datetime, fixture_mode: bool) -> dict:
    if not isinstance(payload, dict) or payload.get("schema_version") != CLAIM_FUNNEL_SCHEMA:
        raise SnapshotError(f"claim funnel must use {CLAIM_FUNNEL_SCHEMA}")
    if payload.get("window_hours") != 720:
        raise SnapshotError("claim funnel must use the 720-hour marketplace window")
    generated_at = parse_timestamp(payload.get("generated_at"), "claim funnel generated_at")
    if abs(generated_at - projection_at) > MAX_SOURCE_SKEW:
        raise SnapshotError("claim funnel and opportunity projection timestamps are inconsistent")
    age = now - generated_at
    if not fixture_mode and (age < -MAX_SOURCE_SKEW or age > MAX_SNAPSHOT_AGE):
        raise SnapshotError("claim funnel is outside the production freshness window")
    outcomes = payload.get("canonical_outcomes")
    if not isinstance(outcomes, dict):
        raise SnapshotError("claim funnel canonical_outcomes must be an object")
    settlements = outcomes.get("settlements_confirmed")
    if isinstance(settlements, bool) or not isinstance(settlements, int) or settlements < 0:
        raise SnapshotError("claim funnel settlements_confirmed must be a non-negative integer")
    if "only canonical BountySettled events prove payout" not in str(payload.get("evidence_boundary", "")):
        raise SnapshotError("claim funnel is missing its canonical payout boundary")
    return payload


def validate_claim_feed(payload: object, ready_by_contract: dict[str, dict]) -> list[dict]:
    if not isinstance(payload, list):
        raise SnapshotError("claimable feed must be a JSON array")
    indexed: list[tuple[int, dict, str]] = []
    seen: set[str] = set()
    for index, item in enumerate(payload):
        if not isinstance(item, dict):
            raise SnapshotError(f"claimable feed item {index} must be an object")
        contract = required_address(item.get("bounty_contract"), f"claimable_feed[{index}].bounty_contract")
        if contract in seen:
            raise SnapshotError(f"claimable feed repeats contract {contract}")
        seen.add(contract)
        indexed.append((index, item, contract))

    # Compare the complete fetched claimable sets before applying either page's
    # rendering cap, so source ordering cannot conceal a missing or extra record.
    projected = set(ready_by_contract)
    if seen != projected:
        projection_only = sorted(projected - seen)
        feed_only = sorted(seen - projected)
        raise SnapshotError(
            "claimable feed and opportunity projection contract sets disagree "
            f"(projection_only={projection_only[:5]!r}, feed_only={feed_only[:5]!r}, "
            f"projection_count={len(projected)}, feed_count={len(seen)})"
        )

    validated: list[dict] = []
    for index, item, contract in indexed:
        projection = ready_by_contract[contract]
        target = base_units(item.get("target_amount"), f"claimable_feed[{index}].target_amount", positive=True)
        funded = base_units(item.get("funded_amount"), f"claimable_feed[{index}].funded_amount")
        solver_reward = base_units(item.get("solver_reward"), f"claimable_feed[{index}].solver_reward", positive=True)
        claim_bond = base_units(item.get("claim_bond"), f"claimable_feed[{index}].claim_bond", positive=True)
        timeout_pool = base_units(item.get("timeout_bond_pool", 0), f"claimable_feed[{index}].timeout_bond_pool")
        if funded < target:
            raise SnapshotError(f"claimable feed contract {contract} is not fully funded")
        if item.get("status") != "claimable" or item.get("terms_valid") is not True or item.get("verification_ready") is not True:
            raise SnapshotError(f"claimable feed contract {contract} is not verifier-ready claimable work")
        terms = item.get("terms")
        document = terms.get("document") if isinstance(terms, dict) else None
        if not isinstance(document, dict):
            raise SnapshotError(f"claimable feed contract {contract} is missing public terms")
        title = required_text(document.get("title"), f"claimable_feed[{index}].terms.document.title", 180)
        goal = required_text(document.get("goal"), f"claimable_feed[{index}].terms.document.goal", 1_200)
        source_url = required_https_url(
            document.get("source_url"), f"claimable_feed[{index}].terms.document.source_url"
        )
        comparisons = {
            "title": (title, projection["_snapshot_title"]),
            "goal": (goal, projection["_snapshot_goal"]),
            "source URL": (source_url, projection["_snapshot_source_url"]),
            "solver reward": (solver_reward, projection["_snapshot_reward"]),
            "claim bond": (claim_bond, projection["_snapshot_bond"]),
            "timeout bonus": (timeout_pool, projection["_snapshot_timeout_bonus"]),
            "funded amount": (funded, projection["_snapshot_funded"]),
            "funding target": (target, projection["_snapshot_target"]),
        }
        disagreements = [name for name, values in comparisons.items() if values[0] != values[1]]
        if disagreements:
            raise SnapshotError(
                f"claimable feed contract {contract} disagrees with the opportunity projection on "
                + ", ".join(disagreements)
            )
        item["_snapshot_contract"] = contract
        item["_snapshot_title"] = title
        item["_snapshot_goal"] = goal
        item["_snapshot_source_url"] = source_url
        item["_snapshot_payout"] = solver_reward + timeout_pool
        item["_snapshot_bond"] = claim_bond
        validated.append(item)
    return validated


def escaped(value: object) -> str:
    return html.escape(str(value), quote=True)


def render_home_card(item: dict, kind: str) -> str:
    title = escaped(item["_snapshot_title"])
    goal = escaped(item["_snapshot_goal"])
    source_url = escaped(item["_snapshot_source_url"])
    contract = escaped(item["_snapshot_contract"])
    reward = escaped(format_usdc(item["_snapshot_reward"]))
    if kind == "claimable":
        bond = escaped(format_usdc(item["_snapshot_bond"]))
        query = urlencode({"bountyContract": item["_snapshot_contract"], "source": "indexable-snapshot"})
        primary_url = f"earn.html?{escaped(query)}"
        state = "Ready to earn at snapshot time"
        economics = f"{reward} USDC committed reward · {bond} USDC refundable bond"
        primary_label = "Check live status"
    else:
        primary_url = escaped(item["_snapshot_proof_url"])
        state = "Paid · canonical settlement recorded"
        economics = f"{reward} USDC solver reward · confirmed BountySettled evidence"
        primary_label = "View settlement proof"
    meta = ""
    if item.get("standing_meta_bounty"):
        meta = (
            '\n              <p class="fine opportunity-meta">Meta-bounty: create and fund qualifying work that '
            "a different wallet completes and receives canonical settlement for.</p>"
        )
    return f'''          <article class="bounty-row home-bounty-row" data-indexable-card data-indexable-kind="{kind}" data-bounty-contract="{contract}">
            <p class="opportunity-state opportunity-state-{'escrowed' if kind == 'claimable' else 'paid'}">{state}</p>
            <h3>{title}</h3>
            <p>{economics}</p>
            <p class="fine">{goal}</p>{meta}
            <div class="actions">
              <a class="button primary" href="{primary_url}">{primary_label}</a>
              <a class="button secondary" href="{source_url}">Read source terms</a>
            </div>
          </article>'''


def render_home_section(key: str, title: str, description: str, items: list[dict]) -> str:
    cards = "\n".join(render_home_card(item, key) for item in items)
    if not cards:
        cards = '          <p class="fine opportunity-empty">No canonical record matched this snapshot.</p>'
    return f'''      <section class="opportunity-section" aria-labelledby="indexable-{key}">
        <div class="opportunity-section-head">
          <div>
            <h3 id="indexable-{key}">{title}</h3>
            <p class="fine">{description}</p>
          </div>
          <span class="opportunity-count">{len(items)}</span>
        </div>
        <div class="bounty-feed home-bounty-feed">
{cards}
        </div>
      </section>'''


def render_home_board(generated_at: datetime, claimable: list[dict], settled: list[dict]) -> str:
    generated = escaped(generated_at.isoformat().replace("+00:00", "Z"))
    return f'''<div class="indexable-snapshot" data-indexable-snapshot data-snapshot-generated-at="{generated}">
{render_home_section("claimable", "Ready to earn", "Canonical work that was funded, claimable, and verifier-ready at snapshot time.", claimable[:HOME_CLAIMABLE_LIMIT])}
{render_home_section("settled", "Recently paid", "Completed work with confirmed canonical BountySettled evidence.", settled[:HOME_SETTLED_LIMIT])}
    </div>'''


def render_earn_card(item: dict) -> str:
    contract = escaped(item["_snapshot_contract"])
    title = escaped(item["_snapshot_title"])
    goal = escaped(item["_snapshot_goal"])
    source_url = escaped(item["_snapshot_source_url"])
    payout = escaped(format_usdc(item["_snapshot_payout"]))
    bond = escaped(format_usdc(item["_snapshot_bond"]))
    benchmark = item.get("terms", {}).get("document", {}).get("benchmark")
    disclosure = ""
    if isinstance(benchmark, dict) and benchmark.get("engine") == "standing_meta_v2_parent":
        disclosure = (
            '\n        <p class="bounty-disclosure">Meta-bounty economics: create and fully fund a qualifying child, '
            "then a different registered participant must complete and receive settlement for it. The parent reward is not guaranteed profit.</p>"
        )
    return f'''      <article class="bounty-row" data-indexable-card data-indexable-kind="claimable" data-bounty-contract="{contract}">
        <h3>{title}</h3>
        <p>{payout} USDC current solver payout | {bond} USDC solver bond | claimable at snapshot time</p>
        <p class="fine">{goal}</p>{disclosure}
        <a href="{source_url}" rel="noopener noreferrer">Read source issue and full acceptance criteria</a>
        <div class="actions">
          <button class="button primary" type="button" data-static-claim-action disabled>Live check required to claim</button>
        </div>
      </article>'''


def render_earn_board(generated_at: datetime, items: list[dict]) -> str:
    generated = escaped(generated_at.isoformat().replace("+00:00", "Z"))
    cards = "\n".join(render_earn_card(item) for item in items[:EARN_CLAIMABLE_LIMIT])
    if not cards:
        cards = '      <p class="fine opportunity-empty">No funded bounty was claimable in this canonical snapshot.</p>'
    return f'''<div class="indexable-snapshot" data-indexable-snapshot data-snapshot-generated-at="{generated}">
{cards}
    </div>'''


def replace_region(source: str, marker: str, content: str) -> str:
    start = f"<!-- {marker}:start -->"
    end = f"<!-- {marker}:end -->"
    if source.count(start) != 1 or source.count(end) != 1 or source.index(start) >= source.index(end):
        raise SnapshotError(f"site template must contain one ordered {marker} marker pair")
    prefix, remainder = source.split(start, 1)
    _, suffix = remainder.split(end, 1)
    return f"{prefix}{start}\n{content}\n        {end}{suffix}"


def render_templates(
    index_source: str,
    earn_source: str,
    generated_at: datetime,
    claimable: list[dict],
    settled: list[dict],
    claim_feed: list[dict],
    settlements_confirmed: int,
) -> tuple[str, str]:
    cutoff = generated_at - timedelta(hours=720)
    recent_settled = [item for item in settled if item["_snapshot_updated_at"] >= cutoff]
    available = sum(item["_snapshot_reward"] for item in claimable)
    paid = sum(item["_snapshot_reward"] for item in recent_settled)
    as_of = human_timestamp(generated_at)
    iso = generated_at.isoformat().replace("+00:00", "Z")
    latest_proof = settled[0]["_snapshot_proof_url"] if settled else None

    summary = (
        f'<output data-home-inventory-summary>{len(claimable)} funded bounties ready · '
        f'{format_usdc(available)} USDC available · {settlements_confirmed} confirmed payouts in 30 days</output>'
    )
    metrics = f'''<div class="hero-metrics" aria-label="Live marketplace metrics; canonical snapshot as of {escaped(as_of)}">
            <div>
              <output data-adoption-ready data-loaded="true" data-value="{len(claimable)}" aria-label="Funded bounties ready at snapshot time">{len(claimable)}</output>
              <span>funded bounties ready</span>
            </div>
            <div>
              <output data-adoption-available data-loaded="true" data-value="{format_usdc(available)}" aria-label="USDC solver rewards available at snapshot time">{format_usdc(available)}</output>
              <span>USDC in solver rewards available</span>
            </div>
            <div>
              <output data-adoption-settled data-loaded="true" data-value="{settlements_confirmed}" aria-label="Bounties paid in the last 30 days">{settlements_confirmed}</output>
              <span>confirmed payouts, 30 days</span>
            </div>
            <div>
              <output data-adoption-paid data-loaded="true" data-value="{format_usdc(paid)}" aria-label="USDC solver rewards settled in the last 30 days">{format_usdc(paid)}</output>
              <span>USDC in settled solver rewards, 30 days</span>
            </div>
          </div>'''
    proof = (
        f'<a data-market-proof href="{escaped(latest_proof)}">Open latest settlement proof</a>'
        if latest_proof
        else '<a data-market-proof hidden href="https://api.agentbounties.app/v1/base/autonomous-bounties/events?network=base-mainnet">Open latest settlement proof</a>'
    )
    source = f'''<p class="hero-metrics-source">
            <span>Canonical snapshot generated from the public Base opportunity projection.</span>
            <span>Only <code>BountySettled</code> counts as payment.</span>
            {proof}
            <time data-adoption-updated datetime="{escaped(iso)}">Snapshot as of {escaped(as_of)} · live revalidation starts on page load</time>
          </p>'''
    detail = (
        f'<output data-home-inventory-detail>Canonical snapshot as of {escaped(as_of)} · '
        f'{len(claimable)} claimable · {len(settled)} settled records · every action revalidates live state</output>'
    )
    earn_status = (
        f'<p class="fine snapshot-status" data-claimable-snapshot-status>'
        f'Snapshot as of <time datetime="{escaped(iso)}">{escaped(as_of)}</time>. '
        "Claim controls stay disabled until the live canonical feed is revalidated.</p>"
    )

    index_rendered = replace_region(index_source, "indexable-home-summary", summary)
    index_rendered = replace_region(index_rendered, "indexable-home-metrics", metrics)
    index_rendered = replace_region(index_rendered, "indexable-home-source", source)
    index_rendered = replace_region(index_rendered, "indexable-home-detail", detail)
    index_rendered = replace_region(
        index_rendered, "indexable-home-board", render_home_board(generated_at, claimable, settled)
    )
    earn_rendered = replace_region(
        earn_source, "indexable-earn-board", render_earn_board(generated_at, claim_feed)
    )
    earn_rendered = replace_region(earn_rendered, "indexable-earn-status", earn_status)
    return index_rendered, earn_rendered


def build_site(
    source_dir: Path,
    output_dir: Path,
    opportunities_path: Path,
    claim_feed_path: Path,
    claim_funnel_path: Path,
    *,
    fixture_mode: bool = False,
    now: datetime | None = None,
) -> dict[str, object]:
    source_dir = source_dir.resolve()
    output_dir = output_dir.resolve()
    if not source_dir.is_dir():
        raise SnapshotError(f"site source directory is unavailable: {source_dir}")
    if output_dir.exists():
        raise SnapshotError(f"staging output already exists: {output_dir}")
    if output_dir == source_dir or source_dir in output_dir.parents:
        raise SnapshotError("staging output must not be the source directory or a child of it")
    if fixture_mode:
        fixture_root = (Path(__file__).resolve().parent / "fixtures").resolve()
        for path in (opportunities_path, claim_feed_path, claim_funnel_path):
            try:
                path.resolve().relative_to(fixture_root)
            except ValueError as error:
                raise SnapshotError("fixture mode accepts only committed scripts/fixtures inputs") from error

    reference_now = (now or datetime.now(timezone.utc)).astimezone(timezone.utc)
    projection_payload = load_json(opportunities_path, "opportunity projection")
    claim_feed_payload = load_json(claim_feed_path, "claimable feed")
    claim_funnel_payload = load_json(claim_funnel_path, "claim funnel")
    projection, generated_at, claimable, settled = validate_projection(
        projection_payload, reference_now, fixture_mode
    )
    funnel = validate_claim_funnel(claim_funnel_payload, generated_at, reference_now, fixture_mode)
    ready_by_contract = {item["_snapshot_contract"]: item for item in claimable}
    claim_feed = validate_claim_feed(claim_feed_payload, ready_by_contract)
    index_source = (source_dir / "index.html").read_text(encoding="utf-8")
    earn_source = (source_dir / "earn.html").read_text(encoding="utf-8")
    index_rendered, earn_rendered = render_templates(
        index_source,
        earn_source,
        generated_at,
        claimable,
        settled,
        claim_feed,
        funnel["canonical_outcomes"]["settlements_confirmed"],
    )

    output_dir.parent.mkdir(parents=True, exist_ok=True)
    temporary_root = Path(tempfile.mkdtemp(prefix=".indexable-site-", dir=output_dir.parent))
    staged_site = temporary_root / "site"
    try:
        shutil.copytree(source_dir, staged_site)
        (staged_site / "index.html").write_text(index_rendered, encoding="utf-8", newline="\n")
        (staged_site / "earn.html").write_text(earn_rendered, encoding="utf-8", newline="\n")
        os.replace(staged_site, output_dir)
    finally:
        shutil.rmtree(temporary_root, ignore_errors=True)

    return {
        "schema_version": "agent-bounties/indexable-site-build-v1",
        "generated_at": generated_at.isoformat().replace("+00:00", "Z"),
        "homepage_claimable": min(len(claimable), HOME_CLAIMABLE_LIMIT),
        "homepage_settled": min(len(settled), HOME_SETTLED_LIMIT),
        "earn_claimable": min(len(claim_feed), EARN_CLAIMABLE_LIMIT),
        "output": str(output_dir),
        "projection_degraded": projection.get("degraded") is True,
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build a fail-closed, indexable GitHub Pages staging copy.")
    parser.add_argument("--source", required=True, type=Path, help="Static site source directory")
    parser.add_argument("--output", required=True, type=Path, help="New staging directory; it must not exist")
    parser.add_argument("--opportunities", required=True, type=Path, help="Authoritative opportunity projection JSON")
    parser.add_argument("--claim-feed", required=True, type=Path, help="Authoritative claimable canonical feed JSON")
    parser.add_argument("--claim-funnel", required=True, type=Path, help="Authoritative 720-hour claim funnel JSON")
    parser.add_argument(
        "--fixture-mode",
        action="store_true",
        help="Allow dated committed scripts/fixtures inputs for pull-request validation only",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        result = build_site(
            args.source,
            args.output,
            args.opportunities,
            args.claim_feed,
            args.claim_funnel,
            fixture_mode=args.fixture_mode,
        )
    except (OSError, SnapshotError) as error:
        print(f"indexable site build failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
