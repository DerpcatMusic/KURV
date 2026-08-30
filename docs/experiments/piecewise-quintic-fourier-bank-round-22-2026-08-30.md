# Piecewise-quintic Fourier bank, round 22 (rejected)

Date: 2026-08-30

Baseline: `7850f6bfb4571cf7ba4a3d0102132d71ae05fb46`

Machine: AMD Ryzen 7 7800X3D, native AVX2/FMA

Verdict: reject as a common production backend; retain only the experiment

## Question

Can the exact cap-at-most-six Fourier cycles be compressed into a tiny fixed
bank, then evaluated directly at 1x with enough SIMD throughput to beat KURV's
current constant-step x8 renderer without recurrence drift or table gathers?

The probe fits one degree-five polynomial to each of eight equal phase pieces.
Saw and triangle each store six cap banks. Square and arbitrary-duty pulse are
derived from two shifted saw evaluations plus DC:

```text
pulse(phase, width) = saw(phase + 1 - width) - saw(phase) + 2 * width - 1
```

The resulting contract is 12 banks at 192 bytes each, or 2,304 immutable bytes.
It adds no oscillator state. The least-squares fit is test-only and runs before
measurement; a production version would embed the resulting constants.

The AVX2/FMA evaluator performs one lane-wise piece permutation for each of six
coefficient planes and five Horner FMAs. Saw and triangle need one evaluation;
square and pulse need two. Cap selection is a fixed comparison chain. Packs
whose eight lanes share a cap take the SIMD path; a pack that straddles a cap
threshold takes the bounded per-lane scalar fallback. Phase gather/publication,
stereo accumulation, and 24/32-frame production-shaped blocks are inside the
timed function. There is no allocation, lock, I/O, logging, or unbounded work in
the candidate render loop.

## Ideal-projection quality

The quality run compared 65,536 phases against direct `f64` Fourier sums and
transformed the candidate cycle. `wanted dB` is complex error in DC and legal
harmonic bins relative to ideal cycle RMS. `alias dB` is energy above the cap
relative to ideal cycle RMS. More negative is better.

| shape | cap | curve RMS | curve peak | wanted dB | alias dB | worst seam |
|---|---:|---:|---:|---:|---:|---:|
| saw | 1 | 4.7e-8 | 2.39e-7 | -157.90 | -139.78 | 1.63e-7 |
| saw | 2 | 1.384e-6 | 5.462e-6 | -152.36 | -111.22 | 9.437e-6 |
| saw | 3 | 1.0348e-5 | 5.5000e-5 | -153.68 | -94.11 | 1.01285e-4 |
| saw | 4 | 5.9637e-5 | 2.64093e-4 | -140.01 | -79.09 | 5.18456e-4 |
| saw | 5 | 1.44657e-4 | 8.53898e-4 | -128.45 | -71.52 | 1.703336e-3 |
| saw | 6 | 3.23811e-4 | 2.034278e-3 | -113.52 | -64.60 | 4.057915e-3 |
| square | 1 | 1.03e-7 | 5.97e-7 | -157.92 | -138.87 | 2.98e-7 |
| square | 2 | 1.03e-7 | 5.68e-7 | -160.16 | -138.89 | 2.98e-7 |
| square | 3 | 2.0511e-5 | 9.9476e-5 | -156.24 | -93.31 | 1.83940e-4 |
| square | 4 | 2.0511e-5 | 9.9476e-5 | -151.35 | -93.31 | 1.83940e-4 |
| square | 5 | 2.64379e-4 | 1.280308e-3 | -127.69 | -71.26 | 2.553880e-3 |
| square | 6 | 2.64380e-4 | 1.280308e-3 | -127.46 | -71.26 | 2.553880e-3 |
| pulse31 | 1 | 1.65e-7 | 4.53e-7 | -135.77 | -138.81 | 1.50e-7 |
| pulse31 | 2 | 2.157e-6 | 6.293e-6 | -136.39 | -112.76 | 9.276e-6 |
| pulse31 | 3 | 1.4066e-5 | 6.8198e-5 | -134.79 | -96.47 | 1.01280e-4 |
| pulse31 | 4 | 7.6984e-5 | 3.31827e-4 | -133.35 | -81.82 | 5.18400e-4 |
| pulse31 | 5 | 2.10798e-4 | 9.32484e-4 | -128.98 | -73.23 | 1.702994e-3 |
| pulse31 | 6 | 4.76881e-4 | 2.376400e-3 | -119.16 | -66.15 | 4.057732e-3 |
| triangle | 1 | 6.2e-8 | 3.28e-7 | -149.92 | -139.70 | 2.21e-7 |
| triangle | 2 | 6.2e-8 | 3.28e-7 | -149.92 | -139.70 | 2.21e-7 |
| triangle | 3 | 4.353e-6 | 2.0924e-5 | -152.97 | -102.44 | 3.8720e-5 |
| triangle | 4 | 4.353e-6 | 2.0924e-5 | -152.97 | -102.44 | 3.8720e-5 |
| triangle | 5 | 2.6889e-5 | 1.31854e-4 | -143.29 | -86.63 | 2.63041e-4 |
| triangle | 6 | 2.6889e-5 | 1.31854e-4 | -143.29 | -86.63 | 2.63041e-4 |

