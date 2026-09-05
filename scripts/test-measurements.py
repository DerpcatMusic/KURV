#!/usr/bin/env python3
"""Run with python3 scripts/test-measurements.py; requires numpy and rustc."""

from pathlib import Path
import re
import subprocess
import tempfile

import numpy as np


root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory() as temporary:
    temporary = Path(temporary)
    raw = temporary / "render.f32"

    def analyze(signal, fft_bin):
        np.asarray(signal, dtype="<f4").tofile(raw)
        return subprocess.run(
            ["python3", str(root / "scripts/analyze-custom-event.py"),
             str(raw), str(fft_bin), str(len(signal))],
            capture_output=True, text=True, timeout=5,
        )

    for fft_bin in (0, -1, 32, 100):
        assert analyze(np.zeros(64), fft_bin).returncode == 2
    assert "no expected harmonic energy" in analyze(np.zeros(64), 1).stderr
    assert "invalid render" in analyze([0, float("nan"), 0, 0], 1).stderr
    exact = analyze([0, 1, 0, -1], 1)
    assert exact.returncode == 0 and "nonharmonic_residual_dbc=-inf" in exact.stdout

    n, k = 4096, 16
    t = np.arange(n) / n
    triangle = sum(
        8 / np.pi**2 * (-1)**((h - 1) // 2) / h**2 * np.sin(2 * np.pi * k * h * t)
        for h in range(1, n // 2 // k, 2)
    )
    residuals = []
    for signal in (triangle, triangle + 0.1 * np.sin(2 * np.pi * 23 * t)):
        result = analyze(signal, k)
        assert result.returncode == 0, result.stderr
        metrics = dict(item.split("=", 1) for item in result.stdout.strip().split(","))
        residuals.append(float(metrics["nonharmonic_residual_dbc"]))
    assert residuals[1] > residuals[0] + 60, residuals

    # Compile the actual accumulator body; fail closed if its source shape changes.
    source = (root / "examples/process_lab.rs").read_text()
    bodies = re.findall(
        r"for sample in buffer.output\(channel\) \{(.*?)\n                \}",
        source, re.S,
    )
    assert len(bodies) == 1, "update extraction for the current process_lab loop"
    check = temporary / "check.rs"
    check.write_text("""
fn measure(blocks: &[&[f32]]) -> (bool, f32) {
    let mut finite = true; let mut peak = 0.0_f32;
    let mut stream_sum = 0.0_f64; let mut stream_energy = 0.0_f64;
    for block in blocks { for sample in *block { BODY } }
    std::hint::black_box((stream_sum, stream_energy)); (finite, peak)
}
fn main() {
    assert_eq!(measure(&[&[2.0], &[0.25]]), (true, 2.0));
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(!measure(&[&[bad], &[0.25]]).0);
    }
    assert_eq!(measure(&[&[0.0], &[0.0]]), (true, 0.0));
}
""".replace("BODY", bodies[0]))
    executable = temporary / "check"
    subprocess.run(["rustc", "--edition=2024", str(check), "-o", str(executable)], check=True)
    subprocess.run([str(executable)], check=True)

print("PASS: invalid probes, exact zero residual, injected spectral tone, early nonfinite/peak")
