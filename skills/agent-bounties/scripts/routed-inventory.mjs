// Existing routed V3 contracts, bound to reviewed source and runtime bytes.
// All reads use one canonical safe block. Runtime recognition is not readiness.
import { readFileSync } from 'node:fs';
import { keccak256Hex } from './check-in.mjs';
const HASH = /^0x[0-9a-f]{64}$/;
const ADDRESS = /^0x[0-9a-f]{40}$/;
const ZERO = `0x${'0'.repeat(64)}`;
export const ROUTED_V3 = Object.freeze(JSON.parse(readFileSync(new URL('../fixtures/routed-v3-release.json', import.meta.url), 'utf8')));

function requireValue(ok, reason) { if (!ok) throw new Error(reason); }
function word(value) { requireValue(typeof value === 'string' && HASH.test(value.toLowerCase()), 'routed_v3_malformed_abi'); return value.toLowerCase(); }
function addressWord(value) { requireValue(ADDRESS.test(value), 'routed_v3_invalid_address'); return `0x${value.slice(2).padStart(64,'0')}`; }
function canonical(value) {
  if (typeof value === 'number') requireValue(Number.isSafeInteger(value), 'routed_v3_inexact_terms_number');
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(k=>`${JSON.stringify(k)}:${canonical(value[k])}`).join(',')}}`;
  const result=JSON.stringify(value); requireValue(result !== undefined, 'routed_v3_invalid_terms'); return result;
}
export function documentHash(value) { return keccak256Hex(`0x${Buffer.from(canonical(value)).toString('hex')}`); }
function selector(signature) { return keccak256Hex(`0x${Buffer.from(signature).toString('hex')}`).slice(0,10); }
export function routedV3Candidate(item) {
  return item?.terms?.document?.benchmark?.engine === 'standing_meta_v3_routed_parent'
    || String(item?.verifier_module || '').toLowerCase() === ROUTED_V3.router;
}
export async function attestRoutedV3(item, rpc, transport, now = Date.now(), reviewed = ROUTED_V3) {
  try {
    const d=item.terms?.document, b=d?.benchmark, h=item.verification_details;
    requireValue(item.status==='claimable' && item.terms_valid===true && item.verification_ready===true, 'routed_v3_not_claimable');
    const checked=Date.parse(h?.last_check_at), evaluated=Date.parse(h?.evaluated_at);
    requireValue(h?.ready===true && h?.operational_ready===true && /^sha256:[0-9a-f]{64}$/.test(h.release_id || '')
      && /^sha256:[0-9a-f]{64}$/.test(h.profile_registry_digest || '') && Number.isFinite(checked) && Number.isFinite(evaluated)
      && checked<=now+30000 && now-checked<=2700000 && evaluated<=now+30000 && now-evaluated<=120000, 'routed_v3_components_unavailable');
    requireValue(item.verification_mode==='deterministic_module' && item.verifier_module?.toLowerCase()===reviewed.router
      && b?.engine==='standing_meta_v3_routed_parent' && b.minimum_child_target===1000000 && b.minimum_parent_gross_margin===1000000
      && b.required_child_engine==='sandboxed_regression_v1' && b.required_child_status==='settled'
      && b.required_child_verifier_set_hash===reviewed.child_quorum_hash && b.required_child_verifier_threshold===reviewed.child_threshold
      && b.participant_registry===reviewed.participant_registry && b.terms_registry===reviewed.terms_registry
      && documentHash(d)===item.terms_hash && documentHash(d.verification_policy)===reviewed.policy_hash
      && documentHash(d.acceptance_criteria)===reviewed.acceptance_hash, 'routed_v3_terms_mismatch');
    const blocks=await transport(rpc,[{key:'v3_chain',method:'eth_chainId',params:[]},{key:'v3_safe',method:'eth_getBlockByNumber',params:['safe',false]}]);
    requireValue(BigInt(blocks.get('v3_chain'))===8453n,'routed_v3_wrong_chain');
    const block=blocks.get('v3_safe'), number=Number(BigInt(block?.number)), timestamp=Number(BigInt(block?.timestamp));
    requireValue(Number.isSafeInteger(number) && number>0 && HASH.test(block?.hash || '') && Number.isSafeInteger(timestamp)
      && timestamp*1000<=now+30000 && now-timestamp*1000<=2700000,'routed_v3_stale_safe_block');
    const ref={blockHash:block.hash,requireCanonical:true}, calls=[], expected=new Map();
    const add=(key,to,signature,args=[],want=null)=>{calls.push({key,method:'eth_call',params:[{to,data:selector(signature)+args.map(x=>x.slice(2)).join('')},ref]});if(want!==null)expected.set(key,want);};
    for (const role of ['router','adapter','participant_registry','terms_registry']) {
      calls.push({key:`code_${role}`,method:'eth_getCode',params:[reviewed[role],ref]});
    }
    add('active',reviewed.router,'isPolicyActive(bytes32)',[reviewed.policy_hash],`0x${'1'.padStart(64,'0')}`);
    add('policy',reviewed.router,'policies(bytes32)',[reviewed.policy_hash]);
    const adapterFields={'verifierRouter()':addressWord(reviewed.router),'committedPolicyHash()':reviewed.policy_hash,'canonicalFactory()':addressWord(reviewed.factory),'settlementToken()':addressWord(reviewed.token),'participantRegistry()':addressWord(reviewed.participant_registry),'termsRegistry()':addressWord(reviewed.terms_registry),'taskVerifierSetHash()':reviewed.child_quorum_hash,'taskVerifierThreshold()':`0x${reviewed.child_threshold.toString(16).padStart(64,'0')}`};
    for(const [signature,want] of Object.entries(adapterFields)) add(`adapter_${signature}`,reviewed.adapter,signature,[],want);
    const contract=String(item.bounty_contract).toLowerCase();
    add('canonical',reviewed.factory,'isCanonicalBounty(address)',[addressWord(contract)],`0x${'1'.padStart(64,'0')}`);
    const amounts={status:1,verificationMode:0,threshold:1,fundedAmount:item.funded_amount,targetAmount:item.target_amount,solverReward:item.solver_reward,verifierReward:item.verifier_reward,claimBond:item.claim_bond};
    requireValue(BigInt(item.funded_amount)===BigInt(item.target_amount) && BigInt(item.target_amount)>0n,'routed_v3_not_funded');
    for(const [field,value] of Object.entries(amounts)) add(`parent_${field}`,contract,`${field}()`,[],`0x${BigInt(value).toString(16).padStart(64,'0')}`);
    for(const [field,want] of Object.entries({bountyId:item.bounty_id,creator:addressWord(item.creator.toLowerCase()),factory:addressWord(reviewed.factory),settlementToken:addressWord(reviewed.token),verifierModule:addressWord(reviewed.router),termsHash:item.terms_hash,policyHash:reviewed.policy_hash,acceptanceCriteriaHash:reviewed.acceptance_hash,benchmarkHash:documentHash(b),evidenceSchemaHash:documentHash(d.evidence_schema)}))add(`parent_${field}`,contract,`${field}()`,[],want);
    const values=await transport(rpc,calls);
    for(const role of ['router','adapter','participant_registry','terms_registry'])requireValue(keccak256Hex(values.get(`code_${role}`))===reviewed[`${role}_code_hash`],`routed_v3_${role}_code_mismatch`);
    for(const [key,want] of expected)requireValue(word(values.get(key))===word(want),`routed_v3_${key}_mismatch`);
    const policy=values.get('policy');requireValue(/^0x[0-9a-fA-F]{384}$/.test(policy || ''),'routed_v3_invalid_policy_record');
    const parts=Array.from({length:6},(_,i)=>`0x${policy.slice(2+i*64,2+(i+1)*64).toLowerCase()}`);
    requireValue(parts[0]===addressWord(reviewed.adapter) && parts[1]===reviewed.adapter_code_hash && BigInt(parts[4])>0n && parts[5]===ZERO,'routed_v3_changed_policy');
    const final=await transport(rpc,[{key:'v3_recheck',method:'eth_getBlockByNumber',params:[block.number,false]}]);
    requireValue(final.get('v3_recheck')?.hash===block.hash,'routed_v3_safe_block_changed');
    return {ready:true,descriptor:{schema_version:reviewed.schema_version,inventory_class:'post_bounty_third_party_completion',verifier_protocol:reviewed.verifier_protocol,verifier_module:reviewed.router,verifier_runtime_code_hash:reviewed.router_code_hash,adapter:reviewed.adapter,adapter_runtime_code_hash:reviewed.adapter_code_hash,policy_hash:reviewed.policy_hash,acceptance_criteria_hash:reviewed.acceptance_hash,requires_funded_canonical_child:true,requires_different_solver_wallet:true,required_child_status:'settled',required_child_quorum:reviewed.child_threshold,observed_block_number:number,observed_block_hash:block.hash,release_id:h.release_id,last_check_at:h.last_check_at}};
  } catch(error) { return {ready:false,reason:error instanceof Error ? error.message : 'routed_v3_verification_unavailable'}; }
}
