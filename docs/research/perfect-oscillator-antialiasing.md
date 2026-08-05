# Real-Time, Continuously Variable Virtual-Analog Oscillators Without a Fixed Wavetable

Research report — 2026-08-04

## Scope and conclusion

This report asks a narrower and harder question than “which oscillator has the least audible aliasing?” The target is a real-time oscillator whose curve can vary continuously, whose representation is procedural rather than a fixed 2048-sample wavetable, and whose behavior remains defensible under pulse-width modulation, hard sync, frequency modulation, pitch movement, and SIMD execution.

The short answer is that there is no single method that is simultaneously exact for every arbitrary curve and modulation, finite-latency, constant-cost, memory-free, and sample-perfect. The strongest designs are hybrids chosen around the signal class:

1. For a static periodic shape with known analytic or piecewise-polynomial coefficients, direct Fourier/additive synthesis is the cleanest exact reference. It can generate only harmonics below Nyquist and therefore has no aliasing under its stated assumptions, but its cost is pitch-dependent and becomes expensive at low frequencies.
2. For KURV’s current Sine → Triangle → Saw → Pulse family, event-aware procedural synthesis is the best practical fit: exact sine, higher-order polynomial BLEP for value discontinuities, polynomial BLAMP for corners, and exact fractional event placement. This remains local, table-free, SIMD-friendly, and constant-cost per sample for the current family.
3. For genuinely arbitrary procedural piecewise-linear or piecewise-polynomial curves, antiderivative antialiasing with an IIR or FIR virtual downsampler is the broadest no-fixed-table technique found. It is useful, but it is an approximation whose passband magnitude, phase, interpolation, and filter-order choices are part of the sound. It is not mathematically exact bandlimiting.
4. Oversampling/resampling is a valuable reference and fallback, especially around nonlinear processing or signal classes without a manageable event description. It does not make the oscillator itself exact; it only moves the first aliasing boundary upward and relies on a finite reconstruction filter.

The recommendation for KURV is therefore a two-tier oscillator core: make the current four-shape path an event-aware procedural oscillator, retain oversampling as a bounded fallback/quality mode, and add an AA-IIR-style procedural path only when the product really needs arbitrary spline-like curve definitions. Build a direct Fourier mode as an offline/reference oracle and an optional low-pitch quality mode, not as the only real-time engine.

This report uses original papers, theses, standards, official author/university copies, official repositories, and official author code pages only. It does not use secondary explanations, product marketing, or derivative summaries.

## What “perfect” can mean

### A reference that can actually be tested

Let (g(phi,m)) be a unit-period curve, with phase (phi(t)) and shape parameter (m(t)). The intended continuous-time signal is

\[
u(t)=g(\operatorname{frac}(\phi(t)),m(t)).
\]

Sampling the raw discontinuous function is not the analog reference. A defensible bandlimited reference first applies an ideal low-pass operator with bandwidth (B\leq F_s/2):

\[
y(t)=(h_B*u)(t),\qquad h_B(t)=2B\,\operatorname{sinc}(2Bt),
\]

