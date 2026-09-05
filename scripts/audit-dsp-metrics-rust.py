#!/usr/bin/env python3
"""Run current production measurement and oversampling tests in the shared VA adapter.

The adapter uses wide SIMD and omits framework persistence/resynthesis plumbing.
This is component evidence, not full-plugin validation. See its README.
"""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cargo', default='cargo')
    parser.add_argument('--toolchain', help='Optional rustup toolchain')
    args = parser.parse_args()
    command = [args.cargo]
    if args.toolchain:
        command.append('+' + args.toolchain)
    command += ['test', '--locked', '--release', '--manifest-path', str(ROOT / 'tools/audits/pm_integration/Cargo.toml')]
    # Run the complete adapter suite so a renamed filter cannot silently run zero tests.
    subprocess.run(command, check=True)

if __name__ == '__main__':
    main()
