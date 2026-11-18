"""Always-run CI cleanup and bounded evidence packaging; exact attempt nonce only."""
import argparse
import hashlib
import json
import os
import re
import subprocess
import tarfile
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
assert os.environ.get('GITHUB_ACTIONS') == 'true'
root = args.output.resolve()
assert root.parent == Path(os.environ['RUNNER_TEMP']).resolve()
assert root.name == 'funding-image-evidence'
assert not root.is_symlink()
if not root.exists():
    raise SystemExit('Qualification did not create evidence; no attempt to clean or upload')
attempt = json.loads((root/'attempt.json').read_text())
nonce = attempt['nonce']
assert re.fullmatch('[0-9a-f]{48}',nonce)
assert attempt['run_id'] == os.environ['GITHUB_RUN_ID'] and attempt['attempt'] == os.environ['GITHUB_RUN_ATTEMPT']
env = {'PATH':os.environ['PATH'],'HOME':str(root.parent),'DOCKER_CONFIG':str(root.parent/(root.name+'-temporary')/'docker-config'),'GOMAXPROCS':'2'}


def run(command):
    result = subprocess.run(command,env=env,capture_output=True,text=True,timeout=10,check=True)
    assert len(result.stdout)<2*1024**2 and len(result.stderr)<65536
    return result.stdout


errors = []
try:
    ids = run(['docker','ps','-aq','--no-trunc','--filter','label=ab.recovery.owner='+nonce]).splitlines()
    assert len(ids)<=4
    for cid in ids:
        obj = json.loads(run(['docker','inspect',cid]))[0]
        assert obj['Id']==cid and obj['Config']['Labels'].get('ab.recovery.owner')==nonce
        assert obj['Config']['Labels'].get('ab.recovery.role') in ('funding-image-fixture','funding-native-fixture')
        run(['docker','rm','--force','--volumes',cid])
    assert not run(['docker','ps','-aq','--no-trunc','--filter','label=ab.recovery.owner='+nonce]).strip()
    history=root/'runtime-identities.jsonl'
    if history.exists():
        assert history.stat().st_size<=1024**2
        for line in history.read_text().splitlines():
            observed=json.loads(line)['observation'];path=Path(observed['path'])
            assert path.is_relative_to('/sys/fs/cgroup')
            if path.exists():
                assert [path.stat().st_dev,path.stat().st_ino]==observed['identity']
                assert dict(row.split() for row in (path/'cgroup.events').read_text().splitlines())['populated']=='0'
except Exception as error:
    errors.append(type(error).__name__+': '+str(error)[:500])
if not errors:
    for folder in root.glob('watchdog-*'):
        assert folder.is_dir() and not folder.is_symlink()
        (folder/'watchdog-cancel.json').write_text('{}\n')
(root/'always-cleanup.json').write_text(json.dumps({'complete':not errors,'errors':errors},indent=2)+'\n')
files=sorted(path for path in root.rglob('*') if path.is_file())
assert len(files)<=256 and sum(p.stat().st_size for p in files)<=32*1024**2
manifest={}
for path in files:
    assert not path.is_symlink()
    assert path.suffix in ('.json','.jsonl','.log','.sql','.iid'), 'unexpected evidence file'
    manifest[str(path.relative_to(root))]=hashlib.sha256(path.read_bytes()).hexdigest()
(root/'evidence-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
archive=root.parent/'funding-image-evidence.tar.gz'
assert not archive.exists(), 'accepted archive path must be absent'
temporary=root.parent/('funding-image-evidence.'+nonce+'.tar.gz.tmp')
with temporary.open('xb') as stream:
    with tarfile.open(fileobj=stream,mode='w:gz') as out:
        for path in files+[root/'evidence-manifest.json']:
            out.add(path,arcname=str(path.relative_to(root)),recursive=False)
assert temporary.stat().st_size<=16*1024**2, 'compressed evidence exceeds 16 MiB'
assert not archive.exists(), 'accepted archive path appeared during packaging'
os.rename(temporary,archive)
accepted_digest=hashlib.sha256(archive.read_bytes()).hexdigest()
with Path(os.environ['GITHUB_OUTPUT']).open('a') as accepted:
    accepted.write('archive_ready=true\narchive_sha256='+accepted_digest+'\n')
print(json.dumps({'evidence_sha256':accepted_digest,'compressed_bytes':archive.stat().st_size,'cleanup_complete':not errors}))
raise SystemExit(1 if errors else 0)
