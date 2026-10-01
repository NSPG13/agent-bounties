"""One attended, reviewed Base deployment. No replacement or automatic retry."""
from datetime import datetime, timezone
from decimal import Decimal, ROUND_CEILING
import hashlib
import json
import os
from pathlib import Path
import sys
import time
import urllib.request

from eth_account import Account
from eth_account.typed_transactions import TypedTransaction
from eth_utils import keccak
from hexbytes import HexBytes

ROOT = Path(__file__).resolve().parents[1]
CALL_FILE = ROOT / 'ops/posting-approved-mainnet-call-20261001.json'
CALL_SHA = 'ef19d8f52a35c9d51b7b68f9cbd466da89fc0ff3fb14353e4f6a02ce389e56e2'
REVIEW_SHA = '7bd435d087309d9208fc9c529d279072ac821d995e2d52b1bdc12b58c2bc9dc8'
REVISION = 'e6bd24bac27c8fb5e77fad3fe91fdb3283d4288e'
SENDER = '0x884834e884d6e93462655a2820140ad03e6747bc'
TARGET = '0x4e59b44847b379578588920ca78fbf26c0b4956c'
RPC = 'https://mainnet.base.org'
API = 'https://api.agentbounties.app'
PRICE = 'https://api.coinbase.com/v2/prices/ETH-USDC/spot'
ORACLE = '0x420000000000000000000000000000000000000f'
JOURNAL = Path('posting-mainnet-outbox.json')


def require(condition, reason):
    if not condition:
        raise RuntimeError(reason)


