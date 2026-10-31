#!/usr/bin/env python3
"""Read-only canonical/finality audit of the fixed October 1 Sepolia fixtures.

No private key, signature or credential is loaded. Public evidence supplies the
exact expected receipts. Optional finality enforcement never changes chain time.
"""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location('hosted', Path(__file__).with_name('check-posting-hosted-sepolia.py'))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
require = h.ex.require
EVIDENCE = h.ex.ROOT / 'docs/evidence'
ACCOUNT = '0xf81e2ec972d570694e64c4583b59c4fc9375505c'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--require-finalized', action='store_true')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    load = lambda name: json.loads((EVIDENCE/name).read_text())
    account = load('posting-sepolia-account-deployment-20261001.json')
    edges = load('posting-hosted-open-edges-20261001.json')
    wallet = load('posting-hosted-contract-wallet-20261001.json')
    require(edges['complete'] and wallet['principal_recovered'], 'Rehearsal closeout incomplete')
    require(len(edges['actions']) == 10 and len(wallet['actions']) == 10 and len(wallet['fixture_calls']) == 3,
            'Unexpected fixed rehearsal scope')
    rows = [{'name':'account-deployment', 'receipt':account['receipt']}, *edges['actions'],
            *wallet['actions'], *wallet['fixture_calls']]
    require(len({r['receipt']['transaction_hash'] for r in rows}) == 24, 'Duplicate or missing transaction')
    rpc = h.ex.Rpc()
    latest = h.ex.fresh_head(rpc)
    finalized = rpc('eth_getBlockByNumber', ['finalized', False])
    safe = rpc('eth_getBlockByNumber', ['safe', False])
    before = h.get('/v1/base/gas-sponsorship')['daily_budget']
    nonce = rpc('eth_getTransactionCount', [h.funding.ADDRESSES['relayer'], 'pending'])
    balances = h.balances(rpc)
    results = []
    for row in rows:
        expected = row['receipt']
        digest = expected['transaction_hash']
        receipt = rpc('eth_getTransactionReceipt', [digest])
        require(receipt is not None and receipt['transactionHash'].lower() == digest
                and int(receipt['status'], 16) == 1, 'Missing or failed exact receipt')
        number = int(receipt['blockNumber'], 16)
        block = rpc('eth_getBlockByNumber', [hex(number), False])
        require(number == expected['block_number'] and receipt['blockHash'].lower() == expected['block_hash']
                and block['hash'].lower() == expected['block_hash'], 'Canonical receipt changed')
        require(int(latest['number'], 16) >= number + 3, 'Insufficient canonical confirmations')
        sender = h.funding.ADDRESSES['deployer' if row['name'] == 'account-deployment' else 'relayer']
        require(receipt['from'].lower() == sender and int(receipt['gasUsed'],16) == expected['gas_used'],
                'Unexpected fee payer or gas use')
        fee = int(receipt['gasUsed'],16)*int(receipt['effectiveGasPrice'],16)+int(receipt['l1Fee'],16)
        fee += h.ex.oracle_fee(rpc, 'getOperatorFee(uint256)', int(receipt['gasUsed'],16),
                              {'blockHash':block['hash'],'requireCanonical':True})
        is_finalized = number <= int(finalized['number'], 16)
        require(not args.require_finalized or is_finalized, 'Receipt not finalized yet; wait without another send')
        results.append({'name':row['name'], 'transaction_hash':digest, 'block_number':number,
                        'block_hash':block['hash'], 'safe':number <= int(safe['number'],16),
                        'finalized':is_finalized, 'actual_fee_wei':str(fee)})
    blocked = wallet['creator_open_quota_blocked']
    require(int(finalized['timestamp'],16) > blocked['authorization_valid_before'],
            'Blocked authority has not expired at finalized chain time')
    tag = {'blockHash':finalized['hash'],'requireCanonical':True}
    require(rpc('eth_getCode',[blocked['bounty'],tag]) == '0x'
            and rpc('eth_getCode',[blocked['bounty'],'latest']) == '0x', 'Quota-blocked contract unexpectedly exists')
    status, _, _ = h.request('/v1/base/creator-open/status',
                            {'network':'base-sepolia','approval_hash':blocked['approval_hash']})
    require(status == 404, 'Quota-blocked request unexpectedly has an operation')
    code = rpc('eth_getCode',[ACCOUNT,tag])
    require(h.keccak256(bytes.fromhex(code[2:])) == account['receipt']['runtime_code_hash'],
            'Approved test account runtime changed')
    require(int(rpc('eth_getBalance',[ACCOUNT,'latest']),16) == 0, 'Contract customer acquired ETH')
    call = '0x'+(h.keccak_bytes(b'balanceOf(address)')[:4]+h.address_word(ACCOUNT)).hex()
    require(int(rpc('eth_call',[{'to':h.funding.USDC,'data':call},'latest']),16) == 0,
            'Test account principal not recovered')
    after = h.get('/v1/base/gas-sponsorship')['daily_budget']
    require(after == before and rpc('eth_getTransactionCount',[h.funding.ADDRESSES['relayer'],'pending']) == nonce
            and h.balances(rpc) == balances, 'Read-only audit changed or raced financial state')
    result = {'observed_at':datetime.now(timezone.utc).isoformat(),'network':'base-sepolia',
              'finalized_block':{'number':int(finalized['number'],16),'hash':finalized['hash'],
                                 'timestamp':int(finalized['timestamp'],16)},'transactions':results,
              'all_finalized':all(r['finalized'] for r in results),
              'actual_total_test_fee_wei':str(sum(int(r['actual_fee_wei']) for r in results)),
              'blocked_authority_expired_at_finalized_block':True,'blocked_contract_absent':True,
              'account_runtime_exact':True,'account_usdc_and_eth_zero':True,'customer_balances':balances,
              'budget_unchanged':after,'sponsor_nonce_unchanged':int(nonce,16),'keys_loaded':False,
              'transactions_submitted':False,'production_changed':False}
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'transactions':len(results),'all_finalized':result['all_finalized'],
                      'total_test_fee_wei':result['actual_total_test_fee_wei']}))


if __name__ == '__main__':
    main()
