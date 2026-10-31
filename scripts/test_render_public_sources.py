import unittest
from check_render_public_sources import audit
from deploy_public_runtime import PUBLIC, ReleaseError


class ObserverTests(unittest.TestCase):
    def test_workers_must_have_public_source_and_live_revision_without_mutations(self):
        expected = {'id': 'srv-worker', 'name': 'worker', 'type': 'background_worker'}
        manifest = {'workspace_id': 'tea-one', 'services': [expected]}
        service = {**expected, 'ownerId': 'tea-one', 'repo': PUBLIC, 'branch': 'main', 'autoDeploy': 'no'}
        deploys = [{'deploy': {'status': 'live', 'commit': {'id': 'a' * 40}}}]
        calls = []

        class Client:
            def request(self, method, path):
                calls.append(method)
                return deploys if '/deploys?' in path else service

        self.assertEqual(audit(Client(), manifest, lambda _: True)[0]['revision'], 'a' * 40)
        with self.assertRaises(ReleaseError):
            audit(Client(), manifest, lambda _: False)
        service['repo'] = 'https://github.com/NSPG13/AB-PRIVATE'
        with self.assertRaises(ReleaseError):
            audit(Client(), manifest, lambda _: True)
        service['repo'] = PUBLIC
        deploys.clear()
        with self.assertRaises(ReleaseError):
            audit(Client(), manifest, lambda _: True)
        self.assertTrue(all(method == 'GET' for method in calls))


if __name__ == '__main__':
    unittest.main()
