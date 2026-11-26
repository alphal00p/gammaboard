"""Exercise the repository launcher with isolated build tools and asset directories."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class FrontendStartupTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.tools = self.root / 'tools'
        self.tools.mkdir()
        for name in ('launch.sh', 'frontend.py'):
            shutil.copy2(ROOT / 'tools' / name, self.tools / name)
        self.dashboard = self.root / 'dashboard'
        (self.dashboard / 'src').mkdir(parents=True)
        (self.dashboard / 'src/app.js').write_text('version one')
        for name in ('package.json', 'package-lock.json'):
            (self.dashboard / name).write_text('{}')
        self.config = self.root / 'instance.toml'
        self.configure()
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.bash = shutil.which('bash')
        for name in ('dirname', 'python3'):
            (self.bin / name).symlink_to(shutil.which(name))
        self.events = self.root / 'events'
        self.env = dict(os.environ, PATH=str(self.bin), CARGO_TARGET_DIR=str(self.root / 'target'),
                        GAMMABOARD_PROFILE='debug', GB_TEST_EVENTS=str(self.events))
        for name in ('cargo', 'rustc', 'node', 'npm'):
            script = self.bin / name
            script.write_text(f'#!{sys.executable}\n' + r'''
import json, os, sys, time
from pathlib import Path
name=Path(sys.argv[0]).name
args=sys.argv[1:]
with open(os.environ['GB_TEST_EVENTS'],'a') as log: log.write(json.dumps([name,*args])+'\n')
if name=='node': print('v24.0.0')
if name=='cargo':
    binary=Path(os.environ['CARGO_TARGET_DIR'])/'debug'/args[args.index('--bin')+1]
    binary.parent.mkdir(parents=True,exist_ok=True)
    if not binary.exists():
        binary.write_text("#!/bin/sh\nprintf '%s\\n' '{\"ready\":true}'\n")
        binary.chmod(0o700)
if name=='npm':
    print('npm progress on stdout')
    if args[0]=='ci':
        path=Path('node_modules/.bin/vite');path.parent.mkdir(parents=True,exist_ok=True);path.touch()
    else:
        time.sleep(float(os.environ.get('GB_BUILD_DELAY','0')))
        if os.environ.get('GB_FAIL_BUILD'): sys.exit(9)
        output=Path(args[args.index('--outDir')+1]);output.mkdir(parents=True,exist_ok=True)
        output.joinpath('index.html').write_text(Path('src/app.js').read_text())
        output.joinpath('app.js').write_text('bundle')
''')
            script.chmod(0o700)

    def configure(self, extra='', output='dashboard/build'):
        self.config.write_text(f'{extra}\n[paths]\n[database]\n[server]\ndashboard_dir={json.dumps(output)}\n')

    def launch(self, *args, success=True):
        result = subprocess.run([self.bash, str(self.tools / 'launch.sh'), 'gb', '--instance',
                                 str(self.config), *args], cwd='/', env=self.env,
                                capture_output=True, text=True, timeout=15, check=False)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), {'ready': True})
        else:
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn('Traceback', result.stderr)
        return result

    def calls(self, name):
        return [row for row in map(json.loads, self.events.read_text().splitlines()) if row[0] == name] if self.events.exists() else []

    def test_first_start_builds_then_cache_skips_node_and_npm(self):
        result = self.launch('start', '--json')
        self.assertIn('npm progress', result.stderr)
        self.assertEqual([call[1] for call in self.calls('npm')], ['ci', 'run'])
        self.assertEqual((self.dashboard / 'build/index.html').read_text(), 'version one')
        for name in ('node', 'npm'):
            (self.bin / name).unlink()
        self.launch('start', '--json')
        self.launch('restart')
        self.assertEqual(len(self.calls('npm')), 2)

    def test_source_and_lock_changes_rebuild_only_what_changed(self):
        self.launch('start')
        before = (self.dashboard / 'src/app.js').stat()
        (self.dashboard / 'src/app.js').write_text('version two')
        os.utime(self.dashboard / 'src/app.js', ns=(before.st_atime_ns, before.st_mtime_ns))
        self.launch('restart')
        self.assertEqual([call[1] for call in self.calls('npm')], ['ci', 'run', 'run'])
        (self.dashboard / 'package-lock.json').write_text('{"changed":true}')
        self.launch('start', 'server')
        self.assertEqual([call[1] for call in self.calls('npm')], ['ci', 'run', 'run', 'ci', 'run'])
        (self.dashboard / 'build/app.js').unlink()
        self.launch('start')
        self.assertEqual(self.calls('npm')[-1][1], 'run')
        shutil.rmtree(self.dashboard / 'node_modules')
        self.launch('start')  # Current assets need no build toolchain or node_modules.
        self.assertEqual(len(self.calls('npm')), 6)

    def test_skips_api_db_remote_help_and_external_management(self):
        (self.bin / 'node').unlink()
        (self.bin / 'npm').unlink()
        for args in [('start', '--api-only'), ('start', 'db'), ('restart', '--api-only'),
                     ('status',), ('start', '--help'), ('--endpoint', 'http://host', 'start')]:
            self.launch(*args)
        self.configure('externally_managed=true')
        self.launch('start')
        self.assertFalse(self.calls('npm'))
        error = self.launch('--allow-local-processes', 'start', success=False)
        self.assertIn('Missing dependency: node', error.stderr)

    def test_configured_paths_and_prebuilt_assets(self):
        output = self.root / 'custom dashboard'
        self.configure(output=str(output))
        self.launch('start')
        self.assertTrue(output.joinpath('index.html').is_file())
        self.assertFalse((self.dashboard / 'build').exists())
        output.joinpath('.gb-frontend.json').unlink()  # A separately supplied bundle.
        output.joinpath('index.html').write_text('prebuilt')
        (self.dashboard / 'src/app.js').write_text('changed')
        (self.bin / 'node').unlink()
        (self.bin / 'npm').unlink()
        self.launch('start')
        self.assertEqual(output.joinpath('index.html').read_text(), 'prebuilt')
        self.assertEqual(len(self.calls('npm')), 2)

    def test_failed_build_preserves_bundle_and_does_not_start_service(self):
        self.launch('start')
        cargo_before = len(self.calls('cargo'))
        (self.dashboard / 'src/app.js').write_text('broken source')
        self.env['GB_FAIL_BUILD'] = '1'
        error = self.launch('restart', success=False)
        self.assertIn('Frontend command failed', error.stderr)
        self.assertEqual((self.dashboard / 'build/index.html').read_text(), 'version one')
        self.assertEqual(len(self.calls('cargo')), cargo_before)
        del self.env['GB_FAIL_BUILD']
        self.launch('restart')
        self.assertEqual((self.dashboard / 'build/index.html').read_text(), 'broken source')
        self.assertEqual(sum(call[1] == 'ci' for call in self.calls('npm')), 1)

    def test_missing_tools_and_nonempty_custom_directory_fail_clearly(self):
        (self.bin / 'npm').unlink()
        error = self.launch('start', success=False)
        self.assertIn('Missing dependency: npm', error.stderr)
        self.assertIn('--api-only', error.stderr)
        output = self.root / 'unrelated'
        output.mkdir()
        output.joinpath('keep').write_text('untouched')
        self.configure(output=str(output))
        error = self.launch('start', success=False)
        self.assertIn('contains files but no index.html', error.stderr)
        self.assertEqual(output.joinpath('keep').read_text(), 'untouched')
        self.configure(output='dashboard/src')
        error = self.launch('start', success=False)
        self.assertIn('Unsafe dashboard output', error.stderr)
        self.assertEqual((self.dashboard / 'src/app.js').read_text(), 'version one')

    def test_global_option_values_are_not_components(self):
        self.launch('start', '--token-file', 'db')
        self.assertEqual([call[1] for call in self.calls('npm')], ['ci', 'run'])
        (self.dashboard / 'src/app.js').write_text('changed')
        self.launch('--token-file', 'restart', 'start', 'server')
        self.assertEqual([call[1] for call in self.calls('npm')], ['ci', 'run', 'run'])

    def test_concurrent_starts_share_one_install_and_build(self):
        self.launch('start', '--api-only')
        self.env['GB_BUILD_DELAY'] = '0.3'
        with ThreadPoolExecutor(max_workers=2) as pool:
            results = [pool.submit(self.launch, 'start') for _ in range(2)]
            for result in results:
                result.result()
        self.assertEqual([call[1] for call in self.calls('npm')], ['ci', 'run'])
        self.env['GAMMABOARD_FRONTEND_BASE'] = '/board/'
        self.launch('restart')
        self.assertEqual([call[1] for call in self.calls('npm')], ['ci', 'run', 'run'])


if __name__ == '__main__':
    unittest.main()
