# Gaussian heat-kernel canonical oscillator experiment

Date: 2026-08-30

Status: rejected; production DSP unchanged

Baseline: `ab4d354` (`0.8.9`)

## Local audit

Repository history, VA source, and the local experiment/research archive contain
no Gaussian, heat-kernel, error-function, or normal-CDF oscillator. The closest
completed families are:

- analytic centered box/triangle ADAA in
  `canonical-analytic-adaa-round-10-2026-08-30.md`;
- centered phase-domain PTR3 in
  `centered-ptr3-phase-domain-2026-08-30.md`;
- fitted compact polynomial BLEPs in
  `local-blep-polynomial-round-11-2026-08-30.md`;
- an exact compact parabolic aperture in
  `va-event-local-parabolic-quadrature-2026-08-30.md`;
- stateless support-two equiripple BLEP/BLAMP tables in
  `support-two-equiripple-blep-round-23-2026-08-30.md`;
- sparse causal minimum-phase BLEP rings in
  `va-sparse-minblep-ring-2026-08-30.md`.

The Gaussian remains a local residual after negligible-tail truncation, but it
does not collapse to any of those formulas. Its transfer is the heat semigroup
`exp(-2 pi^2 sigma^2 k^2)`, its step is an error function, and its ramp is the
analytic integral of that step. It uses no fitted polynomial, residual lookup,
future ring, event schedule, or retained filter state.

## Candidate

The phase-domain standard deviation tracks pitch:

```text
sigma_phase = phase_step * sqrt(2 ln(2)) / pi
```

This places the Gaussian response at `0.5` at Nyquist, the midpoint value of an
ideal brickwall discontinuity. It is a fixed `0.37478125` standard deviation in
output-sample units, not a fixed phase blur.

For signed phase distance `x`, Gaussian standard deviation `sigma`, standard
normal CDF `Phi`, and density `phi`, the upward-step residual is:

```text
r0(x) = Phi(x / sigma) - H(x)
```

The slope-event residual is its analytic ramp integral:

```text
r1(x) = x r0(x) + sigma phi(x / sigma)
```

Saw uses one `-2 r0` wrap event. Square and pulse use `+2 r0` at wrap and
`-2 r0` at pulse width. Triangle uses `+8 r1` at its trough and `-8 r1` at its
peak. Phase remains the sole authoritative state, so pitch, reset, and PWM take
effect immediately.

The probe evaluates the complementary error function with the bounded
Abramowitz-Stegun 7.1.26 approximation. It returns before `exp` outside five
standard deviations. Five sigma is `1.87390625` output samples; the omitted
one-sided step tail is about `2.9e-7`. The maximum tested transition step is
`0.25`, so the truncated periodic images do not overlap. A safety cap keeps
support below half a cycle beyond that boundary.

Runtime shape work is bounded O(1), stateless, deterministic, allocation-free,
lock-free, and I/O-free. Scalar and x8 share the same scalar tail evaluator;
only lanes near a corner spend exponential work. There is no installed SIMD
exponential helper, so scalarized active lanes are the honest first x8 gate.

## Measurement contract

The single ignored probe covers:

- exact ideal-bandlimited whole-cycle RMS and peak against shipping 1x and 2x;
- known Gaussian wanted transfer, folded/numeric alias, off-grid energy, DC,
  gain, and output peak;
- sixteen fractional edge phases at period 7;
- a repeating 24/32-frame pitch, hard-reset, and PWM schedule;
- scalar/x8 sample and published-phase equality through pitch, PWM, and reset;
- paired real scalar and x8 stereo accumulation at 24 and 32 frames, 440 and
  7040 Hz.

The native release probe was compiled alone, pinned to logical CPU 6, and run
with no competing Cargo, rustc, or KURV test process. Bitwig and a Windows VM
remained available, so the CPU table is a coarse rejection gate. Every loss is
large enough that a pristine repeat is not needed.

## Static ideal-projection result

Ideal-relative whole-curve error is in dB; more negative is better. Frequencies
are coherent periods 436, 55, and 7: 110.09, 872.73, and 6857.14 Hz at 48 kHz.

| Wave | Shipping 1x | Gaussian 1x | Shipping 2x |
|---|---:|---:|---:|
| Saw | -28.330 / -19.270 / -9.614 | **-30.713 / -21.733 / -12.189** | **-35.563 / -27.214 / -35.919** |
| Square | -30.076 / -21.224 / -11.104 | **-32.444 / -26.276 / -16.840** | **-36.992 / -28.205 / -35.148** |
| Pulse 31% | -30.165 / -21.050 / -12.202 | **-33.045 / -23.563 / -15.497** | **-37.136 / -29.177 / -41.064** |
| Triangle | -65.232 / -42.715 / -15.670 | **-66.837 / -49.927 / -22.985** | -45.088 / -44.936 / **-39.508** |

