"""Offline captured-dump regression; no PostgreSQL, Docker or network calls."""
import hashlib
from pathlib import Path
import unittest
from unittest.mock import patch
from types import SimpleNamespace

import fixture_restore as restore
import image_runtime as runtime

HERE = Path(__file__).resolve().parent
FIXTURE = HERE / 'fixtures/pg-dump-search-path/core-before.sql'


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


class Controls(unittest.TestCase):
    def test_captured_dump_changes_only_the_expected_setting(self):
        raw = FIXTURE.read_bytes()
        self.assertEqual(len(raw), 30261)
        self.assertEqual(digest(raw), '121b9042e62bf519779048e34b9587eaa4ef63f0bf578d6e9b96f77af4296d2d')
        result = restore.replay(raw, 'core-journal-shipping-restart',
                                'recovery_funding_core', digest(raw))
        replacement = b"SELECT pg_catalog.set_config('search_path', 'recovery_funding_core', false);\n"
        self.assertEqual(result.replace(replacement, restore.RESET, 1), raw)
        self.assertEqual(result.count(replacement), 1)
        self.assertIn(b'INSERT INTO recovery_funding_core.relay_transaction_outbox', result)
        self.assertEqual(FIXTURE.read_bytes(), raw)

    def test_case_and_schema_are_fixed(self):
        for case, suffix in restore.CASES.items():
            schema = 'recovery_funding_' + suffix
            self.assertEqual(restore.schema_for(case), schema)
            raw = restore.RESET
            self.assertIn(schema.encode(), restore.replay(raw, case, schema, digest(raw)))
        for case, schema in [('unknown', 'recovery_funding_core'),
                             ('busy', 'recovery_funding_core'),
                             ('busy', 'recovery_funding_busy; DROP SCHEMA public'),
                             (None, 'recovery_funding_core')]:
            with self.subTest(case=case, schema=schema), self.assertRaises(ValueError):
                restore.replay(restore.RESET, case, schema, digest(restore.RESET))

    def test_missing_duplicate_or_unrecognized_resets_rejected(self):
        for raw in [b'SELECT 1;\n', restore.RESET * 2,
                    b'-- ' + restore.RESET,
                    restore.RESET.replace(b"'',", b"'public',"),
                    restore.RESET.replace(b'\n', b'\r\n'),
                    restore.RESET + b'SET search_path TO public;\n']:
            with self.subTest(raw=raw), self.assertRaisesRegex(ValueError, 'single pg_dump'):
                restore.replay(raw, 'busy', 'recovery_funding_busy', digest(raw))

    def test_original_hash_and_limits_required(self):
        raw = restore.RESET
        for bad_digest in [None, '', 'bad', 'A' * 64, '0' * 64]:
            with self.subTest(digest=bad_digest), self.assertRaises(ValueError):
                restore.replay(raw, 'busy', 'recovery_funding_busy', bad_digest)
        for bad in [b'', 'not bytes', b'a' * (restore.MAX_BYTES + 1)]:
            with self.subTest(size=len(bad)), self.assertRaises(ValueError):
                restore.replay(bad, 'busy', 'recovery_funding_busy', digest(raw))

    def test_runtime_psql_receives_only_in_memory_adaptation(self):
        ctx = SimpleNamespace(pg='synthetic-container', remaining=lambda limit: limit)
        raw = FIXTURE.read_bytes()
        expected = restore.replay(raw, 'core-journal-shipping-restart',
                                  'recovery_funding_core', digest(raw))
        with patch.object(runtime.subprocess, 'run', return_value=SimpleNamespace(
                returncode=0, stdout=b'', stderr=b'')) as run:
            runtime.sql(ctx, '', 'recovery_funding_core', FIXTURE,
                        'core-journal-shipping-restart', digest(raw))
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.kwargs['input'], b'SET search_path TO recovery_funding_core;\n' + expected)
        self.assertIn('ON_ERROR_STOP=1', run.call_args.args[0])
        self.assertEqual(FIXTURE.read_bytes(), raw)

    def test_digest_failure_prevents_psql(self):
        ctx = SimpleNamespace(pg='synthetic-container', remaining=lambda limit: limit)
        with patch.object(runtime.subprocess, 'run') as run, self.assertRaisesRegex(ValueError, 'original dump digest'):
            runtime.sql(ctx, '', 'recovery_funding_core', FIXTURE,
                        'core-journal-shipping-restart', '0' * 64)
        run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
