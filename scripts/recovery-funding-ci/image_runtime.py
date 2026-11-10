"""Actual image replay of exported synthetic process fixtures. No production resources."""
import hashlib,importlib.util,json,os,re,subprocess,time,urllib.parse
from pathlib import Path
import image_build as build

PG='sha256:ba07bdb15f0f2ac1ba7a88891da731cae65ac36d663fdb7de8f698ea47e0e381'
PYTHON='sha256:78976a69c067520ef8be34cde307ef9f6fea6dcefa50edb57039773d241ca0db'
PREFIX='ab-funding-image-fixture-'
WATCHDOG='ab-funding-image-watchdog-20261110.service'
INSPECTOR=Path(__file__).resolve().parent/'inspector'
SETTINGS=json.dumps({'network':'base-mainnet','funding_wallet':'0x'+'f0'*20,'min_paid_age_seconds':0,'deadline_margin_seconds':3600,'max_gas':1500000,'max_fee_per_gas_wei':2000000000,'lease_seconds':180,'poll_seconds':300},separators=(',',':'),sort_keys=True)
V2={'BASE_MAINNET_BOUNTY_V2_FACTORY':'0xc33e2ae33bb9580837ea59df18e57fa1039ae58a','BASE_MAINNET_BOUNTY_V2_IMPLEMENTATION':'0x8420c9bd1ff8a6b1abc4230b349538612c58f3a6','BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_BPS':'750','BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_RECIPIENT':'0x884834e884d6e93462655a2820140ad03e6747bc'}
PROPS=('LoadState','ActiveState','SubState','MainPID','Job','InvocationID','ControlGroup','Description')

def record(ctx,name,obj):build.record(ctx.here,name,obj)

def current(ctx,name,mode):
    raw=json.dumps({'name':name,'rpc_mode':mode})
    (ctx.exchange/'current.tmp').write_text(raw);os.replace(ctx.exchange/'current.tmp',ctx.exchange/'current.json')

def lines(ctx,name,kind):
    p=ctx.exchange/(name+'-'+kind+'.ndjson')
    if not p.exists():return []
    assert p.stat().st_size<4*1024**2
    return [json.loads(x) for x in p.read_text().splitlines()]

def no_connections(ctx,name):
    assert not lines(ctx,name,'rpc') and not lines(ctx,name,'database') and not lines(ctx,name,'errors')

def describe(ctx,cid):
    obj=json.loads(ctx.small(['docker','inspect',cid]).stdout)[0]
    assert obj['Id']==cid and obj['Config']['Labels'].get('ab.recovery.owner')==ctx.attempt['nonce']
    assert obj['Config']['Labels'].get('ab.recovery.role')=='funding-image-fixture'
    return obj

def remove(ctx,cid):
    obj=describe(ctx,cid)
    assert obj['Name'] in ['/'+PREFIX+x for x in ['pg','rpc','worker']]
    ctx.small(['docker','rm','--force','--volumes',cid],timeout=10)
    assert not ctx.small(['docker','ps','-aq','--no-trunc','--filter','id='+cid]).stdout.strip()

