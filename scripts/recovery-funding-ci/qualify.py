"""Draft CI-only image qualification. All money, keys, databases and RPC are fixtures."""
import argparse
import hashlib
import json
import os
import re
import resource
import secrets
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

import fixture_exports
import image_build as binding
import image_runtime as runtime

PREDECESSOR = 'a9a023bae517338963f8aaa89ab8ecf0225cf11a'
PINS = {
    'rust': ('docker.io/library/rust@sha256:4ec71e955e6c08aeb238885083222ddff79d82eb87654a96c76e38e94da1a53b', 'sha256:1c3316753323aa5edd4c2be762323794f4aa9f4335635dc4c7964ee99cb986d2'),
    'debian': ('docker.io/library/debian@sha256:c67e92c8099ba22853ab4cfda7cfb494d253d6012ca82dee98ba8976772af018', 'sha256:43f84a0b210f98845ef003912981774c95efe439eaa4b1cb62cf803fd46fc82a'),
    'postgres': ('docker.io/library/postgres@sha256:43f9275df91e012c3da3d86bea5d7bb5e67a473296cc906c5f9e80b8176b1a37', runtime.PG),
    'python': ('docker.io/library/python@sha256:05a994d2344eb364cc186910f4fc8786a38215f39f00d0acc25bff33a08f4177', runtime.PYTHON),
}
FRONTEND = 'docker.io/docker/dockerfile@sha256:7b32114e84ca21aeb2d6f871a0b62b3a0762193f75d9afd048ee4cbe889f3148'
GIB = 1024 ** 3


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        while chunk := stream.read(1024 * 1024):
            value.update(chunk)
    return value.hexdigest()


