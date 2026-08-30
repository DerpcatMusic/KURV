# Spectral-mip real-render experiment, round 3 (rejected)

Date: 2026-08-30
Baseline: `c88c869e7be7da1ea6922cfa933ace4c3b89f883`, with production DSP unchanged from `427917d`
Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release build, pinned logical CPU 8
Verdict: rejected; no production or benchmark code retained

## Question

Can KURV's existing `BandlimitedWaveCurve` compiler and its 2,048-point, 20-mip table bank become a Pareto-safe 1x backend for high-note static custom/VA oscillators through the real scalar, x4, x8, constant-block, unison, and output-latency paths?

This round reused the representation and Catmull-Rom evaluators in `src/wave_curve/bandlimit.rs`. It added no alternate compiler, coefficients, runtime state, or publication mechanism.

## Reproduction and workload

The temporary ignored benchmark was built and run with:

```text
cargo check --tests --locked
taskset -c 8 cargo test benchmark_spectral_mip_real_render_shapes --lib --release --locked -- --ignored --nocapture --test-threads=1
```

It compiled one exact piecewise-linear saw `WaveCurveRt` with `BandlimitedWaveCurve::compile_boxed`, then tested equal-tempered MIDI notes 93, 105, 117, and 123 at 48 kHz. Timings are medians of five runs:

- scalar: 1,000,000 calls through the real current `VaOscillator::generate_custom_step` path or the selected spectral mip;
- x4 and x8: 500,000 calls through the real `generate_custom4`/`generate_custom8` path or the existing spectral SIMD API, with a 0.1% lane detune spread;
- constant x8 block: 10,000 64-frame calls through the real `accumulate_custom8_block_constant` path or the equivalent existing spectral evaluation/accumulation loop;
- shipping factor 2: 5,000 64-host-frame blocks, rendering a real 128-frame x8 custom block at half-step, reducing the lanes, pushing both subframes through `StereoOversampler`, and reading each host output;
- mip factor 1 direct: 5,000 64-frame spectral x8 blocks, lane reduction, and the real factor-1 direct latency path.

The temporary benchmark was removed after rejection. The command, workload, and complete emitted measurements are retained here.

## CPU results

Nanoseconds are per host frame; lower is better.

| MIDI | Scalar current | Scalar mip | x4 current | x4 mip | x8 current | x8 mip | Block x8 current | Block x8 mip | Shipping 2x block | Mip 1x + direct |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 93  | 7.621 | 10.446 | 27.846 | 35.620 | 17.580 | 71.510 | 18.078 | 71.909 | 62.133 | 72.898 |
| 105 | 5.829 | 8.941  | 27.201 | 35.962 | 17.497 | 70.886 | 18.418 | 78.773 | 66.611 | 81.259 |
| 117 | 5.819 | 8.961  | 27.290 | 35.385 | 17.335 | 72.348 | 18.085 | 72.374 | 62.142 | 75.818 |
| 123 | 5.821 | 8.946  | 27.285 | 35.660 | 17.267 | 71.732 | 17.965 | 74.035 | 62.175 | 75.554 |

The mip backend lost every real factor-1 render shape. It was 1.37-1.54x slower in scalar, 1.28-1.32x slower in x4, 4.07-4.42x slower in x8, and 3.98-4.28x slower in the real constant x8 block. Including lane reduction and KURV's latency path, mip 1x was also 1.17-1.22x slower than the complete shipping factor-2 block plus decimator.

The host-default build does not set AVX2/FMA target features, so `BandlimitedWaveCurve::eval4` and `eval8_mip` used their compiled scalar fallbacks. That is the actual behavior of this checkout's host-default release command, not an AVX2 projection. An architecture-specific build could change the ratios, but it would not address the quality, transition, publication, and cross-platform requirements below.

## Accuracy and interpolation

The ideal reference was the exact Fourier projection of the same compiled polynomial curve with every harmonic strictly below Nyquist. `complex wanted RMS` is the RMS complex-coefficient error across those legal harmonics. Interpolation error isolates Catmull-Rom lookup from the mip table's own sampled spectral coefficients.