def create(ctx,role,image,env,command=(),entrypoint=None):
    limits={'pg':(.3,512*1024**2,64),'rpc':(.2,128*1024**2,32),'worker':(1.35,1216*1024**2,96)}
    cpu,memory,pids=limits[role];name=PREFIX+role
    assert not ctx.small(['docker','ps','-aq','--filter','name=^/'+name+'$']).stdout.strip()
    args=['docker','create','--pull=never','--name',name,'--label','ab.recovery.owner='+ctx.attempt['nonce'],
        '--label','ab.recovery.role=funding-image-fixture','--cpus',str(cpu),'--memory',str(memory),'--memory-swap',str(memory),
        '--pids-limit',str(pids),'--read-only','--cap-drop=ALL','--security-opt=no-new-privileges','--restart=no',
        '--log-driver=local','--log-opt','max-size=1m','--log-opt','max-file=1','--log-opt','compress=false','--tmpfs','/tmp:rw,nosuid,nodev,size=16m']
    if role=='pg':args+=['--network=none','--user=postgres',
        '--tmpfs','/var/lib/postgresql/data:rw,nosuid,nodev,noexec,size=256m,mode=1777',
        '--tmpfs','/var/run/postgresql:rw,nosuid,nodev,noexec,size=16m,mode=1777']
    else:args+=['--network','container:'+ctx.pg]
    if role=='rpc':args+=['--user','1000:1000','--mount',f'type=bind,src={ctx.exchange},dst=/exchange',
        '--mount',f'type=bind,src={ctx.helpers / "rpc_fixture.py"},dst=/rpc_fixture.py,readonly']
    if role=='worker':args+=['--mount',f'type=bind,src={ctx.target / "inspector"},dst=/inspect,readonly',
        '--mount',f'type=bind,src={ctx.target / "policy"},dst=/fixture,readonly']
    for key,value in env.items():
        assert isinstance(value,str) and '\n' not in value and '\0' not in value
        args+=['--env',key+'='+value]
    if entrypoint:args+=['--entrypoint',entrypoint]
    args+=[image,*command]
    with (ctx.here/'runtime-attempts.jsonl').open('a') as f:f.write(json.dumps({'role':role,'name':name,'image':image,'at':time.monotonic()})+'\n')
    cid=ctx.small(args,timeout=10).stdout.strip();assert re.fullmatch('[0-9a-f]{64}',cid)
    ctx.runtime_ids.append(cid)
    ctx.small(['docker','start',cid],timeout=5)
    obj=describe(ctx,cid);host=obj['HostConfig']
    assert obj['Image']==image and host['NanoCpus']==int(cpu*1e9) and host['Memory']==host['MemorySwap']==memory and host['PidsLimit']==pids
    assert host['ReadonlyRootfs'] and host['CapDrop']==['ALL'] and host['SecurityOpt']==['no-new-privileges'] and host['RestartPolicy']['Name']=='no'
    assert host['NetworkMode']==('none' if role=='pg' else 'container:'+ctx.pg) and not host['PortBindings']
    assert not any(m['Type']=='volume' or m['Destination']=='/var/run/docker.sock' for m in obj['Mounts'])
    if role=='pg':
        for mount,expected in [('/var/lib/postgresql/data',256*1024**2),('/var/run/postgresql',16*1024**2),('/tmp',16*1024**2)]:
            kind,block,count=ctx.small(['docker','exec',cid,'stat','-f','-c','%T %S %b',mount]).stdout.split()
            assert kind=='tmpfs' and int(block)*int(count)==expected
    if obj['State']['Running']:
        pid=obj['State']['Pid'];group=Path('/sys/fs/cgroup')/Path(f'/proc/{pid}/cgroup').read_text().strip().split('::',1)[1].lstrip('/')
        assert cid in group.name
        controls={k:(group/k).read_text().strip() for k in ('cpu.max','memory.max','memory.swap.max','pids.max')}
        quota,period=map(int,controls['cpu.max'].split())
        assert quota/period==cpu and int(controls['memory.max'])==memory and controls['memory.swap.max']=='0' and int(controls['pids.max'])==pids
        st=group.stat();ctx.runtime_groups[cid]={'path':str(group),'identity':[st.st_dev,st.st_ino],'controls':controls}
        with (ctx.here/'runtime-identities.jsonl').open('a') as f:f.write(json.dumps({'id':cid,'observation':ctx.runtime_groups[cid]})+'\n')
    return cid

