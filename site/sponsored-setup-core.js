/* Independently reconstruct setup calldata, empty-wallet address and owner approval. */
(function(root,factory){const api=factory();if(typeof module==="object"&&module.exports)module.exports=api;else root.AgentBountiesSponsoredSetup=api;})(typeof window==="object"?window:globalThis,function(){
  "use strict";
  const SCHEMA="agent-bounties/sponsored-setup-v1", ADDRESS=/^0x[0-9a-f]{40}$/i, HASH=/^0x[0-9a-f]{64}$/i, ZERO="0x"+"0".repeat(40), ZHASH="0x"+"0".repeat(64);
  const lower=v=>String(v||"").toLowerCase();
  const stable=v=>v&&typeof v==="object"?(Array.isArray(v)?`[${v.map(stable).join(",")}]`:`{${Object.keys(v).sort().map(k=>JSON.stringify(k)+":"+stable(v[k])).join(",")}}`):JSON.stringify(v);
  const check=(ok,message)=>{if(!ok)throw new Error(message);};
  const fields=s=>s.split(",").map(pair=>{const[name,type]=pair.split(":");return{name,type};});
  const DOMAIN=fields("name:string,version:string,chainId:uint256,verifyingContract:address"), AUTH=fields("owner:address,target:address,callHash:bytes32,releaseHash:bytes32,validBefore:uint256");
  const releaseKeys=["network","bounty_factory","bounty_factory_code_hash","wallet_factory","wallet_factory_code_hash","wallet_implementation","wallet_implementation_code_hash","participant_registry","participant_registry_code_hash","deployment_block"];
  function bytes32(value){check(HASH.test(value),"Invalid 32-byte setup field.");return lower(value).slice(2);}
  function chain(network){check(["base-mainnet","base-sepolia"].includes(network),"Choose Base or Base Sepolia.");return network==="base-mainnet"?8453:84532;}
  function integer(value,min=0){check(Number.isSafeInteger(value)&&value>=min,"Invalid setup amount or time.");return value;}
  function exact(value,keys){check(value&&typeof value==="object"&&!Array.isArray(value)&&Object.keys(value).sort().join()===keys.slice().sort().join(),"Unexpected setup fields.");}
  function policyBytes(p,e){
    exact(p,["delegate","valid_after","valid_until","period_seconds","max_per_action_micro_usdc","max_per_period_micro_usdc","max_lifetime_micro_usdc","max_bounty_target_micro_usdc","allowed_actions","allowed_verification_modes","deterministic_verifier_module","signed_quorum_verifier_set_hash","ai_judge_verifier_set_hash"]);
    check(ADDRESS.test(p.delegate)&&lower(p.delegate)!==ZERO&&ADDRESS.test(p.deterministic_verifier_module)&&HASH.test(p.signed_quorum_verifier_set_hash)&&HASH.test(p.ai_judge_verifier_set_hash),"Invalid policy wallet or verifier commitment.");
    for(const key of ["valid_after","valid_until","period_seconds","max_per_action_micro_usdc","max_per_period_micro_usdc","max_lifetime_micro_usdc","max_bounty_target_micro_usdc","allowed_actions","allowed_verification_modes"])integer(p[key]);
    check(p.valid_until>p.valid_after&&p.allowed_actions>0&&p.allowed_actions<=15&&p.allowed_verification_modes>0&&p.allowed_verification_modes<=7&&p.max_bounty_target_micro_usdc>0,"Invalid policy scope.");
    if(p.allowed_actions&7)check(p.period_seconds>0&&p.max_per_action_micro_usdc>0&&p.max_per_period_micro_usdc>0&&p.max_lifetime_micro_usdc>0,"Spending actions need positive limits.");
    for(const[bit,key,zero]of[[1,"deterministic_verifier_module",ZERO],[2,"signed_quorum_verifier_set_hash",ZHASH],[4,"ai_judge_verifier_set_hash",ZHASH]])check(Boolean(p.allowed_verification_modes&bit)!==(lower(p[key])===zero),"Verifier commitment does not match the policy.");
    return e.addressWord(p.delegate)+[p.valid_after,p.valid_until,p.period_seconds,p.max_per_action_micro_usdc,p.max_per_period_micro_usdc,p.max_lifetime_micro_usdc,p.max_bounty_target_micro_usdc,p.allowed_actions,p.allowed_verification_modes].map(e.uint256Word).join("")+e.addressWord(p.deterministic_verifier_module)+bytes32(p.signed_quorum_verifier_set_hash)+bytes32(p.ai_judge_verifier_set_hash);
  }
  function hashTyped(type,descriptors,values,e){const declaration=`${type}(${descriptors.map(f=>f.type+" "+f.name).join(",")})`;return e.keccak256Hex("0x"+e.bytes32Word(e.keccak256Hex(e.textHex(declaration)))+descriptors.map(f=>f.type==="string"?e.bytes32Word(e.keccak256Hex(e.textHex(values[f.name]))):f.type==="address"?e.addressWord(values[f.name]):f.type==="bytes32"?e.bytes32Word(values[f.name]):e.uint256Word(values[f.name])).join(""));}
  function expected(release,r,e,now=Math.floor(Date.now()/1000),recovery=false){
    exact(release,releaseKeys);exact(r,["network","owner","valid_before","action"]);check(release.network===r.network&&ADDRESS.test(r.owner)&&lower(r.owner)!==ZERO,"Invalid setup owner or network.");
    const normalized={};for(const key of releaseKeys){if(key==="network")normalized[key]=release[key];else if(key==="deployment_block")normalized[key]=integer(release[key],1);else{check((key.endsWith("_hash")?HASH:ADDRESS).test(release[key])&&!/^0x0+$/.test(release[key]),"Missing reviewed release.");normalized[key]=lower(release[key]);}}
    check(new Set([normalized.bounty_factory,normalized.wallet_factory,normalized.wallet_implementation,normalized.participant_registry]).size===4,"Setup release addresses overlap.");
    integer(r.valid_before,1);if(!recovery)check(r.valid_before>now+30&&r.valid_before<=now+7200,"The setup approval has expired or is too long. Prepare a fresh proposal.");
    const a=r.action,w=e.uint256Word,b=bytes32,address=e.addressWord,selector=s=>e.keccak256Hex(e.textHex(s)).slice(0,10);
    let target,data,predicted_wallet=null;
    if(a.kind==="wallet_deploy"){
      exact(a,["kind","policy","user_salt"]);check(HASH.test(a.user_salt),"Invalid wallet salt.");const encoded=policyBytes(a.policy,e);
      if(!recovery)check(a.policy.valid_until>now,"The proposed wallet policy expired.");
      target=normalized.wallet_factory;
      data=selector("createWallet(address,(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32),bytes32)")+address(r.owner)+encoded+b(a.user_salt);
      const salt=e.keccak256Hex("0x"+address(r.owner)+b(a.user_salt)+b(e.keccak256Hex("0x"+encoded))),init=e.keccak256Hex("0x3d602d80600a3d3981f3363d3d373d3d3d363d73"+normalized.wallet_implementation.slice(2)+"5af43d82803e903d91602b57fd5bf3");
      predicted_wallet="0x"+e.keccak256Hex("0xff"+target.slice(2)+salt.slice(2)+init.slice(2)).slice(-40);
    }else{
      exact(a,["kind","participant_id","source_hash","valid_until","registry_nonce","attester_signature"]);
      check(a.kind==="participant_register"&&HASH.test(a.participant_id)&&HASH.test(a.source_hash)&&lower(a.participant_id)!==ZHASH&&lower(a.source_hash)!==ZHASH&&/^0x[0-9a-f]{128}(1b|1c)$/i.test(a.attester_signature),"An exact registry attestation is required.");
      integer(a.valid_until,1);integer(a.registry_nonce);if(!recovery)check(a.valid_until>now&&a.valid_until<=now+365*86400,"Registry attestation validity is out of bounds.");
      target=normalized.participant_registry;data=selector("register(address,bytes32,bytes32,uint64,bytes)")+address(r.owner)+b(a.participant_id)+b(a.source_hash)+w(a.valid_until)+w(160)+w(65)+lower(a.attester_signature).slice(2).padEnd(192,"0");
    }
    const typed={types:{EIP712Domain:DOMAIN,SponsoredSetup:AUTH},domain:{name:"Agent Bounties Sponsored Setup",version:"1",chainId:chain(r.network),verifyingContract:target},primaryType:"SponsoredSetup",message:{owner:lower(r.owner),target,callHash:e.keccak256Hex(data),releaseHash:e.keccak256Hex(e.textHex(JSON.stringify(normalized))),validBefore:String(r.valid_before)}};
    const digest=e.keccak256Hex("0x1901"+hashTyped("EIP712Domain",DOMAIN,typed.domain,e).slice(2)+hashTyped("SponsoredSetup",AUTH,typed.message,e).slice(2));
    return{typed,digest,target,data,predicted_wallet};
  }
  function validatePrepared(p,release,r,e,now){const local=expected(release,r,e,now);check(p?.schema===SCHEMA&&stable(p.request)===stable(r)&&stable(p.release)===stable(release)&&lower(p.digest)===local.digest&&stable(p.typed_data)===stable(local.typed)&&lower(p.transaction?.to)===local.target&&lower(p.transaction?.data)===local.data&&String(p.transaction?.value_wei)==="0"&&p.transaction.from==null&&p.predicted_wallet===local.predicted_wallet,"The service changed the setup you reviewed.");return local;}
  return{SCHEMA,ADDRESS,HASH,stable,expected,validatePrepared,chain};
});
