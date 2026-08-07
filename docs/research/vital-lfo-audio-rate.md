# Vital LFO and audio-rate modulation audit

Research snapshot: Vital commit [`636ca0ef`](https://github.com/mtytel/vital/tree/636ca0ef517a4db087a6a08a6a8a5e704e21f836), inspected 2026-08-06. Vital warns that its public repository trails binary releases, so this report describes that source snapshot rather than claiming parity with the current commercial binary ([README lines 2–7](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/README.md#L2-L7)). Technical claims below use Vital source and first-party statements by Matt Tytel.

## Executive answer

Vital has a good *scheduling and routing* model to learn from, but its LFO code must not be copied into KURV.

- Vital renders each editable LFO curve to a 2,048-sample table, then evaluates that table with four-point Catmull-Rom interpolation. Its audio-rate path emits one value per internal sample, but its own frequency is held for the processing slice and its phase control is linearly interpolated across that slice.
- Free-running frequency is exactly `2^-7` through `2^9`, or **0.0078125–512 Hz**. Keytrack mode bypasses that range and has no explicit Nyquist clamp in the LFO path.
- Audio-rate modulation is selective and destination-driven. If no connected destination needs audio rate, the LFO stays control-rate; unused LFOs are disabled.
- SIMD is four-wide SSE2 or NEON and represents two stereo voices per vector. This source explicitly rejects AVX2.
- Vital's default smoothing, block ramps, mutable curve publication, and oversampling-dependent alias control are poor fits for KURV's stated requirement: sample-exact modulation without hidden fixed-time smoothing.
- Vital source is GPLv3; KURV declares ISC. A direct port into ISC KURV is not license-compatible without a separate Vital license. The safe route is a clean-room implementation of the architecture and behavior described here.

“Not wavetable” for KURV's sound generator should mean **procedural, bandlimited virtual analog**: discontinuities are generated/corrected as VA events rather than played from oscillator tables. A modulation LFO may use a precompiled curve table without turning the *main oscillator* into a wavetable synth, but Vital's LFO is still literally table-backed. If KURV wants the no-table rule to cover modulators too, use analytic primitives and a compiled piecewise-curve evaluator. At audio rate, saw/square/custom discontinuities still require bandlimiting or they alias; merely calling the component an LFO does not exempt it from sampling theory.

## What Vital actually implements

### Shape model and evaluation

`LineGenerator` allows at most 100 points, defaults to 2,048 samples, and reserves three guard values for interpolation ([line_generator.h lines 25–35](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/line_generator.h#L25-L35)). Editing compiles the points, powers, and optional sinusoidal segment easing into that mutable buffer ([line_generator.cpp lines 165–213](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/line_generator.cpp#L165-L213)). `SynthLfo` maps phase to the table and performs Catmull-Rom cubic interpolation ([synth_lfo.h lines 80–96](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.h#L80-L96)).

This is efficient and supports arbitrary drawing, but it is not an analytic or bandlimited oscillator. Cubic interpolation reduces table interpolation error; it does not bandlimit discontinuous shapes.

### State, phase, rate, and output cadence

The persistent state is six `poly_float` values: delay time, fade amplitude, smoothed value, fade amount, offset, and phase ([synth_lfo.h lines 67–78](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.h#L67-L78)). Therefore phase and rate arithmetic are 32-bit floating-point SIMD values. Transport-sync time is stored separately as a shared `double` ([synth_lfo.h lines 127–136](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.h#L127-L136)). Phase is normalized in cycles and wrapped with `mod`.

Control-rate mode computes one source value for the processing slice and advances phase by `frequency * num_samples / internal_sample_rate` ([synth_lfo.cpp lines 71–83](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L71-L83)). Audio-rate mode:

1. Reads frequency once for the slice and computes a constant phase increment, `frequency / internal_sample_rate` ([synth_lfo.cpp lines 397–414](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L397-L414)).
2. Linearly moves the phase-control value from its prior value to the current value across the slice ([synth_lfo.cpp lines 246–253](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L246-L253)).
3. Evaluates the cubic curve and writes one SIMD value for every internal sample ([synth_lfo.cpp lines 271–288](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L271-L288)).

So the **output** is genuinely audio-rate when promoted, but modulation of the LFO's own frequency is not sample-varying inside the slice. This distinction matters for nested audio-rate FM.

### Exact frequency limits

The free-frequency control is exponential with exponent limits `-7..9` ([synth_parameters.cpp lines 372–380](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_parameters.cpp#L372-L380)); the exponent operator clamps before exponentiating ([operators.h lines 608–619](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/operators.h#L608-L619)). That makes the enforceable free-mode range:

| Quantity | Result |
|---|---:|
| Minimum free rate | `2^-7 = 0.0078125 Hz` |
| Maximum free rate | `2^9 = 512 Hz` |

Tempo modes derive cycles per second from the tempo ratio and host beats per second. Keytrack mode instead converts `MIDI + transpose + tune` directly to frequency and does not apply an LFO-frequency or Nyquist clamp ([operators.cpp lines 316–344](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/operators.cpp#L316-L344)). The exposed keytrack bounds are transpose `-60..+36` semitones and tune `-1..+1` ([synth_parameters.cpp lines 391–396](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_parameters.cpp#L391-L396)); nominal MIDI note 127 plus those maxima is about 106.3 kHz before considering any upstream bent-MIDI behavior. That is not a safe audible maximum—only evidence that no protective clamp exists here.

Audio-rate **update cadence** is a separate number: the processor's sample rate is host rate times oversampling ([processor.h lines 158–172](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/processor.h#L158-L172)). Vital declares maximum block size 128, maximum oversampling 8, and maximum supported sample rate 192 kHz ([common.h lines 45–55](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/common.h#L45-L55)). `SoundEngine` reduces oversampling as host sample rate rises ([sound_engine.cpp lines 218–240](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/synth_engine/sound_engine.cpp#L218-L240)). At conventional rates, the highest cadence is 352.8 or 384 ksample/s (`44.1k×8`, `48k×8`, `88.2k×4`, `96k×4`, `176.4k×2`, `192k×2`). This is not enforced as a universal ceiling for arbitrary host sample rates because the reduction uses integer division; “384 kHz maximum” would therefore be too strong a claim.

### Trigger, sync, stereo, and looping behavior

Vital exposes six run modes: trigger, transport sync, one-shot envelope, sustain envelope, loop point, and loop hold; rate can be free, tempo, dotted, triplet, or keytracked ([synth_lfo.h lines 48–65](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.h#L48-L65)). Stereo offsets phase by equal and opposite half-offsets for the left/right lanes ([synth_lfo.cpp lines 79–83](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L79-L83)).

Note events carry sample offsets. On retrigger, the audio-rate state starts at a negative phase proportional to the event offset so the reset reaches phase zero on the intended internal sample ([synth_lfo.cpp lines 35–67](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L35-L67)). Switching a running LFO from control to audio rate copies the control-rate state into the audio-rate state, avoiding an arbitrary phase restart ([synth_lfo.cpp lines 432–442](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L432-L442)). Those two behaviors are worth reproducing clean-room.

### Per-voice, global, and SIMD organization

Vital defines eight LFOs ([synth_constants.h lines 23–30](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_constants.h#L23-L30)). The synth voice handler creates an LFO module for each one and wires note trigger, note count, and bent MIDI into it ([synth_voice_handler.cpp lines 153–170](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/synth_voice_handler.cpp#L153-L170)), so synth-domain LFO state is polyphonic. A separate effects modulation handler derives from a one-voice `VoiceHandler` ([effects_modulation_handler.cpp lines 32–41](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/effects_engine/effects_modulation_handler.cpp#L32-L41)) and creates its own copies of the LFO modules ([effects_modulation_handler.cpp lines 115–132](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/effects_engine/effects_modulation_handler.cpp#L115-L132)). That supplies a monophonic/global effects-domain evaluation path rather than sharing all per-voice state.

`poly_float` is four-wide under SSE2 or NEON, while AVX2 is explicitly rejected in this snapshot ([poly_values.h lines 23–61](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/poly_values.h#L23-L61)). The voice handler defines parallel voices as half that width ([voice_handler.cpp lines 22–27](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/voice_handler.cpp#L22-L27)): two voices, each occupying left/right SIMD lanes. The LFO loop is scalar over time but SIMD across those voice/stereo lanes. There is no source-backed basis for calling this an SSE-optimized eight-lane unison algorithm; it is a general two-stereo-voice SIMD graph.

### Routing and promotion to audio rate

An `LfoModule` starts control-rate ([lfo_module.cpp lines 24–30](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/lfo_module.cpp#L24-L30)). When connecting modulation, Vital promotes the source and connection processor only if both source and destination support non-control-rate processing ([sound_engine.cpp lines 139–160](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/synth_engine/sound_engine.cpp#L139-L160)). Unused LFOs start disabled ([synth_voice_handler.cpp lines 370–380](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/synth_voice_handler.cpp#L370-L380)).

Destination support is deliberately selective. Oscillator transpose, tune, level, and phase are created as audio-rate-capable controls, while wave frame, unison controls, pan, distortion amount, and morph controls are not ([oscillator_module.cpp lines 41–63](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/oscillator_module.cpp#L41-L63)). Tytel confirms that oscillator transpose is audio-rate while wavetable position is not ([forum post](https://forum.vital.audio/t/strange-attack-behavior/6387/6)), and that oscillator/sample level and filter cutoff are intended audio-rate destinations ([forum post](https://forum.vital.audio/t/additional-warp-modes/5664/4)).

Audio-rate routing reads the LFO buffer sample by sample. However, route amount and shaping power are linearly interpolated from their previous values to their new block values ([modulation_connection_processor.cpp lines 90–106](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/modulation_connection_processor.cpp#L90-L106), [lines 123–149](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/modulation_connection_processor.cpp#L123-L149)). Control-rate contributions are likewise linearly ramped across the block before audio-rate contributions are added ([operators.cpp lines 216–247](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/operators.cpp#L216-L247)). This is continuity-oriented, but not sample-offset-exact automation.

### Smoothing and oversampling

LFO smoothing defaults on, with an exponential smoothing-time parameter ([synth_parameters.cpp lines 383–390](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_parameters.cpp#L383-L390)). The audio-rate loop applies a one-pole-like exponential interpolation every sample when enabled ([synth_lfo.cpp lines 261–284](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L261-L284)). Tytel explicitly says the default exists because discontinuous LFO modulation of audio-rate level clicks and is confusing to beginners ([forum post](https://forum.vital.audio/t/lfo-smoothing-should-be-off-by-default/3753/5)).

Oversampling is Vital's principal quality lever for nonlinear and audio-rate-modulated paths. Tytel states that higher oversampling improves audio-rate modulation and that the largest difference is from 1× to 2× ([forum post](https://forum.vital.audio/t/getting-best-audio-quality-from-vital/5852/2)). That is practical, but it is not proof of an alias-free LFO. A discontinuous 2,048-point source evaluated at the internal sample rate still has unbounded harmonics.

## Real-time safety audit

Good properties:

- LFO state is fixed-size and audio buffers are sized for the maximum internal block outside steady processing ([processor.h lines 164–172](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/processor.h#L164-L172)).
- Unused modulation sources are disabled, and audio-rate rendering is demand-driven.
- Sample-offset retrigger compensation and control-to-audio state transfer preserve timing and phase.
- Modulation processors are constructed as a fixed bank before use in the effects handler ([effects_modulation_handler.cpp lines 56–73](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/effects_engine/effects_modulation_handler.cpp#L56-L73)).

Concerns that KURV should not reproduce:

- The plugin drains modulation graph changes inside `processBlock` ([synth_plugin.cpp lines 138–166](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/plugin/synth_plugin.cpp#L138-L166)). The connection path calls `plugNext`, whose fallback allocates a `shared_ptr<Input>` and grows containers if no empty input is available ([processor.cpp lines 105–120](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/processor.cpp#L105-L120)). Pre-created banks may make that fallback uncommon, but the source does not establish a hard no-allocation audio-thread invariant.
- UI editing calls `LineGenerator::render()` directly after point changes ([line_editor.cpp lines 178–195](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/interface/editor_components/line_editor.cpp#L178-L195)), and `render()` writes the same buffer returned to DSP ([line_generator.cpp lines 165–213](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/line_generator.cpp#L165-L213)). No immutable snapshot or double-buffer publication boundary is visible in these paths. That is a source-observed race risk, not proof that every Vital build races.
- Default smoothing alters the requested modulation shape and timing. KURV has explicitly rejected hidden smoothing.
- Block-linear frequency/amount/power changes depend on slice length, not host event offsets.

## What is good, acceptable, and unsuitable for KURV

### Keep as architectural ideas

- Demand-driven control/audio-rate promotion per route.
- Disable unused LFOs and skip buffers when all destinations are control-rate.
- Per-voice state for voice destinations plus a separate global evaluation domain.
- Fixed-capacity modulation slots and output buffers.
- Sample-offset retrigger compensation.
- Phase-preserving promotion from control rate to audio rate.
- Explicit destination capability metadata instead of pretending every parameter can be audio-rate safely.
- SIMD across independent voices while iterating sequentially through time.

### Accept only with changes

- **Editable curve compilation:** keep off the audio thread, but publish an immutable curve generation atomically at a slice boundary. Do not mutate the live DSP table in place.
- **Lookup evaluation:** useful for arbitrary free draw, but analytic sine/triangle and compact piecewise-polynomial curves can be cheaper and more accurate. Measure both. A table is acceptable for a modulator only if product semantics allow it.
- **Selective audio-rate destinations:** necessary for CPU, but make capability visible and predictable to users.
- **Control-rate fallback:** evaluate at exact host event slices and hold or explicitly reconstruct according to destination semantics; do not hide a block-size-dependent ramp.
- **Oversampling:** retain as a destination-specific fallback, not the universal answer. Prefer bandlimited discontinuity generation and topology-aware modulation where possible.

### Do not carry over

- GPLv3 implementation code in the ISC codebase.
- Default LFO smoothing or fixed-time click masking.
- Block-ramping all amount and power changes regardless of event timing.
- In-place UI writes to the DSP curve buffer.
- Audio-thread graph mutation that can allocate.
- Unclamped keytrack rates for discontinuous audio-rate shapes.
- SSE2-only design assumptions or the explicit lack of AVX2.
- Treating oversampling as proof of pristine/alias-free audio-rate modulation.

## Clean-room KURV design direction

This is a behavioral design derived from public facts, not a translation of Vital code.

1. **Keep the main generator procedural and bandlimited VA.** LFO implementation choices do not change that contract. For LFO sine use an analytic SIMD oscillator; for triangle/saw/pulse at audible modulation rates, use bandlimited discontinuity events or a destination-aware antialiasing strategy. A freely drawn curve can compile off-thread to immutable piecewise cubic segments or a guarded table.
2. **Separate phase engine from curve evaluator.** Store normalized phase and increment per voice/global domain. Feed the same phase engine into analytic, piecewise-curve, step, sample-and-hold, and random evaluators.
3. **Represent route rate explicitly.** Each destination declares `ControlOnly` or `AudioRate`. Build a fixed-capacity route program on the control/UI side, then publish an immutable snapshot. Promote an LFO to audio rate only while at least one live route requires it.
4. **Make automation sample-exact.** Split processing at host event offsets. Apply frequency, phase, amount, polarity, and route changes on the exact sample. Do not add a five-millisecond fade or hidden block ramp. Phase continuity is state correctness, not smoothing.
5. **Define discontinuities honestly.** A requested step at sample `n` occurs at `n`. If the destination is audio and the desired mathematical signal has a discontinuity, prevent aliasing by bandlimiting the event—not by changing it into an arbitrary time fade.
6. **Publish curves immutably.** Free draw and Bezier fitting happen off-thread. Produce a bounded immutable compiled curve, validate it, then atomically swap generations at a render-slice boundary. Reclaim old generations outside the audio callback.
7. **Bound audio-rate frequency by evaluator.** Free mode may match Vital's useful 0.0078125–512 Hz range, but audio-rate/keytrack mode needs an explicit maximum derived from internal sample rate and waveform bandwidth. Sine can approach Nyquist with guard margin; discontinuous/custom shapes require a much lower bandlimit or bandlimited evaluator. Never silently run an unbandlimited step curve at 100 kHz.
8. **Vectorize by architecture, not brand.** Provide scalar, SSE2/SSE4, AVX2, and NEON kernels behind one audited interface, chosen outside the hot loop. Pack independent voices/routes, preserve deterministic tails, and benchmark old CPUs rather than assuming the widest ISA always wins.
9. **Keep routing RT-bounded.** No allocation, locks, `shared_ptr` ownership changes, vector growth, or curve fitting in audio processing. Reserve all modulation slots and scratch buffers during activation.
10. **Expose the truth in UI.** Show free/tempo/keytrack, retrigger/sync/one-shot/loop, stereo phase, global/polyphonic scope, and whether a destination accepts audio rate. A modulation panel below the envelope can follow Vital's interaction hierarchy without copying its code or artwork.

## License boundary

Vital's README states that its source is GPLv3 and directs proprietary/closed-source users to `licensing@vital.audio` for a separate license ([README lines 6–7 and 20–21](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/README.md#L6-L21)). GPLv3 section 5(c) requires a conveyed modified work as a whole to be licensed under GPLv3 unless separate permission exists ([LICENSE lines 208–228](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/LICENSE#L208-L228)). KURV currently declares `license = "ISC"` ([KURV Cargo.toml lines 1–8](https://github.com/DerpcatMusic/KURV/blob/62f206a59c99dba1f8876207250db69a3b48fc37/Cargo.toml#L1-L8)).

Consequently:

- Do **not** copy, translate, or port Vital's LFO classes, SIMD wrappers, curve evaluator, routing code, or UI implementation into the current ISC KURV tree.
- A combined distributed derivative would need GPLv3 compliance for the whole combined work, or a separately purchased/negotiated license from Vital.
- Clean-room implementation of general DSP concepts and independently specified behavior is the appropriate path while preserving KURV's current licensing intent.

This is an engineering compatibility assessment, not legal advice.

## Bottom line

Vital's biggest win is not a magical LFO formula. It is **conditional work**: unused modulators are off, control-rate is the default, only capable destinations promote a source to audio-rate, state is SIMD-packed across voices, and oversampling is shared with the synthesis engine. KURV can improve on it by retaining those scheduling ideas while using immutable curve publication, strict real-time routing, exact event slicing, architecture-specific kernels, explicit waveform-dependent rate limits, and bandlimited discontinuities—without copying GPLv3 code or hiding defects behind smoothing.
