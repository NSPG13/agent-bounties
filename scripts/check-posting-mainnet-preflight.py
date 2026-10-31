#!/usr/bin/env python3
"""Read-only refresh of the exact unsigned mainnet proposal. Cannot sign or send.

Checks public dependencies, empty CREATE2 addresses, sender nonces and a fresh
fee/FX bound. This is neither approval nor a shared-ledger reservation.
"""
import argparse
from datetime import datetime, timezone
from decimal import Decimal, ROUND_CEILING
import hashlib
import json
from pathlib import Path
import re
import time
import urllib.request
from _shared.evm import keccak256, keccak_bytes, uint_word

ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT/'docs/evidence/posting-mainnet-unsigned-review-20260930.json'
REVIEW_SHA = '7bd435d087309d9208fc9c529d279072ac821d995e2d52b1bdc12b58c2bc9dc8'
RPC = 'https://mainnet.base.org'
ORACLE = '0x420000000000000000000000000000000000000f'


def require(condition, message):
    if not condition:
        raise ValueError(message)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def request(url, body=None):
    require(url in (RPC, 'https://api.coinbase.com/v2/prices/ETH-USDC/spot',
                    'https://api.agentizable.ai/.well-known/x402.json'), 'Unreviewed public endpoint')
    req = urllib.request.Request(url, data=None if body is None else json.dumps(body).encode(),
        headers={'Content-Type':'application/json','Cache-Control':'no-cache','User-Agent':'posting-readonly-preflight/1'})
    with urllib.request.build_opener(NoRedirect()).open(req, timeout=15) as response:
        raw = response.read(2_000_001)
        require(len(raw) <= 2_000_000, 'Oversized public response')
        return json.loads(raw), response.headers


