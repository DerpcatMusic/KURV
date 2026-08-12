# Oscillator-Domain Adaptive Filter Architectures for KURV

Research report — 2026-08-12

## Scope and verdict

This report asks which oscillator-level techniques can give KURV low-pass, high-pass,
band-pass, peak/formant, or more general adaptive harmonic shaping while retaining its procedural
virtual-analog core. It distinguishes a generated waveform that **has a filter-like spectrum** from
a causal filter that processes an already generated audio stream.

The short answer is:

- **Oscillator-level filter-like synthesis is established.** Mutable Instruments shipped direct
  LP/peak/BP/HP response synthesis; phase distortion has produced low-pass-like sweeps since the
  Casio CZ family; DSF oscillators directly generate exponential spectral envelopes; PAF directly
  generates resonant/formant envelopes; and LP-BLIT directly generates a low-pass spectrum.
- **Oscillator-level does not imply zero phase shift.** It only means there is no independent input
  audio stream whose before/after phase can be compared. Phase distortion, hard sync, windowed
  sine responses, and wavefolding deliberately establish new harmonic phases.
- **The strongest phase-aligned procedural precedent is LP-BLIT**, but it is low-pass only in the
  published method and its overlapping pulses plus sine/hyperbolic-sine evaluation are not cheap.
- **The best practical match for KURV is a dedicated direct-response oscillator family**, modeled
  on Braids/Plaits, with a pitch-synchronous continuous LP → peak → BP → HP response control. It
  should be described as an oscillator response or harmonic contour, not as an analog VCF.
- **The lowest-cost strict-VA option is KURV's existing phase-warp seam extended with an
  analog-grounded Edge/Contour mode.** It can sound filter-like but cannot truthfully promise the
  magnitude response, resonance, or phase behavior of LP/BP/HP filters.
- **There is no zero-cost enabled solution.** Bypass can retain the current exact fast path. Every
  nontrivial enabled method performs additional arithmetic, and every time-varying method creates
  sidebands that need an aliasing policy.

