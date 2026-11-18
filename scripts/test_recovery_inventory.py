import copy
import json
import unittest
from unittest.mock import Mock, patch

import deploy_public_runtime as release
import materialize_recovery_inventory as inventory


class RecoveryInventoryTests(unittest.TestCase):
    def setUp(self):
        self.base = inventory.BASE.read_bytes()
        # Synthetic fixture only. No production mapping is written by this test.
        self.binding = {**inventory.FIXED_BINDING, 'reader_service_id': 'srv-aaaaaaaaaaaaaaaaaaaa'}

    def test_only_six_reviewed_paths_change_and_history_and_hold_are_preserved(self):
        before = json.loads(self.base)
        after = inventory.materialize(self.base, self.binding)
        self.assertEqual(inventory.changed_paths(before, after), inventory.ALLOWED_PATHS)
        for key in ['blueprints', 'autonomous_bootstrap_service_id', 'transition_hold']:
            self.assertEqual(before[key], after[key])
        inventory.validate_materialized(after, self.base, self.binding)
        with self.assertRaises(inventory.InventoryError):
            inventory.materialize(json.dumps(after).encode(), self.binding)

    def test_missing_typo_or_modified_hold_and_extra_base_edits_are_rejected(self):
        for mutation in ['missing', 'typo', 'value', 'extra', 'blueprint', 'bootstrap', 'whitespace']:
            base = json.loads(self.base)
            if mutation == 'missing':del base['transition_hold']
            elif mutation == 'typo':base['transitionHold'] = base.pop('transition_hold')
            elif mutation == 'value':base['transition_hold'] = False
            elif mutation == 'extra':base['unreviewed'] = True
            elif mutation == 'blueprint':base['blueprints'][0]['resource_ids'].pop()
            elif mutation == 'bootstrap':base['autonomous_bootstrap_service_id'] = self.binding['reader_service_id']
            with self.subTest(mutation=mutation), self.assertRaises(inventory.InventoryError):
                inventory.materialize(json.dumps(base).encode(), self.binding)

    def test_wrong_workspace_name_shape_old_duplicate_and_malformed_ids_are_rejected(self):
        for key, value in [('workspace_id', 'tea-other'), ('reader_name', 'other'),
                           ('region', 'virginia'), ('type', 'web_service'), ('schema', 'other'),
                           ('extra', True), ('reader_service_id', ''), ('reader_service_id', None),
                           ('reader_service_id', 'srv-; echo bad')]:
            with self.subTest(key=key, value=value), self.assertRaises(inventory.InventoryError):
                inventory.materialize(self.base, {**self.binding, key: value})
        for value in [inventory.API_ID, *(row['id'] for row in json.loads(self.base)['services'])]:
            with self.subTest(identity=value), self.assertRaises(inventory.InventoryError):
                inventory.materialize(self.base, {**self.binding, 'reader_service_id': value})

    def test_second_binding_and_all_extra_candidate_edits_fail_validation(self):
        valid = inventory.materialize(self.base, self.binding)
        mutations = []
        changed = copy.deepcopy(valid);changed['services'][2]['id'] = 'srv-bbbbbbbbbbbbbbbbbbbb';mutations.append(changed)
        for key in ['transition_hold', 'blueprints', 'autonomous_bootstrap_service_id']:
            changed = copy.deepcopy(valid);changed.pop(key);mutations.append(changed)
        changed = copy.deepcopy(valid);changed['transitionHold'] = changed.pop('transition_hold');mutations.append(changed)
        changed = copy.deepcopy(valid);changed['services'][3]['status'] = 'held';mutations.append(changed)
        changed = copy.deepcopy(valid);changed['services'].reverse();mutations.append(changed)
        for changed in mutations:
            with self.assertRaises(inventory.InventoryError):
                inventory.validate_materialized(changed, self.base, self.binding)

    def test_materialized_inventory_cannot_admit_any_ordinary_release(self):
        manifest = inventory.materialize(self.base, self.binding)
        client = Mock()
        client.request.side_effect = AssertionError('provider request forbidden')
        with patch.object(release, 'reader_readiness', side_effect=AssertionError('network forbidden')) as readiness:
            for reorder in [False, True]:
                if reorder:manifest['services'].reverse()
                for row in manifest['services']:row['status'] = 'ready'
                for call in [lambda: release.prepare(client, manifest, True),
                             lambda: release.prepare_release(client, manifest, True),
                             lambda: release.repair_inventory(client, manifest)]:
                    with self.assertRaisesRegex(release.ReleaseError, '^public_runtime_transition_hold$'):
                        call()
            client.request.assert_not_called()
            readiness.assert_not_called()


if __name__ == '__main__':
    unittest.main()