The worst curve RMS is `4.76881e-4` for cap-6 pulse. Legal-bin accuracy is
excellent: even the worst wanted result is -113.52 dB. Nearly all remaining
error is unwanted energy caused by independently fitted piece seams. It reaches
-64.60 dB for cap-6 saw, so this representation is not the alias-free exact
Fourier projection even though its total curve error is far below the current
high-note renderer. At cap 3, for example, candidate RMS is `1.03e-5` saw,
`2.05e-5` square, `1.41e-5` pulse, and `4.35e-6` triangle. Round 17 measured
the corresponding current renderer near `0.1744`, `0.2722`, `0.2166`, and
`0.0991` RMS.

The native x8 evaluator agreed with the scalar Horner ground truth within
`1.19e-7` over all four shapes, six caps, and 8,192 phases.

## Transition gates

The candidate is stateless. Pitch, reset, and width changes therefore have no
retained recurrence to drift or become stale. The probe swept 16,384 starting
phases and compared each candidate output step to the same exact Fourier step.

| shape | largest intrinsic same-phase cap jump | largest fit excess at cap switch | largest fit excess on adjacent cap frame | same-cap pitch excess | reset excess |
|---|---:|---:|---:|---:|---:|
| saw | 0.318310 | 0.001180 | 0.002162 | 0.000055 | 0.000106 |
| square | 0.424413 | 0.001188 | 0.001528 | 0.000100 | 0.000192 |
| pulse31 | 0.591914 | 0.001515 | 0.002612 | 0.000064 | 0.000127 |
| triangle | 0.090063 | 0.000151 | 0.000157 | 0.000021 | 0.000031 |

The same-cap pitch gate used 7,000 to 7,500 Hz at cap 3. Reset returned to phase
zero at cap 3. Pulse-width changes from 31% to 20%, 40%, and 50% added at most
`0.000100` peak error to the exact transition. These are small approximation
errors. They do not remove the much larger, physically exact hard-cap jump:
changing the retained harmonic set instantaneously remains discontinuous.

## Actual x8 CPU

The release probe used the detected AVX2/FMA production backend, 20,000 blocks
per cell, seven alternating-order repetitions, eight lanes, both 24- and
32-frame blocks, and medians. Frequencies 3.5/4.2/5/7/9/12 kHz exercise caps
6/5/4/3/2/1. `ratio` is candidate/current; values below one win. The ranges
below include both block sizes and all six frequencies.