def rpc(method, params):
    require(method in ('eth_chainId','eth_getBlockByNumber','eth_getCode','eth_getBalance',
                       'eth_getTransactionCount','eth_call','eth_estimateGas','eth_maxPriorityFeePerGas'),
            'Only read-only RPC methods are allowed')
    value, _ = request(RPC, {'jsonrpc':'2.0','id':1,'method':method,'params':params})
    require(value.get('id') == 1 and value.get('jsonrpc') == '2.0' and 'error' not in value,
            'Public RPC unavailable: '+method)
    time.sleep(.15)
    return value['result']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    raw = BUNDLE.read_bytes()
    require(hashlib.sha256(raw).hexdigest() == REVIEW_SHA, 'Unsigned review changed')
    bundle = json.loads(raw)
    action = bundle['unsigned_action']
    require(rpc('eth_chainId',[]) == '0x2105' and bundle['chain_id'] == 8453
            and action['value_wei'] == '0', 'Wrong network or call value')
    for name, expected in bundle['contract_source_hashes'].items():
        require(keccak256((ROOT/'contracts/base-escrow'/name).read_bytes()) == expected,
                'Reviewed source changed: '+name)
    head = rpc('eth_getBlockByNumber',['latest',False])
    require(abs(time.time()-int(head['timestamp'],16)) < 180, 'Stale mainnet head')
    tag = {'blockHash':head['hash'],'requireCanonical':True}
    dependencies = []
    for name, item in bundle['reused_dependencies'].items():
        actual = keccak256(bytes.fromhex(rpc('eth_getCode',[item['address'],tag])[2:]))
        require(actual == item['runtime_code_hash'], 'Reviewed runtime changed: '+name)
        dependencies.append({'name':name,'address':item['address'],'runtime_code_hash':actual})
    require(keccak256(bytes.fromhex(rpc('eth_getCode',[action['to'],tag])[2:])) ==
            '0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989', 'CREATE2 deployer changed')
    for address in (action['factory'], action['implementation']):
        require(rpc('eth_getCode',[address,tag]) == '0x', 'Proposed address is no longer empty')
    simulation = {'from':action['from'],'to':action['to'],'data':action['data'],'value':'0x0'}
    rpc('eth_call',[simulation,tag])
    estimate = int(rpc('eth_estimateGas',[simulation]),16)
    require(estimate <= action['gas_limit'], 'Fresh gas estimate exceeds reviewed gas limit')
    priority = int(rpc('eth_maxPriorityFeePerGas',[]),16)
    maximum = 2*int(head['baseFeePerGas'],16)+priority
    require(0 < maximum <= 100_000_000, 'Fee quote exceeds the preflight ceiling')
    def fee(name, amount):
        value = rpc('eth_call',[{'to':ORACLE,'data':'0x'+(keccak_bytes(name.encode())[:4]+uint_word(amount)).hex()},tag])
        require(len(value) == 66, 'Malformed Base fee oracle result')
        return int(value,16)
    l1 = fee('getL1FeeUpperBound(uint256)',len(bytes.fromhex(action['data'][2:]))+256)
    operator = fee('getOperatorFee(uint256)',action['gas_limit'])
    bounded_wei = action['gas_limit']*maximum+2*(l1+operator)
    price, headers = request('https://api.coinbase.com/v2/prices/ETH-USDC/spot')
    amount = price['data']['amount']
    require(price['data']['base'] == 'ETH' and price['data']['currency'] == 'USDC'
            and re.fullmatch(r'\d+(?:\.\d+)?',amount) and len(amount) <= 32
            and int(headers.get('Age','0')) <= 60, 'Fresh ETH/USDC quote unavailable')
    price_micro = int((Decimal(amount)*1_000_000).to_integral_value(rounding=ROUND_CEILING))
    require(0 < price_micro <= 1_000_000_000_000, 'Quote outside runtime bounds')
    reserve_micro = (bounded_wei*price_micro*5+4*10**18-1)//(4*10**18)
    require(0 < reserve_micro <= 1_000_000, 'Proposed deployment alone exceeds one daily allowance')
    discovery, _ = request('https://api.agentizable.ai/.well-known/x402.json')
    relayer = discovery['hostedRelay']['address']
    require(re.fullmatch(r'0x[0-9a-fA-F]{40}',relayer) is not None, 'Invalid public sponsor address')
    senders = []
    for role, address in (('proposed_deployer',action['from']),('current_production_relayer',relayer)):
        senders.append({'role':role,'address':address,'latest_nonce':int(rpc('eth_getTransactionCount',[address,'latest']),16),
                        'pending_nonce':int(rpc('eth_getTransactionCount',[address,'pending']),16),
                        'balance_wei':str(int(rpc('eth_getBalance',[address,'latest']),16))})
    require(int(senders[0]['balance_wei']) >= bounded_wei, 'Proposed deployment sender lacks gas')
    result = {'observed_at':datetime.now(timezone.utc).isoformat(),'chain_id':8453,'unsigned_bundle_sha256':REVIEW_SHA,
              'observed_block':{'number':int(head['number'],16),'hash':head['hash']},'dependencies':dependencies,
              'proposed_addresses_empty':True,'simulation_passed':True,'estimated_gas':estimate,
              'reviewed_gas_limit':action['gas_limit'],'max_fee_per_gas_wei':str(maximum),
              'max_priority_fee_per_gas_wei':str(priority),'maximum_total_fee_wei':str(bounded_wei),
              'fee_bound_includes_execution_l1_operator':True,'eth_usdc_price':amount,
              'proposed_reservation_micro_usdc_with_25_percent_fx_margin':reserve_micro,'senders':senders,
              'daily_ledger_reservation_created':False,'production_cap_active':False,'signing_approval_granted':False,
              'keys_loaded':False,'transactions_submitted':False,'production_changed':False,
              'boundary':'Read-only quote, not signing authority. Requote, reserve in the coordinated production ledger and reconcile pending senders immediately before any approved send.'}
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'fee_bound_wei':str(bounded_wei),'proposed_reserved_micro_usdc':reserve_micro,
                      'signing_approval_granted':False,'transactions_submitted':False}))


if __name__ == '__main__':
    main()
