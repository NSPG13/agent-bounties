import copy
import unittest
from check_keeper_workflows import violations

class KeeperWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.doc={'jobs':{'sign':{'environment':'legacy-keeper','if':"github.repository == 'NSPG13/agent-bounties' && github.ref == 'refs/heads/main' && github.event_name != 'pull_request'",'steps':[{'uses':'actions/checkout@'+'a'*40,'with':{'persist-credentials':False}},{'run':'python reviewed_signer.py','env':{'KEY':'${{ secrets.BASE_KEEPER_PRIVATE_KEY }}'}}]}}}
    def test_protected_job(self):self.assertEqual(violations(self.doc),[])
    def test_lowercase_reusable_signer_key_requires_execution_gate(self):
        doc=copy.deepcopy(self.doc)
        doc['jobs']['sign']['steps'][1]['env']['KEY']='${{ secrets.verifier_private_key }}'
        del doc['jobs']['sign']['environment']
        self.assertTrue(violations(doc))
    def test_reusable_caller_uses_only_reviewed_local_execution(self):
        doc=copy.deepcopy(self.doc)
        job=doc['jobs']['sign']
        del job['environment'];del job['steps']
        job['secrets']={'key':'${{ secrets.BASE_KEEPER_PRIVATE_KEY }}'}
        job['uses']='./.github/workflows/regression-verifier-signing-reusable.yml'
        self.assertEqual(violations(doc),[])
        job['uses']='attacker/workflow@main'
        self.assertTrue(violations(doc))
    def test_removed_security_boundaries(self):
        for field in ['environment','if']:
            doc=copy.deepcopy(self.doc);del doc['jobs']['sign'][field]
            with self.subTest(field=field):self.assertTrue(violations(doc))
    def test_key_exposed_to_actions_or_job_rejected(self):
        doc=copy.deepcopy(self.doc);doc['jobs']['sign']['env']={'KEY':'${{ secrets.BASE_KEEPER_PRIVATE_KEY }}'}
        self.assertTrue(violations(doc))
        doc=copy.deepcopy(self.doc);doc['jobs']['sign']['steps'][1]['uses']='thirdparty/action@v1'
        self.assertTrue(violations(doc))
    def test_tag_pins_and_persisted_tokens_rejected(self):
        for field,value in [('uses','actions/checkout@v4'),('with',{'persist-credentials':True})]:
            doc=copy.deepcopy(self.doc);doc['jobs']['sign']['steps'][0][field]=value
            with self.subTest(field=field):self.assertTrue(violations(doc))
if __name__=='__main__':unittest.main()