The term “true VA” does not impose one sanctioned oscillator algorithm. Yamaha's current AN-X
documentation explicitly calls AN-X virtual analog while placing Pulse Width, Self Sync, and Wave
Shaper inside each oscillator and keeping Filter 1/2 as separate blocks. Its Japanese original says
the same: [Yamaha AN-X manual, English](https://manual.yamaha.com/mi/synth/montage_m/en/om01basicoperation0350.html),
[Yamaha AN-X manual, Japanese](https://manual.yamaha.com/mi/synth/montage_m/ja/om01basicoperation0350.html).
KURV can therefore remain a procedural VA synth with an oscillator-domain digital extension, but
the extension itself is only an analog model when it is tied to an analog topology or circuit.

## Requirements made explicit

For one unison lane with normalized phase \(\phi\), phase increment \(\Delta\phi=f_0/f_s\), and a
stationary periodic waveform

\[
x(\phi)=\sum_{n=-\infty}^{\infty} C_n e^{j2\pi n\phi},
\]

an oscillator-domain “filter” is a generator

\[
y[k]=F(\phi_k,\Delta\phi_k,q_k)
\]

whose output coefficients resemble \(C_n H_n\). A streaming audio filter instead reads an input
sequence and has temporal state or a convolution history. This distinction permits an oscillator to
evaluate a known cycle at arbitrary phase positions, but it does not make spectral information free.

The useful harmonic ceiling is lane-specific:

\[
N_{\max}=\left\lfloor\frac{g f_s}{2f_0}\right\rfloor
=\left\lfloor\frac{g}{2\Delta\phi}\right\rfloor,
\]

where \(g<1\) is an alias guard. Every detuned unison lane has a different \(f_0\), so a genuinely
adaptive oscillator must calculate or bound its response per lane, not once after unison mixing.

“No phase shift” has three distinct meanings which must not be conflated:

1. **No causal filter delay:** no stream history and no output latency.
2. **Linear phase:** all harmonics receive the same delay; waveform shape is delayed but preserved.
3. **Zero harmonic-phase change:** each retained \(C_n\) is multiplied by a nonnegative real number.

Only the third is strict phase preservation relative to the source cycle. A real symmetric response
can still be negative and invert some harmonics by \(\pi\). Julius O. Smith's filter text states that
a real zero-phase impulse response is even and that a nontrivial zero-phase streaming filter cannot
be causal: [Filters Preserving Phase](https://www.dsprelated.com/freebooks/filters/Filters_Preserving_Phase.html).

## Architecture comparison

| Architecture | Procedural realtime | Filter-like range | Relative harmonic phase | Pitch adaptation | Enabled cost per lane | Honest VA status |
|---|---:|---|---|---|---|---|
| Braids/Plaits direct response | Yes | LP, peak, BP, HP; continuous morph possible | Newly constructed; not source-preserving | Clamp/interpolate internal carrier and formant increments | Fixed small state; several sine/shape evaluations | Digital oscillator inside a VA synth, not an analog VCF |
| Classic PD / vector phaseshaping | Yes | LP-like brightness, pulses, formants | Deliberately changed | Limit warp slope/depth from \(\Delta\phi\) | One phase warp plus source evaluation | Established digital synthesis; VA-compatible when calibrated to an analog oscillator |
| Closed-form DSF | Yes | Exponential LP/HP sections; multiple sections form BP/peaks | Explicit and phase-aligned for positive cosine coefficients | Bound highest partial from \(f_0\) and modulation range | O(sections), independent of partial count; trig and divide | Procedural spectral oscillator, not inherently an analog circuit model |
| PAF / synchronized wavepacket | Yes | Tunable resonant peak/formant and bandwidth | Phase-aligned when generators share a phasor | Express center/bandwidth relative to \(f_0\); update ratios at cycle boundaries | Fixed: waveshaper plus two cosine carriers | Digital oscillator/resonator model, not analog VCF emulation |
| LP-BLIT with Hammerich pulse | Yes | Smooth LP cutoff and roll-off | Linear phase; harmonics remain aligned | Published algorithm limits \(N_h,\alpha\) from current \(f_0\) | O(overlapping pulses × polynomial order); sine/sinh limiting | Strong VA-source algorithm, but no analog filter circuit |
| Circuit-modeled wavefolder | Yes | Harmonic expansion/folding, not LP/BP/HP attenuation | Changed by nonlinearity | Oversampling/AA need increases with drive and input pitch | Nonlinear cells plus BLAMP/ADAA and usually oversampling | Genuinely VA because it models analog circuitry |
| Symmetric cycle-domain taps | Yes | Small nonnegative comb/selector family | Exactly preserved only where \(H_n\ge0\) | Offset must respect lane harmonic ceiling | O(taps × full source evaluations) | Novel digital shaper unless fitted to an analog target |
| Vital Fourier/wave-buffer morph | No under KURV's constraints | General phase-retaining LP/HP and other masks | Retained bin phases preserved | Truncates bins from phase increment | FFT/buffer rebuild amortized by crossfade | Wavetable spectral processing, not procedural VA oscillator math |

## 1. Direct filter-response waveform synthesis

### Mutable Braids: exact shipped LP/peak/BP/HP precedent

The official Braids manual describes `ZLPF`, `ZPKF`, `ZBPF`, and `ZHPF` as directly
synthesizing the time-domain response of those filter classes to classic waveforms, rather than
generating a waveform and then filtering it:
[Braids official manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/braids/manual/).

The pinned implementation is concrete. `RenderDigitalFilter` maintains a source phase, a
modulator phase, a square-modulator phase, a polarity bit, and one integrator. It resets the
modulator phases at half/full carrier cycles, evaluates windowed sine carriers, constructs
saw/triangle windows and pulse components, and selects equations by filter type:
[Braids pinned source, commit `08460a6`](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/braids/digital_oscillator.cc#L328-L408).

This is oscillator state, not a filter history applied to arbitrary input audio. It proves all four
named responses can be delivered by a compact generator. It does **not** preserve the phase of an
arbitrary KURV saw, pulse, triangle, custom curve, or warped shape; it constructs a different
waveform family.

Its modulation strategy is relevant to KURV: the target modulator increment is computed from the
tone parameter, then linearly approached over the output block. That avoids a hard frequency jump
without regenerating a wavetable. The implementation does not establish modern high-quality
aliasing performance by itself; synchronized phase resets and time-varying shape parameters still
need explicit discontinuity correction and/or KURV's oversampling.

### Plaits: interpolation and subsample reset correction

Plaits' `ZOscillator` is a later, more explicit precedent. It interpolates carrier frequency,
formant frequency, carrier shape, and response mode across each block; resets the formant phase at
carrier discontinuities; calculates the exact fractional reset time; and applies a two-sample BLEP
to the reset discontinuity:
[Plaits `z_oscillator.h`, pinned source](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/plaits/dsp/oscillator/z_oscillator.h#L59-L185).
The official Plaits manual describes the auxiliary result as filtered waveforms simulated with
windowed sines and a continuous peaking/LP/BP/HP morph:
[Plaits official manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/plaits/manual/).

This is the closest implementation precedent for KURV's requested control behavior. The KURV
version could use its existing SIMD sine polynomial instead of Mutable's sine lookup and its
existing raw-phase BLEP timing for resets. It would still be a **dedicated response oscillator**,
not a universal transform applied to every existing shape.

### Phase and cost

There is no meaningful “input phase shift” because the response oscillator is the source. Its
harmonic phases are stable and repeatable for a fixed setting, but are set by its carrier/window
equations. The method has fixed O(1) arithmetic and a few scalars of state per lane. That is much
cheaper than a Fourier buffer rebuild and suitable for modulation, but not zero cost—especially at
KURV's maximum unison count.

## 2. Phase distortion and vector phaseshaping

Classic phase distortion feeds a uniformly advancing phasor through a piecewise phase mapping and
then into a sinusoid. The bend point controls brightness and produces an effect similar to moving a
low-pass cutoff. The Vector Phaseshaping paper gives the equations, extends the bend to a 2-D
vector, treats control- and audio-rate modulation, and warns that bright parameter combinations are
more alias-prone:
[Kleimola et al., DAFx 2011](https://www.dafx.de/paper-archive/2011/Papers/55_e.pdf).

Casio's original Japanese filing is direct historical evidence. Its phase-address generator changes
the address rate within each cycle under a harmonic-control signal, explicitly contrasting this
with additive synthesis and variable digital filtering:
[Casio `波形発生方式`, JPS59-111515A](https://patents.google.com/patent/JPS59111515A/ja),
[US family member 4,658,691](https://patents.google.com/patent/US4658691A/en).

Phase shaping is the smallest conceptual addition to KURV because it already exists in
`src/oscillators/va/warp.rs`: `Pwm`, `PhaseBend`, and `Harmonic` alter the sampled phase and carry a
warped phase step into BLEP/BLAMP evaluation. The implementation clamps warp depth as phase step
increases. That protects local phase monotonicity and event width, but it is not a proof that all
modulation products remain under Nyquist.

Advantages:

- fixed storage and bounded O(1) arithmetic;
- natural per-lane pitch dependence because the warp already receives \(\Delta\phi\);
- existing SIMD scalar/4-wide/8-wide paths and current modulation plumbing can be reused;
- an exact bypass is already present.

Limits:

- it is not a general LP/BP/HP response;
- it deliberately changes harmonic phases;
- rapidly moving the warp produces sidebands, and audio-rate nonsynchronous modulation can become
  inharmonic;
- stronger corners or derivative discontinuities need matching antialias correction.

This is the best route for an `Edge` or `Contour` control when strict VA identity and minimum CPU
matter more than filter labels.

## 3. Closed-form spectra: DSF and `gbuzz`

Discrete Summation Formulae replace an explicit loop over partials with a closed form. The primary
Lazzaro/Wawrzynek chapter starts from a low-pass-shaped series

\[
B(p,a)=S\sum_{k=0}^{H} a^k\cos(2\pi(k+1)p),
\]

where \(0<a<1\) gives an exponential high-frequency decay. It then derives an exact rational
closed form. For static \(a\), its stated per-sample cost is a phasor advance, two sine/cosine
calculations, nine multiplies, five additions, and one divide—independent of \(H\). It also identifies
the singular numerical region near \(a=1\), sets \(H\) from the maximum possible fundamental to
avoid aliasing, and constructs piecewise spectra by combining sections with different lowest and
highest harmonics:
[Lazzaro and Wawrzynek, “Subtractive Synthesis without Filters”](https://john-lazzaro.github.io/sa/pubs/pdf/buzz.pdf).
The underlying method originates in Moorer's primary JAES paper:
[Moorer 1976, AES E-Library](https://aes.org/publications/elibrary-page/?id=2590).

Csound's `gbuzz` is a mature open implementation. Its API exposes lowest harmonic, number of
harmonics, and a geometric-series multiplier, and its manual permits performance-time modulation:
[Csound `gbuzz` manual](https://csound.com/docs/manual/gbuzz.html). The pinned C implementation
caches powers and normalization when the multiplier or partial count changes and evaluates the
closed numerator/denominator per sample:
[Csound pinned source, commit `ded5d15`](https://github.com/csound/csound/blob/ded5d15dece77539c04fbaaa160144df090771e2/OOps/ugens4.c#L147-L240).

DSF is phase-predictable: with positive geometric coefficients and cosine terms, generated partials
are phase-aligned to the phasor. It is also genuinely procedural and does not require a partial bank
or FFT. But it generates its own waveform family. It cannot preserve the phases or detailed
spectrum of an arbitrary KURV shape.

Pitch and modulation constraints:

- \(H\) must remain below the lane-specific Nyquist harmonic. Choosing it once from the maximum
  possible pitch is safe but wastes harmonics on lower notes.
- Changing integer \(H\) directly changes the waveform and can click. A real-time adaptive version
  must interpolate the boundary partial or crossfade adjacent \(H\) values.
- Modulating \(a\) is continuous but requires recalculation of its powers/normalization and still
  creates sidebands. The \(a\approx1\) singular region needs a numerically stable alternate form or
  an explicit exclusion zone.
- A BP, HP, or resonant profile needs multiple DSF sections. Cost is O(number of sections), not
  O(number of harmonics), but each section brings more trig/rational work.

For KURV this is a valuable reference and a possible dedicated `Spectrum` oscillator, but it is a
worse first implementation than the direct-response family because it departs from the current
classic waveform identity and is expensive across 64 unison lanes.

## 4. Direct resonance: phase-aligned formant synthesis

Phase-Aligned Formant (PAF) synthesis directly constructs a spectral peak without a resonant filter.
Miller Puckette's primary text gives a synchronized wavepacket formula: a Gaussian or Cauchy-shaped
modulator controls bandwidth, while two weighted cosine carriers place the peak between adjacent
harmonics. Fundamental, center frequency, and bandwidth are the user parameters. The center ratio
is sampled at fundamental phase wrap so it never momentarily points to an incorrect harmonic, while
amplitude correction is ramped. Because the expansions and carriers are cosines, PAF generators
sharing one phasor combine with aligned partial phases:
[Puckette, PAF generator](https://msp.ucsd.edu/techniques/v0.11/book-html/node96.html).

This is a strong peak/BP/formant complement to DSF's spectral slope. It remains a new oscillator
family, not a filter over KURV's saw/pulse/custom output. Its wavepacket bandwidth must be restricted
so sidebands above Nyquist are not generated, and cycle-synchronous parameter latching limits the
highest useful modulation bandwidth. It is fixed-cost, but several cosine/waveshaper evaluations per
sample still matter at maximum unison.

## 5. LP-BLIT: phase-aligned low-pass synthesis

Kraft and Zölzer's LP-BLIT is the strongest published answer to “can the oscillator directly have a
low-pass spectrum without a recursive filter?” It replaces the sinc pulse of a bandlimited impulse
train with a Hammerich pulse

\[
h_H(n)=\alpha\,\frac{\sin(\Omega_c n)}{\sinh(\alpha\Omega_c n)},
\]

whose cutoff \(f_c=N_h f_0\) and stop-band roll-off \(\alpha\) are independently controllable. The
paper states that the closed form can be evaluated per sample without a wavetable, permits immediate
parameter modulation, and corresponds to linear-phase FIR low-pass shaping so the harmonics remain
aligned:
[Kraft and Zölzer, “LP-BLIT,” DAFx 2017](https://dafx.de/paper-archive/2017/papers/DAFx17_paper_59.pdf).

It is explicitly pitch-adaptive: the paper continuously restricts \(N_h\) and \(\alpha\) from the
current fundamental to keep aliasing low during a 20 Hz–7 kHz sweep. This is precisely the kind of
Nyquist adaptation KURV needs per detuned lane.

The cost boundary is equally explicit:

- the ideal pulse is infinite, so a practical implementation truncates/windows it;
- the number of overlapping pulses trades CPU against spectral accuracy;
- sine and hyperbolic sine are the limiting operations;
- the authors propose Taylor/Horner approximations and report seven terms as sufficient for errors
  below -100 dB in their experiment;
- generating saw, square, and triangle variants adds one or two leaky integrations.

Most importantly, the published algorithm is **low-pass only**. Its conclusion leaves high-pass,
band-pass, and resonant-low-pass pulse shapes as future work. It is not evidence of a ready-made
procedural Vital replacement. For KURV it should be considered only as a dedicated phase-aligned LP
oscillator after measured profiling—not as the universal default shaper.

## 6. Analog-modeled oscillator waveshapers

Wavefolders and oscillator edge/slope circuits prove that true analog harmonic shaping can live in
the oscillator/timbre path. Yamaha AN-X exposes per-oscillator Wave Shaper and Self Sync, while its
separate Modifier contains a per-note Wave Folder controlled by envelopes, LFOs, velocity, and
polyphonic aftertouch:
[Yamaha AN-X oscillator edit](https://manual.yamaha.com/mi/synth/montage_m/en/om02screenparameters0180.html),
[Yamaha AN-X Modifier/Wave Folder](https://manual.yamaha.com/mi/synth/montage_m/en/om02screenparameters0170.html).

The Buchla 259 wavefolder paper derives memoryless mappings from the actual op-amp folding stages,
validates them against SPICE, and calls the result a virtual-analog model. It also demonstrates the
price of nonlinear spectral expansion: the proposed high-quality implementation combines BLAMP
correction with 8× oversampling:
[Esqueda et al., “Virtual Analog Buchla 259 Wavefolder,” DAFx 2017](https://www.dafx.de/paper-archive/2017/papers/DAFx17_paper_82.pdf).

This route is the most defensible “true VA” oscillator modifier, but it solves the opposite spectral
problem: it adds and folds harmonics. It does not supply LP/BP/HP attenuation or source-phase
preservation. It should be a separate VA timbre feature, not presented as the requested adaptive
filter replacement.

## 7. Symmetric zero-phase cycle-domain kernels

Because a procedural oscillator owns the entire periodic function, it can evaluate future-looking
cycle positions without waiting for future host-audio samples:

\[
y(\phi)=\sum_{m=-M}^{M} a_m x(\phi+m\delta).
\]

For real symmetric weights \(a_m=a_{-m}\), harmonic \(n\) is multiplied by

\[
H_n=a_0+2\sum_{m=1}^{M} a_m\cos(2\pi n m\delta),
\]

which is real. This proves no filter history or output latency is necessary. It does **not** prove
phase preservation: every \(H_n<0\) flips that harmonic by \(\pi\). Strict preservation requires
\(H_n\ge0\) for every audible retained harmonic.

The smallest always-nonnegative example is

\[
y(\phi)=\tfrac12x(\phi)+\tfrac14x(\phi-\delta)+\tfrac14x(\phi+\delta),
\qquad H_n=\cos^2(\pi n\delta).
\]

It is phase-preserving, but it is a repeated harmonic comb, not a monotonic low-pass except before
its first zero. The complement has \(\sin^2(\pi n\delta)\), also a comb. Half-cycle sum/difference
can exactly select even/odd harmonics. These are useful, honest modes; they are not general
LP/BP/HP filters.

For arbitrary KURV shapes each tap must evaluate the **whole effective source function** at its
shifted raw phase, including phase warp and correct BLEP/BLAMP event timing. Three taps therefore
approach three source evaluations per sample per lane. More taps improve envelope control but grow
linearly in cost. A finite cosine polynomial cannot deliver an arbitrary sharp monotonic response
without lobes; a universal exact response requires explicit Fourier coefficients, an integral, or a
special closed form for the selected waveform family.

No primary source in this survey implements this exact cycle-kernel construction as an analog VCO
filter. It is mathematically valid but should be named `Cycle Shape`, `Harmonic Comb`, or similar,
not “analog zero-phase filter.”

## 8. Vital's actual architecture and why it is not free

Vital is the correct reference for the requested *behavior* and the wrong implementation template
for KURV's no-FFT/no-wavetable constraint. At commit `636ca0e`, Vital separates every wavetable
bin into amplitude and normalized complex direction. Its LP/HP morph zeros or attenuates bins while
reusing that normalized complex direction, then inverse-transforms the result. Retained bins keep
their original phase:
[Vital `spectral_morph.h`, pinned lines 243–305](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/spectral_morph.h#L243-L305).

Vital derives the last legal harmonic from each phase increment, fills a Fourier buffer, transforms
it into a waveform, and can share the buffer across unison voices unless spectral spread requires
separate results:
[Vital `synth_oscillator.cpp`, pinned lines 736–864](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L736-L864).
It sets the wavetable fade to 7 ms, crossfades regenerated buffers, and refreshes them at the end of
the fade rather than doing an FFT per output sample:
[Vital 7 ms fade constant, pinned source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L40-L50),
[Vital buffer refresh/crossfade, pinned lines 1381–1428](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1381-L1428).

That architecture is efficient, not free. It spends memory, Fourier transforms, buffer generation,
and crossfade arithmetic to obtain response generality and phase retention. Removing those
mechanisms also removes the information needed for the same universal operation.

## What “adaptive” should mean in KURV

The control should have an explicit musical coordinate. For a per-lane normalized cutoff, use a
continuous target harmonic rather than a raw Hz cutoff:

\[
N_c = 2^{u\log_2(\max(1,N_{\max}))},\qquad u\in[0,1],
\]

then \(f_c=N_c f_0\). This makes the timbre broadly key-tracking while the top of the control always
reaches the lane's legal spectrum. An absolute-Hz option can be expressed as \(N_c=f_c/f_0\), but it
will intentionally sound different across notes.

For safe realtime modulation:

- calculate/clamp against each lane's current \(\Delta\phi\), including detune, pitch bend, glide,
  jitter, and oscillator transpose;
- interpolate continuous frequencies/shape controls sample by sample or across the existing small
  render block;
- never jump an integer partial count—crossfade or use a fractional boundary harmonic;
- evaluate a discontinuity correction at its fractional reset time when a response oscillator
  resynchronizes an internal carrier;
- keep a conservative Nyquist guard for modulation sidebands, not merely the stationary harmonic
  ceiling;
- retain the exact current oscillator function when the amount is zero;
- do no allocation, locking, logging, I/O, coefficient-vector resize, or table rebuild on the audio
  thread.

Audio-rate modulation is a different signal, not a sequence of static spectra. Even a perfectly
phase-aligned stationary response creates sidebands when its cutoff or shape changes. “No aliasing
with arbitrary modulation” is not a truthful contract for any of these methods.

## Current KURV seams

The current tree already has the right high-level placement:

- `src/oscillators/va/mod.rs` owns one continuous phase accumulator per unison lane and dispatches
  procedural scalar/4-wide/8-wide generation.
- `src/oscillators/va/render.rs:57-150` advances raw phase, applies optional phase warp, and then
  evaluates the canonical or custom source. A direct-response oscillator would belong beside these
  evaluators, before stereo/unison accumulation.
- `src/oscillators/va/warp.rs:8-25` defines the existing `None`, `Pwm`, `PhaseBend`, and `Harmonic`
  oscillator-domain modes. This is the smallest seam for Edge/Contour behavior, but not for a
  universal filter.
- `src/voices/voice.rs:1301-1548` selects exact waveform, warped, and custom render paths in 8-wide,
  4-wide, and scalar lanes. The bypass criterion in `VoiceSettings::legacy_primary_fast_path`
  already shows how a new disabled feature must leave the current fast route untouched.
- `src/voices/oscillator_bank.rs:413-492` already interpolates oscillator configuration without
  allocation, including a dedicated phase-warp transition. A response control should reuse that
  state/update pattern.
- `src/lib.rs:1618-1638` runs synthesis at the oversampled DSP rate. This is an available safety net,
  but it does not excuse missing oscillator-specific discontinuity control.
- `src/filters/svf.rs` and `src/voices/voice.rs:6074-6146` are the separate ordered TPT-SVF path. Those
  filters process the accumulated stereo signal and have integrator history. They are not the seam
  for this oscillator-level feature.

A direct-response implementation needs small extra per-lane oscillator state. A phase-warp mode
needs no new history. DSF needs parameter history/crossfade state but not a harmonic array. LP-BLIT
needs a bounded pulse-overlap structure and is the worst fit for KURV's 64-lane hot path.

## Ranked recommendation

There is one unavoidable product fork. If continuous LP/peak/BP/HP behavior and low CPU rank first,
choose recommendation 1. If retaining the phase of explicitly generated partials is non-negotiable,
choose a restricted DSF/PAF family instead; it will not preserve an arbitrary pre-existing KURV
waveform or reproduce arbitrary analog-filter curves. No surveyed architecture supplies both sides
of that fork under KURV's constraints.

### 1. Implement a dedicated `Osc Response` family modeled on Plaits/Braids

This is the best match for the requested musical result. It is proven, procedural, fixed-memory,
modulatable, and genuinely oscillator-level. Use one continuous `Response` coordinate across LP →
peak → BP → HP and a `Tone` coordinate for pitch-relative cutoff/formant frequency. Reuse KURV's
SIMD sine polynomial, raw-phase reset timing, BLEP correction, oversampled DSP rate, and existing
block-constant/modulated dispatch.

Do not run it as a transform over arbitrary KURV shapes. Treat it as a dedicated waveform family
with its own stable phase definition. At amount/type bypass, call the existing canonical evaluator
directly so disabled cost is only the dispatch condition.

Marketing/UX name: **Osc Response**, **Contour**, or **Harmonic Shape**. Do not call it a
zero-phase analog filter.

### 2. Extend existing warp with an analog-grounded `Edge` mode

This is the cheapest route and the strongest fit to strict procedural VA. It should be calibrated to
known analog VCO slope/edge behavior or a documented VA oscillator control. It can produce a
convincing dark-to-bright sweep with minimal new arithmetic, but it must not expose LP/BP/HP labels
or promise resonance.

### 3. Add PAF only if a precise peak/formant mode is still missing

PAF provides a directly controlled center and bandwidth with predictable phase alignment and no
recursive filter. It is a better dedicated resonant source than forcing resonance into a short
cycle kernel. Add it only after measuring the direct-response family, because it duplicates much of
the peak/BP territory and costs multiple trigonometric/waveshaping evaluations per lane.

### 4. Prototype LP-BLIT only behind a measured quality/cost gate

LP-BLIT is the research-backed choice if phase-aligned low-pass synthesis becomes a hard product
requirement. It is not the first implementation for a 64-unison oscillator because the published
cost/accuracy tradeoff is substantial and HP/BP/resonant shapes remain unsolved in that paper.

### 5. Keep DSF and symmetric kernels as specialist oscillators, not the default filter system

DSF is excellent for explicit exponential spectral families and symmetric kernels are excellent for
odd/even/comb modes. Neither is the shortest honest path to a uniform Vital-like LP/BP/HP control
over KURV's existing shapes.

### 6. Reject a hidden Vital clone unless the product constraint changes

If the requirement becomes exact arbitrary phase-preserving spectral masks, adopt the Fourier/
wave-buffer architecture openly. Do not disguise it as procedural VA or recreate a partial FFT
system behind the oscillator while retaining a “no FFT/wavetable” claim.

## Final engineering boundary

KURV can have all of these at oscillator level:

- filter-like LP/peak/BP/HP motion;
- pitch-aware Nyquist limiting per unison lane;
- bounded fixed-memory realtime processing;
- fast modulation with interpolated controls;
- an exact zero-additional-work bypass;
- a procedural VA core.

It cannot simultaneously promise, for every arbitrary source shape:

- exact LP/BP/HP/resonant magnitude responses;
- exact preservation of every retained harmonic phase;
- no FFT, table, additive representation, or multi-tap evaluation;
- literally zero enabled CPU cost;
- arbitrary audio-rate modulation without aliasing.

The direct-response oscillator is the smallest established architecture that satisfies the user's
actual audible goal without mislabeling a spectral comb or hiding a wavetable engine.

## Primary-source index

- [Yamaha MONTAGE M AN-X engine, English](https://manual.yamaha.com/mi/synth/montage_m/en/om01basicoperation0350.html)
- [Yamaha MONTAGE M AN-X engine, Japanese original](https://manual.yamaha.com/mi/synth/montage_m/ja/om01basicoperation0350.html)
- [Yamaha AN-X oscillator edit](https://manual.yamaha.com/mi/synth/montage_m/en/om02screenparameters0180.html)
- [Yamaha AN-X Modifier/Wave Folder](https://manual.yamaha.com/mi/synth/montage_m/en/om02screenparameters0170.html)
- [Casio original Japanese phase-distortion patent, JPS59-111515A](https://patents.google.com/patent/JPS59111515A/ja)
- [Casio phase-distortion patent, US 4,658,691](https://patents.google.com/patent/US4658691A/en)
- [Vector Phaseshaping Synthesis, DAFx 2011](https://www.dafx.de/paper-archive/2011/Papers/55_e.pdf)
- [Braids official manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/braids/manual/)
- [Braids `RenderDigitalFilter`, pinned source](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/braids/digital_oscillator.cc#L328-L408)
- [Plaits official manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/plaits/manual/)
- [Plaits `ZOscillator`, pinned source](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/plaits/dsp/oscillator/z_oscillator.h#L59-L185)
- [Moorer, “The Synthesis of Complex Audio Spectra by Means of Discrete Summation Formulae,” 1976](https://aes.org/publications/elibrary-page/?id=2590)
- [Lazzaro and Wawrzynek, “Subtractive Synthesis without Filters”](https://john-lazzaro.github.io/sa/pubs/pdf/buzz.pdf)
- [Csound `gbuzz` manual](https://csound.com/docs/manual/gbuzz.html)
- [Csound `gbuzz`, pinned source](https://github.com/csound/csound/blob/ded5d15dece77539c04fbaaa160144df090771e2/OOps/ugens4.c#L147-L240)
- [Puckette, Phase-Aligned Formant generator](https://msp.ucsd.edu/techniques/v0.11/book-html/node96.html)
- [Kraft and Zölzer, “LP-BLIT,” DAFx 2017](https://dafx.de/paper-archive/2017/papers/DAFx17_paper_59.pdf)
- [IBM pitch-adaptive upper-harmonic reduction patent, EP 0 484 048](https://patents.google.com/patent/EP0484048A2/en)
- [Esqueda et al., “Virtual Analog Buchla 259 Wavefolder,” DAFx 2017](https://www.dafx.de/paper-archive/2017/papers/DAFx17_paper_82.pdf)
- [Julius O. Smith, Filters Preserving Phase](https://www.dsprelated.com/freebooks/filters/Filters_Preserving_Phase.html)
- [Vital LP/HP spectral morph, pinned source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/spectral_morph.h#L243-L305)
- [Vital Fourier buffer generation and pitch limiting, pinned source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L736-L864)
- [Vital 7 ms buffer-fade constant, pinned source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L40-L50)
- [Vital buffer refresh and crossfade, pinned source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1381-L1428)
