import test from 'node:test';
import assert from 'node:assert/strict';
import {attestRoutedV3, ROUTED_V3, documentHash, routedV3Candidate} from '../skills/agent-bounties/scripts/routed-inventory.mjs';
import {keccak256Hex} from '../skills/agent-bounties/scripts/check-in.mjs';
const NOW=Date.parse('2026-10-01T07:00:00Z');
const word=n=>`0x${BigInt(n).toString(16).padStart(64,'0')}`;
const addr=a=>`0x${a.slice(2).padStart(64,'0')}`;
function fixture() {
  const r={...ROUTED_V3};
  for(const role of ['router','adapter','participant_registry','terms_registry'])r[`${role}_code_hash`]=keccak256Hex('0x6000');
  const document={acceptance_criteria:['Exact fixture criterion'],verification_policy:{mechanism:'deterministic_module',verifier_module:r.router,threshold:1},benchmark:{engine:'standing_meta_v3_routed_parent',minimum_child_target:1000000,minimum_parent_gross_margin:1000000,required_child_engine:'sandboxed_regression_v1',required_child_status:'settled',required_child_verifier_set_hash:r.child_quorum_hash,required_child_verifier_threshold:r.child_threshold,participant_registry:r.participant_registry,terms_registry:r.terms_registry},evidence_schema:{required:['child_bounty_contract']}};
  r.policy_hash=documentHash(document.verification_policy);r.acceptance_hash=documentHash(document.acceptance_criteria);
  const item={bounty_id:word(99),bounty_contract:`0x${'44'.repeat(20)}`,creator:`0x${'55'.repeat(20)}`,status:'claimable',terms_valid:true,verification_ready:true,verification_mode:'deterministic_module',verifier_module:r.router,funded_amount:'3010000',target_amount:'3010000',solver_reward:'3000000',verifier_reward:'10000',claim_bond:'10000',terms:{document},terms_hash:documentHash(document),verification_details:{ready:true,operational_ready:true,release_id:`sha256:${'a'.repeat(64)}`,profile_registry_digest:`sha256:${'b'.repeat(64)}`,last_check_at:new Date(NOW).toISOString(),evaluated_at:new Date(NOW).toISOString()}};
  const block={number:'0x100',hash:word(500),timestamp:`0x${(NOW/1000-30).toString(16)}`};
  const values={v3_chain:'0x2105',v3_safe:block,v3_recheck:block,active:word(1),canonical:word(1),policy:addr(r.adapter)+r.adapter_code_hash.slice(2)+word(1).slice(2)+word(2).slice(2)+word(3).slice(2)+word(0).slice(2)};
  for(const role of ['router','adapter','participant_registry','terms_registry']) values[`code_${role}`]='0x6000';
  for(const [k,v] of Object.entries({'verifierRouter()':addr(r.router),'committedPolicyHash()':r.policy_hash,'canonicalFactory()':addr(r.factory),'settlementToken()':addr(r.token),'participantRegistry()':addr(r.participant_registry),'termsRegistry()':addr(r.terms_registry),'taskVerifierSetHash()':r.child_quorum_hash,'taskVerifierThreshold()':word(2)}))values[`adapter_${k}`]=v;
  for(const [k,v] of Object.entries({status:1,verificationMode:0,threshold:1,fundedAmount:item.funded_amount,targetAmount:item.target_amount,solverReward:item.solver_reward,verifierReward:item.verifier_reward,claimBond:item.claim_bond}))values[`parent_${k}`]=word(v);
  for(const [k,v] of Object.entries({bountyId:item.bounty_id,creator:addr(item.creator),factory:addr(r.factory),settlementToken:addr(r.token),verifierModule:addr(r.router),termsHash:item.terms_hash,policyHash:r.policy_hash,acceptanceCriteriaHash:r.acceptance_hash,benchmarkHash:documentHash(document.benchmark),evidenceSchemaHash:documentHash(document.evidence_schema)}))values[`parent_${k}`]=v;
  let readCount=0;
  const transport=async(_,calls)=>new Map(calls.map(call=>{
    readCount++;
    if(call.method==='eth_call'||call.method==='eth_getCode')assert.deepEqual(call.params[1],{blockHash:block.hash,requireCanonical:true});
    assert.ok(Object.hasOwn(values,call.key),call.key);return [call.key,values[call.key]];
  }));
  return {r,item,values,transport,readCount:()=>readCount};
}
test('routed V3 requires funded state and complete code, policy, terms and health proof at one block',async()=>{
 const f=fixture();assert.equal(routedV3Candidate(f.item),true);
 const result=await attestRoutedV3(f.item,'https://rpc.invalid',f.transport,NOW,f.r);
 assert.equal(result.ready,true,result.reason);assert.equal(result.descriptor.schema_version,'agent-bounties/standing-meta-bounty-v3-routed');assert.equal(result.descriptor.observed_block_hash,f.values.v3_safe.hash);
});
test('changed router, adapter, registries, routing, economics, terms or block never count ready',async()=>{
 for(const key of ['code_router','code_adapter','code_participant_registry','code_terms_registry','active','canonical','adapter_taskVerifierThreshold()','adapter_termsRegistry()','adapter_committedPolicyHash()','parent_termsHash','parent_status','parent_fundedAmount','parent_targetAmount','parent_benchmarkHash']){
  const f=fixture();f.values[key]=key.startsWith('code_')?'0x6001':word(0);
  const result=await attestRoutedV3(f.item,'https://rpc.invalid',f.transport,NOW,f.r);assert.equal(result.ready,false,key);
 }
 for(const change of [f=>f.values.v3_safe={...f.values.v3_safe,timestamp:'0x1'},f=>f.values.v3_recheck={...f.values.v3_recheck,hash:word(6)},f=>f.values.policy=f.values.policy.slice(0,-64)+word(1).slice(2),f=>f.values.v3_chain='0x1']){const f=fixture();change(f);assert.equal((await attestRoutedV3(f.item,'https://rpc.invalid',f.transport,NOW,f.r)).ready,false);}
});
test('missing or stale operational health blocks before RPC, even for recognized V3',async()=>{
 for(const change of [f=>delete f.item.verification_details,f=>f.item.verification_details.ready=false,f=>f.item.verification_details.last_check_at=new Date(NOW-2700001).toISOString(),f=>f.item.verification_details.evaluated_at=new Date(NOW-120001).toISOString(),f=>f.item.verification_ready=false]){const f=fixture();change(f);assert.equal(routedV3Candidate(f.item),true);assert.equal((await attestRoutedV3(f.item,'https://rpc.invalid',f.transport,NOW,f.r)).ready,false);assert.equal(f.readCount(),0);}
});
test('altered published terms and unsupported execution settings cannot reuse a known descriptor',async()=>{
 for(const change of [f=>f.item.terms.document.benchmark.minimum_child_target=0,f=>f.item.terms.document.acceptance_criteria.push('New funded term'),f=>f.item.terms.document.evidence_schema.required=[],f=>f.item.funded_amount='0']){const f=fixture();change(f);assert.equal((await attestRoutedV3(f.item,'https://rpc.invalid',f.transport,NOW,f.r)).ready,false);}
});
