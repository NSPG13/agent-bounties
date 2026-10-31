#!/usr/bin/env python3
"""Exercise real HTTP/TLS S3 storage and both services with disposable data only.

Requires built api/mcp-server binaries and scripts/requirements-evidence-test.txt.
The PostgreSQL harness creates its own loopback fixture. No live provider, object
store, wallet or production credential is inherited by the services.
"""
from __future__ import annotations

import base64
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import ipaddress
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from urllib.parse import urlsplit
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def exercise(directory: Path, database: str, binaries: dict[str, Path]) -> None:
    import boto3
    from botocore.config import Config
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID
    from moto.server import DomainDispatcherApplication, create_backend_app
    from werkzeug.serving import make_server, WSGIRequestHandler

    now = datetime.now(timezone.utc)
    root_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'Disposable evidence test root')])
    ca = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
          .public_key(root_key.public_key()).serial_number(x509.random_serial_number())
          .not_valid_before(now-timedelta(minutes=1)).not_valid_after(now+timedelta(days=1))
          .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
          .sign(root_key, hashes.SHA256()))
    leaf_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    leaf = (x509.CertificateBuilder().subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, '127.0.0.1')]))
            .issuer_name(name).public_key(leaf_key.public_key()).serial_number(x509.random_serial_number())
            .not_valid_before(now-timedelta(minutes=1)).not_valid_after(now+timedelta(days=1))
            .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
            .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]), critical=False)
            .sign(root_key, hashes.SHA256()))
    pem = ca.public_bytes(serialization.Encoding.PEM)
    (directory/'ca.pem').write_bytes(pem)
    (directory/'server.pem').write_bytes(leaf.public_bytes(serialization.Encoding.PEM))
    (directory/'server.key').write_bytes(leaf_key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))

    class Quiet(WSGIRequestHandler):
        def log(self, *_args, **_kwargs):
            pass
    s3 = make_server('127.0.0.1', 0, DomainDispatcherApplication(create_backend_app), threaded=True,
                     ssl_context=(str(directory/'server.pem'), str(directory/'server.key')), request_handler=Quiet)
    threading.Thread(target=s3.serve_forever, daemon=True).start()
    endpoint = f'https://127.0.0.1:{s3.server_port}'
    client = boto3.client('s3', endpoint_url=endpoint, region_name='us-east-1',
        aws_access_key_id='local-fixture', aws_secret_access_key='local-fixture-secret',
        verify=str(directory/'ca.pem'), config=Config(signature_version='s3v4', proxies={}))
    bucket = 'local-evidence-fixture'
    client.create_bucket(Bucket=bucket)
    provider_calls = []
    class ProviderSpy(BaseHTTPRequestHandler):
        def do_POST(self):
            provider_calls.append(self.path)
            self.send_response(500)
            self.end_headers()
        do_GET = do_POST
        def log_message(self, *_args):
            pass
    spy = ThreadingHTTPServer(('127.0.0.1', 0), ProviderSpy)
    threading.Thread(target=spy.serve_forever, daemon=True).start()

    def local_url():
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            return f'http://127.0.0.1:{listener.getsockname()[1]}'
    api, second_api, mcp, site = [local_url() for _ in range(4)]
    secret = 'local-only-' + uuid.uuid4().hex
    processes, logs, checks = [], [], []
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(base, path, value=None, headers=None):
        encoded = json.dumps(value).encode() if value is not None else None
        req = urllib.request.Request(base+path, data=encoded, headers={'Content-Type':'application/json', **(headers or {})})
        try:
            response = http.open(req, timeout=30)
        except urllib.error.HTTPError as error:
            response = error
        raw = response.read()
        try:
            result = json.loads(raw)
        except ValueError:
            result = raw
        return response.status, result, response.headers

    def check(label, condition):
        if not condition:
            raise AssertionError(label)
        checks.append(label)

    def auth(identity='owner'):
        timestamp = int(time.time())
        user = dict(provider='github', sub='evidence-'+secret+'-'+identity, name='Fixture', email='fixture@example.invalid', avatar='', iat=timestamp, exp=timestamp+1800)
        encoded = json.dumps(user, separators=(',', ':')).encode().hex()
        signature = hmac.new(secret.encode(), encoded.encode(), hashlib.sha256).hexdigest()
        return {'Cookie':f'agent_bounties_session={encoded}.{signature}', 'Origin':site}

    def start(binary, base):
        env = {
            'PATH': os.environ.get('PATH', ''), 'DATABASE_URL': database,
            'PUBLIC_BASE_URL': api, 'MCP_BASE_URL': mcp, 'WEBSITE_BASE_URL': site,
            'SITE_AUTH_ALLOWED_ORIGINS': site, 'MCP_ALLOWED_ORIGINS': site,
            'API_BIND_ADDR': base.removeprefix('http://'), 'MCP_BIND_ADDR': base.removeprefix('http://'),
            'AUTH_SESSION_SECRET': secret, 'AUTH_WALLET_LINK_SECRET': secret,
            'ARTIFACT_S3_ENDPOINT': endpoint, 'ARTIFACT_S3_REGION':'us-east-1', 'ARTIFACT_S3_BUCKET':bucket,
            'ARTIFACT_S3_ACCESS_KEY_ID':'local-fixture', 'ARTIFACT_S3_SECRET_ACCESS_KEY':'local-fixture-secret',
            'ARTIFACT_S3_CA_CERTIFICATE':pem.decode(),
            'ENABLE_BASE_TX_BROADCAST':'false', 'ENABLE_STRIPE_LIVE_EXECUTION':'false', 'ENABLE_X402_HOSTED_RELAY':'false',
            'CLOUD_AGENT_ENABLED':'true', 'CLOUD_AGENT_API_KEY':'fake-test-key',
            'CLOUD_AGENT_ENDPOINT':f'http://127.0.0.1:{spy.server_port}/v1/responses', 'RUST_LOG':'error',
        }
        log = (directory/f'{binary}-{len(processes)}.log').open('w')
        logs.append(log)
        process = subprocess.Popen([str(binaries[binary])], cwd=ROOT, env=env, stdout=log, stderr=log, stdin=subprocess.DEVNULL)
        processes.append(process)
        for _ in range(120):
            if process.poll() is not None:
                raise RuntimeError(f'{binary} failed to start: '+Path(log.name).read_text()[-2000:])
            try:
                if request(base, '/health')[0] == 200:
                    return process
            except (OSError, urllib.error.URLError):
                pass
            time.sleep(.25)
        raise RuntimeError(f'{binary} local startup timed out')

    def stop(process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    try:
        primary = start('api', api)
        secondary = start('api', second_api)
        start('mcp-server', mcp)
        owner, wrong = auth(), auth('unrelated')
        body = {'files':[{'path':name, 'base64':base64.b64encode(content).decode()} for name, content in
            [('code/main.py', b'print(42)'), ('notes.txt', b'Build notes'), ('dataset.csv', b'id,value\na,42\n'), ('design.bin', b'\x00\xffopaque design'), ('preview.html', b'<script>alert(1)</script>')]]}
        headers = {**owner, 'Idempotency-Key':'exact-version'}
        check('anonymous uploads denied before storage', request(api, '/v1/verification/artifacts', body)[0] == 401)
        with ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(lambda base: request(base, '/v1/verification/artifacts', body, headers), [api, second_api]))
        check('concurrent service instances reuse one ready artifact', all(r[0] == 200 and r[1]['status'] == 'ready' for r in results) and results[0][1]['id'] == results[1][1]['id'])
        artifact = results[0][1]
        path = '/v1/verification/artifacts/'+artifact['id']
        check('ready means bytes exist in S3', len(client.list_objects_v2(Bucket=bucket).get('Contents', [])) == len(body['files']))
        changed = {'files':[{'path':'changed.txt','base64':'Y2hhbmdlZA=='}]}
        check('same key cannot replace exact files', request(api, '/v1/verification/artifacts', changed, headers)[0] == 409)
        for caller in [{}, wrong]:
            check('private manifest hidden from unrelated callers', request(api, path, headers=caller)[0] == 404)
            check('private file hidden from unrelated callers', request(api, path+'/files/0', headers=caller)[0] == 404)
        html_index = next(i for i,f in enumerate(artifact['manifest']['files']) if f['path']=='preview.html')
        download = request(api, path+'/files/'+str(html_index), headers=owner)
        check('HTML is downloaded, never served as website code', download[0] == 200 and download[2].get('Content-Disposition') == 'attachment' and download[2].get('Content-Type') == 'application/octet-stream' and download[2].get('X-Content-Type-Options') == 'nosniff' and 'no-store' in download[2].get('Cache-Control',''))
        plan = {'schema_version':'broad-verification/v1','criteria':[{'id':'delivery','text':'Deliver all files','review':'Review whether these solve the brief.'}], 'checks':[{'id':'exact','criteria':['delivery'],'purpose':'required','rule':{'checker':'integrity_v1'}}, {'id':'contents','criteria':['delivery'],'purpose':'required','rule':{'checker':'contents_v1','paths':[f['path'] for f in body['files']],'folders':[]}}]}
        run_body = {'artifact_id':artifact['id'],'plan':plan}
        with ThreadPoolExecutor(max_workers=2) as pool:
            runs = list(pool.map(lambda base: request(base, '/v1/verification/runs', run_body, owner), [api, second_api]))
        check('concurrent checks reuse one run', all(r[0]==200 for r in runs) and runs[0][1]['id']==runs[1][1]['id'])
        run_id = runs[0][1]['id']
        for _ in range(120):
            result = request(api, '/v1/verification/runs/'+run_id, headers=owner)[1]
            if result['status']=='completed':
                break
            time.sleep(.25)
        check('stored bytes pass broad checks but never approve payment', result['status']=='completed' and result['report']['required_checks_passed'] and not result['report']['payment_authorized'] and len(result['report']['review_questions'])==1)
        check('private cached report hidden from another account', request(api, '/v1/verification/runs/'+run_id, headers=wrong)[0] == 404)
        check('private cached report hidden from anonymous readers', request(api, '/v1/verification/runs/'+run_id)[0] == 404)
        check('invalid sessions cannot probe private reports', request(api, '/v1/verification/runs/'+run_id, headers={'Authorization':'Bearer invalid'})[0] == 404)
        for tool, arguments, expected in [('get_verification_artifact', {'id':artifact['id']}, artifact), ('get_verification_run', {'id':run_id}, result)]:
            direct = request(mcp, '/tools/'+tool, arguments, owner)
            check('direct MCP shares authorized '+tool, direct[0]==200 and direct[1].get('content',[{}])[0].get('json')==expected)
            remote = request(mcp, '/mcp', {'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':tool,'arguments':arguments}}, owner)
            check('remote MCP shares authorized '+tool, remote[0]==200 and remote[1].get('result',{}).get('structuredContent')==expected)
        hold = request(api, path+'/disputes', {'reason':'Please retain evidence while I check the report.'}, owner)
        check('dispute saves a hold', hold[0]==200 and request(api,path,headers=owner)[1]['dispute_hold'])
        dispute_id = hold[1]['id']
        check('another account cannot close a dispute', request(api, '/v1/verification/disputes/'+dispute_id+'/resolve', {'reason':'wrong owner'}, wrong)[0]==404)
        check('owner can resolve their own hold', request(api, '/v1/verification/disputes/'+dispute_id+'/resolve', {'reason':'Review finished.'}, owner)[0]==200)
        bad_zip = io.BytesIO()
        with zipfile.ZipFile(bad_zip, 'w') as archive:
            archive.writestr('../escape', b'unsafe')
        unsafe = {'files':[{'path':'work.zip','base64':base64.b64encode(bad_zip.getvalue()).decode()}]}
        check('unsafe archives rejected before storage', request(api,'/v1/verification/artifacts',unsafe,{**owner,'Idempotency-Key':'unsafe'})[0]==400)
        stop(primary)
        stop(secondary)
        primary = start('api', api)
        check('artifact survives service restart', request(api,path,headers=owner)[1]['manifest_hash']==artifact['manifest_hash'])
        check('reports survive service restart', request(api,'/v1/verification/runs/'+run_id,headers=owner)[1]==result)
        original = artifact['manifest']['files'][0]
        key = 'submissions/'+artifact['id']+'/'+original['sha256'].removeprefix('sha256:')
        client.put_object(Bucket=bucket, Key=key, Body=b'x'*original['size'])
        check('changed stored bytes are never returned as original work', request(api,path+'/files/0',headers=owner)[0]==503)
        check('zero outbound model-provider calls, including restart', provider_calls==[])
        print(json.dumps({'passed':len(checks), 'checks':checks}, indent=2))
    finally:
        for process in processes:
            stop(process)
        for log in logs:
            log.close()
        s3.shutdown()
        spy.shutdown()


