# Oscillator-Level Harmonic Shaping in a Procedural Virtual-Analog Oscillator

Research report — 2026-08-12

## Question and short verdict

Can a procedural virtual-analog oscillator change its harmonic balance at the oscillator level, without sending generated audio through a causal low-pass, band-pass, or high-pass filter?

**Yes. This is established and technically sound.** Phase distortion, variable-slope and variable-notch oscillators, hard sync, wavefolding, discrete-summation-formula oscillators, and direct synthesis of filter-like waveforms all change harmonic content inside the sound generator. Casio documented phase-address distortion in the early 1980s; oscillator research has treated phaseshaping and closed-form spectral control for decades; Mutable Instruments shipped oscillator models explicitly named `ZLPF`, `ZPKF`, `ZBPF`, and `ZHPF` that directly construct filter-like waveforms.

However, four claims must not be collapsed into one:

1. **Oscillator-level** means the waveform is constructed from oscillator phase before voice mixing or a downstream audio filter. It does not automatically mean virtual analog or phase-neutral.
2. **Procedural** means the realtime waveform is evaluated by an algorithm rather than played from a stored sampled cycle. It does not mean zero cost, and it does not forbid small coefficient or correction tables.
3. **Phase-neutral harmonic shaping** means retained Fourier coefficients keep their phase relative to the oscillator's phase reference. Most established oscillator shapers, including phase distortion, sync, and wavefolding, do not satisfy this.
4. **Virtual analog (VA)** normally means a digital emulation of an analog instrument, circuit, or subtractive-synthesis behavior. A novel digital spectral facility can live inside a VA synth, but it is not itself an analog model unless it has an analog target.

The exact proposed construction based on symmetric evaluations of the procedural waveform is mathematically valid and can preserve harmonic phases. The survey found no established synth product, paper, patent, or open-source oscillator that names that exact construction as a standard VA method. It is best described as a **cycle-domain zero-phase harmonic shaper**, not an analog filter. This is an evidence-of-absence limitation, not proof that nobody has ever implemented it.

One correction to the earlier proposal is important: the three-sample symmetric kernel

\[
y(\phi)=\tfrac12x(\phi)+\tfrac14x(\phi-\delta)+\tfrac14x(\phi+\delta)
\]

has harmonic response \(\cos^2(\pi n\delta)\). That is a **comb response over harmonic number**, not a general monotonic low-pass. It is monotonic only before its first zero. It is useful, but by itself it cannot honestly provide arbitrary LP/BP/HP behavior.

## What “the harmonics” are and where they sit

For a stationary periodic oscillator waveform \(x(\phi)\), with normalized cycle phase \(0\leq\phi<1\), write

\[
x(\phi)=\sum_{n=-\infty}^{\infty}C_n e^{j2\pi n\phi}.
\]

The coefficient \(C_n\) contains the magnitude and phase of harmonic \(n\). At fundamental frequency \(f_0\), that harmonic appears at \(n f_0\). A real waveform has the conjugate symmetry \(C_{-n}=C_n^*\).

In a direct additive or Fourier oscillator, those coefficients are explicitly stored or calculated. In a procedural VA oscillator they usually are **not stored anywhere**. They are implicit in the shape function and its discontinuities:

- a sine contains only the fundamental;
- an ideal saw has every integer harmonic with magnitude proportional to \(1/n\);
- a symmetric square contains odd harmonics proportional to \(1/n\);
- a triangle contains odd harmonics proportional to \(1/n^2\);
- pulse width, phase bending, sync, nonlinear shaping, and corner geometry change the resulting coefficients.

The oscillator's antialiasing method then approximates a bandlimited version of that periodic function. Stilson and Smith's primary paper treats additive truncation, wavetable synthesis, discrete summation formulae, and bandlimited impulse trains as alternative ways of producing classic analog waveforms, not as different locations for a downstream synthesizer filter: [Stilson and Smith, “Alias-Free Digital Synthesis of Classic Analog Waveforms,” ICMC 1996](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext).

