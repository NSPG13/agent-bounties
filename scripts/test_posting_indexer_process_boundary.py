"""Real child-process checks for the prepared existing-worker supervisor."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ProductionIndexerBoundary(unittest.TestCase):
    def run_supervisor(self, enabled=None, service='srv-d982ftu7r5hc73ar4st0', manifest=True, v2=None):
        with tempfile.TemporaryDirectory(prefix='posting-indexer-boundary-') as folder:
            root = Path(folder)
            child = root / 'indexer'
            child.write_text('#!/usr/bin/python3\nimport os,json,time\nfrom pathlib import Path\n'
                + 'root=Path(' + repr(folder) + ')\n'
                + '(root/(os.environ["BASE_INDEXER_PROTOCOL"]+".json")).write_text(json.dumps(dict(os.environ)))\n'
                + 'time.sleep(0.5)\n')
            child.chmod(0o700)
            script = (ROOT/'scripts/run-posting-indexers.sh').read_text().replace('/usr/local/bin/posting-indexer', str(child))
            supervisor = root/'run.sh'
            supervisor.write_text(script)
            env = {'PATH': os.environ['PATH'], 'RENDER_SERVICE_ID': service,
                'DATABASE_URL':'postgres://fixture-only', 'BASE_MAINNET_RPC_URL':'https://mainnet.base.org',
                'BASE_INDEXER_PROTOCOL':'open-competition-v2-keeper', 'BASE_INDEXER_START_BLOCK':'42',
                'BASE_INDEXER_MAX_BLOCKS_PER_QUERY':'1000', 'DISCOVERY_WEBHOOK_SIGNING_KEY':'existing-discovery-only',
                'X402_RELAYER_PRIVATE_KEY':'must-not-inherit', 'GAS_SPONSOR_BUDGET_TOKEN':'must-not-inherit',
                'GITHUB_APP_PRIVATE_KEY':'must-not-inherit', 'RESEND_API_KEY':'must-not-inherit',
                'VERIFIER_EMAIL_ENABLED':'true', 'CREATOR_PUSH_ENABLED':'true'}
            if enabled is not None: env['POSTING_CREATOR_INDEXER_ENABLED'] = enabled
            if manifest: env['CREATOR_OPEN_RELEASE_JSON'] = '{"network":"base-mainnet"}'
            if v2 is not None: env['POSTING_V2_INDEXER_ENABLED'] = v2
            env['BASE_INDEXER_FACTORY_CONTRACT'] = '0x082c52131aaf0c56e76b075f895eab6fcab6d2f9'
            result = subprocess.run(['bash', str(supervisor)], env=env, capture_output=True, timeout=5)
            self.assertNotEqual(result.returncode, 0, 'A child exit must stop the container')
            return {p.stem:json.loads(p.read_text()) for p in root.glob('*.json')}

    def test_default_keeps_creator_indexer_disabled(self):
        children = self.run_supervisor()
        self.assertEqual(set(children), {'autonomous-v1'})
        self.assertEqual(children['autonomous-v1']['BASE_INDEXER_START_BLOCK'], '42')

    def test_only_readonly_configuration_reaches_creator_child(self):
        children = self.run_supervisor('true')
        self.assertEqual(set(children), {'autonomous-v1','creator-open-v1'})
        for env in children.values():
            for key in ('X402_RELAYER_PRIVATE_KEY','GAS_SPONSOR_BUDGET_TOKEN','GITHUB_APP_PRIVATE_KEY',
                        'RESEND_API_KEY','VERIFIER_EMAIL_ENABLED','CREATOR_PUSH_ENABLED'):
                self.assertNotIn(key, env)
        self.assertNotIn('DISCOVERY_WEBHOOK_SIGNING_KEY', children['creator-open-v1'])
        self.assertEqual(children['autonomous-v1']['DISCOVERY_WEBHOOK_SIGNING_KEY'], 'existing-discovery-only')
        self.assertEqual(children['creator-open-v1']['DATABASE_URL'], children['autonomous-v1']['DATABASE_URL'])

    def test_v2_child_indexes_only_the_pinned_factory_without_credentials(self):
        children = self.run_supervisor(v2='true')
        self.assertEqual(set(children), {'autonomous-v1', 'autonomous-v2'})
        v2 = children['autonomous-v2']
        self.assertEqual(v2['BASE_INDEXER_FACTORY_CONTRACT'], '0xc33e2ae33bb9580837ea59df18e57fa1039ae58a')
        self.assertEqual(v2['BASE_INDEXER_START_BLOCK'], '52275492')
        self.assertEqual(v2['BASE_INDEXER_NETWORK'], 'base-mainnet')
        self.assertEqual(v2['BASE_INDEXER_RPC_URL'], 'https://mainnet.base.org')
        for key in ('X402_RELAYER_PRIVATE_KEY', 'GAS_SPONSOR_BUDGET_TOKEN', 'GITHUB_APP_PRIVATE_KEY',
                    'RESEND_API_KEY', 'DISCOVERY_WEBHOOK_SIGNING_KEY', 'BASE_INDEXER_MAX_BLOCKS_PER_QUERY'):
            self.assertNotIn(key, v2)
        self.assertEqual(children['autonomous-v1']['BASE_INDEXER_FACTORY_CONTRACT'],
                         '0x082c52131aaf0c56e76b075f895eab6fcab6d2f9')

    def test_invalid_scope_or_missing_release_starts_no_children(self):
        for options in ({'service':'another-service'}, {'enabled':'yes'}, {'enabled':'true','manifest':False},
                        {'v2':'yes'}):
            with self.subTest(options=options):
                self.assertEqual(self.run_supervisor(**options), {})


if __name__ == '__main__':
    unittest.main()