| shape | lane profile | ratio range | losses |
|---|---|---:|---:|
| saw | coherent | 0.596-1.047 | 2 / 12 |
| saw | decorrelated | 0.609-0.866 | 0 / 12 |
| saw | structural detuned | 0.769-2.671 | 2 / 12 |
| square | coherent | 0.633-1.319 | 8 / 12 |
| square | decorrelated | 0.544-0.758 | 0 / 12 |
| square | structural detuned | 0.567-3.129 | 2 / 12 |
| pulse31 | coherent | 0.708-1.308 | 8 / 12 |
| pulse31 | decorrelated | 0.530-0.793 | 0 / 12 |
| pulse31 | structural detuned | 0.571-3.123 | 2 / 12 |
| triangle | coherent | 0.355-0.581 | 0 / 12 |
| triangle | decorrelated | 0.266-0.500 | 0 / 12 |
| triangle | structural detuned | 0.264-1.164 | 2 / 12 |

The losses are structural, not benchmark noise:

| case | current ns/frame | candidate ns/frame | ratio |
|---|---:|---:|---:|
| coherent saw, 5 kHz, block 32 | 3.001 | 3.142 | 1.047 |
| coherent square, 3.5 kHz, block 32 | 4.029 | 5.315 | 1.319 |
| coherent pulse31, 3.5 kHz, block 24 | 4.065 | 5.317 | 1.308 |
| detuned saw, 12 kHz, caps 1-2, block 32 | 5.046 | 13.477 | 2.671 |
| detuned square, 12 kHz, caps 1-2, block 32 | 9.765 | 30.555 | 3.129 |
| detuned pulse31, 12 kHz, caps 1-2, block 32 | 9.821 | 30.671 | 3.123 |
| detuned triangle, 12 kHz, caps 1-2, block 24 | 11.685 | 13.607 | 1.164 |

Triangle is a compelling common-cap result, and decorrelated phases favor every
shape. However, phase coherence is musical state rather than a safe dispatch
condition. Square/pulse pay for two polynomial evaluations and lose most
coherent cap-3-through-cap-6 cells. More importantly, ordinary detuning can put
adjacent lanes on opposite sides of a Nyquist cap boundary; the scalar fallback
then loses 1.16-3.13x. Using the lower common cap would avoid that CPU cliff only
by deleting a legal harmonic from some lanes. Selecting the current backend for
mixed-cap packs would create another backend-switch and ownership problem and
would not repair coherent square/pulse losses.

## Reproduce

```text
CARGO_TARGET_DIR=/tmp/kurv-va-fourier3-target \
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native' \
  cargo test --release --no-default-features --lib piecewise_projection \
  --locked --no-run

taskset -c 5 \
  /tmp/kurv-va-fourier3-target/release/deps/pure_va_dispersion_core-623d25e54b34024e \
  oscillators::va::experiment::piecewise_projection_quality_transition_report \
  --ignored --exact --nocapture --test-threads=1

taskset -c 5 \
  /tmp/kurv-va-fourier3-target/release/deps/pure_va_dispersion_core-623d25e54b34024e \
  oscillators::va::experiment::piecewise_projection_cpu_report \
  --ignored --exact --nocapture --test-threads=1
```

The timing process was audited for competing Cargo, rustc, and KURV test
processes before the pinned run. The complete 144-cell report is reproducible
from the retained ignored probe.

## Decision and limits

Reject production integration. The 2,304-byte bank is genuinely tiny, the
wanted waveform is extremely accurate, and triangle/common-cap throughput is
excellent. It nevertheless fails the required universal CPU gate, introduces
measurable out-of-cap energy, and retains the exact hard-cap discontinuity. No
production renderer, selector, oscillator state, package version, or published
bank changed.

This is one `target-cpu=native` AVX2/FMA machine, not a portable or
cross-platform matrix; the compiler may also use other native ISA features
outside the explicit AVX2 evaluator. The portable scalar x8 fallback was not
separately compiled or timed, although the scalar per-lane mixed-cap path was.
The probe covers constant-step canonical 1x rendering, lane
coherence/decorrelation/detuning, phase reset, hard cap changes, and pulse width
changes. It does not claim custom curves, warp, sync, PM/FM, audio-rate morph, a
full voice/DAW workload, or host audition acceptance.
