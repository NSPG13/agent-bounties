import contextlib
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import deploy_public_runtime as release


class TransitionHoldTests(unittest.TestCase):
    def cli(self, raw, arguments=(), environment=None, expected='public_runtime_transition_hold'):
        with tempfile.TemporaryDirectory() as temporary, contextlib.ExitStack() as stack:
            root = Path(temporary)
            (root / 'ops').mkdir()
            if raw is not None:
                (root / 'ops/public-runtime-services.json').write_text(raw)
            evidence = root / 'evidence'
            stack.enter_context(patch.object(release, 'ROOT', root))
            stack.enter_context(patch('sys.argv', ['release', '--revision', 'a' * 40,
                '--evidence-dir', str(evidence), *arguments]))
            stack.enter_context(patch.dict(os.environ, environment or {}, clear=True))
            # Even a credential lookup or approval check before the hold is a
            # regression: refusal must precede every external admission step.
            forbidden = []
            original_get = os.environ.get
            def checked_get(key, default=None):
                if key in {'RENDER_API_KEY', 'APPROVED_REVISION', 'RELEASE_EVENT', 'PRODUCTION_EXPECTED_REVISION'}:
                    self.fail('held run read release environment')
                return original_get(key, default)
            stack.enter_context(patch.object(os.environ, 'get', side_effect=checked_get))
            for target in ('validate_release_approval', 'public_ci_revision',
                           'reader_readiness', 'Render', 'repair.Render', 'repair.Ticket',
                           'repair.validate_request', 'urllib.request.urlopen',
                           'public_provenance_open', 'subprocess.run', 'subprocess.check_output'):
                forbidden.append(stack.enter_context(patch('deploy_public_runtime.' + target,
                    side_effect=AssertionError('held run touched ' + target))))
            with self.assertRaisesRegex(release.ReleaseError, '^' + expected + '$'):
                release.main()
            for operation in forbidden:
                operation.assert_not_called()
            self.assertFalse(evidence.exists())

    def test_every_present_hold_value_blocks_without_echoing_metadata(self):
        values = [None, False, True, 0, '', 'untrusted-secret', [], ['x'], {},
                  {'schema': 'unknown'}, {'case_id': 'other'}, {'enabled': False},
                  {'expires_at': '2000-01-01'},
                  json.loads((release.ROOT / 'ops/public-runtime-services.json').read_text())['transition_hold']]
        for value in values:
            with self.subTest(value=value):
                self.cli(json.dumps({'transition_hold': value}))

    def test_every_cli_admission_path_is_held_before_credentials_or_network(self):
        paths = [[], ['--migrate-source'], ['--migrate-source', '--bootstrap-autonomous-readers'],
                 ['--accepted-autonomous-deploy-id', 'dep-accepted'],
                 ['--bootstrap-repair-case', 'autonomous-bootstrap-20261106']]
        for arguments in paths:
            for event, approved, pin in [('workflow_run', '', 'a' * 40),
                                         ('workflow_dispatch', 'a' * 40, 'a' * 40),
                                         ('workflow_dispatch', 'a' * 40, 'b' * 40)]:
                with self.subTest(arguments=arguments, event=event, pin=pin):
                    self.cli('{"transition_hold": false}', arguments,
                        {'RELEASE_EVENT': event, 'APPROVED_REVISION': approved,
                         'PRODUCTION_EXPECTED_REVISION': pin})

    def test_missing_malformed_and_nonobject_inventory_fail_closed(self):
        for raw in [None, '{', 'null', 'false', '0', '[]', '"text"']:
            with self.subTest(raw=raw):
                self.cli(raw, expected='invalid_public_runtime_inventory')

    def test_direct_helpers_stop_before_native_or_readiness_requests(self):
        client = Mock()
        client.request.side_effect = AssertionError('provider reached')
        with patch.object(release, 'reader_readiness', side_effect=AssertionError('readiness reached')) as readiness:
            for hold in [None, False, {}, {'enabled': False}]:
                for invoke in [lambda m: release.prepare(client, m, True, True),
                               lambda m: release.prepare_release(client, m, True, True),
                               lambda m: release.repair_inventory(client, m)]:
                    with self.assertRaisesRegex(release.ReleaseError, '^public_runtime_transition_hold$'):
                        invoke({'transition_hold': hold})
            client.request.assert_not_called()
            readiness.assert_not_called()

    def test_actual_inventory_is_held_regardless_of_row_order_or_ignored_status(self):
        manifest = json.loads((release.ROOT / 'ops/public-runtime-services.json').read_text())
        for rows in [manifest['services'], list(reversed(manifest['services']))]:
            modified = copy.deepcopy(manifest)
            modified['services'] = copy.deepcopy(rows)
            for row in modified['services']:
                row['status'] = 'ready'
            self.cli(json.dumps(modified))

    def test_absent_key_retains_ordinary_policy_validation(self):
        release.enforce_transition_hold({})
        with self.assertRaisesRegex(release.ReleaseError, 'invalid public source policy'):
            release.prepare(Mock(), {}, False)


if __name__ == '__main__':
    unittest.main()
