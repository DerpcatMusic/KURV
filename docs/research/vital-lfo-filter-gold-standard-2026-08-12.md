# Vital audio-rate LFO and morphing-filter implementation notes

Research date: 2026-08-12. Source snapshot: Vital commit
[`636ca0ef`](https://github.com/mtytel/vital/tree/636ca0ef517a4db087a6a08a6a8a5e704e21f836),
which is still the official repository HEAD. Vital says the repository trails its binary releases,
so these findings describe that source rather than an unreleased/current commercial build
([README lines 2-7](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/README.md#L2-L7)).
The implementation is GPLv3; KURV may reproduce public behavior and architecture clean-room, but
must not copy the code into its differently licensed source
([README lines 20-26](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/README.md#L20-L26)).

## Bottom line

Vital's useful lesson is not one magic DSP function. It is a strict split between:

- continuous, sample-by-sample modulation inside a fixed topology; and
- discrete model/style selection that is not treated as an audio-rate morph.

KURV should keep its cheaper procedural LFO evaluator, remove every rate-dependent evaluation
shortcut, and adopt Vital's demand-driven source scheduling. For filters, KURV should replace the
separate LP/BP/HP choices with one fixed SVF whose output weights morph continuously, keep all
stages warm while slope/stage count is being modulated, and reserve the picker for genuinely
different algorithms: `SVF`, `PHASER`, and `FIBONACCI`.

## LFO: what Vital actually does

Vital compiles each editable curve to a 2,048-sample guarded buffer
([`line_generator.h` lines 25-35](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/line_generator.h#L25-L35),
[`line_generator.cpp` lines 165-213](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/line_generator.cpp#L165-L213)).
At audio rate it performs four-point Catmull-Rom evaluation for every internal sample
([`synth_lfo.h` lines 80-96](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.h#L80-L96),
[`synth_lfo.cpp` lines 246-288](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L246-L288)).
That makes the *output cadence* genuinely audio-rate. It does not make the curve bandlimited, and
it does not make every LFO input sample-rate: frequency is read once per processing slice, while
phase control is linearly advanced from its previous value across the slice
([`synth_lfo.cpp` lines 397-429](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modulators/synth_lfo.cpp#L397-L429)).

The efficiency comes primarily from scheduling:

- An LFO module starts control-rate
  ([`lfo_module.cpp` lines 24-30](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/lfo_module.cpp#L24-L30)).
- A route promotes the source and connection only when both source and destination support audio
  rate
  ([`sound_engine.cpp` lines 139-160](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/synth_engine/sound_engine.cpp#L139-L160)).
- Unused LFOs are disabled
  ([`synth_voice_handler.cpp` lines 370-380](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/synth_voice_handler.cpp#L370-L380)).
- Oscillator transpose, tune, level, and phase are explicitly audio-rate-capable, while many other
  oscillator controls are not
  ([`oscillator_module.cpp` lines 41-63](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/oscillator_module.cpp#L41-L63)).
- Audio-rate route values are consumed sample by sample, but route amount and shaping power are
  block-linear ramps
  ([`modulation_connection_processor.cpp` lines 90-106](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/modulation_connection_processor.cpp#L90-L106),
  [`lines 123-149`](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/modulation_connection_processor.cpp#L123-L149)).

Vital's `poly_float` is four lanes on SSE2 or NEON and rejects AVX2 in this snapshot
([`poly_values.h` lines 23-61](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/poly_values.h#L23-L61)).
The time loop remains sequential; SIMD covers independent stereo/voice lanes. This is a sound
stateful-DSP layout, but KURV should not inherit Vital's SSE2 ceiling.

## LFO: exact KURV comparison

KURV's current working tree now evaluates every active source directly at every internal sample.
The global fast path calls the 16-segment cubic `WaveCurveRt::eval` and advances its `f64` phase
once per sample ([`src/modulators/lfo.rs`](../../src/modulators/lfo.rs#L910-L989)); per-note LFOs do
the same in each active voice ([`src/modulators/lfo.rs`](../../src/modulators/lfo.rs#L483-L524)).
The render block fillers call modulation advancement inside the internal/oversampled-sample loop,
not once per host block ([`src/runtime/render.rs`](../../src/runtime/render.rs#L212-L240)). Dynamic
rate and phase automation is read per host sample and held only across that host sample's
oversampling substeps ([`src/modulators/lfo.rs`](../../src/modulators/lfo.rs#L1012-L1074)).

This removes the reported pitch-dependent engine seam. At repository HEAD, the global direct path
sometimes evaluated a curve only at endpoints and linearly interpolated for 2-64 samples. The
stride was derived from a `1/1024`-cycle phase-span threshold, so crossing specific rates changed
the evaluator and its error characteristic. The current working tree removes that entire temporal
interpolation state.

KURV's direct curve evaluation is already smaller than Vital's table lookup: phase selects one of
16 compiled cubic segments and evaluates three multiply-add stages
([`src/wave_curve.rs`](../../src/wave_curve.rs#L243-L297)). Replacing it with Vital's 2,048-sample
table would add memory and licensing risk without fixing aliasing. If audible curve-shape error
remains after the temporal seam removal, increase the compiled segment count or provide exact
analytic primitives; scalar evaluation cost remains one segment lookup plus one cubic either way.

The missing Vital-style optimization is destination capability. KURV masks unused sources, but a
connected source is generally advanced at internal sample rate even for destinations that need
only slow control. Add explicit `AudioRate` versus `ControlRate` destination metadata and promote
only the connected sources that require it. Oscillator pitch/phase/level, filter cutoff, and other
audibly sensitive paths should remain audio-rate. Layout, voice count, picker/model, and similar
structural controls should remain bounded control-rate operations.

## Filter morph and slope in Vital

Vital exposes filter `model` and `style` as indexed controls, while `blend` is continuous
([`synth_parameters.cpp` lines 418-438](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_parameters.cpp#L418-L438)).
All model processors are created up front; selecting a model enables exactly one and resets it when
the selection changes
([`filter_module.cpp` lines 31-58](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/filter_module.cpp#L31-L58),
[`lines 180-215`](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/modules/filter_module.cpp#L180-L215)).
Therefore the model picker is not an audio-rate crossfade. The continuous behavior lives inside
each fixed model.

For the ordinary digital SVF, Vital clamps `pass_blend` to `0..2`, recenters it to `-1..1`, and in
the normal style computes:

```text
band = sqrt(1 - morph^2)
low  = max(-morph, 0)
high = max( morph, 0)
```

The resulting state-variable output weights are then derived without switching filter state
([`digital_svf.cpp` lines 224-313](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/digital_svf.cpp#L224-L313)).
Thus `-1`, `0`, and `+1` are LP, BP, and HP, with a continuous curved blend between them. KURV
should adopt this behavior clean-room and delete LP/BP/HP as separate picker entries.

Vital's slope is not continuously modulated. `12 dB` runs one SVF; `24 dB` runs a non-resonant
pre-SVF followed by saturation and the resonant SVF
([`digital_svf.cpp` lines 57-65](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/digital_svf.cpp#L57-L65),
[`lines 320-380`](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/digital_svf.cpp#L320-L380)).
Cutoff is the special audio-rate filter control; its normalized coefficient is obtained from a
2,048-entry lookup with cubic interpolation on every sample
([`digital_svf.h` lines 32-39](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/digital_svf.h#L32-L39),
[`digital_svf.cpp` lines 68-95](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/digital_svf.cpp#L68-L95)).
Resonance, drive, blend weights, and output gain ramp linearly over the processing slice.

Before this redesign, KURV dispatched LP/BP/HP/peak/notch as separate modes, overloaded `Q` for
phaser stage count, and recomputed `tan()`-based coefficients at a rate-dependent stride of 1-64
samples. That made both response semantics and modulation quality change across hidden thresholds.

The current implementation uses one fixed two-stage TPT SVF for the continuous LP/BP/HP morph and
a fixed four-stage second-order allpass bank for Phaser/Fibonacci
([`src/filters/svf.rs`](../../src/filters/svf.rs)). `Q` remains resonance/damping, `Morph` owns
response or warm-tap selection, and `dB/oct` owns slope/density. Every active filter-modulation
sample now computes from a prepared normalized coefficient lookup; the table's `tan()` runs during
prepare, and bounded fast exponent approximations replace real-time `powf()` calls. There is no
rate-dependent coefficient cadence or filter-algorithm swap.

## Phaser topology

Vital's phaser preallocates twelve one-pole allpass stages, always processes all twelve, and saves
the outputs after stages 4, 8, and 12
([`phaser_filter.h` lines 33-35](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/phaser_filter.h#L33-L35),
[`lines 109-145`](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/phaser_filter.h#L109-L145)).
`pass_blend` linearly selects those three warm taps: `0` gives 4 stages, `1` gives 8, and `2` gives
12. Feedback is spectrally bounded by extra low/high clearing stages, and the final output mixes
dry with the selected allpass output; style flips polarity to swap notch/peak behavior
([`phaser_filter.cpp` lines 59-73](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/phaser_filter.cpp#L59-L73),
[`phaser_filter.h` lines 105-120](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/filters/phaser_filter.h#L105-L120)).

This is the right topology rule for KURV: never add/remove live stages during modulation. Tick a
fixed maximum, crossfade adjacent saved taps, and give every stage persistent state. `Q` controls
allpass damping/phase sharpness, `Cutoff` controls the allpass center, `Morph` selects the warm stage
taps, and `dB/oct` controls the fixed bank's density/range rather than reallocating or branching the graph.
The same core can produce a Fibonacci variant by assigning deterministic Fibonacci-spaced
allpass centers; Vital itself contains no Fibonacci filter precedent.

## Clean-room implementation contract for KURV

1. **LFO:** keep `WaveCurveRt`, direct per-internal-sample evaluation, `f64` phase, active-source
   masks, immutable compiled curves, and the current no-allocation path. Do not restore a
   rate-dependent interpolation shortcut. Add destination-driven control/audio-rate promotion only
   after the current correctness build is stable.
2. **SVF:** one model, continuous `LP -> BP -> HP` morph using the weight law above. `Q` affects
   resonance only. Start with a fixed two-stage cascade and warm 12/24 dB taps; crossfade taps for
   modulated slope instead of changing topology. Add 36/48 dB only after an actual CPU budget proves
   it viable.
3. **Phaser/Fibonacci:** use a fixed, preallocated allpass bank. Always advance every enabled-bank
   stage, morph between saved taps, bound feedback, clear denormals, and never reset state because a
   continuous knob crossed an integer stage boundary.
4. **Picker:** `SVF`, `PHASER`, `FIBONACCI` are discrete and not audio-rate modulation targets.
   Their four common side controls are `Cutoff`, `Q`, `dB/oct`, and `Morph`; each model maps those
   controls to its own fixed topology.
5. **CPU contract:** bypassed modules and unused sources cost effectively zero; enabled modules
   have bounded O(active voices x fixed stages) work, no allocation, locks, I/O, or graph mutation.
   Audio-rate cutoff uses a prepared normalized lookup; slow controls use ramps. SIMD should run
   across independent stereo/voice lanes, never across recursive time samples.

This is stricter than Vital where KURV needs it: no source-rate engine swap, no audio-rate model
switch, no stale stage state, and no claim that cubic curve interpolation by itself is alias-free.
