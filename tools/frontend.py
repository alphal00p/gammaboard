"""Prepare checkout dashboard assets before local startup; no system package installs."""
import fcntl
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import tomllib

STAMP = '.gb-frontend.json'


def wants_frontend(args):
    if any(arg in ('--api-only', '--help', '-h', '--version', '-V', '--endpoint')
           or arg.startswith('--endpoint=') for arg in args):
        return False
    values = ('--instance', '--token-file', '--workers', '--timeout', '--allowed-origin')
    flags = ('--json', '--allow-local-processes', '--wait', '--restore-workers', '--foreground')
    remaining = iter(args)
    command = None
    for arg in remaining:
        if arg in values:
            next(remaining, None)
        elif any(arg.startswith(option + '=') for option in values) or arg in flags:
            continue
        elif command is None:
            if arg not in ('start', 'restart'):
                return False
            command = arg
        elif arg != 'server' or command != 'start':
            return False  # Database-only or invalid command; let the CLI handle it.
    return command is not None


def fingerprint(files, base, extra=None):
    entries = [(str(path.relative_to(base)), hashlib.sha256(path.read_bytes()).hexdigest())
               for path in sorted(files)]
    return hashlib.sha256(json.dumps([entries, extra], sort_keys=True).encode()).hexdigest()


def read_stamp(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, ValueError):
        return None


def run(command, cwd):
    result = subprocess.run(command, cwd=cwd, stdout=sys.stderr, stderr=sys.stderr, check=False)
    if result.returncode:
        raise RuntimeError(f'Frontend command failed (exit {result.returncode}): {" ".join(command)}. '
                           'See the output above; fix the error or use --api-only.')


def prepare(repo, instance, args):
    if not wants_frontend(args):
        return
    with instance.open('rb') as source:
        config = tomllib.load(source)
    if not all(key in config for key in ('paths', 'database')):
        return  # The CLI reports incomplete/client-only deployment configuration.
    if config.get('externally_managed', False) and '--allow-local-processes' not in args:
        return  # The CLI owns the process gate; never run npm before it.
    source = repo / 'dashboard'
    server = config.get('server', {})
    if not isinstance(server, dict) or not isinstance(server.get('frontend', {}), dict):
        raise TypeError('server and server.frontend must be TOML tables.')
    directory = server.get('dashboard_dir', server.get('frontend', {}).get('build_dir', 'dashboard/build'))
    if not isinstance(directory, str) or not directory.strip():
        raise RuntimeError('server.dashboard_dir must be a nonempty path string.')
    output = (instance.parent / directory).resolve()
    default_output = (source / 'build').resolve()
    if output == source or output in source.parents or any(output.is_relative_to(source / folder)
                                                          for folder in ('src', 'public', 'node_modules', '.cache')):
        raise RuntimeError(f'Unsafe dashboard output directory {output}; choose a separate build directory.')
    # Explicitly supplied prebuilt bundles remain owned by the deployment.
    if output != default_output and (output / 'index.html').is_file() and not (output / STAMP).exists():
        return
    if output.exists() and output != default_output and not (output / STAMP).exists() and any(output.iterdir()):
        raise RuntimeError(f'Dashboard directory {output} contains files but no index.html. '
                           'Choose an empty build directory, install a prebuilt dashboard, or use --api-only.')
    cache = source / '.cache' / 'gb'
    cache.mkdir(parents=True, exist_ok=True)
    with (cache / 'frontend.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        files = [path for folder in ('src', 'public') for path in (source / folder).rglob('*') if path.is_file()]
        files += [path for path in source.iterdir() if path.is_file() and
                  (path.name in ('package.json', 'package-lock.json', '.npmrc', 'index.html')
                   or path.name.startswith(('vite.config.', '.env')))]
        environment = {key: value for key, value in os.environ.items()
                       if key.startswith('VITE_') or key in ('GAMMABOARD_FRONTEND_BASE', 'NODE_ENV')}
        build_key = fingerprint(files, source, environment)
        previous = read_stamp(output / STAMP)
        if (isinstance(previous, dict) and previous.get('source') == build_key
                and (output / 'index.html').is_file() and previous.get('files')
                and all((output / name).is_file() for name in previous['files'])):
            return
        for tool in ('node', 'npm'):
            if not shutil.which(tool):
                raise RuntimeError(f'Missing dependency: {tool}. Install Node.js/npm to build the dashboard, '
                                   'provide prebuilt assets, or use --api-only.')
        node = subprocess.run(['node', '--version'], capture_output=True, text=True, check=True).stdout.strip()
        packages = [source / 'package.json', source / 'package-lock.json']
        if (source / '.npmrc').is_file():
            packages.append(source / '.npmrc')
        dependency_key = fingerprint(packages, source, [node, sys.platform, platform.machine()])
        installed = source / 'node_modules' / STAMP
        if read_stamp(installed) != dependency_key or not (source / 'node_modules/.bin/vite').exists():
            print('Installing dashboard dependencies...', file=sys.stderr)
            # Remove the success marker before npm may modify node_modules.
            installed.unlink(missing_ok=True)
            run(['npm', 'ci', '--include=dev', '--no-audit', '--no-fund'], source)
            installed.write_text(json.dumps(dependency_key))
        print(f'Building dashboard for {output}...', file=sys.stderr)
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.gb-frontend-build-', dir=output.parent) as directory:
            staged = Path(directory)
            run(['npm', 'run', 'build', '--', '--outDir', str(staged)], source)
            if not (staged / 'index.html').is_file():
                raise RuntimeError('Frontend build produced no index.html; startup aborted.')
            artifacts = [str(path.relative_to(staged)) for path in staged.rglob('*') if path.is_file()]
            (staged / STAMP).write_text(json.dumps({'source': build_key, 'files': artifacts}))
            # Keep the served bundle intact until compilation succeeds.
            backup = None
            if output.exists():
                backup = Path(tempfile.mkdtemp(prefix='.gb-frontend-old-', dir=output.parent))
                backup.rmdir()
                output.rename(backup)
            try:
                staged.rename(output)
            except OSError:
                if backup is not None:
                    backup.rename(output)
                raise
            if backup is not None:
                try:
                    shutil.rmtree(backup)
                except OSError as error:
                    print(f'Dashboard updated; previous assets retained at {backup}: {error}', file=sys.stderr)


if __name__ == '__main__':
    try:
        prepare(Path(__file__).resolve().parents[1], Path(sys.argv[1]).resolve(), sys.argv[2:])
    except (OSError, ValueError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        sys.exit(f'Dashboard preparation failed: {error}')