Gaussian smoothing improves shipping 1x in all twelve cells by 1.60-7.31 dB.
Shipping 2x remains better for every discontinuous-wave cell, often by more
than 20 dB at the high note. Gaussian triangle beats shipping 2x at low and mid
pitch but not at the high note.

The candidate remains zero-phase after common output alignment and keeps peaks
at or below one. Its finite Gaussian transfer trades folded alias for wanted
droop: high-note wanted-transfer error is -16.43 dB saw, -17.29 dB square,
-18.86 dB pulse, and -23.07 dB triangle. Scalar and x8 samples and published
phase matched exactly through pitch, PWM, and reset (`max_abs_error=0`,
`phase_error=0`).

## Artifact result

The static maximum adjacent step regressed in every saw, square, and pulse cell.
Examples at 6857 Hz are `0.6092 -> 0.7067` saw, `1.1400 -> 1.6357` square, and
`1.0275 -> 1.3231` pulse. High-note triangle also rises slightly from `0.5571`
to `0.5709`.

Sixteen fractional edge phases at period 7 expose folded DC error: worst
absolute error is about `0.00568` saw, `0.01137` square, `0.00573` pulse relative
to its `-0.38` analytic mean, and `0.00103` triangle. The candidate is therefore
not phase-universal even where its aligned coherent RMS improves.

The rapid schedule also rejects it:

| Wave / metric | Shipping 1x | Gaussian 1x |
|---|---:|---:|
| Saw pitch step | 1.4448 | 1.6465 |
| Saw reset step | 1.2081 | 1.3374 |
| Square pitch step | 1.7210 | 1.9397 |
| Square reset step | 1.8648 | 1.9940 |
| Pulse pitch step | 1.7210 | 1.9397 |
| Pulse reset step | 1.7719 | 1.9883 |
| Triangle global step | 0.7020 | 0.9266 |

Square PWM improves from `0.01937` to `0.00022`, triangle pitch improves from
`0.3151` to `0.2451`, and pulse PWM remains `2.0`. Those isolated wins do not
offset the cross-wave transition regressions.

## Real 24/32-frame CPU

Each cell is a five-repeat alternating-order median over real scalar or x8
stereo accumulation before the common oversampler. Candidate/current ratios:

| Wave | Scalar ratio range | x8 ratio range |
|---|---:|---:|
| Saw | 1.172-1.766x | 4.540-9.905x |
| Square | 1.265-1.994x | 4.996-8.757x |
| Pulse 31% | 1.248-1.974x | 4.938-8.791x |
| Triangle | 1.248-2.894x | 3.896-6.766x |

All sixteen scalar and all sixteen x8 cells lose. Scalarized active-lane
exponentials are especially expensive as edge density rises; even low-note
scalar saw is 17.2% slower. There is no near-win requiring a quieter rerun.

## Verdict

Reject. The Gaussian is a legitimate stateless quality candidate and advances
the shipping-1x coherent curve frontier, but it is not Pareto-safe: every CPU
cell loses, discontinuous-wave transition and adjacent-step artifacts regress,
off-grid phase creates DC error, and shipping 2x remains the stronger physical
reference for saw, square, and pulse.

Approximating the exponential with another compact polynomial would collapse
the candidate back into the already-rejected local polynomial-kernel family.
SIMD assembly would only optimize a formula whose artifact gates already fail.
The 721-line ignored probe was therefore removed. No production code, state,
dependency, version, preset, latency, or runtime behavior changed.

## Reproduction

```bash
env CARGO_TARGET_DIR=/tmp/kurv-va-gaussian-target \
  CARGO_BUILD_JOBS=1 RUSTFLAGS='-C target-cpu=native' \
  cargo test --release --no-default-features --lib --locked \
  oscillators::va::gaussian_experiment::gaussian_heat_kernel_canonical_report \
  --no-run

taskset -c 6 \
  /tmp/kurv-va-gaussian-target/release/deps/pure_va_dispersion_core-4541c32e2a044c33 \
  oscillators::va::gaussian_experiment::gaussian_heat_kernel_canonical_report \
  --exact --ignored --nocapture --test-threads=1
```

The focused report passed `1/1` with 400 tests filtered out in 2.89 seconds.
The cold native release build took 2m53s and emitted only the checkout's 26
existing test warnings after one local array-length inference correction.
