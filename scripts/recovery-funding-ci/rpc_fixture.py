"""Loopback-only RPC and DB-connection counter. No upstream RPC or Internet forwarding."""
import http.server,json,os,re,select,socket,socketserver,sys,threading,time
from pathlib import Path

ROOT=Path('/exchange')
LOCK=threading.Lock()
END=time.monotonic()+int(sys.argv[1])

def context():
    data=json.loads((ROOT/'current.json').read_text())
    assert re.fullmatch('[a-z][a-z0-9-]{0,63}',data['name'])
    return data

def event(kind,data):
    case=context();path=ROOT/(case['name']+'-'+kind+'.ndjson')
    with LOCK:
        assert not path.exists() or path.stat().st_size<4*1024**2
        with path.open('a') as f:f.write(json.dumps(data,separators=(',',':'))+'\n')

class Rpc(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_POST(self):
        length=int(self.headers.get('Content-Length','0'));assert 0<length<=65536
        request=json.loads(self.rfile.read(length));assert isinstance(request,dict)
        event('rpc',request)
        mode=context()['rpc_mode'];method=request['method'];params=request.get('params',[])
        oracle=bool(params and isinstance(params[0],dict) and params[0].get('to','').lower()=='0x420000000000000000000000000000000000000f')
        error=None
        if mode=='mined' and method=='eth_getTransactionReceipt':
            result={'transactionHash':params[0],'blockNumber':'0x64','blockHash':'0x'+'aa'*32,'status':'0x1','logs':[]}
        elif mode in ('null','prepare') and method=='eth_getTransactionReceipt':result=None
        elif mode in ('prepare','reject') and method=='eth_chainId':result='0x2105'
        elif mode=='reject' and method=='eth_call':result=None;error={'code':-32000,'message':'synthetic simulation rejected'}
        elif mode=='prepare' and method=='eth_call':result='0x'+f'{10:064x}' if oracle else '0x'
        elif mode=='prepare' and method=='eth_estimateGas':result='0x5208'
        elif mode=='prepare' and method=='eth_feeHistory':result={'oldestBlock':'0x1','baseFeePerGas':['0x1','0x1'],'gasUsedRatio':[.5],'reward':[['0x1']]}
        elif mode=='prepare' and method=='eth_getTransactionCount':result='0x0'
        elif mode=='prepare' and method=='eth_getBalance':result='0x1000000000000000000'
        else:
            event('errors',{'unexpected_method':method,'mode':mode})
            result=None;error={'code':-32601,'message':'fixture rejects this method'}
        response={'jsonrpc':'2.0','id':request.get('id')}
        response['error' if error else 'result']=error or result
        body=json.dumps(response).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)

class Proxy(socketserver.BaseRequestHandler):
    def handle(self):
        event('database',{'connected':True})
        # This single fixed loopback destination is the disposable PG in the same netns.
        with socket.create_connection(('127.0.0.1',5432),timeout=2) as upstream:
            sockets=[self.request,upstream]
            while time.monotonic()<END:
                ready,_,_=select.select(sockets,[],[],.5)
                for source in ready:
                    body=source.recv(65536)
                    if not body:return
                    (upstream if source is self.request else self.request).sendall(body)

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address=True
    daemon_threads=True
    def handle_error(self,request,client_address):
        event('errors',{'sidecar_handler_failed':True})

rpc=http.server.ThreadingHTTPServer(('127.0.0.1',8545),Rpc)
proxy=Server(('127.0.0.1',15432),Proxy)
threading.Thread(target=rpc.serve_forever,daemon=True).start()
threading.Thread(target=proxy.serve_forever,daemon=True).start()
(ROOT/'ready').write_text('ready\n')
while time.monotonic()<END:time.sleep(.1)
os._exit(124)
