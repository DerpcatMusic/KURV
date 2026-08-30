# Exact-Fourier production crossover, round 2 (rejected)

Date: 2026-08-30
Baseline: `db0fb778f911a15949b6263da58f6ab3efcfb3f3` (the round-1 oracle record, based on the `427917d` oscillator baseline)
Machine: AMD Ryzen 7 7800X3D, x86-64 host-default release build
Verdict: rejected; no production code retained

## Question

Could the round-1 exact polynomial Fourier evaluator become a conservative production backend for static high-note custom/VA curves while beating both the current 1x path and the shipping hidden-factor-2 path?

The prototype was deliberately guarded to the promising case: a prepared static curve, constant phase increment, supported harmonic count, and no warp or phase modulation. The existing renderer remained the intended fallback for dynamic morph, warp, PM, unsupported partial counts, and transitions. Before adding transition state or crossfades, the candidate had to pass the cheaper CPU gate in the actual scalar, x4, and x8 render shapes.

## Prototype

The temporary implementation compiled the exact Fourier coefficients of the piecewise polynomial curve into `WaveCurveRt`, published them through the existing atomic curve bank, and evaluated a bounded complex recurrence in scalar, x4, and x8 constant-frequency renderers.

This had two structural costs even before transition handling:

- `WaveCurveRt` grew from 256 to 368 bytes: 112 bytes for 13 complex partials plus metadata (28 additional `f32` words).
- Atomic publication grew from 64 to 92 words per curve. Across the 16-frame VA table bank this added 1,792 bytes and 448 atomic words per full publication.

The coefficient accessor was called inside the harmonic loop used to produce samples. Hoisting the lookup would remove indexing overhead, but not the dominant 13 complex recurrences, so it could not plausibly reverse the x8 result below.

The audio path remained bounded and allocation-, lock-, I/O-, and logging-free. That only establishes the prototype's real-time shape, not a reason to keep a slower backend.

## Command and workload

The pinned release run was:

```text
taskset -c 8 cargo test benchmark_production_fourier_scalar_x4_x8 --lib --release --locked -- --ignored --nocapture --test-threads=1
```

The discarded benchmark exercised one prepared static custom curve at MIDI notes 93, 105, 117, and 123, reporting nanoseconds per host output frame for the existing factor-1 render and the exact-Fourier candidate in scalar, x4, and x8 shapes. It also measured an x8 factor-2 synthesis-only lower bound. That lower bound excludes the shipping decimator, so it is favorable to factor 2 and is not a complete end-to-end shipping number.

| MIDI | scalar 1x | scalar Fourier | x4 1x | x4 Fourier | x8 1x | x8 Fourier | x8 factor-2 synthesis-only |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 93  | 12.492 | 33.018 | 30.326 | 24.089 | 22.654 | 38.686 | 39.354 |
| 105 | 9.383  | 24.170 | 29.071 | 22.071 | 20.988 | 39.782 | 39.344 |
| 117 | 9.391  | 27.119 | 29.299 | 21.970 | 19.074 | 36.807 | 37.904 |
| 123 | 10.363 | 21.891 | 30.593 | 23.175 | 20.275 | 40.222 | 40.621 |

Units are ns per host output frame; lower is better.

## Result

The scalar candidate was 2.1-2.9x slower than the current factor-1 path. The x8 candidate was 1.7-2.0x slower. Although the x4 micro-path was faster than factor 1, that isolated result did not survive the actual x8/unison-shaped gate. Fourier was only roughly tied with the deliberately incomplete factor-2 synthesis lower bound, before factor 2's decimator cost, and it still failed the required factor-1 comparison decisively.

Round 1 already established that this exact evaluator can improve stationary high-note ideal-reference accuracy relative to sampled spectral mips. Round 2 therefore tested whether that quality result could be integrated cheaply enough. It could not. This run did not repeat the ideal-reference sweep because the evaluator and coefficients were unchanged; the new question was production-path cost.

Pitch glides, harmonic-cap crossings, morph/static eligibility transitions, crossfades, and ineligible-path equivalence were intentionally not implemented or claimed. The candidate failed the CPU gate before transition machinery could earn its state, branch, and publication costs. Consequently there is no retained production backend, no new runtime state, and no shipped memory/publication increase.

## Limitations and next direction

- Results are host-default x86-64 release measurements on one 7800X3D core, not an x86-64-v3 or cross-platform matrix.
- The benchmark source was part of the rejected prototype and was removed with it; the exact command and emitted results are preserved above.
- The factor-2 column omits decimation and is not a full shipping-path timing. It is useful only as a favorable lower bound.
- Full-voice modulation, filter, envelope, and output costs were not included. Because the oscillator backend already loses in the x8 oscillator workload, adding shared downstream work cannot make it a backend CPU win.

The next materially different experiment should benchmark KURV's already-existing spectral-mip backend in real high-note x4/x8/unison paths. It should not add another Fourier recurrence framework unless new evidence changes the dominant-cost result.
