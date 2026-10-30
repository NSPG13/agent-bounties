#!/usr/bin/env python3
"""Bounded read-only marketplace probe. Never authorizes a restart or a payment."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import urllib.error
import urllib.request

PROTOCOLS = {
    'agent-bounties/autonomous-v1',
    'agent-bounties/open-competition-v1',
    'agent-bounties/open-competition-v2-beta3',
}
READER_BLOCKERS = {
    'heartbeat_missing', 'reader_failed', 'safe_head_unavailable',
    'latest_head_unavailable', 'head_order_inconsistent',
    'durable_cursor_unavailable', 'cursor_inconsistent',
    'cursor_ahead_of_latest_head', 'heartbeat_in_future',
    'reader_stale_or_behind', 'cursor_ahead_of_observed_head',
}
UNAVAILABLE_REASONS = {
    'marketplace_head_unavailable': 'A required chain head is unavailable; reader readiness cannot be established.',
    'marketplace_reader_unavailable': 'A reader reported unavailable or inconsistent evidence; staleness is not established.',
    'marketplace_readiness_invalid': 'The marketplace readiness report is malformed or inconsistent.',
    'marketplace_readiness_stale': 'The marketplace readiness report is too old to assess current readers.',
    'marketplace_readiness_clock_skew': 'The marketplace readiness report is dated in the future; check the observer and service clocks.',
    'marketplace_probe_unavailable': 'The marketplace readiness endpoint could not be read.',
    'marketplace_readiness_http_inconsistent': 'The readiness HTTP status contradicts its healthy report.',
}


def unavailable(blocker, reason=None, readers=None):
    result = {'state': 'unavailable', 'restartable': False,
              'blocker': blocker, 'readers': readers or []}
    if reason:
        result['validation_reason'] = reason
    return result


def bounded_integer(value, minimum=0):
    return type(value) is int and minimum <= value <= 2**64 - 1


def enabled_for_release(expected_revision, verified_revision):
    """Activate only after an operator records the verified endpoint release."""
    if not verified_revision:
        return False
    if (not isinstance(expected_revision, str)
            or not isinstance(verified_revision, str)
            or not re.fullmatch(r'[0-9a-fA-F]{40}', expected_revision)
            or not re.fullmatch(r'[0-9a-fA-F]{40}', verified_revision)
            or expected_revision.lower() != verified_revision.lower()):
        raise ValueError('marketplace readiness verified revision must match the exact expected release')
    return True


def inspect(payload, now=None):
    now = now or datetime.now(timezone.utc)
    def invalid(reason):
        return unavailable('marketplace_readiness_invalid', reason)

    if not isinstance(payload, dict) or payload.get('schema') != 'agent-bounties/marketplace-readiness-v1':
        return invalid('schema_invalid')
    try:
        checked = datetime.fromisoformat(payload['checked_at'].replace('Z', '+00:00'))
        if checked.tzinfo is None:
            return invalid('checked_at_invalid')
        age_seconds = (now - checked).total_seconds()
        if age_seconds < 0:
            return dict(unavailable('marketplace_readiness_clock_skew', 'report_in_future'),
                        checked_at=checked.astimezone(timezone.utc).isoformat(), report_age_seconds=age_seconds)
        if age_seconds > 90:
            return dict(unavailable('marketplace_readiness_stale', 'report_stale'),
                        checked_at=checked.astimezone(timezone.utc).isoformat(), report_age_seconds=age_seconds)
    except (KeyError, TypeError, ValueError, AttributeError, OverflowError):
        return invalid('checked_at_invalid')
    try:
        readers = payload['readers']
        if not isinstance(readers, list) or len(readers) not in (len(PROTOCOLS), len(PROTOCOLS) + 1):
            return invalid('readers_invalid')
        if (not all(isinstance(r, dict) and isinstance(r.get('protocol'), str) for r in readers)
                or len({r['protocol'] for r in readers}) != len(readers)
                or {r['protocol'] for r in readers} not in (
                    PROTOCOLS, PROTOCOLS | {'agent-bounties/autonomous-v2'})
                or type(payload.get('ready')) is not bool):
            return invalid('readers_invalid')
        safe, latest = payload['safe_block'], payload['latest_block']
        missing_heads = safe is None or latest is None
        if ((safe is not None and not bounded_integer(safe))
                or (latest is not None and not bounded_integer(latest))
                or (not missing_heads and safe > latest)):
            return invalid('head_order_invalid')
        if missing_heads and payload['ready']:
            return invalid('head_readiness_inconsistent')
        checks = []
        for r in readers:
            age, cursor, lag = (r.get(k) for k in ('heartbeat_age_seconds', 'cursor_block', 'lag_blocks'))
            blocker = r.get('blocker')
            if (type(r.get('ready')) is not bool
                    or (blocker is not None and (not isinstance(blocker, str) or blocker not in READER_BLOCKERS))
                    or (age is not None and not bounded_integer(age, -(2**63)))
                    or (cursor is not None and not bounded_integer(cursor))
                    or (lag is not None and not bounded_integer(lag))):
                return invalid('reader_fields_invalid')
            if r['ready'] != (blocker is None):
                return invalid('reader_readiness_inconsistent')
            if missing_heads and (r['ready'] or blocker is None
                                  or (safe is None and lag is not None)):
                return invalid('head_readiness_inconsistent')
            if ((cursor is None and lag is not None)
                    or (safe is not None and cursor is not None
                        and (lag is None or lag != max(0, safe - cursor)))):
                return invalid('reader_lag_inconsistent')
            healthy = (not missing_heads and r['ready'] and blocker is None
                       and type(age) is int and 0 <= age <= 90
                       and type(cursor) is int and 0 <= cursor <= latest
                       and type(lag) is int and 0 <= lag <= 20
                       and lag == max(0, safe - cursor))
            checks.append({'protocol': r['protocol'], 'ready': healthy,
                           'blocker': blocker, 'heartbeat_age_seconds': age,
                           'cursor_block': cursor, 'lag_blocks': lag})
        if missing_heads:
            result = unavailable('marketplace_head_unavailable', readers=checks)
        elif payload.get('ready') is True and all(r['ready'] for r in checks):
            result = {'state': 'healthy', 'restartable': False, 'blocker': None, 'readers': checks}
        elif any((r['heartbeat_age_seconds'] is not None and r['heartbeat_age_seconds'] > 90)
                 or (r['lag_blocks'] is not None and r['lag_blocks'] > 20)
                 or r['blocker'] == 'reader_stale_or_behind' for r in checks):
            result = {'state': 'stale', 'restartable': False,
                      'blocker': 'marketplace_reader_not_ready', 'readers': checks}
        else:
            result = unavailable('marketplace_reader_unavailable', readers=checks)
        result['checked_at'] = checked.astimezone(timezone.utc).isoformat()
        result['report_age_seconds'] = age_seconds
        return result
    except (KeyError, TypeError, ValueError, AttributeError, OverflowError):
        return invalid('readers_invalid')


def observe(base_url, timeout_seconds=15):
    evidence = {'started_at': datetime.now(timezone.utc).isoformat(), 'http_status': None}
    request = urllib.request.Request(base_url.rstrip('/') + '/readiness/marketplace',
                                     headers={'User-Agent': 'Agentizable-MarketplaceMonitor/1'})
    try:
        try:
            response = urllib.request.urlopen(request, timeout=timeout_seconds)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            status = response.status
            raw = response.read(65537)
        evidence.update(http_status=status, response_bytes=len(raw),
                        response_sha256=hashlib.sha256(raw).hexdigest(),
                        response_sha256_scope='bounded_prefix' if len(raw) > 65536 else 'complete_body')
        if status not in (200, 503):
            result = unavailable('marketplace_probe_unavailable', 'http_status_unavailable')
        elif len(raw) > 65536:
            result = unavailable('marketplace_readiness_invalid', 'response_too_large')
        else:
            try:
                result = inspect(json.loads(raw))
            except (ValueError, UnicodeError):
                result = unavailable('marketplace_readiness_invalid', 'json_invalid')
            if status != 200 and result['state'] == 'healthy':
                result = unavailable('marketplace_readiness_http_inconsistent', 'http_status_inconsistent')
    except (urllib.error.URLError, TimeoutError, OSError, ValueError):
        result = unavailable('marketplace_probe_unavailable', 'transport_unavailable')
    evidence['finished_at'] = datetime.now(timezone.utc).isoformat()
    result['probe_evidence'] = evidence
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-base-url', required=True)
    parser.add_argument('--expected-revision')
    parser.add_argument('--verified-revision')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args(argv)
    try:
        enabled = enabled_for_release(args.expected_revision, args.verified_revision)
    except ValueError as error:
        parser.error(str(error))
    result = observe(args.api_base_url) if enabled else {
        'state': 'unknown', 'restartable': False,
        'blocker': 'marketplace_readiness_not_activated', 'readers': [],
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(result, indent=2))
    return 0 if not enabled or result['state'] == 'healthy' else 1


if __name__ == '__main__':
    raise SystemExit(main())
