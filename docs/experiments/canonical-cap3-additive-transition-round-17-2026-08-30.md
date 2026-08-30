# Canonical cap-3 additive transition, round 17 (rejected)

Date: 2026-08-30

Verdict: reject the common production region. The steady cap-3 recurrence remains a strong quality/CPU frontier, but dual-kernel entry/exit loses short voice lifetimes for saw, square, and pulse, and the harmonic amplitude ramp is not uniformly peak-safe. Production code and object sizes remain unchanged.

## Prototype

The temporary test-only renderer used exactly three x8 complex oscillators. Fundamental phase and rotation came from KURV's existing vector sine/cosine evaluator; harmonics two and three were complex products. Each 64-frame block reinitialized from authoritative oscillator phase and advanced only the legal harmonics below Nyquist. It retained no recurrence state between blocks.

Analytic coefficients covered ascending saw, 50% square, 31% pulse, and triangle. Scalar/x4, warp, morph, audio-rate width/pitch, and non-constant steps stayed on current code. The transition prototype rendered both the actual current constant-step kernel and additive kernel for one block, then linearly crossfaded their outputs. Its timing includes both kernels, temporary fixed stack buffers, and blending.

The complete runtime prototype was reverted after the gate failed. Only this record remains.

## Commands and workload

```text
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native' \
  cargo test additive_cap3_transition_report --lib --release --locked --no-run

taskset -c 8 \
  target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce \
  oscillators::va::experiment::additive_cap3_transition_report \
  --ignored --exact --nocapture --test-threads=1
```

Ryzen 7 7800X3D, rustc 1.98.0, detected AVX2/FMA current backend. Medians of five runs, 20,000 blocks per cell, 64 stereo frames and eight lanes per block. Voice-lifetime ratios model one entry and one exit fade: `(2 * fade + (blocks - 2) * additive) / (blocks * current)`.

## Actual block and duty-cycle CPU

Nanoseconds per block; ratios below 1 beat an all-current voice.

| shape | Hz | current | additive | dual fade | 4-block ratio | 16-block ratio | 64-block ratio |
|---|---:|---:|---:|---:|---:|---:|---:|
| saw | 7k | 198.0 | 145.9 | 449.2 | 1.503 | 0.928 | 0.785 |
| saw | 8k | 202.4 | 160.7 | 505.0 | 1.645 | 1.007 | 0.847 |
| saw | 9k | 200.5 | 147.2 | 465.6 | 1.528 | 0.933 | 0.784 |
| saw | 10k | 231.3 | 177.7 | 482.9 | 1.428 | 0.933 | 0.810 |
| saw | 12k | 333.4 | 179.3 | 556.9 | 1.104 | 0.679 | 0.573 |
| square | 7k | 426.6 | 175.6 | 774.3 | 1.113 | 0.587 | 0.455 |
| square | 8k | 327.5 | 158.5 | 670.5 | 1.266 | 0.680 | 0.533 |
| square | 9k | 364.9 | 168.1 | 637.1 | 1.103 | 0.621 | 0.501 |
| square | 10k | 405.0 | 166.6 | 689.2 | 1.057 | 0.573 | 0.452 |
| square | 12k | 434.5 | 157.3 | 745.8 | 1.039 | 0.531 | 0.404 |
| pulse31 | 7k | 331.7 | 165.6 | 624.1 | 1.190 | 0.672 | 0.543 |
| pulse31 | 8k | 352.9 | 168.7 | 647.9 | 1.157 | 0.648 | 0.520 |
| pulse31 | 9k | 378.2 | 171.4 | 827.9 | 1.321 | 0.670 | 0.508 |
| pulse31 | 10k | 417.2 | 162.0 | 707.0 | 1.041 | 0.552 | 0.429 |
| pulse31 | 12k | 555.2 | 164.0 | 794.4 | 0.863 | 0.437 | 0.331 |
| triangle | 7k | 461.1 | 127.5 | 670.1 | 0.865 | 0.424 | 0.313 |
| triangle | 8k | 471.3 | 127.1 | 713.6 | 0.892 | 0.425 | 0.309 |
| triangle | 9k | 496.7 | 127.1 | 728.5 | 0.861 | 0.407 | 0.294 |
| triangle | 10k | 522.1 | 127.5 | 742.0 | 0.833 | 0.391 | 0.281 |
| triangle | 12k | 509.8 | 126.8 | 728.5 | 0.839 | 0.396 | 0.286 |

The steady kernel wins every cell, but the requested real duty-cycle gate fails. Four-block saw, square, and pulse voices lose everywhere except pulse at 12 kHz; even a 16-block saw at 8 kHz is parity/slower. Triangle alone survives, which is not the requested common canonical region.

## Accuracy and artifacts

This prototype uses the same analytic cap-limited Fourier coefficients and per-block recurrence as round 4. Against the exact ideal projection at cap 3, round 4 measured additive RMS of 0.00123 saw, 0.00234 square, 0.00181 pulse37, and 0.00119 triangle, versus current RMS of 0.1744, 0.2722, 0.2166, and 0.0991. Complex coefficient RMS was 0.8e-9 to 18e-9. There is no theoretical out-of-cap alias term; remaining error is bounded f32 recurrence/evaluator drift. The pulse duty changed to 31% here, but uses the same exact analytic coefficient construction.

Transition analysis swept 2,048 starting phases over one 64-frame cap-3 to cap-2 pitch block. Peak adjacent-sample magnitude includes the naturally large high-note waveform slope:

| shape | hard cap change | linear harmonic-3 ramp |
|---|---:|---:|
| saw | 1.505 | 1.709 |
| square | 1.837 | 2.241 |
| pulse31 | 2.037 | 2.065 |
| triangle | 0.901 | 0.899 |

The requested amplitude ramp is therefore not uniformly safer: it worsens saw and square and is neutral/slightly worse for pulse. A 31%-duty pulse width change was also swept. Linear coefficient/width interpolation improved the peak for 31% to 20% (2.599 to 2.246), but slightly worsened 31% to 40% (1.852 to 1.907) and was neutral at 50% (2.122 to 2.123). Width changes must remain on current rendering; entering/exiting additive around them still incurs both fade blocks.

## State and RT accounting

The recurrence itself needs only fixed local vectors and zero retained bytes. A safe selector cannot be stateless: because structural x8 packs can change membership, voice-lifetime eligibility must live with each oscillator, minimally backend mode, previous cap, and fade cursor (at least three logical bytes per oscillator before layout padding). Per-pack state would attach transitions to the wrong voice after repacking. No allocation, lock, I/O, logging, or unbounded work is needed, but the state and dual-render complexity earn no common Pareto-safe region.

## Decision

Reject and revert. The attractive steady numbers do not survive short saw/square/pulse voices, and neither the cap ramp nor width ramp satisfies the adjacent-peak non-regression requirement. Restricting to long triangle notes would be a product-specific special case rather than the requested shared canonical backend. No production selector, fade state, recurrence, version bump, or publication change is retained.

Limitations: the detailed ideal metrics are inherited from the identical round-4 coefficient/recurrence construction rather than recomputed in the final reverted source. Transition sweeps use analytic f64 sums to isolate cap/width behavior; release CPU uses the temporary f32 SIMD implementation.
