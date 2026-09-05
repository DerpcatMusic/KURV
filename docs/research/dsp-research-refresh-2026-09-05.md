# Oscillator DSP research refresh — 2026-09-05

Inspected integration revision: `cc6968e5a6b650253805ce7416c980899fbc0ca9`.
This is a literature and source review, not a benchmark, implementation plan, or release acceptance record.

## Finding

There is relevant new research through DAFx26, including papers published in August 2026. It supports specialized improvements, not replacing KURV's complete oscillator engine with a universally faster antialiaser. No reviewed source establishes a fastest-CPU synthesizer, or an inexpensive general solution for arbitrary nested audio-rate PM with unchanged sound.

The [official DAFx26 program](https://dafx26.mit.edu/program/) confirms the September 1–4, 2026 conference and the two 2026 oscillator papers below. Its program is more current than the central DAFx archive, which still lists proceedings only through 2025. Publication dates below come from papers or author records, not search-engine crawl dates.

## Existing KURV evidence

- Canonical VA uses the existing spline correction variants, selecting the optimized variant at factors up to 2; see [`antialias.rs`](../../src/oscillators/va/antialias.rs). The [runtime AVX2 audit](../audits/pm-runtime-avx2.md) reports selected PM block gains while explicitly excluding whole-plugin speed and improved spectral quality. A faster equivalent polynomial evaluation is useful without a newer synthesis algorithm.
- Custom periodic curves already have an offline harmonic-mip compiler, contiguous aligned tables, pitch-dependent mip selection, and interpolated scalar/SIMD playback; see [`bandlimit.rs`](../../src/wave_curve/bandlimit.rs). This is an existing reuse point, not missing infrastructure. A stationary harmonic cap alone cannot certify a modulated waveform.
- The [existing spectral research](oscillator-domain-filtering-2026-08-31.md) distinguishes harmonic-prefix playback from FFT compilation. Its live Ratio contract rejects control-rate rebuild/crossfade as a substitute for audio-rate parameter movement.
- The prior session withdrew its particular minBLEP replacement recommendation after unfavorable speed/reconstruction measurements. The candidate remains in [`minblep_experiment.rs`](../../src/oscillators/va/minblep_experiment.rs); this refresh inspected its existence but did not reproduce that prior numerical verdict. That rejection applies to the measured candidate, not every minimum-phase BLEP implementation.
- PR #15's [1x investigation](https://github.com/DerpcatMusic/KURV/pull/15) was also read from the sibling `one-x/docs/audits/1x-dsp-improvements.md`. It records high-note specialization gains, a roughly 2.3–2.4× saw crossover cost, and intense nested-PM counterexamples. Signed event reconstruction was rejected as the default because its reported scalar cost was 7–8× with 17 samples of latency. These are historical experiment results, not newly reproduced measurements or proof of the current merged state.
- The [measurement audit](../audits/dsp-metrics-and-oversampling.md) correctly separates wanted-harmonic coloration, complex reconstruction error, and off-grid energy. Aliases can coincide with wanted bins. Neither a clean-looking harmonic grid nor a low off-grid reading proves alias-free synthesis.

## Eight strongest sources and their actual scope

### 1. Roth, Keller, Castañeda and Studer — Alias-Free Oscillator Synchronization via Additive Synthesis (2026)

[Author manuscript, v1, August 27](https://arxiv.org/html/2608.27648v1); [publication record](https://arxiv.org/abs/2608.27648).

The paper derives Fourier-coefficient transformations for hard, mirrored and pulsar sync, followed by additive synthesis. This is strong reference material for the intended spectrum of synchronized oscillators. It is not a cheap general software replacement: the coefficient transform is O(N²), with many trigonometric evaluations/divisions. Section 3 explicitly describes prohibitive software cost under changing sync ratios and instead introduces the specialized HASY ASIC. Its unoptimized NumPy timing is not a fair comparison with KURV's Rust SIMD renderer. Applicability inference: useful as an independent mathematical oracle or a deliberately limited partial-count method; it does not solve arbitrary nested PM or establish CPU savings in KURV.

### 2. Argentieri and Scagliola — Arbitrary Polygon Oscillator (2026)

[Author manuscript, v1, August 25](https://arxiv.org/html/2608.24726v1); [author demonstrations and implementation link](https://www.antonioargentieri.com/polygon_demo/).

The method derives polyBLAMP corrections from adjacent Bézier tangents, caches geometry, and combines correction with adaptive 2–6× oversampling. This is directly relevant to curves whose corners and derivative jumps are known. It confirms that procedural geometry can supply correction magnitudes without a separate derivation for every shape. It does not establish that correction alone removes the need for oversampling. Its harmonic-neighborhood SNR counts remaining spectral energy as aliasing; by KURV's documented metric limitation, that cannot identify folded components landing inside wanted neighborhoods. No KURV CPU result follows from this RNBO implementation, and its pitch-based oversampling heuristic is not a nested-PM bandwidth bound.

### 3. Gabrielli and Squartini — Simplifying Antiderivative Antialiasing with Lookup Table Integration (DAFx25)

[Proceedings paper](https://dafx.de/paper-archive/2025/DAFx25_paper_30.pdf), September 2–5, 2025.

Numerically integrated lookup tables replace analytic antiderivative evaluation for nonlinear transfer functions. The paper evaluates table-size/error tradeoffs, static nonlinearities, and a diode clipper. It reports gains against its analytic ADAA implementations; those are not gains against KURV's spline oscillator or mip lookup. Applicability inference: potentially valuable when a custom transfer function makes analytical ADAA expensive. The current curve tables already remove substantial runtime work, so a replacement must account for interpolation, divided-difference numerical stability, state and the intended input trajectory. This paper does not by itself justify another table bank.

### 4. Zheleznov and Bilbao — Interpolation Filters for Antiderivative Antialiasing (DAFx24)

[Proceedings paper](https://www.dafx.de/paper-archive/2024/papers/DAFx24_paper_33.pdf), September 3–7, 2024.

Higher-order interpolation can improve AA-IIR alias reduction for the studied memoryless nonlinearities. The paper compares operation counts and reports benefits for higher-frequency input; its stateful-system results are much less favorable because of compensation-filter stability restrictions. This is evidence that the reconstructed trajectory matters, not that increasing interpolation order is always beneficial. Applicability inference: relevant to KURV's distinction between correcting a linearly reconstructed PM path and recovering the intended continuous trajectory. It supplies no universal stability or quality guarantee for a nested generator graph.

### 5. Werner and Azelborn — Antialiasing Piecewise Polynomial Waveshapers (DAFx23)

[Proceedings paper](https://www.dafx.de/paper-archive/2023/DAFx23_paper_61.pdf), September 4–7, 2023.

The authors derive ADAA for piecewise-polynomial waveshapers and polynomial smoothing of jumps/corners. This is useful for explicitly represented custom curves and nonlinear warp functions. Smoothing changes the transfer function and therefore timbre; it cannot be sold as transparent alias removal. The paper also considers combining the approach with light oversampling. Applicability inference: relevant only where KURV can express the actual audible function in the supported polynomial form and retain its reset/modulation semantics.

### 6. Gabrielli, D'Angelo, La Pastina and Squartini — Antiderivative Antialiasing for Arbitrary Waveform Generation (2022)

[Author publication record and MATLAB implementation](https://dangelo.audio/taslp-antialias-waveform), IEEE/ACM TASLP 30, 2743–2753, DOI `10.1109/TASLP.2022.3198007`.

This connects oscillator antialiasing with ADAA, covering classical waveforms and arbitrary wavetables through AA-FIR/AA-IIR formulations. It is the most direct general-waveform lead in this review. The author-provided abstract and implementation establish the method's scope, not a measured advantage in KURV. Applicability inference: any comparison must include the existing mip compiler and the cost of arbitrary phase movement, rather than compare only against naive playback. Generality of the waveform representation is not proof of arbitrary nested-modulation bandlimiting.

### 7. Nielsen — Practical Linear and Exponential Frequency Modulation for Digital Music Synthesis (DAFx20)

[Proceedings paper](https://www.dafx.de/paper-archive/2020/proceedings/papers/DAFx2020_paper_61.pdf), September 2020.

This explicitly distinguishes linear FM, through-zero FM, PM and exponential FM, including pitch/DC behavior and practical antialiasing. It remains relevant because modulation semantics are the requirement being preserved: changing depth laws, discarding negative instantaneous frequency, or treating FM as PM can change the instrument. Applicability inference: a correct optimization must preserve those semantics throughout the graph. A stationary oscillator comparison is insufficient evidence for nested modulation.

### 8. Pekonen — Filter-Based Oscillator Algorithms for Virtual Analog Synthesis (2014 thesis)

[Aalto institutional repository](https://aaltodoc.aalto.fi/items/79571fc5-2f60-4534-afc1-7d10e429878a).

The thesis places polynomial/table-based bandlimited functions, nonlinear-phase functions and post-processing in a common oscillator framework. It is a useful foundational comparison and audibility context, not a current CPU ranking. In particular, the distinction between alias reduction and faithful waveform reproduction remains relevant to the rejected minBLEP candidate and compensating EQ. A modern paper does not invalidate an efficient older polynomial kernel that wins under the actual required workload.

## Remaining concrete evidence gaps

These are unresolved claims, not an implementation schedule:

- No quality-matched whole-synth comparison in this literature review establishes maximum polyphony or callback-tail behavior for consolidated KURV across hardware/hosts. Kernel timings and ASIC throughput cannot answer that question.
- No reviewed work establishes an inexpensive, generally alias-free solution for KURV's arbitrary nested PM, custom curves, changing depth, phase reversals and nonlinear warp together. Supported input trajectories and bandwidth limits remain necessary facts to state.
- Static mip safety does not establish safety under audio-rate phase/shape movement. Conversely, conservative harmonic removal can reduce desired content. Reconstruction error and wanted-spectrum change must remain separate claims.
- The 1x crossover and nested-PM counterexamples remain evidence against blanket promotion. A newer paper cannot erase those measured counterexamples.
- Full-host sample-rate response, latency, preset sound and project-reopen acceptance are not supplied by literature or this report.

## Search coverage and limits

Searches covered DAFx 2023–2026 proceedings/programs, exact paper titles, author publication/implementation pages, arXiv manuscripts, Aalto and other university repositories, SMC/Zenodo and ICMC archives. German oscillator/aliasing and French antirepliement/synthesis/thesis searches were included; most non-English hits concerned RF hardware or unrelated filtering and were excluded rather than padded into the evidence. The [SMC portal](https://smcnetwork.org/index.html) places SMC26 in November 2026, after this report date; it is not a source of already published September results. The [ICMC 1999 nonlinear-antialiasing paper](https://quod.lib.umich.edu/i/icmc/bbp2372.1999.340/1/--antialiasing-for-nonlinearities-acoustic-modeling?page=root;view=text) was checked for historical context but adds no modern KURV CPU evidence.

A less formal first-party [integrated-wavetable PM implementation note](https://joelkp.frama.io/tech/dpw-wavelut-pm.html), updated November 2024, discusses PM-aware divided differences and near-zero phase differences. It is a useful engineering lead, not a peer-reviewed nested-PM guarantee. DAFx26's program also lists PolyADAA and a time-varying-delay loopback-FM paper; this refresh did not obtain their full primary manuscripts and makes no algorithm/performance claim from their titles.

This was a focused, multi-source refresh, not proof that every paper in every language was discovered. No tests, benchmarks, or production DSP were added or changed.
