#!/usr/bin/env python3
"""Fixed compatible-wallet and pooled-funding checks on isolated Base Sepolia.

Read-only by default. Each --execute phase uses only the approved test account,
existing dedicated identities and native test USDC. No production key or network.
Exact envelopes and manually signed transactions are saved privately before send.
"""
import argparse
import copy
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import stat
import time
from eth_account import Account
from eth_account.messages import encode_defunct, encode_typed_data
from eth_abi import encode, decode
from web3 import Web3
import posting_sepolia_fixture_sender as fixture

spec=importlib.util.spec_from_file_location('legacy',Path(__file__).with_name('check-posting-hosted-legacy.py'))
L=importlib.util.module_from_spec(spec);spec.loader.exec_module(L)
h,ex=L.h,L.ex
ACCOUNT='0xf81e2ec972d570694e64c4583b59c4fc9375505c'
FACTORY='0xf699c4261709e67ad748c731ac0f7d014677e6e7'
EOA=h.funding.ADDRESSES['creator'];SPONSOR=h.funding.ADDRESSES['relayer']
STATE=h.STATE/'contract-wallet-journal.json'
OUTPUT=ex.ROOT/'docs/evidence/posting-hosted-contract-wallet-20261001.json'
TRANSFER_FIELDS=[('from','address'),('to','address'),('value','uint256'),('validAfter','uint256'),('validBefore','uint256'),('nonce','bytes32')]
SUBMIT_FIELDS=[('bounty','address'),('bountyId','bytes32'),('solver','address'),('round','uint64'),('submissionHash','bytes32'),('evidenceHash','bytes32'),('policyHash','bytes32'),('deadline','uint256')]
VERDICT_FIELDS=[('bounty','address'),('bountyId','bytes32'),('round','uint64'),('verifier','address'),('submissionHash','bytes32'),('evidenceHash','bytes32'),('policyHash','bytes32'),('passed','bool'),('responseHash','bytes32'),('deadline','uint256')]

def digest(name):return h.keccak256(('posting-contract-wallet-20261001/'+name).encode())
def wide(signature):return '0x7f'+signature[2:]
def rsv(signature):return {'v':int(signature[-2:],16),'r':'0x'+signature[2:66],'s':'0x'+signature[66:130]}
def token_call(name,types,values):return '0x'+(h.keccak_bytes(name.encode())[:4]+encode(types,values)).hex()
def balance(rpc,address):
    data=token_call('balanceOf(address)',['address'],[address])
    return int(rpc('eth_call',[{'to':h.funding.USDC,'data':data},'latest']),16)
def snapshot(rpc):
    value=h.balances(rpc)
    value['account']={'address':ACCOUNT,'eth_wei':str(int(rpc('eth_getBalance',[ACCOUNT,'latest']),16)),'test_usdc_micro':str(balance(rpc,ACCOUNT))}
    ex.require(value['account']['eth_wei']=='0','Contract customer must hold no ETH')
    return value

def lease_ready(j):
    end=max((r.get('reservation',{}).get('signer_lease_expires_at',0) for r in j.get('fixture_calls',{}).values()),default=0)
    ex.require(time.time()>end,'Manual fixture sender lease remains active; wait for its natural expiry')

