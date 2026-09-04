#!/usr/bin/env python3
"""Compile exact measurement helpers and oversampling source without private deps.

Uses wide::f32x8 as a disclosed replacement for truce_simd::simd::f32x8. This is
algorithm/regression evidence, not production backend or full-plugin validation.
Requires Rust 1.97.1+ and Cargo registry access (or cached dependencies).
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def extract_function(source, name):
    match = re.search(rf"^(?:pub\(super\) )?fn {name}\b.*?^}}", source, re.M | re.S)
    if match is None:
        raise ValueError(f"Could not extract {name}")
    return match.group().removeprefix("pub(super) ")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--toolchain", help="Optional rustup toolchain, e.g. 1.97.1")
    args = parser.parse_args()
    source = (ROOT / "src/oscillators/va/experiment.rs").read_text()
    helpers = "\n".join(extract_function(source, name) for name in ("db_ratio", "spectral_error"))
    test = extract_function(source, "spectral_metrics_distinguish_coloration_from_off_grid_energy")
    with tempfile.TemporaryDirectory(prefix="kurv-metrics-probe-") as directory:
        project = Path(directory)
        (project / "src").mkdir()
        (project / "Cargo.toml").write_text('''[package]
name = "kurv-metrics-probe"
version = "0.0.0"
edition = "2024"
[dependencies]
wide = "=0.7.33"
rustfft = "=6.4.1"
''')
        (project / "src/lib.rs").write_text(f'''#![allow(dead_code)]
extern crate self as truce_simd;
pub mod simd {{ pub use wide::f32x8; }}
#[path = {json.dumps(str(ROOT / "src/oversampling.rs"))}]
pub mod oversampling;
#[path = {json.dumps(str(ROOT / "src/dsp.rs"))}]
mod dsp;
use dsp::{{Complex, fft}};
{helpers}
#[test]
{test}
''')
        command = [args.cargo]
        if args.toolchain:
            command.append("+" + args.toolchain)
        command += ["test", "--manifest-path", str(project / "Cargo.toml"), "--lib"]
        print("Standalone source probe; wide SIMD substitute; not full plugin validation.", flush=True)
        subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
