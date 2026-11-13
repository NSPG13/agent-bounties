import hashlib,json,os,subprocess,time
from pathlib import Path
CASES={
    'startup-invalid':'startup','startup-idle':'startup','idle-first':'idle','idle-restart':'idle',
    'busy':'busy','expired':'expired','disputed':'disputed','mined-first':'mined','mined-restart':'mined',
    'old-journal':'old','wrong-network':'wrong_network','wrong-factory':'wrong_factory',
    'wrong-wallet':'wrong_wallet','wrong-digest':'wrong_digest','simulation-rejected':'core',
    'core-journal-shipping-restart':'core','fx-unavailable-unsigned':'fx_unsigned','fx-unavailable-retry':'core',
}

def save(here,name,value):
    with (here/name).open('x') as f:json.dump(value,f,indent=2);f.write('\n')

def exports(small,here,pgcid,deadline):
    root=here/'exports'
    for folder in root.iterdir():
        if not folder.is_dir():continue
        assert not folder.is_symlink() and folder.name in CASES
        request=folder/'request.json';ack=folder/'export-ready.json'
        if not request.exists() or ack.exists():continue
        assert request.stat().st_size<=1024 and not request.is_symlink()
        obj=json.loads(request.read_text());schema='recovery_funding_'+CASES[folder.name]
        assert obj=={'name':folder.name,'database_schema':schema}
        assert time.monotonic()+10<deadline
        # No request supplies a command, path, identifier or container ID outside this mapping.
        output=folder/'before.sql';errors=folder/'dump-error.log'
        with output.open('xb') as out,errors.open('xb') as err:
            run=subprocess.run(['prlimit','--fsize=16777216','--','docker','exec',pgcid,'pg_dump','-U','agent_bounties_test','-d','agent_bounties_test','--data-only','--column-inserts','--no-owner','--no-privileges','--schema',schema],stdout=out,stderr=err,timeout=min(10,deadline-time.monotonic()))
        assert run.returncode==0 and output.stat().st_size<=16*1024**2 and errors.stat().st_size<=65536
        receipt={'sha256':hashlib.sha256(output.read_bytes()).hexdigest(),'bytes':output.stat().st_size}
        save(folder,'export-ready.tmp',receipt);os.rename(folder/'export-ready.tmp',ack)