For sample rate \(f_s\), the largest stationary harmonic that can exist without crossing Nyquist is approximately

\[
K=\left\lfloor\frac{f_s/2}{f_0}\right\rfloor.
\]

That is the useful meaning of pitch adaptivity at the oscillator: a high note has fewer legal harmonics than a low note. An oscillator-relative control can operate on harmonic number \(n\); an absolute-Hz cutoff maps to a changing harmonic index \(f_c/f_0\).

Once a shaping parameter moves over time, the signal is no longer one stationary periodic waveform. Modulation creates sidebands, and nonsynchronous audio-rate modulation can make the spectrum inharmonic. The Vector Phaseshaping paper explicitly analyzes control-rate and audio-rate waveshape modulation and warns that some parameter combinations alias: [Kleimola et al., “Vector Phaseshaping Synthesis,” DAFx 2011](https://www.dafx.de/paper-archive/2011/Papers/55_e.pdf). Therefore “the harmonic phase is preserved” is exact only for a frozen setting, or for a carefully specified time-varying operator. It is not a blanket statement about arbitrarily fast modulation.

## Causal audio filtering versus cycle-domain shaping

A normal audio filter consumes a sequence of past/current samples. A causal LTI filter has

\[
y[k]=\sum_m h[m]x[k-m]
\]

or an equivalent state-space/IIR recurrence. Its complex response \(H(e^{j\omega})\) has both magnitude and phase. Resonant filters also require state and usually feedback.

A cycle-domain shaper instead computes the current sample from known oscillator phase:

\[
y[k]=F(\phi_k,\Delta\phi_k,q_k).
\]

It may evaluate the waveform at other locations within the same mathematical cycle because the oscillator owns the formula for the whole cycle. Those are not future host-audio samples. This removes the causality obstacle that would apply to a streaming zero-phase audio filter.

There is a simple proof of why the distinction matters. A zero-phase LTI impulse response is even: \(h[m]=h[-m]\). If it is also causal, \(h[m]=0\) for \(m<0\); evenness then forces \(h[m]=0\) for \(m>0\). Only the instantaneous tap \(h[0]\) remains, which cannot have a non-flat magnitude response. A nontrivial zero-phase streaming filter must therefore be noncausal, buffered, or offline. A procedural oscillator can evade this only because it can evaluate its known periodic function at \(\phi+\delta\) without waiting for future audio.

This does **not** make the cycle-domain operation an analog filter. It is a different synthesis operator that can be made to have a filter-like harmonic magnitude response.

## Mathematical check of symmetric phase sampling

Consider a finite phase-domain kernel

\[
y(\phi)=\sum_{m=-M}^{M}a_m x(\phi+m\delta).
\]

Substituting the Fourier series gives

\[
y(\phi)=\sum_n C_n H_n e^{j2\pi n\phi},\qquad
H_n=\sum_{m=-M}^{M}a_m e^{j2\pi nm\delta}.
\]

If the real weights are symmetric, \(a_m=a_{-m}\), then

\[
H_n=a_0+2\sum_{m=1}^{M}a_m\cos(2\pi nm\delta),
\]

which is real. The consequences are precise:

- \(H_n>0\): harmonic \(n\) keeps its phase;
- \(H_n=0\): harmonic \(n\) is removed;
- \(H_n<0\): harmonic \(n\) is inverted by \(\pi\), so strict phase preservation is lost.

Symmetry alone therefore gives a zero-or-\(\pi\) response. **Strict phase preservation requires \(H_n\geq0\) for every retained harmonic.**

For the proposed three-point kernel,

\[
H_n=\tfrac12+\tfrac12\cos(2\pi n\delta)=\cos^2(\pi n\delta)\geq0.
\]

The phase-preservation claim is correct. The low-pass claim needs qualification: \(\cos^2\) repeatedly falls and rises with \(n\). If the first zero is placed at or above the highest legal harmonic, the retained portion is a gentle monotonic tilt. If the zero is moved into the audible harmonic range, higher lobes return and it becomes a comb.

Two other exact identities are useful:

\[
x(\phi)-x(\phi+\tfrac12)
\]

removes every even harmonic and doubles every odd one, while

\[
x(\phi)+x(\phi+\tfrac12)
\]

removes every odd harmonic and doubles every even one (and DC). These are genuine stateless oscillator-domain selectors, but they are not LP/BP/HP filters.

General smooth, nonnegative harmonic envelopes also exist mathematically. Circular convolution with the Poisson kernel multiplies harmonic \(n\) by \(r^{|n|}\), and a Fejér kernel gives a finite triangular harmonic taper. The difficulty is implementation: applying those exact operators to an arbitrary procedural waveform requires an integral, enough phase samples, explicit harmonic coefficients, or a special closed form for that waveform family. The mathematics does not grant a universal constant-cost evaluator.

## Established precedents

### 1. Casio phase distortion: oscillator-level filter-like timbre control

Casio's own history describes the CZ-101's PD source as modifying waveform phase angles to generate complex overtones: [Casio CZ-101 history](https://web.casio.com/emi/40th/history/cz-101.html). Casio's current CZ app description shows the original DCO/DCW/DCA organization and says the DCW envelope varies tone over time: [Casio CZ app](https://web.casio.com/app/en/cz/).

The underlying Casio patent is unusually explicit. A phase accumulator produces a uniform address; a harmonic-control signal changes the address rate within one cycle; the modified address reads waveform memory; and the control may change over time. The claims specify that this happens without feedback from the waveform store: [US 4,658,691](https://patents.google.com/patent/US4658691A/en). The original Japanese filing, `波形発生方式`, contrasts its method with additive synthesis and variable digital filters and states the goal of smoothly changing the spectrum and removing high-frequency components of saw/square signals: [JPS59-111515A, Japanese original](https://patents.google.com/patent/JPS59111515A/ja).

This proves that oscillator-level, dynamically controlled, filter-like harmonic shaping is not new. It also demonstrates the limitation: phase distortion deliberately changes the phase trajectory and generally changes harmonic phases. It is digital phase synthesis, not a phase-neutral analog filter.

The research literature reaches the same classification. Lazzarini et al. describe Casio PD as a way to imitate conventional source-filter controls and derive it as phase-synchronous complex phase modulation: [“Adaptive Phase Distortion Synthesis,” DAFx 2009](https://www.dafx.de/paper-archive/2009/papers/paper_12.pdf). Kleimola et al. show that a PD control can sound like a low-pass sweep and extend the technique to formant peaks and audio-rate shape modulation: [Vector Phaseshaping, DAFx 2011](https://www.dafx.de/paper-archive/2011/Papers/55_e.pdf).

### 2. Direct synthesis of LP/peak/BP/HP responses: Mutable Instruments Braids and Plaits

Mutable Instruments provides the clearest product precedent. The official Braids manual says its `ZLPF`, `ZPKF`, `ZBPF`, and `ZHPF` models directly synthesize the time-domain response of those filter types excited by classic analog waveforms. It explicitly contrasts this with first generating a waveform and then filtering it: [Braids official manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/braids/manual).

The corresponding open-source implementation advances carrier/modulator phases, resets them at cycle boundaries, constructs windowed sine/pulse components, and integrates one component for some modes. There is no generic downstream LP/BP/HP processor in this model: [Braids `RenderDigitalFilter`, pinned source](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/braids/digital_oscillator.cc#L328-L408).

Plaits carried the idea forward. Its official manual calls the auxiliary output a simulation of filtered waveforms by windowed sine waves and morphs continuously through peaking, LP, BP, and HP behavior: [Plaits official manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/plaits/manual/).

This is almost exactly the user's conceptual category: oscillator-level, filter-like, pitch-synchronous waveform construction. It is not phase-neutral, and Braids' own manual does not call the direct-response method a conventional VA filter.

### 3. Closed-form harmonic control: DSF and `gbuzz`

Moorer's Discrete Summation Formulae (DSF) synthesize controlled partial series using closed forms rather than one oscillator per partial. The original paper emphasizes exact partial-count limits and controllable one-sided spectra: [Moorer, “The Synthesis of Complex Audio Spectra by Means of Discrete Summation Formulas,” JAES 1976](https://aes.org/publications/elibrary-page/?id=2590). Stilson and Smith specifically include DSF among algorithms for classic analog oscillator waveforms.

Csound's `gbuzz` is a long-lived open-source implementation. Its manual exposes the lowest harmonic, number of harmonics, and a multiplier that creates an exponential amplitude series; that multiplier can be modulated during performance: [Csound `gbuzz` manual](https://csound.com/docs/manual/gbuzz.html). The implementation evaluates a closed-form numerator/denominator per sample instead of looping over every partial: [Csound `gbuzz`, pinned source](https://github.com/csound/csound/blob/ded5d15dece77539c04fbaaa160144df090771e2/OOps/ugens4.c#L147-L240).

DSF proves that procedural, bounded-cost, oscillator-level spectral-envelope control is possible without a harmonic bank or FFT. It does not provide a universal filter for an arbitrary input waveform. It generates a waveform family whose coefficients follow the chosen formula.

### 4. Pitch-adaptive harmonic reduction by changing oscillator geometry

IBM patented a particularly simple source-level method that progressively converts saw into triangle as oscillator frequency rises. The patent explicitly motivates this as reducing upper harmonics and aliasing without spending the processor budget on a low-pass filter: [EP 0 484 048 A2 / US 5,194,684 family](https://patents.google.com/patent/EP0484048A2/en). Its selectable-offset and absolute-value construction is a waveform transform, not a spectral filter, but it is pitch-adaptive oscillator-level harmonic reduction.

Another oscillator patent evaluates a continuously variable function directly from accumulated phase and a shape parameter, producing smooth waveforms with changing harmonic content: [US 6,806,413](https://patents.google.com/patent/US6806413B1/en). These patents reinforce the established engineering pattern: alter waveform geometry at the generator when the desired spectral motion is simple enough.

### 5. Analog oscillator-level harmonic shaping and genuine VA models

Analog synthesis is not limited to East-Coast VCO → VCF subtraction. The Buchla 259's wavefolder changes timbre at oscillator level through nonlinear circuitry. Esqueda et al. derive a digital model from the actual circuit, use memoryless input-output mappings for its folding cells, validate against SPICE, and explicitly call the result a virtual-analog wavefolder: [“Virtual Analog Buchla 259 Wavefolder,” DAFx 2017](https://www.dafx.de/paper-archive/2017/papers/DAFx17_paper_82.pdf).

This is decisive for terminology: a memoryless oscillator-level shaper can be genuinely VA when it models an actual analog shaper. Wavefolding expands the spectrum and changes harmonic phases; it is not a phase-neutral LP/BP/HP operation.

### 6. Vital: true phase-preserving spectral masks, but in a wavetable/Fourier architecture

Vital is relevant as a contrast, not as the implementation template for strict procedural VA. Its oscillator enumerates spectral morph modes including low-pass and high-pass: [Vital spectral morph enum, pinned source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.h#L100-L113).

Vital stores each harmonic's magnitude separately from its normalized complex coefficient. Its LP/HP morphs retain or zero bins and multiply the retained magnitude by that same normalized complex value before inverse transformation: [LP/HP implementation](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/spectral_morph.h#L243-L305), [magnitude/phase decomposition](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/lookups/wavetable.cpp#L160-L177). Thus retained bins keep their original phases.

It also chooses the last harmonic from phase increment, inverse-transforms into wave buffers, and crossfades regenerated buffers: [buffer generation and pitch limit](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L736-L864), [buffer refresh/crossfade](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1381-L1432).

Vital therefore demonstrates the complete phase-preserving, pitch-adaptive feature—but by using explicit Fourier/wavetable buffers. It does not demonstrate that the same generality is free or available in a strict no-table procedural VA oscillator.

## Is it accurately called “true VA”?

There is no standards-defined certification called “true VA.” Technical literature uses VA for digital emulation of analog synthesis or analog circuitry. Pekonen's Aalto dissertation defines its topic as digital modeling of the subtractive-synthesis principle and includes two models of the measured Minimoog Voyager saw output—one using phase distortion and one using pitch-dependent post-filtering: [Pekonen, “Filter-Based Oscillator Algorithms for Virtual Analog Synthesis,” Aalto 2014](https://research.aalto.fi/en/publications/filter-based-oscillator-algorithms-for-virtual-analog-synthesis/). The method need not reproduce every transistor if it is modeling an analog target's observable behavior.

That yields a practical classification:

| Technique | Oscillator-level | Procedural realtime path | Harmonic phase-neutral | Honest VA label |
|---|---:|---:|---:|---|
| BLEP/BLAMP classic saw, pulse, triangle | Yes | Yes | Not a separate shaper | Yes: established VA oscillator approximation |
| PWM, variable slope/notch, hard sync | Yes | Yes | Generally no | Yes when modeling analog oscillator behavior |
| Circuit-derived wavefolder | Within oscillator/timbre section | Yes | No | Yes: it models an analog circuit |
| Casio phase distortion | Yes | Yes, often with sine lookup | No | Historically digital synthesis, not inherently VA |
| Braids/Plaits direct LP/BP/HP-like waveforms | Yes | Yes | No | Filter-like oscillator synthesis; not a causal VA filter |
| DSF spectral-envelope oscillator | Yes | Yes, closed form | Can retain the designed series phase | VA-capable source algorithm, not an analog filter model by itself |
| Vital LP/HP spectral morph | Yes | No: Fourier/wave buffers | Yes for retained bins | Digital wavetable spectral processing |
| Symmetric KURV phase kernel | Yes | Yes | Yes if every \(H_n\geq0\) | Digital oscillator extension unless calibrated to an analog target |

So KURV can remain accurately described as a procedural VA synth while offering such a feature, just as many VA instruments contain digital extensions. The precise feature should not be marketed as an “analog filter with no phase shift.” Better names are **Harmonic Shape**, **Cycle Filter**, **Spectral Tilt**, or **Oscillator Tone**. If strict VA purity is the product requirement, choose analog-grounded shapers—PWM, sync, variable slope/notch, or a modeled wavefolder—instead of arbitrary phase-neutral LP/BP/HP masks.

## Current KURV classification

The inspected working tree already has a procedural VA core:

- `src/oscillators/va/mod.rs:1-58` defines the module as a procedural VA oscillator and stores one phase value per oscillator lane.
- `src/oscillators/va/render.rs:57-72` advances phase and evaluates bandlimited saw/pulse or the procedural shape morph.
- `src/oscillators/va/render.rs:85-107` applies oscillator phase warp before waveform evaluation.
- `src/oscillators/va/antialias.rs:143-220` constructs triangle and saw directly from phase and applies local BLAMP/BLEP-style corrections.
- `src/oscillators/va/render.rs:2595-2639` dispatches sine, triangle, saw, and pulse evaluators for scalar/SIMD paths.

This is consistent with the accepted algorithmic use of “virtual-analog oscillator”: classic analog waveform families are generated from phase with antialiasing corrections. It is not a transistor-level VCO simulation, and “true” adds no measurable technical requirement.

Two qualifications matter:

1. KURV already contains oscillator-level digital extensions. `src/oscillators/va/warp.rs:8-15` exposes PWM, phase-bend, and harmonic warp modes. These are oscillator-phase transformations, but they are not generally harmonic-phase-neutral.
2. The custom-wave path is not sampled wavetable playback, but it is not a single closed analytic primitive either. `src/wave_curve.rs:17-21` compiles the editable curve to 16 cubic segments, and `src/oscillators/va/table.rs:11-45` holds up to 16 compiled curve frames. The canonical VA shapes remain procedural; the custom system is a fixed-coefficient phase-function/table hybrid.

The existing ordered filters are genuinely separate causal audio processors. `src/filters/svf.rs:93-167` stores two integrator states per stereo channel, and `src/voices/voice.rs:6100-6136` renders oscillator modules into a group and then processes a filter module encountered in program order. That is the path the user explicitly does not mean.

## What is feasible in KURV without FFTs or sample wavetables

### A small phase-preserving family is feasible

The existing pure waveform samplers can be evaluated at symmetrically offset raw phases, combined with fixed SIMD weights, and advanced only once. For a static control this adds no filter history, feedback, allocation, lock, FFT, or waveform-buffer rebuild.

To preserve the harmonics of the *effective warped waveform*, each tap must evaluate the whole effective function at \(\phi\pm\delta\): phase warp, waveform selection, and the appropriate edge correction. Adding offsets only after the warp is not generally equivalent and loses the Fourier guarantee relative to the warped source.

Useful honest modes from very few taps include:

- odd-only and even-only selectors;
- one or more comb/notch families over harmonic number;
- a gentle upper-harmonic tilt when the first kernel zero remains above the legal harmonic range;
- complementary phase-preserving comb families using \(\cos^2\) and \(\sin^2\) responses.

These can be pitch-adaptive by deriving their safe offset or active mode from each unison lane's phase increment. They should run per lane before unison mixing, because detuned lanes have different \(f_0\) and Nyquist harmonic counts.

### General LP/BP/HP behavior is the hard boundary

A few symmetric taps cannot produce arbitrary sharp monotonic LP, HP, BP, and notch responses without repeated lobes. The honest options are:

1. **More phase taps:** still procedural and phase-preserving if designed carefully, but CPU grows as \(O(T)\) waveform evaluations per lane per sample.
2. **Closed-form waveform families:** DSF or waveform-specific formulas provide bounded cost and controllable envelopes, but they synthesize a new family rather than filter every KURV shape uniformly.
3. **Direct filter-response synthesis:** follow the Casio/Braids/Plaits pattern. This is efficient and oscillator-level, but generally does not preserve the original harmonic phases.
4. **Explicit harmonic/Fourier representation:** gives the most general phase-preserving LP/BP/HP behavior, as Vital demonstrates, but violates the strict no-FFT/no-wavetable direction.

There is no method in the surveyed primary literature or implementations that simultaneously provides all of the following for an arbitrary procedural waveform:

- exact general LP/BP/HP magnitude control;
- exact preservation of every retained harmonic phase;
- no temporal filter state;
- no Fourier/additive/table representation;
- constant tiny CPU independent of shape and cutoff;
- arbitrary audio-rate modulation without new aliases.

That combination is not merely uncommon; its constraints remove the known mechanisms that supply the missing spectral information.

## Modulation and CPU constraints

“No additional filter state” is achievable. “No CPU usage” is not. A bypassed feature can keep KURV's current exact fast path. An enabled three-point kernel requires approximately three full waveform evaluations instead of one unless common subexpressions or waveform-specific identities remove work. BLEP/BLAMP evaluation must also occur at each shifted phase to preserve the intended shifted bandlimited waveform.

Control modulation can remain allocation-free and lock-free, but it is still time-varying DSP. Fast modulation of offsets or weights creates sidebands. It needs bounded parameter interpolation and an antialiasing policy; the DAFx phaseshaping literature explicitly treats audio-rate shape modulation as a new synthesis regime, not a free control update.

An adaptive implementation can skip work when:

- the mode is bypassed;
- the source is sine and the response only rescales its sole harmonic;
- a high note has too few legal harmonics for the selected distinction;
- two taps collapse to the same phase/weight;
- modulation is static across a block and coefficients can be hoisted.

Those optimizations reduce measured cost; they cannot make an active nontrivial transform literally free.

## Recommendation

For KURV's stated identity, the technically honest direction is:

1. Keep the current BLEP/BLAMP procedural oscillator as the canonical source.
2. Treat any new phase-symmetric operation as a **digital cycle-domain harmonic shaper inside the VA oscillator**, not as an analog LP/BP/HP filter.
3. Start only with responses that a small nonnegative symmetric kernel implements truthfully—odd/even, gentle tilt, and harmonic comb/notch shapes.
4. If the product requirement is specifically convincing LP/BP/HP/formant sweeps at low cost, use a Braids/Plaits-style direct-response oscillator family and accept that it is not phase-neutral.
5. If the requirement is exact arbitrary phase-preserving spectral filtering, the Fourier/wave-buffer architecture previously rejected is the established solution; do not disguise a short phase kernel as equivalent.
6. If “true VA” must apply to every enabled oscillator mode, implement and calibrate analog-grounded shapes such as variable slope/notch, sync, PWM, and a circuit-derived wavefolder instead.

The result is still a real oscillator-level design. The limitation is not whether oscillator-level harmonic control exists—it clearly does—but which of **analog fidelity, phase preservation, response generality, modulation bandwidth, and CPU** KURV chooses to prioritize.

## Primary-source index

- [Casio CZ-101 official history](https://web.casio.com/emi/40th/history/cz-101.html)
- [Casio CZ official app / DCO-DCW-DCA description](https://web.casio.com/app/en/cz/)
- [Casio phase-address synthesis patent, US 4,658,691](https://patents.google.com/patent/US4658691A/en)
- [Casio original Japanese filing, JPS59-111515A](https://patents.google.com/patent/JPS59111515A/ja)
- [Moorer 1976 DSF paper, AES](https://aes.org/publications/elibrary-page/?id=2590)
- [Stilson and Smith 1996 VA oscillator paper](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main;view=fulltext)
- [Adaptive Phase Distortion Synthesis, DAFx 2009](https://www.dafx.de/paper-archive/2009/papers/paper_12.pdf)
- [Vector Phaseshaping Synthesis, DAFx 2011](https://www.dafx.de/paper-archive/2011/Papers/55_e.pdf)
- [Pekonen 2014 VA oscillator dissertation, Aalto](https://research.aalto.fi/en/publications/filter-based-oscillator-algorithms-for-virtual-analog-synthesis/)
- [Virtual Analog Buchla 259 Wavefolder, DAFx 2017](https://www.dafx.de/paper-archive/2017/papers/DAFx17_paper_82.pdf)
- [IBM pitch-adaptive upper-harmonic-reduction patent](https://patents.google.com/patent/EP0484048A2/en)
- [Continuously variable procedural oscillator patent, US 6,806,413](https://patents.google.com/patent/US6806413B1/en)
- [Mutable Instruments Braids manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/braids/manual)
- [Mutable Instruments Braids pinned source](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/braids/digital_oscillator.cc#L328-L408)
- [Mutable Instruments Plaits manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/plaits/manual/)
- [Mutable Instruments variable-saw pinned source](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/plaits/dsp/oscillator/variable_saw_oscillator.h#L44-L160)
- [Csound `gbuzz` manual](https://csound.com/docs/manual/gbuzz.html)
- [Csound `gbuzz` pinned source](https://github.com/csound/csound/blob/ded5d15dece77539c04fbaaa160144df090771e2/OOps/ugens4.c#L147-L240)
- [Vital LP/HP spectral morph pinned source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/spectral_morph.h#L243-L305)
