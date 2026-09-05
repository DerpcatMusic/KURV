#!/usr/bin/env python3
"""Check tracked first-party Rust files without resolving Cargo dependencies."""
from pathlib import Path
import subprocess
import tomllib

root = Path(__file__).resolve().parents[2]
edition = str(tomllib.loads((root / 'Cargo.toml').read_text())['package']['edition'])
paths = subprocess.check_output(['git', 'ls-files', '-z', '--', '*.rs'], cwd=root).decode().split('\0')
paths = [p for p in paths if p and not p.startswith('vendor/')]
if not paths:
    raise SystemExit('No tracked Rust sources found')
# skip_children avoids Cargo/module discovery; every tracked module is checked explicitly.
failed = False
for offset in range(0, len(paths), 80):
    result = subprocess.run(['rustfmt', '--check', '--edition', edition, '--config',
                             'skip_children=true', *paths[offset:offset+80]], cwd=root)
    failed |= result.returncode != 0
raise SystemExit(int(failed))
