#!/usr/bin/env python3
"""Parse an exact nonsecret predeploy receipt; no network or executable startup.

Provider log framing must be removed separately. Partial, repeated, reordered or
extra records are not a receipt. A parsed receipt still needs provider provenance.
"""
import argparse
import json
from pathlib import Path
import re


def parse_receipt(text: str, expected_revision: str, expected_kind: str) -> dict[str, str]:
    if (not isinstance(text, str) or len(text) > 2048
            or re.fullmatch('[0-9a-f]{40}', expected_revision) is None
            or expected_kind not in ('api', 'reader', 'funding')):
        raise ValueError('invalid receipt inputs')
    names = ['schema', 'source_revision', 'artifact_kind', 'binary_sha256']
    if expected_kind == 'reader':
        names += ['supervisor_sha256', 'bash_sha256']
    elif expected_kind == 'funding':
        names += ['launcher_sha256']
    names += ['guard_sha256', 'receipt_complete']
    lines = text.split('\n')
    if not lines or lines.pop() != '' or len(lines) != len(names):
        raise ValueError('invalid receipt framing')
    values = {}
    for name, line in zip(names, lines):
        prefix = name + '='
        if not line.startswith(prefix):
            raise ValueError('invalid receipt fields')
        values[name] = line[len(prefix):]
    if (values['schema'] != 'agentizable/recovery-artifact-receipt-v1'
            or values['source_revision'] != expected_revision
            or values['artifact_kind'] != expected_kind
            or values['receipt_complete'] != 'true'
            or any(re.fullmatch('[0-9a-f]{64}', value) is None
                   for name, value in values.items() if name.endswith('_sha256'))):
        raise ValueError('invalid receipt identity or digest')
    return values


def verify_file_hashes(receipt: dict[str, str], text: str) -> None:
    """Compare with a separate fixed-path sha256sum observation of the same image."""
    expected = {'/usr/local/bin/check-recovery-revision': receipt['guard_sha256']}
    if receipt['artifact_kind'] == 'api':
        expected['/usr/local/bin/agent-bounties-service'] = receipt['binary_sha256']
    elif receipt['artifact_kind'] == 'reader':
        expected.update({'/usr/local/bin/recovery-reader': receipt['binary_sha256'],
                         '/usr/local/bin/run-recovery-indexers': receipt['supervisor_sha256'],
                         '/bin/bash': receipt['bash_sha256']})
    elif receipt['artifact_kind'] == 'funding':
        expected.update({'/usr/local/bin/managed-funding-worker': receipt['binary_sha256'],
                         '/usr/local/bin/managed-funding-launcher': receipt['launcher_sha256']})
    else:
        raise ValueError('invalid artifact kind')
    actual = {}
    for line in text.splitlines():
        digest, separator, path = line.partition('  ')
        if not separator or path in actual or re.fullmatch('[0-9a-f]{64}', digest) is None:
            raise ValueError('invalid image hash observation')
        actual[path] = digest
    if actual != expected:
        raise ValueError('image hash observation mismatch')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('receipt', type=Path)
    parser.add_argument('--expected-revision', required=True)
    parser.add_argument('--expected-kind', required=True, choices=('api', 'reader', 'funding'))
    parser.add_argument('--file-hashes', type=Path)
    args = parser.parse_args()
    try:
        if args.receipt.stat().st_size > 2048:
            raise ValueError('receipt too large')
        receipt = parse_receipt(args.receipt.read_bytes().decode('ascii'),
                                args.expected_revision, args.expected_kind)
        if args.file_hashes:
            if args.file_hashes.stat().st_size > 2048:
                raise ValueError('image hash observation too large')
            verify_file_hashes(receipt, args.file_hashes.read_bytes().decode('ascii'))
    except (OSError, UnicodeError, ValueError):
        parser.exit(1, 'recovery artifact receipt rejected\n')
    print(json.dumps(receipt, sort_keys=True))


if __name__ == '__main__':
    main()