def worker_env(ctx,schema,settings=SETTINGS,revision=build.FUNDING_REVISION):
    return dict(V2,RENDER_SERVICE_ID=build.SERVICE,RENDER_GIT_COMMIT=revision,
        RECOVERY_FUNDING_MODE='active',ENABLE_HOSTED_FINANCIAL_SIGNING='true',BASE_INDEXER_PROTOCOL='managed-bounty-funding',RUST_LOG='info',
        DATABASE_URL='postgres://agent_bounties_test:fixture-only@127.0.0.1:15432/agent_bounties_test?'+urllib.parse.urlencode({'options':'-csearch_path='+schema}),
        BASE_MAINNET_RPC_URL='http://127.0.0.1:8545',MANAGED_BOUNTY_FUNDING=settings,MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY=f'{1:064x}')

def wait_exit(ctx,cid,deadline):
    while time.monotonic()<deadline:
        obj=describe(ctx,cid)
        if not obj['State']['Running']:
            output=ctx.small(['docker','logs',cid]).stdout
            error=ctx.small(['docker','logs',cid]).stderr
            return obj['State']['ExitCode'],output+error
        time.sleep(.05)
    raise TimeoutError('fixture process exit deadline')

def sql(ctx,statement,schema=None,script=None):
    assert schema is None or re.fullmatch('recovery_funding_[a-z_]+',schema)
    prefix='SET search_path TO '+schema+';\n' if schema else ''
    if script is not None:
        assert script.stat().st_size<=16*1024**2
        body=prefix.encode()+script.read_bytes()
    else:body=(prefix+statement).encode()
    result=subprocess.run(['docker','exec','-i',ctx.pg,'psql','-X','-q','-A','-t','-v','ON_ERROR_STOP=1','-U','agent_bounties_test','-d','agent_bounties_test'],input=body,capture_output=True,timeout=ctx.remaining(10))
    assert result.returncode==0 and len(result.stdout)<=1024*1024 and len(result.stderr)<=65536,(result.stderr[:200]).decode(errors='replace')
    return result.stdout.decode().strip()

def snapshot(ctx,schema):
    statement="""CREATE FUNCTION pg_temp.fixture_snapshot(s text) RETURNS jsonb LANGUAGE plpgsql AS $$
    DECLARE t record; value text; result jsonb := '{}'::jsonb; n integer := 0;
    BEGIN
      FOR t IN SELECT tablename FROM pg_tables WHERE schemaname=s ORDER BY tablename LOOP
        n := n+1; IF n>400 THEN RAISE EXCEPTION 'fixture table cap'; END IF;
        EXECUTE format('SELECT md5(COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text)::text,''[]'')) FROM %I.%I t',s,t.tablename) INTO value;
        result := result || jsonb_build_object(t.tablename,value);
      END LOOP; RETURN result;
    END $$; SELECT pg_temp.fixture_snapshot('"""+schema+"');"
    return json.loads(sql(ctx,statement))

def actual_policy(ctx):
    spec=importlib.util.spec_from_file_location('funding_policy',INSPECTOR/'compile_policy.py');module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    data={'schema':'agentizable/funding-process-inspection-policy-v1','profile':'funding-active','service_id':build.SERVICE,'revision':build.FUNDING_REVISION,'branch':'main','service_name':'fixture-funding','service_type':'worker',
        'settings':dict(V2,DATABASE_URL='@present',BASE_MAINNET_RPC_URL='@present',MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY='@private_scalar',MANAGED_BOUNTY_FUNDING=SETTINGS)}
    policy=module.compile_policy(data);(ctx.target/'policy/policy.tsv').write_text(policy);(ctx.target/'policy/policy.tsv').chmod(0o444)
    return hashlib.sha256(policy.encode()).hexdigest()

def inspect_worker(ctx,cid):
    output=ctx.small(['docker','exec',cid,'/usr/bin/timeout','--signal=TERM','--kill-after=1s','20s','/bin/sh','/inspect/inspect.sh',
        'funding-active',build.SERVICE,build.FUNDING_REVISION,ctx.funding_receipt['launcher_sha256'],ctx.funding_receipt['binary_sha256'],'/fixture/policy.tsv',ctx.policy_hash],timeout=21).stdout
    assert 'inspection_complete=true' in output
    return output

