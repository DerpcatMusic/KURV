# Canonical exact-additive crossover, round 4 (rejected)

Date: 2026-08-30
Baseline: `febad0ad4e09d37e32ab1842a6d02bd09071e592`, production DSP unchanged from `427917d`
Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, pinned CPU 8
Verdict: rejected; no production or benchmark code retained

## Candidate

This round tested analytic canonical coefficients only: ascending saw, 50% square, 37% pulse, and triangle. It added no curve coefficients, no `WaveCurveRt` fields, and no publication cost. A stable-frequency complex recurrence updated only harmonics that exist: saw/pulse used the legal cap; square/triangle skipped every zero even harmonic. At the four notes, this meant 13/6/3/2 terms for saw and pulse, and 7/3/2/1 for square and triangle.

The temporary benchmark exercised real scalar, x4, x8, constant x8 block, shipping factor-2 block/decimator, and factor-1 direct-latency shapes. It also tested a phase-derived x8 evaluator with no recurrence state and a recurrence reinitialized once per 64-frame block.

```text
cargo check --tests --locked
taskset -c 8 cargo test benchmark_canonical_additive_real_render_shapes --lib --release --locked -- --ignored --nocapture --test-threads=1
```

Timings were medians of five runs: 750,000 scalar frames, 350,000 x4/x8 frames, 8,000 64-frame blocks, and 4,000 64-host-frame factor-2/direct blocks. SIMD lanes used a 0.1% detune spread. Factor 2 rendered 128 real x8 frames at half-step and passed them through `StereoOversampler`; factor 1 used its real direct latency path.

## Accuracy

The ideal is the analytic Fourier series with every harmonic strictly below Nyquist. RMS/peak cover 131,072 continuous recurrence samples. Complex coefficient RMS is the measured `f32` rounding error of the analytic coefficients; there is no sampled-table or curve-fit error.

| Shape | MIDI | Cap / active | Current 1x RMS | Additive RMS | Additive peak | Complex coefficient RMS |
|---|---:|---:|---:|---:|---:|---:|
| Saw | 93 | 13 / 13 | 0.08469977 | 0.00143457 | 0.00659185 | 4.16e-9 |
| Saw | 105 | 6 / 6 | 0.11631077 | 0.00151921 | 0.00620034 | 6.06e-9 |
| Saw | 117 | 3 / 3 | 0.17440378 | 0.00122629 | 0.00357530 | 8.35e-9 |
| Saw | 123 | 2 / 2 | 0.20939722 | 0.00401389 | 0.01366559 | 1.02e-8 |
| Square | 93 | 13 / 7 | 0.12316502 | 0.00243394 | 0.00700588 | 9.86e-9 |
| Square | 105 | 6 / 3 | 0.15393996 | 0.00277999 | 0.00883287 | 1.50e-8 |
| Square | 117 | 3 / 2 | 0.27224993 | 0.00234476 | 0.00613451 | 1.83e-8 |
| Square | 123 | 2 / 1 | 0.25005059 | 0.00493524 | 0.01207780 | 2.57e-8 |
| Pulse 37% | 93 | 13 / 13 | 0.11992921 | 0.00223791 | 0.00770700 | 7.26e-9 |
| Pulse 37% | 105 | 6 / 6 | 0.15796490 | 0.00196118 | 0.00770667 | 1.06e-8 |
| Pulse 37% | 117 | 3 / 3 | 0.21657959 | 0.00180691 | 0.00503969 | 1.49e-8 |
| Pulse 37% | 123 | 2 / 2 | 0.33552670 | 0.00646673 | 0.02100866 | 1.82e-8 |
| Triangle | 93 | 13 / 7 | 0.01211504 | 0.00110560 | 0.00324784 | 8.15e-10 |
| Triangle | 105 | 6 / 3 | 0.03337061 | 0.00102832 | 0.00309951 | 1.23e-9 |
| Triangle | 117 | 3 / 2 | 0.09906628 | 0.00118984 | 0.00358464 | 1.42e-9 |
| Triangle | 123 | 2 / 1 | 0.15909145 | 0.00314183 | 0.00769199 | 1.99e-9 |

The remaining additive error is long-run `f32` recurrence drift, not coefficient error. Reinitializing from authoritative phase once per block bounds that drift without retained voice state.

## Real x8 block CPU

Nanoseconds per host frame, final pinned run. `Recurrence/block` includes recurrence initialization once per 64-frame block. `Phase-derived` needs no recurrence state but recomputes the fundamental sine/cosine every sample. `Shipping 2x` includes both synthesis subframes, lane reduction, and decimation. `Recurrence + direct` uses a continuously retained recurrence and the factor-1 latency path, so it is a favorable lower bound that excludes transition handling.