def login(address,key,j):
    c,o,_=h.request('/v1/auth/wallet/challenge',{'address':address,'chain_id':84532})
    ex.require(c==200 and address in o['message'].lower() and 'Chain ID: 84532' in o['message'],'Unexpected test SIWE challenge')
    signature='0x'+Account.from_key(key).sign_message(encode_defunct(text=o['message'])).signature.hex()
    if address==ACCOUNT:signature=wide(signature)
    c,session,_=h.request('/v1/auth/wallet/session',{'challenge_id':o['challenge_id'],'signature':signature,'chain_id':84532})
    ex.require(c==200,'Test account login failed: HTTP '+str(c))
    j.setdefault('login_evidence',{})[address]={'verified':True,'signature_bytes':(len(signature)-2)//2,'chain_id':84532}
    ex.save(STATE,j)
    return {'Authorization':'Bearer '+session['token']}

def send(j,rpc,name,route,build,complete,headers=None):
    body=j['actions'][name]['body'] if name in j['actions'] else build()
    return L.send(name,route,body,j,STATE,rpc,headers,complete)

def document(label,creator,reward,initial):
    d=L.new_document();ct=d['contract_terms']
    ct.update(creator_wallet=creator,creation_nonce=digest(label+'-creation'))
    ct['solver_reward']['amount']=reward;ct['initial_funding']['amount']=initial
    d['verification_policy']['verifiers']=[creator]
    d.update(title='Synthetic '+label+' compatible wallet check',goal='Verify native test USDC and sponsored canonical actions for the separately approved test account.')
    return d

def prepare(j,label,creator,reward,initial,creator_login):
    flow=j.setdefault('flows',{}).setdefault(label,{'document':document(label,creator,reward,initial),'valid_before':int(time.time())+1800})
    ex.save(STATE,j)
    creator_headers = L.publication_headers(creator, flow['document'], creator_login)
    if 'terms' not in flow:
        c,o,_=h.request('/v1/base/autonomous-bounties/terms',{'creator_wallet':creator,'document':flow['document']}, creator_headers)
        ex.require(c==200,'Fixture terms unavailable: '+str(c)+' '+str(o));flow['terms']=o;ex.save(STATE,j)
    L.terms_publication_client.validate_record(flow['terms'], creator, flow['document'])
    ct=flow['document']['contract_terms'];terms=flow['terms']
    create={'creator':creator,**{k:ct[k] for k in ('solver_reward','verifier_reward','funding_deadline','claim_window_seconds','verification_window_seconds','initial_funding','creation_nonce')},
        **{k:terms[k] for k in ('terms_hash','policy_hash','acceptance_criteria_hash','benchmark_hash','evidence_schema_hash')},
        'verification_mode':'signed_quorum','verifier_module':None,'verifier_reward_recipient':None,'verifiers':[creator],'threshold':1}
    if 'plan' not in flow and initial > 0:
        c,o,_=h.request('/v1/base/autonomous-bounties/creation-plan',{'network':'base-sepolia','authorization_valid_before':flow['valid_before'],'create':create})
        ex.require(c==200,'Fixture creation plan unavailable: '+str(c)+' '+str(o));flow['plan']=o;ex.save(STATE,j)
    flow['create']=create;ex.save(STATE,j)
    return flow

def complete_bounty(j,rpc,label,flow,creator,creator_key,solver,solver_key):
    plan=flow['plan'];bounty=plan['predicted_bounty_contract'].lower();headers=login(solver,solver_key,j)
    def claim():
        body={'idempotency_key':'hosted-contract-wallet:'+label+':'+bounty,'network':'base-sepolia','bounty_contract':bounty,'solver_wallet':solver,'request_bond_sponsorship':False}
        c,o,_=h.request('/v1/base/autonomous-bounties/claims',body,headers)
        ex.require(c==200,'Claim prepare failed: '+str(c)+' '+str(o))
        body['wallet_signature']=L.transfer_signature(solver_key,o['signing_payload'],solver,bounty,100000)
        return body
    send(j,rpc,label+'-claim','/v1/base/autonomous-bounties/claims',claim,lambda o:o.get('candidate',{}).get('status')=='claimed',headers)
    submission={'bounty_contract':bounty,'bounty_id':plan['bounty_id'],'solver':solver,'round':1,'submission_hash':digest(label+'-submission'),'evidence_hash':digest(label+'-evidence'),'policy_hash':flow['terms']['policy_hash'],'deadline':flow['valid_before']}
    message={'bounty':bounty,'bountyId':plan['bounty_id'],'solver':solver,'round':1,'submissionHash':submission['submission_hash'],'evidenceHash':submission['evidence_hash'],'policyHash':submission['policy_hash'],'deadline':flow['valid_before']}
    def submit():
        signature=L.sign(solver_key,'Submit',SUBMIT_FIELDS,message,bounty)
        return {'network':'base-sepolia','submission':submission,'signature':wide(signature) if solver==ACCOUNT else signature}
    send(j,rpc,label+'-submission','/v1/base/autonomous-bounties/submission-relay',submit,lambda o:o.get('confirmed') is True)
    def verdict():
        deadline=int(time.time())+540
        att={k:v for k,v in submission.items() if k!='solver'};att.update(verifier=creator,passed=True,response_hash=digest(label+'-passed'),deadline=deadline)
        m={k:v for k,v in message.items() if k!='solver'};m.update(verifier=creator,passed=True,responseHash=att['response_hash'],deadline=deadline)
        sig=L.sign(creator_key,'VerificationAttestation',VERDICT_FIELDS,m,bounty)
        return {'network':'base-sepolia','attestation':att,'signature':wide(sig) if creator==ACCOUNT else sig}
    send(j,rpc,label+'-verdict','/v1/base/autonomous-bounties/creator-verdict-relay',verdict,lambda o:o.get('solver_paid') is True)
    ex.require(balance(rpc,bounty)==0,'Settled fixture retains USDC')

def account_flow(j,rpc,wallets):
    owner=wallets['solver']['private_key']
    flow=prepare(j,'account-creator',ACCOUNT,300000,400000,login(ACCOUNT,owner,j))
    def create():
        sig=L.transfer_signature(owner,flow['plan']['eip3009_authorization'],ACCOUNT,flow['plan']['predicted_bounty_contract'].lower(),400000)
        return {'network':'base-sepolia','authorization_valid_before':flow['valid_before'],'create':flow['create'],'signature':rsv(sig)}
    send(j,rpc,'account-creation','/v1/base/autonomous-bounties/creation-relay',create,lambda o:o.get('funding_confirmed') is True)
    complete_bounty(j,rpc,'account-creator',flow,ACCOUNT,owner,EOA,wallets['creator']['private_key'])
    def revoke():
        message={'authorizer':ACCOUNT,'nonce':digest('unused-authorization')}
        sig=L.sign(owner,'CancelAuthorization',[('authorizer','address'),('nonce','bytes32')],message,h.funding.USDC,True)
        return {'network':'base-sepolia',**message,'signature':wide(sig)}
    send(j,rpc,'account-revocation','/v1/base/usdc/authorization-revocation-relay',revoke,lambda o:o.get('authorization_cancelled') is True)

def pooled_creation(j,rpc,wallets):
    flow=prepare(j,'pooled',SPONSOR,500000,0,login(SPONSOR,wallets['relayer']['private_key'],j));c=flow['create']
    params=(500000,100000,*[bytes.fromhex(c[k][2:]) for k in ('terms_hash','policy_hash','acceptance_criteria_hash','benchmark_hash','evidence_schema_hash')],c['funding_deadline'],c['claim_window_seconds'],c['verification_window_seconds'],1,'0x'+'00'*20,'0x'+'00'*20,1)
    signature='createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)'
    types=['(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8)','address[]','uint256','bytes32']
    data=token_call(signature,types,[params,[SPONSOR],0,bytes.fromhex(c['creation_nonce'][2:])])
    # Public creation planning intentionally requires full atomic funding. This
    # fixed sponsor-owned protocol fixture is separate from that customer route,
    # just as in the existing local pooled-contribution integration test.
    reviewed=ex.load_bundle()['transactions'][0]
    ex.require(reviewed['expected_contract']==FACTORY and rpc('eth_getCode',[FACTORY,'latest']).lower()==reviewed['expected_runtime_code'].lower(),'Unreviewed fixture factory')
    tuple_type=types[0];nonce=bytes.fromhex(c['creation_nonce'][2:])
    bid=h.keccak256(encode(['uint256','address','address','bytes32',tuple_type,'address[]'],[84532,FACTORY,SPONSOR,nonce,params,[SPONSOR]]))
    getter_types=['address',tuple_type,'address[]','bytes32'];getter_values=[SPONSOR,params,[SPONSOR],nonce]
    read=lambda name:rpc('eth_call',[{'to':FACTORY,'data':token_call(name,getter_types,getter_values)},'latest'])
    ex.require(read('bountyIdFor(address,'+tuple_type+',address[],bytes32)')==bid,'Independent fixture bounty ID differs')
    predicted='0x'+read('predictBountyAddress(address,'+tuple_type+',address[],bytes32)')[-40:]
    implementation='0x'+rpc('eth_call',[{'to':FACTORY,'data':'0x'+h.keccak_bytes(b'implementation()')[:4].hex()},'latest'])[-40:]
    clone=bytes.fromhex('3d602d80600a3d3981f3363d3d373d3d3d363d73'+implementation[2:]+'5af43d82803e903d91602b57fd5bf3')
    expected='0x'+h.keccak_bytes(b'\xff'+bytes.fromhex(FACTORY[2:])+bytes.fromhex(bid[2:])+h.keccak_bytes(clone))[-20:].hex()
    ex.require(predicted==expected,'Independent CREATE2 prediction differs')
    plan={'predicted_bounty_contract':predicted,'bounty_id':bid,'fixture_source':'Exact approved factory getters plus independent ABI and CREATE2 encoding; public full-funding planner gate retained'}
    ex.require('plan' not in flow or flow['plan']==plan,'Saved pooled plan changed')
    flow['plan']=plan;ex.save(STATE,j)
    return fixture.send(h,'pooled-creation',FACTORY,data,800000,j,STATE,rpc,wallets)

def pooled_flow(j,rpc,wallets):
    ex.require(j.get('fixture_calls',{}).get('pooled-creation',{}).get('receipt'),'Pooled fixture creation not reconciled')
    flow=j['flows']['pooled'];bounty=flow['plan']['predicted_bounty_contract'].lower()
    feed=h.get('/v1/base/autonomous-bounties/feed?network=base-sepolia&bounty_contract='+bounty)
    ex.require(len(feed)==1 and feed[0]['terms_valid'],'Pooled creation not indexed with valid terms')
    j['pooled_canonical_index']=feed[0];ex.save(STATE,j)
    for label,address,role,amount in [('contract',ACCOUNT,'solver',200000),('eoa',EOA,'creator',400000)]:
        def contribute(label=label,address=address,role=role,amount=amount):
            expiry=int(time.time())+1500;nonce=digest('pooled-'+label)
            contribution={'bounty_contract':bounty,'contributor':address,'amount':{'amount':amount,'currency':'usdc'},'authorization_nonce':nonce,'authorization_valid_before':expiry}
            message={'from':address,'to':bounty,'value':amount,'validAfter':0,'validBefore':expiry,'nonce':nonce}
            sig=L.sign(wallets[role]['private_key'],'TransferWithAuthorization',TRANSFER_FIELDS,message,h.funding.USDC,True)
            return {'network':'base-sepolia','contribution':contribution,'signature':sig}
        send(j,rpc,'pooled-'+label+'-contribution','/v1/base/autonomous-bounties/contribution-relay',contribute,lambda o:o.get('funding_confirmed') is True)
    complete_bounty(j,rpc,'pooled',flow,SPONSOR,wallets['relayer']['private_key'],ACCOUNT,wallets['solver']['private_key'])

def open_flow(j,rpc,wallets):
    ex.require(j['actions'].get('pooled-verdict',{}).get('replay_verified'),'Finish pooled lifecycle first')
    if 'open_plans' not in j:
        req=copy.deepcopy(h.new_plan()[0]['request']);terms=req['original_terms']
        terms.update(bounty_id=digest('open-wide'),solver_reward_micro_usdc=500000,review_reward_micro_usdc=100000,
                     title='Synthetic wider-signature contract submission',goal='Verify contract-account token and action signatures through hosted creator-open settlement.')
        req.update(user_salt=digest('open-wide-salt'),expected_solver_reward_micro_usdc=500000,action={'kind':'create','token_nonce':digest('open-wide-create')})
        plans=[{'name':'open-wide-create','request':copy.deepcopy(req),'event':'CreatorOpenBountyCreated'}]
        req['action']={'kind':'submit','solver':ACCOUNT,'submission_hash':digest('open-wide-submission'),'evidence_hash':digest('open-wide-evidence'),'token_nonce':digest('open-wide-bond')}
        plans.append({'name':'open-wide-submit','request':copy.deepcopy(req),'event':'OpenSubmissionAdded'})
        req['action']={k:v for k,v in req['action'].items() if k!='token_nonce'}
        req['action'].update(kind='review',entry=1,passed=True,response_hash=digest('open-wide-verdict'))
        plans.append({'name':'open-wide-review','request':copy.deepcopy(req),'event':'OpenBountySettled'})
        j['open_plans']=plans;j['open_records']={};ex.save(STATE,j)
    original=h.sign_requirement
    def contract_signature(requirement,identities,expected_signer,contract,primary,message,domain,version):
        if expected_signer!=ACCOUNT:return original(requirement,identities,expected_signer,contract,primary,message,domain,version)
        aliases={**identities,'approved_contract_owner':{'address':ACCOUNT,'private_key':wallets['solver']['private_key']}}
        sig=wide(original(requirement,aliases,expected_signer,contract,primary,message,domain,version))
        call=token_call('isValidSignature(bytes32,bytes)',['bytes32','bytes'],[bytes.fromhex(requirement['digest'][2:]),bytes.fromhex(sig[2:])])
        result=rpc('eth_call',[{'to':ACCOUNT,'data':call},'latest'])
        ex.require(result.startswith('0x1626ba7e'),'Reviewed contract did not accept exact owner signature')
        return sig
    h.sign_requirement=contract_signature
    try:
        for plan in j['open_plans']:
            record=j['open_records'].setdefault(plan['name'],{})
            if plan['name']=='open-wide-submit' and 'invalid_wide_signature_rejected' not in j:
                c,o,_=h.request('/v1/base/creator-open/prepare',plan['request']);ex.require(c==200,'Contract entry preparation unavailable')
                record['signed_body']=h.open_body(plan['request'],o['prepared'],wallets);ex.save(STATE,j)
                bad=copy.deepcopy(record['signed_body']);bad['signatures']['action']='0x7e'+bad['signatures']['action'][4:]
                before=h.get('/v1/base/gas-sponsorship')['daily_budget'];nonce=rpc('eth_getTransactionCount',[SPONSOR,'pending'])
                c,_,_=h.request('/v1/base/creator-open/relay',bad)
                ex.require(c==401 and before==h.get('/v1/base/gas-sponsorship')['daily_budget'] and nonce==rpc('eth_getTransactionCount',[SPONSOR,'pending']),'Invalid wide signature did not fail before gas')
                j['invalid_wide_signature_rejected']={'http':c,'budget_unchanged':True,'sponsor_nonce_unchanged':True};ex.save(STATE,j)
            h.run_open(plan,record,j,STATE,wallets,rpc)
    finally:h.sign_requirement=original
    ex.require(balance(rpc,j['open_records']['open-wide-create']['outcome']['bounty_contract'])==0,'Creator-open escrow not empty')

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--execute',action='store_true');parser.add_argument('--phase',choices=['fund','account','pooled-create','pooled','open','return'],default='account');parser.add_argument('--revision',required=True);args=parser.parse_args()
    rpc=ex.Rpc(broadcast=args.execute);ex.load_bundle();ex.fresh_head(rpc)
    bundle=json.loads((ex.ROOT/'docs/evidence/posting-sepolia-account-unsigned-review-20260930.json').read_text())
    ex.require(rpc('eth_getCode',[ACCOUNT,'latest']).lower()==bundle['transaction']['expected_runtime_code'].lower(),'Test account runtime changed')
    code,health,headers=h.request('/health');ex.require(code==200 and health=='ok' and headers.get('x-agent-bounties-revision')==args.revision,'Unexpected staging revision')
    caps=h.get('/v1/base/creator-open/capabilities');ex.require(not next(n for n in caps['networks'] if n['network']=='base-mainnet')['available'],'Mainnet must be unavailable')
    budget=h.get('/v1/base/gas-sponsorship')['daily_budget'];ex.require(budget['limit_micro_usdc']==1000000 and budget['carries_forward'] is False,'Shared daily cap differs')
    initial=snapshot(rpc);print(json.dumps({'phase':args.phase,'execute':args.execute,'daily_budget':budget,'account':initial['account']}),flush=True)
    if not args.execute:return
    ex.require(budget['remaining_micro_usdc']>=100000,'Insufficient shared test allowance')
    with os.fdopen(os.open(h.STATE/'lock',os.O_CREAT|os.O_RDWR,0o600),'w') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        if STATE.exists():
            ex.require(stat.S_IMODE(STATE.stat().st_mode)==0o600,'Private journal permissions');j=json.loads(STATE.read_text())
        else:j={'network':'base-sepolia','api':h.API,'revision':args.revision,'account':ACCOUNT,'actions':{},'initial_balances':initial,'initial_budget':budget}
        ex.require(j['network']=='base-sepolia' and j['api']==h.API and j['account']==ACCOUNT and j['revision']==args.revision,'Saved scope changed')
        path=ex.PRIVATE/'wallets.json';ex.require(stat.S_IMODE(path.stat().st_mode)==0o600,'Private key permissions');wallets=json.loads(path.read_text())
        for role,address in h.funding.ADDRESSES.items():ex.require(Account.from_key(wallets[role]['private_key']).address.lower()==address,'Wrong dedicated key')
        lease_ready(j);ex.save(STATE,j)
        if args.phase=='fund':
            data=token_call('transfer(address,uint256)',['address','uint256'],[ACCOUNT,1000000])
            fixture.send(h,'account-funding',h.funding.USDC,data,120000,j,STATE,rpc,wallets)
        elif args.phase=='account':account_flow(j,rpc,wallets)
        elif args.phase=='pooled-create':pooled_creation(j,rpc,wallets)
        elif args.phase=='pooled':pooled_flow(j,rpc,wallets)
        elif args.phase=='open':open_flow(j,rpc,wallets)
        else:
            ex.require(all(j['actions'].get(n,{}).get('replay_verified') for n in ('account-revocation','pooled-verdict')),'Lifecycle checks incomplete')
            open_complete=j.get('open_records',{}).get('open-wide-review',{}).get('replay_verified')
            blocked=j.get('open_quota_blocked')
            ex.require(open_complete or blocked,'Wide creator-open lifecycle is neither complete nor reconciled as quota-blocked')
            if not open_complete:
                ex.require(blocked['relay_http']==429 and not any(r.get('receipt') for r in j.get('open_records',{}).values())
                           and rpc('eth_getCode',[blocked['bounty'],'latest'])=='0x','Quota-blocked fixture unexpectedly executed')
                c,_,_=h.request('/v1/base/creator-open/status',{'network':'base-sepolia','approval_hash':blocked['approval_hash']})
                ex.require(c==404,'Quota-blocked request acquired an operation; reconcile before recovery')
            amount=1500000 if open_complete else 1000000
            if 'return_call' not in j:
                ex.require(balance(rpc,ACCOUNT)==amount,'Unexpected recoverable test principal')
                before=int(time.time())+1500;nonce=digest('principal-return');m={'from':ACCOUNT,'to':SPONSOR,'value':amount,'validAfter':0,'validBefore':before,'nonce':nonce}
                sig=wide(L.sign(wallets['solver']['private_key'],'TransferWithAuthorization',TRANSFER_FIELDS,m,h.funding.USDC,True))
                j['return_call']=token_call('transferWithAuthorization(address,address,uint256,uint256,uint256,bytes32,bytes)',['address','address','uint256','uint256','uint256','bytes32','bytes'],[ACCOUNT,SPONSOR,amount,0,before,bytes.fromhex(nonce[2:]),bytes.fromhex(sig[2:])]);ex.save(STATE,j)
            decoded=decode(['address','address','uint256','uint256','uint256','bytes32','bytes'],bytes.fromhex(j['return_call'][10:]))
            ex.require(j['return_call'][:10]=='0x'+h.keccak_bytes(b'transferWithAuthorization(address,address,uint256,uint256,uint256,bytes32,bytes)')[:4].hex()
                       and decoded[:4]==(ACCOUNT,SPONSOR,amount,0) and decoded[5]==bytes.fromhex(digest('principal-return')[2:]) and len(decoded[6])==66,'Saved return authority differs')
            fixture.send(h,'account-principal-return',h.funding.USDC,j['return_call'],150000,j,STATE,rpc,wallets)
            ex.require(balance(rpc,ACCOUNT)==0,'Account principal not fully recovered')
        evidence={'network':'base-sepolia','observed_at':datetime.now(timezone.utc).isoformat(),'revision':args.revision,'account':ACCOUNT,'initial_balances':j['initial_balances'],'current_balances':snapshot(rpc),'initial_budget':j['initial_budget'],'current_budget':h.get('/v1/base/gas-sponsorship')['daily_budget'],
            'login_evidence':j.get('login_evidence',{}),'actions':[{'name':n,'receipt':r['receipt'],'replay_verified':r.get('replay_verified',False)} for n,r in j['actions'].items() if r.get('receipt')],
            'creator_open_actions':[{'name':n,'receipt':r['receipt'],'replay_verified':r.get('replay_verified',False)} for n,r in j.get('open_records',{}).items() if r.get('receipt')],
            'invalid_wide_signature_rejected':j.get('invalid_wide_signature_rejected'),
            'creator_open_quota_blocked':j.get('open_quota_blocked'),
            'fixture_calls':[{'name':n,'receipt':r['receipt'],'reservation':r['reservation']} for n,r in j.get('fixture_calls',{}).items() if r.get('receipt')],
            'principal_recovered':bool(j.get('fixture_calls',{}).get('account-principal-return',{}).get('receipt')),'production_changed':False}
        OUTPUT.write_text(json.dumps(evidence,indent=2)+'\n');print(json.dumps({'completed_actions':len(evidence['actions']),'principal_recovered':evidence['principal_recovered']}))

if __name__=='__main__':main()