| MIDI | Legal harmonics | Selected cap | Ideal RMS | Complex wanted RMS | Interpolation RMS | Interpolation peak |
|---:|---:|---:|---:|---:|---:|---:|
| 93  | 13 | 12 | 0.03472280 | 0.00680721 | 0.0000000503 | 0.0000003464 |
| 105 | 6  | 6  | 0.00201431 | 0.00048828 | 0.0000000329 | 0.0000002969 |
| 117 | 3  | 3  | 0.00191180 | 0.00048828 | 0.0000000301 | 0.0000002510 |
| 123 | 2  | 2  | 0.00190961 | 0.00048828 | 0.0000000294 | 0.0000002443 |

Catmull-Rom interpolation is not the problem: its isolated error stayed below `5.1e-8` RMS and `3.5e-7` peak. The material errors come from the sampled coefficient source and coarse harmonic caps. At MIDI 93 the cap-12 mip drops the legal 13th harmonic. Round 1 measured the same spectral evaluator against the same reference for both saw and square and found this omission produced 0.034723 saw RMS and 0.069350 square RMS.

For context, round 1's shipping factor-2 saw RMS was 0.064217, 0.078779, 0.111781, and 0.129416 at the four notes. The mip is more accurate for these stationary probes, but it does not beat shipping CPU and therefore cannot satisfy the joint gate.

## Mip-boundary artifacts

The benchmark compared the two adjacent tables at phase-step boundaries. These values are the possible instantaneous waveform change when pitch motion changes the selected mip without a crossfade.

| Entered cap below boundary | Removed cap above boundary | RMS jump | Peak jump |
|---:|---:|---:|---:|
| 2 | 1 | 0.22507943 | 0.31831041 |
| 3 | 2 | 0.15005325 | 0.21220738 |
| 4 | 3 | 0.11254025 | 0.15915596 |
| 6 | 4 | 0.11719628 | 0.23105401 |
| 8 | 6 | 0.08545270 | 0.16959274 |

These are not bounded click-safe transitions. A crossfade would require evaluating two already-slower mip tables around every crossing and retaining transition state. The candidate failed before that extra work could earn its cost.

## Eligibility and integration cost

The representation is only naturally eligible for an immutable static curve and a phase-step bound known to cover every lane. Existing KURV behavior makes the other cases materially harder:

- audio-rate curve morph and VA-table interpolation produce a new polynomial curve continuously; compiling FFT mips is forbidden on the audio thread, while evaluating and mixing two table banks doubles lookup cost;
- warp requires a conservative maximum derivative in the phase-step bound, and nonlinear/audio-rate warp or PM creates sidebands not described by merely truncating the unwarped source spectrum;
- pitch glides cross discrete caps and exhibit the jumps measured above;
- unsupported or out-of-band steps must keep the existing renderer rather than mute through `None`.

One `BandlimitedWaveCurve` is 163,840 bytes versus 256 bytes for `WaveCurveRt`. A full 16-frame VA table is 2,621,440 bytes rather than 4,096 bytes, a 640x increase before alignment, ownership, transition copies, or publication state. KURV's current curve publication transfers 64 atomic words per curve. Atomically copying 40,960 mip words per curve is not reasonable; immutable pointer publication would be a new ownership/reclamation seam and still would not cure the CPU or transition losses. No such seam was added.

## Verdict

Reject this existing spectral-mip bank as a production high-note crossover. It improves stationary ideal-reference error over the current analytic curve and shipping factor 2 in these probes, and its interpolation is excellent, but it loses CPU to both required paths, clicks at cap transitions, is not naturally eligible for morph/warp/PM, and carries prohibitive per-frame storage/publication costs.

No production code, runtime state, memory increase, or benchmark seam remains. A future mip experiment would need materially different evidence: compact caps/tables, click-safe continuous bandwidth, and a proven vector path across shipping architectures—not another integration of this 160 KiB object.
