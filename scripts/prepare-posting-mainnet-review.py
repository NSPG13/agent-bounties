#!/usr/bin/env python3
"""Read-only mainnet inventory and exact local-fork CREATE2 proposal. Never signs.

Only the owned loopback Anvil process accepts mutations. No private key is read.
Existing mainnet contract authority is preserved, including the real attester.
"""
from datetime import datetime, timezone
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request
import urllib.error
from urllib.parse import urlsplit
from _shared.evm import address_word, artifact_hex, create_address, keccak256, keccak_bytes

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('release_builder',Path(__file__).with_name('build-posting-sepolia-release.py'))
builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)
RPC='https://mainnet.base.org'
USDC='0x833589fcd6edb6e08f4c7c32d4f71b54bda02913'
CREATE2='0x4e59b44847b379578588920ca78fbf26c0b4956c'
CREATE2_HASH='0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989'
ADMIN='0x884834e884d6e93462655a2820140ad03e6747bc'
ALLOWED={'eth_chainId','eth_getBlockByNumber','eth_getCode','eth_getBalance','eth_getTransactionCount','eth_call'}


def rpc(url,method,params):
    if url!=RPC and not (urlsplit(url).hostname=='127.0.0.1' and urlsplit(url).scheme=='http'):
        raise ValueError('Unapproved endpoint')
    if url==RPC and method not in ALLOWED:raise ValueError('Remote mutations forbidden')
    req=urllib.request.Request(url,data=json.dumps({'jsonrpc':'2.0','id':1,'method':method,'params':params}).encode(),headers={'Content-Type':'application/json','User-Agent':'agent-bounties-release/1'})
    if url==RPC:time.sleep(.2)
    for attempt in range(3):
        try:
            with urllib.request.build_opener(builder.NoRedirect()).open(req,timeout=30)as response:value=json.load(response)
            break
        except urllib.error.HTTPError as error:
            if url!=RPC or error.code!=429 or attempt==2:raise
            time.sleep(5*(attempt+1))
    if value.get('jsonrpc')!='2.0' or value.get('id')!=1 or 'error'in value:raise ValueError(f'RPC {method} failed')
    return value['result']


def call(address,name,block):
    return rpc(RPC,'eth_call',[{'to':address,'data':keccak256((name+'()').encode())[:10]},block])


def verify_runtime(name,address,expected_immutables,block):
    artifact,_,metadata=builder.checked_artifact(name)
    code=bytes.fromhex(rpc(RPC,'eth_getCode',[address,block])[2:]);actual=bytearray(code)
    expected=bytearray(artifact_hex(artifact['deployedBytecode'],name));observed=set()
    for refs in artifact['deployedBytecode'].get('immutableReferences',{}).values():
        for ref in refs:
            start,n=ref['start'],ref['length'];observed.add('0x'+code[start:start+n].hex())
            actual[start:start+n]=bytes(n);expected[start:start+n]=bytes(n)
    if actual!=expected or observed!={'0x'+address_word(a).hex()for a in expected_immutables}:
        raise ValueError(name+' differs from reviewed bytecode or immutable authority')
    return {'address':address,'runtime_code_hash':keccak256(code),'reviewed_nonimmutable_bytes_exact':True,
            'immutable_addresses':sorted(expected_immutables),'source_hashes':{p:v['keccak256']for p,v in metadata['sources'].items()}}


