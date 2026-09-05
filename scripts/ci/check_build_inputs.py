#!/usr/bin/env python3
"""Fail before Cargo with all missing local manifests and required source modules."""
import argparse
from pathlib import Path
import sys
import tomllib


def missing_inputs(root):
    pending = [root / 'Cargo.toml']
    seen = set()
    missing = set()
    while pending:
        manifest = pending.pop().resolve()
        if manifest in seen:
            continue
        seen.add(manifest)
        if not manifest.is_file():
            missing.add(manifest)
            continue
        data = tomllib.loads(manifest.read_text())
        def scan(value):
            if isinstance(value, dict):
                for key, child in value.items():
                    if key == 'path' and isinstance(child, str):
                        candidate = manifest.parent / child
                        # Targets also have paths; dependency paths are directories.
                        if not child.endswith('.rs'):
                            pending.append(candidate / 'Cargo.toml')
                    else:
                        scan(child)
            elif isinstance(value, list):
                for child in value:
                    scan(child)
        scan(data)
    # The root has an unconditional `mod licensing`; neither variant is tracked
    # in the current base. Report it rather than generating a substitute.
    if not any((root / path).is_file() for path in ('src/licensing.rs', 'src/licensing/mod.rs')):
        missing.add(root / 'src/licensing/mod.rs')
    return sorted(missing)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    missing = missing_inputs(args.root.resolve())
    if missing:
        for path in missing:
            print(f'MISSING build input: {path}', file=sys.stderr)
        print('Restore the authentic pinned dependencies and commit the missing product sources. Optional features do not remove Cargo path resolution.', file=sys.stderr)
        return 1
    print('Required local manifests and licensing entry point are present.')
    return 0

if __name__ == '__main__':
    sys.exit(main())
