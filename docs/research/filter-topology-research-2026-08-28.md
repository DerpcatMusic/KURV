# KURV filter-topology research (2026-08-28)

Scope: primary papers, owner-maintained source repositories, and official API documentation. This note does not claim listening-test or profiler evidence. It does not prescribe a new dependency merely because an implementation exists elsewhere.

## Decision brief

- **Accept:** keep a small, fixed-capacity TPT SVF implementation inside KURV. The existing state and SIMD representation are already closer to the required per-voice, allocation-free seam than the reusable libraries surveyed below.
- **Accept, but name honestly:** morph filter *order* by running the adjacent integer-order responses and crossfading their outputs. This is continuous and stable when both endpoints are stable, but it is not a fractional-order Butterworth transfer function.
- **Reject:** changing a cascade's active stage count or damping table abruptly, or linearly interpolating arbitrary direct-form IIR denominator coefficients. Both make time-varying state behaviour harder to control; coefficient interpolation is not a general stability guarantee.
- **Accept:** define phaser complexity in **all-pass sections** and separately derive/display its observable notch count. Use a nested, progressively refinable frequency layout only if the UI preview evaluates the exact same transfer function.
- **Reject:** calling the current Scream mode a port of Cure Audio Scream. It borrows the broad LP/feedback-HP/saturation shape, but omits Cure's input gain semantics, ADAA2 tanh, coefficient design, detector/expander gate, and keytracking.
- **Reject for now:** a new general-purpose DSP dependency. `fundsp`, JUCE, DaisySP, Faust, `synfx-dsp`, and the Surge crates are useful references, but none is a lower-risk drop-in for KURV's fixed Rust/SIMD/per-voice hot path.

## 1. What “1 to 128 poles” can mean

KURV currently has two different structures behind the same broad complexity control:

