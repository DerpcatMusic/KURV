# Triangle correction optimization: measured, limited, bit-identical

Baseline: `d084681411a95803bb52206647c2bc881c4cbf8b` (0.8.145).

## Accepted change

The scalar dynamic triangle correction calls a BLAMP residual at both periodic
edges. Each residual has support `abs(position) < 2`. If `step < 0.25`, the two
supports are disjoint. Only the nearest edge can contribute; the other result is
exactly zero. Select the nearest edge and evaluate one residual. Keep the original
overlap handling, arithmetic precision, coefficients, sample alignment, and output
polarity. This changes work, not the filter design or antialiasing quality.

The specialization is enabled **only when both AVX2 and FMA are compile-time
features**. There is no new runtime dispatch or per-voice state. Portable and ARM
builds retain the previous implementation. This is deliberately limited: portable
measurements showed a small low-note regression, and ARM was not measured.

**The current baseline Linux release build does not benefit from this change.**
`scripts/build-linux-bundles.sh` requests `--target-cpu x86-64`. Native development
builds and release tiers compiled with AVX2/FMA can benefit. This PR neither changes
CPU compatibility nor introduces runtime CPU dispatch.

## Proof and measurements

An isolated Rust harness imports the actual before/after `antialias.rs` sources.
Its only source adaptation is replacing `truce_simd::simd::{f32x4,f32x8}` with the
same wide types. This was verified against the published truce-simd 6.3.0 source,
which directly reexports wide's types. No oscillator equations are reimplemented
in the measured kernels.

- 15,728,640 bit-identical output comparisons per executable: scalar f64 and SIMD
  f32 triangles, both spline kernels, 12 steps, uniform phase coverage, mixed-lane
  steps, inactive steps, and the overlapping-support region.
- A production-module unit test additionally compares the scalar correction with
  the prior periodic sum at support boundaries and adjacent representable phases.
- Unit test passed in baseline and x86-64-v3 release builds.
- CPU: AMD EPYC 9V74 under KVM, one benchmark process pinned to logical CPU 2.
  Rust 1.97.1, wide 0.7.33, opt-level 3, one codegen unit, no LTO.
- Final scalar measurement: 262,144 samples per trial, eight measured trials after
  a warm-up pair, alternating baseline/candidate order, median timing. Runtime
  phase/step slices pass through `black_box`, and output sums are consumed.

The recorded AVX2/FMA scalar results (nanoseconds per sample **including driver
work**) are:

| Kernel | Workload | Before | After | Time change |
|---|---|---:|---:|---:|
| Spline | low step 0.01 | 7.090 | 6.874 | -3.06% |
| Spline | high step 0.10 | 11.827 | 10.797 | -8.72% |
| Spline | nested varying step | 17.135 | 16.123 | -5.91% |
| Spline | overlapping support | 24.779 | 24.134 | -2.60% |
| Optimized spline | low step 0.01 | 7.095 | 6.875 | -3.09% |
| Optimized spline | high step 0.10 | 11.128 | 10.376 | -6.75% |
| Optimized spline | nested varying step | 16.987 | 15.292 | -9.98% |
| Optimized spline | overlapping support | 23.017 | 21.630 | -6.02% |

These are **microbenchmark observations on one virtualized machine**, not a
whole-synth speedup or a statistical guarantee. In particular, overlap arithmetic
is unchanged; apparent overlap improvements can reflect code generation and
measurement variation. The `nested_rate` driver varies the step with a nested
periodic expression every sample. It is **not** the production modulation graph,
not a phase-modulation quality test, and not proof of arbitrary nested audio-rate
routing performance.

The harness also runs 4/8/16/64 lanes. The accepted change leaves those SIMD kernels
unchanged: no multiple-unison improvement is claimed. The full plugin could not be
built because the sibling `derpcat-access` path dependency was unavailable, so
host rendering, scheduling, allocation, and UI integration remain untested here.

## Rejected experiments matter

A prototype applied the same nearest-edge choice to scalar and SIMD BLAMP helpers.
It passed the output checks and improved some 16/64-lane AVX2 workloads by roughly
20–44%, but worsened four-lane low-step work by roughly 6–9% and several overlapping
support cases by roughly 3–6%. Portable SIMD also had regressions. **Those SIMD
changes are not shipped.** Their results and a patch against the baseline are
retained for reproducibility, rather than advertising the best row as an overall
win. Initial SIMD measurements used 65,536 samples per trial; do not directly
compare their absolute timings with the longer final scalar run.

An ungated scalar-only experiment improved several portable workloads but slowed
low-step work by 1.4–2.6% in the longer run. That is why the accepted specialization
has a compile-time feature gate.

Next optimization work should use real process-lab routing presets and host
callback measurements: single voice, 4/8/16/64 lanes, modulation source fan-out,
and nested audio-rate routes. Frequency and phase modulation need separate cases.
Static-waveform wins are not evidence for those graphs. Runtime SIMD dispatch is
another project, requiring CPU compatibility and dispatch-overhead tests.

## Reproduce

From the repository root, with the baseline commit present locally:

```sh
cargo test --release --manifest-path tools/audits/triangle_kernel/Cargo.toml
cargo run --release --manifest-path tools/audits/triangle_kernel/Cargo.toml -- --verify-only
RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test --release --manifest-path tools/audits/triangle_kernel/Cargo.toml
RUSTFLAGS='-C target-cpu=x86-64-v3' cargo build --release --manifest-path tools/audits/triangle_kernel/Cargo.toml
# Choose an available isolated CPU rather than assuming CPU 2 on another machine.
taskset -c 2 tools/audits/triangle_kernel/target/release/kurv-triangle-kernel-audit --scalar-only
# Omit --scalar-only for the 1/4/8/16/64-lane matrix.
```

`KURV_AUDIT_BASE` overrides the baseline revision. The harness only depends on wide;
it does not require the unavailable licensing dependency or plugin GUI libraries.
No LTO is used because this environment's Rust 1.97.1 thin-LTO link failed with an
undefined `miniz_oxide::inflate::core::decompress` sysroot symbol. No-LTO builds
linked successfully; compiler settings must accompany any future comparison.

For the rejected SIMD prototype, create a disposable worktree at the baseline,
copy `tools/audits/triangle_kernel` into it, apply `rejected-simd.patch`, and run the
same harness. Do not apply that patch to a shipping branch.

Raw observations are in `docs/audits/triangle-kernel/*.csv`; the rejected prototype
is `tools/audits/triangle_kernel/rejected-simd.patch`.
