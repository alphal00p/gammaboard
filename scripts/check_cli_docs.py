#!/usr/bin/env python3
"""Reject obsolete CLI spellings in maintained guides, templates and errors."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# Global flags can precede commands; quoted paths are common in shell examples.
VALUE = r'''(?:"[^"\n]*"|'[^'\n]*'|[^\s`]+)'''
GLOBAL_FLAGS = (
    rf'(?:\s+--(?:json|allow-local-processes)\b'
    rf'|\s+--(?:instance|endpoint|token-file|runtime-config|database-url)'
    rf'(?:=|\s+){VALUE})*\s+'
)
SERVICE = rf'(?<![\w-])(?:\./)?gammaboard{GLOBAL_FLAGS}'
CLIENT = rf'(?<![\w-])(?:\./)?gb{GLOBAL_FLAGS}'
OBSOLETE = re.compile(
    SERVICE + r'(?:deploy\b|server\b|run\s|node\s+(?:list|start|stop|assign)\b)'
    + '|'
    + r'--wait-for-workers\b|'
    + CLIENT + r'(?:db\s+status\b|local\s+(?:db\s+start|check-config|check-run)\b'
    r'|worker\s+(?:launch-request\b|list\b[^`\n]*--offline\b)'
    r'|run\s+(?:resume\b|task\s+inspect\b|results\b[^`\n]*--output\b'
    r'|export\s+RUN(?![^`\n]*--kind)))'
)


def maintained_paths():
    yield ROOT / 'README.md'
    yield ROOT / 'dashboard/README.md'
    yield ROOT / 'src/server/auth.rs'
    yield ROOT / 'src/config_defaults/run.toml'
    for folder in ['docs', 'process_api', 'resources', 'ops']:
        for path in (ROOT / folder).rglob('*'):
            relative = path.relative_to(ROOT)
            # Reports document the software and commands used at their revision.
            historical = relative.parts[:2] in [('docs', 'validation'), ('docs', 'benchmarks')]
            if path.suffix in ('.md', '.toml') and not historical:
                yield path


def main():
    errors = []
    for path in maintained_paths():
        # Treat continued commands as one line while retaining the start line number.
        pending = ''
        start = 1
        for number, line in enumerate(path.read_text().splitlines(), 1):
            if not pending:
                start = number
            pending += line.rstrip().removesuffix('\\') + ' '
            if line.rstrip().endswith('\\'):
                continue
            if OBSOLETE.search(pending):
                errors.append(f'{path.relative_to(ROOT)}:{start}: {pending.strip()}')
            pending = ''
    if errors:
        raise SystemExit('\n'.join(errors))
    print('Current CLI command documentation checked.')


if __name__ == '__main__':
    main()
