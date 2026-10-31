#!/usr/bin/env python3
"""Verify the production posting release without signing or submitting transactions."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import time
import urllib.error
import urllib.request

API = 'https://api.agentizable.ai'
MCP = 'https://mcp.agentizable.ai'
ROOT = Path(__file__).resolve().parents[1]
POSTS = {'/v1/base/sponsored-setup/prepare', '/tools/get_posting_options',
         '/tools/get_posting_status', '/mcp'}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def request(base, path, body=None, headers=None):
    assert base in (API, MCP) and path.startswith('/') and '://' not in path
    assert body is None or path in POSTS
    if path == '/mcp':
        assert body['method'] in ('server/discover', 'tools/list', 'tools/call')
        if body['method'] == 'tools/call':
            assert body['params']['name'] in ('get_posting_options', 'get_posting_status')
    req = urllib.request.Request(base + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Accept':'application/json','Content-Type':'application/json',
                 'Cache-Control':'no-cache','User-Agent':'posting-production-readonly/1', **(headers or {})})
    try:
        response = urllib.request.build_opener(NoRedirect()).open(req, timeout=45)
    except urllib.error.HTTPError as error:
        response = error
    raw = response.read(4_000_001)
    assert len(raw) <= 4_000_000
    try:
        value = json.loads(raw)
    except ValueError:
        value = raw.decode()
    return response.status, value, response.headers


def exercise(revision):
    checks = []
    def check(label, condition):
        if not condition:
            raise AssertionError(label)
        checks.append({'check':label,'passed':True})
    def get(path):
        status, value, headers = request(API, path)
        check(path + ' status', status == 200)
        check(path + ' uncached', 'no-store' in headers.get('Cache-Control',''))
        return value
    for name, base in [('API',API),('MCP',MCP)]:
        status, value, headers = request(base,'/health')
        check(name+' exact release', status == 200 and value == 'ok'
              and headers.get('x-agent-bounties-revision') == revision)
        if name == 'API':
            check('Hosted sponsor enabled', headers.get('x-agent-bounties-x402-relay') == 'enabled')
    gas = get('/v1/base/gas-sponsorship')
    budget = gas['daily_budget']
    check('Shared 1 USDC allowance without rollover', budget['limit_micro_usdc'] == 1_000_000
          and budget['carries_forward'] is False and budget['remaining_micro_usdc'] > 0
          and budget['remaining_micro_usdc'] + budget['reserved_micro_usdc'] == 1_000_000)
    reset = datetime.fromisoformat(budget['resets_at'].replace('Z','+00:00'))
    check('Next UTC midnight reset', reset.utcoffset().total_seconds() == 0
          and (reset.hour,reset.minute,reset.second)==(0,0,0)
          and 0 < (reset-datetime.now(timezone.utc)).total_seconds() <= 86400)
    check('Zero customer gas and no paid fallback', gas['silent_customer_paid_fallback'] is False
          and gas['universal_coverage'] is False
          and all(gas[a]['available'] and gas[a]['customer_gas_wei']=='0' for a in
                  ('creation','creator_verdict','submission','contribution','claim','authorization_revocation')))
    caps = {}
    for protocol, filename, actions in [
        ('creator-open','posting-creator-open-mainnet-release-20261001.json',
         {'create','submit','review','increase','cancel','expire','refund_bond','refund_principal'}),
        ('sponsored-setup','posting-setup-mainnet-release-20261001.json',{'wallet_deploy','participant_register'})]:
        value = get('/v1/base/'+protocol+'/capabilities');caps[protocol]=value
        expected = json.loads((ROOT/'docs/evidence'/filename).read_text())
        mainnet = next(n for n in value['networks'] if n['network']=='base-mainnet')
        check(protocol+' exact active mainnet manifest', mainnet['available'] is True and mainnet['release']==expected)
        check(protocol+' zero gas and bounded authority', value['customer_gas_wei']=='0'
              and value['maximum_authorization_seconds']==7200 and set(value['actions'])==actions)
        check(protocol+' testnet remains off', all(not n['available'] for n in value['networks'] if n['network']!='base-mainnet'))
    for path in ['/v1/site-auth/posting-drafts/00000000-0000-0000-0000-000000000001',
                 '/v1/site-auth/github-origin-drafts','/v1/site-auth/push-notifications']:
        check(path+' private',request(API,path)[0]==401)
    github=get('/v1/github-app/capabilities')
    check('GitHub remains gated without payment authority', github['available'] is False
          and github['funding_authority'] is False and github['provider_writes'] is False)
    for name,base in [('API',API),('MCP',MCP)]:
        status,value,_=request(base,'/.well-known/agent-bounties.json')
        check(name+' canonical discovery',status==200 and value['endpoints']['api_base']==API
              and value['endpoints']['mcp_streamable_http']==MCP+'/mcp')
    for tool,args in [('get_posting_options',{}),('get_posting_status',{'operation_id':'00000000-0000-0000-0000-000000000001'})]:
        status,envelope,_=request(MCP,'/tools/'+tool,args)
        check(tool+' direct tool',status==200 and 'error' not in envelope)
        value=envelope['content'][0]['json']
        if tool=='get_posting_status':
            check('No private approval from discovery',value['private_draft']['status']=='authentication_required' and value['paid'] is False)
        params={'name':tool,'arguments':args,'_meta':{'io.modelcontextprotocol/protocolVersion':'2026-07-28',
                'io.modelcontextprotocol/clientInfo':{'name':'posting-production-readonly','version':'1.0.0'},
                'io.modelcontextprotocol/clientCapabilities':{}}}
        status,envelope,_=request(MCP,'/mcp',{'jsonrpc':'2.0','id':tool,'method':'tools/call','params':params},
                                 {'mcp-protocol-version':'2026-07-28','mcp-method':'tools/call','mcp-name':tool})
        check(tool+' remote tool',status==200 and 'error' not in envelope and envelope['result'].get('isError') is not True)
    # Only prepare a review for an empty wallet. This sends no signature, relay,
    # registration attestation, principal, reservation or chain transaction.
    now=int(time.time());owner='0xc26a630e85134ed30968735c8e7de4576cfa5dbc'
    zero='0x'+'00'*32
    setup={'network':'base-mainnet','owner':owner,'valid_before':now+600,'action':{'kind':'wallet_deploy',
        'user_salt':'0x'+hashlib.sha256(b'posting-mainnet-readonly-smoke-20261001').hexdigest(),
        'policy':{'delegate':owner,'valid_after':now-60,'valid_until':now+3600,'period_seconds':3600,
            'max_per_action_micro_usdc':1,'max_per_period_micro_usdc':1,'max_lifetime_micro_usdc':1,
            'max_bounty_target_micro_usdc':1,'allowed_actions':8,'allowed_verification_modes':2,
            'deterministic_verifier_module':'0x'+'00'*20,'signed_quorum_verifier_set_hash':'0x'+'11'*32,
            'ai_judge_verifier_set_hash':zero}}}
    status,value,_=request(API,'/v1/base/sponsored-setup/prepare',setup)
    check(f'Live setup runtime verification and simulation (HTTP {status})',status==200 and value['prepared']['request']==setup
          and value['customer_gas_wei']=='0' and value['funding_confirmed'] is False and value['solver_paid'] is False)
    after=get('/v1/base/gas-sponsorship')['daily_budget']
    check('Read-only preparation reserved no gas',after==budget)
    return {'observed_at':datetime.now(timezone.utc).isoformat(),'revision':revision,'checks':checks,
            'daily_budget':budget,'capabilities':caps,'setup_preparation':value,
            'chain_transactions_submitted':False,'signatures_created':False}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision',required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    assert len(args.revision)==40 and all(c in '0123456789abcdef' for c in args.revision)
    result=exercise(args.revision);args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(f"PASS {len(result['checks'])} production read-only checks at {args.revision}")