def receipt(ctx,image,revision,name):
    current(ctx,name,'silent')
    cid=create(ctx,'worker',image,worker_env(ctx,'recovery_funding_probe',revision=revision),['/usr/local/bin/check-recovery-revision',revision])
    code,out=wait_exit(ctx,cid,min(ctx.runtime_end,time.monotonic()+10));assert code==0
    parsed=dict(line.split('=',1) for line in out.strip().splitlines());assert parsed['source_revision']==revision and parsed['receipt_complete']=='true'
    kind='funding' if image==ctx.funding else 'reader'
    assert parsed['schema']=='agentizable/recovery-artifact-receipt-v1' and parsed['artifact_kind']==kind
    files={'guard_sha256':'/usr/local/bin/check-recovery-revision'}
    if kind=='funding':files.update(binary_sha256='/usr/local/bin/managed-funding-worker',launcher_sha256='/usr/local/bin/managed-funding-launcher')
    else:files.update(binary_sha256='/usr/local/bin/recovery-reader',supervisor_sha256='/usr/local/bin/run-recovery-indexers',bash_sha256='/bin/bash')
    for key,path in files.items():
        copied=ctx.target/(name+'-'+key)
        ctx.small(['docker','cp',cid+':'+path,str(copied)],timeout=15)
        assert copied.stat().st_size<=256*1024**2
        digest=hashlib.sha256()
        with copied.open('rb') as f:
            while block:=f.read(1024*1024):digest.update(block)
        assert digest.hexdigest()==parsed[key],'independent file hash disagrees with guard'
    remove(ctx,cid);no_connections(ctx,name)
    record(ctx,name+'.json',{'receipt':parsed,'no_dependency_connections':True})
    return parsed

def probe(ctx,image,name,env,command=(),expected_exit=None,live=False):
    current(ctx,name,'silent');cid=create(ctx,'worker',image,env,command)
    if live:
        time.sleep(.3);obj=describe(ctx,cid);assert obj['State']['Running']
        output=ctx.small(['docker','exec',cid,'/bin/sh','-c','. /inspect/task_children.sh; tasks_snapshot; readlink /proc/1/exe'],timeout=10).stdout
        expected='/usr/local/bin/managed-funding-launcher' if image==ctx.funding else '/usr/local/bin/recovery-reader'
        assert output.strip().splitlines()[-1]==expected
    else:
        code,output=wait_exit(ctx,cid,min(ctx.runtime_end,time.monotonic()+10))
        assert (code==0) if expected_exit==0 else (code!=0)
    remove(ctx,cid);no_connections(ctx,name)
    record(ctx,name+'.json',{'output':output,'no_dependency_connections':True,'kept_live':live})
    return output

def startup(ctx,schema,case_name):
    current(ctx,case_name,'silent')
    cid=create(ctx,'worker',ctx.funding,worker_env(ctx,schema,settings='{'))
    code,out=wait_exit(ctx,cid,min(ctx.runtime_end,time.monotonic()+15));assert code!=0 and 'MANAGED_BOUNTY_FUNDING is invalid' in out
    remove(ctx,cid)

def protected(before,after,action,operation_id):
    assert before['operations']==after['operations'],'committed funding identity changed'
    for key in ['prior_hashes','journals','gas']:
        for row,value in before[key].items():assert after[key].get(row)==value,'immutable '+key+' row changed'
        if key in ['journals','gas']:assert after[key]==before[key],'shipping fixture must not create a signature/gas reservation'