and then samples (y(n/F_s)). This is the reconstruction model underlying Shannon’s sampling theorem, but it exposes the practical problem: the ideal sinc has infinite support and is noncausal for zero phase. See Claude Shannon’s original paper, [“Communication in the Presence of Noise,” Proceedings of the IRE, 1949](https://doi.org/10.1109/jrproc.1949.232969).

For a static periodic shape and constant frequency (f_0), write

\[
g(\phi,m)=\sum_{k=-\infty}^{\infty}c_k(m)e^{j2\pi k\phi}.
\]

The exact ideal-bandlimited periodic oscillator is then the finite series

\[
y[n]=\sum_{|k|\leq K}c_k(m)e^{j2\pi k\phi[n]},\qquad
K=\left\lfloor\frac{B}{f_0}\right\rfloor,
\]

assuming the coefficients are exact, the phase is exact, and the requested reference is the harmonic truncation at (B). This is “perfect” in a precise, testable sense: zero alias energy, exact in-band amplitude and phase relative to the chosen continuous-time low-pass reference, and exact event phase modulo the unavoidable smoothing of the bandlimited reference. Stilson and Smith make the same central distinction in [“Alias-Free Digital Synthesis of Classic Analog Waveforms,” ICMC 1996](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext): additive synthesis is trivially bandlimited when harmonics above Nyquist are omitted.

For a piecewise-polynomial curve, the coefficients do not require a wavetable. If segment (i) is a polynomial (p_i(\phi,m)) over ([a_i,b_i]),

\[
c_k(m)=\sum_i\int_{a_i}^{b_i}p_i(\phi,m)e^{-j2\pi k\phi}\,d\phi.
\]

Repeated integration by parts gives closed forms consisting of endpoint values, derivative jumps, and powers of (1/k). Thus “analytic Fourier coefficients” are a real route to exact static synthesis for a sufficiently described shape. They are not a free solution for arbitrary audio-rate shape movement: changing (m) is modulation, and modulation creates sidebands that must also fit the bandwidth budget.

### Four different claims that are often conflated

“Alias-free,” “bandlimited,” “smooth,” and “sounds clean” are not synonyms.

- **Exact bandlimiting:** the generated continuous-time reference has no content above the specified (B), and the samples equal that reference. Ideal sinc, exact Fourier truncation, or an exact continuous-time filter are the relevant standards.
- **Quasi-bandlimited approximation:** a finite correction, interpolation kernel, polynomial, or filter suppresses aliases but changes wanted-band magnitude and/or phase. PolyBLEP, BLAMP, DPW, PTR, EPTR, BLIT-FDF, and finite minBLEP are in this category.
- **Perceptual alias suppression:** alias components are below a masking threshold for a stated listening test. The threshold is useful engineering evidence, not mathematical exactness. The DPW and integrated-polynomial studies explicitly evaluate this way; for example, the fourth-order DPW results are reported as perceptually alias-free over a piano register under the authors’ conditions in [Välimäki, Nam, Smith, and Abel, TASLP 2010](https://doi.org/10.1109/TASL.2009.2026507) and the author-hosted [full paper](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf).
- **Post-filtered nonlinear processing:** oversampling or antiderivative antialiasing may reduce aliases created by a nonlinear operation. That is a different problem from generating the oscillator’s discontinuities correctly. Once a nonlinear sampler has folded energy into baseband, a later low-pass cannot identify which baseband energy was an alias.

### What “more accurate sample placement” actually means

The output can remain at the host's 1× sample rate while retaining sub-sample event timing. Suppose phase crosses a waveform knot at phase \(q\) between host samples. With a constant phase increment \(d\) and starting phase \(p_n\), the fractional crossing position is

\[
\mu=\frac{q-p_n}{d},\qquad 0\leq\mu<1.
\]

The correction kernel is then evaluated at offsets such as \(k-\mu\), not at integer \(k\). A step at \(\mu=0.17\) and one at \(\mu=0.83\) therefore produce different sequences of ordinary 1× output samples: ideal low-pass filtering spreads each event over neighboring samples, and the spread is shifted by \(\mu\). Oversampling is one way to approximate that shift, but it is not the only way. Variable fractional-delay BLEP/BLAMP, Farrow, spline, and direct residual-convolution methods encode the same timing directly. This is the central result of [Pekonen, Välimäki, Nam, and Smith, “Variable Fractional Delay Filters in Bandlimited Oscillator Algorithms,” ICGCS 2010](https://mac.kaist.ac.kr/pubs/Pekonen2010ICGCS.pdf).

For changing frequency, the simple division above is exact only if phase is linear across the sample. If phase increment is ramped, integrate that ramp and solve the resulting quadratic crossing equation. For hard sync, PWM, or a moving curve knot, solve the corresponding reset or moving-boundary crossing. The event's value and derivative jumps must be captured at that solved time. Variable-step virtual-analog circuit work reaches the same conclusion from another direction: locating a comparator transition inside the sample interval reduces oscillator frequency error, and the resulting fractional event can then drive a BLEP correction; see [Werner, Bank, and Parker, “Network Variable Preserving Step-Size Control in Wave Digital Filters,” DAFx 2017](https://dafx.de/paper-archive/2017/papers/DAFx17_paper_74.pdf).

KURV's current PolyBLEP already contains a basic fractional coordinate through `phase / phase_step`; it does not simply snap every edge to the nearest output sample. The quality ceiling is instead set by the very short quadratic residual, the missing triangle BLAMP, the assumption of one constant phase step during each sample, and numeric phase precision. `VaOscillator` currently stores phase as `f32`, including the SIMD paths. Converting that value to `f64` later in the scalar sampler cannot recover discarded phase bits. A `f64` or 64-bit fixed-point phase/event accumulator would reduce phase-quantization spurs and make event solving more stable while the final audio can remain `f32`. This is a secondary refinement, not a substitute for proper bandlimited event correction.

### Why universal perfection is impossible

There are several independent bounds.

1. A nonconstant discontinuity has infinite Fourier bandwidth. A finite-bandwidth version must round, spread, or ring around the event. The question is where to put the error: before/after the event, in the transition shape, in wanted-band amplitude, in phase, or in computational cost.
2. The ideal bandlimited step and ramp corrections have infinite support. A finite minBLEP, polynomial BLEP, or BLAMP is necessarily an approximation. The exact BLEP is the integral of sinc; the exact BLAMP is an integral of the bandlimited ramp. The finite-support formulas in [Välimäki, Pekonen, and Nam, JASA 2012](https://doi.org/10.1121/1.3651227) and [Esqueda, Välimäki, and Bilbao, “Rounding Corners with BLAMP,” DAFx 2016](https://dafx.de/paper-archive/2016/dafxpapers/18-DAFx-16_paper_33-PN.pdf) make this finite-support tradeoff explicit.
3. Exact additive synthesis costs (O(K)) per sample, where (K\approx B/f_0). At low pitch, many harmonics are wanted; at high pitch, fewer are wanted. A fixed constant-cost algorithm cannot also be exact for every pitch unless it hides work in a precomputed representation or accepts approximation.
4. Hard sync creates an event sequence whose rate depends on the slave/master ratio. Brandt’s original [“Hard Sync Without Aliasing,” ICMC 2001](https://www.cs.cmu.edu/~eli/L/icmc01/hardsync.html) identifies the cost of minBLEP-style correction as increasing with the number of slave impulses and notes sublinear exact synthesis as an open problem. A universal oscillator supporting arbitrary sync, PWM, FM, and arbitrary shape events cannot promise a single fixed operation count without limiting the signal class.
5. A causal IIR antialiasing filter has phase/group delay and a finite filter order. A linear-phase FIR has latency and finite transition-band error. The arbitrary-order antiderivative-antialiasing work by La Pastina, D’Angelo, Gabrielli, and collaborators explicitly treats filter order, passband, and phase as tunable compromises rather than exact reconstruction: [“Arbitrary-Order IIR Antiderivative Antialiasing,” DAFx 2021](https://www.dafx.de/paper-archive/2021/proceedings/papers/DAFx20in21_paper_27.pdf), and [“Antiderivative Antialiasing for Arbitrary Waveform Generation,” TASLP 2022](https://doi.org/10.1109/TASLP.2022.3198007).
6. If (f_0(t)), (m(t)), or pulse width changes without a known bandwidth limit, the signal need not have a fixed harmonic support. FM can generate infinitely many sidebands in the mathematical model. “Keep harmonics below Nyquist” is exact only for stationary periodic synthesis or for a separately specified time-varying bandwidth model.

The honest engineering target is therefore: define the input signal class, define a reference filter and latency, and bound alias energy, wanted-band error, timing error, and worst-case CPU for that class.

## Method families

### Direct harmonic/additive synthesis and analytic coefficients

For a static periodic shape, direct harmonic synthesis is the strongest exact method in the literature surveyed. Compute or derive (c_k(m)), omit harmonics above the chosen band, and evaluate the retained complex sinusoids. A piecewise-polynomial curve is especially attractive because its Fourier coefficients have analytic endpoint forms. This avoids a fixed wavetable entirely and gives an excellent oracle for testing other oscillators.

The cost is (O(K)) per sample, plus coefficient evaluation when (m) changes. It is pitch-dependent: low notes have many retained harmonics; high notes have few. The work is SIMD-friendly if partials are batched, but phase recurrence, sine/cosine evaluation, coefficient updates, and large partial counts complicate the real-time path. A dynamically changing harmonic cutoff also needs amplitude crossfades or another continuity rule to avoid pops during pitch glides. Stilson and Smith specifically discuss this problem in their DSF/additive treatment and recommend fading changing harmonics rather than abruptly changing the count; see the [official ICMC paper](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext).

The method is exact only with respect to a defined reference. A sawtooth’s infinite series is not the requested raw discontinuity after truncation; it is the bandlimited version. A shape morph is exact if the desired morph is defined by the coefficient morph (c_k(m)), not merely by sample-domain interpolation between two approximations. If coefficients are derived analytically for every (m), this is a strong solution. If they are numerically integrated at audio rate, it becomes a different CPU and accuracy problem.

The contemporary hardware example [“An Aliasing-Free Hybrid Digital-Analog Polyphonic Synthesizer,” Roth et al., DAFx 2023](https://www.dafx.de/paper-archive/2023/DAFx23_paper_36.pdf) demonstrates the direct-Fourier idea at a much larger scale: a configurable “big Fourier oscillator” with up to 1024 partials. Its results are evidence for the quality of the method, not evidence that a 1024-partial software oscillator is the right KURV implementation; its cost remains tied to the number of partials.

### BLIT, integrated BLIT, and minBLEP

The bandlimited impulse train (BLIT) starts with an impulse train whose impulses are represented by bandlimited sinc pulses. Integrating it produces a saw; appropriate alternating or phase-shifted integration produces square and triangle families. This is conceptually close to exact additive synthesis and makes event timing explicit. Stilson and Smith’s original [ICMC paper](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext) is the primary source for the classical BLIT/DSF construction and its practical limitations.

An infinite sinc is not a real-time local kernel. Windowing/truncating it creates Gibbs and passband errors; storing one period as a table reintroduces table resolution and interpolation choices. A minBLEP stores a short, integrated bandlimited step correction around each naive discontinuity. It has finite work around events, but normally requires a table or an equivalent precomputed kernel, careful fractional placement, and state to accumulate the correction. Brandt’s [“Hard Sync Without Aliasing”](https://www.cs.cmu.edu/~eli/L/icmc01/hardsync.html) is especially important because it tests the technique on hard sync rather than only a stationary saw:

- the correction is placed at the actual fractional reset time;
- a windowed-sinc implementation needs lookahead when it is to be symmetric;
- a predictor based on constant frequency becomes unreliable when frequency varies inside the correction window;
- minBLEP reduces lookahead and integration work but does not remove event-rate-dependent cost.

BLIT/minBLEP is excellent for discontinuity-defined waveforms and sync/PWM events when a small correction table and bounded event queue are acceptable. It is not a general exact arbitrary-curve solution. For KURV, the strongest parts to retain are event-based correction and fractional timing, not the assumption that every curve can be reduced to one precomputed BLEP.

### Polynomial BLEP and higher-order integrated polynomial interpolation

PolyBLEP replaces a discontinuity’s ideal sinc-integral correction with a compact polynomial. It is table-free, cheap, local, branch/mask friendly, and easy to add to a naive saw or pulse. The 2007 review by Välimäki and Huovilainen, [“Antialiasing oscillators in subtractive synthesis,” IEEE Signal Processing Magazine](https://doi.org/10.1109/MSP.2007.323276), places PolyBLEP among the important quasi-bandlimited oscillator families.

The more detailed primary evaluation is [“Perceptually informed synthesis of bandlimited classical waveforms using integrated polynomial interpolation,” Välimäki, Pekonen, and Nam, JASA 2012](https://doi.org/10.1121/1.3651227), with the authors’ [full PDF](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf). It establishes the useful hierarchy:

- ideal BLEP is the integral of sinc and has infinite support;
- a compact polynomial or B-spline interpolation of the BLEP provides a no-table approximation;
- increasing polynomial order improves alias rejection but also changes wanted-band magnitude and increases arithmetic;
- fractional discontinuity placement is part of the formula, not an optional refinement;
- a passband correction can restore some wanted harmonic amplitude, but this is another approximation/filter-design choice.

The cited experiments found that third-order B-spline correction was a good tradeoff and that sufficiently high-order polynomial methods could be perceptually clean over the tested range. The paper also shows that very short tabled BLEPs require substantial oversampling of the table to compete with the polynomial versions. That matters for KURV: the current local two-sample quadratic PolyBLEP is a sensible low-cost baseline, but it is not an exact BLEP, and its passband/timing errors are measurable rather than zero.

The terminology “BLEP fixes any sharp waveform” is incomplete. A saw and a pulse have value discontinuities, so a BLEP correction targets the step. A triangle has continuous value but a slope discontinuity at its corner. Its correct event correction is a bandlimited ramp (BLAMP), not a BLEP.

### BLAMP, polyBLAMP, and higher derivative events

[“Rounding Corners with BLAMP,” Esqueda, Välimäki, and Bilbao, DAFx 2016](https://dafx.de/paper-archive/2016/dafxpapers/18-DAFx-16_paper_33-PN.pdf), derives the ideal BLAMP and a four-point cubic B-spline polyBLAMP. The method uses the local derivative jump and the fractional position of the corner. It reports substantial alias reduction and improved SNR in the tested triangle and related cases, with lower computational cost than the tested 2×/4× oversampling choices.

The core generalization is simple:

- discontinuity in value → BLEP;
- discontinuity in first derivative → BLAMP;
- discontinuity in the (r)-th derivative → an (r)-times integrated bandlimited correction, approximated by a higher-order polynomial/B-spline kernel.

The later primary work [“Eliminating aliasing caused by discontinuities using integrals of the sinc function,” ISMRA 2016](https://www.ness.music.ed.ac.uk/wp-content/uploads/2016/12/ISMRA2016-48-1.pdf) shows why one correction order is not universally enough: a signal can contain more than one discontinuity derivative, and the required integrals depend on which derivative jumps are present.

For an arbitrary piecewise-polynomial curve, this suggests a useful representation: store the polynomial segments and emit the event location plus every derivative jump at each segment boundary. The oscillator then applies the corresponding finite correction basis. This remains no-table and local, but the quality depends on the polynomial order, correction support, and how accurately the fractional event is located. Smooth segments with no derivative discontinuity still have ordinary bandwidth; they do not become magically bandlimited merely because they are polynomial.

The same mathematics applies to nonlinear processing but should not be confused with oscillator generation. [“Antialiased soft clipping using a polynomial approximation of the integrated bandlimited ramp function,” Esqueda, Välimäki, and Bilbao, ICA 2016](https://www.research.ed.ac.uk/files/25687826/ica_2016_integrated_BLAMP.pdf) applies integrated BLAMP ideas to soft clipping’s derivative discontinuities. That is evidence that the event correction framework generalizes to nonlinearities; it does not mean that a post-oscillator soft clip is fixed by improving the oscillator’s BLEP.

### DPW and differentiated/integrated polynomial waveforms

Discrete polynomial waveform (DPW) methods generate a smooth polynomial waveform, differentiate it to obtain the desired discontinuity order, then apply frequency-dependent scaling or filtering. The sawtooth result was introduced in [“Discrete-Time Synthesis of the Sawtooth Waveform with Reduced Aliasing,” Vesa Välimäki, IEEE Signal Processing Letters, 2005](https://research.aalto.fi/en/publications/discrete-time-synthesis-of-the-sawtooth-waveform-with-reduced-ali/) with the [author/university-linked IEEE paper](http://ieeexplore.ieee.org/iel5/97/30366/01395943.pdf?isNumber=30366&arnumber=1395943&prod=JNL&arSt=+214&ared=+217&arAuthor=+Valimaki%2C+V).

The higher-order treatment is [“Alias-Suppressed Oscillators Based on Differentiated Polynomial Waveforms,” Välimäki, Nam, Smith, and Abel, IEEE TASLP 2010](https://doi.org/10.1109/TASL.2009.2026507), [full author PDF](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf). Its useful properties are:

- constant arithmetic work per output sample for a given order;
- no wavetable and no per-event queue for ordinary saw/triangle-like periodic waveforms;
- higher differentiation/integration order gives a steeper spectral rolloff and lower aliasing;
- a crossover or multirate design can use cheaper low order where it is sufficient.

Its limits are equally important. The scaling is frequency-dependent, low-frequency numerical behavior requires care, and the method is naturally tied to polynomial waveforms and their derivative structure. It does not directly solve arbitrary smooth curve morphing, arbitrary hard sync, or arbitrary modulation of the curve. The sound is quasi-bandlimited, with wanted-band amplitude and phase behavior determined by the polynomial and any correction filter.

DPW is attractive for a scalar/SIMD oscillator because it has predictable work and few branches. It is a good candidate for a saw/triangle specialized path, but it is not a complete replacement for event-aware correction when KURV needs pulse width, fractional event location, or arbitrary segment shapes.

### PTR and EPTR

Polynomial Transition Regions (PTR) replace a finite region around a discontinuity with a smooth polynomial transition. The method is described and measured in [“Reducing Aliasing from Synthetic Audio Signals Using Polynomial Transition Regions,” Kleimola and Välimäki, IEEE Signal Processing Letters, 2012](https://doi.org/10.1109/LSP.2011.2177819), [primary full reprint](https://aaltodoc.aalto.fi/bitstreams/0b7c5649-cd2b-476f-ba53-4e33a8103f06/download). The paper reports lower operation counts than DPW in the musical range and a transient-free construction.

EPTR, [“Improved Polynomial Transition Regions Algorithm for Alias-Suppressed Signal Synthesis,” Ambrits and Bank, SMC 2013](https://home.mit.bme.hu/~bank/publist/smc13.pdf), removes a phase offset in PTR, reduces the work around a saw transition, and extends the idea to asymmetric triangles/continuously variable symmetry. It is particularly relevant to a morphing shape system because it demonstrates a procedural continuum beyond a single fixed saw or symmetric triangle.

PTR/EPTR are still quasi-bandlimited. Their transition region is a deliberate time-domain deformation whose spectrum is smoother than the raw discontinuity. Their quality depends on order and transition width; their event timing is excellent when the transition is anchored at the exact fractional phase, but the transition itself is not the ideal sinc-smoothed event. They are cheap and SIMD-friendly, but their natural domain is piecewise-linear/classical curves, not an arbitrary analytic curve with an arbitrary spectrum.

### Fractional-delay BLIT/FDF and bandlimited interpolation

Nam, Välimäki, Abel, and Smith’s [“Efficient Antialiasing Oscillator Algorithms Using Low-Order Fractional Delay Filters,” IEEE TASLP 2010](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf) represents each bandlimited impulse with a fractional-delay filter impulse response. A third-order B-spline was chosen as a practical compromise. Integration yields saw, square, and triangle waveforms; the same mechanism is shown for PWM, hard sync, and supersaw-like constructions.

This family is valuable when event timing is more important than exact passband identity. Fractional delay tracks a discontinuity between samples instead of snapping it to the nearest sample. But a low-order fractional-delay filter has a nonflat magnitude and nonzero phase error; the B-spline’s high-frequency attenuation is part of its alias-suppression mechanism. The resulting oscillator is quasi-bandlimited, not an exact sinc oscillator. Its ordinary cost is attractive, but event bookkeeping and event rate make PWM and sync costs frequency- and ratio-dependent.

The source’s broader lesson is that a “no fixed wavetable” design still needs a basis function. A polynomial spline basis can be generated procedurally, but its finite support and frequency response must be measured. The related primary work [“Optimized Polynomial Spline Basis Function Design for Quasi-Bandlimited Classical Waveform Synthesis,” Pekonen, Nam, Smith, and Välimäki, IEEE SPL 2012](https://koasas.kaist.ac.kr/handle/10203/201094) optimizes the basis coefficients for a stated bandwidth; it improves the tradeoff but does not remove the approximation bound.

### Antiderivative antialiasing for arbitrary curves

Antiderivative antialiasing (AA) is the most promising general method found for procedural arbitrary waveform shapes. The construction treats the input signal as a continuously interpolated curve, applies the nonlinear operation in continuous time, and uses antiderivatives to implement a virtual downsampling filter. The FIR version is described in the 2021 paper above; the IIR generalization is formalized in [“Antiderivative Antialiasing for Arbitrary Waveform Generation,” Gabrielli, D’Angelo, La Pastina, and Squartini, IEEE/ACM TASLP 2022](https://doi.org/10.1109/TASLP.2022.3198007).

The author’s official page provides the paper and [official MATLAB code](https://dangelo.audio/taslp-antialias-waveform) (direct code archive: [aaiir-osc.zip](https://dangelo.audio/assets/code/aaiir-osc.zip)). The method’s appeal for KURV is structural:

- it does not require a fixed 2048-sample table;
- it accepts arbitrary periodic or nonperiodic piecewise-linear waveforms;
- a higher-order IIR filter improves rejection without a long FIR lookahead;
- the antiderivative approach avoids deriving a special BLEP formula for every new curve/nonlinearity.

The method’s limits must remain explicit. Linear interpolation of the input curve is itself a source of spurious components and imposes an SNR ceiling. A causal IIR has phase distortion and group delay. Higher order costs more multiplications and state, and the chosen Butterworth/Chebyshev/elliptic filter controls the magnitude/phase/alias tradeoff. The 2021 paper’s measured comparison also shows that high-order oversampling can remain a strong competitor; the IIR method is a configurable approximation, not an exact ideal low-pass.

For KURV, AA-IIR becomes compelling if “arbitrary curve” means user-defined piecewise-linear or spline segments that may contain corners and nonlinear operations, not merely the current four classical shapes. It is less compelling if the product only needs Sine/Triangle/Saw/Pulse: those have better specialized event corrections with less state and less phase distortion.

### Oversampling and sample-rate conversion

Oversampling is the most universal fallback. Generate at (M F_s), then low-pass and decimate. It gives every method—naive procedural curves, nonlinear processing, event correction, and arbitrary user functions—more transition bandwidth. It does not prove the generated signal is bandlimited at the internal rate, and a finite decimator cannot reproduce the ideal sinc exactly.

The primary sample-rate-conversion literature makes the same point. Smith and Phil Gossett’s original [“A Flexible Sample Rate Conversion Method,” ICASSP 1984](https://doi.org/10.1109/ICASSP.1984.1172555) establishes the windowed-sinc approach; Dave Rossum’s primary follow-up, [“Some Aspects of Sample Rate Conversion,” ICMC 1985](https://quod.lib.umich.edu/i/icmc/bbp2372.1985.019/--some-aspects-of-sample-rate-conversion?rgn=main;view=fulltext), explains bandlimited interpolation as the mathematical basis of high-quality conversion and gives the cost of high-quality finite interpolation. Simple linear or zero-order interpolation is not a sufficient arbitrary-waveform reconstruction filter.

Oversampling has constant multiplier cost (M) for a fixed internal algorithm, plus decimator cost proportional to the filter length. It is straightforward to SIMD across voices and is robust around nonlinearities. Its weaknesses are latency, CPU scaling, passband/stopband design, and residual aliasing. It also cannot repair aliasing already created by generating a discontinuity at the host rate and then applying a nonlinear operation after decimation.

### Time-varying frequency, FM, PWM, and hard sync

#### Frequency modulation and pitch movement

With FM, the instantaneous phase is not enough to define a stationary harmonic truncation. A periodic carrier under sinusoidal phase modulation has Bessel sidebands extending indefinitely in the mathematical model. Any finite implementation must choose a sideband budget or a time-domain bandwidth criterion.

The primary FM-specific reference [“A Modified FM Synthesis Approach to Bandlimited Signal Generation,” Timoney, Lazzarini, and Lysaght, DAFx 2008](https://mural.maynoothuniversity.ie/id/eprint/4137/2/VL-Modified-FM-Synthesis.pdf) derives a modified FM expression with controlled spectral rolloff and compares it with BLIT, BLEP, and DPW. It reports a direct tradeoff: reducing modulation at high pitch reduces aliasing but also changes the intended spectrum. This is a useful specialized analytic construction, not a universal arbitrary-curve oscillator.

DPW and PTR are easy to keep running under changing phase step, but their frequency-dependent scale/transition assumptions must be updated consistently. MinBLEP and BLIT-FDF need event timing and fractional delay to follow moving frequency; a correction window assuming constant frequency becomes inaccurate when the phase step changes significantly inside that window. AA-IIR naturally processes a changing input stream, but its interpolation and filter state still define the approximation and phase response.

#### PWM

PWM is two value discontinuities per period: one at the rising edge and one at the falling edge. A correct correction algorithm must place both at fractional phase, update the second event when duty changes, and handle narrow pulses without overlapping correction support in an uncontrolled way. BLIT-FDF explicitly treats PWM; the 2012 integrated-polynomial BLEP paper gives the fractional edge construction for a pulse. A pulse-width control changing at audio rate is itself modulation and must be bandwidth-limited or oversampled if the sidebands are part of the intended sound.

KURV’s current pulse path clamps the width away from the phase-step-sized extremes, emits two shifted local PolyBLEP terms, and therefore has the right event topology. It remains a compact approximation; its correction support, edge response, and behavior when the two events approach each other should be tested rather than assumed.

#### Hard sync

Hard sync is the stress test for event-aware oscillators. A slave phase reset can create an arbitrary discontinuity in the slave waveform; for a sine reset, the value and derivative behavior depend on the reset phase, and the complete reset waveform is not covered by a single saw BLEP. Brandt’s original paper is the required baseline: [“Hard Sync Without Aliasing,” ICMC 2001](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf).

For sine hard sync specifically, [“A General Antialiasing Method for Sine Hard Sync,” La Pastina and D’Angelo, DAFx 2022](https://www.dafx.de/paper-archive/2022/papers/DAFx20in22_paper_3.pdf) uses a residual FIR low-pass/convolution construction and compares polynomial, B-spline, trigonometric, and frequency-shifted alternatives. The ideal residual uses an infinite sinc; finite kernels give a controllable cost/alias tradeoff. This is a specialized hard-sync solution, not a reason to treat every arbitrary curve as a table.

An oscillator advertised as “perfect” must state whether hard sync is in scope. If it is, the event representation must include reset phase, fractional timing, and the local derivative data of the pre- and post-reset curves. Otherwise the claim should be limited to stationary waveforms/PWM.

## Comparison matrix

The ratings below separate mathematical status from engineering usefulness. “Exact” means exact relative to a defined bandlimited reference, not exact reproduction of an ideal discontinuous analog curve.

| Method | Alias rejection and wanted-band error | Event timing | Modulation / sync behavior | CPU and RT shape |
|---|---|---|---|---|
| Direct Fourier with analytic coefficients | Exact for a static, known periodic shape after harmonic truncation; in-band amplitude/phase are exact up to numerical error | Exact phase for retained harmonics; events are represented by the filtered series, not a raw edge | Strong for static morph if coefficients are known; expensive coefficient updates and sideband budgeting for fast morph/FM; hard sync needs a new spectrum/event treatment | (O(F_s/f_0)); pitch-dependent, SIMD-batchable, no allocations if partial storage is bounded; low notes are the worst case |
| Ideal BLIT / infinite sinc | Exact in the ideal mathematical construction | Exact fractional event in the continuous reference | Good conceptually; infinite support/lookahead and changing-frequency problems | Not finite real-time without approximation |
| Windowed BLIT / minBLEP | Strong if the window/support is long; finite support creates residual alias and passband error | Excellent when fractional event position is tracked; symmetric kernels introduce lookahead | Good for PWM and sync with event queues; cost rises with event rate and time-varying phase | Event work plus kernel state/table; bounded only if event count/support are bounded |
| Quadratic / higher PolyBLEP | Cheap quasi-bandlimited step correction; higher order reduces alias but can attenuate/warp wanted highs | Good only if the discontinuity fraction is included in the correction; nearest-sample placement is wrong | Good for saw/pulse; not sufficient for triangle corners or general resets | (O(1)) per sample per waveform; very SIMD-friendly, table-free, RT-safe |
| PolyBLAMP / integrated polynomial corrections | Correct derivative order for corners; higher order gives more rejection at extra arithmetic; finite support remains approximate | Good fractional corner timing when the local derivative jump is known | Good for triangle and piecewise-polynomial corners; arbitrary hard reset needs all relevant derivative jumps | Local constant work per event/sample; SIMD masks possible; no table required |
| DPW / higher-order DPW | Low alias for classical polynomial waveforms; frequency-dependent scale and numerical low-frequency limits; not exact | Phase is continuous, but raw event location is represented through a smooth polynomial construction | Easy ordinary pitch movement; less general for PWM, sync, or arbitrary morph | Constant per sample for fixed order; excellent SIMD/RT profile |
| PTR / EPTR | Smooth finite transition; reported efficient and clean for classical/asymmetric shapes; not ideal bandlimiting | Excellent if transition is anchored to fractional phase | Good for saw/triangle families; limited generality outside transition-defined shapes | Constant small arithmetic; SIMD-friendly; no table |
| BLIT-FDF / spline fractional delay | Good quasi-bandlimited behavior; interpolator magnitude/phase error is part of sound | Very good; fractional impulse placement is the core feature | Explicit PWM, sync, and supersaw constructions; event rate and changing phase matter | Small per-sample arithmetic plus event work; stateful but RT-safe |
| AA-FIR / AA-IIR | General arbitrary-waveform approximation; filter order controls stopband, passband, and SNR; IIR adds phase distortion, FIR adds latency | Filtered output has defined delay/ringing; not an exact raw event | Best general procedural path for changing piecewise curves; still bounded by interpolation and control bandwidth; special hard sync may need residual treatment | (O(P)) per sample for order (P); constant for fixed order, stateful, SIMD-able across voices; no fixed table |
| Oversampling + finite decimator | Raises alias boundary; residual alias and passband error are filter-dependent; not exact | Internal event can be sub-sample relative to host, but decimation filters it with finite latency | Broad fallback for FM, nonlinear processing, and unknown curves; CPU/latency multiply with factor | (O(M)) source work plus filter taps; predictable but expensive; current KURV allocates fixed arrays |

### Relative ranking by requirement

- **Mathematical exactness for static periodic shapes:** direct Fourier with exact coefficients.
- **Lowest bounded CPU for classical waves:** DPW/PTR/EPTR or low-order PolyBLEP, with the expected quasi-bandlimited error.
- **Best event fidelity without a fixed table:** higher-order fractional PolyBLEP/PolyBLAMP and a procedural event description.
- **Broadest arbitrary procedural curve coverage:** AA-IIR/AA-FIR, provided its filter and interpolation error are specified.
- **Best general-purpose safety net:** oversampling/resampling.
- **Most demanding special case:** hard sync with changing ratio/frequency; use a dedicated residual/event algorithm or explicitly limit the guarantee.

## Read-only mapping to the current KURV checkout

This section intentionally maps the research to the live source only. No source or test files were modified.

### Current oscillator path

- [`src/oscillator.rs`](../../src/oscillator.rs#L76-L92) advances each oscillator phase and, for a non-endpoint shape, samples both adjacent waveform families and linearly blends their output samples.
- [`shape_segment`](../../src/oscillator.rs#L230-L248) defines Sine → Triangle → Saw → Pulse, with the final Pulse endpoint.
- [`sample_waveform_normalized`](../../src/oscillator.rs#L254-L267) uses analytic sine and triangle expressions, while Saw and Pulse use local corrections.
- [`bandlimited_saw` and `bandlimited_pulse`](../../src/oscillator.rs#L325-L376) apply a naive ramp/pulse plus local PolyBLEP terms. The pulse uses two edges, including a shifted falling edge and a minimum width tied to phase step.
- [`poly_blep`](../../src/oscillator.rs#L378-L393) is a two-sample quadratic correction. It is a useful low-cost approximation, not an ideal BLEP.
- The triangle path currently has the raw absolute-value corner and no BLAMP correction. Its value is continuous, but its derivative jumps at the corners; that is precisely the case covered by polyBLAMP.
- [`VoiceBank::render`](../../src/voice.rs#L748-L813) uses 8-lane and 4-lane SIMD paths, then a scalar tail. Endpoint waveforms have specialized calls; intermediate shape values use the sample-domain blend.

This means KURV currently has three different aliasing contracts hidden behind one morph control: exact analytic sine, uncorrected triangle corners, and locally corrected saw/pulse edges. The morph is a blend of generated endpoint samples. For a static shape parameter, a linear blend of two bandlimited signals remains bandlimited, but here the endpoints are not equally bandlimited and the triangle is not corrected. For a moving shape parameter, the blend is also a modulation signal; its sidebands are not covered by the endpoint corrections alone.

### Current oversampling path

- [`StereoOversampler`](../../src/oversampling.rs#L7-L117) supports factors 1 through 4 with fixed arrays, a direct-delay factor-1 path, and a crossfade when the factor changes.
- The decimator uses 97 taps at 2×, 145 taps at 3×, and 193 taps at 4×. Its linear-phase equiripple kernels target a 20.5 kHz passband and 24 kHz stopband; explicit delay compensation keeps the existing 33-sample host contract.
- The process path renders the synth once per internal sample and pushes those results through the selected decimator ([`process`](../../src/lib.rs#L651-L716)). KURV declares 33 samples of latency ([`latency`](../../src/lib.rs#L732-L734)).
- The shape and pulse-width settings used for the inner oversampled renders are constructed once for the host sample before the internal loop ([`process`](../../src/lib.rs#L669-L695)). Therefore the current x2–x4 path raises the oscillator’s internal rate, but it does not by itself create a higher-rate interpolation trajectory for a control that changes between host samples.

The current architecture is already RT-conscious: fixed storage, bounded factor selection, SIMD voice accumulation, and no need for a dynamically sized correction table. The main research implication is not “replace everything with a table.” It is “make the curve/event model explicit, then select the smallest correction order that meets a measured error budget.”

## Recommendation for KURV

### 1. First choice: event-aware procedural oscillator for the existing family

Implement the following conceptual contract around the current phase oscillator:

- Sine stays analytic.
- Saw and Pulse use a higher-order polynomial BLEP with fractional discontinuity placement, preferably the compact B-spline/Lagrange family evaluated in the 2012 integrated-polynomial paper rather than a longer lookup table.
- Triangle uses a fractional polyBLAMP correction at each slope corner.
- Any future curve represented as piecewise polynomial emits segment-boundary locations and derivative jumps; apply BLEP/BLAMP/integrated corrections by derivative order.
- Define the morph as either (a) a linear combination of the endpoint bandlimited references, or (b) a single morphable curve with its own event derivatives. Do not leave this semantic distinction implicit.

This is the strongest KURV-specific balance: table-free, local, predictable, SIMD-compatible, and able to correct the actual failure in the live path—the triangle corners—without turning every voice into an additive synthesizer. It is not exact ideal bandlimiting, so call it “alias-suppressed” unless the acceptance test demonstrates a stated bound.

The implementation should preserve fractional event phase. For a discontinuity between samples, the correction coordinate must be based on the phase at the event, not simply on the current sample’s wrapped phase. For Pulse, the rising and falling event coordinates must be independently derived from the current width. For a shape transition that changes derivative order, the event data must change continuously or be crossfaded over a defined control bandwidth.

### 2. Second choice: procedural AA-IIR for genuinely arbitrary curves

If KURV’s intended future is a user-defined curve/spline system rather than the four current families, use the AA-IIR literature as the general path. Represent the curve procedurally or as a bounded segment list, generate its continuous interpolation and required antiderivative, and run a fixed-order virtual downsampling filter per voice. This avoids a fixed wavetable and handles curves for which enumerating every derivative event is awkward.

Start with a deliberately modest fixed order and measure it. A causal filter introduces group delay and changes phase; a higher order consumes more CPU. Make the filter choice part of the oscillator’s documented sound contract, not a hidden “perfect” switch. If the output must have linear phase, use a finite FIR with explicit latency instead of pretending the IIR phase is transparent.

### 3. Third choice: direct Fourier as an oracle and optional quality mode

Build or retain a separate direct-Fourier implementation for static shapes and tests. For piecewise-polynomial KURV curves, analytic coefficient generation provides the strongest reference and can become a low-pitch or offline/high-quality mode. It should not be the sole ordinary real-time implementation until worst-case low-pitch/polyphony cost is measured.

Its greatest value is experimental: it can establish what the bandlimited target should be for a static morph, allowing PolyBLEP/BLAMP/AA-IIR/oversampling to be compared against a truth model rather than against one another.

### 4. Fourth choice: retain x1–x4 oversampling as fallback/reference

Keep the existing oversampling path as a quality fallback for arbitrary future processing and as a cross-check. Do not use x4 as evidence that the oscillator is perfectly bandlimited. The finite equiripple decimator and fixed latency remain explicit, measurable approximations. A future quality mode can choose a larger factor or different filter, but the success claim must still report residual alias, passband error, phase, latency, and CPU.

### What not to choose as the universal core

- Do not make a fixed minBLEP table the universal representation; it adds table-resolution/fractional-index decisions and does not generalize cleanly to arbitrary curve derivative structure.
- Do not make DPW the universal arbitrary-shape solution; it is excellent for classical polynomial waveforms but naturally tied to their derivative/scaling structure.
- Do not rely on post-decimation low-pass filtering to remove aliases created by nonlinear processing after the decimator.
- Do not claim “perfect” for a moving morph/FM signal without specifying the modulation bandwidth and reference filter.

## Experimentally falsifiable success definition

The following is a proposed acceptance test, not a claim that the current implementation passes it.

### Reference

For each static shape and static morph value, create a reference in one of two ways:

1. exact analytic/piecewise-polynomial Fourier coefficients, retaining only harmonics below a declared (B); or
2. generate the continuous procedural curve at at least 32× the host rate, apply a verified high-order zero-phase low-pass with cutoff (B), and decimate using a filter whose residual is at least 20 dB below the product threshold.

For FM, PWM, hard sync, and moving morph, define the modulation trajectories explicitly and create the reference at 32× or higher. Do not compare against a host-rate naive oscillator.

### Signals

At 44.1, 48, and 96 kHz, test:

- Sine, Triangle, Saw, Pulse, and every 0.05 increment of the Sine–Triangle–Saw–Pulse morph;
- fundamental frequencies from 20 Hz through (0.45F_s), with special points at 20 Hz, 440 Hz, 4 kHz, 8 kHz, 12 kHz, and the Nyquist approach;
- Pulse widths 0.03, 0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95, and 0.97;
- randomized starting phases and fractional event phases;
- linear/exponential pitch glides, audio-rate frequency modulation, audio-rate shape modulation, PWM sweeps, and hard-sync ratios where supported;
- single voice and maximum realistic polyphony/unison, including SIMD lane tails.

### Metrics and pass/fail bounds

For the static oscillator path, call the implementation successful if all of the following hold against the reference:

- no discrete non-reference spur exceeds **−100 dBc** for (f_0\leq 8) kHz, and integrated non-reference alias energy is below **−100 dBFS** in a full-band FFT;
- wanted-band harmonic magnitude error is below **0.25 dB** and phase error below **1 degree** through 15 kHz, excluding bins deliberately removed by the reference cutoff;
- the measured zero crossing, pulse edge, or triangle corner is within **0.10 host sample** of the reference event after accounting for the documented latency;
- the result is numerically stable at 20 Hz and near Nyquist, with no DC drift or denormal-dependent behavior.

For moving controls and hard sync, use a separate, honest bound:

- no non-reference sideband/spur exceeds **−90 dBc** for the declared modulation trajectories;
- event timing remains within **0.20 host sample**;
- the report for the mode states its maximum control bandwidth, maximum sync ratio, and whether its phase response is linear, minimum-phase, or uncompensated.

For real-time behavior:

- no allocation, lock, I/O, unbounded loop, or data-dependent correction queue growth in the audio callback;
- worst-case CPU must be measured at maximum supported voice/unison count, not just average CPU on a single oscillator;
- the event-aware path should remain (O(1)) per active voice/sample for the current four waveforms;
- the AA-IIR path should have a fixed documented filter order and fixed state footprint;
- the direct-Fourier path should report its maximum retained partial count and its low-pitch worst case;
- changing quality/oversampling mode must preserve the documented latency and transition behavior.

These thresholds are intentionally falsifiable. If a method misses them, the useful conclusion is not that the method is “bad”; it identifies whether the failure is alias rejection, passband error, event timing, modulation, or CPU. A method should be called perfect only when it meets the exact-reference definition for a narrowly stated signal class. For the broader KURV oscillator, “success” should mean that every supported mode publishes its approximation bounds and passes the measured thresholds above.

## Primary-source bibliography

The links below are the original papers or official author/university/repository copies used for the report.

1. Claude E. Shannon, “Communication in the Presence of Noise,” *Proceedings of the IRE*, 1949. [DOI](https://doi.org/10.1109/jrproc.1949.232969).
2. Timothy Stilson and Julius O. Smith, “Alias-Free Digital Synthesis of Classic Analog Waveforms,” *ICMC*, 1996. [University of Michigan primary repository](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext).
3. Eli Brandt, “Hard Sync Without Aliasing,” *ICMC*, 2001. [Author page](https://www.cs.cmu.edu/~eli/L/icmc01/hardsync.html), [author PDF](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf).
4. Vesa Välimäki and Antti Huovilainen, “Antialiasing Oscillators in Subtractive Synthesis,” *IEEE Signal Processing Magazine*, 2007. [DOI](https://doi.org/10.1109/MSP.2007.323276), [Aalto metadata](https://research.aalto.fi/en/publications/antialiasing-oscillators-in-subtractive-synthesis/).
5. Vesa Välimäki, “Discrete-Time Synthesis of the Sawtooth Waveform with Reduced Aliasing,” *IEEE Signal Processing Letters*, 2005. [Aalto metadata and paper link](https://research.aalto.fi/en/publications/discrete-time-synthesis-of-the-sawtooth-waveform-with-reduced-ali/).
6. Vesa Välimäki, Juhan Nam, Julius O. Smith, and Jonathan Abel, “Alias-Suppressed Oscillators Based on Differentiated Polynomial Waveforms,” *IEEE TASLP*, 2010. [DOI](https://doi.org/10.1109/TASL.2009.2026507), [author PDF](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf).
7. Jussi Pekonen, Juhan Nam, Julius O. Smith, and Vesa Välimäki, “Optimized Polynomial Spline Basis Function Design for Quasi-Bandlimited Classical Waveform Synthesis,” *IEEE Signal Processing Letters*, 2012. [KAIST primary record](https://koasas.kaist.ac.kr/handle/10203/201094).
8. Vesa Välimäki, Jussi Pekonen, and Juhan Nam, “Perceptually Informed Synthesis of Bandlimited Classical Waveforms Using Integrated Polynomial Interpolation,” *Journal of the Acoustical Society of America*, 2012. [DOI](https://doi.org/10.1121/1.3651227), [author PDF](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf).
9. Jussi Kleimola and Vesa Välimäki, “Reducing Aliasing from Synthetic Audio Signals Using Polynomial Transition Regions,” *IEEE Signal Processing Letters*, 2012. [DOI](https://doi.org/10.1109/LSP.2011.2177819), [Aalto reprint](https://aaltodoc.aalto.fi/bitstreams/0b7c5649-cd2b-476f-ba53-4e33a8103f06/download).
10. Dániel Ambrits and Balázs Bank, “Improved Polynomial Transition Regions Algorithm for Alias-Suppressed Signal Synthesis,” *SMC*, 2013. [BME author/university PDF](https://home.mit.bme.hu/~bank/publist/smc13.pdf).
11. Juhan Nam, Vesa Välimäki, Jonathan Abel, and Julius O. Smith, “Efficient Antialiasing Oscillator Algorithms Using Low-Order Fractional Delay Filters,” *IEEE TASLP*, 2010. [KAIST author PDF](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf).
12. Vesa Välimäki, Jussi Pekonen, and Juhan Nam, “Variable Fractional Delay Filters in Bandlimited Oscillator Algorithms for Music Synthesis,” *ICGCS*, 2010. [Author PDF](https://mac.kaist.ac.kr/pubs/Pekonen2010ICGCS.pdf), [DOI](https://doi.org/10.1109/ICGCS.2010.5543077).
13. Esqueda, Välimäki, and Bilbao, “Rounding Corners with BLAMP,” *DAFx*, 2016. [DAFx primary PDF](https://dafx.de/paper-archive/2016/dafxpapers/18-DAFx-16_paper_33-PN.pdf).
14. Esqueda, Välimäki, and Bilbao, “Antialiased Soft Clipping Using a Polynomial Approximation of the Integrated Bandlimited Ramp Function,” *ICA*, 2016. [Author/university PDF](https://www.research.ed.ac.uk/files/25687826/ica_2016_integrated_BLAMP.pdf).
15. Esqueda, Välimäki, and Bilbao, “Eliminating Aliasing Caused by Discontinuities Using Integrals of the Sinc Function,” *ISMRA*, 2016. [Primary PDF](https://www.ness.music.ed.ac.uk/wp-content/uploads/2016/12/ISMRA2016-48-1.pdf).
16. Juhan Nam, Vesa Välimäki, Jonathan Abel, and Julius O. Smith, “Alias-Free Virtual Analog Oscillators Using a Feedback Delay Loop,” *DAFx*, 2009. [Official DAFx archive](https://dafx.de/paper-archive/details/C4iObNyhou3Sz9mbNa3AkQ).
17. Timoney, Lazzarini, and Lysaght, “A Modified FM Synthesis Approach to Bandlimited Signal Generation,” *DAFx*, 2008. [Maynooth author repository PDF](https://mural.maynoothuniversity.ie/id/eprint/4137/2/VL-Modified-FM-Synthesis.pdf).
18. La Pastina and D’Angelo, “A General Antialiasing Method for Sine Hard Sync,” *DAFx20in22*, 2022. [Official DAFx PDF](https://www.dafx.de/paper-archive/2022/papers/DAFx20in22_paper_3.pdf), [author publications](https://dangelo.audio/publications).
19. La Pastina, D’Angelo, Gabrielli, and collaborators, “Arbitrary-Order IIR Antiderivative Antialiasing,” *DAFx20in21*, 2021. [Official DAFx PDF](https://www.dafx.de/paper-archive/2021/proceedings/papers/DAFx20in21_paper_27.pdf).
20. Gabrielli, D’Angelo, La Pastina, and Squartini, “Antiderivative Antialiasing for Arbitrary Waveform Generation,” *IEEE/ACM TASLP*, 2022. [DOI](https://doi.org/10.1109/TASLP.2022.3198007), [official author page and code](https://dangelo.audio/taslp-antialias-waveform).
21. Julius O. Smith III and Phil Gossett, “A Flexible Sample Rate Conversion Method,” *ICASSP*, 1984. [DOI](https://doi.org/10.1109/ICASSP.1984.1172555).
22. Dave Rossum, “Some Aspects of Sample Rate Conversion,” *ICMC*, 1985. [University of Michigan primary repository](https://quod.lib.umich.edu/i/icmc/bbp2372.1985.019/--some-aspects-of-sample-rate-conversion?rgn=main;view=fulltext).
23. Jonas Roth, Domenic Keller, Oscar Castañeda, and Christoph Studer, “An Aliasing-Free Hybrid Digital-Analog Polyphonic Synthesizer,” *DAFx*, 2023. [DAFx primary PDF](https://www.dafx.de/paper-archive/2023/DAFx23_paper_36.pdf), [arXiv primary preprint](https://arxiv.org/abs/2311.18774).
24. Kurt James Werner, Balázs Bank, and Julian D. Parker, “Network Variable Preserving Step-Size Control in Wave Digital Filters,” *DAFx*, 2017. [DAFx primary PDF](https://dafx.de/paper-archive/2017/papers/DAFx17_paper_74.pdf).
