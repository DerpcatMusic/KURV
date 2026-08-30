# Pruned real-output recurrence, round 22 (rejected)

Date: 2026-08-30
Baseline: `7850f6bfb4571cf7ba4a3d0102132d71ae05fb46`

## Verdict

Reject and revert. A block-reinitialized real second-order recurrence reproduced
the exact cap-limited canonical Fourier projection to roughly `1e-6` RMS and
made steady x8 rendering substantially cheaper. It did not survive the actual
production boundary: every measured four-block saw voice was 1.106-1.521x the
current cost after one entry and one exit block, and a one-block linear
crossfade made the 5.9-to-6.1 kHz entry artifact worse for every shape. No
runtime renderer, selector, transition state, test harness, or version bump is
retained.

## Distinct candidate

This was not another complex-phasor additive bank. For each legal harmonic, the
prototype formed only the first two real output values,

```text
r_k[n] = 2 Re(c_k exp(i 2 pi k phase[n]))
```

then advanced them for the remainder of an actual 24- or 32-frame KURV block:

```text
r_k[n + 2] = 2 cos(2 pi k step) r_k[n + 1] - r_k[n]
x[n] = dc + sum(r_k[n])
```

The block starts used the existing vector sine/cosine evaluator for phase and
step. There was no per-sample trigonometry. Square and triangle pruned their
zero even harmonics, and the cap-2 dispatch for those shapes collapsed to the
cap-1 kernel. Saw and pulse retained every legal harmonic. Pulse coefficients
used one scalar width sine/cosine setup per block.

The exact cap was `ceil(0.5 / step) - 1`. Eligibility required that all legal
harmonics fit in the fixed cap of three, so at 48 kHz the candidate covered
`step >= 0.125` and `step < 0.5` (6-24 kHz), with strict `< Nyquist` harmonic
selection in every scalar and x8 lane. Lower notes, custom curves, warp, morph,
phase modulation, and audio-rate pitch or pulse width remained on current
rendering.

This is materially different from round 4/17's complex oscillator recurrence:
the sample loop stores and advances `current`, `next`, and real `2 cos(theta)`
vectors, not real/imaginary phasors plus complex rotations. It is also distinct
from round 21's stateless per-sample sine/cosine polynomial, because it pays the
phase evaluation once per block.

## Exact-projection result

The oracle evaluated the analytic f64 coefficients at the same published f32
phase and exact legal cap. It checked saw, square, 31% pulse, and triangle over
four consecutive blocks, arbitrary scalar and eight-lane starting phases,
detuned mixed-cap x8 packs, exact cap boundaries and the immediately preceding
f32 values below one-third, one-half, and full Nyquist. Scalar and x8 phase bits
matched the oracle after every 24/32-frame run.

Representative x8 RMS error against the ideal cap-limited projection was:

| shape | cap / Hz | current x8 | real recurrence, 24 | real recurrence, 32 |
|---|---:|---:|---:|---:|
| saw | 3 / 6857 | 0.173627863 | 0.000000639 | 0.000000862 |
| saw | 2 / 9600 | 0.205042320 | 0.000000425 | 0.000000573 |
| saw | 1 / 12000 | 0.170902244 | 0.000000018 | 0.000000018 |
| square | 3 / 6857 | 0.264298928 | 0.000001061 | 0.000001434 |
| square | 2 / 9600 | 0.235016020 | 0.000000838 | 0.000001134 |
| square | 1 / 12000 | 0.341804487 | 0.000000036 | 0.000000036 |
| pulse31 | 3 / 6857 | 0.230006097 | 0.000000977 | 0.000001315 |
| pulse31 | 2 / 9600 | 0.357856457 | 0.000000689 | 0.000000934 |
| pulse31 | 1 / 12000 | 0.283428088 | 0.000000026 | 0.000000026 |
| triangle | 3 / 6857 | 0.094935542 | 0.000000523 | 0.000000712 |
| triangle | 2 / 9600 | 0.149392284 | 0.000000544 | 0.000000739 |
| triangle | 1 / 12000 | 0.216842209 | below 0.0000016 | below 0.0000016 |

Across the complete stationary report, recurrence RMS was approximately
`1.8e-8` to `1.6e-6`; peak error stayed approximately `5.3e-6` or lower in the
ordinary quality periods. Wanted-bin gain stayed approximately one, pulse DC
was `-0.37999998`, and unwanted energy was generally -119 to -150 dB where the
FFT cell was non-degenerate. No block seam exceeded the recurrence's ordinary
f32 error. The boundary/continuity assertion used a deliberately looser
`5e-4` peak ceiling and passed.

The focused diagnostic initially exposed a pulse-only peak error of 1.6196.
Expanding
`(1 - exp(-i 2 pi k width)) / (i pi k)` shows that the real coefficient is
positive `sin(2 pi k width) / (pi k)`; the prototype had its sign reversed.
Correcting only that coefficient sign made the full correctness check pass.

Pitch changes with an unchanged cap, cap 3-to-2, pulse width 31%-to-20%, and
triangle phase reset were also reinitialized at the block boundary. Candidate
RMS remained about `0.3e-6` to `4.3e-6`; its excess boundary delta was about
`0.05e-6` to `20e-6`. This verifies that the recurrence itself has no retained
drift or stale phase across ordinary parameter/reset boundaries.

## The production crossover still fails

The required selector boundary is 5.9-to-6.1 kHz: 5.9 kHz still has a legal
fourth harmonic and is ineligible, while 6.1 kHz has cap three. A 256-phase
sweep compared a hard backend switch with one 24/32-frame dual-render linear
crossfade. The metric below is the maximum excess adjacent-sample delta over the
transition block; ranges span the two real block sizes.

