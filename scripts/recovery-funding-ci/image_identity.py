"""Store-aware image bindings for synthetic CI qualification, with raw readbacks."""
import hashlib
import json
import re
from pathlib import Path

MANIFEST_TYPES = {
    'application/vnd.oci.image.manifest.v1+json',
    'application/vnd.docker.distribution.manifest.v2+json',
}
CONFIG_TYPES = {
    'application/vnd.oci.image.config.v1+json',
    'application/vnd.docker.container.image.v1+json',
}
PINS = {
    'rust': ('docker.io/library/rust@sha256:4ec71e955e6c08aeb238885083222ddff79d82eb87654a96c76e38e94da1a53b', 'sha256:1c3316753323aa5edd4c2be762323794f4aa9f4335635dc4c7964ee99cb986d2'),
    'debian': ('docker.io/library/debian@sha256:c67e92c8099ba22853ab4cfda7cfb494d253d6012ca82dee98ba8976772af018', 'sha256:43f84a0b210f98845ef003912981774c95efe439eaa4b1cb62cf803fd46fc82a'),
    'postgres': ('docker.io/library/postgres@sha256:43f9275df91e012c3da3d86bea5d7bb5e67a473296cc906c5f9e80b8176b1a37', 'sha256:ba07bdb15f0f2ac1ba7a88891da731cae65ac36d663fdb7de8f698ea47e0e381'),
    'python': ('docker.io/library/python@sha256:05a994d2344eb364cc186910f4fc8786a38215f39f00d0acc25bff33a08f4177', 'sha256:78976a69c067520ef8be34cde307ef9f6fea6dcefa50edb57039773d241ca0db'),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(value):
    return 'sha256:' + hashlib.sha256(value).hexdigest()


def valid_digest(value):
    return isinstance(value, str) and re.fullmatch(r'sha256:[0-9a-f]{64}', value) is not None


def observe(ctx, label, command):
    """Persist bounded, complete output before parsing or asserting its identity."""
    result = ctx.small(command, check=False)
    with (ctx.here / 'image-identity-readbacks.jsonl').open('a') as target:
        target.write(json.dumps({'label': label, 'command': command,
                                'returncode': result.returncode,
                                'stdout': result.stdout, 'stderr': result.stderr}) + '\n')
    ctx.disk_and_logs()
    require(result.returncode == 0, label + ': command failed; see raw identity readback')
    return result.stdout


def inspect(ctx, label, command):
    value = json.loads(observe(ctx, label, command))
    require(isinstance(value, list) and len(value) == 1, label + ': expected one inspect object')
    return value[0]


def store_mode(info):
    require(info.get('OSType') == 'linux', 'Docker server OS must be linux')
    require(info.get('Architecture') in ('amd64', 'x86_64'), 'Docker server architecture must be amd64')
    pairs = info.get('DriverStatus') or []
    require(isinstance(pairs, list) and all(isinstance(p, list) and len(p) == 2 for p in pairs),
            'invalid Docker driver status')
    values = [p[1] for p in pairs if p[0] == 'driver-type']
    if values == ['io.containerd.snapshotter.v1']:
        return 'containerd'
    require(not values and info.get('Driver') == 'overlay2', 'unsupported Docker image store')
    return 'classic'


def platform(obj):
    require(obj.get('Os') == 'linux' and obj.get('Architecture') == 'amd64'
            and not obj.get('Variant') and not obj.get('OsVersion'), 'image platform mismatch')


def descriptor(actual, expected):
    require(isinstance(actual, dict), 'missing manifest descriptor')
    require(expected.get('mediaType') in MANIFEST_TYPES, 'expected single-platform image manifest')
    for key in ('digest', 'size', 'mediaType'):
        require(actual.get(key) == expected[key], 'manifest descriptor ' + key + ' mismatch')


def local_id(obj, mode, expected, config_digest):
    require(valid_digest(config_digest), 'invalid config digest')
    if mode == 'containerd':
        descriptor(obj.get('Descriptor'), expected)
        require(obj.get('Id') == expected['digest'], 'containerd local ID must equal manifest digest')
    else:
        require(mode == 'classic', 'unknown store mode')
        require(not obj.get('Descriptor'), 'classic store unexpectedly returned a manifest descriptor')
        require(obj.get('Id') == config_digest, 'classic local ID must equal config digest')
    return obj['Id']


def rootfs(obj):
    fs = obj.get('RootFS') or {}
    layers = fs.get('Layers')
    require(fs.get('Type') == 'layers' and isinstance(layers, list)
            and 0 < len(layers) <= 128 and all(valid_digest(layer) for layer in layers),
            'invalid image rootfs layers')
    return layers


def normalized_ref(ref):
    # Docker's familiar name omits docker.io/library for official images.
    require(isinstance(ref, str), 'invalid repository digest')
    if ref.startswith('docker.io/'):
        ref = ref[len('docker.io/'):]
    if ref.startswith('library/'):
        ref = ref[len('library/'):]
    return ref


def pin_bytes(name, manifest_bytes, config_bytes):
    reference, config_digest = PINS[name]
    manifest_digest = reference.split('@')[1]
    require(sha(manifest_bytes) == manifest_digest, name + ': frozen manifest bytes mismatch')
    require(sha(config_bytes) == config_digest, name + ': frozen config bytes mismatch')
    manifest, config = json.loads(manifest_bytes), json.loads(config_bytes)
    require(manifest.get('schemaVersion') == 2 and manifest.get('mediaType') in MANIFEST_TYPES,
            name + ': invalid pinned manifest')
    cfg = manifest.get('config') or {}
    require(cfg.get('digest') == config_digest and cfg.get('size') == len(config_bytes)
            and cfg.get('mediaType') in CONFIG_TYPES, name + ': manifest/config binding mismatch')
    require(config.get('os') == 'linux' and config.get('architecture') == 'amd64'
            and not config.get('variant') and not config.get('os.version'), name + ': pinned platform mismatch')
    fs = config.get('rootfs') or {}
    require(fs.get('type') == 'layers' and isinstance(fs.get('diff_ids'), list)
            and len(fs['diff_ids']) == len(manifest.get('layers', [])), name + ': pinned layers mismatch')
    expected = {'digest': manifest_digest, 'size': len(manifest_bytes), 'mediaType': manifest['mediaType']}
    return reference, config_digest, expected, config


def verify_pin(name, obj, mode, directory=None):
    folder = Path(directory) if directory else Path(__file__).with_name('pins')
    reference, config_digest, expected, config = pin_bytes(
        name, (folder / (name + '-manifest.json')).read_bytes(),
        (folder / (name + '-config.json')).read_bytes())
    platform(obj)
    require(normalized_ref(reference) in [normalized_ref(r) for r in obj.get('RepoDigests') or []],
            name + ': pinned repository manifest absent')
    ident = local_id(obj, mode, expected, config_digest)
    require(rootfs(obj) == config['rootfs']['diff_ids'], name + ': rootfs layers mismatch')
    defaults = {'User': '', 'Env': [], 'Entrypoint': [], 'Cmd': [], 'WorkingDir': '',
                'Labels': {}, 'ExposedPorts': {}, 'Volumes': {}, 'StopSignal': '',
                'Healthcheck': {}, 'Shell': [], 'OnBuild': []}
    for key, default in defaults.items():
        require((obj.get('Config', {}).get(key) or default) == (config.get('config', {}).get(key) or default),
                name + ': execution config ' + key + ' mismatch')
    return {'id': ident, 'store': mode, 'manifest': expected, 'config_digest': config_digest,
            'rootfs': rootfs(obj), 'reference': reference, 'basis': 'frozen raw manifest/config bytes'}


def verify_build(obj, metadata, iid, mode, base, owner):
    platform(obj)
    config_digest = metadata.get('containerimage.config.digest')
    iid_kind = 'config'
    config_source = 'exporter metadata'
    if mode == 'classic':
        require(valid_digest(config_digest) and iid == config_digest, 'build iid/config digest mismatch')
        # Moby's classic default-driver exporter returns the config ID in both
        # digest fields, and no manifest descriptor. Do not invent a manifest.
        require('containerimage.descriptor' not in metadata, 'unexpected classic build descriptor')
        require(metadata.get('containerimage.digest') == config_digest, 'classic build config digest mismatch')
        expected = None
    else:
        require(mode == 'containerd', 'unknown store mode')
        expected = metadata.get('containerimage.descriptor') or {}
        require(valid_digest(metadata.get('containerimage.digest'))
                and expected.get('digest') == metadata['containerimage.digest'], 'build manifest digest mismatch')
        require(isinstance(expected.get('size'), int) and expected['size'] > 0, 'invalid build manifest size')
        # Bind the actual loaded manifest and every reported platform first.
        descriptor(obj.get('Descriptor'), expected)
        require(obj.get('Id') == expected['digest'], 'containerd local ID must equal manifest digest')
        for desc in (expected, obj['Descriptor']):
            if 'platform' in desc:
                require(desc['platform'] == {'os': 'linux', 'architecture': 'amd64'},
                        'build descriptor platform mismatch')
        if 'containerimage.config.digest' in metadata:
            require(valid_digest(config_digest) and iid == config_digest, 'build iid/config digest mismatch')
        else:
            # Buildx 0.38.1 removes the config field when prefer-image-digest
            # applies to --load. getImageID then writes the manifest to iid.
            require(iid == expected['digest'], 'build manifest iid mismatch')
            config_digest = (obj['Descriptor'].get('annotations') or {}).get('config.digest')
            require(valid_digest(config_digest), 'missing or invalid loaded config digest annotation')
            iid_kind, config_source = 'manifest', 'loaded descriptor annotation'
        for desc in (expected, obj['Descriptor']):
            annotations = desc.get('annotations') or {}
            if 'config.digest' in annotations:
                require(annotations['config.digest'] == config_digest, 'build descriptor/config digest mismatch')
    ident = local_id(obj, mode, expected, config_digest)
    layers = rootfs(obj)
    require(len(layers) > len(base['rootfs']) and layers[:len(base['rootfs'])] == base['rootfs'],
            'built image base layers mismatch')
    config = obj.get('Config') or {}
    require((config.get('Labels') or {}).get('ab.recovery.owner') == owner, 'build owner label mismatch')
    require(not config.get('Entrypoint'), 'unexpected build entrypoint')
    require(isinstance(obj.get('Size'), int) and 0 < obj['Size'] <= 3 * 1024**3, 'built image size limit')
    # New layers/config are attested by Buildx metadata + Docker; this is not
    # an independent raw config or full image archive extraction.
    return {'id': ident, 'store': mode, 'manifest': expected, 'config_digest': config_digest,
            'rootfs': layers, 'basis': 'Buildx exporter metadata and Docker inspect',
            'built_manifest_attested': mode == 'containerd',
            'iid_kind': iid_kind, 'config_digest_source': config_source,
            'raw_built_config_independently_extracted': False}


def verify_container(obj, cid, expected_image, verified_images):
    require(expected_image in verified_images, 'container requested an unverified image')
    require(obj.get('Id') == cid, 'container ID mismatch')
    require(obj.get('Image') == expected_image, 'container image binding mismatch')
    require((obj.get('Config') or {}).get('Image') == expected_image, 'container requested image mismatch')
