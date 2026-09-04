# DSP quality measurements and oversampling contracts

Source audited: `d084681` (0.8.145). This change adds measurement diagnostics,
regression evidence, and documentation; it does not alter the audio renderer.

## Confirmed measurement problem

The `quality` and `warp_quality` reports historically print `alias_error_db` as
`10 log10(sum((output-reference)^2) / sum(reference^2))`, after phase alignment.
This measures total reconstruction error: aliases, wanted-harmonic attenuation,
phase differences, and DC can all contribute. A pure gain change produces a bad
score with no aliasing whatsoever. Calling the value an alias measurement can
misdirect optimization toward a colored waveform.

The reports now also emit:

| Field | Meaning |
| --- | --- |
| `reference_error_db` | Correct name for the legacy total error; smaller is better. |
| `wanted_amp_error_db` | Wanted harmonic magnitude error, ignoring phase. |
| `wanted_complex_error_db` | Harmonic-grid complex error, including phase. |
| `off_grid_energy_db` | Energy outside the harmonic grid, relative to wanted AC energy; excludes DC and Nyquist. |
| `alias_error_db` | Deprecated compatibility alias for `reference_error_db`; retained unchanged for old parsers. |

**None of these individually isolates all alias energy.** At integer periods,
aliased upper harmonics often land on the same harmonic grid as desired partials.
An off-grid result near the numerical floor therefore cannot certify a clean
oscillator. DC/Nyquist also need separate inspection. A periodic grid is suitable
for static waveform reconstruction tests; it cannot characterize arbitrary
nested audio-rate modulation.

The new test `spectral_metrics_distinguish_coloration_from_off_grid_energy`
proves three distinct cases with the production FFT and measurement function:

1. A 0.5-amplitude sine has -6.0206 dB wanted error and negligible off-grid energy.
2. A component at `Fs-f0` folds exactly onto the fundamental; its 0.25-amplitude
   alias gives -12.0412 dB wanted error while escaping off-grid detection.
3. A 0.125-amplitude off-grid spur is measured at -18.0618 dB without wanted-bin error.

For optimization decisions, compare against a converged, high-rate rendering
of the same intended signal, anti-alias-filtered before downsampling. Report
response/coloration separately from residual error; inspect coherent aliases,
DC, Nyquist, and modulation sidebands. Specify phase alignment and reference
bandwidth. Existing `aligned()` selects a fractional lag using linear
interpolation but applies it using a Fourier phase shift; that mismatch remains
an additional measurement limitation, not corrected by this PR.

## Oversampling: specify the complete path

`scripts/audit-dsp-metrics.py --check` reads the actual f32-rounded FIR and EQ
coefficients from `src/oversampling.rs`. It uses only Python's standard library.
It is an analytical coefficient response and f64 stream model, **not a plugin
render, SIMD validation, listening test, or CPU benchmark**.

At 20 kHz:

| Host rate | Factor | FIR alone | FIR + passband EQ | FIR + spline EQ |
| --- | --- | ---: | ---: | ---: |
| 44.1 kHz | 2x | -5.481 dB | -4.894 dB | -2.653 dB |
| 44.1 kHz | 3x | -5.469 dB | -4.881 dB | n/a |
| 44.1 kHz | 4x | -5.470 dB | -4.882 dB | n/a |
| 48 kHz | 2x | +0.031 dB | +0.592 dB | +2.668 dB |
| 48 kHz | 3x | +0.030 dB | +0.591 dB | n/a |
| 48 kHz | 4x | +0.031 dB | +0.592 dB | n/a |
| 96 kHz | 2x | +0.029 dB | +0.256 dB | +0.712 dB |

These are transfer functions from internal audio into the output filter. They
exclude the oscillator's own response. Spline EQ intentionally compensates the
oscillator, so its positive filter gain is **not proof that the complete synth
is too bright**. The 3x/4x paths do not apply the spline EQ.

The fixed normalized FIR coefficients specify 20.5 kHz passband/24 kHz stopband
at **48 kHz host rate**. At 44.1 kHz these become 18.835/22.05 kHz. The original
FIR-only regression tests cover the 48 kHz design, not a universal 20.5 kHz
passband. This is an explicit bandwidth/CPU design tradeoff, not a demonstrated
filter bug. Do not "fix" it by blindly increasing cutoff or flattening the EQ:
that changes stopband rejection or reverses intended oscillator compensation.

If a fixed-Hz bandwidth is a product requirement, design rate-specific
coefficients offline, keep callback cost fixed, and verify stopband plus complete
oscillator response at every supported rate. If it is not, document the current
normalized bandwidth and retain the cheaper fixed design.

## Streaming phase versus nominal latency

The FIR delay is 24 host samples, EQ adds two, and the output delay adds seven.
However, runtime pushes all `factor` internal samples before reading an output.
Relative to an input sampled at the **start** of each host frame, this advances
the decimation phase by `(factor-1)/factor` host samples:

| Factor | Nominal host latency | Measured/model start-of-frame lag |
| --- | ---: | ---: |
| 1x | 33 | 33 |
| 2x | 33 | 32.5 |
| 3x | 33 | 32.333333 |
| 4x | 33 | 32.25 |

The new `streaming_phase_includes_decimation_offset` test exercises the real
`StereoOversampler` push/output schedule with a coherent sine, both channels,
and both correction settings. The Python model independently reproduces the
oversampled cases. An existing `wave_curve.rs` Fourier experiment already uses
`LATENCY_SAMPLES - 0.5` for 2x, so this is not a newly discovered universal
latency failure. Host latency remains the same integer contract; assess musical
impact in actual parallel paths before changing timing or adding a fractional
filter with CPU and spectral costs.

## Reproduction

```sh
python3 scripts/audit-dsp-metrics.py --check
cargo test spectral_metrics_distinguish_coloration_from_off_grid_energy --lib
cargo test streaming_phase_includes_decimation_offset --lib
```

Full crate tests require KURV's private sibling dependencies. Run `python3 scripts/audit-dsp-metrics-rust.py --toolchain 1.97.1` for a
standalone probe that compiles `src/oversampling.rs` against `wide::f32x8` re-exported as
`truce_simd::simd::f32x8`, and compiles the exact metric helper/test plus
`src/dsp.rs` with rustfft. This validates source algorithms but does **not**
validate the production truce SIMD backend or complete plugin linkage.

Validation performed in the audit environment: Python `--check` passed;
standalone Rust source probe passed all three tests (metric fixture, production
streaming schedule, and existing FIR response contract). Rust 1.97.1,
`wide 0.7.33`, `rustfft 6.4.1`. Full plugin tests were not run because the private
sibling dependency was unavailable. This draft carries no release version bump.