| shape | entry hard | entry fade | exit hard | exit fade |
|---|---:|---:|---:|---:|
| saw | 0.39251-0.39254 | 0.58355-0.58577 | 0.73660 | 0.72756-0.72967 |
| square | 0.34669 | 0.60021-0.61009 | 0.59672 | 0.58402-0.58725 |
| pulse31 | 0.40655 | 0.48404-0.49056 | 0.65236 | 0.64984-0.65034 |
| triangle | 0.14454 | 0.14498-0.14663 | 0.14453 | 0.14157-0.14167 |

The fade slightly helps exit, but materially worsens entry for saw, square, and
pulse and is neutral/slightly worse for triangle. Entry saw RMS likewise rose
from about 0.11903 hard to 0.12241-0.12247 faded. A fade therefore cannot be
claimed as an artifact-safe repair for switching waveform families.

## Actual 24/32-frame CPU and short-note gate

The release probe used Rust 1.98.0, `-C target-cpu=native`, the detected AVX2/FMA
backend, CPU 4 affinity, five medians, and 30,000 blocks per cell. Eight lanes
were slightly detuned and began at different phases. The timing included stereo
accumulation and phase publication. Entry rendered current plus eligible cap-3
recurrence and blended them; the measured 5.9 kHz exit rendered current plus a
forced cap-3 recurrence and blended them. Voice ratios were:

```text
(entry_dual + exit_dual + (blocks - 2) * recurrence) /
    (blocks * current)
```

The second coarse screen produced the decisive saw cells below. Lower is better.

| Hz | block | steady recurrence/current | four-block voice/current |
|---:|---:|---:|---:|
| 6100 | 24 | 0.688 | 1.345 |
| 6100 | 32 | 0.714 | 1.374 |
| 7000 | 24 | 0.777 | 1.454 |
| 7000 | 32 | 0.652 | 1.228 |
| 8000 | 24 | 0.784 | 1.521 |
| 8000 | 32 | 0.645 | 1.279 |
| 10000 | 24 | 0.640 | 1.270 |
| 10000 | 32 | 0.629 | 1.248 |
| 12000 | 24 | 0.516 | 1.153 |
| 12000 | 32 | 0.491 | 1.106 |

At 6100 Hz the 24-frame current, recurrence, entry, and exit costs were 103.231,
71.015, 210.083, and 203.090 ns/block; the corresponding 32-frame costs were
125.250, 89.481, 257.870, and 251.297 ns/block. Sixteen/64-block voice ratios
were 0.852/0.729 for 24 frames and 0.879/0.756 for 32 frames: the attractive
steady result only amortizes after the short-note region.

The 6100 Hz row uses the separately measured forced-cap-3 render at 5900 Hz for
the exit cost. The other rows use the same-frequency dual-render cost for both
voice entry and voice exit; either policy fails the four-block saw gate.

A separate coarse repeat found the same rejection: four-block saw ratios were
1.341/1.394 at 6100, 1.467/1.398 at 7000, 1.451/1.389 at 8000,
1.411/1.368 at 10000, and 1.119/1.130 at 12000 for 24/32 frames. Across all
shapes in the second screen, steady x8 ratios ranged from 0.216 to 0.784;
square, pulse, and triangle do not rescue the required common saw/square/pulse/
triangle backend.

These CPU measurements are intentionally labeled coarse, not pristine. No
compiler process overlapped the two accepted report executions, but Bitwig had
a low steady background load and aggregate idle varied roughly 76-93%. The
large, repeated four-block saw losses are sufficient to reject. No parity or win
from these runs is accepted as production evidence.

## State, latency, and commands

The recurrence had zero retained bytes and zero latency; every block was seeded
from authoritative oscillator phase. The existing oscillator remained 40 bytes.
Fixed local harmonic scratch was 36 bytes scalar or 288 bytes x8 at cap three.
It allocated, locked, blocked, logged, and performed I/O nowhere in rendering.
A production selector would nevertheless need per-oscillator backend/fade state
so x8 repacking cannot attach a transition to the wrong voice. That state and
dual work do not earn their place after the short-note and artifact failures.

```text
CARGO_BUILD_JOBS=1 \
CARGO_TARGET_DIR=/tmp/kurv-va-pruned-idft-target \
RUSTFLAGS='-C target-cpu=native' \
taskset -c 4 cargo test --release --no-default-features --lib \
  pruned_resonator_matches_exact_projection_for_actual_blocks \
  --locked --no-run

taskset -c 4 \
  /tmp/kurv-va-pruned-idft-target/release/deps/\
pure_va_dispersion_core-c78bc2803a9e2383 \
  oscillators::va::experiment::\
pruned_resonator_matches_exact_projection_for_actual_blocks \
  --exact --nocapture --test-threads=1

KURV_PRUNED_BLOCKS=30000 taskset -c 4 \
  /tmp/kurv-va-pruned-idft-target/release/deps/\
pure_va_dispersion_core-c78bc2803a9e2383 \
  oscillators::va::experiment::\
pruned_resonator_quality_transition_and_cpu_report \
  --exact --ignored --nocapture --test-threads=1
```

The correctness command passed 1/1 with 391 filtered tests. The first clean
release build took 7m55s while unrelated builds were active; those build timings
were discarded and do not contribute to any CPU claim. The report itself took
about 5.4 seconds.

## Decision

The real-output recurrence is an excellent stationary high-note reference and
confirms that exact cap-limited Fourier synthesis can beat the current 1x
renderer after setup amortizes. It is not a universal production win. The only
safe next attempt would have to remove most entry/exit dual work and improve the
backend-identity transition itself; assembly can reduce steady arithmetic but
cannot repair either failed gate.
