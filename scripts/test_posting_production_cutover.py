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
        with patch.object(controller, 'api', return_value={'id': 'dep-fixture',
                'status': 'created', 'commit': {'id': 'unreviewed'}}):
            with self.assertRaisesRegex(RuntimeError, 'exact deployment revision'):
                controller.deploy('api')


if __name__ == '__main__':
    unittest.main()