def case(ctx,folder):
    metadata=json.loads((folder/'case.json').read_text());native=json.loads((folder/'native-result.json').read_text())
    name=metadata['name'];schema=metadata['database_schema'];assert metadata['schema']=='agentizable/funding-process-fixture-v1'
    assert name==folder.name and time.time()<metadata['valid_until'] and re.fullmatch('recovery_funding_[a-z_]+',schema)
    assert hashlib.sha256((folder/'before.sql').read_bytes()).hexdigest()==metadata['sql_sha256']
    sql(ctx,f'DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}')
    if metadata['action']!='config_error':
        startup(ctx,schema,'bootstrap-'+name)
        tables=json.loads(sql(ctx,"SELECT jsonb_agg(tablename) FROM pg_tables WHERE schemaname='"+schema+"'"))
        assert all(re.fullmatch('[a-z_][a-z_0-9]*',t) for t in tables)
        sql(ctx,'TRUNCATE '+','.join(tables)+' CASCADE',schema)
        sql(ctx,'',schema,folder/'before.sql')
        if name=='busy':
            # Preserve the live-lease branch despite the independent build between phases.
            sql(ctx,"UPDATE managed_bounty_funding_operations SET lease_until=now()+interval '180 seconds'",schema)
    before=snapshot(ctx,schema)
    if name!='busy':assert before==metadata['before'],'exported input differs'
    current(ctx,name,metadata['rpc_mode'])
    env=worker_env(ctx,schema,settings='{' if metadata['action']=='config_error' else SETTINGS)
    cid=create(ctx,'worker',ctx.funding,env);deadline=min(ctx.runtime_end,time.monotonic()+30);observed=None;inspection=None
    while time.monotonic()<deadline:
        obj=describe(ctx,cid);logs=ctx.small(['docker','logs',cid]);output=logs.stdout+logs.stderr
        if metadata['action']=='config_error':
            if not obj['State']['Running']:
                assert obj['State']['ExitCode']!=0 and 'MANAGED_BOUNTY_FUNDING is invalid' in output
                observed={'action':'config_error'};break
        else:
            for line in output.splitlines():
                try:report=json.loads(line)
                except ValueError:continue
                if report.get('schema') in ('agent-bounties/managed-funding-v1','agent-bounties/managed-funding-recovery-v1'):
                    observed=report;break
            if observed:
                inspection=inspect_worker(ctx,cid);break
            assert obj['State']['Running'],'worker exited before its first completed poll'
        time.sleep(.05)
    assert observed is not None,'worker poll deadline'
    remove(ctx,cid)
    if metadata['action']=='poll_error':assert observed['decision']=='retry_from_persisted_operation'
    else:assert observed['action']==metadata['action']
    actual_rpc=lines(ctx,name,'rpc');assert not lines(ctx,name,'errors')
    assert [v['method'] for v in actual_rpc]==[v['method'] for v in native['requests']]
    assert not any(v['method'].startswith('eth_send') for v in actual_rpc)
    after=snapshot(ctx,schema)
    if metadata['action']!='config_error':
        assert before.keys()==after.keys()
        for table,digest in before.items():
            if table not in metadata['mutable_tables']:assert after[table]==digest,'unexpected table write '+table
    projection=json.loads(sql(ctx,metadata['protected_sql'],schema))
    if metadata['action']!='config_error':protected(metadata['protected_before'],projection,metadata['action'],metadata['operation_id'])
    facts=json.loads(sql(ctx,"SELECT jsonb_build_object('held',(SELECT count(*) FROM managed_stripe_bank_actions WHERE state='held' AND funding_operation_id IS NOT NULL),'journals',(SELECT count(*) FROM relay_transaction_outbox),'funded',(SELECT count(*) FROM managed_bounty_funding_operations WHERE outcome='funded'),'orders',(SELECT COALESCE(jsonb_object_agg(order_id::text,outcome),'{}') FROM managed_bounty_funding_operations),'journal',(SELECT COALESCE(jsonb_agg(jsonb_build_object('operation_id',operation_id,'nonce',nonce,'tx_hash',tx_hash,'raw',signed_transaction,'send_attempts',send_attempts) ORDER BY operation_id),'[]') FROM relay_transaction_outbox))",schema))
    assert facts==native['facts'] and facts['funded']==0
    record(ctx,'case-'+name+'.json',{'report':observed,'facts':facts,'protected_after':projection,'before':before,'after':after,'process_inspection':inspection,'rpc':actual_rpc,'no_settlement_claim':True,'lease_refreshed_only':name=='busy'})

