# Triangle-only additive ownership, round 18 (rejected)

Date: 2026-08-30

Verdict: triangle is a real steady-state Pareto exception, but not a uniformly Pareto-safe stateful renderer. Production remains unchanged and no version bump is warranted.

## Minimum ownership prototype

The smallest correct ownership is one packed byte per `VaOscillator`, not per transient x8 pack. It encodes fresh/current/additive mode, cap 1-3, and a bounded stability count. Repacking copies each oscillator and therefore preserves ownership. A layout probe mirroring the current field order measured 40 bytes both before and after inserting this byte: it consumes the existing byte of padding and adds no object/publication bytes on this target.

The recurrence remains round 17's fixed local x8 state, initialized from authoritative phase once per block. No sine/cosine state is retained. Fresh note-on can start directly in additive mode because there is no preceding current sample; reset returns the ownership byte to fresh. Note-off can remain additive until the envelope terminates and reset occurs. Mid-voice eligibility entry/exit still requires the dual-render fade, while cap 3-to-2 uses a harmonic amplitude ramp.

This selector was modeled and the runtime prototype was reverted: the transition and two-block interval gates below fail before the ownership byte earns a production field.

## CPU including state and fades

Round 17's actual detected-AVX2 renderer timings already include per-block recurrence initialization, two fixed temporary kernels, blending, and oscillator phase publication. One byte read/write per oscillator is below the timer resolution relative to those kernels; the conservative table does not subtract it.

`note` begins additive directly. `interval` is a mid-voice eligible interval with entry and exit fades; a one-block excursion remains current. Ratios are against all-current rendering.

| Hz | note 1/2/4/8/16 blocks | interval 1 | interval 2 | interval 4 | interval 8 | interval 16 |
|---:|---:|---:|---:|---:|---:|---:|
| 7k | 0.277 | 1.000 | 1.453 | 0.865 | 0.571 | 0.424 |
| 8k | 0.270 | 1.000 | 1.514 | 0.892 | 0.581 | 0.425 |
| 9k | 0.256 | 1.000 | 1.467 | 0.861 | 0.559 | 0.407 |
| 10k | 0.244 | 1.000 | 1.421 | 0.833 | 0.539 | 0.391 |
| 12k | 0.249 | 1.000 | 1.429 | 0.839 | 0.544 | 0.396 |

Fresh notes are excellent at every requested lifetime. A two-block pitch/eligibility excursion loses 42-51%. Hysteresis cannot make every duration Pareto-safe: delaying entry makes short excursions current/parity, but for any finite delay there is a later interval containing two expensive fade blocks and too little additive steady state. Repacking with mixed current/additive ownership creates exactly this bounded-excursion case. Persisting per-oscillator ownership fixes correctness, not the CPU loss.

## Ideal projection

An offline analytic evaluator used the current optimized BLAMP polynomial and exact odd-harmonic triangle projection at coherent periods. Additive is the reference itself, apart from the production prototype's previously measured f32 recurrence error.

| Hz | cap | current RMS | current wanted error dB | current unwanted | current DC | current gain | additive RMS / unwanted / gain |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 6,000 | 3 | 0.075852 | -17.62 | 0 | 0 | 0.885055 | 0 / 3.8e-34 / 1.0 |
| 6,857 | 3 | 0.094936 | -15.67 | 0 | -2.7e-6 | 0.853322 | 0 / 0 / 1.0 |
| 8,000 | 2 | 0.108570 | -14.50 | 1.30e-4 | 0 | 0.811866 | 0 / 1.3e-33 / 1.0 |
| 9,600 | 2 | 0.149392 | -11.68 | 0 | -5.3e-6 | 0.739410 | 0 / 0 / 1.0 |
| 12,000 | 1 | 0.216842 | -8.44 | 0 | 0 | 0.621672 | 0 / 6.2e-34 / 1.0 |

The f32 SIMD recurrence from round 4 measured triangle RMS around 0.0011-0.0031 and coefficient error around 1e-9, so the exact-zero rows are an oracle bound rather than a floating-point runtime claim. The direction is nevertheless decisive: wanted accuracy, alias, DC, and gain all improve.

## Transition and adjacent peaks

One-block current-to-additive and additive-to-current crossfades were swept over 4,096 starting phases. Values are maximum absolute adjacent-sample differences; they include the additive waveform's restoration of ideal gain.

| Hz | current | entry fade | exit fade |
|---:|---:|---:|---:|
| 6,000 | 0.498636 | 0.558548 | 0.556742 |
| 6,857 | 0.563651 | 0.626337 | 0.624077 |
| 8,000 | 0.635085 | 0.809201 | 0.806459 |
| 9,600 | 0.697469 | 0.950890 | 0.946899 |
| 12,000 | 0.711959 | 1.142930 | 1.136143 |

The cap-3 to cap-2 harmonic ramp from round 17 was essentially neutral/slightly better for triangle (0.900657 hard versus 0.899276 ramp). Eligibility fades are not neutral: adjacent peaks rise 11-61%. Much of that is physically correct gain restoration, but it still violates the requested non-regression gate and can present as a pitch-change transient.

## Repacking, reset, and complexity

- Note-on/reset: direct additive is phase-authoritative and needs no fade.
- Stable repacking of additive oscillators with the same cap: safe because recurrence state is reconstructed from each oscillator phase.
- Mixed-cap repacking: needs lane-specific old/new harmonic gains.
- Mixed current/additive repacking: needs both kernels, lane masks, and a fade for every affected oscillator.
- Eligibility exit: must pay a fade or hard-switch a waveform whose gain differs by as much as 38% at 12 kHz.

The byte fits existing padding, but the renderer does not stay small: it needs per-lane state decoding, mixed-pack masks, dual fixed buffers, cap interpolation, entry/exit policy, and special reset semantics. That complexity is disproportionate once two-block intervals and transition peaks already fail.

## Decision

Reject and revert. Triangle additive is worth remembering as a note-on-only steady kernel, but a note-on-only implementation would evade the explicitly required pitch, cap, and repacking behavior. There is no uniformly Pareto-safe transition policy for arbitrary eligibility durations, and the measured fades increase adjacent peaks. No DSP code, state byte, tests, version change, or publication change is retained.

Exact offline commands used the same inline analytic Python evaluator recorded in round 16, substituting KURV's optimized BLAMP coefficients from `antialias.rs` and triangle coefficients `-8/(pi^2 k^2)` for odd `k`. Release CPU inputs are the round-17 command and workload.