| Shape | MIDI | Current 1x block | Recurrence/block | Phase-derived | Shipping 2x | Recurrence + direct |
|---|---:|---:|---:|---:|---:|---:|
| Saw | 93 | 20.846 | 48.201 | 41.300 | 54.286 | 35.997 |
| Saw | 105 | 20.540 | 19.991 | 27.413 | 62.204 | 24.217 |
| Saw | 117 | 21.749 | 11.594 | 20.307 | 69.695 | 13.446 |
| Saw | 123 | 21.475 | 8.363 | 17.154 | 66.503 | 5.458 |
| Square | 93 | 25.224 | 30.726 | 36.652 | 60.951 | 16.078 |
| Square | 105 | 18.910 | 11.159 | 25.484 | 67.749 | 7.913 |
| Square | 117 | 24.672 | 9.311 | 17.173 | 88.343 | 11.476 |
| Square | 123 | 27.988 | 8.829 | 18.587 | 69.085 | 4.085 |
| Pulse 37% | 93 | 21.939 | 52.496 | 42.450 | 51.401 | 29.705 |
| Pulse 37% | 105 | 19.344 | 22.129 | 32.048 | 61.353 | 13.787 |
| Pulse 37% | 117 | 22.291 | 15.867 | 25.967 | 72.176 | 16.481 |
| Pulse 37% | 123 | 22.722 | 9.358 | 16.962 | 64.305 | 5.911 |
| Triangle | 93 | 10.042 | 24.122 | 36.354 | 62.117 | 21.108 |
| Triangle | 105 | 12.501 | 11.614 | 25.006 | 72.239 | 13.020 |
| Triangle | 117 | 28.945 | 8.600 | 17.066 | 52.954 | 13.486 |
| Triangle | 123 | 28.397 | 6.535 | 14.580 | 52.176 | 4.079 |

The recurrence is a real high-note SIMD frontier, especially at cap 3 or 2, and it beats shipping factor 2 throughout. It is not a safe M105+ backend: saw had only a 2.7% factor-1 margin at MIDI 105, while pulse lost by 14.4%. The no-state phase-derived design lost factor 1 at MIDI 105 for every shape and still lost saw/pulse at MIDI 117.

Scalar was never viable for saw or pulse and usually cost several times current factor 1. x4 and x8 became faster at shape-specific caps, but no common M105+ threshold covered every real path. This is why a broad production branch was not retained.

## Pitch and cap transitions

Instantaneously changing the exact cap produces the following possible waveform jumps. Zero rows are square/triangle even harmonics that do not exist.

| Shape | Cap change | RMS jump | Peak jump |
|---|---:|---:|---:|
| Saw | 13 to 12 / 6 to 5 / 3 to 2 / 2 to 1 | 0.03463 / 0.07503 / 0.15005 / 0.22508 | 0.04897 / 0.10610 / 0.21221 / 0.31831 |
| Square | 13 to 12 / 6 to 5 / 3 to 2 / 2 to 1 | 0.06926 / 0 / 0.30011 / 0 | 0.09794 / 0 / 0.42441 / 0 |
| Pulse 37% | 13 to 12 / 6 to 5 / 3 to 2 / 2 to 1 | 0.03893 / 0.09565 / 0.10166 / 0.32815 | 0.05505 / 0.13527 / 0.14376 / 0.46408 |
| Triangle | 13 to 12 / 6 to 5 / 3 to 2 / 2 to 1 | 0.00339 / 0 / 0.06368 / 0 | 0.00480 / 0 / 0.09006 / 0 |

These hard changes do not meet click-safe shipping behavior. A safe implementation needs look-ahead bands or current-path crossfades around every nonzero cap boundary and around eligibility entry/exit. During those fades both kernels run, erasing the already marginal saw-M105 margin. Pitch/shape/width modulation, morph, custom curves, warp, and PM must remain on the current path.

The prototype recurrence state was 2,560 bytes per x8 pack because it stored vector gains as well as sine, cosine, and rotations. Removing redundant vector gains still leaves about 1,824 bytes per retained pack. KURV's fixed oscillator bank makes broad persistent storage unattractive. Per-block stack reinitialization removes persistent bytes, but its measured initialization cost is included above and it still needs transition state. No voice or oscillator object was enlarged.

## Verdict

Reject production integration in this round. The cap-3/cap-2 x8 recurrence is the first canonical candidate with a convincing stationary quality/CPU frontier, but the requested common M105+ region is not Pareto-safe across saw/pulse/block paths, and hard cap/eligibility transitions remain substantially discontinuous. No speculative transition framework was added after that gate failed.

The retained evidence supports only a future, narrower experiment: cap 3 or 2, block-only, shape-specific eligibility with transition bands measured inside a real voice lifetime. It does not support shipping the current prototype.
