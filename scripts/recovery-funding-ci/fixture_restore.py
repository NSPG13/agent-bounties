"""Replay a verified synthetic data dump in its fixed application schema.

pg_dump clears search_path even though restored rows can invoke application
triggers with unqualified table references. Adapt only that exact session
setting in memory; preserve the original dump and all trigger definitions.
"""
import hashlib
import re

from fixture_exports import CASES

RESET = b"SELECT pg_catalog.set_config('search_path', '', false);\n"
MAX_BYTES = 16 * 1024 ** 2


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def schema_for(case_name):
    require(isinstance(case_name, str) and case_name in CASES, 'unknown fixture case')
    schema = 'recovery_funding_' + CASES[case_name]
    require(re.fullmatch('recovery_funding_[a-z_]+', schema), 'invalid fixed schema')
    return schema


def replay(raw, case_name, schema, expected_sha256):
    require(isinstance(raw, bytes) and 0 < len(raw) <= MAX_BYTES, 'dump size/type')
    require(schema == schema_for(case_name), 'fixture schema mismatch')
    require(isinstance(expected_sha256, str) and
            re.fullmatch('[0-9a-f]{64}', expected_sha256), 'dump digest shape')
    require(hashlib.sha256(raw).hexdigest() == expected_sha256, 'original dump digest')
    require(raw.splitlines(keepends=True).count(RESET) == 1 and
            raw.count(b'search_path') == 1, 'expected single pg_dump search_path reset')
    replacement = ("SELECT pg_catalog.set_config('search_path', '" + schema + "', false);\n").encode('ascii')
    return raw.replace(RESET, replacement, 1)
