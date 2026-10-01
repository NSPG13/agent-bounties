import os
import unittest
from unittest.mock import patch
import posting_production_cutover as controller


class CutoverTests(unittest.TestCase):
    def test_unattended_event_never_reaches_network(self):
        with patch.dict(os.environ, {'GITHUB_REPOSITORY': 'NSPG13/agent-bounties',
                'GITHUB_REF': 'refs/heads/' + controller.BRANCH,
                'GITHUB_EVENT_NAME': 'push'}, clear=True), patch.object(controller, 'api') as api:
            with self.assertRaisesRegex(RuntimeError, 'attended dispatch'):
                controller.run('pause-and-api')
            api.assert_not_called()

    def test_other_branch_never_reaches_network(self):
        with patch.dict(os.environ, {'GITHUB_REPOSITORY': 'NSPG13/agent-bounties',
                'GITHUB_REF': 'refs/heads/main', 'GITHUB_EVENT_NAME': 'workflow_dispatch'}, clear=True), \
                patch.object(controller, 'api') as api:
            with self.assertRaises(RuntimeError):
                controller.run('pause-and-api')
            api.assert_not_called()

    def test_service_identity_and_paid_plan_cannot_drift(self):
        base = {'id': controller.SERVICES['api'], 'ownerId': controller.WORKSPACE,
                'repo': controller.PRIVATE_REPO, 'serviceDetails': {'plan': 'starter', 'region': 'oregon'},
                'autoDeploy': 'no', 'type': 'web_service'}
        with patch.object(controller, 'api', return_value=base):
            self.assertEqual(controller.service('api'), base)
        for key, value in [('id', 'srv-unrelated'), ('ownerId', 'tea-unrelated'),
                           ('repo', 'https://github.com/other/private'), ('autoDeploy', 'yes'),
                           ('serviceDetails', {'plan': 'standard', 'region': 'oregon'})]:
            with self.subTest(key=key), patch.object(controller, 'api', return_value={**base, key: value}):
                with self.assertRaises(RuntimeError):
                    controller.service('api')

    def test_environment_updates_only_one_scoped_key(self):
        with patch.object(controller, 'api', side_effect=[None, {'value': 'false'}]) as api:
            controller.set_env('api', 'ENABLE_CREATOR_OPEN', 'false')
            self.assertEqual(api.call_args_list[0].args,
                ('render', '/services/' + controller.SERVICES['api'] + '/env-vars/ENABLE_CREATOR_OPEN',
                 'PUT', {'value': 'false'}))
        with patch.object(controller, 'api', side_effect=[None, {'value': 'true'}]):
            with self.assertRaisesRegex(RuntimeError, 'read back'):
                controller.set_env('api', 'ENABLE_CREATOR_OPEN', 'false')

    def test_deployment_must_acknowledge_exact_revision(self):
        with patch.object(controller, 'api', side_effect=[[], {'id': 'dep-fixture',
                'status': 'created', 'commit': {'id': 'unreviewed'}}]):
            with self.assertRaisesRegex(RuntimeError, 'exact deployment revision'):
                controller.deploy('api')

    def test_queued_deploy_resolves_commit_without_resubmission(self):
        with patch.object(controller, 'api', side_effect=[[],
                {'id': 'dep-fixture', 'status': 'created', 'commit': None},
                {'id': 'dep-fixture', 'status': 'build_in_progress',
                 'commit': {'id': controller.SPENDER_REVISION}}]) as api, \
                patch.object(controller.time, 'sleep'):
            result = controller.deploy('keeper', controller.SPENDER_REVISION)
            self.assertEqual(result['commit'], controller.SPENDER_REVISION)
            self.assertEqual(api.call_count, 3)
            self.assertEqual(api.call_args_list[2].args,
                ('render', '/services/' + controller.SERVICES['keeper'] + '/deploys/dep-fixture'))

    def test_unresolved_deploy_fails_closed_without_resubmission(self):
        with patch.object(controller, 'api', side_effect=[[]] + [{
                'id': 'dep-fixture', 'status': 'created', 'commit': None}] * 21) as api, \
                patch.object(controller.time, 'sleep'):
            with self.assertRaisesRegex(RuntimeError, 'exact deployment revision'):
                controller.deploy('keeper', controller.SPENDER_REVISION)
            self.assertEqual(sum(len(call.args) > 2 and call.args[2] == 'POST'
                                 for call in api.call_args_list), 1)

    def test_empty_accepted_response_reconciles_unique_new_api_deploy(self):
        old = {'deploy': {'id': 'dep-old', 'trigger': 'api'}}
        new = {'deploy': {'id': 'dep-new', 'trigger': 'api', 'status': 'created',
                         'commit': {'id': controller.SPENDER_REVISION}}}
        with patch.object(controller, 'api', side_effect=[[old], None, [new, old]]) as api:
            result = controller.deploy('keeper', controller.SPENDER_REVISION)
            self.assertEqual(result['id'], 'dep-new')
            self.assertEqual(sum(len(call.args) > 2 and call.args[2] == 'POST'
                                 for call in api.call_args_list), 1)

    def restoration_fixture(self):
        return {'services': {role: {'deploys': [{'status': 'live', 'commit':
            controller.ACTIVE_REVISION if role in ('api', 'mcp') else controller.SPENDER_REVISION}]}
            for role in controller.SERVICES}, 'workflows': {w: {'state': 'disabled_manually',
            'active_runs': []} for w in controller.WORKFLOWS}}

    def test_restoration_preserves_holds_and_disables_controller_last(self):
        states = {w: 'disabled_manually' for w in controller.WORKFLOWS}
        states['render-deploy-recovery.yml'] = 'active'
        def provider(provider, path, method='GET', body=None):
            if path == '/git/ref/heads/main':
                return {'object': {'sha': controller.PUBLIC_RESTORATION_REVISION}}
            if provider == 'render':
                return {'value': 'false'}
            parts = path.split('/');workflow = parts[3]
            if method == 'PUT':
                states[workflow] = 'active' if parts[-1] == 'enable' else 'disabled_manually'
                return None
            return {'state': states[workflow]}
        evidence = self.restoration_fixture()
        with patch.object(controller, 'api', side_effect=provider) as api:
            controller.restore_workflows(evidence)
        writes = [c.args for c in api.call_args_list if len(c.args)>2 and c.args[2]=='PUT']
        self.assertEqual(len(writes), 11)
        self.assertEqual(writes[-1][1], '/actions/workflows/render-deploy-recovery.yml/disable')
        self.assertTrue(all(states[w]=='disabled_manually' for w in controller.HELD_WORKFLOWS))
        self.assertEqual(sum(v=='active' for v in evidence['restored_workflows'].values()), 10)

    def test_restoration_rejects_moved_main_before_any_write(self):
        with patch.object(controller, 'api', return_value={'object': {'sha': 'unreviewed'}}) as api:
            with self.assertRaisesRegex(RuntimeError, 'Public main changed'):
                controller.restore_workflows(self.restoration_fixture())
            self.assertEqual(api.call_count, 1)

    def test_restoration_rejects_uncapped_runtime_before_any_write(self):
        evidence = self.restoration_fixture();evidence['services']['keeper']['deploys'][0]['commit'] = 'old'
        with patch.object(controller, 'api', return_value={'object': {'sha': controller.PUBLIC_RESTORATION_REVISION}}) as api:
            with self.assertRaisesRegex(RuntimeError, 'Exact reviewed runtime'):
                controller.restore_workflows(evidence)
            self.assertEqual(api.call_count, 1)


if __name__ == '__main__':
    unittest.main()
