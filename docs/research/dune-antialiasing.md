# DUNE oscillator quality without a visible oversampling control

Research report — 2026-08-06

## Executive answer

Synapse does not publish the antialiasing algorithm used by DUNE 3 or 3.5's VA oscillators. The
official material establishes that DUNE has VA, wavetable, and FM models; that VA offers saw, pulse,
triangle, and sync; that each of the first two oscillator blocks can contain up to 32 oscillators; and
that Swarm individually modulates the oscillators with a user Rate control. Synapse also documents
optimized SSE processing, multithreading, and selectable modulation evaluation rates. It does **not**
identify BLEP, BLAMP, DPW, BLIT, a mipmap, an oscillator oversampling factor, a Swarm PRNG, or a
Swarm interpolation/update law. [[DUNE 3.5 manual, pp. 7-8, 18-19, 25-30](https://www.synapse-audio.com/DUNE-35-Manual.pdf)]
[[DUNE 3 product page](https://www.synapse-audio.com/dune3.html)]

Therefore the defensible conclusion is not "DUNE avoids oversampling." It is:

1. DUNE exposes no oscillator-quality/oversampling control to the user.
2. The manual explicitly calls out 3x oversampling for most analog-modelled filters, but makes no
   corresponding disclosure for the VA oscillators. This makes blanket full-stack oscillator
   oversampling unconfirmed, not disproved. [[DUNE 3.5 manual, p. 40](https://www.synapse-audio.com/DUNE-35-Manual.pdf)]
3. DUNE's documented ability to make eight Swarm oscillators resemble 32 ordinary stack oscillators,
   together with SSE and control-rate modulation options, is consistent with a cheap vectorized
   oscillator kernel plus lower-rate per-lane pitch motion. That is an engineering inference, not a
   proprietary fact. [[DUNE 3.5 manual, pp. 7-8](https://www.synapse-audio.com/DUNE-35-Manual.pdf)]

For KURV, the best procedural path is a hybrid of **event-aware polynomial BLEP/BLAMP for the
canonical waves**, **analytic segment integration for the custom periodic cubic curve**, and a
**hidden 2x fallback only for the cases whose bandwidth cannot be bounded cheaply**. Mipmapped
wavetables, FFT generation, and additive/DSF replacement are excluded: they either violate the
product constraint or stop composing cleanly with KURV's phase warp and arbitrary curve.

## Evidence boundary: confirmed versus inferred

| Claim | Status | Evidence |
|---|---|---|
| DUNE 3 VA synthesizes saw, pulse, and triangle and supports oscillator sync | Confirmed | Official manual pp. 29-30 |
| Oscillator blocks 1 and 2 are stacks of up to 32 oscillators | Confirmed | Official manual pp. 25-28 |
| Swarm individually modulates every oscillator and exposes Rate | Confirmed | Official manual pp. 7-8 and 27-28 |
| Eight Swarm oscillators may have an effect similar to 32 ordinary stack oscillators | Confirmed, qualitative | Official manual p. 8 |
| DUNE uses optimized SSE, up to six cores, and selectable modulation rates including sample-by-sample Audio Rate | Confirmed | Official manual pp. 7, 18-19 |
| Most analog-modelled filters, except Alpha, use 3x oversampling | Confirmed for those filters only | Official manual p. 40 |
| DUNE's VA oscillator is BLEP, BLIT, DPW, wavetable-backed, or oversampled by a specific factor | Unknown | No first-party disclosure found |
| Swarm is a particular noise generator, sine LFO, update cadence, smoothing kernel, or correlation scheme | Unknown | First-party sources say only individual/subtle modulation and Rate |
| DUNE's efficiency comes from host-rate specialized oscillators and control-rate Swarm updates | Plausible inference | Fits the public CPU architecture and feature semantics, but many proprietary implementations fit the same observations |

The DUNE 3.5 release itself added browser, workflow, polyphony, compressor, and memory improvements;
its first-party release note does not claim a changed oscillator algorithm. [[Synapse DUNE 3.5 release
note](https://www.synapse-audio.com/news-dune35update.html)] An older DUNE 1.2 note says VCO
quality was improved, but gives no method, so it is historical evidence of ongoing engine work rather
than evidence for any particular DUNE 3 algorithm. [[Synapse DUNE 1.2 release
note](https://www.synapse-audio.com/news-dune-v12-released.html)]

The current 3.6 manual retains the same individual-modulation plus Rate description and likewise does
not disclose an oscillator antialiasing method. [[DUNE 3.6 manual, pp. 8 and
30](https://www.synapse-audio.com/DUNE-36-Manual.pdf)]

In a 2011 DSP-forum reply, Synapse founder Richard Hoffmann said that brute-force 8x oversampling of
saw/square solely for oscillator antialiasing is generally unnecessary when an oscillator is coded
properly, while noting that oversampling may still be useful for downstream clipping. This is useful
first-person design context, but it predates DUNE 3 and does not identify DUNE's oscillator kernel.
[[Richard Hoffmann on oscillator oversampling](https://www.kvraudio.com/forum/viewtopic.php?p=4768973)]

No relevant Synapse/DUNE oscillator patent or source publication was found in searches by product,
company, and named developers. That search result is not proof that no patent, trade secret, licensed
algorithm, or unpublished implementation exists.

A separate Korg patent—not a Synapse patent—claims specific phase- and magnitude-aware correction
pulses around oscillator discontinuities, including hard-sync embodiments. Google Patents currently
labels US7589272B2 active with an adjusted 2028-07-04 expiration. That does not mean every BLEP method
is covered, but it does mean a shipping implementation should have the actual claims reviewed rather
than treating the research literature as a freedom-to-operate opinion. [[Korg
US7589272B2](https://patents.google.com/patent/US7589272)]

## What the current KURV core actually has to solve

This report targets the KURV procedural generator architecture current on 2026-08-06.

- [`src/oscillator.rs`](../../src/oscillator.rs) already contains a two-point legacy PolyBLEP, four-point
  integrated cubic B-spline and Lagrange BLEP/BLAMP families, SIMD paths, phase warp, and a separate
  experimental spectral engine.
- The canonical morph is Sine -> Triangle -> Saw -> Pulse. Adjacent antialiased endpoint samples are
  blended, with a gain correction in the Triangle -> Saw interval
  ([`sample_shape_normalized`](../../src/oscillator.rs#L2178)).
- Saw and pulse use BLEP at value discontinuities; triangle uses BLAMP at its two slope
  discontinuities ([`bandlimited_triangle`](../../src/oscillator.rs#L2368),
  [`bandlimited_saw`](../../src/oscillator.rs#L3226),
  [`bandlimited_pulse`](../../src/oscillator.rs#L3347)).
- Phase warp changes both sampled phase and the local phase step using the derivative of the warp
  ([`warp_phase_scalar`](../../src/oscillator.rs#L1768)).
- The user curve is compiled to eight periodic cubic B-spline segments and evaluated directly in the
  realtime path ([`WaveCurveData::compile_rt`](../../src/wave_curve.rs#L64),
  [`WaveCurveRt::eval`](../../src/wave_curve.rs#L225)). Every nonzero custom contribution includes
  this directly evaluated curve; at full custom mix it completely bypasses the canonical BLEP/BLAMP
  path ([`generate_custom_step`](../../src/oscillator.rs#L145)).
- Swarm/Jitter creates independent deterministic per-lane pitch trajectories, recenters them, converts
  cents to phase-step ratios, and linearly slews toward bounded control-rate targets while pan and gain
  remain static ([`fill_unison_jitter_offsets_mode`](../../src/voice.rs#L720),
  [`prepare_swarm_jitter_target`](../../src/voice.rs#L3554)).
- KURV currently renders at a hidden 1x-4x factor and decimates with fixed-allocation FIR state
  ([`generator_configuration`](../../src/lib.rs#L1826),
  [`StereoOversampler`](../../src/oversampling.rs#L15)). The procedural recommendations below do not
  depend on the experimental spectral engine.

The important consequence is that there is no universal "oscillator antialiasing" switch. KURV has
four different bandwidth problems:

1. known value discontinuities in saw/pulse;
2. known derivative discontinuities in triangle and higher derivatives at cubic-curve knots;
3. time remapping from phase warp;
4. time variation from morph, PWM, and pitch jitter.

Each needs the smallest matching tool.

## Alternatives to brute-force oversampling

### Polynomial BLEP and BLAMP: best fit for canonical waves

BLEP adds a local residual around each known step discontinuity. PolyBLEP replaces a stored
windowed-sinc correction with a short closed-form polynomial. Välimäki, Pekonen, and Nam derived
integrated Lagrange and B-spline forms and reported the integrated third-order B-spline as the best
cost/quality tradeoff in their comparison, with perceptually alias-free saw synthesis up to 7.8 kHz
at 44.1 kHz under their masking model. The method needs neither runtime oversampling nor a correction
table. [[Välimäki, Pekonen & Nam, JASA 2012](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf)]

BLAMP is the corresponding correction for a discontinuity in the first derivative. Esqueda,
Välimäki, and Bilbao show that the fractional event delay and slope jump must be estimated; their
four-point polyBLAMP reduced aliasing for triangle/corner cases without oversampling and outperformed
2x/4x oversampling in the reported clipping/rectification comparisons. [[Esqueda, Välimäki & Bilbao,
"Rounding Corners with BLAMP," DAFx-16](https://www.dafx.de/paper-archive/details/xRM0GiuF-JmeN0_aaP9VNg)]

**Fit to KURV:** excellent for saw, pulse, triangle, and any morph expressed as known endpoint event
heights. It is branch-light, table-free, fixed-state, and already vectorized in KURV. The remaining
quality opportunity is accurate event timing under a nonlinear warp or changing phase step, not a new
antialiasing family.

**Limit:** a four-point correction is approximately bandlimited, not mathematically alias-free. It
also only corrects the discontinuity order it models. It cannot remove modulation sidebands created
by fast pitch, shape, or warp motion.

### minBLEP and longer BLEP residuals: higher rejection, event-driven cost

Brandt's minimum-phase BLEP replaces a discontinuity with a precomputed minimum-phase bandlimited
step. It eliminates lookahead and applies to saw, square, and hard-sync discontinuities; derivative
discontinuities require the corresponding ramp correction. Its average work scales with event rate
and residual length, while high oscillator frequencies make events and overlaps frequent.
[[Brandt, "Hard Sync Without Aliasing," ICMC 2001](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf)]

**Fit to KURV:** a strong quality ceiling for saw/pulse and future sync, especially at low/mid notes
where wrap events are sparse. A small per-lane residual accumulator is less SIMD-friendly and uses
more state/cache than the existing polynomials. Strong warp can move events, but minBLEP handles that
well if the true fractional crossing and step height are supplied.

**Limit:** it does not solve the arbitrary periodic curve by itself, and at high notes or very narrow
pulses residuals overlap until its cost advantage shrinks.

### Ideal closed-form BLEP/BLAMP: a reference, not a realtime kernel

The ideal bandlimited step is the integral of the ideal low-pass sinc and can be written with the sine
integral; the ideal BLAMP is its next integral. The same primary derivation explains why the sinc and
its correction have infinite support. Truncating/windowing yields a practical BLEP/minBLEP; replacing
it with compact polynomials yields PolyBLEP/PolyBLAMP. [[Välimäki, Pekonen & Nam, equations 5-11](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf)]

**Fit to KURV:** use a long offline sine-integral correction as an oracle for residual error. Direct
realtime evaluation still needs truncation/history, so "closed form" does not remove the support,
state, or latency problem.

### DPW / differentiated polynomial waves: cheap classical endpoints

DPW samples a smoother polynomial waveform and differentiates it one or more times to obtain the
target saw/triangle spectral slope with reduced aliases. The fourth-order method was perceptually
alias-free to about 4.6 kHz at 44.1 kHz in the paper's model; increasing polynomial order generally
cost less than doubling the rate in that comparison. The paper also recommends a lower-order method
at low frequencies because higher-order normalization becomes numerically awkward.
[[Välimäki, Nam, Smith & Abel, IEEE TASLP 2010](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf)]

**Fit to KURV:** worthwhile as a low-CPU comparator for unwarped, steady saw and triangle endpoints.
It uses regular arithmetic and little state.

**Limit:** the differentiator history and frequency-dependent scaling make abrupt frequency changes,
phase reset, strong jitter, and nonlinear phase warp harder than the existing event correction. It
does not naturally cover the continuous four-wave morph or arbitrary edited curve. Making a separate
DPW construction for every topology would grow more code than the shared BLEP/BLAMP seam.

### BLIT with low-order fractional-delay filters

The foundational BLIT construction uses sampled sinc pulses or a closed-form periodic sinc, then
integrates/differences the impulse train to obtain classical waveforms. It is bandlimited by choosing
the number of harmonics below Nyquist, with guard-band/window tradeoffs in practical forms.
[[Stilson & Smith, "Alias-Free Digital Synthesis of Classic Analog Waveforms," ICMC
1996](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext)]

BLIT generates a compact bandlimited impulse at each period and integrates it into saw, pulse, or
triangle. Nam et al. show low-order B-spline fractional-delay filters that avoid a lookup table and
runtime prototype oversampling; they also demonstrate PWM, hard sync, and a six-oscillator supersaw.
Their third-order B-spline BLIT was perceptually alias-free up to 5.8 kHz in the stated evaluation.
[[Nam, Välimäki, Abel & Smith, IEEE TASLP 2010](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf)]

**Fit to KURV:** credible for a fixed classical-wave oscillator or future hard sync. It is particularly
useful evidence that a high-quality supersaw does not require oversampling every lane.

**Limit:** integration needs DC and transient management, while changing frequency and arbitrary phase
warp complicate the impulse schedule. A DUNE-like modulated stack still needs independent phase and
pitch state per lane. The existence of a published BLIT supersaw is not evidence that DUNE uses it.

### Discrete summation formulas (DSF)

Moorer's DSF uses closed forms of harmonic sums, allowing a signal to be explicitly limited to a
chosen number of partials without evaluating every sinusoid separately. [[Moorer, JAES 1976](https://aes.org/publications/elibrary-page/?id=2590)]

**Fit to KURV:** useful only as a static bandlimited reference or specialized spectrum generator.

**Limit:** it is fundamentally a harmonic/additive spectral construction. Dynamic phase warp and an
arbitrary cubic curve change the harmonic law; keeping them bandlimited would require recomputing a
safe spectrum or adding more dimensions. It violates KURV's procedural non-additive direction and is
not recommended.

### ADAA and exact integration of the custom curve

Antiderivative antialiasing evaluates differences of an antiderivative instead of directly sampling
a memoryless nonlinearity. Higher antiderivatives increase rejection and can use fewer operations than
straight oversampling for suitable functions, but require stable handling when adjacent inputs are
nearly equal. [[Bilbao, Esqueda, Parker & Välimäki, IEEE SPL 2017](https://www.research.ed.ac.uk/en/publications/antiderivative-antialiasing-for-memoryless-nonlinearities/)]
Recent work gives the same treatment explicitly for piecewise-polynomial functions.
[[Werner & Azelborn, "Antialiasing Piecewise Polynomial Waveshapers," DAFx-23](https://www.dafx.de/paper-archive/details/cu51KU_JsiXj1cvMi-2DOw)]

KURV's compiled custom curve is an unusually good target. Every realtime segment is cubic, so its
phase integral is quartic and cheap. For unwarped phase, one sample can be the exact average of the
continuous cubic over the phase interval, splitting only when the interval crosses a segment boundary
or wrap. This stays procedural: no FFT, wavetable, or harmonic bank. It also handles bunched edit
knots because runtime segments are fixed and periodic.

This is a **KURV-specific inference/experiment**, not a claim copied from the ADAA papers. The output
will include the box-kernel magnitude response and roughly half-sample timing of first-order interval
averaging, so gain/phase must be compared with the current path. Under nonlinear phase warp, integrating
the cubic over warped phase is not the same as averaging it over time because the warp Jacobian varies.
The cheap options are a locally-linear approximation, analytic/numerical integration of the composed
warp, or a fallback to hidden 2x for strong warp.

### Adaptive hidden oversampling

Oversampling remains the general fallback when the source bandwidth cannot be described by sparse
events or a cheap antiderivative. It need not be user-visible or applied to the entire synth. A voice
can stay at 1x for unwarped canonical waves and engage 2x only for strong phase warp, rapid pitch
motion, or a custom curve whose local phase advance crosses too many segments per host sample.

This is an engineering proposal, not a disclosed DUNE technique. It must retain fixed host latency,
preallocated state, and click-free mode changes. Switching only at note start or silence is much
simpler than changing rate mid-note. Per-sample factor switching is not recommended.

As a concrete first-party precedent, Dawesome says KULT's adaptive oversampling depends on played
pitch, but publishes neither thresholds nor filter/rate details. It establishes that hidden
pitch-dependent policy exists in commercial synths, not that DUNE uses it or that KURV should copy an
unknown implementation. [[KULT manual, p. 23](https://assets.tracktion.com/pdf/2026/kult-manual.pdf)]

### Mipmapped wavetables: useful reference, excluded implementation

Bandlimited wavetable banks select a prefiltered harmonic level by pitch and interpolate between
levels. They can be very clean and cheap for fixed periodic waveforms, which is why the oscillator
literature treats them as an important bandlimited category. [[Välimäki & Huovilainen, "Antialiasing
Oscillators in Subtractive Synthesis," IEEE SPM 2007](https://research.aalto.fi/en/publications/antialiasing-oscillators-in-subtractive-synthesis/)]

They are a poor architectural fit here: arbitrary curve edits require regeneration; continuous shape
and phase warp need additional dimensions or realtime filtering; per-lane pitch jitter causes table
level motion and gathers; and the user explicitly excludes wavetable/FFT substitution. Keep a
mipmapped result only as an offline quality/CPU comparator, not as KURV's source.

## Interaction with KURV's modulation topology

### Phase warp

A static smooth warp does not introduce a value discontinuity by itself, but it compresses time and
raises the local instantaneous phase step. For BLEP/BLAMP, the correct data are the actual fractional
crossing, jump magnitude, and local slope at the event. Passing the warped phase and local first
derivative is a good small-step approximation; it becomes less exact when curvature across the
four-sample correction support is large.

Recommended treatment:

- solve `g(phase) = edge` only when an edge is crossed, using the known monotone warp and one bounded
  Newton/bisection refinement;
- scale BLEP by the actual value jump and BLAMP by the derivative jump in time;
- use the local warped step for support, but fall back to 2x when curvature over support exceeds a
  measured error threshold.

DPW and BLIT are less attractive here because their histories/impulse schedules assume a more regular
phase trajectory. Exact custom-curve integration remains cheap only when the warp is absent or locally
linear.

### Continuous Sine -> Triangle -> Saw -> Pulse morph

At a fixed morph value, a linear combination of two sufficiently bandlimited endpoints is still
bandlimited. Therefore endpoint BLEP/BLAMP corrections can be blended with their actual event
amplitudes; no new universal morph algorithm is required. Saw -> Pulse deserves a fused implementation
because both endpoints share the wrap edge and pulse adds one duty-cycle edge.

When the morph amount itself changes, multiplication by a time-varying weight creates legitimate
sidebands. Slow sample-smoothed automation is fine; audio-rate shape modulation needs either a bounded
modulation bandwidth or selective oversampling. Endpoint corrections cannot remove aliases from
sidebands that have already crossed Nyquist.

### Custom periodic cubic curves

The periodic cubic construction is smoother than a hand-drawn polygon and has no saw-like value jump.
Its remaining bandwidth is driven by tight curvature, higher-derivative changes at segment boundaries,
phase warp, and time-varying curve/mix coefficients. BLEP at the cycle wrap is therefore the wrong
default tool.

The best direct experiment is exact quartic segment integration per sample. If a curve segment or
coefficient changes during the interval, interpolate coefficients first and integrate that trajectory,
or constrain edits to the existing block/parameter smoothing seam. Higher-order BLAMP-style correction
of derivative jumps is possible, but calculating and maintaining all jump orders is more specialized
than integrating the piecewise cubic already present.

### Pitch jitter / Swarm

KURV's current target-and-slew model is a cheap implementation of the plausible pitch-only reading of
DUNE's public semantics: independent per-lane motion with static pan/gain. It is not confirmed as
DUNE's internal modulation law. Smooth phase-step ramps avoid the extra broadband impulse produced by
abrupt frequency changes.

Oscillator-edge AA still sees a moving phase increment. For slow jitter, use the instantaneous step
and true fractional crossing. For fast/deep jitter, FM sidebands—not merely waveform discontinuities—
become the limiting source, so the remedies are:

1. bandlimit the jitter trajectory and its slew;
2. clamp instantaneous phase step below the existing safety ceiling;
3. engage 2x only when the modulation bandwidth plus the wanted oscillator harmonics exceed the host
   Nyquist budget.

Do not randomize pan or gain as an antialiasing measure. That changes the sound and costs more without
reducing oscillator foldback.

## Ranked experiments for KURV

| Rank | Experiment | Expected sound | Expected CPU | Why it ranks here |
|---:|---|---|---|---|
| 1 | **Warp-aware four-point BLEP/BLAMP at 1x.** Keep the existing SIMD cubic spline, but calculate the true fractional crossing and event slope under phase warp/dynamic pitch. Compare the existing standard and optimized coefficients. | High improvement on warped saw/pulse/triangle; preserves current morph | Lowest likely cost; extra solve only at events | Deepens code already present and attacks the known approximation rather than replacing the engine |
| 2 | **Exact interval integration for `WaveCurveRt`.** Add a quartic antiderivative per cubic segment and average across the traversed phase interval; start with no warp, then local-linear warp. | Largest likely improvement for full custom mix and tight curves | Small fixed arithmetic plus rare segment/wrap splits | Covers the current procedural path that BLEP/BLAMP does not address |
| 3 | **Hidden risk-gated 2x fallback.** Engage only at note start for strong warp, fast/deep jitter, or too many custom-curve segment crossings; retain fixed output latency. | Broad safety net for modulation cases | Near-1x for ordinary patches, 2x only for risky voices/notes | General solution without making all 64 lanes pay all the time |
| 4 | **Longer event-sparse minBLEP for saw/pulse and sync comparator.** Use a precomputed 8-32-sample minimum-phase residual and fixed per-lane accumulator. | Potentially highest static edge rejection | Excellent at low/mid pitches; worsens with event rate, overlap, and cache pressure | Establishes the quality ceiling and tests whether event sparsity beats a second full render |
| 5 | **DPW4 endpoint comparator.** Restrict it to steady unwarped saw/triangle, with a documented low-frequency crossover. | Strong classical-wave quality in its comfort zone | Very low regular arithmetic | Useful benchmark; too topology-specific to become the universal KURV core without evidence |
| 6 | **BLIT-FDF supersaw comparator.** Build only in an offline/lab harness with dynamic pitch motion and measure DC/transients. | Strong static supersaw and sync quality | Event-dependent; more state than PolyBLEP | Tests a published non-oversampled supersaw, but integration and warp complexity make it a weaker production fit |

Use one frozen source snapshot because the working tree is active. Each experiment should render the
same matrix at 44.1, 48, and 96 kHz:

- fundamentals from 27.5 Hz through the highest supported MIDI note;
- all canonical endpoints and 0.25 morph increments;
- pulse widths near the minimum, 0.5, and maximum;
- every warp mode at 0, 0.5, and maximum amount;
- representative smooth and tightly curved custom shapes;
- Swarm off, slow/deep, and fastest/deepest;
- 1, 8, 32, and 64 lanes.

Compare against a long offline bandlimited/16x-decimated oracle, not against DUNE's unknown internals.
Record wanted-harmonic error, nonharmonic/alias residual, DC, peak, and nanoseconds per host sample.
The acceptance target should be stated as a residual floor and wanted-band tolerance; "sounds like
DUNE" cannot identify an algorithm.

## What cannot be known about DUNE from public evidence

The following questions remain unanswered without source access, a developer disclosure, or careful
black-box measurements that still cannot uniquely identify implementation:

- oscillator BLEP/BLAMP/BLIT/DPW/wavetable family, polynomial order, or correction length;
- fixed, frequency-dependent, or patch-dependent oscillator oversampling;
- whether saw/pulse/triangle share one kernel or use separate optimized algorithms;
- Swarm generator, random distribution, seed/retrigger rules, smoothing law, cadence, and
  cross-oscillator correlation;
- phase accumulator precision, SIMD layout, state packing, lane culling, or cache strategy;
- special handling of high notes, narrow pulses, sync, FM, or modulation-rate modes;
- how much CPU advantage comes from oscillator math versus multithreading, host buffering, static
  lane weighting, or other engine architecture.

A spectrum can falsify a poor implementation and a CPU profile can measure total cost, but neither can
prove which proprietary algorithm produced the result. Recommendations should therefore be judged on
KURV's own sound/CPU measurements and procedural contract.

## Primary source ledger

- Synapse Audio, [DUNE 3.5 User's Manual](https://www.synapse-audio.com/DUNE-35-Manual.pdf),
  especially pp. 7-8, 18-19, 25-30, and 40.
- Synapse Audio, [DUNE 3.6 User's Manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf),
  especially pp. 8, 17, 20, and 30.
- Synapse Audio, [DUNE 3 product page](https://www.synapse-audio.com/dune3.html).
- Synapse Audio, [DUNE 3.5 release note](https://www.synapse-audio.com/news-dune35update.html).
- Synapse Audio, [DUNE 1.2 release note](https://www.synapse-audio.com/news-dune-v12-released.html).
- Richard Hoffmann (Synapse Audio), [forum reply on oscillator oversampling](https://www.kvraudio.com/forum/viewtopic.php?p=4768973), 2011.
- Välimäki, Pekonen & Nam, ["Perceptually informed synthesis of bandlimited classical waveforms using integrated polynomial interpolation"](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf), JASA 131(1), 2012.
- Brandt, ["Hard Sync Without Aliasing"](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf), ICMC 2001.
- Stilson & Smith, ["Alias-Free Digital Synthesis of Classic Analog Waveforms"](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext), ICMC 1996.
- Nam, Välimäki, Abel & Smith, ["Efficient Antialiasing Oscillator Algorithms Using Low-Order Fractional Delay Filters"](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf), IEEE TASLP 18(4), 2010.
- Välimäki, Nam, Smith & Abel, ["Alias-Suppressed Oscillators Based on Differentiated Polynomial Waveforms"](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf), IEEE TASLP 18(4), 2010.
- Esqueda, Välimäki & Bilbao, ["Rounding Corners with BLAMP"](https://www.dafx.de/paper-archive/details/xRM0GiuF-JmeN0_aaP9VNg), DAFx-16.
- Bilbao, Esqueda, Parker & Välimäki, ["Antiderivative Antialiasing for Memoryless Nonlinearities"](https://www.research.ed.ac.uk/en/publications/antiderivative-antialiasing-for-memoryless-nonlinearities/), IEEE SPL, 2017.
- Werner & Azelborn, ["Antialiasing Piecewise Polynomial Waveshapers"](https://www.dafx.de/paper-archive/details/cu51KU_JsiXj1cvMi-2DOw), DAFx-23.
- Moorer, ["The Synthesis of Complex Audio Spectra by Means of Discrete Summation Formulas"](https://aes.org/publications/elibrary-page/?id=2590), JAES 24(9), 1976.
- Välimäki & Huovilainen, ["Antialiasing Oscillators in Subtractive Synthesis"](https://research.aalto.fi/en/publications/antialiasing-oscillators-in-subtractive-synthesis/), IEEE Signal Processing Magazine 24(2), 2007.
- Korg, [US7589272B2](https://patents.google.com/patent/US7589272), specific oscillator-discontinuity correction claims.
- Dawesome, [KULT manual](https://assets.tracktion.com/pdf/2026/kult-manual.pdf), p. 23 adaptive-oversampling statement.