class Context:
    def __init__(self, args):
        assert os.environ.get('GITHUB_ACTIONS') == 'true', 'CI-only driver'
        assert os.environ.get('GITHUB_REPOSITORY') == 'NSPG13/agent-bounties'
        assert os.getuid() > 0 and os.getgid() > 0, 'nonroot runner UID and GID required'
        assert re.fullmatch('[0-9a-f]{40}', args.head)
        self.source, self.predecessor = args.source.resolve(), args.predecessor.resolve()
        self.here = args.output.resolve()
        self.here.mkdir(mode=0o700)
        self.target = self.here.parent / (self.here.name + '-temporary')
        self.target.mkdir(mode=0o755)
        self.helpers = Path(__file__).resolve().parent
        self.started = time.monotonic()
        self.work_end, self.final_end = self.started + 3190, self.started + 3290
        self.phase_end = self.started + 2100
        self.attempt = {'nonce': secrets.token_hex(24), 'source_head': args.head, 'run_id': os.environ['GITHUB_RUN_ID'], 'attempt': os.environ['GITHUB_RUN_ATTEMPT']}
        config = self.target / 'docker-config'
        config.mkdir(mode=0o700)
        self.env = {'PATH': os.environ['PATH'], 'HOME': str(self.target), 'DOCKER_CONFIG': str(config), 'GOMAXPROCS': '2'}
        self.runtime_ids, self.runtime_groups, self.watchdogs = [], {}, []
        self.record('attempt.json', dict(self.attempt, start_monotonic=self.started, work_end=self.work_end, final_end=self.final_end))

    def record(self, name, value):
        binding.record(self.here, name, value)

    def remaining(self, maximum):
        value = min(maximum, self.phase_end - time.monotonic())
        if value <= 0:
            raise TimeoutError('absolute CI phase deadline')
        return value

    def set_phase(self, end):
        self.phase_end = min(end, self.work_end)

    def small(self, args, timeout=10, check=True, env=None, cwd=None):
        result = subprocess.run(args, cwd=cwd or self.source, env=env or self.env, capture_output=True, text=True, timeout=self.remaining(timeout), check=check)
        assert len(result.stdout) <= 2 * 1024 ** 2 and len(result.stderr) <= 65536
        return result

    def source_check(self):
        result = []
        for path, head in [(self.source, self.attempt['source_head']), (self.predecessor, PREDECESSOR)]:
            assert self.small(['git', 'rev-parse', 'HEAD'], cwd=path).stdout.strip() == head
            assert not self.small(['git', 'status', '--porcelain', '--untracked-files=all'], cwd=path).stdout
            result.append({'head': head, 'tree': self.small(['git', 'rev-parse', 'HEAD^{tree}'], cwd=path).stdout.strip()})
        return result

    def disk_and_logs(self):
        assert shutil.disk_usage(self.target).free >= 4 * GIB, 'runner disk floor'
        files = [p for p in self.here.rglob('*') if p.is_file()]
        assert len(files) <= 256 and sum(p.stat().st_size for p in files) <= 32 * 1024 ** 2, 'evidence cap'

    def long(self, args, label, deadline, tick=None):
        def file_limit():
            resource.setrlimit(resource.RLIMIT_FSIZE, (16 * 1024 ** 2, 16 * 1024 ** 2))
        with (self.here / (label + '.log')).open('xb') as log:
            process = subprocess.Popen(args, cwd=self.source, env=self.env, stdout=log, stderr=subprocess.STDOUT, preexec_fn=file_limit, start_new_session=True)
            try:
                while process.poll() is None:
                    assert time.monotonic() < min(deadline, self.phase_end), label + ' deadline'
                    self.disk_and_logs()
                    if tick:
                        tick()
                    time.sleep(.2)
                assert process.returncode == 0, label + ' failed'
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=2)

    def start_runtime_watchdog(self, deadline):
        folder = self.here / ('watchdog-' + str(len(self.watchdogs)))
        folder.mkdir()
        process = subprocess.Popen([sys.executable, str(self.helpers / 'watchdog.py'), str(folder), self.attempt['nonce'], str(deadline), self.env['DOCKER_CONFIG']], env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        self.watchdogs.append((folder, process))


def build(ctx, source, dockerfile, name, revision, extra=()):
    tag = 'ab-funding-ci-' + ctx.attempt['nonce'] + ':' + name
    args = ['docker', 'buildx', 'build', '--builder', 'default', '--platform', 'linux/amd64', '--progress', 'plain', '--load', '--tag', tag,
            '--file', str(source / dockerfile), '--iidfile', str(ctx.here / (name + '.iid')), '--metadata-file', str(ctx.here / (name + '-metadata.json')),
            '--build-context', 'rust:1.94-bookworm=docker-image://' + PINS['rust'][0],
            '--build-context', 'debian:bookworm-slim=docker-image://' + PINS['debian'][0],
            '--build-arg', 'BUILDKIT_SYNTAX=' + FRONTEND,
            '--build-arg', 'RENDER_SERVICE_ID=' + binding.SERVICE, '--build-arg', 'RENDER_GIT_COMMIT=' + revision,
            '--label', 'ab.recovery.owner=' + ctx.attempt['nonce'], *extra, str(source)]
    ctx.record(name + '-command.json', {'command': args, 'source': ctx.source_check()})
    ctx.long(args, name + '-build', ctx.phase_end)
    iid = (ctx.here / (name + '.iid')).read_text().strip()
    assert re.fullmatch('sha256:[0-9a-f]{64}', iid)
    obj = json.loads(ctx.small(['docker', 'image', 'inspect', tag]).stdout)[0]
    assert obj['Id'] == iid and obj['Config']['Labels'].get('ab.recovery.owner') == ctx.attempt['nonce']
    assert not obj['Config']['Entrypoint'] and obj['Size'] <= 3 * GIB
    return {'id': iid, 'tag': tag, 'config': obj['Config'], 'rootfs': obj['RootFS'], 'size': obj['Size']}


def native_exports(ctx, fixture):
    ctx.set_phase(min(ctx.work_end - 600, time.monotonic() + 420))
    ctx.start_runtime_watchdog(ctx.phase_end)
    exports = ctx.here / 'exports'
    exports.mkdir(mode=0o777)
    exports.chmod(0o777)  # Only synthetic fixture data in an ephemeral runner.
    # Reuse the runtime PG creator and actual bounded tmpfs settings.
    ctx.exchange = ctx.target / 'native-exchange'
    ctx.exchange.mkdir()
    (ctx.target / 'inspector').mkdir(exist_ok=True)
    (ctx.target / 'policy').mkdir(exist_ok=True)
    duration = max(1, int(ctx.phase_end - time.monotonic()))
    ctx.pg = runtime.create(ctx, 'pg', runtime.PG, {'POSTGRES_USER': 'agent_bounties_test', 'POSTGRES_DB': 'agent_bounties_test', 'POSTGRES_PASSWORD': 'fixture-only', 'PGDATA': '/var/lib/postgresql/data/pgdata'}, ['--signal=TERM', '--kill-after=10s', str(duration) + 's', '/usr/local/bin/docker-entrypoint.sh', 'postgres'], '/usr/bin/timeout')
    ready = time.monotonic() + 30
    while ctx.small(['docker', 'exec', ctx.pg, 'pg_isready', '-U', 'agent_bounties_test', '-d', 'agent_bounties_test'], check=False).returncode:
        assert time.monotonic() < ready
        time.sleep(.1)
    args = ['docker', 'create', '--pull=never', '--name', 'ab-funding-native-fixture', '--label', 'ab.recovery.owner=' + ctx.attempt['nonce'], '--label', 'ab.recovery.role=funding-native-fixture',
            '--cpus', '1.4', '--memory', '6016m', '--memory-swap', '6016m', '--pids-limit', '224', '--network', 'container:' + ctx.pg,
            '--read-only', '--cap-drop=ALL', '--security-opt=no-new-privileges', '--user', f'{os.getuid()}:{os.getgid()}', '--restart=no', '--tmpfs', '/tmp:rw,nosuid,nodev,size=64m',
            '--log-driver=local', '--log-opt', 'max-size=2m', '--log-opt', 'max-file=1', '--log-opt', 'compress=false',
            '--mount', 'type=bind,src=' + str(exports) + ',dst=/exports', '--env', 'RECOVERY_FUNDING_FIXTURE_EXPORT=/exports',
            '--env', 'AGENT_BOUNTIES_LOCAL_TEST_DATABASE=1', '--env', 'AGENT_BOUNTIES_TEST_DATABASE_URL=postgres://agent_bounties_test:fixture-only@127.0.0.1:5432/agent_bounties_test',
            '--entrypoint', '/usr/bin/timeout', fixture['id'], '--signal=TERM', '--kill-after=10s', str(duration) + 's', '/bin/sleep', str(duration)]
    cid = ctx.small(args).stdout.strip()
    assert re.fullmatch('[0-9a-f]{64}', cid)
    ctx.small(['docker', 'start', cid])
    obj = json.loads(ctx.small(['docker', 'inspect', cid]).stdout)[0]
    host = obj['HostConfig']
    assert obj['State']['Running'] and obj['Image'] == fixture['id']
    assert obj['Config']['User'] == f'{os.getuid()}:{os.getgid()}', 'native fixture user mismatch'
    assert host['NetworkMode'] == 'container:' + ctx.pg and not host['PortBindings']
    assert host['ReadonlyRootfs'] and host['CapDrop'] == ['ALL'] and host['SecurityOpt'] == ['no-new-privileges']
    assert (host['NanoCpus'], host['Memory'], host['MemorySwap'], host['PidsLimit']) == (1400000000, 6016*1024**2, 6016*1024**2, 224)
    assert not any(m['Destination'] == '/var/run/docker.sock' or m['Type'] == 'volume' for m in obj['Mounts'])
    pid = obj['State']['Pid']
    group = Path('/sys/fs/cgroup') / Path(f'/proc/{pid}/cgroup').read_text().strip().split('::', 1)[1].lstrip('/')
    controls = {key: (group/key).read_text().strip() for key in ('cpu.max','memory.max','memory.swap.max','pids.max')}
    quota, period = map(int, controls['cpu.max'].split())
    assert quota/period == 1.4 and int(controls['memory.max']) == 6016*1024**2 and controls['memory.swap.max'] == '0' and controls['pids.max'] == '224'
    observed = {'path': str(group), 'identity': [group.stat().st_dev, group.stat().st_ino], 'controls': controls}
    ctx.runtime_groups[cid] = observed
    with (ctx.here/'runtime-identities.jsonl').open('a') as target:
        target.write(json.dumps({'id':cid,'observation':observed})+'\n')
    namespaces = []
    for owned in [ctx.pg, cid]:
        namespaces.append(ctx.small(['docker','exec',owned,'readlink','/proc/1/ns/net']).stdout.strip())
        assert ctx.small(['docker','exec',owned,'ls','/sys/class/net']).stdout.split() == ['lo']
        assert len(ctx.small(['docker','exec',owned,'cat','/proc/net/route']).stdout.strip().splitlines()) == 1
    assert len(set(namespaces)) == 1
    ctx.native_worker_hash = ctx.small(['docker','exec',cid,'sha256sum','/workspace/target/release/worker']).stdout.split()[0]
    assert re.fullmatch('[0-9a-f]{64}',ctx.native_worker_hash)
    ctx.record('native-fixture-isolation.json', {'container':cid,'cgroup':observed,'namespace':namespaces[0],'external_routes':0,'worker_sha256':ctx.native_worker_hash,'fixture_user':obj['Config']['User']})
    ctx.long(['docker', 'exec', cid, '/opt/recovery-funding-fixture-test', '--ignored', '--test-threads=1'], 'native-fixture', ctx.phase_end, lambda: fixture_exports.exports(ctx.small, ctx.here, ctx.pg, ctx.phase_end))
    output = (ctx.here / 'native-fixture.log').read_text()
    assert re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', output)[-1] == ('8', '0', '0')
    ctx.small(['docker', 'rm', '--force', cid])
    runtime.remove(ctx, ctx.pg)
    for folder, process in ctx.watchdogs:
        assert not (folder/'watchdog-fired.json').exists(), 'native watchdog fired'
        if process.poll() is None:
            (folder / 'watchdog-cancel.json').write_text('{}\n')
            process.wait(timeout=2)
    files = {str(p.relative_to(exports)): digest(p) for p in exports.rglob('*') if p.is_file()}
    assert len(files) <= 200
    assert {p.name for p in exports.iterdir() if p.is_dir()} == set(fixture_exports.CASES)
    ctx.manifest = {'native_exports': {'directory': str(exports), 'files': files}}
    ctx.record('native-exports.json', ctx.manifest)
    ctx.record('native-fixture-result.json', {'passed': 8, 'failed': 0, 'ignored': 0, 'worker_sha256':ctx.native_worker_hash, 'shipping_hash_comparison_pending':True, 'positive_core_poll_is_separate_in_process_evidence': True})


def cleanup(ctx):
    ctx.phase_end = ctx.final_end
    errors = []
    try:
        ids = ctx.small(['docker', 'ps', '-aq', '--no-trunc', '--filter', 'label=ab.recovery.owner=' + ctx.attempt['nonce']]).stdout.splitlines()
        assert len(ids) <= 4
        for cid in ids:
            obj = json.loads(ctx.small(['docker', 'inspect', cid]).stdout)[0]
            assert obj['Id'] == cid and obj['Config']['Labels'].get('ab.recovery.owner') == ctx.attempt['nonce']
            assert obj['Config']['Labels'].get('ab.recovery.role') in ('funding-image-fixture', 'funding-native-fixture')
            ctx.small(['docker', 'rm', '--force', '--volumes', cid], timeout=15)
        assert not ctx.small(['docker', 'ps', '-aq', '--filter', 'label=ab.recovery.owner=' + ctx.attempt['nonce']]).stdout.strip()
        observations = list(ctx.runtime_groups.values())
        history = ctx.here/'runtime-identities.jsonl'
        if history.exists():
            assert history.stat().st_size <= 1024**2
            observations += [json.loads(line)['observation'] for line in history.read_text().splitlines()]
        for observed in observations:
            path = Path(observed['path'])
            if path.exists():
                assert [path.stat().st_dev, path.stat().st_ino] == observed['identity']
                assert dict(line.split() for line in (path / 'cgroup.events').read_text().splitlines())['populated'] == '0'
    except Exception as error:
        errors.append(str(error)[:500])
    if not errors:
        for folder, process in ctx.watchdogs:
            (folder / 'watchdog-cancel.json').write_text('{}\n')
            try:
                process.wait(timeout=2)
            except Exception as error:
                errors.append(str(error)[:200])
    ctx.record('cleanup.json', {'complete': not errors, 'errors': errors, 'owned_containers_absent': not errors})
    return not errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'predecessor', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--head', required=True)
    ctx = Context(parser.parse_args())
    failure = None
    try:
        assert shutil.disk_usage(ctx.target).free >= 12 * GIB
        ctx.record('source-before.json', ctx.source_check())
        builder = ctx.small(['docker', 'buildx', 'inspect', 'default']).stdout
        assert re.search(r'^Driver:\s+docker\s*$', builder, re.M), 'default Docker driver required'
        ctx.record('builder.json', {'name': 'default', 'description': builder, 'same_runner_cache': True, 'new_privileged_builder': False})
        for name, (reference, image_id) in PINS.items():
            ctx.long(['docker', 'pull', '--platform', 'linux/amd64', reference], name + '-pull', ctx.phase_end)
            assert ctx.small(['docker', 'image', 'inspect', reference, '--format', '{{.Id}}']).stdout.strip() == image_id
        standby = build(ctx, ctx.predecessor, 'Dockerfile.recovery-indexers', 'standby', binding.STANDBY_REVISION, ['--build-arg', 'RECOVERY_READER_ARTIFACT_CAPABILITY=standby'])
        funding = build(ctx, ctx.source, 'Dockerfile.recovery-funding', 'funding', binding.FUNDING_REVISION)
        fixture = build(ctx, ctx.source, 'Dockerfile.recovery-funding-fixture', 'fixture', binding.FUNDING_REVISION, ['--build-arg', 'FUNDING_IMAGE=' + funding['tag']])
        images = {'standby': standby, 'funding': funding, 'fixture': fixture}
        ctx.record('built-images.json', {'images': images, 'pins': PINS, 'frontend': FRONTEND, 'fixture_service': binding.SERVICE, 'production_artifacts': False})
        native_exports(ctx, fixture)
        # Remove empty native-phase mount points before the runtime creates them.
        (ctx.target / 'inspector').rmdir()
        (ctx.target / 'policy').rmdir()
        ctx.set_phase(ctx.work_end)
        runtime.start(ctx, images)
        assert ctx.native_worker_hash == ctx.funding_receipt['binary_sha256'], 'native child differs from actual shipping image'
        ctx.record('shipping-native-binary-equivalence.json', {'equal':True,'sha256':ctx.native_worker_hash})
        ctx.record('source-after.json', ctx.source_check())
    except BaseException as error:
        failure = type(error).__name__ + ': ' + str(error)[:1000]
    complete = cleanup(ctx)
    ctx.record('outcome.json', {'passed': failure is None and complete, 'failure': failure, 'cleanup_complete': complete, 'production_signing_enabled': False, 'EVM_or_FX_settlement_qualified': False})
    ctx.disk_and_logs()
    print(json.dumps({'passed': failure is None and complete, 'failure': failure, 'cleanup_complete': complete}))
    return 0 if failure is None and complete else 1


if __name__ == '__main__':
    raise SystemExit(main())
