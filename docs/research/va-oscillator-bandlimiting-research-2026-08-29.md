# KURV VA oscillator research: drawn-curve accuracy, antialiasing, and CPU

Date: 2026-08-29

Inspected KURV revision: [76545de8706db95bdee1cfd44e40a5cf8e521996](https://github.com/DerpcatMusic/KURV/commit/76545de8706db95bdee1cfd44e40a5cf8e521996)

## Scope and evidence rules

This record answers three separate questions:

1. Where can KURV use objectively less CPU?
2. Where can KURV reproduce the ideal band-limited projection of a drawn curve more accurately?
3. Under which operating conditions can one design improve both at once?

Claims in this note use four evidence classes:

- **Observed:** directly visible in the pinned KURV source revision.
- **Published:** stated or measured by a linked primary paper or owner-maintained implementation.
- **Mathematical consequence:** follows from the equations shown here and does not establish an implementation benchmark.
- **Hypothesis:** a production recommendation that must pass the stated KURV benchmark gate before being called a win.

No proposed optimization is described as measured KURV performance. The approximately -33 dB KURV versus -37 dB Dune comparison is user-reported context, not reproducible evidence yet; window, frequency, waveform, normalization, harmonic exclusion, host, build flags, and CPU workload must be fixed before the values can support a product claim.

## Decision brief

- **Accept:** retain the continuous-phase analytic curve engine as KURV's primary VA identity. Small residual tables may describe an antialiasing filter; they do not turn the waveform into a wavetable.
- **Accept first:** fix the drawn-curve representation and expose its derivative events before replacing the current four-point kernel. The active full-custom path currently receives no custom-wave BLEP or BLAMP correction.
- **Accept as the main candidate:** shared adaptive polynomial segments, forward-difference evaluation when parameters are stable, and sparse fractional-time residuals for value, slope, curvature, and periodic-wrap events.
- **Accept as an optional high-note backend:** derive exact Fourier coefficients from the same polynomial curve and switch to a truncated additive evaluator only where the surviving harmonic count is cheaper than the analytic event path.
- **Accept as a hostile-modulation fallback:** mild adaptive oversampling for audio-rate VATABLE movement, PM, and time-varying warp. These operations create broadband sidebands between the ordinary curve events.
- **Benchmark, do not assume:** analytic ADAA or AA-IIR is a credible comparison backend, but ordinary ADAA runs over every sample and is not intrinsically selective.
- **Reject:** declaring the existing approximately -33 dB result a limit of KURV's four-point PolyBLEP. The full-custom waveform bypasses that correction in the inspected code.
- **Reject:** claiming exact raw visual shape and zero aliasing simultaneously when the drawing contains energy above Nyquist. Accuracy must mean closeness to the ideal band-limited projection of the intended continuous curve.

## 1. Pinned source audit

### 1.1 The full-custom curve bypasses custom-wave antialiasing

**Observed.** In the eight-lane custom generator, a full custom mix advances phase and directly returns the curve evaluator. The antialiasing selector is not consulted on this branch:

- [generate_custom8 full-custom branch](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/oscillators/va/render.rs#L121-L136)
- [constant block full-custom branch](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/oscillators/va/render.rs#L913-L964)
- [four-lane full-custom branch](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/oscillators/va/render.rs#L2221-L2255)

When custom mix is below one, the canonical saw, pulse, or triangle component receives its existing edge correction, but the custom component is still a direct curve evaluation. Therefore aliasing from the drawn curve's wrap, corners, fitted joins, and clamp crossings is not corrected.

**Verdict:** connect the compiled custom curve's own differential events to the antialiasing system before judging or replacing the existing canonical four-point residual.

**Proof for verdict:** the source branches above return or mix curve.eval4/curve.eval8 without any correction derived from the custom curve. A different canonical kernel cannot remove aliasing generated independently by the custom component.

### 1.2 The editor curve is forced into sixteen independent uniform pieces

**Observed.** The realtime representation has sixteen segments and four coefficients per segment:

- [WaveCurveRt segment constants](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/wave_curve.rs#L22-L24)

The ordinary compiler samples each uniform interval at local positions 0, 1/3, 2/3, and 1, then independently fits one cubic:

- [uniform cubic compilation](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/wave_curve.rs#L294-L326)

The endpoints agree in value, so neighboring pieces are value-continuous. Their slopes and curvatures are generally unrelated because the fits are independent. A visually smooth source curve can therefore acquire fifteen artificial derivative events. Tight-transition fallback compilation instead samples only the seventeen uniform boundaries and creates linear pieces, which can lose narrow detail.

**Mathematical consequence.** For a periodic function, a value jump produces an asymptotic spectral tail proportional to 1/k; a slope jump produces roughly 1/k²; a curvature jump produces roughly 1/k³. Enforcing C1 or C2 continuity where the editor asks for a smooth join reduces the unwanted tail before any runtime antialiasing.

**Verdict:** use an error-driven shared partition across every VATABLE frame. Preserve explicitly hard editor points, but make smooth points C1 and preferably C2. The compiler should split where approximation error or curvature requires it instead of spending equal capacity on every sixteenth of the cycle.

**Proof boundary:** the pinned compiler has a fixed positional budget rather than an error budget and does not constrain neighboring derivatives. An error-driven splitter makes the actual approximation tolerance explicit and directs new pieces to the largest measured error. At the same hard segment cap it is not automatically superior for every possible curve, so the compiler must compare both representations and reject any candidate that fails the declared tolerance.

### 1.3 Runtime clamping creates hidden nonlinear corners

**Observed.** Scalar, four-lane, and eight-lane evaluators clamp every polynomial result to [-1, 1]:

- [runtime curve clamps](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/wave_curve.rs#L350-L408)

If a fitted polynomial overshoots and crosses a limit, the clamp creates a new slope discontinuity at a phase that is not represented in the curve topology. It is therefore both a small permanent CPU cost and an untracked alias source.

**Verdict:** solve the extrema of every compiled polynomial off the audio thread. Adjust shape-preserving tangents or subdivide until the segment is range-safe, then remove the audio-rate clamp. If hard limiting is intentional, compile its exact crossing phases as explicit derivative events.

**Proof for verdict:** the derivative of a cubic is quadratic, so all interior extrema are available analytically during compilation. Once endpoints and extrema are verified inside the range, a runtime clamp cannot change the output and is redundant.

### 1.4 The common one-lane custom path is scalar over time

**Observed.** The single-lane block renderer builds the output array by calling generate_custom_step once per frame:

- [single-lane custom block path](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/voices/voice/block_render.rs#L1534-L1555)

The dense-unison path already evaluates independent voices in SIMD lanes. The one-unison path does not instead pack consecutive time samples into those lanes.

**Hypothesis:** for block-stable parameters, form eight consecutive phases and use the existing eight-lane curve evaluator. This should reduce loop, indexing, and dispatch overhead on AVX2. It remains a hypothesis until paired benchmarks show a release-build gain without changing samples outside the declared rounding tolerance.

### 1.5 The portable four-lane selector scans every segment

**Observed.** Outside the AVX2/FMA path, eval4 selects coefficients by scanning segments 1 through 15. Each iteration performs one comparison and blends all four coefficients:

- [portable select4 scan](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/wave_curve.rs#L468-L483)

That is fifteen vector comparisons and sixty coefficient blends before the polynomial itself is evaluated.

**Verdict:** prototype indexed scalar lane loads followed by vector Horner evaluation, and a dedicated NEON/time-SIMD path. This is the clearest portable CPU target because the existing operation count is source-proven.

**Proof boundary:** the new method has a lower explicit selector operation count, but cache behavior and compiler lowering decide wall-clock performance. Only the paired Apple Silicon and baseline-x86 benchmarks can prove the final verdict.

### 1.6 A conventional spectral mip compiler already exists

**Observed.** KURV contains a twenty-level bank of 2,048-sample tables with harmonic caps from 1 through 1,023 and four-point periodic Catmull-Rom evaluation:

- [spectral bank layout](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/wave_curve/bandlimit.rs#L1-L35)
- [four-point mip evaluation](https://github.com/DerpcatMusic/KURV/blob/76545de8706db95bdee1cfd44e40a5cf8e521996/src/wave_curve/bandlimit.rs#L208-L228)

The active custom render branches inspected above use WaveCurveRt directly. The spectral module is valuable as a comparison backend and test oracle, but a sampled 2,048-point phase function should not define KURV's primary analytic identity.

## 2. What “accurate to the drawn curve” can objectively mean

Let the intended periodic continuous curve be g(φ), where phase φ is in [0, 1). At note frequency f0 and sample rate Fs, a naive analytic oscillator samples:

\[
y[n] = g((\phi_0 + n f_0/F_s) \bmod 1).
\]

This has continuous phase-address resolution. It does not have infinite output bandwidth: no sampled system can represent components above Fs/2.

The objective target is the ideal band-limited projection:

\[
g_{BL}(\phi; f_0) =
\sum_{|k| \le \lfloor F_s/(2f_0)\rfloor}
c_k e^{j2\pi k\phi},
\qquad
c_k = \int_0^1 g(\phi)e^{-j2\pi k\phi}\,d\phi.
\]

Consequences:

- A narrow corner may become visibly smoother at high notes and still be more accurate.
- A method must not be rewarded for reducing aliasing merely by dulling wanted harmonics.
- Accuracy measurement must include complex wanted-harmonic error, not only energy outside harmonic bins.
- “Calculate exact sample placement per note” describes the phase accumulator correctly, but exact placement alone does not low-pass an above-Nyquist corner.

## 3. Main architecture: adaptive analytic segments plus sparse event residuals

### 3.1 Compile once off the audio thread

Compile every VATABLE frame onto one shared phase partition containing:

- polynomial coefficients for each segment;
- exact segment boundaries;
- range/extrema proof;
- value jump Δ0;
- slope jump Δ1;
- curvature jump Δ2;
- optionally third-derivative jump Δ3;
- wrap events at phase zero;
- intentional hard-point metadata.

The common partition matters. Linear VATABLE interpolation can then interpolate coefficients and all derivative jumps without changing event topology:

\[
\Delta_r(\mu) = (1-\mu)\Delta_{r,A} + \mu\Delta_{r,B}.
\]

This follows from linearity of interpolation and differentiation. It avoids running two independently corrected oscillators merely to morph between neighboring frames.

### 3.2 Evaluate stable polynomial runs through forward differences

For equally spaced evaluations of one cubic, initialize y, Δy, Δ²y, and Δ³y exactly, then advance with additions:

    output y
    y   <- y   + Δy
    Δy  <- Δy  + Δ²y
    Δ²y <- Δ²y + Δ³y

**Mathematical consequence:** this replaces repeated cubic Horner evaluation with three recurrence additions inside a segment.

**Conditions:**

- constant or block-stable phase increment;
- constant segment coefficients;
- no uncompiled time-varying phase warp;
- exact reload at boundaries;
- periodic Horner resynchronization or higher-precision state to bound drift.

Audio-rate pitch, morph, PM, or warp falls back to direct evaluation unless a safe block-local polynomial composition is available.

### 3.3 Correct only differential events

When phase crosses a compiled boundary, determine its exact fractional time inside the sample and add the appropriate band-limiting residual:

| Continuous-time event | Stored amount | Correction family |
|---|---:|---|
| Value step | Δ0 | BLEP / integrated impulse order 1 |
| Slope change | Δ1 | BLAMP / order 2 |
| Curvature change | Δ2 | Next integrated residual / order 3 |
| Third-derivative change | Δ3 | Order 4 residual or accept its faster-decaying remainder |

The residual can be an 8-, 12-, 16-, or 24-sample fractional-delay kernel deposited into a fixed ring buffer. A small table of correction kernels is not a waveform table: it represents one shared low-pass filter response, independent of the drawn wave.

This is the rigorous version of “antialias only the complex part.” Smooth polynomial spans receive no event-filter work. The correction has a finite tail around each event; it cannot physically affect only one sample.

### 3.4 Published proof that event correction and mild oversampling are complementary

The 2026 Arbitrary Polygon Oscillator derives derivative jumps from editable Bézier geometry, determines exact fractional vertex crossings, applies four-point PolyBLAMP, and then uses adaptive oversampling. Its published experiment reports approximately 22 dB better alias SNR from BLAMP alone relative to naive rendering. BLAMP plus 2x oversampling is best in every row of its table.

Primary source:

- [Arbitrary Polygon Oscillator, Sections 6.1.1–6.1.3](https://arxiv.org/html/2608.24726v1)

This is evidence for the architecture, not a promised KURV number. Its waveform, metric, frequencies, filter, and implementation differ from KURV. KURV's advantage is that polynomial boundaries and derivatives can be compiled directly instead of recovered from a runtime geometry traversal.

### 3.5 Higher-quality residual candidate

Signalsmith's Elliptic BLEP is explicitly designed to sample continuous-time polynomial-segment signals through a predesigned eleventh-order continuous-time elliptic low-pass. Its API accepts the event amount, derivative order, and fractional samples in the past:

- [Elliptic BLEP README](https://github.com/Signalsmith-Audio/elliptic-blep/blob/main/README.md#how-to-use)
- [filter coefficients and supported integrated orders](https://github.com/Signalsmith-Audio/elliptic-blep/blob/main/elliptic-blep.h)

The supplied implementation advances eight complex-treated pole states per oscillator instance every sample. That constant state cost could damage KURV's excellent per-lane unison scaling.

**Verdict:** use Elliptic BLEP as a quality reference, an optional Ultra candidate, and a design target for short shared fractional FIR residuals. Do not select the per-lane IIR implementation without unison benchmarks.

**Proof for verdict:** the source loops over all pole states during every step, whereas a sparse FIR residual pays mainly when an event is crossed. Which is cheaper depends on event density; the proposed benchmark must measure both.

## 4. CPU-only opportunities

| Candidate | Source/mathematical proof | Expected winning region | Proof still required |
|---|---|---|---|
| Time-SIMD one-lane custom rendering | Current path calls one scalar step per frame | One-unison, stable block, AVX2 | Paired cycles/frame and checksum |
| Replace portable select4 scan | Current path performs 15 comparisons plus 60 coefficient blends | Apple Silicon, baseline x86 four-lane path | NEON/SSE release disassembly and benchmark |
| Cubic forward differences | Three-add recurrence shown above | Static note, morph, and warp | Drift bound, boundary cost, wall-clock benchmark |
| Compile range safety and remove clamp | Cubic extrema are analytically enumerable | Every custom evaluation | Exact range tests over all compiled segments |
| Precompute repeating event metadata | Static phase increment makes crossing order deterministic | Held static notes | Must retain exact fractional time and pitch |

These candidates do not by themselves prove lower aliasing. Range-safe compilation is the exception: eliminating an active hidden clamp corner can also improve accuracy.

## 5. Accuracy-only opportunities

| Candidate | Accuracy target | Cost expectation |
|---|---|---|
| 64x/128x f64 reference plus steep offline decimation | Numerical approximation to the ideal continuous curve projection | Offline only; test oracle |
| Exact Fourier coefficients of piecewise polynomials | Static ideal band-limited projection within coefficient precision | Potentially expensive at low f0 |
| Longer fractional residual kernels | Closer approximation to the selected antialiasing filter | More work per event |
| 2x–4x synthesis with existing steep decimation | Broadband suppression under time variation | Roughly multiplies oscillator work |
| Elliptic BLEP direct/residue mode | Sharp continuous-time filter response for polynomial events | Constant pole-state work per lane |

## 6. Where CPU and accuracy can improve together

### 6.1 Static and moderately moving low/mid notes

Use:

1. adaptive/shared curve compilation;
2. range-safe C1/C2 smooth joins plus explicit hard points;
3. forward-difference evaluation;
4. sparse event residuals.

The evaluator removes generic work from smooth samples while the residual corrects precisely the phases responsible for the slowest spectral decay.

Illustrative operation density, not a benchmark: sixteen boundaries at 440 Hz produce 7,040 boundary crossings per second. A twelve-tap deposit produces 84,480 tap additions per second, averaging 1.76 tap additions per 48 kHz output sample. Actual density depends on the adaptive segment count and which derivative jumps are nonzero.

### 6.2 Exact-Fourier crossover for high notes

Because every segment is polynomial, compute its Fourier coefficients during curve compilation. For a static note, retain only:

\[
H = \left\lfloor \frac{F_s}{2f_0} \right\rfloor
\]

positive-frequency harmonics. At 48 kHz:

| Fundamental | Maximum ordinary harmonic index |
|---:|---:|
| 440 Hz | 54 |
| 4 kHz | 6 |
| 8 kHz | 3 |

At sufficiently high notes, a few recursively advanced complex partials can be both cheaper and closer to the exact band-limited curve than evaluating sharp events. The crossover must be measured; six partials are not automatically cheaper than one polynomial plus a sparse residual.

For a linear VATABLE morph with runtime clamp removed:

\[
c_k(\mu) = (1-\mu)c_{k,A} + \mu c_{k,B}
\]

is exact. This backend does not have a 2,048-sample phase grid and has no waveform-table interpolation error. It is genuinely additive internally, which is why it remains an optional high-note backend rather than the product's authoring model.

### 6.3 Audio-rate VATABLE movement, PM, and time-varying warp

At a morph rate such as 1.9 times the note frequency, oscillator and modulator phases do not generally return to their joint starting state after one host cycle. Coefficient modulation creates legitimate sidebands; some may exceed Nyquist even if every static frame is band-limited.

The event engine still corrects moving value/slope/curvature events, but it cannot remove all broadband modulation aliasing for free.

**Verdict:** retain exact live evaluation and event placement, then enable mild adaptive oversampling according to effective phase velocity, PM/warp depth, and control bandwidth. This is the region where “both” is conditional rather than guaranteed.

**Published support:** the 2026 polygon paper's event correction and oversampling are complementary, with their combination winning every reported test row. The exact KURV adaptation remains benchmark-dependent.

## 7. ADAA and AA-IIR verdict

Antiderivative antialiasing reconstructs an interval between samples, applies a continuous-time antialiasing kernel, and samples the result. It is not inherently an event-only method.

Relevant primary work:

- [Antiderivative Antialiasing for Arbitrary Waveform Generation](https://dangelo.audio/publications) applies virtual IIR downsampling to arbitrary piecewise-linear waveforms.
- [IEEE/ACM paper record and DOI](https://ieeexplore.ieee.org/document/9854137)
- [Interpolation Filters for Antiderivative Antialiasing, DAFx24](https://www.dafx.de/paper-archive/details/VEph91o4UTFBAN6Fha2z_A) reports improved memoryless-nonlinearity suppression with cubic interpolation, while its stateful stability analysis remains in favor of linear interpolation.
- [Simplifying Antiderivative Antialiasing with Lookup Tables, DAFx25](https://www.dafx.de/paper-archive/2025/DAFx25_paper_30.pdf) makes higher antiderivatives practical for functions that do not have convenient closed forms.

KURV's cubic segment already has an exact quartic first antiderivative, so the main ADAA-LUT motivation is weaker here. An interval crossing a segment boundary or wrap must still be split correctly. Higher order adds state, delay, near-equal-input handling, and potential passband droop.

**Verdict:** implement exact polynomial ADAA/AA-IIR as a benchmark candidate, particularly for comparison under difficult modulation. Do not choose it as the primary architecture until it beats sparse event residuals on both wanted-spectrum error and cycles.

## 8. Objective benchmark and proof contract

### 8.1 Reference renderer

For each editor source curve and modulation case:

1. Evaluate the original source curve in f64 at 128x the host rate.
2. Apply a documented steep linear-phase offline low-pass.
3. Downsample to the host rate.
4. For static piecewise-polynomial cases, independently compare against exact integrated Fourier coefficients.

Agreement between the oversampled and Fourier references bounds mistakes in either oracle.

### 8.2 Accuracy metrics

- Integrated error energy against the reference.
- Static alias energy outside expected harmonic-bin neighborhoods.
- Complex magnitude and phase error of every wanted harmonic below Nyquist.
- Maximum wanted-harmonic attenuation; a duller result is not an AA win.
- Time-aligned RMS and peak error.
- Event-local waveform error around wraps, corners, and clamp-limit cases.

Static test sweep:

- MIDI 24–120;
- 44.1, 48, 88.2, 96, and 192 kHz;
- saw, pulse, triangle, smooth curve, explicit step, narrow corner, pathological overshoot, and every shipped VATABLE frame/midpoint;
- phase offsets chosen both exactly on and fractionally between samples.

Dynamic test sweep:

- VATABLE morph at DC, 0.5 f0, 1.0 f0, and 1.9 f0;
- pitch bend and vibrato;
- PM and every phase-warp mode at conservative and maximum depths;
- sync/reset if introduced later;
- quality transitions and note stealing.

### 8.3 CPU metrics

Measure release/production builds with fixed CPU affinity and paired alternating runs:

- ns/frame and cycles/output-sample;
- p50 and p99 callback time;
- unison 1, 2, 4, 8, 16, 32, and 64;
- one and many active notes;
- static, control-rate, and audio-rate modulation;
- 1x through 4x quality;
- AVX2/FMA Ryzen 7 7800X3D, Apple Silicon NEON, and baseline x86-64.

Record checksums and compare audio against the declared numerical tolerance. No allocation, locking, unbounded event loop, or data-dependent callback-size growth is acceptable.

### 8.4 Acceptance thresholds

- **Measured CPU win:** lower paired median and p99 without accuracy regression. A “much better CPU” headline requires at least 20% in the affected shipping scenario; smaller repeatable wins may still be accepted as ordinary optimization.
- **Measured accuracy win:** at least 10 dB lower integrated alias/error energy in the declared target set, with wanted harmonics within 0.1 dB unless a stricter reference tolerance is chosen.
- **Measured combined win:** lower cycles and lower reference error at the same note, unison, modulation, sample rate, and latency. This is Pareto dominance.
- **External Dune comparison:** identical input curve where possible, pitch, gain, sample rate, render length, FFT window, bin exclusion, plugin quality, polyphony, unison, and measured host CPU. Otherwise label the comparison informal.

## 9. Recommended implementation sequence

1. Add the reference renderer and metric harness before changing DSP.
2. Compile a shared adaptive VATABLE partition with derivative metadata and range proof.
3. Remove runtime clamp only after exhaustive range tests pass.
4. Generalize the existing four-point system to custom Δ0/Δ1/Δ2 events as the cheapest baseline.
5. Prototype 12-, 16-, and 24-tap fractional event residuals, including an order-4 candidate.
6. Prototype exact polynomial ADAA/AA-IIR and Signalsmith Elliptic BLEP as comparison backends.
7. Add one-lane time-SIMD, portable selector replacement, and forward differences as isolated benchmarkable changes.
8. Add the exact-Fourier high-note backend and choose its crossover from measured Pareto results.
9. Gate adaptive 2x+ on measured modulation hostility rather than note pitch alone.
10. Preserve every accepted optimization as a small checkpoint and delete trials that do not beat paired medians.

## 10. Verdict-to-proof ledger

| Verdict | Evidence/proof | What would overturn it |
|---|---|---|
| Keep analytic phase-function rendering as primary | Current WaveCurveRt evaluates mathematical polynomial pieces at continuous phase; correction tables need not contain waveform samples | A sampled backend Pareto-dominates analytic rendering across all target notes/modulation while satisfying the product identity |
| Fix custom event coverage before replacing four-point PolyBLEP | Pinned full-custom branches call curve.eval directly; canonical correction cannot cancel independent custom aliasing | A source revision showing equivalent custom derivative correction elsewhere in the active path |
| Replace uniform independent fits with adaptive shared segments | Pinned compiler uses sixteen equal intervals and unconstrained neighboring derivatives | Tests show the current representation already meets the chosen source-error and derivative-continuity bounds for every editable curve |
| Remove runtime clamp through range-proof compilation | Cubic extrema are analytically solvable; pinned evaluator clamps every sample | Compilation cannot guarantee range under the full morph/warp contract, or removing the clamp fails safety tests |
| Use sparse derivative-event residuals as the main AA candidate | DAFx26 publishes effective geometry-derived fractional BLAMP; KURV can precompute polynomial derivatives | ADAA, Elliptic BLEP, spectral, or oversampling backend Pareto-dominates it across the complete matrix |
| Use forward differences only for stable runs | Three-add recurrence is exact in ideal arithmetic for equally spaced cubic samples; modulation changes the recurrence | A direct SIMD Horner path is measured faster or forward-difference drift/boundary overhead exceeds tolerance |
| Keep Elliptic BLEP as reference/optional until profiled | Owner source advances all pole states per sample; KURV unison efficiency depends on low per-lane constant work | Cross-lane SIMD makes it Pareto-superior at target unison counts |
| Permit exact-Fourier high-note crossover | Harmonic cap falls as floor(Fs/(2f0)); polynomial Fourier coefficients are computable off-thread | Measured partial recursion stays slower than analytic events through the playable high register |
| Use adaptive oversampling for hostile time variation | Audio-rate coefficient/phase modulation creates broadband sidebands; DAFx26 shows event AA plus oversampling are complementary | A 1x method matches the 128x reference at lower CPU throughout the dynamic matrix |
| Treat ADAA as a candidate, not the assumed winner | Published arbitrary-waveform support is credible, but KURV already has closed-form polynomial antiderivatives and ADAA works every interval | Exact polynomial ADAA Pareto-dominates sparse event residuals in KURV measurements |

## Bottom line

The strongest research-backed KURV target is:

**shared adaptive C1/C2 polynomial VATABLE curve → range-safe analytic compilation → forward-difference stable evaluator → sparse exact-time derivative residuals → exact-Fourier high-note crossover → adaptive 2x only for hostile audio-rate modulation**

This retains continuous phase-address resolution, makes the drawn and sounded curve share one mathematical representation, spends antialiasing work where differential events actually occur, and preserves KURV's SIMD/unison advantage. It is the leading hypothesis, not a performance claim, until the reference and paired benchmark gates above pass.

## Primary sources

- [KURV pinned source revision](https://github.com/DerpcatMusic/KURV/tree/76545de8706db95bdee1cfd44e40a5cf8e521996)
- [Arbitrary Polygon Oscillator: Generalizing Polygonal Synthesis to Arbitrary Shapes, Morphing, and Three-Dimensional Polyhedra, 2026](https://arxiv.org/html/2608.24726v1)
- [Signalsmith Elliptic BLEP owner-maintained implementation](https://github.com/Signalsmith-Audio/elliptic-blep)
- [Antiderivative Antialiasing for Arbitrary Waveform Generation, IEEE/ACM TASLP 2022](https://ieeexplore.ieee.org/document/9854137)
- [Author-maintained publication summary and code link for arbitrary-waveform ADAA](https://dangelo.audio/publications)
- [Interpolation Filters for Antiderivative Antialiasing, DAFx24](https://www.dafx.de/paper-archive/details/VEph91o4UTFBAN6Fha2z_A)
- [Simplifying Antiderivative Antialiasing with Lookup Tables, DAFx25](https://www.dafx.de/paper-archive/2025/DAFx25_paper_30.pdf)
- [Alias-Suppressed Oscillators Based on Differentiated Polynomial Waveforms](https://ieeexplore.ieee.org/document/5153306)
