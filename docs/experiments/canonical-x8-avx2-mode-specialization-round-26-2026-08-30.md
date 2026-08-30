# Canonical x8 AVX2 mode specialization (round 26, 2026-08-30)

## Question

Can KURV remove the event-time `Spline` versus `SplineOptimized` branch from
the selected AVX2/FMA constant-saw x8 kernel by dispatching to const-specialized
Rust monomorphs once per block, without changing output or regressing any
production oversampling factor?

The portable fallback already routes to
`accumulate_saw8_block_constant_spline_impl<const OPTIMIZED: bool>`. The
selected AVX2/FMA kernel is separate: it derives a runtime `optimized` boolean
and retains both residual polynomials. Existing x86-64-v3 disassembly confirmed
packed AVX2 comparisons, blends, and FMAs, plus the mode branches inside the
sparse event path. The stable backend source blob was
`9548f603f6ffd912070e994a6cf4b3ca743f5af8` before this disposable probe.

## Probe and contract

The `cfg(test)` probe preserved the production structural outer router. Its
current arm called `VaVoice::accumulate_structural_saw8_block` unchanged; its
candidate arm repeated the same fixed step/gain/jitter setup and called AVX2
monomorphs with const `OPTIMIZED=true` or `false`. No handwritten assembly,
allocation, locking, I/O, logging, or new realtime state entered the measured
path.

The workload was one exact saw at 7,040 Hz, eight slightly detuned lanes, 48 kHz
host rate, and production antialiasing selection:

- factors 1 and 2: `SplineOptimized`, 32 internal samples;
- factor 3: `Spline`, 24 internal samples;
- factor 4: `Spline`, 32 internal samples.

The correctness gate compared every left/right lane, every oscillator phase,
the settled-jitter clock, and its interval with `to_bits()` after each of 256
consecutive blocks. All four factors were bit-identical.

## Measurement

Host: AMD Ryzen 7 7800X3D, rustc 1.98.0 / LLVM 22.1.8, x86-64-v3 release,
pinned logical CPU 8. Each cell used 1,000,000 blocks per variant, 11 paired
repeats, and alternating current/candidate order. The predeclared universal
gate required every paired-median candidate/current ratio to be at most 0.995.

| Factor | Samples | Current ns/block | Candidate ns/block | Paired median | Pair range | Gate |
|---:|---:|---:|---:|---:|---:|---|
| 1 | 32 | 152.047 | 150.707 | 0.9925 | 0.9431-1.0391 | pass |
| 2 | 32 | 154.060 | 152.522 | 0.9795 | 0.8710-1.0253 | pass |
| 3 | 24 | 114.471 | 115.941 | 1.0112 | 0.9396-1.0467 | fail |
| 4 | 32 | 162.587 | 170.478 | 1.0287 | 0.9729-1.0690 | fail |

Commands used before removing the disposable probe:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-simd-mode-target \
CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo test --lib --release --no-default-features --locked --no-run

taskset -c 8 /tmp/kurv-va-simd-mode-target/release/deps/pure_va_dispersion_core-fa92bc6eae4d1d88 \
  voices::voice::tests::canonical_saw8_specialization_bit_identity \
  --ignored --exact --nocapture --test-threads=1

taskset -c 8 /tmp/kurv-va-simd-mode-target/release/deps/pure_va_dispersion_core-fa92bc6eae4d1d88 \
  voices::voice::tests::canonical_saw8_specialization_cpu_report \
  --ignored --exact --nocapture --test-threads=1
```

## Verdict

Reject. Const dispatch modestly helps the optimized factor-1/2 cells, but it
regresses both exact-spline cells and therefore is not Pareto-safe. The pair
ranges also show appreciable host noise, but the longer paired medians fail the
unchanged threshold in the same exact-mode direction; no threshold was tuned.

All executable probe code was removed. Production, version, and runtime state
remain unchanged. The selected AVX2 kernel is already lane-vectorized and the
remaining time dependency is the oscillator phase recurrence. Since the only
identified instruction-level gap was expressible in ordinary Rust and failed
the real-router gate, handwritten intrinsics beyond the existing kernel or
assembly are not justified.