def digest(raw):
    return '0x' + keccak(raw).hex()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def request(url, body=None, token=None):
    require(url in (RPC, PRICE, API + '/health', API + '/v1/base/gas-sponsorship',
                    API + '/v1/operator/gas-sponsorship/reservations'), 'Endpoint outside review')
    require(token is None or url == API + '/v1/operator/gas-sponsorship/reservations',
            'Credential destination outside review')
    headers = {'Content-Type': 'application/json', 'Cache-Control': 'no-cache',
               'User-Agent': 'posting-approved-mainnet/1'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(url, data=None if body is None else json.dumps(body).encode(), headers=headers)
    with urllib.request.build_opener(NoRedirect()).open(req, timeout=10) as response:
        raw = response.read(2_000_001)
        require(len(raw) <= 2_000_000, 'Oversized response')
        return (raw.decode() if url.endswith('/health') else json.loads(raw)), response.headers


def rpc(method, params, *, allow_send=False):
    reads = ('eth_chainId', 'eth_getBlockByNumber', 'eth_getCode', 'eth_getBalance',
             'eth_getTransactionCount', 'eth_call', 'eth_estimateGas',
             'eth_maxPriorityFeePerGas', 'eth_getTransactionReceipt', 'eth_getTransactionByHash')
    require(method in reads or (allow_send and method == 'eth_sendRawTransaction'), 'RPC write forbidden')
    value, _ = request(RPC, {'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params})
    require(value.get('id') == 1 and value.get('jsonrpc') == '2.0' and 'error' not in value,
            'RPC unavailable: ' + method)
    return value['result']


def save(journal):
    raw = (json.dumps(journal, indent=2) + '\n').encode()
    temporary = JOURNAL.with_suffix('.tmp')
    with os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'wb') as file:
        file.write(raw)
        file.flush()
        os.fsync(file.fileno())
    os.replace(temporary, JOURNAL)


def reviewed_call():
    raw = CALL_FILE.read_bytes()
    require(hashlib.sha256(raw).hexdigest() == CALL_SHA, 'Reviewed call projection changed')
    bundle = json.loads(raw)
    action = bundle['unsigned_action']
    require(bundle['approved_review_sha256'] == REVIEW_SHA and bundle['chain_id'] == 8453
            and bundle['expected_nonce'] == 82 and action['from'] == SENDER and action['to'] == TARGET
            and action['value_wei'] == '0' and action['gas_limit'] == 4_233_944
            and bundle['maximum_reservation_micro_usdc'] == 200_000, 'Approved scope changed')
    return bundle, action


def fresh_head():
    require(rpc('eth_chainId', []) == '0x2105', 'Wrong chain')
    head = rpc('eth_getBlockByNumber', ['latest', False])
    require(abs(time.time() - int(head['timestamp'], 16)) < 120, 'Stale mainnet head')
    return head


def dependencies(bundle, action, head, empty=True):
    tag = {'blockHash': head['hash'], 'requireCanonical': True}
    for name, expected in bundle['reused_dependencies'].items():
        code = rpc('eth_getCode', [expected['address'], tag])
        require(digest(bytes.fromhex(code[2:])) == expected['runtime_code_hash'], 'Dependency changed: ' + name)
    require(digest(bytes.fromhex(rpc('eth_getCode', [TARGET, tag])[2:])) ==
            '0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989', 'CREATE2 deployer changed')
    if empty:
        for address in (action['factory'], action['implementation']):
            require(rpc('eth_getCode', [address, tag]) == '0x', 'Deployment exists; reconcile without sending')


def oracle_fee(name, value, tag):
    data = '0x' + (keccak(text=name)[:4] + value.to_bytes(32, 'big')).hex()
    result = rpc('eth_call', [{'to': ORACLE, 'data': data}, tag])
    require(isinstance(result, str) and len(result) == 66, 'Invalid fee oracle response')
    return int(result, 16)


def fee_bound(action, fee, head):
    tag = {'blockHash': head['hash'], 'requireCanonical': True}
    l1 = oracle_fee('getL1FeeUpperBound(uint256)', len(bytes.fromhex(action['data'][2:])) + 256, tag)
    operator = oracle_fee('getOperatorFee(uint256)', action['gas_limit'], tag)
    return action['gas_limit'] * fee + 2 * (l1 + operator)


def fresh_price_bound(wei):
    quote, headers = request(PRICE)
    require(quote['data']['base'] == 'ETH' and quote['data']['currency'] == 'USDC'
            and int(headers.get('Age', '0')) <= 60, 'Fresh price unavailable')
    price = Decimal(quote['data']['amount'])
    require(price.is_finite() and 0 < price <= 1_000_000, 'Invalid price')
    micro = int((price * 1_000_000).to_integral_value(rounding=ROUND_CEILING))
    return (wei * micro * 5 + 4 * 10**18 - 1) // (4 * 10**18)


def budget():
    health, headers = request(API + '/health')
    require(health == 'ok' and headers.get('x-agent-bounties-revision') == REVISION, 'Budget runtime differs')
    readiness, _ = request(API + '/v1/base/gas-sponsorship')
    require(readiness['creation']['available'] is False, 'Customer sends must remain paused')
    value = readiness['daily_budget']
    require(value['limit_micro_usdc'] == 1_000_000 and value['carries_forward'] is False,
            'Shared daily allowance differs')
    return value


def validate_record(record, action):
    raw = HexBytes(record['raw'])
    require(digest(raw) == record['hash'] and Account.recover_transaction(raw).lower() == SENDER,
            'Saved signature differs')
    tx = TypedTransaction.from_bytes(raw).as_dict()
    require(tx['chainId'] == 8453 and tx['nonce'] == 82 and tx['value'] == 0 and
            tx['gas'] == action['gas_limit'] and tx['type'] == 2 and not tx['accessList'] and
            bytes(tx['to']).hex() == TARGET[2:] and bytes(tx['data']).hex() == action['data'][2:] and
            tx['maxFeePerGas'] == record['fee_cap'] and tx['maxPriorityFeePerGas'] == record['priority'] and
            0 <= tx['maxPriorityFeePerGas'] <= 1_000_000 and 0 < tx['maxFeePerGas'] <= 100_000_000,
            'Saved transaction scope differs')
    return tx


def prepare():
    require(not JOURNAL.exists(), 'Existing outbox must be reconciled')
    bundle, action = reviewed_call()
    require(os.environ.get('POSTING_APPROVED_REVIEW_SHA256') == REVIEW_SHA and
            os.environ.get('GITHUB_RUN_ATTEMPT') == '1', 'Exact attended approval required; no reruns')
    signer = Account.from_key(os.environ['BASE_KEEPER_PRIVATE_KEY'])
    require(signer.address.lower() == SENDER, 'Configured signer is not the approved sender')
    head = fresh_head()
    dependencies(bundle, action, head)
    for kind in ('latest', 'pending'):
        require(int(rpc('eth_getTransactionCount', [SENDER, kind]), 16) == 82, 'Sender nonce changed')
    call = {'from': SENDER, 'to': TARGET, 'data': action['data'], 'value': '0x0'}
    rpc('eth_call', [call, 'latest'])
    require(int(rpc('eth_estimateGas', [call]), 16) <= action['gas_limit'], 'Gas estimate exceeds approval')
    priority = min(int(rpc('eth_maxPriorityFeePerGas', []), 16), 1_000_000)
    fee = max(1_000_000, 2 * int(head['baseFeePerGas'], 16) + priority)
    require(fee <= 100_000_000, 'Fee ceiling exceeded')
    maximum = fee_bound(action, fee, head)
    require(0 < maximum <= 100_000_000_000_000 and
            0 < fresh_price_bound(maximum) <= 200_000, 'Fee exceeds approved 0.20 USDC ceiling')
    require(int(rpc('eth_getBalance', [SENDER, 'latest']), 16) >= maximum, 'Deployer balance too low')
    before = budget()
    require(before['remaining_micro_usdc'] >= 200_000, 'Insufficient shared daily capacity')
    tx = {'chainId': 8453, 'nonce': 82, 'to': HexBytes(TARGET), 'data': action['data'],
          'value': 0, 'gas': action['gas_limit'], 'type': 2, 'accessList': [],
          'maxFeePerGas': fee, 'maxPriorityFeePerGas': priority}
    scope = {**tx, 'to': TARGET}
    intent = {'network': 'base-mainnet', 'relayer_address': SENDER,
              'transaction_digest': hashlib.sha256(json.dumps(scope, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
              'maximum_fee_wei': str(maximum)}
    journal = {'review_sha256': REVIEW_SHA, 'projection_sha256': CALL_SHA,
               'reservation_attempted': True, 'reservation_request': intent, 'budget_before': before,
               'run_id': os.environ['GITHUB_RUN_ID'], 'record': None, 'broadcast_attempted': False}
    save(journal)
    ticket, _ = request(API + '/v1/operator/gas-sponsorship/reservations', intent,
                        os.environ['GAS_SPONSOR_BUDGET_TOKEN'])
    journal['reservation'] = ticket
    save(journal)
    require(ticket.get('reserved') is True and ticket.get('sender_nonce') == 82 and
            all(ticket.get(k) == v for k, v in intent.items()) and ticket.get('shared_signer_lease') is True and
            ticket.get('carries_forward') is False and ticket.get('daily_limit_micro_usdc') == 1_000_000 and
            ticket.get('transaction_submitted') is False and
            time.time() + 3 < ticket['valid_until'] <= time.time() + 35 and
            ticket['signer_lease_expires_at'] == ticket['valid_until'] + 15, 'Reservation differs or expired')
    after = budget()
    reserved = after['reserved_micro_usdc'] - before['reserved_micro_usdc']
    journal['budget_after'] = after
    journal['reserved_micro_usdc'] = reserved
    save(journal)
    require(after['resets_at'] == before['resets_at'] and 0 < reserved <= 200_000,
            'Observed reservation exceeds approval; do not sign')
    signed = signer.sign_transaction(tx)
    record = {'raw': '0x' + signed.raw_transaction.hex(), 'hash': digest(signed.raw_transaction),
              'reserved_wei': maximum, 'fee_cap': fee, 'priority': priority}
    validate_record(record, action)
    journal['record'] = record
    save(journal)
    print(json.dumps({'prepared_transaction': record['hash'], 'reserved_micro_usdc': reserved,
                      'broadcast': False, 'review_sha256': REVIEW_SHA}), flush=True)


def reconcile(journal, action):
    record = journal['record']
    validate_record(record, action)
    receipt = rpc('eth_getTransactionReceipt', [record['hash']])
    if receipt is None:
        return False
    block = rpc('eth_getBlockByNumber', [receipt['blockNumber'], False])
    require(block['hash'].lower() == receipt['blockHash'].lower() and receipt['status'] == '0x1'
            and receipt['from'].lower() == SENDER and receipt['to'].lower() == TARGET
            and receipt['transactionHash'].lower() == record['hash'], 'Receipt is not the approved successful transaction')
    tag = {'blockHash': block['hash'], 'requireCanonical': True}
    for kind in ('factory', 'implementation'):
        require(rpc('eth_getCode', [action[kind], tag]).lower() == action[kind + '_runtime_code'].lower(),
                'Deployed runtime differs: ' + kind)
    actual = int(receipt['gasUsed'], 16) * int(receipt['effectiveGasPrice'], 16) + int(receipt['l1Fee'], 16)
    actual += oracle_fee('getOperatorFee(uint256)', int(receipt['gasUsed'], 16), tag)
    require(actual <= record['reserved_wei'], 'Receipt fee exceeds reservation')
    journal['receipt'] = {'transaction_hash': record['hash'], 'block_number': int(receipt['blockNumber'], 16),
                          'block_hash': block['hash'], 'fee_wei': str(actual), 'runtime_bytes_verified': True,
                          'factory': action['factory'], 'implementation': action['implementation']}
    save(journal)
    print(json.dumps(journal['receipt']), flush=True)
    return True


def broadcast():
    bundle, action = reviewed_call()
    journal = json.loads(JOURNAL.read_text())
    require(journal['review_sha256'] == REVIEW_SHA and journal['run_id'] == os.environ['GITHUB_RUN_ID']
            and os.environ.get('POSTING_DURABLE_ARTIFACT_ID', '').isdigit(), 'Durable outbox not acknowledged')
    validate_record(journal['record'], action)
    if reconcile(journal, action):
        return
    require(not journal['broadcast_attempted'], 'Unknown prior send; reconcile without retry')
    head = fresh_head()
    require(int(rpc('eth_getTransactionCount', [SENDER, 'pending']), 16) == 82, 'Reserved nonce changed')
    execution = action['gas_limit'] * journal['record']['fee_cap']
    fresh_total = execution + (fee_bound(action, journal['record']['fee_cap'], head) - execution) // 2
    require(fresh_total <= journal['record']['reserved_wei']
            and int(head['baseFeePerGas'], 16) + journal['record']['priority'] <= journal['record']['fee_cap'],
            'Fees changed; preserve exact signed transaction')
    require(time.time() + 1 < journal['reservation']['valid_until'], 'Short send lease expired; preserve outbox')
    journal['broadcast_attempted'] = True
    save(journal)
    sent = rpc('eth_sendRawTransaction', [journal['record']['raw']], allow_send=True)
    require(sent.lower() == journal['record']['hash'], 'Unknown send response; no retries')
    for _ in range(15):
        if reconcile(journal, action):
            return
        time.sleep(2)
    raise RuntimeError('Receipt pending; reconcile the recorded hash without another send')


if __name__ == '__main__':
    try:
        require(sys.argv[1] in ('prepare', 'broadcast'), 'Unknown mode')
        {'prepare': prepare, 'broadcast': broadcast}[sys.argv[1]]()
    except Exception as error:
        # Exception bodies may contain provider details; keep logs scope-only.
        print('Approved deployment stopped: ' + type(error).__name__ +
              (': ' + str(error) if isinstance(error, RuntimeError) else ''), file=sys.stderr)
        sys.exit(1)
