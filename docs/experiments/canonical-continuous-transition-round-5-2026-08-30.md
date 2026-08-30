# Canonical continuous-transition crossover, round 5 (rejected)

Date: 2026-08-30
Integrated main: `364ba31d` via merge `17419d9fc6d0c6c1c94aa4a7586727b2909c7d27`
Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, pinned CPU 8
Verdict: transition quality succeeded; CPU failed; no runtime or benchmark code retained

## Candidate

This round made transition safety primary and used main's current structural x8 constant-block seam. The test-only candidate had no persistent voice state and no curve/publication changes:

- analytic coefficients for saw, 50% square, 31% pulse, and triangle;
- at most seven canonical partials, derived from authoritative lane phases each sample;
- a smooth per-harmonic fade over the final 15% below Nyquist instead of hard integer-cap switching;
- a smooth current-to-additive backend blend from normalized step 0.075 to 0.140 (roughly MIDI 105 to 117 at 48 kHz);
- current structural rendering below the band, both kernels inside the band, and additive rendering above it;
- custom curves, morph, warp, PM, dynamic shape/width, scalar, and x4 remained on existing paths.

The branch first merged current local `main`. `git diff 427917d..main` confirms the canonical render, antialias, oversampler, and structural block files were unchanged; main's new architecture adds the shared reference harness and custom-curve experiments without changing this canonical baseline.

## Reproduction

```text
cargo clean --profile dev
cargo fmt
taskset -c 8 cargo test continuous_canonical_transition_report --lib --release --locked -- --ignored --nocapture --test-threads=1
```

The clean removed 1,023.9 MiB of generated debug artifacts after `/tmp` filled; no source or user data was removed. The release test passed. It used 65,536 samples per quality case and median-of-five timing over 8,000 real 64-frame x8 blocks with 0.1% lane detune.

## Ideal-reference quality and structural CPU

RMS compares against the exact hard ideal projection with every harmonic strictly below Nyquist. CPU is ns per host frame for the real structural factor-1 x8 block seam. The candidate timing includes both kernels throughout the transition band.

| Shape | MIDI | Backend mix | Current RMS | Candidate RMS | Current ns | Candidate ns |
|---|---:|---:|---:|---:|---:|---:|
| Saw | 105 | 0.000 | 0.11630995 | 0.11630995 | 20.463 | 21.554 |
| Saw | 111 | 0.413 | 0.13599241 | 0.07983950 | 21.016 | 82.123 |
| Saw | 117 | 1.000 | 0.17440243 | 0.01560526 | 21.007 | 58.418 |
| Saw | 123 | 1.000 | 0.20939987 | 0.00000000 | 21.780 | 60.081 |
| Square | 105 | 0.000 | 0.15394021 | 0.15394021 | 20.690 | 20.392 |
| Square | 111 | 0.413 | 0.17358002 | 0.10190673 | 19.288 | 206.491 |
| Square | 117 | 1.000 | 0.27224796 | 0.03121052 | 21.498 | 177.569 |
| Square | 123 | 1.000 | 0.25005212 | 0.00000000 | 20.521 | 188.689 |
| Pulse 31% | 105 | 0.000 | 0.16356055 | 0.16356055 | 29.767 | 30.843 |
| Pulse 31% | 111 | 0.413 | 0.17765835 | 0.10430107 | 28.530 | 199.852 |
| Pulse 31% | 117 | 1.000 | 0.23669354 | 0.00680858 | 25.085 | 156.955 |
| Pulse 31% | 123 | 1.000 | 0.37455538 | 0.00000000 | 20.057 | 168.295 |
| Triangle | 105 | 0.000 | 0.03337055 | 0.03337055 | 11.295 | 15.414 |
| Triangle | 111 | 0.413 | 0.05554810 | 0.03261162 | 15.925 | 59.593 |
| Triangle | 117 | 1.000 | 0.09906767 | 0.00662326 | 26.918 | 49.794 |
| Triangle | 123 | 1.000 | 0.15909263 | 0.00000000 | 36.836 | 61.766 |

The harmonic fade deliberately attenuates the highest legal partial near Nyquist, so MIDI 117 does not exactly match the hard projection. It still improves RMS substantially. At MIDI 123 the retained partials are outside the fade and the f64 quality probe matches the projection at reported precision.

## Transition artifacts

The maximum same-phase waveform difference immediately below versus above the cap boundaries was:

| Shape | Harmonic 2 | Harmonic 3 | Harmonic 4 | Harmonic 6 |
|---|---:|---:|---:|---:|
| Saw | 4e-9 | 3e-9 | 2e-9 | 1e-9 |
| Square | 0 | 6e-9 | 0 | 0 |
| Pulse 31% | 8e-9 | 1e-9 | 3e-9 | 1e-9 |
| Triangle | 0 | 1e-9 | 0 | 0 |

This is the successful part of the experiment. Round 4's hard-cap peaks were 0.318 saw, 0.424 square, 0.464 pulse, and 0.090 triangle. Analytic fades remove those discontinuities without cap state or hysteresis.

## Full-path factor-2 gate

Main does not change canonical rendering or `StereoOversampler`, so round 4's pinned full-path factor-2 x8 block measurements remain the exact applicable baseline. At MIDI 117/123 they were:

| Shape | Shipping 2x MIDI 117 / 123 ns | Candidate block MIDI 117 / 123 ns |
|---|---:|---:|
| Saw | 69.695 / 66.503 | 58.418 / 60.081 |
| Square | 88.343 / 69.085 | 177.569 / 188.689 |
| Pulse 37% baseline / 31% candidate | 72.176 / 64.305 | 156.955 / 168.295 |
| Triangle | 52.954 / 52.176 | 49.794 / 61.766 |

The pulse widths differ, so that row is directional rather than an exact waveform-identical timing. This does not affect the verdict: square and pulse lose factor 2 by large margins, triangle loses at MIDI 123, and every pure-additive case loses current factor 1. Candidate numbers exclude lane reduction/direct latency, so complete factor-1 output would be slower still.

## Verdict

Reject runtime integration. Continuous harmonic fades solve the primary cap-transition artifact cleanly and are worth preserving as a research direction, but the minimum dual-kernel transition costs 3.7-10.7 times current factor 1 at MIDI 111. The no-state pure backend also costs 1.9-8.4 times factor 1 at MIDI 117/123. It is not uniformly Pareto-safe against either required baseline.

No production path, transition state, object growth, or benchmark helper remains. A future continuation would need to apply the continuous harmonic weights to round 4's block-reinitialized recurrence, not recompute phase harmonics every sample, and must account for dual-kernel transition cost before integration.
