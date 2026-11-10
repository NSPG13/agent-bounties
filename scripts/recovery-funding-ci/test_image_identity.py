"""Offline image-identity controls. No Docker executable or network is invoked."""
import copy
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import image_identity as identity

HERE = Path(__file__).resolve().parent
ZERO = 'sha256:' + '0' * 64
CONFIG = 'sha256:' + '1' * 64
MANIFEST = 'sha256:' + '2' * 64
EXTRA = 'sha256:' + '3' * 64
CID = '4' * 64
OWNER = '5' * 48


def base_object(name, mode):
    reference, config_digest, desc, config = identity.pin_bytes(
        name, (HERE / 'pins' / (name + '-manifest.json')).read_bytes(),
        (HERE / 'pins' / (name + '-config.json')).read_bytes())
    value = {'Id': config_digest if mode == 'classic' else desc['digest'],
             'RepoDigests': [identity.normalized_ref(reference)],
             'Os': 'linux', 'Architecture': 'amd64', 'Variant': '',
             'Config': config['config'],
             'RootFS': {'Type': 'layers', 'Layers': config['rootfs']['diff_ids']}}
    if mode == 'containerd':
        value['Descriptor'] = desc
    return value


def built_object(mode, annotated=False):
    base = identity.verify_pin('debian', base_object('debian', mode), mode)
    desc = {'digest': MANIFEST, 'size': 500, 'mediaType': 'application/vnd.oci.image.manifest.v1+json'}
    if annotated:
        desc['annotations'] = {'config.digest': CONFIG}
    metadata = {'containerimage.digest': CONFIG, 'containerimage.config.digest': CONFIG}
    if mode == 'containerd':
        metadata.update({'containerimage.digest': MANIFEST, 'containerimage.descriptor': copy.deepcopy(desc)})
    obj = {'Id': CONFIG if mode == 'classic' else MANIFEST,
           'Os': 'linux', 'Architecture': 'amd64',
           'RootFS': {'Type': 'layers', 'Layers': base['rootfs'] + [EXTRA]},
           'Config': {'Labels': {'ab.recovery.owner': OWNER}, 'Entrypoint': None}, 'Size': 1000}
    if mode == 'containerd':
        obj['Descriptor'] = copy.deepcopy(desc)
    return obj, metadata, base


def change(obj, path, value):
    for key in path[:-1]:
        obj = obj[key]
    obj[path[-1]] = value