- SVF mode cascades up to 64 second-order sections (`MAX_SVF_STAGES`), hence at most 128 poles. It computes Butterworth section damping values, runs the whole integer portion of the cascade, and blends only the newest section's output in [src/filters/svf.rs](../../src/filters/svf.rs#L866-L951).
- Phaser mode cascades up to 128 first-order all-pass sections and makes the newest section approach bypass by moving its coefficient from `1.0` toward its target in [src/filters/svf.rs](../../src/filters/svf.rs#L953-L1005).

These should not share an unexplained “poles” promise. For an integer cascade with section transfer functions `H_k(z)`, the total transfer function is their product. Magnitude in dB and phase therefore add per section. Raising order necessarily adds causal state and phase rotation; a sharper transition also lengthens the effective transient response. Ringing is not an independent implementation bug: its amount depends on pole radius/Q and prototype choice. Butterworth prioritizes flat passband magnitude, while a lower-Q or Bessel-like distribution sacrifices transition sharpness for better transient/group-delay behaviour.

The [Cytomic TPT SVF derivation](https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf) and Zavalishin's [*The Art of VA Filter Design*](https://www.native-instruments.com/fileadmin/ni_media/downloads/pdf/VAFilterDesign_2.0.0a.pdf) support the present two-integrator TPT family and its simultaneous LP/BP/HP outputs. Neither makes “fractional filter order” a free parameter of one SVF. A rational IIR still has an integer number of states/poles.

### Recommended continuous-order contract

For a requested order between adjacent realizable orders `N` and `N+1`, evaluate both endpoint taps from the same cascade:

`y = lerp(y_N, y_(N+1), fraction)`

This is exactly the useful shape of KURV's current SVF stage blend. It is bounded for a bounded crossfade between stable endpoint outputs, costs only the additional section already being processed, and gives continuous samples as the control moves. It should be documented and visualized as **order morphing**, not as an exact fractional-order prototype. It also means the intermediate transfer is `(1-f)H_N + fH_(N+1)`, not `H^(N+f)`.

For modulation, retain the TPT states and smooth the perceptual controls. JUCE's official [`StateVariableTPTFilter`](https://github.com/juce-framework/JUCE/blob/master/modules/juce_dsp/processors/juce_StateVariableTPTFilter.h#L46-L59) likewise describes TPT as designed for fast modulation but explicitly warns that cutoff smoothing may still be required. Do not reset state on routine order motion; initialize a newly introduced section so its bypass endpoint is continuous, then retire it only after the blend reaches zero.

### Cost boundary

The cascade is `O(voices × samples × active_sections)` with fixed `O(voices × max_sections)` state. A 64-section stereo SVF per active note is intrinsically expensive and stage-recursive, so SIMD can operate across independent lanes/voices but not across consecutive stages of one signal. Coefficients and Butterworth damping should remain cached outside the inner stage tick, as they are now in [src/filters/svf.rs](../../src/filters/svf.rs#L736-L773). A useful product ceiling should be established from measured worst-case polyphony, not from the visual appeal of “128 poles.”

## 2. Phaser: stages, notches, spacing, and feedback

A first-order all-pass section has unity magnitude and frequency-dependent phase. Audible notches appear only after dry and all-pass-chain signals are summed (or differenced) where their phase relationship cancels. Consequently, **one all-pass section is not one audible notch**. The notch count depends on accumulated phase, mix polarity, coefficient distribution, and feedback.

The owner-maintained Faust phaser is a useful reference implementation. Its API calls the compile-time count `Notches`, spaces adjacent center frequencies by an explicit ratio, sweeps the first center with an LFO, and provides dry/all-pass depth, inversion, and signed feedback in [`phaflangers.lib`](https://github.com/grame-cncm/faustlibraries/blob/master/phaflangers.lib#L166-L210). Its inner all-pass chain is inside a feedback loop and uses second-order resonant all-pass sections with pole radius derived from notch width in [`phaflangers.lib`](https://github.com/grame-cncm/faustlibraries/blob/master/phaflangers.lib#L144-L163). This is a deeper phaser model than merely distributing first-order coefficients.

Actionable recommendations:

1. **Accept the current first-order cascade as a lean topology**, but label the control “stages” (1–128), not “notches” or an unexplained pole count.
2. **Accept progressive insertion only with an exact response preview.** Moving the newest first-order coefficient from the algebraic bypass point toward its target can make stage activation continuous, but it initially introduces phase near the edge of the band rather than creating a uniformly deeper bank. Verify its state initialization and modulation audibly; the formula alone does not establish click-free behaviour.
3. **Replace the current bit-reversed/radical-inverse distribution only if the intended sound is ordered spacing.** KURV's `nested_phase_unit()` in [src/filters/svf.rs](../../src/filters/svf.rs#L1072-L1105) is excellent for adding points without moving old ones, but adjacent stage indices are deliberately not adjacent frequencies. That makes a simple “skew” control perceptually nonlocal. An ordered geometric/log-frequency grid gives a more legible center/width/skew contract, but adding stages then moves old frequencies unless the bank is crossfaded.
4. **Add signed feedback only with a bounded, smoothed gain and explicit stability headroom.** Feedback sharpens/colours cancellation peaks and notches; it does not create a trustworthy one-to-one stage/notch mapping. Keep the loop sample-recursive and allocation-free.
5. **Do not use integer delay-line all-pass modulation as a shortcut.** DaisySP's official library promises static memory globally in [`daisysp.h`](https://docs.daisy.audio/DaisySP/daisysp_8h_source/), but its current phaser is a delay-modulation design rather than the coefficient-modulated pole bank KURV is building.

The high-quality visualization should evaluate the same complex transfer function, stage frequencies, order blend, wet/dry polarity, and feedback used by DSP. An approximate collection of decorative notches will necessarily disagree, especially while a stage is partially active. Response calculation/rendering belongs off the audio thread; publish only immutable parameter snapshots to it.

## 3. Cure Audio Scream and ADAA

Cure Audio is unusually clear about what its model does and does not reproduce. Its [README at the inspected revision](https://github.com/Cure-Audio/Scream/blob/d60a22015b8bf2d83df10e37c49f9b746849dcae/README.md#L8-L36) says the design was informed by Zavalishin's nonlinear-filter chapter, that input amplitude materially changes saturation versus resonance, and that the original synth's internal LP/HP cutoff and Q were keytracked. It also admits that the remaining distortion match is only “close enough” and invites different future topologies in [the same README](https://github.com/Cure-Audio/Scream/blob/d60a22015b8bf2d83df10e37c49f9b746849dcae/README.md#L54-L66).

The actual per-sample signal path is authoritative:

`input gain -> input + prior feedback -> tanh ADAA2 -> low-pass -> wet mix`

and the feedback branch is:

`low-pass output -> feedback gain -> high-pass -> tanh ADAA2 -> detector/expander gate -> next sample`

See Cure's [`plugin.c`](https://github.com/Cure-Audio/Scream/blob/d60a22015b8bf2d83df10e37c49f9b746849dcae/src/plugin.c#L601-L645). When parameters are smoothed or modulated, Cure recalculates its LP/HP coefficients per sample in [`plugin.c`](https://github.com/Cure-Audio/Scream/blob/d60a22015b8bf2d83df10e37c49f9b746849dcae/src/plugin.c#L550-L599).

Its tanh implementation is not an ordinary soft clip. Cure's C port uses a **second-order antiderivative antialiasing** state with double-precision antiderivatives, explicit near-equal-input branches, and a dilogarithm in [`ADAA.h`](https://github.com/Cure-Audio/Scream/blob/d60a22015b8bf2d83df10e37c49f9b746849dcae/src/libs/ADAA.h#L34-L107). That file retains Jatin Chowdhury's BSD-3-Clause notice; the surrounding Scream project is MIT licensed.

The underlying peer-reviewed ADAA result is Bilbao, Esqueda, Parker, and Välimäki, [“Antiderivative Antialiasing for Memoryless Nonlinearities”](https://aaltodoc.aalto.fi/items/470aab15-1702-4ccf-a148-24e6173079fb), IEEE Signal Processing Letters 24(7), 2017, DOI `10.1109/LSP.2017.2675541`. It establishes higher-order antiderivative methods as a way to reduce aliasing from memoryless nonlinearities with fewer operations than conventional high-factor oversampling. It does **not** make arbitrary placement inside a nonlinear feedback loop automatically equivalent to the analog system; ADAA is stateful and its delay/phase must be considered in the loop.

KURV's current Scream implementation in [src/filters/svf.rs](../../src/filters/svf.rs#L829-L863) uses a rational `x / sqrt(1+x²)` saturator before a TPT LP, a TPT HP in feedback, and a simple peak-derived gate. This is a Scream-inspired filter, not Cure Scream.

Recommendation:

- **Accept:** rename/describe it as Scream-inspired unless KURV deliberately ports the complete semantics.
- **Accept for a faithful mode:** port the topology and ADAA under their retained licenses, but first budget two double-precision ADAA2 evaluations per channel/sample/voice plus LP/HP work. Preallocate all state. Keep keytracking because KURV, unlike an effect plug-in, knows the note.
- **Reject:** dropping Cure's ADAA2 code into the existing SIMD feedback loop and declaring parity. The nonlinear function, latency/state, gain calibration, coefficient family, and gate all change the loop sound and stability.
- **Consider instead:** modest fixed oversampling around the nonlinear mode if measurement shows ADAA2's dilogarithm/branch cost is unsuitable. That is simpler to reason about but multiplies the entire nonlinear-loop workload and requires fixed preallocated resampler state.

## 4. Reuse survey

| Candidate | What is credible | KURV decision |
|---|---|---|
| [Cytomic TPT SVF papers](https://cytomic.com/technical-papers/) | Compact, owner-published optimized linear TPT SVF derivations. | **Accept as formula/reference.** KURV already implements this family; no dependency needed. |
| [JUCE `StateVariableTPTFilter`](https://github.com/juce-framework/JUCE/blob/master/modules/juce_dsp/processors/juce_StateVariableTPTFilter.h) | Maintained C++ TPT SVF; fast-modulation intent; sample API. It stores per-channel state in `std::vector`, prepared before processing. | **Reject dependency/FFI.** GPL/commercial framework and C++ bridge are disproportionate for one small Rust kernel. |
| [Faust libraries](https://github.com/grame-cncm/faustlibraries) | Maintained reference phaser with spacing, width, mix, inversion, and feedback; can generate static DSP. | **Accept as behavioural reference; reject generated-code toolchain for this change.** Check the per-function STK/LGPL notices before copying code. |
| [DaisySP](https://github.com/electro-smith/DaisySP) | MIT C++; official docs state all memory is static and processing is sample-oriented. | **Accept as embedded reference; reject FFI.** Its SVF/phaser algorithms do not directly supply KURV's variable-order TPT bank. |
| [`fundsp`](https://github.com/SamiPerttu/fundsp) | MIT/Apache-2.0 Rust graph DSP. Its official README says most `AudioNode`s can be stack allocated and `allocate()` preallocates needed memory before a real-time context. | **Reject for the filter hot path.** The graph/dynamic-control surface is much wider than KURV needs and does not solve 128-pole morph semantics. |
| [`synfx-dsp`](https://github.com/WeirdConstructor/synfx-dsp) | Rust collection containing a Simper SVF and other VA filters. | **Reject.** GPL-3.0-or-later is a licensing mismatch for likely proprietary plug-in distribution, and importing the collection adds no architectural leverage. |
| [Surge Rust filter crates](https://crates.io/crates/surge-filter) | SIMD-oriented ports of Surge filter infrastructure. | **Reject.** Alpha-era, multi-crate surface and GPL-family provenance are poor fits for one contained kernel. |
| [Cure Audio Scream](https://github.com/Cure-Audio/Scream) | MIT reference topology plus separately BSD-3-Clause ADAA implementation, with unusually useful calibration caveats. | **Accept as the only faithful Scream reference**, subject to attribution, retained notices, profiling, and loop-specific validation. |

No surveyed library is already installed. KURV currently has `truce-simd`, `wide`, and `rustfft` in [Cargo.toml](../../Cargo.toml#L71-L93); only the SIMD crates are relevant to the sample filter kernel. `rustfft` is appropriate for an off-audio-thread response/analyser implementation, not for evaluating each voice's filter.

## 5. Concrete acceptance criteria before DSP changes

These are evidence gates, not requests for a new test suite:

- Define UI terminology: SVF “poles” (2 per section), phaser “all-pass stages,” and derived visible notch count.
- Plot the exact complex transfer at integer and fractional stage values; include dry/wet and feedback. Compare that plot with an offline impulse FFT from the same parameter snapshot.
- Measure worst-case release-build time for maximum voices × maximum stages and for Scream nonlinear alternatives. A 128-pole headline is not acceptable if it cannot sustain the supported polyphony.
- Listen to automated order/stage sweeps on steady tones, impulses, noise, and note releases. Specifically check stage insertion, state retirement, cutoff modulation, and phaser feedback near its limit.
- For any Cure-compatible claim, match input level, note/keytracking, LP/HP cutoff/Q, feedback gain, wet path, gate envelope, and nonlinearity—not just the block diagram.

## Bottom line

KURV does not need a filter crate. It needs a precise contract around its already-deep fixed-capacity filter module. Keep the TPT kernel local; expose adjacent-order taps; distinguish phaser stages from notches; make visualization consume the same coefficient snapshot and transfer equation; and treat Cure Scream as a complete gain-dependent nonlinear feedback design rather than a name for any saturated resonant filter.
