import copy
import io
import json
import unittest
from unittest.mock import patch
from deploy_public_runtime import PUBLIC,ReleaseError,prepare,validate_deploy,public_ci_revision
class Client:
    def __init__(self,services):self.services=copy.deepcopy(services);self.calls=[]
    def request(self,method,path,body=None):
        self.calls.append((method,path,body));value=self.services[path.split('/')[-1]]
        if method=='PATCH':value.update(repo=body['repo'],branch=body['branch'],autoDeploy='no')
        return value
class PublicReleaseTests(unittest.TestCase):
    def setUp(self):
        self.expected={'id':'srv-one','name':'api','type':'web_service'}
        self.manifest={'repository':PUBLIC,'branch':'main','migration_from_repository':'https://github.com/NSPG13/AB-PRIVATE','workspace_id':'tea-reviewed','services':[self.expected]}
        self.service={**self.expected,'ownerId':'tea-reviewed','repo':self.manifest['migration_from_repository'],'branch':'reviewed-private','autoDeploy':'no'}
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
if __name__=='__main__':unittest.main()
