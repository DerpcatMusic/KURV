# KURV oscillator-domain filtering research

Date: 2026-08-31

Inspected KURV revision: [1f8ad68e7c7ef03ad58eca88e6ce738364f06233](https://github.com/DerpcatMusic/KURV/commit/1f8ad68e7c7ef03ad58eca88e6ce738364f06233), with the current dirty checkout preserved. External source links pin the exact revisions inspected.

## Bitwig multi-output note routing

The reported Cardinal behavior is a host-routing boundary, not a missing KURV audio bus. Bitwig's multi-out chains expose a multichannel plug-in's individual **audio** channels; the note stream continues through the instrument's parent device chain. Consequently Cardinal immediately after KURV receives notes, while Cardinal inside `Output 1/2` receives that bus's audio but no notes. Bitwig documents multi-out chains as audio sources in its [VST plug-in guide](https://www.bitwig.com/userguide/latest/vst_plug-ins/) and provides [Note Receiver](https://www.bitwig.com/userguide/latest/routing/) to import notes into another processing path.

KURV cannot attach a CLAP/VST3 event stream to one stereo output bus: audio and event ports are separate plugin-wide port families. Advertising MIDI output and copying input events would not turn Bitwig's audio-only child chain into a note chain, and risks duplicating notes on the parent chain. The correct Bitwig patch is to keep Cardinal on the desired KURV output and explicitly feed that path from the track's note source with Note Receiver/Note Source.

## Decision brief

- **Name the distinction by domain:** `OSCILLATOR` or `SPECTRAL` filtering changes one VA source's harmonic coefficients before playback and unison accumulation. KURV's existing `SVF`, `PHASER`, and `SCREAM` are `AUDIO/VOICE` processors in the ordered generator stack. Do not call that whole second family “minimum phase”: Phaser is all-pass-based and Scream is nonlinear.
- **The first shipped slice is `RATIO BRICKWALL`:** an ordered harmonic low-pass/high-pass mask. `HIGH R×` nulls canonical VA partials `k <= R`, `LOW R×` nulls partials `k > R`, and `0×` bypasses either direction. The upper cap still derives from each oscillator lane's actual phase step.
- **Use the cheapest exact representation per operation:** the implemented canonical brickwall uses immutable harmonic-prefix cycle tables and subtracts two prefix lookups. Phase-reset Sync remains a separate realtime phase operation.
- **No FFT, artifact rebuild, mip selection, or crossfade belongs in the live Ratio path:** the selected product contract is direct oscillator-source evaluation with audio-rate cutoff changes. Earlier compiler alternatives below remain research comparisons, not the implementation direction.
- **Do not normalize each result:** automatic peak normalization turns closing a spectral filter into automatic gain riding and becomes ill-conditioned near silence. Preserve source gain; consider only restrained, bounded energy compensation later.
- **Expose domain in the filter type while executing at the source seam:** `RATIO BRICKWALL` is a `FilterMode` so it lives beside the existing filter types, but the ordered renderer lowers it into every preceding oscillator rather than processing the mixed stereo bus. A future general phase/spectral chain may still justify a dedicated `WARP` module.

## 1. What the operation actually is

For a real periodic source with discrete Fourier coefficients `X[k]`, oscillator-domain filtering is:

\[
Y[k] = G(k; \theta) X[k], \qquad 0 \le k \le N/2,
\]

followed by Hermitian mirroring and an inverse DFT. `G` is a harmonic-number response, not a time-recursive filter. For the first modes:

- **Harmonic LP:** `G` is one below a harmonic cutoff and zero above it, with a fractional or short soft boundary.
- **Harmonic HP:** the complement, with an explicit DC policy and a short soft boundary.
- **Comb:** `G` repeats across harmonic number; period/offset choose the retained or attenuated harmonic families and amount blends toward unity.

Multiplying both the real and imaginary parts by the same nonnegative gain preserves every retained partial's source phase. A signed comb gain can additionally flip selected partials by π. The output has no causal filter state, transient tail, feedback, self-oscillation, or conventional IIR group delay. A sharp mask can still cause Gibbs-like ripples within the reconstructed cycle; “stateless” does not mean “ring-free waveform.” Spectral resonance, if added later, can only amplify partials that exist in the source. It is not feedback resonance and cannot self-oscillate.

This operation is also not a minimum-phase filter. A minimum-phase response requires a causal phase response tied to its magnitude response; preserving the source's complex phase is periodic waveform reconstruction instead. Julius O. Smith's [minimum-phase filter design summary](https://www.dsprelated.com/freebooks/sasp/Minimum_Phase_Filter_Design.html) gives the relevant magnitude/phase construction distinction.

## 2. What Vital does

Vital's public GPLv3 snapshot exposes `Low Pass` and `High Pass` among spectral morphs such as formant/harmonic scale, smear, phase disperse, and skew ([mode enum](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.h#L100-L114)). Its source cycle is 2,048 samples ([`WaveFrame` size](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/lookups/wave_frame.h#L25-L32)).

The runtime LP/HP algorithms are unusually simple and musically useful:

- LP copies complex bins below an exponential harmonic cutoff, zeros bins above it, and fades one boundary harmonic.
- HP zeros below the cutoff, copies above it, and fades one boundary harmonic.
- Both preserve the stored normalized complex phase and inverse-transform the result into a wrapped periodic buffer.

The exact loops are in Vital's [`lowPassMorph` and `highPassMorph`](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/spectral_morph.h#L243-L305); the [inverse transform and periodic padding](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/spectral_morph.h#L35-L51) follow. Vital computes a pitch-dependent last safe harmonic before each rebuild ([source](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L768-L800)).

Vital does not execute an FFT for every output sample. It rebuilds alternating preallocated buffers and crossfades over roughly 7 ms ([update loop](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1396-L1428)). It shares a generated buffer across unison lanes unless spectral-unison or frame-spread settings require distinct results ([sharing path](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L821-L862)). Its fixed buffers and transform object are [preallocated in the oscillator](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.h#L291-L315).

Therefore Vital demonstrates a bounded-control-rate, crossfaded spectral rebuilder—not a per-sample causal filter. Its approximate rebuild cost is `O(H) + O(N log N)` for each distinct table generation, followed by cheap table playback. Per-unison spectral spread can multiply that regeneration cost.

Vital's wavetable editor has a separate offline frequency-filter modifier. It multiplies every complex bin by one real response, inverse-transforms, and optionally normalizes ([render path](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/wavetable/frequency_filter_modifier.cpp#L71-L80)); its four shapes are LP, BP, HP, and Comb ([gain functions](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/wavetable/frequency_filter_modifier.cpp#L96-L113)). This supports Comb as a genuine oscillator-domain operation, but it should not be confused with Vital's runtime spectral-morph list.

## 3. Other source implementations

### ZynAddSubFX and Yoshimi

ZynAddSubFX is the clearest older precedent. Its oscillator generator offers smooth and brick LP/HP/BP/notch responses, cosine/sine harmonic masks, a shelf, one-harmonic boost, and a resonant-LP-shaped response ([14 gain functions](https://github.com/zynaddsubfx/zynaddsubfx/blob/3ab608c432996ba4d582176572c0b0f82328c825/src/Synth/OscilGen.cpp#L1841-L2005)). `oscilfilter()` multiplies each complex FFT bin by the selected scalar gain and then normalizes ([source](https://github.com/zynaddsubfx/zynaddsubfx/blob/3ab608c432996ba4d582176572c0b0f82328c825/src/Synth/OscilGen.cpp#L646-L662)). Crucially, Zyn exposes whether filtering happens before or after waveshaping ([ordering](https://github.com/zynaddsubfx/zynaddsubfx/blob/3ab608c432996ba4d582176572c0b0f82328c825/src/Synth/OscilGen.cpp#L974-L988)), proving that spectral-operator order is audible rather than an implementation detail.

The maintained Yoshimi descendant documents the same musician-facing model: its left view shows the complete oscillator after harmonic effects, its editor exposes 128 harmonics, and a switch moves the harmonic filter before waveshaping ([Yoshimi manual](https://github.com/Yoshimi/yoshimi/blob/fb3e82c67b313d6905d140d51eccc52d5ad727ba/doc/yoshimi_user_guide/wave/wave.html#L17-L54)). This is useful corroboration that users can understand “filter the harmonics of this oscillator” as a source operation.

### Less obvious alternatives

- [Flow](https://github.com/eclab/flow/blob/460fb1eb1d2c07265a986a7a20c4e040cfc1e916/flow/modules/Filter.java#L79-L218) evaluates an analogue-like multimode magnitude response independently at each additive partial. Its “resonance” therefore emphasizes existing partials rather than creating feedback oscillation. Flow's [renderer culls additive partials at Nyquist](https://github.com/eclab/flow/blob/460fb1eb1d2c07265a986a7a20c4e040cfc1e916/flow/Output.java#L607-L710).
- The Rust demoscene synthesizer [Oidos](https://github.com/askeksa/Oidos/blob/256132bb4fa00de8b5d012cd1becc4296539d8cf/synth/src/oidos_generate.rs#L427-L446) applies independently moving high/low gain edges while directly summing partials. It is a compact proof of concept, not a KURV-ready antialiasing reference; the inspected loop has no explicit Nyquist rejection.
- Mutable Instruments Plaits uses a fixed-size [Chebyshev additive harmonic oscillator](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/plaits/dsp/oscillator/harmonic_oscillator.h#L41-L115) with smoothed partial amplitudes and [pitch-aware high-harmonic attenuation](https://github.com/pichenettes/eurorack/blob/08460a69a7e1f7a81c5a2abcc7189c9a6b7208d4/plaits/dsp/engine/additive_engine.cc#L55-L149). Direct additive synthesis permits fast coefficient modulation but costs `O(partials × samples)` and is a poor default for KURV's dense unison.

These sources converge on two practical architectures: rebuild a periodic table at bounded control rate, or render a bounded number of partials directly. None makes an STFT over already-generated oscillator audio necessary.

## 4. Procedural VA architecture: no authored wavetable required

A procedural VA oscillator still has a periodic cycle. If its parameters are fixed over one master period, its Fourier coefficients are

\[
X[k]=\int_0^1 x(\phi)e^{-j2\pi k\phi}\,d\phi.
\]

Those coefficients can be known analytically, accumulated directly, or obtained by sampling the procedure and transforming one cycle. An authored wavetable is therefore not a prerequisite. A generated table is merely a compiled cache of the procedure, just as machine code is not source code.

KURV's current canonical path is not coefficient-based: it evaluates phase functions and applies spline BLEP/BLAMP corrections to discontinuities ([saw example](../../src/oscillators/va/antialias.rs#L186-L227), [renderer](../../src/oscillators/va/render.rs#L57-L108)). There is no harmonic array in that hot path to multiply. Oscillator-domain filtering consequently needs either another representation or a deliberately narrow closed-form fast path.

### Cutoff and slope, exactly

For oscillator `i`, unison lane `l`, and harmonic `k`, let

\[
f_{i,l,k}=f_{note}\,2^{(transpose_i+cents_i/100+detune_{i,l}+pitchMod_i)/12}\,k.
\]

The user's `+5`-semitone example is therefore correct: the local fundamental is multiplied by `2^(5/12)`, and an absolute-Hz response evaluates every partial at that shifted frequency. There are two honest coordinate systems:

- **Harmonic-relative:** evaluate `G(k/K)`. The timbre keytracks and the same artifact can be shared across pitches and unison lanes.
- **Absolute Hz:** evaluate `G(f_partial/fc)`, equivalently `K_lane=fc/fundamental_lane`. This really is pitch-aware, but detuned lanes have different `K` and cannot all share one exact filtered artifact.

A `S` dB/octave asymptotic magnitude slope is logarithmic in frequency, not a fixed dB decrement per harmonic. With `p=S/6.0206`, simple piecewise masks are

\[
G_{HP}[k]=\min(1,(k/K)^p),\qquad
G_{LP}[k]=\min(1,(K/k)^p).
\]

They are 0 dB at `K`. If the control is meant to behave like an order-`n` Butterworth magnitude with a -3 dB cutoff, use

\[
G_{LP}[k]=\frac{1}{\sqrt{1+(k/K)^{2n}}},\qquad
G_{HP}[k]=\frac{1}{\sqrt{1+(K/k)^{2n}}},
\]

where an integer order `n=2` approaches 12 dB/octave and `n=4` approaches 24 dB/octave. In an oscillator magnitude mask `n` may also be continuous; it then describes a curve, not a realizable analogue filter order. A brick wall is the limiting step mask. The safe final harmonic is

\[
k_{max}=\left\lfloor\frac{f_s/2-B_{mod}}{f_{i,l}}\right\rfloor,
\]

where `Bmod` reserves headroom for supported gain/phase modulation sidebands.

Multiplying `X[k]` by a real nonnegative `G[k]` preserves the source phase. Multiplying by a complex `H[k]` can reproduce the steady-state magnitude and phase samples of a causal filter, but the reconstructed cycle still has no causal state, attack transient, feedback, or self-oscillation. “Magnitude only” and “minimum phase” are choices about coefficient phase; neither changes the backend cost.

### Backend choices

| Backend | Exact capability | Audio cost | Memory and modulation | KURV fit |
|---|---|---:|---|---|
| Direct additive bank | Arbitrary gain, phase, and partial position for every retained sinusoid | `O(V × U × P)` per sample; `P` grows sharply on low notes | `O(P)` coefficients/state per independently shaped source; gains can move per sample, but their modulation creates sidebands | Most general, worst default for dense polyphonic unison. Plaits demonstrates the bounded-small-`P` version. |
| Residual partial correction | Existing source plus only `R[k]=(G[k]-1)X[k]` | `O(V × U × M)` for `M` corrected partials | Audio-rate-capable with smoothed gains; vectorize across lanes or harmonic batches | Excellent for sparse brick/short-knee masks, not a universal spectral engine. |
| DSF / BLIT family | Closed forms for particular harmonic families: finite equal-amplitude sums, geometric envelopes, and integrated versions | Usually `O(V × U)` per sample | Tiny state and immediate modulation; restricted response vocabulary | Viable specialized oscillator, not general MagShift/formant/warp infrastructure. |
| DPW / BLEP / BLAMP | Efficient antialiasing of known waveform discontinuities or derivative discontinuities | `O(V × U)` per sample | Tiny state; handles fast phase and shape movement well | Keep for unaffected VA. These methods suppress aliases but do not expose arbitrary per-harmonic gains. |
| Generated cycle + FFT | Arbitrary ordered periodic phase and coefficient operations, followed by final harmonic mips | Playback `O(V × U)`; rebuild `O(N log N)` per distinct artifact | `O(N × levels × artifacts)`; modulation is bounded-rate table crossfade unless a morph bank is precomputed | Best general fit because KURV already has a 2,048-point FFT/mip compiler. |
| Hybrid | Current BLEP path when no spectral module; generated artifact or sparse correction only when needed | Pays only for the selected path | More backend switching, but avoids taxing every oscillator | Recommended, provided backend transitions are phase-aligned and crossfaded. |

The direct-additive complexity is real even with SIMD. At 48 kHz, a 27.5 Hz fundamental has up to 872 integer harmonics below Nyquist before modulation headroom; multiplying that by voices and unison lanes makes a general bank unattractive. A small fixed harmonic engine such as Plaits is a different cost envelope.

The pasted `less_slow.rs` benchmark is useful as a performance-method reminder, not as the filtering algorithm. Its closure/iterator/coroutine and approximate-`sin` comparisons support measuring small kernels and removing abstraction overhead, but calling even a fast sine once per active harmonic would still make oscillator cost grow with note, cutoff, unison, and polyphony. The shipped path therefore precomputes harmonic prefixes once and performs a fixed number of interpolated table reads per sample; a dedicated KURV microbenchmark is only worth adding after host profiling identifies this kernel as the bottleneck.

#### Computing only affected harmonics

For a base renderer with exact coefficients `X[k]`, the desired cycle can be written

\[
y(\phi)=x(\phi)+\sum_k (G[k]-1)X[k]e^{j2\pi k\phi}.
\]

This gives a useful sparse rule:

- For a brick LP, sum the retained low harmonics when few remain; otherwise subtract the rejected high harmonics from the base.
- For a brick HP, subtract the rejected low harmonics when few are removed; otherwise sum the retained highs.
- For a compact soft knee, correct only the transition plus the fully removed side and choose the cheaper retained/residual form.

This is exact only when the correction coefficients describe the **same** base signal. KURV's spline-BLEP waveform is a quasi-bandlimited approximation, so subtracting ideal analytic saw coefficients from it will not cancel perfectly. A Butterworth mask also never becomes exactly zero or one at finite distance, so “only affected harmonics” becomes a thresholded approximation unless the product explicitly defines finite pass/stop regions. Measure that error before adding a fast path.

#### Closed forms, integration, and LP-BLIT

Discrete summation formulas evaluate certain finite harmonic sums without visiting every partial. A geometric series gives exponentially changing amplitude per **harmonic number**, which is not the same as constant dB/octave. BLIT gives a finite equal-amplitude harmonic family; integrating once multiplies harmonic `k` by `1/(j2πk)` and produces a global -6.02 dB/octave tilt, while integrating twice produces -12.04 dB/octave. Differentiation gives the inverse +6.02 dB/octave tilt and magnifies the high-frequency/alias boundary. These are global tilts, not independently placed cutoff filters.

Kraft and Zölzer's [LP-BLIT paper](https://www.dafx17.eca.ed.ac.uk/papers/DAFx17_paper_59.pdf) is the closest published match to the proposal. It replaces BLIT's sinc pulse with a closed-form Hammerich pulse whose cutoff and stop-band roll-off can both be modulated, synthesizes saw/pulse/triangle by combinations and leaky integration, and requires pitch-dependent parameter limits to keep the stop band below Nyquist. It requires no wavetable and demonstrates real-time low-pass-like oscillator synthesis. The authors explicitly leave high-pass, band-pass, and resonant pulse families as future work, so it is evidence for a strong LP fast path rather than a general spectral-warp architecture. Nam et al.'s [BLIT/DPW/BLEP comparison](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf) likewise treats these as efficient generators for constrained classical-waveform spectra, not arbitrary coefficient editors.

#### Circular curve filters and phase-vector alternatives

Filtering the **cycle coordinate** can be exact oscillator-domain filtering. Circular convolution

\[
y(\phi)=x(\phi)\circledast h(\phi)
\]

is equivalent to `Y[k]=X[k]H[k]` by the [DFT convolution theorem](https://www.dsprelated.com/freebooks/mdft/Fourier_Theorems_DFT.html#Convolution_Theorem). It can be evaluated in several ways:

- FFT the generated cycle, multiply bins, and inverse-transform: general and cheap at compile time.
- Sum phase-shifted copies `Σ w_m x(φ+δ_m)`: no stored table, `O(M)` evaluator calls per sample, with exact response `H[k]=Σ w_m exp(j2πkδ_m)`.
- Repeated circular smoothing/diffusion: a phase-circle heat step gives a Gaussian-like `exp(-c k²)` harmonic response.

A symmetric phase-tap kernel has a real response, so each harmonic keeps its phase or flips by π where the response is negative. A very short kernel normally produces repeated zeros/ripples—a comb or broad smoothing gesture—not a monotonic 12/24 dB/octave cutoff. A wide kernel costs many evaluations; FFT compilation is then the same operation more efficiently. Directly smoothing Bézier/control vectors is only musically filter-like unless the uniform circular kernel and its response are specified; changing curve handles has no waveform-independent cutoff contract.

#### Constant-work filtering of KURV's cubic cycle

There is one stronger shape-domain option than arbitrary control-point smoothing. Let `F` be a periodic antiderivative of the zero-mean cycle `x`. A circular box smoother of width `w` cycles is

\[
B_wx(\phi)=\frac{F(\phi+w/2)-F(\phi-w/2)}{w}.
\]

Its exact harmonic response is `sinc(k w)`, with `sinc(z)=sin(πz)/(πz)`. Repeating the box operation `n` times gives `sinc(k w)^n`: the lobe envelope falls at approximately `6n` dB/octave. The repeated operation can be evaluated from `n+1` phase samples of the `n`th antiderivative using a fixed binomial finite difference, rather than by summing harmonics. KURV already represents a custom cycle as 16 fixed piecewise-cubic segments, so their antiderivative coefficients and cumulative segment areas can be compiled into bounded fixed storage. Runtime work is independent of the number of audible harmonics and vectorizes across unison lanes. The width can be pitch-aware with `w = c_n f0_eff / cutoff_hz`, where `c_n` calibrates the requested -3 dB point; a `+5`-semitone oscillator therefore changes the phase width automatically.

This is a real oscillator-domain linear filter, but it is not a Butterworth impersonation: sinc lobes introduce nulls and sidelobes, `x-B_wx` is its corresponding high-pass, and a brickwall still needs a global kernel. It should be exposed as a distinct `SMOOTH` or `PHASE BLUR` character unless those lobes are explicitly the desired filter contract. The implementation also has hard boundaries:

- The integrated evaluator must describe the same procedural/custom blend and antialias correction as playback; blurring a naive discontinuous curve does not itself guarantee a Nyquist-safe result.
- Very small `w` needs a dry or derivative-series fallback to avoid cancellation in the finite difference.
- A preceding nonlinear phase map or hard-sync reset generally destroys the cheap precompiled antiderivative unless the composed piecewise cycle is rebuilt first.
- MagShift and arbitrary per-bin masks remain spectral operations.

Parker, Zavalishin, and Le Bivic derive the broader principle—analytical convolution through antiderivatives for piecewise-polynomial kernels—in their [continuous-time convolution paper](https://www.dafx.de/paper-archive/2016/dafxpapers/20-DAFx-16_paper_41-PN.pdf). Their target is antialiasing nonlinear waveshapers rather than periodic oscillator filtering, so the application above is a KURV-specific inference from that method and the circular convolution theorem.

### Can phase and spectral warps share one ordered module domain?

They can share the **product and compiler domain**, but not one primitive representation:

- Phase operation: `x(φ) → x(g(φ))`, including a constant-ratio hard-sync cycle relative to master phase.
- Spectral operation: `X[k] → F(X,k)`, including LP/HP, MagShift, and spectral phase changes.

An ordered compiler can alternate `cycle → FFT → spectrum → IFFT → cycle` when the domain changes, while fusing consecutive phase maps and consecutive scalar magnitude masks. This preserves the audible fact that `SYNC → LP` differs from `LP → SYNC`. It is control-rate compilation, not a promise of sample-accurate audio-rate modulation. A true partial-position stretch leaves the integer harmonic grid and still requires an additive/oscillator-bank backend.

For KURV, the minimum general architecture is therefore hybrid:

1. Keep the current SIMD BLEP/BLAMP renderer for oscillators with no source-domain module.
2. Lower each ordered spectral module into one prepared oscillator-source description before the unison lane loop; lanes share the description while retaining their own detuned phase and Nyquist ceiling.
3. Keep the axis harmonic-relative across notes and unison. If absolute-Hz cutoff is later required, it needs per-lane evaluation; do not label a shared harmonic mask “Hz.”
4. Add sparse residual partial correction or LP-BLIT only if measurement shows that genuinely audio-rate LP/HP modulation is worth a specialized path.

This reuses KURV's existing compiler and leaves the fast VA path alone. A new all-purpose additive engine is not justified for `SYNC`, `HARMONIC LP`, and `MAGSHIFT`.

## 5. Fit with KURV's current architecture

KURV already has the low-level per-oscillator seam. Every VA oscillator stores a phase-warp mode and amount in [`OscillatorConfig`](../../src/generators/state.rs#L66-L104); current modes are `None`, `Pwm`, `PhaseBend`, and `Harmonic` ([source](../../src/oscillators/va/warp.rs#L7-L25)). The renderer applies warp before evaluating a custom curve ([eight-lane path](../../src/oscillators/va/render.rs#L110-L145)). That does not require warp controls to remain oscillator-card properties: an ordered stack module can compile into the affected oscillators' low-level render settings.

The useful module rule is:

- `OSC A → WARP → OSC B` applies the warp to A.
- `OSC A → OSC B → WARP` applies it independently to A and B.
- V1 keeps source-domain warps above the first audio-domain Filter/Aux boundary. Reaching backward through an already applied nonlinear or stateful processor would silently reorder non-commuting operations.

The patch compiler can implement this without analysing mixed audio: while walking a group, retain the preceding oscillator slots and append each Warp operation to those slots' fixed-capacity compiled chains. Rendering still evaluates each oscillator and unison lane directly, then sums them exactly where it does now. The module owns identity, automation, modulation, reset, and ordering; the oscillator renderer owns execution.

By contrast, current `FilterMode` is `Svf`, `Phaser`, or `Scream` ([source](../../src/filters/engine/mod.rs#L74-L101)). The voice renderer walks ordered filter modules over accumulated stereo samples ([source](../../src/voices/voice/render.rs#L2393-L2425)). A Warp module therefore cannot execute as another sample processor at that point; its compiled operation must execute inside each affected source renderer.

### Pitch-aware coordinates

For oscillator `i`, unison lane `l`, and harmonic `k`, the actual partial frequency is

\[
f_{i,l,k}=f_{note}\,2^{(transpose_i+cents_i/100+detune_{i,l}+pitchMod_i)/12}\,k.
\]

An oscillator transposed `+5` semitones therefore moves all of its partial locations by `2^(5/12)` before a fixed-Hz response is evaluated. A harmonic-relative response instead evaluates `G(k)` and intentionally follows pitch. Both are valid, but the UI must say whether its axis is `HARMONIC` or `HZ`. Exact fixed-Hz behavior across detuned unison lanes needs per-lane evaluation; harmonic-relative masks can be shared across lanes.

### Current warp audit

The current feature is not an adequate implementation of this module family:

- There is no oscillator `SYNC` variant in `PhaseWarpMode`; only None, PWM, Bend, and Harmonic exist.
- All three shipping modes are fixed sinusoidal phase-displacement formulas, not independently specified musical algorithms.
- KURV's retained reference reports that Bend and Harmonic are mathematically unchanged on a 50%-duty square because both preserve its two discontinuity phases ([finding](../experiments/va-static-phase-warp-reference-2026-08-30.md#representative-48-khz-findings)). It also reports large high-note 1x projection error.
- The compact VA oscillator card exposes neither `phase_warp_mode` nor `phase_warp_amount`; its only direct use of `PhaseWarpAmount` is repurposed as Noise `STEREO` ([source](../../src/editor_generator/oscillator_card.rs#L680-L686)). The waveform preview can display stored warp state, but that is not a complete editing workflow.

The current paths prove only that a phase-coordinate hook exists. They do not prove useful Bend/Harmonic behavior, real hard sync, or a usable warp product.

### MAGSHIFT and spectral warps

`MAGSHIFT` should move the magnitude envelope without moving the oscillator's fundamental grid. With `A[k]=|X[k]|`, a ratio-form shift can be

\[
A'[k]=interp(A[k/r]), \qquad r=2^{shiftSemitones/12},
\]

while output energy remains at `k × f0`. That changes timbre without pitch-shifting the oscillator. Zero-fill outside the source range; wrapping magnitudes would create an unrelated cyclic effect. Retaining each destination bin's original phase is the least surprising V1 phase policy.

A magnitude warp similarly remaps the envelope lookup coordinate, for example `A'[k] = interp(A[k^gamma])`. A true partial-position stretch is different: moving energy to non-harmonic frequencies cannot be represented by one ordinary one-cycle wavetable and needs an additive/oscillator-bank or longer-period representation. Keep `MAG WARP` and `PARTIAL STRETCH` as separate contracts.

KURV also contains a strong but not yet shipping spectral artifact. [`BandlimitedWaveCurve`](../../src/wave_curve/bandlimit.rs#L1-L35) samples one 2,048-point period, performs one FFT, and builds 20 inverse-FFT mip levels capped from 1 through 1,023 harmonics ([compiler](../../src/wave_curve/bandlimit.rs#L55-L108)). Runtime lookup is fixed, lock-free, and allocation-free, and its selector explicitly requires the caller's maximum warp-aware phase derivative ([selection contract](../../src/wave_curve/bandlimit.rs#L110-L133)). The active custom-wave renderer still evaluates `WaveCurveRt`, so this bank is currently an implementation seam/reference, not proof that KURV ships spectral VA playback.

### Memory boundary

One bank is `20 × 2,048 × 4 = 163,840` bytes, or 160 KiB. A VA table permits 16 custom frames ([limit](../../src/oscillators/va/table.rs#L11-L13)) and the generator stack permits 32 oscillators ([limit](../../src/generators/stack.rs#L9-L13)). Materializing a full bank for every possible frame and oscillator would therefore consume about 80 MiB before double-buffering, alignment, or publication overhead:

`160 KiB × 16 × 32 = 80 MiB`.

Do not allocate that grid. Compile only bounded active/selected artifacts, share identical immutable results, and account for the old plus new artifact during a crossfade. A single active bank per maximum oscillator slot is about 5 MiB; double-buffering all 32 would be about 10 MiB. Actual residency must be measured under a defined maximum-polyphony patch before choosing the cache limit.

### Ordering and antialiasing boundary

Filtering a base spectrum and then applying phase warp can regenerate harmonics that the LP removed. KURV's own warp code returns the effective warped phase step ([source](../../src/oscillators/va/warp.rs#L42-L79)), and the existing spectral bank already says callers must include the maximum warp derivative when choosing a legal mip. The honest conceptual order is:

`base/frame morph → phase warp → oscillator spectral response → final Nyquist cap → playback`.

That complete composition can be compiled when warp and frame position are static or move at a deliberately bounded control rate. If phase warp, phase modulation, or wavetable position moves at audio rate, the signal is time-varying and no single rebuilt cycle represents the result. In that case KURV must either:

- define the spectral response as filtering only the carrier's base spectrum, then use the existing warp/PM antialias strategy for newly generated sidebands;
- use a precomputed multidimensional morph surface with a documented interpolation and memory budget; or
- switch to a measured oversampled/additive backend.

A worker-built table is not automatically deterministic under dense host automation or offline bounce. V1 must either make spectral controls bounded/control-rate by contract, guarantee synchronous non-audio-thread rebuild scheduling, or defer host/audio-rate modulation for them. Do not silently drop automation points when the worker falls behind.

Static bandlimiting follows the standard oscillator rule: no generated partial may reach Nyquist. Välimäki and Huovilainen's survey, [“Antialiasing Oscillators in Subtractive Synthesis”](https://doi.org/10.1109/MSP.2007.323276), and Stilson and Smith's [BLIT paper](https://ccrma.stanford.edu/~stilti/papers/blit.pdf) provide the primary background. Dynamic gain motion also creates amplitude-modulation sidebands; crossfading two individually safe cycles does not guarantee those sidebands remain below Nyquist. Reserve spectral headroom based on the maximum supported control bandwidth or oversample the morph path.

## 6. Minimal product recommendation

The product surface should be an ordered `WARP` module with two domains:

- **PHASE:** begin with real `SYNC`; replace the current Bend/PWM/Harmonic entries only when each has an audible, waveform-independent contract and a matching preview.
- **SPECTRAL:** keep the shipped Ratio LP/HP brickwall, then add Band/Notch or Comb only when each has a measured direct oscillator-domain backend.

Real hard sync requires a master phase, a faster slave phase, reset at every master wrap, and antialias correction at the reset discontinuity. Renaming the current smooth phase displacement to Sync would not implement sync.

### 1. Real Sync

Use separate master and slave phase. `RATIO` controls the slave frequency; every master wrap resets the slave and deposits the required antialias correction for the reset discontinuity. This must be audible and visible on every non-constant source waveform.

### 2. Harmonic LP

Use a log harmonic-number cutoff, not an Hz cutoff presented as though it were independent of pitch. A cutoff at harmonic `K` tracks the note at approximately `K × f0`. Apply a one-bin fractional edge or a short soft knee. The existing harmonic-cap mips suggest a low-risk prototype: choose the lower of the timbral cap and pitch/warp-safe cap and interpolate adjacent levels. That reuses one compiled representation, though the 20 existing caps are too coarse to promise continuous modulation without evaluating/blending adjacent levels.

### 3. MagShift (rejected product direction)

Use the ratio-form magnitude-envelope remap above with `SHIFT` and `AMOUNT`. Keep energy on the destination oscillator's harmonic grid, preserve its pitch, use one documented phase policy, and zero-fill rather than wrap.

One shared compact control surface is enough: `TYPE`, one type-dependent primary control (`RATIO`, `CUTOFF`, or `SHIFT`), and `AMOUNT`. Every preview must evaluate the actual source operation and use the operation's real coordinate instead of drawing an ordinary Hz-domain IIR response.

Defer:

- **HP/Comb/Band/notch:** useful once the shared harmonic-mask path is proven.
- **Formant/harmonic stretch:** remap bins and require phase/interpolation policy, collision handling, and stronger alias protection.
- **Smear/random amplitudes:** require stable random identity and careful loudness behavior.
- **Phase disperse:** changes partial phase rather than magnitude; it is a spectral warp, not a filter, and should be named separately.

## 7. Licensing and patent boundary

KURV declares the ISC license ([manifest](../../Cargo.toml#L1-L7)). Vital is GPLv3 and explicitly discusses separate commercial licensing for proprietary products ([Vital README](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/README.md#L1-L21)); ZynAddSubFX is GPLv2. Their source is suitable evidence for behavior, but copying implementation text into an ISC/proprietary KURV distribution would create licensing obligations. Reimplement the small public mathematical operation independently, retain provenance notes, and do not copy names, constants, comments, or UI trade dress.

Flow is Apache-2.0, whose license includes an express contributor patent grant; Plaits is MIT and Oidos is zlib. Those are more permissive references if KURV later borrows implementation detail, but notices and exact file-level terms still need review.

A narrow Google Patents search found no source repository claiming a patent on these particular harmonic masks. A broad result, Stanford's 1985 [US4622877A wavetable-modification patent](https://patents.google.com/patent/US4622877A/en), is marked expired and concerns a different probabilistic wavetable-modification system. This is a technical prior-art scan, not a freedom-to-operate opinion. Before shipping in a commercial product, counsel should search claims and families around wavetable spectral morphing, not rely on repository licenses or search-result legal status.

## Bottom line

KURV can add oscillator-domain processing without duplicating its existing filters by exposing the domain as a filter type and lowering that operation into every affected upstream oscillator before summation. Version 0.8.19 starts with a ratio-native high-pass brickwall; real `SYNC`, general custom-cycle filtering, and `MAGSHIFT` remain separate follow-up operations. Keep the current `SVF`, `PHASER`, and `SCREAM` category named by its audio/voice domain, not by a phase property it does not share.

## 8. Implemented decision: Ratio Brickwall in 0.8.20

This section records the technical substance of the implementation conversation and supersedes the earlier sequencing recommendation where it conflicts.

### Locked contract

- The control is a harmonic ratio `R`, not Hz. `0x` bypasses the operation. In `HIGH` mode, `1x` removes the fundamental and `10x` removes harmonics 1 through 10. In `LOW` mode, `1x` retains only the fundamental and `10x` retains harmonics 1 through 10.
- The operation is a brickwall high-pass or low-pass over the oscillator's coefficients. It does not expose Q, slope, resonance, or morph controls because those would not have an implemented meaning.
- The pitch-dependent quantity is only the final legal harmonic: `k_max` is chosen from the effective phase step of each lane, so oscillator transpose, cents, note pitch, unison detune, jitter, and pitch modulation move the Nyquist cap automatically. The lower mask remains ratio-native and pitch-independent.
- A Ratio Brickwall affects every preceding oscillator in the ordered group. A later oscillator is unaffected. Multiple downstream Ratio Brickwalls compose to the greatest cutoff. Ordinary linear filters may be evaluated afterward because they commute with a fixed source-spectrum mask; nonlinear `SCREAM` can create new harmonics after the source mask and those are intentionally not removed.

### Shipped backend

[`src/oscillators/va/ratio.rs`](../../src/oscillators/va/ratio.rs) builds process-global 2,048-sample Fourier-prefix tables for saw and triangle harmonics 0 through 1,023 during DSP initialization. Runtime rendering evaluates

`wave[k_max] - wave[floor(R)]` for `HIGH`, or `wave[min(k_max, floor(R))]` for `LOW`

with periodic Catmull-Rom interpolation. This is constant work per sample: the renderer does not allocate, lock, transform, or spawn one sine oscillator per retained harmonic. Arbitrary-width pulse uses the exact identity `pulse(phi,w) = saw(phi-w) - saw(phi) + DC`, so the high-pass range needs shifted saw-prefix lookups and no separate pulse bank. Sine is its one known coefficient; the existing sine/triangle/saw/pulse morph remains linear with the existing triangle-to-saw compensation.

The two prefix banks occupy `2 × 1,024 × 2,048 × 4 = 16 MiB`, process-global rather than per voice, oscillator, unison lane, or plugin instance. Runtime table access occurs only after explicit initialization; a missing bank returns silence instead of allocating on the audio thread.

The ordered scalar and settled block renderers both lower a downstream Ratio Brickwall into the affected canonical VA source. The old mixed-bus filter processor treats the mode as pass-through, and terminal-filter shortcuts are disabled for groups containing it so they cannot accidentally skip the source operation. Filter mode byte `12` persists the type without reinterpreting older mode values. The editor shows `TYPE`, `HIGH`/`LOW`, and `RATIO`, plots the actual hard step on a `0x..1024x` axis, and reuses the existing Cutoff automation destination with ratio-aware normalization.

### Deliberate V1 boundary

- The exact prefix backend covers canonical VA only. Custom curves, Noise, and Resynth pass unchanged; the editor tooltip states this instead of pretending they were filtered.
- Current oscillator phase warp is evaluated after the masked base cycle. It may create new partials, just as any later nonlinear spectral/phase operation can. Exact `warp -> brickwall` ordering needs compilation of the composed cycle and remains part of the general ordered Warp architecture.
- The bank ceiling is harmonic 1,023, matching KURV's existing 2,048-point spectral representation. A cutoff above the lane's legal `k_max` returns silence.
- Ratio modulation is continuous in the control system but the hard mask changes only when `ceil(R)` crosses an integer harmonic. A future fractional edge would be a different, explicitly non-brick response.

### Verification boundary

No tests were added or run. DAW listening, automation sweeps, CPU measurement at maximum polyphony/unison, and spectrum captures remain runtime acceptance work.

## 9. SIMD and handwritten-assembly audit

The `8` in KURV's `f32x8` renderer is a machine-width batch of eight independent unison oscillator lanes, not a cap of eight harmonics. A 64-lane unison oscillator is visited as eight AVX2-width packs, followed by the existing four-lane and scalar tail paths. The one explicit time-axis exception renders eight consecutive samples from one oscillator. These are separate choices of SIMD axis; neither changes the Ratio Brickwall's harmonic range.

Ratio Brickwall already compiles every integer prefix from 1x through 1023x during initialization. Runtime playback obtains any retained interval with prefix subtraction, so its work is constant with respect to the number of retained harmonics. Replacing that lookup with eight additive partials at a time would change playback from `O(1)` table work to `O(H)` oscillators per unison lane and would be a regression on low notes. Harmonic-axis SIMD remains useful for offline artifact compilation or a deliberately small sparse residual-correction backend, not for this exact brickwall.

The real missing fast path is that Ratio Brickwall currently makes the settled block renderer call the scalar structural renderer once per frame, and its nominal eight-lane branch calls `generate_shape_step_ratio` eight times. The first implementation experiment should therefore be a complete eight-unison Ratio block kernel. Its table lookup should compare two implementations under the same full-plugin workload:

1. scalar lane loads packed into vectors, followed by SIMD Catmull-Rom arithmetic;
2. AVX2 `_mm256_i32gather_ps` loads, followed by the same arithmetic.

Gather is not an assumed win. LLVM documents gather/scatter as cost-model-dependent rather than universally profitable ([Loop Vectorizer](https://llvm.org/docs/Vectorizers.html#scatter-gather)), and measured Zen 4 `VGATHERDPS` data shows a relatively heavy instruction ([uops.info](https://uops.info/html-instr/VGATHERDPS_YMM_K_VSIB_YMM.html)). Scalar-packed loads also preserve the portable SSE/NEON fallback; ordinary NEON has no equivalent gather, while Arm introduces gather/scatter with SVE ([Arm SVE overview](https://developer.arm.com/community/arm-research/b/articles/posts/the-arm-scalable-vector-extension-sve)).

Handwritten `asm!` is not the first implementation. Rust's target-feature contract already supports guarded AVX2/FMA kernels ([Rust Reference](https://doc.rust-lang.org/reference/attributes/codegen.html#the-target_feature-attribute)), and `std::arch` exposes the required gather intrinsic ([Rust core::arch](https://doc.rust-lang.org/core/arch/x86_64/fn._mm256_i32gather_ps.html)). Inline assembly adds a larger unsafe memory/clobber contract and can hide optimization opportunities from LLVM ([Rust inline assembly](https://doc.rust-lang.org/reference/inline-assembly.html)). It earns a shipping path only if a complete intrinsic kernel has correct output, remains a measured hotspot, and disassembly proves a specific code-generation defect.

The second credible whole-kernel target is dynamic phase-warp rendering. KURV's current AVX2 wrapper keeps phase wrapping and stereo FMA in vectors but round-trips warped phases and samples through arrays and the generic `f32x8` sampler each frame. A useful experiment must fuse phase advance, warp, BLEP/BLAMP sampling, and stereo accumulation for the whole block. Replacing isolated instructions with assembly is too small to matter at plugin level.

External synths reinforce choosing the SIMD axis from data layout rather than harmonic numbering. Vital modifies packed Fourier bins during spectral-table rebuild and vectorizes oscillator playback afterward ([spectral morph](https://github.com/mtytel/vital/blob/main/src/synthesis/producers/spectral_morph.h#L243-L305), [oscillator](https://github.com/mtytel/vital/blob/main/src/synthesis/producers/synth_oscillator.cpp#L288-L334)). Surge vectorizes consecutive FIR/output samples in its wavetable oscillator ([source](https://github.com/surge-synthesizer/surge/blob/main/src/common/dsp/oscillators/WavetableOscillator.cpp#L430-L466)). Neither architecture assigns an arbitrary live partial bank to SIMD lanes during ordinary oscillator playback.

One semantic issue must be resolved before deeply optimizing this backend: KURV currently warps phase and then samples the harmonic-limited canonical prefix, which produces `warp(filtered_cycle)`. Phase warping can create new harmonics after the brickwall. An exact `warp -> brickwall` module order requires compiling the composed cycle or another representation; optimizing the current prefix sampler does not change that ordering.

### Measured 0.8.22 result

The Ratio renderer now packs as many complete eight-lane groups as the active unison count permits, then one four-lane tail, then at most three scalar lanes. On the existing `stress4-ratio` full-plugin workload (128-frame callbacks, four voices, 64 unison lanes per oscillator, four oscillators, 2x oversampling), its median callback time fell from 11.787 ms to 7.690 ms with portable packed loads and to 6.821 ms with the runtime-guarded AVX2/FMA gather kernel. The retained `std::arch` seam is therefore about 11% faster than the portable packed lookup and 42% faster than the original scalar Ratio block path on this machine. Output remained finite with the same 0.385290 peak; checksum differences were approximately 4e-5 and came from changed floating-point evaluation order.

Stable Rust's portable `std::simd` API remains unstable, so KURV continues to use its existing `truce_simd`/`wide` vectors for the portable path. No handwritten assembly was added: the intrinsic kernel emitted the required gathers and FMAs and left no measured assembly-level defect to repair.

### Measured 0.8.23 follow-up

A post-change `perf` profile showed that scalar `sincosf` still consumed about 15% of the Ratio stress workload. Ratio was unpacking each eight-lane phase vector and calling the scalar warp evaluator even though KURV already had a shared eight-lane phase-warp evaluator. Reusing it reduced the median from 6.821 ms to 4.873 ms per callback. Fusing the retained-row and removed-row Catmull-Rom work into one range interpolation reduced that again to 4.680 ms. Against the original 11.787 ms scalar Ratio path, the final measured reduction is about 60% on this workload.

The post-change profile is now dominated by random prefix-table access. Disassembly shows the intrinsic path emitting eight `vgatherdps` operations for four cubic taps across the retained and removed harmonic rows. Handwritten assembly cannot eliminate those memory accesses without changing interpolation quality or table architecture. A separate non-Ratio stress workload measured about 1.27 ms per callback and its dominant oscillator functions already emitted AVX2-width YMM arithmetic, so the Ratio result must not be presented as a 60% whole-synth speedup.

### 0.8.24 control and boundary corrections

Ratio control, host automation, the response graph, and the readout now share one reversible `ln(1 + R/4)` mapping. It keeps the full `0x..1024x` range while placing `1x` at about 4% and `2x` at about 7% of the travel instead of making the first useful harmonics unusually sticky. The previous automation formula divided by `MIN_RATIO = 0`, so it could produce non-finite values; the shared mapping removes that path.

`0x` bypasses both modes. `LOW 1024x` also bypasses because it removes no representable harmonic, while `HIGH 1024x` remains active because it removes the whole supported band. The bypass is decided before choosing the Ratio renderer, so LOW at maximum preserves the native bandlimited oscillator instead of truncating it to the prefix bank.

This distinction matters for very low notes. A native 1 Hz saw may contain legal harmonics far above 1023, up to the playback Nyquist limit. Active Ratio filtering is still backed by the fixed 2048-sample prefix representation: LOW cutoffs through 1023 are exact for their retained band, but HIGH can only retain harmonics through 1023 and therefore drops higher legal partials on sufficiently low fundamentals. Raising the constant would make the dense prefix bank impractical; a future exact low-note HIGH path needs a different representation, such as native full-band output minus the rejected low prefix.

The group-header preview also now wraps scalar sine phase for arbitrary preview-cycle counts. Playback phases were already normalized, but the preview intentionally warmed filters over five cycles and the old helper only subtracted one cycle, causing sine and triangle-to-sine shapes to leave their polynomial domain and appear as positive DC.

### Measured 0.8.25 active-Ratio correction and rejected RustFFT alternative

The earlier optimized Ratio path still cost 5.75 ms per 128-frame callback versus 1.61 ms without Ratio in the `stress4` torture workload. Profiling found redundant zero-prefix gathers, repeated scalar phase wrapping and Nyquist rounding inside the x8 sampler, and scalarized eight-lane Ratio playback in the modulated renderer. The retained implementation skips the immutable zero row, wraps playback phase once in SIMD, short-circuits Nyquist work when the requested harmonic ceiling is already legal, uses two-point interpolation only through harmonic 64, and shares the x8 sampler with modulated playback. The same workload now measures 3.07 ms active and 3.89 ms with cutoff modulation. This is about 47% lower than the previous active result, but Ratio remains roughly 1.9 times the native torture path because exact harmonic-prefix lookup still performs indexed table reads per unison lane.

## 10. Domain taxonomy and shared spectral engine decision

The implemented menu taxonomy is `AUDIO` and `SPECTRAL`, not `MINIMUM PHASE` and `SPECTRAL`. SVF, Phaser, and Scream share KURV's fixed-size audio-filter engine, but they do not share a minimum-phase contract: Phaser deliberately builds all-pass phase rotation and cancellation, while nonlinear Scream has no single LTI magnitude/phase response. Ratio Brickwall is spectral because it changes each upstream oscillator's partial set before summation.

The source layout now mirrors that distinction without duplicating the common engine:

```text
filters/
  engine/
    mod.rs                 shared fixed-size state, coefficients, dispatch
    audio/
      svf.rs
      phaser.rs
      scream.rs
  spectral/
    ratio_brickwall.rs     ratio contract, mapping, and bypass rules
```

Ratio playback remains inside `oscillators/va/ratio.rs`. That is deliberate locality rather than a second filter base: it needs the VA oscillator's phase, actual per-lane step, warp evaluator, waveform morph, and unison batching. Pulling those internals outward into a generic filter interface would enlarge the interface without sharing implementation.

### RustFFT verdict

KURV already depends on RustFFT 6.4.1. [`dsp::fft`](../../src/dsp.rs) caches a planner and scratch storage for worker/editor compilation, and [`BandlimitedWaveCurve`](../../src/wave_curve/bandlimit.rs) already samples a procedural cycle, transforms it, masks bins, inverse-transforms 20 harmonic mips, and publishes contiguous playback tables. RustFFT's planner automatically selects supported SIMD backends, and its documented `process_with_scratch` path supports caller-owned reusable scratch ([RustFFT README](https://github.com/ejmahler/RustFFT/tree/v6.4.1#usage), [`FftPlanner`](https://docs.rs/rustfft/6.4.1/rustfft/struct.FftPlanner.html), [`Fft::process_with_scratch`](https://docs.rs/rustfft/6.4.1/rustfft/trait.Fft.html#method.process_with_scratch)).

RustFFT cannot make the current Ratio hot loop faster in place. Ratio playback is already `O(1)` per sample: it selects harmonic-prefix rows and interpolates them. An FFT would instead compile a different 2,048-sample cycle in `O(N log N)`, after which playback could use one small contiguous table. That can trade the current scattered 16 MiB prefix-bank reads for cheaper playback, but only under a bounded update contract:

- compile outside `Plugin::process()`;
- preallocate transform and publication storage;
- crossfade immutable old/new cycles;
- coalesce bounded control-rate changes without silently claiming sample-accurate modulation;
- share one artifact only when pitch/warp/unison requirements make it valid for all consumers.

Calling the existing thread-local `dsp::fft` helper from the audio callback would be invalid because planner access, scratch resize, and artifact publication may allocate or coordinate. Vital follows the same broad architecture: spectral bins are rebuilt into alternating buffers, then ordinary oscillator playback crossfades those buffers rather than performing an FFT per output sample ([rebuild/crossfade](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1396-L1428)).

KURV explicitly rejects that rebuild/crossfade contract for its live spectral modules. The comparison explains why RustFFT is not used here; it is not a deferred implementation plan.

### Algorithms worth keeping distinct

| Operation | Natural representation | Best KURV backend | Modulation ceiling |
|---|---|---|---|
| Ratio LP/HP brickwall | contiguous harmonic interval | current prefix difference | audio-rate integer edge, but expensive scattered playback |
| Harmonic band/notch | one or two contiguous intervals | prefix-range composition first | audio-rate edge; cost grows with range count |
| Sparse harmonic correction | a few changed partials | residual sine correction over native VA | audio-rate while the changed set stays small |
| Magnitude tilt/EQ | scalar gain per bin | RustFFT-compiled cycle/mip bank | bounded control rate plus crossfade |
| MagShift/formant warp | interpolated magnitude-envelope remap | RustFFT-compiled cycle/mip bank | bounded control rate plus crossfade |
| Harmonic comb | periodic bin mask | RustFFT compiler; special closed form only for a deliberately narrow family | bounded control rate by default |
| Spectral phase disperse | phase offset per bin | the same compiler, but a spectral warp rather than a filter | bounded control rate plus crossfade |
| Sync/phase bend/PWM | phase-coordinate/reset operation | procedural VA phase path with BLEP at discontinuities | audio rate |
| Inharmonic partial stretch | frequencies leave the integer harmonic grid | additive/oscillator bank or longer-period approximation | expensive; not an ordinary single-cycle spectral filter |
| Smooth low-pass oscillator | closed-form pulse/kernel family | LP-BLIT or circular antiderivative convolution | audio rate, but its response is a named character rather than arbitrary bin EQ |

The reusable future module is therefore an **ordered spectral compiler**, not a second copy of the audio-filter state machine. It should accept a procedural cycle plus a short list of magnitude/phase operations, fuse consecutive same-domain operations, use RustFFT only when crossing between cycle and spectrum, build final antialiased mips, and publish one immutable artifact. One implementation does not yet justify adding an abstract trait or public compiler interface; MagShift or another genuinely general operation should create that seam.

### Ranked next additions

1. **Harmonic Band/Notch** if immediate audio-rate modulation and reuse of the measured prefix backend matter most. This is the smallest honest extension, but every additional retained interval adds table reads and must be profiled against the current Ratio cost.
2. **Harmonic Comb** if a direct periodic-mask evaluator remains cheap under dense unison.
3. **Sparse harmonic correction** for a small, bounded changed-partial set where direct residual evaluation beats more prefix reads.
4. **LP-BLIT/Smooth** only as a separate oscillator-filter character when audio-rate smooth cutoff is more important than exact arbitrary spectral masks.

No placeholder modes should be exposed merely to fill the `SPECTRAL` category. Each addition must ship its real backend, automation semantics, ordering, antialias policy, and playback-faithful preview together.

### Measured 0.8.27 prepared-source correction

Ratio now resolves the oscillator shape, ratio band, warp controls, prefix-bank reference, and morph constants once per oscillator block, then shares that prepared source across every complete eight-lane unison pack and scalar tail. The evaluator reuses phase/table coordinates across same-phase waveform morphs, removes the duplicate saw lookup from saw-to-pulse morphing, and uses a direct uniform-row AVX2 gather when all eight lanes share the common low-pass ceiling. It still evaluates every lane's independent phase and pitch; copying one finished lane would destroy unison.

On the existing `stress4-ratio` workload (128 frames, four synth voices, four VA oscillators, 64 unison lanes each, 2x oversampling), the five-run median fell from 3.849 ms to 2.500 ms per callback, about 35%. The native comparison remained about 1.66 ms. Output stayed finite and the post-change five-run checksum was identical across runs. The modulated Ratio case stabilized around 3.35 ms, but its older baseline was scheduler-noisy and is not used for a percentage claim.

The same change adds explicit audio-rate drag semantics in the oscillator readout strip: while an oscillator source is held, the four destination cells read `AM`, `FM`, `RM`, and `PM` in the theme's tertiary semantic color. RM is a real post-carrier ring multiplication route, not a relabeled level route. Routed VA sources also feed the carrier's deterministic single-cycle preview using the same AM/FM/RM/PM equations, route depth, source pitch ratio, source phase/warp, and source waveform selection used by playback.
