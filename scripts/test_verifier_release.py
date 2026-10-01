from __future__ import annotations
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
from scripts import verifier_release as release

FIXTURE=Path(__file__).resolve().parents[1]/'crates/service-runtime/fixtures/verifier-release-test-v1.json'
class ReleaseTests(unittest.TestCase):
    def setUp(self): self.value=json.loads(FIXTURE.read_text())
    def test_identity_matches_rust_and_rejects_replaced_settings(self):
        release.validate_release(self.value)
        for field in ['worker_build_digest','worker_binary_digest','signing_runtime_digest','profile_registry_digest','source_revision','network','signers']:
            changed=copy.deepcopy(self.value);changed[field]='substituted'
            with self.subTest(field=field),self.assertRaises((ValueError,TypeError)):release.validate_release(changed)
        changed=copy.deepcopy(self.value);changed['extra']='uncommitted'
        with self.assertRaises(ValueError):release.validate_release(changed)
    def test_cache_is_verified_every_time_and_symlinks_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'worker';path.write_bytes(b'reviewed worker fixture')
            release.ensure_worker(path,self.value)
            path.write_bytes(b'changed worker')
            with self.assertRaisesRegex(ValueError,'binary differs'):release.ensure_worker(path,self.value)
            target=Path(directory)/'linked';target.symlink_to(path)
            with self.assertRaises(ValueError):release.ensure_worker(target,self.value)
    def test_changed_download_never_becomes_executable_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'worker'
            for data,valid in [(b'substituted worker',False),(b'reviewed worker fixture',True)]:
                stream=io.BytesIO(data)
                with mock.patch.object(release.urllib.request,'build_opener') as opener:
                    opener.return_value.open.return_value=stream
                    if valid:release.ensure_worker(path,self.value,'https://raw.githubusercontent.com/owner/repo/commit/worker')
                    else:
                        with self.assertRaises(ValueError):release.ensure_worker(path,self.value,'https://raw.githubusercontent.com/owner/repo/commit/worker')
                        self.assertFalse(path.exists());self.assertEqual(list(Path(directory).iterdir()),[])
            self.assertEqual(path.read_bytes(),b'reviewed worker fixture')
    def test_health_cannot_send_credentials_to_another_origin_or_component(self):
        for url in ['http://api.agentbounties.app','https://evil.test','https://api.agentbounties.app@evil.test','https://api.agentbounties.app/private?x=y']:
            with self.subTest(url=url),self.assertRaises(ValueError):release.post_checkpoint(url,self.value,'runner',True,'x'*40)
        with self.assertRaises(ValueError):release.post_checkpoint('https://api.agentbounties.app',self.value,'operator',True,'x'*40)
    def test_candidates_remain_valid_only_for_the_same_approved_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);path=root/'release.json'
            with self.assertRaises(ValueError):release.authorize_candidate_release(root,self.value)
            path.write_text(json.dumps(self.value));release.authorize_candidate_release(root,self.value)
            other=copy.deepcopy(self.value);other['worker_binary_digest']='sha256:'+'f'*64
            body={k:v for k,v in other.items() if k!='release_id'}
            other['release_id']='sha256:'+release.hashlib.sha256(json.dumps(body,sort_keys=True,separators=(',',':')).encode()).hexdigest()
            path.write_text(json.dumps(other))
            with self.assertRaisesRegex(ValueError,'different reviewed release'):release.authorize_candidate_release(root,self.value)
    def test_polling_keeps_cadence_and_downloads_verified_release_instead_of_compiling(self):
        root=FIXTURE.parents[3]
        for name in ['regression-verifier-runner.yml','regression-verifier-signer.yml','regression-verifier-signing-reusable.yml']:
            workflow=(root/'.github/workflows'/name).read_text()
            self.assertNotIn('cargo build --release -p worker',workflow)
            self.assertIn('scripts/verifier_release.py fetch',workflow)
        self.assertIn('7,22,37,52 * * * *',(root/'.github/workflows/regression-verifier-runner.yml').read_text())

    def test_component_subprocesses_receive_only_their_own_signer_credential(self):
        with mock.patch.dict(release.os.environ, {'PATH':'/usr/bin','REGRESSION_VERIFIER_PRIVATE_KEY':'fixture-only','REGRESSION_CHECKPOINT_TOKEN':'health-token','BASE_KEEPER_PRIVATE_KEY':'keeper','OPENAI_API_KEY':'provider','AWS_SECRET_ACCESS_KEY':'cloud'},clear=True):
            runner=release.component_environment('runner')
            signer=release.component_environment('signer:'+self.value['signers'][0])
        self.assertEqual(set(runner),{'PATH','LANG'})
        self.assertEqual(set(signer),{'PATH','LANG','REGRESSION_VERIFIER_PRIVATE_KEY'})
        self.assertEqual(signer['REGRESSION_VERIFIER_PRIVATE_KEY'],'fixture-only')
    def test_unverified_worker_never_receives_signer_key(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'worker';path.write_bytes(b'changed')
            with mock.patch.object(release.subprocess,'run') as run, self.assertRaises(ValueError):
                release.diagnose_component(path,self.value,'signer:'+self.value['signers'][0],'https://api.agentbounties.app')
            run.assert_not_called()
    def test_healthy_signer_requires_a_real_diagnostic_and_pending_job_discovery(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'worker';path.write_bytes(b'reviewed worker fixture')
            with mock.patch.dict(release.os.environ,{'REGRESSION_VERIFIER_PRIVATE_KEY':'fixture-only'}), mock.patch.object(release.subprocess,'run') as run, mock.patch.object(release.urllib.request,'build_opener') as opener:
                run.return_value.stdout='{"healthy":true}'
                opener.return_value.open.return_value=io.BytesIO(b'[]')
                component='signer:'+self.value['signers'][0]
                result=release.diagnose_component(path,self.value,component,'https://api.agentbounties.app')
                self.assertTrue(result['healthy']);self.assertEqual(result['release_id'],self.value['release_id'])
                self.assertNotIn('fixture-only',str(run.call_args.args))
                run.return_value.stdout='{"healthy":false}'
                with self.assertRaises(ValueError):release.diagnose_component(path,self.value,component,'https://api.agentbounties.app')

    def test_download_redirects_remain_bounded_to_https_release_hosts(self):
        redirect=release.ReleaseRedirect()
        request=release.urllib.request.Request('https://github.com/owner/repo/releases/download/reviewed/worker')
        result=redirect.redirect_request(request,None,302,'Found',{},'https://release-assets.githubusercontent.com/worker?signature=temporary')
        self.assertEqual(result.host,'release-assets.githubusercontent.com')
        for url in ['https://evil.test/worker','http://github.com/worker','https://github.com:8443/worker','https://user:password@github.com/worker']:
            with self.subTest(url=url),self.assertRaises(ValueError):redirect.redirect_request(request,None,302,'Found',{},url)
        self.assertEqual(redirect.max_redirections,3)
if __name__=='__main__':unittest.main()