def rehearse_native_lifecycle(local,factory_address):
    from eth_account import Account
    from eth_abi import encode
    from web3 import Web3
    if urlsplit(local).hostname!='127.0.0.1':raise ValueError('Fixture writes require owned loopback fork')
    w3=Web3(Web3.HTTPProvider(local,request_kwargs={'timeout':30}));admin=Web3.to_checksum_address(ADMIN)
    abi=[]
    for name,inputs,outputs,mutability in [('masterMinter',[],['address'],'view'),('configureMinter',['address','uint256'],['bool'],'nonpayable'),('mint',['address','uint256'],['bool'],'nonpayable'),('balanceOf',['address'],['uint256'],'view'),('DOMAIN_SEPARATOR',[],['bytes32'],'view')]:
        abi.append({'type':'function','name':name,'stateMutability':mutability,'inputs':[{'name':f'a{i}','type':t}for i,t in enumerate(inputs)],'outputs':[{'name':'','type':t}for t in outputs]})
    token=w3.eth.contract(address=Web3.to_checksum_address(USDC),abi=abi)
    def send(fn,sender=admin):
        digest=fn.transact({'from':sender,'gas':3_000_000});receipt=w3.eth.wait_for_transaction_receipt(digest)
        if receipt.status!=1:raise ValueError('Local lifecycle reverted')
        return receipt
    master=token.functions.masterMinter().call();rpc(local,'anvil_impersonateAccount',[master]);rpc(local,'anvil_setBalance',[master,hex(10**18)])
    send(token.functions.configureMinter(admin,100_000_000),master)
    creator=Account.from_key(keccak_bytes(b'agent-bounties/posting-mainnet-fork/creator/20260930'))
    owner=Account.from_key(keccak_bytes(b'agent-bounties/posting-mainnet-fork/solver/20260930'))
    solver=owner.address
    if any(w3.eth.get_balance(a)!=0 for a in [creator.address,solver]):raise ValueError('Customer fixtures must start with zero ETH')
    send(token.functions.mint(creator.address,21_300_000));send(token.functions.mint(solver,1_000_000))
    fa=json.loads((ROOT/'contracts/base-escrow/out/CreatorReviewBountyFactoryV1.sol/CreatorReviewBountyFactoryV1.json').read_text())
    factory=w3.eth.contract(address=Web3.to_checksum_address(factory_address),abi=fa['abi'])
    now=w3.eth.get_block('latest').timestamp;h=lambda s:keccak_bytes(('posting-exact-mainnet-fork/'+s).encode())
    c=(h('bounty'),creator.address,20_300_000,1_000_000,h('terms'),h('criteria'),h('schema'),now+90*86400,2*86400,8)
    salt=h('salt');bounty_address=factory.functions.predict(c,salt).call()
    def authorization(account,holder,value,nonce,wide=False):
        deadline=now+3600
        struct=keccak_bytes(encode(['bytes32','address','address','uint256','uint256','uint256','bytes32'],[keccak_bytes(b'ReceiveWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)'),holder,bounty_address,value,0,deadline,nonce]))
        digest=keccak_bytes(b'\x19\x01'+token.functions.DOMAIN_SEPARATOR().call()+struct)
        signature=bytes(account.unsafe_sign_hash(digest).signature)+(b'\x01'if wide else b'')
        return (0,deadline,nonce,signature)
    funding=authorization(creator,creator.address,21_300_000,h('fund'))
    created=send(factory.functions.createWithAuthorization(c,salt,funding))
    replay=send(factory.functions.createWithAuthorization(c,salt,funding))
    if token.functions.balanceOf(bounty_address).call()!=21_300_000:raise ValueError('Funding replay moved principal')
    ba=json.loads((ROOT/'contracts/base-escrow/out/CreatorReviewBountyV1.sol/CreatorReviewBountyV1.json').read_text());bounty=w3.eth.contract(address=bounty_address,abi=ba['abi'])
    submission=(solver,h('submission'),h('evidence'),c[4],1,now+3600)
    bond=authorization(owner,solver,1_000_000,h('bond'))
    digest=bounty.functions.submissionDigest(submission,bond[2]).call()
    submitted=send(bounty.functions.submitAuthorized(submission,bond,bytes(owner.unsafe_sign_hash(digest).signature)))
    response=h('response');digest=bounty.functions.reviewDigest(1,True,response,now+3600).call()
    settled=send(bounty.functions.reviewAuthorized(1,True,response,now+3600,bytes(creator.unsafe_sign_hash(digest).signature)))
    balances={'creator':token.functions.balanceOf(creator.address).call(),'solver':token.functions.balanceOf(solver).call(),'escrow':token.functions.balanceOf(bounty_address).call()}
    if balances!={'creator':1_000_000,'solver':21_300_000,'escrow':0}or bounty.functions.winner().call()!=1:raise ValueError('Fork payout failed conservation')
    if any(w3.eth.get_balance(a)!=0 for a in [creator.address,solver]):raise ValueError('Customer paid gas in local fork')
    return {'network':'local-mainnet-fork','factory':factory_address,'bounty':bounty_address,'native_usdc':USDC,'wider_erc1271_solver':False,'customer_eth_wei':'0','final_micro_usdc':balances,'duplicate_creation_did_not_move_principal':True,'local_transactions':[r.transactionHash.hex()for r in [created,replay,submitted,settled]],'canonical_settlement_on_remote_chain':False,'fixture_minting':'local impersonation only; no real tokens moved'}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--anvil',required=True);p.add_argument('--output',type=Path,required=True);args=p.parse_args()
    if rpc(RPC,'eth_chainId',[])!='0x2105':raise ValueError('Wrong network')
    head=rpc(RPC,'eth_getBlockByNumber',['latest',False])
    if abs(time.time()-int(head['timestamp'],16))>180:raise ValueError('Stale mainnet')
    block=rpc(RPC,'eth_getBlockByNumber',['safe',False]);tag=block['number']
    manifest=json.loads((ROOT/'deployments/bounded-agent-wallet-v2-base-mainnet.json').read_text())
    f=manifest['wallet_factory'];legacy=manifest['canonical']['bounty_factory'];implementation='0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9'
    registry='0x9875dcaf570bde8ff1aa62275d3c8985f4fd1294';attester='0x5563ca06363eedb3d39ae1733bb8aa6eca3d5728'
    inventory={name:verify_runtime(name,address,immutables,tag)for name,address,immutables in [
        ('AgentBountyFactory',legacy,[USDC,implementation]),('AgentBounty',implementation,[]),
        ('BoundedAgentWalletV2Factory',f['address'],[legacy,USDC,f['implementation']]),
        ('BoundedAgentWalletV2',f['implementation'],[legacy,USDC,f['address']]),
        ('ParticipantEligibilityRegistry',registry,[attester])]}
    for address,name,expected in [(legacy,'implementation',implementation),(legacy,'settlementToken',USDC),(f['address'],'bountyFactory',legacy),(f['address'],'implementation',f['implementation']),(f['address'],'settlementToken',USDC),(registry,'attester',attester)]:
        if call(address,name,tag)!='0x'+address_word(expected).hex():raise ValueError('Dependency getter mismatch')
    if keccak256(bytes.fromhex(rpc(RPC,'eth_getCode',[CREATE2,tag])[2:]))!=CREATE2_HASH:raise ValueError('CREATE2 deployer mismatch')
    artifact,creation_hash,metadata=builder.checked_artifact('CreatorReviewBountyFactoryV1')
    init=artifact_hex(artifact['bytecode'],'CreatorReviewBountyFactoryV1')+address_word(USDC)
    salt=keccak_bytes(b'agent-bounties/base-mainnet/creator-open-factory/v1')
    predicted='0x'+keccak_bytes(b'\xff'+bytes.fromhex(CREATE2[2:])+salt+keccak_bytes(init))[-20:].hex()
    impl=create_address(predicted,1)
    if any(rpc(RPC,'eth_getCode',[a,tag])!='0x'for a in[predicted,impl]):raise ValueError('Proposed address occupied; reconcile before proposing a transaction')
    nonce=rpc(RPC,'eth_getTransactionCount',[ADMIN,'pending']);balance=rpc(RPC,'eth_getBalance',[ADMIN,tag])
    with socket.socket()as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    local=f'http://127.0.0.1:{port}'
    with tempfile.TemporaryDirectory(prefix='posting-mainnet-review-')as directory:
        with open(Path(directory)/'anvil.log','w')as log:
            process=subprocess.Popen([args.anvil,'--silent','--host','127.0.0.1','--port',str(port),'--fork-url',RPC,'--fork-header','User-Agent: agent-bounties-release/1','--fork-block-number',str(int(tag,16)),'--chain-id','8453'],stdout=log,stderr=log,stdin=subprocess.DEVNULL)
            try:
                for _ in range(120):
                    if process.poll()is not None:raise RuntimeError('Local Anvil exited')
                    try:
                        if rpc(local,'eth_chainId',[])=='0x2105':break
                    except(OSError,ValueError):time.sleep(.25)
                else:raise RuntimeError('Local Anvil startup timeout')
                if 'anvil'not in rpc(local,'web3_clientVersion',[]).lower() or rpc(local,'eth_getBlockByNumber',[tag,False])['hash']!=block['hash']:raise ValueError('Wrong local fork')
                rpc(local,'anvil_setBalance',[ADMIN,hex(10**18)]);rpc(local,'anvil_impersonateAccount',[ADMIN])
                tx={'from':ADMIN,'to':CREATE2,'value':'0x0','data':'0x'+(salt+init).hex()}
                estimate=int(rpc(local,'eth_estimateGas',[tx]),16);gas=(estimate*13+9)//10
                if gas>5_000_000:raise ValueError('Deployment exceeds the bounded review ceiling')
                tx['gas']=hex(gas);digest=rpc(local,'eth_sendTransaction',[tx]);receipt=rpc(local,'eth_getTransactionReceipt',[digest])
                if not receipt or receipt['status']!='0x1':raise ValueError('Local deployment failed')
                code=rpc(local,'eth_getCode',[predicted,'latest']);implcode=rpc(local,'eth_getCode',[impl,'latest'])
                for address,name,expected in [(predicted,'implementation',impl),(predicted,'settlementToken',USDC)]:
                    observed=rpc(local,'eth_call',[{'to':address,'data':keccak256((name+'()').encode())[:10]},'latest'])
                    if observed!='0x'+address_word(expected).hex():raise ValueError('Local constructor mismatch')
                lifecycle=rehearse_native_lifecycle(local,predicted)
            finally:
                process.terminate()
                try:process.wait(timeout=5)
                except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
    if rpc(RPC,'eth_getTransactionCount',[ADMIN,'pending'])!=nonce or rpc(RPC,'eth_getCode',[predicted,'latest'])!='0x':raise ValueError('Remote state changed during rehearsal')
    if rpc(RPC,'eth_getBlockByNumber',[tag,False])['hash']!=block['hash']:raise ValueError('Preflight block changed')
    output={'schema':'agent-bounties/posting-mainnet-unsigned-review-v1','observed_at':datetime.now(timezone.utc).isoformat(),'chain_id':8453,
      'source_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'contract_source_hashes':{p:v['keccak256']for p,v in metadata['sources'].items()},
      'safe_block':{'number':int(tag,16),'hash':block['hash']},'reused_dependencies':inventory,'preserved_attester':attester,
      'unsigned_action':{'from':ADMIN,'to':CREATE2,'value_wei':'0','data':tx['data'],'gas_limit':gas,'salt':'0x'+salt.hex(),'init_code_hash':keccak256(init),'artifact_creation_hash':creation_hash,'factory':predicted,'implementation':impl,'factory_runtime_code':code,'factory_runtime_code_hash':keccak256(bytes.fromhex(code[2:])),'implementation_runtime_code':implcode,'implementation_runtime_code_hash':keccak256(bytes.fromhex(implcode[2:]))},
      'fork_rehearsal':{'anvil_version':subprocess.check_output([args.anvil,'--version'],text=True).strip(),'gas_used':int(receipt['gasUsed'],16),'constructors_and_dependencies_verified':True,'full_lifecycle_in_this_probe':True,'lifecycle':lifecycle},
      'admin_observation':{'address':ADMIN,'safe_eth_balance_wei':str(int(balance,16)),'pending_nonce':int(nonce,16)},
      'remote_transactions_submitted':False,'private_keys_read':False,'approval_granted':False,'production_activation_ready':False,
      'remaining_gates':['merged runtime validation','remaining hosted test cases after natural UTC budget reset','fresh total fee quote and bounded signing approval','coordinated budget-aware production release'],
      'evidence_boundary':'Read-only mainnet inventory and local deployment simulation only. The sender is the existing reviewed deployment administrator; no new wallet or production key was created or accessed.'}
    encoded=(json.dumps(output,indent=2)+'\n').encode();args.output.write_bytes(encoded)
    print(json.dumps({'path':str(args.output),'sha256':hashlib.sha256(encoded).hexdigest(),'factory':predicted,'implementation':impl,'gas_limit':gas,'remote_transactions_submitted':False}))


if __name__=='__main__':main()
