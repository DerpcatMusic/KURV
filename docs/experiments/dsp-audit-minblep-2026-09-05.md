# Minimum-phase BLEP experiment: evidence and limits

The completed release probe does **not** establish a replacement for KURV's shipping oscillator, and does not test continuous audio-rate phase modulation. Both the original and corrected measurements were rerun. The correction changes the measurement contract, not oscillator audio.

## What actually ran

Evidence: `/tmp/kurv-minblep-20260905.log`, existing ignored test `oscillators::va::minblep_experiment::sparse_minblep_ring_report`, completed with 1 passed, 426 filtered out, in 112.66 seconds. Its implementation is `src/oscillators/va/minblep_experiment.rs`; shared Fourier reference and alignment are in `src/oscillators/va/experiment.rs`. Both raw runs, compiler information and binary hashes are retained in [the measurement log](dsp-audit-minblep-2026-09-05.log).

- Saw, square and 31% pulse; minimum-phase step residuals with 8/16/32 taps and 16 fractional table positions per tap. Tables occupy 516/1028/2052 bytes; reported scalar state is 48/80/144 bytes and eight-lane state is 352/608/1120 bytes.
- Steady quality uses periods 436/55/7 at 48 kHz: approximately 110.091743/872.727273/6857.142857 Hz, each over 32 periods after warm-up. These are coherent pitches, not exact 110/880/7040 Hz claims.
- Separate fractional-phase probes cover 16 offsets at period 7. Transition probes change pitch, pulse width or reset phase; scalar/vector parity runs 4096 frames with transitions at 1024/2048/3072. Parity tests agreement between implementations, not agreement with an independent acoustic reference.
- CPU loops cover scalar and eight-lane paths, block sizes 24/32 and pitches 440/3520/7040 Hz. This run overlapped other workloads, including reported busy-loop workers. Its timings and ratios are unsuitable for speedup claims, even though the output completed successfully.

## Measurement defects in that run

`alias_error_db` is `db_ratio(sum((aligned_candidate-reference)^2), sum(reference^2))`: total aligned reconstruction error, including gain, phase, DC and missing/incorrect wanted harmonics. It is **not isolated alias energy**. Off-harmonic residual would also miss folded components landing on wanted harmonic bins. Retain distinct reconstruction, wanted-bin and nonharmonic measurements rather than treating one as a complete alias estimate.

The shared pulse reference contains DC `-0.38` (the raw +/-1 waveform at duty 0.31), whereas shipping pulse is now zero-mean. Example: shipping 1x at 110.091743 Hz reports mean 0.000000363, curve RMS 0.380099425 and `alias_error_db=-8.394`. The approximately 0.38 RMS discrepancy is dominated by the obsolete reference mean; it does not demonstrate poor shipping pulse alias rejection. A corrected comparison must explicitly apply the same zero-mean contract to both candidate and reference, while separately reporting any measured DC defect.

Even without that pulse defect, the sampled saw rows do not justify a general quality win. At 872.727273 Hz, shipping 1x reports reconstruction error -31.117 dB and minimum-phase 8-tap reports -21.978 dB; these are reconstruction figures under fitted alignment, not independently isolated alias measurements. Minimum-phase magnitude and phase behavior must be evaluated separately.

The event detector handles forward phase advance, wrap crossings and pulse edges. A one-time reset probe is not continuous PM: it does not establish correctness for reverse phase traversal, several discontinuity crossings per sample, continuously moving edges, or modulation sidebands. Increasing residual support alone does not establish those properties.

## Corrected results

The corrected report finished in 114.36 seconds. It subtracts each signal's mean before measuring **AC reconstruction**, reports the candidate DC separately, and retains wanted-bin magnitude and complex-error diagnostics. The experimental MinBLEP oscillator is unchanged: its nonzero pulse DC remains visible and is not silently repaired. Analytic controls distinguish a 6.0206 dB gain loss, a 0.38 DC offset, an independent spurious tone, and invalid audio.