class Controls(unittest.TestCase):
    def test_store_selection(self):
        common = {'OSType': 'linux', 'Architecture': 'x86_64'}
        self.assertEqual(identity.store_mode(dict(common, Driver='overlay2', DriverStatus=[])), 'classic')
        self.assertEqual(identity.store_mode(dict(common, Driver='overlayfs', DriverStatus=[
            ['driver-type', 'io.containerd.snapshotter.v1']])), 'containerd')
        for obj in [
            dict(common, Driver='overlayfs'), dict(common, Driver='overlay2', DriverStatus=[['driver-type', 'unknown']]),
            dict(common, Driver='overlay2', DriverStatus=[['driver-type', 'io.containerd.snapshotter.v1']] * 2),
            dict(common, OSType='windows', Driver='overlay2'),
            dict(common, Architecture='arm64', Driver='overlay2'),
        ]:
            with self.subTest(info=obj), self.assertRaises(ValueError):
                identity.store_mode(obj)

    def test_all_exact_base_pins_in_both_stores(self):
        for mode in ('classic', 'containerd'):
            for name in identity.PINS:
                with self.subTest(mode=mode, name=name):
                    obj = base_object(name, mode)
                    verified = identity.verify_pin(name, obj, mode)
                    self.assertEqual(verified['id'], obj['Id'])
                    obj['RepoDigests'] = [identity.PINS[name][0]]
                    self.assertEqual(identity.verify_pin(name, obj, mode)['id'], verified['id'])

    def test_base_rejects_wrong_manifest_config_platform_layers(self):
        controls = [
            ('classic', ['Id'], ZERO, 'classic local ID'),
            ('classic', ['Id'], identity.PINS['rust'][0].split('@')[1], 'classic local ID'),
            ('containerd', ['Id'], identity.PINS['rust'][1], 'containerd local ID'),
            ('containerd', ['Descriptor', 'digest'], ZERO, 'descriptor digest'),
            ('containerd', ['Descriptor', 'size'], 1, 'descriptor size'),
            ('containerd', ['Descriptor', 'mediaType'], 'application/vnd.oci.image.index.v1+json', 'descriptor mediaType'),
            ('containerd', ['RepoDigests'], ['rust@' + ZERO], 'repository manifest'),
            ('containerd', ['RepoDigests'], ['evil.example/rust@' + identity.PINS['rust'][0].split('@')[1]], 'repository manifest'),
            ('classic', ['Architecture'], 'arm64', 'platform'),
            ('containerd', ['Os'], 'windows', 'platform'),
            ('containerd', ['Variant'], 'v8', 'platform'),
            ('containerd', ['RootFS', 'Layers', 0], ZERO, 'rootfs layers'),
            ('classic', ['RootFS', 'Layers'], [], 'rootfs layers'),
            ('classic', ['Config', 'Cmd'], ['/bin/evil'], 'execution config'),
        ]
        for mode, path, value, reason in controls:
            with self.subTest(mode=mode, path=path, reason=reason):
                obj = base_object('rust', mode)
                change(obj, path, value)
                with self.assertRaisesRegex(ValueError, reason):
                    identity.verify_pin('rust', obj, mode)
        with self.assertRaisesRegex(ValueError, 'classic store'):
            identity.verify_pin('rust', base_object('rust', 'containerd'), 'classic')

    def test_frozen_raw_bytes_and_manifest_config_link(self):
        raw_m = (HERE / 'pins/rust-manifest.json').read_bytes()
        raw_c = (HERE / 'pins/rust-config.json').read_bytes()
        for manifest, config, reason in [(raw_m + b' ', raw_c, 'manifest bytes'),
                                          (raw_m, raw_c + b' ', 'config bytes')]:
            with self.subTest(reason=reason), self.assertRaisesRegex(ValueError, reason):
                identity.pin_bytes('rust', manifest, config)
        # Even a digest-consistent manifest cannot point to a different config.
        manifest = json.loads(raw_m)
        manifest['config']['digest'] = ZERO
        edited = json.dumps(manifest).encode()
        pins = {'rust': ('docker.io/library/rust@' + identity.sha(edited), identity.PINS['rust'][1])}
        with patch.dict(identity.PINS, pins), self.assertRaisesRegex(ValueError, 'manifest/config binding'):
            identity.pin_bytes('rust', edited, raw_c)

    def test_built_image_mapping_old_and_new_exporter(self):
        for mode in ('classic', 'containerd'):
            for annotated in (False, True):
                with self.subTest(mode=mode, annotated=annotated):
                    obj, metadata, base = built_object(mode, annotated)
                    result = identity.verify_build(obj, metadata, CONFIG, mode, base, OWNER)
                    self.assertEqual(result['id'], obj['Id'])
                    self.assertFalse(result['raw_built_config_independently_extracted'])
                    self.assertEqual(result['built_manifest_attested'], mode == 'containerd')
                    if mode == 'classic':
                        self.assertIsNone(result['manifest'])

    def test_build_rejects_mismatched_bindings(self):
        controls = [
            ('metadata', ['containerimage.digest'], ZERO, 'manifest digest'),
            ('metadata', ['containerimage.config.digest'], ZERO, 'iid/config'),
            ('metadata', ['containerimage.descriptor', 'annotations', 'config.digest'], ZERO, 'descriptor/config'),
            ('obj', ['Descriptor', 'annotations', 'config.digest'], ZERO, 'descriptor/config'),
            ('obj', ['Descriptor', 'digest'], ZERO, 'descriptor digest'),
            ('obj', ['Descriptor', 'size'], 499, 'descriptor size'),
            ('obj', ['Id'], CONFIG, 'containerd local ID'),
            ('obj', ['Architecture'], 'arm64', 'platform'),
            ('obj', ['RootFS', 'Layers', 0], ZERO, 'base layers'),
            ('obj', ['RootFS', 'Layers', 1], 'not-a-digest', 'rootfs layers'),
            ('obj', ['Config', 'Labels', 'ab.recovery.owner'], 'wrong', 'owner'),
        ]
        for target, path, value, reason in controls:
            with self.subTest(target=target, path=path):
                obj, metadata, base = built_object('containerd', True)
                change(obj if target == 'obj' else metadata, path, value)
                with self.assertRaisesRegex(ValueError, reason):
                    identity.verify_build(obj, metadata, CONFIG, 'containerd', base, OWNER)
        obj, metadata, base = built_object('containerd')
        metadata['containerimage.descriptor']['mediaType'] = 'application/vnd.oci.image.index.v1+json'
        with self.assertRaisesRegex(ValueError, 'single-platform'):
            identity.verify_build(obj, metadata, CONFIG, 'containerd', base, OWNER)
        obj, metadata, base = built_object('classic')
        obj['Id'] = MANIFEST
        with self.assertRaisesRegex(ValueError, 'classic local ID'):
            identity.verify_build(obj, metadata, CONFIG, 'classic', base, OWNER)
        with self.assertRaisesRegex(ValueError, 'iid/config'):
            identity.verify_build(obj, metadata, MANIFEST, 'classic', base, OWNER)
        obj, metadata, base = built_object('classic')
        metadata['containerimage.digest'] = MANIFEST
        with self.assertRaisesRegex(ValueError, 'classic build config digest'):
            identity.verify_build(obj, metadata, CONFIG, 'classic', base, OWNER)
        # Reject the unrealistic shape used by the preserved first draft.
        obj, metadata, base = built_object('classic')
        metadata['containerimage.descriptor'] = {'digest': MANIFEST}
        with self.assertRaisesRegex(ValueError, 'unexpected classic build descriptor'):
            identity.verify_build(obj, metadata, CONFIG, 'classic', base, OWNER)

    def test_container_uses_only_verified_local_identity(self):
        for mode in ('classic', 'containerd'):
            bound = identity.verify_pin('postgres', base_object('postgres', mode), mode)
            image = bound['id']
            obj = {'Id': CID, 'Image': image, 'Config': {'Image': image}}
            identity.verify_container(obj, CID, image, {image: bound})
            for path, value in [(['Id'], '9' * 64), (['Image'], ZERO), (['Config', 'Image'], ZERO)]:
                bad = copy.deepcopy(obj)
                change(bad, path, value)
                with self.subTest(mode=mode, path=path), self.assertRaises(ValueError):
                    identity.verify_container(bad, CID, image, {image: bound})
            with self.assertRaisesRegex(ValueError, 'unverified'):
                identity.verify_container(obj, CID, image, {})

    def test_readback_precedes_parse_and_identity_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = SimpleNamespace(here=Path(tmp), disk_and_logs=lambda: None)
            ctx.small = lambda command, **kw: SimpleNamespace(returncode=0, stdout='not JSON', stderr='diagnostic')
            with self.assertRaises(json.JSONDecodeError):
                identity.inspect(ctx, 'malformed', ['mock-inspect'])
            raw = json.loads((ctx.here / 'image-identity-readbacks.jsonl').read_text())
            self.assertEqual(raw['stdout'], 'not JSON')
            self.assertEqual(raw['stderr'], 'diagnostic')
            ctx.small = lambda command, **kw: SimpleNamespace(returncode=7, stdout='failed output', stderr='failure')
            with self.assertRaisesRegex(ValueError, 'command failed'):
                identity.observe(ctx, 'failed-command', ['mock-command'])
            self.assertEqual(len((ctx.here / 'image-identity-readbacks.jsonl').read_text().splitlines()), 2)

    def test_build_wrapper_command_and_returned_identity_without_docker(self):
        import qualify
        with tempfile.TemporaryDirectory() as tmp:
            for mode in ('classic', 'containerd'):
                with self.subTest(mode=mode):
                    obj, metadata, base = built_object(mode)
                    folder = Path(tmp) / mode
                    folder.mkdir()
                    ctx = SimpleNamespace(here=folder, source=Path('/source'),
                        predecessor=Path('/predecessor'), attempt={'nonce': OWNER}, image_store=mode,
                        base_images={'debian': base}, verified_images={}, phase_end=1,
                        source_check=lambda: [], record=lambda *args: None, disk_and_logs=lambda: None)
                    commands = []
                    def long(command, *args):
                        commands.append(command)
                        (folder / 'funding.iid').write_text(CONFIG)
                        (folder / 'funding-metadata.json').write_text(json.dumps(metadata))
                    def small(command, **kwargs):
                        self.assertEqual(command[:3], ['docker', 'image', 'inspect'])
                        return SimpleNamespace(returncode=0, stdout=json.dumps([obj]), stderr='')
                    ctx.long, ctx.small = long, small
                    # Any accidental real process would fail the test before execution.
                    with patch.object(subprocess, 'Popen', side_effect=AssertionError('real process forbidden')):
                        result = qualify.build(ctx, Path('/source'), 'Dockerfile.recovery-funding',
                                               'funding', qualify.binding.FUNDING_REVISION)
                    self.assertEqual(result['id'], obj['Id'])
                    self.assertIn(obj['Id'], ctx.verified_images)
                    command = commands[0]
                    self.assertEqual(command[:6], ['docker', 'buildx', 'build', '--builder', 'default', '--platform'])
                    self.assertEqual(command[6], 'linux/amd64')
                    for flag in ('--provenance=false', '--sbom=false', '--load'):
                        self.assertEqual(command.count(flag), 1)
                    self.assertNotIn('--push', command)
                    self.assertIn('rust:1.94-bookworm=docker-image://' + identity.PINS['rust'][0], command)
                    self.assertIn('debian:bookworm-slim=docker-image://' + identity.PINS['debian'][0], command)
                    self.assertIn('BUILDKIT_SYNTAX=' + qualify.FRONTEND, command)

    def test_mismatched_container_never_started(self):
        import image_runtime
        with tempfile.TemporaryDirectory() as tmp:
            base = identity.verify_pin('python', base_object('python', 'containerd'), 'containerd')
            ctx = SimpleNamespace(here=Path(tmp), runtime_ids=[], pg='6' * 64,
                attempt={'nonce': OWNER}, exchange=Path(tmp), helpers=HERE,
                verified_images={base['id']: base}, disk_and_logs=lambda: None)
            calls = []
            def small(command, **kwargs):
                calls.append(command)
                if command[:3] == ['docker', 'ps', '-aq']:
                    out = ''
                elif command[:2] == ['docker', 'create']:
                    out = CID
                elif command == ['docker', 'inspect', CID]:
                    out = json.dumps([{'Id': CID, 'Image': ZERO, 'Config': {'Image': base['id']}}])
                else:
                    raise AssertionError('unexpected command ' + repr(command))
                return SimpleNamespace(returncode=0, stdout=out, stderr='')
            ctx.small = small
            with patch.object(subprocess, 'Popen', side_effect=AssertionError('real process forbidden')):
                with self.assertRaisesRegex(ValueError, 'container image binding'):
                    image_runtime.create(ctx, 'rpc', base['id'], {})
            self.assertFalse(any(command[:2] == ['docker', 'start'] for command in calls))
            self.assertIn(ZERO, (ctx.here / 'image-identity-readbacks.jsonl').read_text())


if __name__ == '__main__':
    unittest.main()
