#!/usr/bin/env python3
"""Run selected actual KURV DSP source without plugin/UI/private dependencies.

The harness extracts method bodies verbatim; it does not translate their DSP.
wide's f32x4/f32x8 are re-exported under the truce_simd import used by KURV.
Run portable and native separately to exercise both coefficient layouts.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile


def item(source, marker):
    start = source.index(marker)
    brace = source.index('{', start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-root', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--cargo', default='cargo')
    parser.add_argument('--toolchain', default=None)
    parser.add_argument('--native', action='store_true')
    args = parser.parse_args()
    root = args.source_root.resolve()
    curve = (root / 'src/wave_curve.rs').read_text()
    function = (root / 'src/wave_curve/function.rs').resolve()
    avx_cfg = '#[cfg(all(target_arch="x86_64",target_feature="avx2",target_feature="fma"))]'
    scalar_cfg = '#[cfg(not(all(target_arch="x86_64",target_feature="avx2",target_feature="fma")))]'
    methods = ['pub const fn zero(', 'pub(crate) const fn from_coefficients(',
               'pub(crate) fn with_function(', 'pub fn interpolate(', 'fn eval_raw(',
               'pub fn eval(', 'pub fn eval4(', 'pub fn eval8(']
    definitions = '\n'.join(item(curve, name) for name in methods)
    definitions += '\n' + avx_cfg + '\n' + item(curve, 'fn eval8_avx2(')
    definitions += '\n' + scalar_cfg + '\n' + item(curve, 'fn select4(')
    rust = '''#![allow(dead_code)]
extern crate self as truce_simd;
pub mod simd { pub use wide::{f32x4, f32x8}; }
'''
    rust += f'#[path = "{function}"]\nmod function;\n'
    rust += 'mod wave_curve {\nuse crate::function;\nuse function::VaFunctionRt;\nuse crate::simd::{f32x4,f32x8};\n'
    rust += 'const RT_SEGMENTS:usize=16; const COEFFICIENTS_PER_SEGMENT:usize=4; const RT_VALUES:usize=64;\n'
    rust += item(curve, 'const fn coefficient_index(')
    rust += '\n#[derive(Clone,Copy,Debug,PartialEq)]\n' + item(curve, 'pub struct WaveCurveRt')
    rust += '\nimpl WaveCurveRt {\n' + definitions + '\n}\n'
    # Proof modules come from this PR even when --source-root points to the old tree.
    proof_curve = (Path(__file__).resolve().parents[2] / 'src/wave_curve.rs').read_text()
    rust += '\n#[cfg(test)]\n' + item(proof_curve, 'mod function_morph_parity_tests') + '\n}\n'
    proof_function = (Path(__file__).resolve().parents[2] / 'src/wave_curve/function.rs').read_text()
    if 'mod parity_tests' not in function.read_text():
        rust += '\n#[cfg(test)]\n' + item(proof_function, 'mod parity_tests').replace('use super::compile_expression;', 'use crate::function::compile_expression;')
    with tempfile.TemporaryDirectory(prefix='kurv-custom-proof-') as directory:
        directory = Path(directory)
        (directory / 'src').mkdir()
        (directory / 'Cargo.toml').write_text('[package]\nname="kurv-custom-source-proof"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nwide="=0.7.33"\n')
        (directory / 'src/lib.rs').write_text(rust)
        env = dict(os.environ)
        env['RUSTFLAGS'] = '-C target-cpu=native' if args.native else ''
        command = [args.cargo] + ([f'+{args.toolchain}'] if args.toolchain else [])
        subprocess.run(command + ['test', '--manifest-path', str(directory / 'Cargo.toml'), '--', '--nocapture'], env=env, check=True)


if __name__ == '__main__':
    main()
