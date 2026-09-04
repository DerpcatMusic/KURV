# Guarded compact 1x BLEP experiment

Baseline: KURV `d084681`. Three static kernels: 6/8/12 samples (radius 3/4/6),
Kaiser beta 6, cutoff 0.300/0.325/0.375 cycles/sample respectively. Degree-five (6sample) and degree-seven (8/12sample)
polynomials cover each unit interval. Fit endpoints are constrained, including
exact half-step cancellation. The generator evaluates the continuous impulse
with SciPy quadrature and fits the integrated residual off the audio thread.

These are actual reusable Rust candidates, deliberately not selected by the
shipping renderer. They need a low-pitch/high-pitch crossover with the harmonic
renderer to avoid expensive overlapping edge sums. They are NOT a solution to
continuous PM/FM/warp sideband aliasing. Instantaneously substituting a new step
into the zero-state kernel is not a rigorous correction for changing phase speed.

## Measured quality

All runs use 48 kHz, 65,536 samples, initial phase 0.173, exact binary phase
increments, three shapes and five frequencies (27.10, 440.19, 2930.42, 7324.95,
18311.28 Hz). Pulse width is 0.31, clamped to the step at high pitch exactly as
production. Original `antialias.rs` is compiled directly by the standalone
harness; only the truce-simd wide-type reexport is replaced with wide.

`quality.json` contains all 60 cells. At 440.19 Hz:

| Shape | Current total error | 12sample total error | Current off-wanted energy | 12sample off-wanted energy |
|---|---:|---:|---:|---:|
| Saw | -22.58 dB | -25.70 dB | -48.41 dB | -72.21 dB |
| Square | -24.42 dB | -27.64 dB | -49.71 dB | -72.10 dB |
| Pulse | -23.63 dB | -26.68 dB | -49.92 dB | -76.57 dB |

The 12sample kernel improves total reconstruction error at all 15 shape/pitch
cells, and improves off-wanted-bin energy at all 15. Improvement at the 18.3 kHz
square cell is only 0.87 dB for off-wanted energy; the baseline already has very
little there. Near Nyquist this kernel still rolls off wanted harmonics and
cannot compete with an exact one-partial oscillator.

The cheaper 6sample kernel improves off-wanted energy throughout the matrix,
with almost unchanged total error at low notes; some high-note total errors
regress. This is an alternate quality/CPU tradeoff, not a universal replacement.

Off-wanted-bin energy is a LOWER BOUND on alias power: aliases that land on a
wanted bin are inseparable from response error. Total error is measured against
an exact harmonic projection without gain normalization, phase/time alignment,
or postfiltering. Wanted-bin complex error and fundamental magnitude are also
reported. DC and peak magnitude are retained so overshoot cannot be hidden.

An initial unguarded cutoff-0.5 design improved total error by 9–29 dB but WORSENED
low/mid-note alias energy by 10–14 dB. It was rejected. Those coefficients are not
included. This demonstrates why reconstruction error alone cannot justify an
antialiasing claim.

## CPU

`timing.csv`: 9 alternating-order repetitions, median nanoseconds/sample,
65,536 samples/repetition, Rust 1.97.1, release x86-64-v3. Shared container,
unpinned process, single-core scalar microbenchmark, includes phase stepping,
method/shape selection and output-vector allocation. These are provisional
kernel measurements, not DAW callback, unison, SIMD or full-plugin claims.

At 440 Hz the 12sample kernel is 1.076x saw, 1.112x square, 1.188x pulse relative
to production. At 27 Hz all are approximately equal. At 7.3–18.3 kHz the candidate
is 2.95–5.35x current: **do not use it unconditionally**. The 6sample option costs
1.021x/1.072x/1.088x at 440 Hz for roughly 10 dB less off-wanted energy.

The cost rise comes from overlapping periodic corrections: unlike a two-edge
implementation, this candidate correctly sums every event whose compact support
intersects the current sample. Omitting distant periodic edges gives incorrect
high-pitch results. Three contract tests check endpoints, continuity and periodic
edge sums against an independent fixed-range summation over 7 phase increments
and 1,024 phases, including Nyquist.

## Integration recommendation

Keep the 12sample guarded kernel as a low-pitch quality candidate. Benchmark it
against a harmonic renderer and use a measured crossover. Do not scalarize SIMD
lanes blindly: this report establishes no SIMD speedup. Before production enablement,
measure real scalar/x4/x8 render paths and pitch/reset/PM transitions. The useful
result is a substantial low-pitch alias reduction with a modest measured scalar
cost, not a claim that all sound modes are solved or optimized.

## Reproduce

```bash
python tools/experiments/one_x_correction/generate.py
RUSTUP_TOOLCHAIN=1.97.1 RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test --offline --release --manifest-path tools/experiments/one_x_correction/Cargo.toml
RUSTUP_TOOLCHAIN=1.97.1 RUSTFLAGS='-C target-cpu=x86-64-v3' cargo run --offline --release --manifest-path tools/experiments/one_x_correction/Cargo.toml -- dump /tmp/kurv-polynomial.bin
python tools/experiments/one_x_correction/analyze.py /tmp/kurv-polynomial.bin tools/experiments/one_x_correction/quality.json
RUSTUP_TOOLCHAIN=1.97.1 RUSTFLAGS='-C target-cpu=x86-64-v3' cargo run --offline --release --manifest-path tools/experiments/one_x_correction/Cargo.toml > tools/experiments/one_x_correction/timing.csv
```

## SIMD follow-up

`saw6_narrow4` and `saw6_narrow8` implement the nonoverlapping low-pitch case
(step < 1/6), with event masks and conditional evaluation of only the touched
polynomial intervals. No scalar table gathers. Degree five is sufficient for the
6sample kernel: measured alias values are unchanged to roughly 0.3 dB or better
versus the first degree-seven fit. `simd_prepared_timing.csv` compares the x8
prototype to the fully prepared current production BLEP (all invariant bounds
and reciprocals hoisted in both). Same release target, 9 alternating medians,
524,288 samples each, no concurrent benchmark reported during the timing window.

| Hz | Phase-aligned lanes ratio | Staggered lanes ratio |
|---|---:|---:|
| 27.1 | 0.878 | 0.922 |
| 440.2 | 0.919 | 1.119 |
| 1000 | 0.941 | 1.369 |
| 2930.4 | 1.113 | 1.306 |

These remain microbenchmarks. The very-low-note result merits integration work;
the 12–37% mid-note staggered-lane regressions prevent an unconditional default
replacement. Maximum scalar/SIMD sample difference in the 80,000-phase sweeps is
1.46e-7. Only x8 timing was measured; x4 implementation is supplied but has no
performance claim. `simd_timing.csv` retains the earlier degree-seven comparison
against the non-prepared narrow production helper, for an auditable record; use
`simd_prepared_timing.csv` for the current prototype.

```bash
RUSTUP_TOOLCHAIN=1.97.1 RUSTFLAGS='-C target-cpu=x86-64-v3' cargo run --offline --release --manifest-path tools/experiments/one_x_correction/Cargo.toml -- simd
```
