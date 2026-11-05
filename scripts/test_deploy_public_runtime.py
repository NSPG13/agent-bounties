import copy
import datetime as dt
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from deploy_public_runtime import PUBLIC,ReleaseError,prepare,prepare_release,reader_readiness,validate_deploy,public_ci_revision,verify_live_readers,main,validate_release_approval,validate_accepted_reader
class Client:
    def __init__(self,services):self.services=copy.deepcopy(services);self.calls=[]
    def request(self,method,path,body=None):
        self.calls.append((method,path,body));value=self.services[path.split('/')[-1]]
        if method=='PATCH':value.update(repo=body['repo'],branch=body['branch'],autoDeploy='no')
        return value
class PublicReleaseTests(unittest.TestCase):
    def setUp(self):
        self.expected={'id':'srv-one','name':'api','type':'web_service'}
        self.manifest={'repository':PUBLIC,'branch':'main','migration_from_repository':'https://github.com/NSPG13/AB-PRIVATE','workspace_id':'tea-reviewed','services':[self.expected],'blueprints':[]}
        self.service={**self.expected,'ownerId':'tea-reviewed','repo':self.manifest['migration_from_repository'],'branch':'reviewed-private','autoDeploy':'no'}
    def test_manual_approval_is_exact_and_separate_from_live_expectation(self):
        validate_release_approval('a'*40,'b'*40,'a'*40,'workflow_dispatch')
        validate_release_approval('a'*40,'a'*40,'','workflow_run')
        validate_release_approval('a'*40,'','','workflow_run')
        for expected,approved,event in [('b'*40,'','workflow_dispatch'),
                ('b'*40,'c'*40,'workflow_dispatch'),('a'*40,'a'*40,'workflow_run'),
                ('a'*40,'a'*7,'workflow_dispatch')]:
            with self.subTest(expected=expected,approved=approved,event=event),self.assertRaises(ReleaseError):
                validate_release_approval('a'*40,expected,approved,event)
    def test_private_source_requires_explicit_migration(self):
        client=Client({'srv-one':self.service})
        with self.assertRaises(ReleaseError):prepare(client,self.manifest,False)
        self.assertTrue(all(call[0]=='GET' for call in client.calls))
    def test_migration_only_changes_source_and_disables_autodeploy(self):
        client=Client({'srv-one':self.service});prepare(client,self.manifest,True)
        writes=[call for call in client.calls if call[0]!='GET']
        self.assertEqual(writes,[('PATCH','services/srv-one',{'repo':PUBLIC,'branch':'main','autoDeployTrigger':'off'})])
        self.assertEqual(client.calls[-1][0],'GET')
    def test_wrong_identity_or_workspace_never_mutates(self):
        for field in ['id','name','type','ownerId','repo']:
            service={**self.service,field:'other'};client=Client({'srv-one':service})
            with self.subTest(field=field),self.assertRaises(ReleaseError):prepare(client,self.manifest,True)
            self.assertTrue(all(call[0]=='GET' for call in client.calls))
    def test_all_services_validated_before_first_mutation(self):
        manifest=copy.deepcopy(self.manifest);manifest['services'].append({**self.expected,'id':'srv-two'})
        client=Client({'srv-one':self.service,'srv-two':{**self.service,'id':'srv-two','ownerId':'wrong'}})
        with self.assertRaises(ReleaseError):prepare(client,manifest,True)
        self.assertTrue(all(call[0]=='GET' for call in client.calls))
    def test_blueprint_drift_blocks_all_source_mutations(self):
        expected={'id':'exs-reviewed','name':'marketplace','path':'render.yaml',
            'resource_ids':['srv-one','dpg-database','evg-shared']}
        manifest={**self.manifest,'blueprints':[expected]}
        blueprint={key:expected[key] for key in ('id','name','path')}
        blueprint.update(ownerId='tea-reviewed',repo=PUBLIC,branch='main',autoSync=False,
            status='paused',resources=[{'id':value} for value in expected['resource_ids']])
        for mutation in ('id','name','path','ownerId','repo','branch','autoSync','status',
                'missing_sync','string_false','missing_resource','extra_resource','duplicate_resource'):
            value=copy.deepcopy(blueprint)
            if mutation=='missing_sync':del value['autoSync']
            elif mutation=='string_false':value['autoSync']='false'
            elif mutation=='missing_resource':value['resources'].pop()
            elif mutation=='extra_resource':value['resources'].append({'id':'srv-unreviewed'})
            elif mutation=='duplicate_resource':value['resources'][-1]=value['resources'][0]
            else:value[mutation]=True if mutation=='autoSync' else 'other'
            client=Client({'srv-one':self.service,'exs-reviewed':value})
            with self.subTest(mutation=mutation),self.assertRaises(ReleaseError):
                prepare(client,manifest,True)
            self.assertTrue(all(call[0]=='GET' for call in client.calls))
        client=Client({'srv-one':self.service,'exs-reviewed':blueprint})
        prepare(client,manifest,True)
        self.assertIn(('GET','blueprints/exs-reviewed',None),client.calls)
        self.assertEqual([call[1] for call in client.calls if call[0]!='GET'],['services/srv-one'])
    def test_missing_blueprint_inventory_cannot_silently_skip_control(self):
        manifest=copy.deepcopy(self.manifest);del manifest['blueprints']
        client=Client({'srv-one':self.service})
        with self.assertRaises(ReleaseError):prepare(client,manifest,True)
        self.assertTrue(all(call[0]=='GET' for call in client.calls))
    def test_production_inventory_keeps_the_reviewed_blueprint_dependency(self):
        manifest=json.loads((Path(__file__).resolve().parents[1]/'ops/public-runtime-services.json').read_text())
        self.assertEqual([row['id'] for row in manifest['blueprints']],['exs-d97too57vvec73ctvmh0'])
        self.assertIn(manifest['autonomous_bootstrap_service_id'],manifest['blueprints'][0]['resource_ids'])
    def bootstrap_fixture(self):
        manifest=copy.deepcopy(self.manifest)
        reader={'id':'srv-reader','name':'reader','type':'background_worker','dockerfile':'./Dockerfile.posting-indexers'}
        manifest['services'].append(reader)
        manifest['autonomous_bootstrap_service_id']=reader['id']
        service={**self.service,**reader,'serviceDetails':{'envSpecificDetails':{
            'dockerfilePath':reader['dockerfile'],'dockerContext':'.','dockerCommand':''}}}
        return manifest,Client({'srv-one':self.service,'srv-reader':service})
    def test_bootstrap_only_migrates_and_selects_the_reviewed_reader(self):
        manifest,client=self.bootstrap_fixture()
        selected=prepare(client,manifest,True,bootstrap=True)
        self.assertEqual([expected['id'] for expected,_ in selected],['srv-reader'])
        self.assertEqual([call for call in client.calls if call[0]!='GET'],[
            ('PATCH','services/srv-reader',{'repo':PUBLIC,'branch':'main','autoDeployTrigger':'off'})])
        self.assertEqual(client.services['srv-one']['repo'],self.manifest['migration_from_repository'])
    def test_bootstrap_cannot_hide_an_invalid_unselected_service(self):
        manifest,client=self.bootstrap_fixture()
        client.services['srv-one']['ownerId']='wrong'
        with self.assertRaises(ReleaseError):prepare(client,manifest,True,bootstrap=True)
        self.assertTrue(all(call[0]=='GET' for call in client.calls))
    def test_bootstrap_requires_explicit_migration_and_exact_reader_inventory(self):
        for mutation in ('implicit','unknown','web','duplicate'):
            manifest,client=self.bootstrap_fixture()
            if mutation=='unknown':manifest['autonomous_bootstrap_service_id']='srv-other'
            elif mutation=='web':manifest['autonomous_bootstrap_service_id']='srv-one'
            elif mutation=='duplicate':manifest['services'].append(manifest['services'][-1])
            with self.subTest(mutation=mutation),self.assertRaises(ReleaseError):
                prepare(client,manifest,mutation!='implicit',bootstrap=True)
            self.assertEqual(client.calls,[])
    def test_accepted_reader_must_be_newest_live_exact_artifact_with_original_public_binding(self):
        manifest,client=self.bootstrap_fixture()
        expected=manifest['services'][-1]
        previous={**client.services['srv-reader'],'repo':PUBLIC,'branch':'main'}
        deploy={'id':'dep-accepted','status':'live','commit':{'id':'a'*40}}
        validate_accepted_reader(previous,expected,manifest,[{'deploy':deploy}],deploy['id'],'a'*40)
        for mutation in ('source','branch','recipe','empty','hash','id','pending','historical'):
            service=copy.deepcopy(previous);deploys=[copy.deepcopy(deploy)]
            if mutation=='source':service['repo']=manifest['migration_from_repository']
            elif mutation=='branch':service['branch']='other'
            elif mutation=='recipe':service['serviceDetails']['envSpecificDetails']['dockerfilePath']='./other'
            elif mutation=='empty':deploys=[]
            elif mutation=='hash':deploys[0]['commit']['id']='b'*40
            elif mutation=='id':deploys[0]['id']='dep-other'
            elif mutation=='pending':deploys[0]['status']='build_in_progress'
            else:deploys.insert(0,{'id':'dep-newer','status':'build_in_progress'})
            with self.subTest(mutation=mutation),self.assertRaises(ReleaseError):
                validate_accepted_reader(service,expected,manifest,deploys,deploy['id'],'a'*40)
    def test_bootstrap_deploys_only_reader_and_does_not_claim_release_acceptance(self):
        manifest,client=self.bootstrap_fixture()
        revision='a'*40
        metadata_request=client.request
        deployments=[]
        def request(method,path,body=None):
            if '/deploys' not in path:return metadata_request(method,path,body)
            deployments.append((method,path,body))
            if method=='POST':return {'id':'dep-bootstrap','status':'build_in_progress'}
            if path.endswith('limit=20'):return []
            return {'id':'dep-bootstrap','status':'live','commit':{'id':revision}}
        client.request=request
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'ops').mkdir()
            (root/'ops/public-runtime-services.json').write_text(json.dumps(manifest))
            evidence=root/'evidence'
            arguments=['release','--revision',revision,'--migrate-source','--bootstrap-autonomous-readers','--evidence-dir',str(evidence)]
            with patch('deploy_public_runtime.ROOT',root),patch('sys.argv',arguments), \
                    patch.dict(os.environ,{'PRODUCTION_EXPECTED_REVISION':'b'*40,'APPROVED_REVISION':revision,'RELEASE_EVENT':'workflow_dispatch'}), \
                    patch('deploy_public_runtime.public_ci_revision') as ci, \
                    patch('deploy_public_runtime.Render',return_value=client), \
                    patch('deploy_public_runtime.reader_readiness',return_value={}), \
                    patch('deploy_public_runtime.verify_live_readers',side_effect=AssertionError('not a full release')), \
                    patch('deploy_public_runtime.time.sleep'):
                main()
            ci.assert_called_once_with(revision)
            record=json.loads((evidence/'release.json').read_text())
            self.assertEqual(record['status'],'bootstrap_deployed_pending_acceptance')
            self.assertEqual(record['phase'],'autonomous-bootstrap')
            self.assertEqual([row['id'] for row in record['services']],['srv-reader'])
            self.assertNotIn('marketplace_readiness',record)
            self.assertEqual([call for call in deployments if call[0]=='POST'],[
                ('POST','services/srv-reader/deploys',{'commitId':revision,'clearCache':'do_not_clear'})])
    def test_bootstrap_rejects_wrong_commit_in_immediate_live_post_response(self):
        manifest,client=self.bootstrap_fixture();revision='a'*40
        metadata_request=client.request
        def request(method,path,body=None):
            if '/deploys' not in path:return metadata_request(method,path,body)
            if method=='POST':return {'id':'dep-wrong','status':'live','commit':{'id':'b'*40}}
            if path.endswith('limit=20'):return []
            self.fail('wrong immediate live result must fail before polling')
        client.request=request
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'ops').mkdir()
            (root/'ops/public-runtime-services.json').write_text(json.dumps(manifest))
            evidence=root/'evidence'
            arguments=['release','--revision',revision,'--migrate-source','--bootstrap-autonomous-readers','--evidence-dir',str(evidence)]
            with patch('deploy_public_runtime.ROOT',root),patch('sys.argv',arguments), \
                    patch.dict(os.environ,{'PRODUCTION_EXPECTED_REVISION':'b'*40,'APPROVED_REVISION':revision,'RELEASE_EVENT':'workflow_dispatch'}), \
                    patch('deploy_public_runtime.public_ci_revision'), \
                    patch('deploy_public_runtime.Render',return_value=client), \
                    patch('deploy_public_runtime.reader_readiness',return_value={}),self.assertRaises(ReleaseError):
                main()
            self.assertEqual(json.loads((evidence/'release.json').read_text())['status'],'failed')
    def exercise_full_promotion(self, reader_poll_id):
        manifest,client=self.bootstrap_fixture();revision='a'*40
        client.services['srv-reader'].update(repo=PUBLIC,branch='main')
        metadata_request=client.request;deployments=[]
        def request(method,path,body=None):
            if '/deploys' not in path:return metadata_request(method,path,body)
            deployments.append((method,path,body))
            if method=='POST':return {'id':'dep-api','status':'build_in_progress'}
            if path.endswith('limit=20'):
                return [{'deploy':{'id':'dep-accepted','status':'live','commit':{'id':revision}}}] if 'srv-reader' in path else []
            return {'id':reader_poll_id if 'srv-reader' in path else path.split('/')[-1],
                    'status':'live','commit':{'id':revision}}
        client.request=request
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'ops').mkdir()
            (root/'ops/public-runtime-services.json').write_text(json.dumps(manifest))
            evidence=root/'evidence'
            arguments=['release','--revision',revision,'--migrate-source','--accepted-autonomous-deploy-id','dep-accepted','--evidence-dir',str(evidence)]
            with patch('deploy_public_runtime.ROOT',root),patch('sys.argv',arguments), \
                    patch.dict(os.environ,{'PRODUCTION_EXPECTED_REVISION':'b'*40,'APPROVED_REVISION':revision,'RELEASE_EVENT':'workflow_dispatch'}), \
                    patch('deploy_public_runtime.public_ci_revision'), \
                    patch('deploy_public_runtime.Render',return_value=client), \
                    patch('deploy_public_runtime.reader_readiness',return_value={}), \
                    patch('deploy_public_runtime.verify_live_readers',return_value={'revision':revision}) as readiness, \
                    patch('deploy_public_runtime.time.sleep'):
                if reader_poll_id=='dep-accepted':main()
                else:
                    with self.assertRaises(ReleaseError):main()
            record=json.loads((evidence/'release.json').read_text())
            if reader_poll_id!='dep-accepted':
                self.assertEqual(record['status'],'failed')
                readiness.assert_not_called()
                return
            self.assertEqual(record['status'],'passed')
            self.assertEqual([call for call in deployments if call[0]=='POST'],[
                ('POST','services/srv-one/deploys',{'commitId':revision,'clearCache':'do_not_clear'})])
            self.assertTrue(record['services'][1]['reused_accepted_deployment'])
            readiness.assert_called_once_with(manifest,revision)
    def test_full_promotion_reuses_only_the_bound_accepted_reader(self):
        for reader_poll_id in ('dep-accepted','dep-other',None):
            with self.subTest(reader_poll_id=reader_poll_id):self.exercise_full_promotion(reader_poll_id)
    def test_deployment_detail_response_must_identify_the_requested_artifact(self):
        for status in ('live','build_in_progress'):
            for response_id in ('dep-other',None):
                with self.subTest(status=status,response_id=response_id),self.assertRaises(ReleaseError):
                    validate_deploy({'id':response_id,'status':status,'commit':{'id':'a'*40}},'a'*40,'dep-requested')
    def test_only_exact_live_revision_is_success(self):
        self.assertTrue(validate_deploy({'status':'live','commit':{'id':'a'*40}},'a'*40))
        self.assertFalse(validate_deploy({'status':'build_in_progress'},'a'*40))
        for deploy in [{'status':'build_failed'},{'status':'live','commit':{'id':'b'*40}}]:
            with self.assertRaises(ReleaseError):validate_deploy(deploy,'a'*40)
    def test_unreviewed_build_recipe_is_rejected_before_migration(self):
        manifest=copy.deepcopy(self.manifest);manifest['services'][0]['dockerfile']='./Dockerfile'
        service={**self.service,'serviceDetails':{'envSpecificDetails':{'dockerfilePath':'./private.Dockerfile','dockerContext':'.','dockerCommand':''}}}
        client=Client({'srv-one':service})
        with self.assertRaises(ReleaseError):prepare(client,manifest,True)
        self.assertTrue(all(call[0]=='GET' for call in client.calls))
    def test_green_ci_does_not_bypass_pending_or_failed_container_checks(self):
        revision='a'*40
        def response(conclusion):
            return io.StringIO(json.dumps({'workflow_runs':[{'id':1,'head_sha':revision,'head_branch':'main','event':'push','conclusion':conclusion}]}))
        for conclusion in [None,'failure']:
            with self.subTest(conclusion=conclusion),patch('deploy_public_runtime.subprocess.check_output',return_value=revision),patch('deploy_public_runtime.subprocess.run'),patch('deploy_public_runtime.urllib.request.urlopen',side_effect=[response('success'),response(conclusion)]):
                with self.assertRaises(ReleaseError):public_ci_revision(revision)

    def readiness_fixture(self):
        manifest = {**self.manifest, 'marketplace_readiness': {
            'url': 'https://api.agentizable.ai/readiness/marketplace',
            'protocols': ['v1', 'v2'],
        }}
        value = {'schema': 'agent-bounties/marketplace-readiness-v1', 'ready': True,
                 'checked_at': dt.datetime.now(dt.timezone.utc).isoformat(),
                 'readers': [{'protocol': protocol, 'ready': True, 'blocker': None}
                             for protocol in ['v1', 'v2']]}
        return manifest, value

    def readiness_response(self, value, revision='a'*40):
        response = io.StringIO(json.dumps(value))
        response.status = 200
        response.headers = {'x-agent-bounties-revision': revision}
        return response

    def test_ready_summary_does_not_hide_missing_duplicate_failed_or_stale_readers(self):
        for mutation in ('failed', 'missing', 'duplicate', 'stale', 'future', 'schema', 'ready'):
            manifest, value = self.readiness_fixture()
            if mutation == 'failed':value['readers'][0]['blocker'] = 'reader_failed'
            elif mutation == 'missing':value['readers'].pop()
            elif mutation == 'duplicate':value['readers'][1] = value['readers'][0]
            elif mutation in ('stale', 'future'):
                value['checked_at'] = (dt.datetime.now(dt.timezone.utc) + dt.timedelta(
                    minutes=-2 if mutation == 'stale' else 2)).isoformat()
            elif mutation == 'schema':value['schema'] = 'other'
            else:value['ready'] = False
            with self.subTest(mutation=mutation), patch('deploy_public_runtime.urllib.request.urlopen',
                    return_value=self.readiness_response(value)), self.assertRaises(ReleaseError):
                reader_readiness(manifest)

    def test_readiness_requires_the_exact_deployed_revision(self):
        manifest, value = self.readiness_fixture()
        with patch('deploy_public_runtime.urllib.request.urlopen', return_value=self.readiness_response(value)):
            self.assertEqual(reader_readiness(manifest, 'a'*40)['revision'], 'a'*40)
        with patch('deploy_public_runtime.urllib.request.urlopen', return_value=self.readiness_response(value)), self.assertRaises(ReleaseError):
            reader_readiness(manifest, 'b'*40)

    def test_legacy_header_fallback_is_explicit_preflight_only(self):
        manifest,value=self.readiness_fixture()
        responses=[self.readiness_response({}),self.readiness_response(value,revision=''),self.readiness_response({})]
        with patch('deploy_public_runtime.urllib.request.urlopen',side_effect=responses) as request:
            result=reader_readiness(manifest,allow_legacy_revision=True)
            self.assertEqual(result['revision'],'a'*40)
            self.assertEqual(result['revision_source'],'legacy_health_bracket')
            self.assertEqual([call.args[0].full_url for call in request.call_args_list],[
                'https://api.agentizable.ai/health',manifest['marketplace_readiness']['url'],
                'https://api.agentizable.ai/health'])
        for revision,allow in [(None,False),('a'*40,False),('a'*40,True)]:
            with self.subTest(revision=revision,allow=allow), \
                    patch('deploy_public_runtime.urllib.request.urlopen',return_value=self.readiness_response(value,revision='')) as request, \
                    self.assertRaises(ReleaseError):
                reader_readiness(manifest,revision,allow_legacy_revision=allow)
            self.assertEqual(request.call_count,1)

    def test_legacy_bracket_cannot_hide_revision_or_reader_failures(self):
        for failure in ('missing_before','missing_after','changed','bad_health_status','bad_ready_status',
                        'malformed_header','failed','missing','stale','future','malformed_json'):
            manifest,value=self.readiness_fixture()
            before=self.readiness_response({});after=self.readiness_response({})
            ready=self.readiness_response(value,revision='')
            if failure=='missing_before':before.headers={}
            elif failure=='missing_after':after.headers={}
            elif failure=='changed':after.headers['x-agent-bounties-revision']='b'*40
            elif failure=='bad_health_status':before.status=503
            elif failure=='bad_ready_status':ready.status=503
            elif failure=='malformed_header':ready.headers['x-agent-bounties-revision']='invalid'
            elif failure=='malformed_json':ready=io.StringIO('{');ready.status=200;ready.headers={}
            else:
                if failure=='failed':value['readers'][0]['ready']=False
                elif failure=='missing':value['readers'].pop()
                else:value['checked_at']=(dt.datetime.now(dt.timezone.utc)+dt.timedelta(minutes=-2 if failure=='stale' else 2)).isoformat()
                ready=self.readiness_response(value,revision='')
            with self.subTest(failure=failure),patch('deploy_public_runtime.urllib.request.urlopen',side_effect=[before,ready,after]),self.assertRaises(ReleaseError):
                reader_readiness(manifest,allow_legacy_revision=True)

    def test_only_explicit_migration_enables_legacy_preflight(self):
        for migrate in (False,True):
            client=Client({'srv-one':{**self.service,'repo':PUBLIC,'branch':'main'}})
            record={}
            with patch('deploy_public_runtime.reader_readiness',return_value={'revision':'a'*40}) as observe:
                prepare_release(client,self.manifest,migrate,preflight_record=record)
            observe.assert_called_once_with(self.manifest,allow_legacy_revision=migrate)
            self.assertEqual(record,{'revision':'a'*40})

    def test_reader_incident_blocks_all_source_changes_and_deploys(self):
        client = Client({'srv-one': self.service})
        with patch('deploy_public_runtime.reader_readiness', side_effect=ReleaseError('reader halted')), self.assertRaises(ReleaseError):
            prepare_release(client, self.manifest, True)
        self.assertEqual(client.calls, [])

    def test_reader_recovery_requires_eight_consecutive_observations(self):
        sequence = [{}]*7 + [ReleaseError('stale')] + [{}]*8
        with patch('deploy_public_runtime.reader_readiness', side_effect=sequence) as observe, \
                patch('deploy_public_runtime.time.sleep'), patch('deploy_public_runtime.time.monotonic', return_value=0):
            verify_live_readers(self.manifest, 'a'*40)
        self.assertEqual(observe.call_count, 16)

    def test_reader_recovery_is_bounded(self):
        with patch('deploy_public_runtime.reader_readiness', side_effect=ReleaseError('halted')), \
                patch('deploy_public_runtime.time.monotonic', side_effect=[0, 300]), self.assertRaises(ReleaseError):
            verify_live_readers(self.manifest, 'a'*40)
if __name__=='__main__':unittest.main()