def main() -> int:
    parser=argparse.ArgumentParser(description=__doc__)
    suffix='.exe' if os.name=='nt' else ''
    parser.add_argument('--api-binary',type=Path,default=ROOT/'target/debug'/('api'+suffix))
    parser.add_argument('--mcp-binary',type=Path,default=ROOT/'target/debug'/('mcp-server'+suffix))
    args=parser.parse_args()
    binaries={'api':args.api_binary.resolve(),'mcp-server':args.mcp_binary.resolve()}
    if any(not path.is_file() for path in binaries.values()):
        parser.error('Build both services first, or give paths to local production-container launchers.')
    if os.environ.get('AGENT_BOUNTIES_LOCAL_TEST_DATABASE') != '1':
        return subprocess.call([sys.executable,str(ROOT/'scripts/run-with-test-postgres.py'),'--',sys.executable,str(Path(__file__).resolve()),*sys.argv[1:]], cwd=ROOT)
    database = os.environ.get('DATABASE_URL','')
    parsed = urlsplit(database)
    if parsed.scheme not in ('postgres','postgresql') or parsed.hostname not in ('127.0.0.1','localhost') or parsed.username!='agent_bounties_test' or parsed.path!='/agent_bounties_test' or parsed.query or parsed.fragment:
        raise RuntimeError('Evidence tests require the disposable loopback PostgreSQL fixture')
    with tempfile.TemporaryDirectory(prefix='broad-evidence-test-') as directory:
        exercise(Path(directory), database, binaries)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
