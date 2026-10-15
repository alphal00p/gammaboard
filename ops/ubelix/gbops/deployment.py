from __future__ import annotations

from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import tomllib
import uuid


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def write_json(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temporary = tempfile.mkstemp(dir=path.parent, prefix='.writing-')
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(data, stream, indent=2)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_DIRECTORY)
        try: os.fsync(directory)
        finally: os.close(directory)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)


def merge(base, override):
    result = dict(base)
    for key, value in override.items():
        result[key] = merge(result[key], value) if isinstance(value, dict) and isinstance(result.get(key), dict) else value
    return result


def toml_dump(value):
    lines = []
    def table(mapping, path):
        if path: lines.append('[' + '.'.join(json.dumps(part) for part in path) + ']')
        for key, item in mapping.items():
            if not isinstance(item, dict):
                lines.append(json.dumps(key) + ' = ' + json.dumps(item, ensure_ascii=False))
        for key, item in mapping.items():
            if isinstance(item, dict):
                lines.append('')
                table(item, [*path, key])
    table(value, [])
    return '\n'.join(lines) + '\n'


class Deployments:
    def __init__(self, workspace, offset):
        self.workspace = Path(workspace).resolve()
        self.offset = offset
        if not 0 <= offset <= 65535 - 8080: raise ValueError('port offset out of range')
        self.root = self.workspace / 'deployments'
        self.slot = hashlib.sha256(f'{self.workspace}\0{offset}'.encode()).hexdigest()[:16]
        self.pointer = self.root / f'active-{self.slot}.json'

    @contextmanager
    def locked(self):
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        fd = os.open(self.root / f'{self.slot}.lock', os.O_CREAT | os.O_RDWR, 0o600)
        with os.fdopen(fd, 'w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            yield

    def load(self):
        if not self.pointer.exists(): return None
        record = json.loads(self.pointer.read_text())
        return self.by_id(record['id'])

    def by_id(self, identifier):
        if not isinstance(identifier, str) or not identifier.isalnum(): raise ValueError('invalid deployment id')
        manifest = json.loads((self.root / identifier / 'manifest.json').read_text())
        if manifest['workspace'] != str(self.workspace) or manifest['port_offset'] != self.offset:
            raise ValueError('deployment does not belong to this workspace/offset')
        return manifest

    def save(self, manifest):
        write_json(self.root / manifest['id'] / 'manifest.json', manifest)

    def selected_image(self, path=None):
        pointer = Path(path) if path else self.workspace / 'images/gammaboard/current.json'
        info = json.loads(pointer.read_text())
        image = Path(info['path'])
        if not image.is_absolute(): image = (pointer.parent / image).resolve()
        if digest(image) != info['sha256']: raise ValueError('image digest does not match its manifest')
        if not info.get('validated') or not info.get('build_info', {}).get('build_id'): raise ValueError('image has no validated build identity')
        return {**info, 'path': str(image)}

    def configuration(self, single, local_port, server_override=None, runtime_override=None):
        base = self.workspace / 'ops/config' / ('server-single-node.toml' if single else 'server.toml')
        server = tomllib.loads(base.read_text())
        override = Path(server_override) if server_override else self.workspace / 'config/server.toml'
        if server_override and not override.is_file(): raise ValueError('private server configuration does not exist')
        if override.exists():
            if any(override.resolve().is_relative_to(self.workspace / folder) for folder in ('ops','docs','resources/templates','gbops')): raise ValueError('private overrides must be outside managed sync directories')
            extra = tomllib.loads(override.read_text())
            build_dir = extra.get('frontend', {}).get('build_dir')
            if build_dir and not Path(build_dir).is_absolute(): extra['frontend']['build_dir'] = str((override.parent / build_dir).resolve())
            server = merge(server, extra)
        runtime_path = Path(runtime_override) if runtime_override else self.workspace / 'config/runtime.toml'
        if runtime_override and not runtime_path.is_file(): raise ValueError('private runtime configuration does not exist')
        if runtime_path.exists() and any(runtime_path.resolve().is_relative_to(self.workspace / folder) for folder in ('ops','docs','resources/templates','gbops')): raise ValueError('private overrides must be outside managed sync directories')
        runtime = tomllib.loads(runtime_path.read_text()) if runtime_path.exists() else {}
        # Ports and launch mode belong to the deployment, and are supplied explicitly.
        if server.get('api_port', 4000) != 4000 or server.get('frontend', {}).get('port', 8080) != 8080:
            raise ValueError('use --port-offset instead of overriding base server ports')
        if 'url' in runtime.get('database', {}):
            raise ValueError('cluster deployments manage their database URL; configure PostgreSQL tuning only')
        server['allow_local_node_spawn'] = single
        origins = [f'http://localhost:{local_port}', f'http://127.0.0.1:{local_port}']
        data = {'server': server, 'runtime': runtime, 'origins': origins}
        data['hash'] = hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()
        return data

    def create(self, image, config, single, resume):
        identifier = uuid.uuid4().hex
        directory = self.root / identifier
        directory.mkdir(parents=True, mode=0o700)
        # Queued jobs must keep their launcher even after managed ops are synced.
        launcher = directory / 'launcher'
        launcher.mkdir(mode=0o700)
        shutil.copy2(self.workspace / 'ubelix.py', launcher / 'ubelix.py')
        shutil.copytree(self.workspace / 'gbops', launcher / 'gbops', ignore=shutil.ignore_patterns('__pycache__'))
        for name in ('control', 'worker'):
            shutil.copy2(self.workspace / f'ops/slurm/{name}.sbatch', launcher / f'{name}.sbatch')
        for name in ('server', 'runtime'):
            path = directory / f'{name}.toml'
            path.write_text(toml_dump(config[name]))
            path.chmod(0o600)
        manifest = {'version': 1, 'id': identifier, 'workspace': str(self.workspace),
                    'port_offset': self.offset, 'single_node': single, 'resume_workers': resume,
                    'created_at': datetime.now(timezone.utc).isoformat(), 'image': image,
                    'config_hash': config['hash'], 'origins': config['origins'],
                    'server_config': str(directory / 'server.toml'), 'runtime_config': str(directory / 'runtime.toml'),
                    'server_digest': digest(directory / 'server.toml'), 'runtime_digest': digest(directory / 'runtime.toml'),
                    'control_job_id': None, 'control_submission': 'prepared', 'state': 'starting'}
        self.save(manifest)
        write_json(self.pointer, {'id': identifier})
        return manifest


def tag(manifest, suffix):
    return f'gb:{manifest["id"]}:{suffix}'


def job_name(manifest, suffix):
    return f'gb-{manifest["id"]}-{suffix}'


def owns(job, manifest, suffix=None):
    if suffix is None:
        return job.name.startswith(job_name(manifest, '')) and (not job.comment or job.comment.startswith(tag(manifest, '')))
    return job.name == job_name(manifest, suffix) and (not job.comment or job.comment == tag(manifest, suffix))