def start(ctx,images):
    ctx.runtime_end=min(ctx.work_end,time.monotonic()+600)
    ctx.set_phase(ctx.runtime_end)
    ctx.exchange=ctx.target/'exchange';ctx.exchange.mkdir(mode=0o700)
    # The standard runner UID need not equal the unprivileged fixture UID1000.
    ctx.exchange.chmod(0o777)
    (ctx.target/'policy').mkdir(mode=0o755)
    inspector=ctx.target/'inspector';inspector.mkdir(mode=0o755)
    for name in ['inspect.sh','identity.awk','environment.awk','task_children.sh']:
        (inspector/name).write_bytes((INSPECTOR/name).read_bytes());(inspector/name).chmod(0o555)
    ctx.runtime_ids=[];ctx.runtime_groups={}
    ctx.funding=images['funding']['id'];standby=images['standby']['id'];current(ctx,'initial','silent')
    ctx.start_runtime_watchdog(ctx.runtime_end)
    duration=max(1,int(ctx.runtime_end-time.monotonic()))
    ctx.pg=create(ctx,'pg',PG,{'POSTGRES_USER':'agent_bounties_test','POSTGRES_DB':'agent_bounties_test','POSTGRES_PASSWORD':'fixture-only','PGDATA':'/var/lib/postgresql/data/pgdata'},['--signal=TERM','--kill-after=10s',str(duration)+'s','/usr/local/bin/docker-entrypoint.sh','postgres'],'/usr/bin/timeout')
    ready=min(ctx.runtime_end,time.monotonic()+30)
    while ctx.small(['docker','exec',ctx.pg,'pg_isready','-U','agent_bounties_test','-d','agent_bounties_test'],check=False).returncode:
        assert time.monotonic()<ready;time.sleep(.1)
    for mount,expected in [('/var/lib/postgresql/data',256*1024**2),('/var/run/postgresql',16*1024**2),('/tmp',16*1024**2)]:
        kind,block,count=ctx.small(['docker','exec',ctx.pg,'stat','-f','-c','%T %S %b',mount]).stdout.split()
        assert kind=='tmpfs' and int(block)*int(count)==expected
    ctx.rpc=create(ctx,'rpc',PYTHON,{},['/rpc_fixture.py',str(max(1,int(ctx.runtime_end-time.monotonic())))],'python3')
    ready=min(ctx.runtime_end,time.monotonic()+10)
    while not (ctx.exchange/'ready').exists():assert time.monotonic()<ready;time.sleep(.05)
    namespaces=[]
    for cid in [ctx.pg,ctx.rpc]:
        namespaces.append(ctx.small(['docker','exec',cid,'readlink','/proc/1/ns/net']).stdout.strip())
        assert ctx.small(['docker','exec',cid,'ls','/sys/class/net']).stdout.split()==['lo']
        assert len(ctx.small(['docker','exec',cid,'cat','/proc/net/route']).stdout.strip().splitlines())==1
    assert len(set(namespaces))==1
    record(ctx,'runtime-network.json',{'namespace':namespaces[0],'interfaces':['lo'],'external_routes':0,'host_ports':0,'container_limits_sum':{'cpu':1.85,'memory_mib':1856,'pids':192,'swap':0},'manager_and_watchdog_are_within_standard_runner_VM':True})
    for image,revision,executable,capability,label in [(standby,build.STANDBY_REVISION,'recovery-reader','standby','standby-build-identity'),(ctx.funding,build.FUNDING_REVISION,'managed-funding-launcher','funding','funding-build-identity')]:
        output=probe(ctx,image,label,worker_env(ctx,'recovery_funding_probe',revision=revision),['/usr/local/bin/'+executable,'--build-identity'],expected_exit=0)
        identity=json.loads(output)
        assert identity['binding']=={'service_id':build.SERVICE,'source_revision':revision,'capability':capability}
    ctx.funding_receipt=receipt(ctx,ctx.funding,build.FUNDING_REVISION,'funding-direct-guard')
    receipt(ctx,standby,build.STANDBY_REVISION,'standby-direct-guard')
    env=worker_env(ctx,'recovery_funding_probe',revision=build.STANDBY_REVISION)
    probe(ctx,standby,'standby-files-absent',env,['/bin/sh','-c','test ! -e /usr/local/bin/managed-funding-worker && test ! -e /usr/local/bin/managed-funding-launcher && test ! -e /usr/local/bin/worker'],expected_exit=0)
    probe(ctx,standby,'standby-future-settings',env,live=True)
    probe(ctx,standby,'standby-forced-active',dict(env,RECOVERY_READER_MODE='active'),expected_exit=1)
    probe(ctx,standby,'standby-future-revision',worker_env(ctx,'recovery_funding_probe'),expected_exit=1)
    probe(ctx,ctx.funding,'funding-guard-mismatch',worker_env(ctx,'recovery_funding_probe'),['/usr/local/bin/check-recovery-revision',build.STANDBY_REVISION],expected_exit=1)
    probe(ctx,ctx.funding,'funding-global-false',dict(worker_env(ctx,'recovery_funding_probe'),ENABLE_HOSTED_FINANCIAL_SIGNING='false'),expected_exit=1)
    probe(ctx,ctx.funding,'funding-held',dict(worker_env(ctx,'recovery_funding_probe'),RECOVERY_FUNDING_MODE='held',ENABLE_HOSTED_FINANCIAL_SIGNING='false'),live=True)
    ctx.policy_hash=actual_policy(ctx)
    native=Path(ctx.manifest['native_exports']['directory'])
    for relative,digest in ctx.manifest['native_exports']['files'].items():
        assert hashlib.sha256((native/relative).read_bytes()).hexdigest()==digest,'native fixture export changed'
    expected_folders={Path(name).parts[0] for name in ctx.manifest['native_exports']['files'] if name.endswith('/case.json')}
    folders=sorted(native/name for name in expected_folders)
    assert {p.name for p in native.iterdir() if p.is_dir()}==expected_folders
    assert len(folders)==18
    for folder in folders:
        assert time.monotonic()<ctx.runtime_end
        case(ctx,folder)
    record(ctx,'actual-image-results.json',{'case_count':len(folders),'same_cache_standby_and_funding':True,'positive_core_polls_are_separate_native_evidence':True,'actual_FX_or_EVM_payment_qualified':False})

def cleanup(ctx,deadline):
    errors=[]
    ids=[]
    try:
        ids=ctx.small(['docker','ps','-aq','--no-trunc','--filter','label=ab.recovery.owner='+ctx.attempt['nonce'],'--filter','label=ab.recovery.role=funding-image-fixture']).stdout.splitlines()
        assert len(ids)<=3
    except Exception as error:errors.append({'listing':str(error)[:300]})
    for cid in ids:
        try:remove(ctx,cid)
        except Exception as error:errors.append({'container':cid,'error':str(error)[:300]})
    for cid,observed in getattr(ctx,'runtime_groups',{}).items():
        try:
            group=Path(observed['path'])
            if group.exists():
                st=group.stat();assert [st.st_dev,st.st_ino]==observed['identity']
                assert dict(line.split() for line in (group/'cgroup.events').read_text().splitlines())['populated']=='0'
        except Exception as error:errors.append({'cgroup':cid,'error':str(error)[:300]})
    if not errors:record(ctx,'runtime-cleanup-complete.json',{'containers_removed':True})
    return {'complete':not errors,'errors':errors}