| Shipping 1x 31% pulse | Earlier total RMS (includes reference DC mismatch) | Corrected AC RMS | Corrected AC reconstruction error |
| --- | --- | --- | --- |
| 110.091743 Hz | 0.380099425 | 0.008677378 | -40.545 dB |
| 872.727273 Hz | 0.380728332 | 0.023525585 | -31.817 dB |
| 6857.142857 Hz | 0.381933092 | 0.038239216 | -27.006 dB |

These are different metrics on the same oscillator, **not an audio quality improvement**. Neither isolates aliases that coincide with wanted harmonics. The before binary used thin LTO and one codegen unit; the corrected validation binary used no LTO and 32 codegen units. In addition to machine contention, that build difference rules out interpreting their CPU rows as before/after performance evidence.

Reproduce from a checkout with the existing local build prerequisites described in [the continuation report](dsp-audit-continuation-2026-09-05.md):

```sh
cargo test --release --lib --no-default-features --features clap,vst3 minblep_quality
cargo test --release --lib --no-default-features --features clap,vst3 sparse_minblep_ring_report -- --ignored --nocapture --test-threads=1
```

## Primary research checked on 2026-09-05

These are research directions, not measured KURV wins. Publication years below distinguish foundational work from recent work.

| Primary source | Relevant result and limit |
| --- | --- |
| Franck and Välimäki, [Higher-Order Integrated Wavetable Synthesis](https://www.dafx.de/paper-archive/2012/papers/dafx12_submission_69.pdf), DAFx 2012 | Repeated table integration and output differentiation improve alias suppression over first order; the paper also examines interpolation and quantization. Its basic wavetable/resampling evaluation does not validate arbitrary audio-rate PM. Relevance to KURV is a fixed-cost alternative to depositing longer residuals at every event, subject to a separate modulated-phase derivation and numerical checks. |
| La Pastina, D'Angelo and Gabrielli, [Arbitrary-Order IIR Antiderivative Antialiasing](https://www.dafx.de/paper-archive/2021/proceedings/papers/DAFx20in21_paper_27.pdf), DAFx 2021 | Replaces restrictive FIR antialiasing kernels with adjustable IIR rational filters for nonlinear functions. The authors explicitly identify linear input reconstruction as an SNR ceiling. Applying the method to a periodic phase-to-waveform map is a research inference, not a result this paper establishes for KURV's PM routing. |
| Gabrielli and Squartini, [Simplifying Antiderivative Antialiasing with Lookup Table Integration](https://www.dafx.de/paper-archive/2025/DAFx25_paper_30.pdf), DAFx 2025 | Numerically integrated lookup tables supply antiderivatives, including second-order construction, without requiring unwieldy symbolic expressions. This is relevant to arbitrary curve evaluators; the published waveshaper examples do not settle periodic wrap continuity, modulated phase reversal, or oscillator CPU cost. |
| Gabrielli and Squartini, [PolyADAA](https://dafx26.mit.edu/assets/papers/DAFx26_paper_47.pdf), DAFx 2026 | Replaces linear input reconstruction with quadratic/cubic Lagrange interpolation and uses Chebyshev approximation to evaluate the resulting antialiasing integral. This directly addresses a limitation of higher-order ADAA. The paper concerns memoryless nonlinearities; applying it to wrapped, reversing PM trajectories is an inference that still needs oscillator-specific quality and CPU evidence. No implementation of PolyADAA is included here. |

## Remaining evidence gaps

Existing analytic/integrated experiments are documented in `canonical-analytic-adaa-round-10-2026-08-30.md` and `va-custom-exact-adaa-2026-08-30.md`. They remain research evidence, not a selected production replacement. This continuation implements no higher-order oscillator candidate.

Unproven workloads include continuous PM with near-zero or negative effective phase increments, multiple wrap crossings, and deep modulation chains. A reference would need convergence at successively higher sample rates, with passband magnitude, timing, DC and reconstruction assessed separately. Runtime superiority also remains unproven; it requires an uncontended machine and paired runs through the same production paths.
