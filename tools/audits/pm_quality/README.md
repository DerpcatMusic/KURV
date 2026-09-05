# Local phase travel and analytic saw integration

Experimental code for KURV baseline `d084681411a95803bb52206647c2bc881c4cbf8b`.
The module is deliberately not registered by `va.rs`: these results do not
justify changing the production PM renderer automatically.

## Implementations

- `saw_local_width`: use the magnitude of signed unwrapped phase travel as the
  existing spline correction width. No change to its residual polynomial.
- Centered-width experiment: same kernel, with `(phase[n+1]-phase[n-1])/2`.
  This requires future input; the quality comparison does not hide that cost.
- `saw_linear_average`: integrate the raw saw exactly along a linear phase
  segment. If `a` and `b` are endpoint phases, its value is
  `(A(b)-A(a))/(b-a)`, where `A(p)=fract(p)*(fract(p)-1)` is continuous and
  periodic. This handles reverse travel and multiple cycle crossings with
  bounded work. The cancellation-resistant factored implementation returns
  the midpoint exactly when no cycle boundary is crossed.
- `saw_linear_average_rebased`: cheaper scalar entry when the previous phase
  is already in `[0,1)` and the current endpoint is locally unwrapped.
- `saw_linear_average8`: eight-lane SIMD entry using `wide` (production aliases).

The integral averages the interval `[n-1,n]`, so its timestamp is **n-0.5**.
It has the passband attenuation of a box filter. It does not restore missing
intersample curvature, and it is not an ideal bandlimited PM oscillator.
Using it at multiple stages changes the modulation graph timing. There is no
safe claim that a half-sample delay can simply be ignored or globally removed.

## Reproduction

```sh
cargo +1.97.1 test --release --offline --manifest-path tools/audits/pm_quality/Cargo.toml
cargo +1.97.1 run --release --offline --manifest-path tools/audits/pm_quality/Cargo.toml > /tmp/pm-quality.csv
python3 tools/audits/pm_quality/analyze.py /tmp/pm-quality.csv > /tmp/pm-quality-results.csv
RUSTFLAGS='-C target-cpu=native' cargo +1.97.1 build --release --offline --manifest-path tools/audits/pm_quality/Cargo.toml
taskset -c 2 tools/audits/pm_quality/target/release/kurv-pm-quality-audit --bench > /tmp/pm-quality-benchmark.csv
```

The build script compiles the actual production `antialias.rs`, replacing only
its SIMD import with the identical `wide` types. No private licensing shim or
plugin dependency is substituted. These are kernel proofs, not a full plugin
build, live graph benchmark, or listening test.

## Quality evidence

`results.csv` contains 96 cases: 3 carrier frequencies, 4 modulator frequencies,
and 8 depth/parent configurations. Bins refer to a 4096-sample period; at 48 kHz
multiply by `48000/4096` for Hz. Parent frequency is bin 367. Positive `nested`
means sine phase modulation of the modulator; negative `nested` means parent
modulation of its amplitude, `depth*(1-nested*sin(parent))*sin(modulator)`.

The oracle evaluates this continuous analytic trajectory at 256x and 1024x,
forms the raw saw, then applies an ideal Fourier lowpass strictly below Nyquist.
This high-rate calculation exists only in the offline oracle. All candidate
renderers operate at 1x. The known ADAA half-sample timestamp is compensated by
multiplying the reference spectrum by `exp(-j*pi*k/N)`, never by fitting delay
or gain to make the answer look better.

For this fixed test set the integral improves total reference error in 87/96
cases, median improvement 4.06 dB, largest improvement 6.47 dB. Its worst
regression is 3.36 dB at carrier bin 37, modulator bin 1501, depth 0.02 cycles.
The ordinary travel-width substitution can regress by 4.67 dB; centered width
can regress by 4.99 dB. Neither is a defensible unconditional fix.

Total error includes aliasing, desired harmonic attenuation, and phase error.
`adaa_gain_db` measures projection on the unscaled delayed reference and is
reported separately, never corrected before scoring. Wanted complex error
uses reference bins above -80 dB relative to its strongest bin. The complement
is reported separately, but aliases overlapping intended components cannot be
identified by this support split. Dense nested PM occupies almost every bin.
Reference convergence is reported separately for every case; claims below that
resolution are not justified.

## Cost evidence and limits

`benchmark-native.csv` records seven alternating-order repetitions, with input
trajectories generated outside the timed region. Algorithm IDs: 0 scalar
production spline, 1 scalar rebased analytic integral, 2 production x8 spline,
3 analytic x8 integral. Timings include the harness loop, black-box barriers,
and accumulation; they do not include graph scheduling, input generation,
per-voice state management, or whole-plugin rendering. Eight lanes here pack
eight successive trajectory samples, rather than a full unison voice graph.
The machine is an AMD EPYC 9V74 VM, native target, one pinned logical CPU.
Portable architectures must be measured separately.

In this run the rebased scalar integral was approximately at parity for the
low/middle carrier cases (ratios 0.986–1.007), and 24–25% cheaper for the high
carrier. The x8 integral was 14–49% cheaper at the middle/high carriers, but
9–45% more expensive at the low carrier. These ratios are kernel measurements
at different sound quality and timing; they are not equivalent-quality speedups.

Tests verify the analytic integral against dense piecewise saw integration,
reversed endpoints, integer phase translations, and SIMD/scalar parity through
wraps and reversals, including tiny crossings that must not take a midpoint
shortcut. All three tests pass. No runtime allocation or unbounded crossing loop is used.

## Decision

Keep this as an executable candidate, not a silent replacement. It trades
waveform behavior and timing for lower total error in most tested cases. An
explicit quality mode would need production graph timing semantics, scalar
and SIMD routing, transition behavior, broader waveform support, and actual
full-plugin listening/performance gates. Simple derivative tracking does not
solve general nested PM, even though it is very cheap to calculate.
