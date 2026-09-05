#!/usr/bin/env python3
"""Run current-source correctness checks; never interpret elapsed time as DSP CPU."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def sources():
    paths = [ROOT / 'src' / name for name in
             ('dsp.rs', 'oversampling.rs', 'performance.rs', 'wave_curve.rs')]
    helper = ROOT / 'src/oversampling_test_spectrum.rs'
    if helper.exists():
        paths.append(helper)
    for folder in (ROOT/'src/wave_curve', ROOT/'src/oscillators/va', HERE/'src'):
        paths += sorted(folder.rglob('*.rs'))
    paths += [HERE/'build.rs', HERE/'Cargo.toml', HERE/'Cargo.lock', HERE/'run.py']
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(set(paths))}

def command(args, env=None):
    result = subprocess.run(args, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT)
    print(result.stdout, end='', flush=True)
    if result.returncode:
        raise RuntimeError(f'command failed ({result.returncode}): {args}')
    return {'command': args, 'output': result.stdout}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cargo', default=str(Path.home()/'.cargo/bin/cargo'))
    parser.add_argument('--toolchain', default='1.97.1')
    parser.add_argument('--seeds', nargs='+', type=int, default=[1, 8675309, 82829825679398])
    parser.add_argument('--output', type=Path, default=HERE/'results.json')
    parser.add_argument('--verify-record', type=Path)
    args = parser.parse_args()
    if args.verify_record:
        recorded = json.loads(args.verify_record.read_text())['source_sha256']
        actual = sources()
        changed = sorted(k for k in recorded.keys() | actual.keys() if recorded.get(k) != actual.get(k))
        if changed:
            raise SystemExit('STALE results; changed source inputs: '+', '.join(changed))
        print('Recorded source hashes match current harness and DSP inputs.')
        return
    if not args.seeds or any(seed <= 0 or seed >= 2**64 for seed in args.seeds):
        parser.error('seeds must be in 1..2^64-1')
    cargo = [args.cargo, '+'+args.toolchain]
    manifest = ['--manifest-path', str(HERE/'Cargo.toml')]
    result = {'scope': 'VA correctness, not whole voice/plugin or CPU profiling',
              'platform': platform.platform(), 'seeds': args.seeds,
              'rustflags': os.environ.get('RUSTFLAGS', ''),
              'logical_cpus': os.cpu_count(),
              'source_sha256': sources(), 'runs': []}
    result['compiler'] = command([str(Path(args.cargo).with_name('rustc')), '+'+args.toolchain, '-vV'])['output']
    result['runs'].append(command(cargo+['test','--release','--locked','--offline']+manifest))
    result['runs'].append(command(cargo+['build','--release','--locked','--offline']+manifest))
    binary = HERE/'target/release/kurv-pm-integration'
    result['binary_sha256'] = digest(binary)
    for backend in ('baseline','avx2'):
        for seed in args.seeds:
            env = dict(os.environ, KURV_SIMD=backend, KURV_TEST_SEED=str(seed))
            run = command([str(binary)], env)
            run.update(requested_backend=backend, seed=seed)
            result['runs'].append(run)
    if result['source_sha256'] != sources():
        raise RuntimeError('source changed during run; discard result and rerun')
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2)+'\n')

if __name__ == '__main__':
    main()
