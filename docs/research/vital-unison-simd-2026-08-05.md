# Vital unison/SIMD audit for KURV

Date: 2026-08-05
Scope: Vital's public source, commit `636ca0ef517a4db087a6a08a6a8a5e704e21f836`, compared with the live KURV working tree. No Vital code was copied and no DSP code was changed.

## Bottom line

Vital's advertised "efficient unison" is real, but it is not a secret higher-width instruction or an alias-reduction algorithm. The published x86 engine is a hand-written **SSE2, four-lane data model**. Its important trick is what it puts in those four lanes:

```text
lane:       0             1             2             3
meaning: note A / left  note A / right note B / left  note B / right
unison:   detune down     detune up     detune down     detune up
```

One SSE operation therefore advances and samples one sharp/flat unison pair for **two polyphonic notes at once**, while the result is already arranged as two stereo pairs. Vital then iterates over unison pairs and over a whole audio chunk. This is the most substantive meaning visible in source behind Vital's official claim that it uses "clever SSE optimizations" for unison. The official page makes the claim but does not describe the implementation. [Vital feature page](https://vital.audio/) [lane constants](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_constants.h#L149-L159) [parallel-note packing](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/voice_handler.cpp#L874-L895) [unison detune packing](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L610-L634)

KURV is **not missing SSE**. It already renders eight unison lanes together through `f32x8` and four-lane tails through `f32x4`; with the current x86-64-v3 build, `wide::f32x8` is one 256-bit AVX vector and `mul_add` uses FMA. Without AVX it becomes two four-lane vectors. This is wider than published Vital, whose AVX2 branch deliberately fails compilation. [`src/voice.rs`, SIMD banks](../../src/voice.rs#L939-L1103) [`src/oscillator.rs`, eight-wide phase/render path](../../src/oscillator.rs#L137-L255) [Truce SIMD aliases](https://github.com/truce-audio/truce/blob/052c1d6160882c0bf4da5587531e2ef342f815f9/crates/truce-simd/src/lib.rs#L27-L33) [`wide` 0.7.33 AVX/fallback layout](https://github.com/Lokathor/wide/blob/7a18c367fbfbf7980f75d89f1854818022b5d0d9/src/f32x8_.rs#L3-L12) [`wide` FMA dispatch](https://github.com/Lokathor/wide/blob/7a18c367fbfbf7980f75d89f1854818022b5d0d9/src/f32x8_.rs#L626-L640) [Vital SIMD selection](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/poly_values.h#L23-L38)

The direct transferable opportunity is therefore **layout and loop ordering**, not “add SSE.” Vital keeps phase in packed SIMD-shaped state and renders an event-free chunk with the unison-pair loop outside the sample loop. KURV keeps scalar phase records, assembles/disassembles vectors, and batches only two time samples. A real packed-state, block-major prototype is worth measuring. Merely widening arithmetic, copying Vital's wavetable core, or turning on unsafe fast-math is not.

## Source provenance and limits

- Vital's own README says the public repository trails binary releases and is GPLv3. The audit can identify ideas, but KURV must use a clean implementation and must not paste or adapt GPL implementation text unless the resulting distribution deliberately accepts the GPL obligations or obtains a separate license. [Vital README and licensing](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/README.md#L1-L21)
- A full remote-ref and history check on 2026-08-05 found one branch (`main`), no tags, and 17 commits. The oscillator and SIMD files have only the February 2021 source-import commit; later commits edit the README. There is no public per-feature history from which to recover a newer unison implementation. [oscillator file history](https://github.com/mtytel/vital/commits/main/src/synthesis/producers/synth_oscillator.cpp) [SIMD file history](https://github.com/mtytel/vital/commits/main/src/synthesis/framework/poly_values.h)
- The checked-in Linux project identifies itself as Vital 1.0.6. It is evidence for that public engine, not proof that current commercial Vital binaries use exactly the same implementation. [Linux release project](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/plugin/builds/linux_vst/Makefile#L75-L106)
- The live KURV comparison used working-tree blob hashes `voice.rs 9a6bf518...`, `oscillator.rs ea63cc67...`, `Cargo.toml a6ff0e71...`, and `Cargo.lock 4a2a1973...`. Those files were concurrently under active development, so the KURV line links describe this snapshot rather than the old Git commit.

## What Vital actually vectorizes

### SIMD backend and lane width

`poly_float` and `poly_int` wrap native SIMD intrinsics. Published x86 selects SSE2 (`__m128` / `__m128i`) with four 32-bit lanes; ARM selects four-lane NEON. A nominal AVX2 branch declares eight lanes but immediately `static_assert(false, "AVX2 is not supported yet.")`. [integer backend](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/poly_values.h#L23-L61) [float backend](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/poly_values.h#L437-L500)

The Linux build explicitly requests only `-msse2`, then combines `-Ofast`, LTO, `-ffast-math`, GCC tree vectorization, SLP vectorization, and loop unrolling. SSE2 multiply-add is two instructions; only the disabled AVX2 branch has FMA in this source. [top-level SIMD flags](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/Makefile#L17-L27) [release flags](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/plugin/builds/linux_vst/Makefile#L102-L107) [multiply-add implementation](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/poly_values.h#L552-L577)

There is no runtime SSE/AVX multiversioning in the oscillator. Startup only rejects x86 machines without SSE2; AVX2 machines pass because they also support SSE2. [startup check](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/startup.cpp#L32-L37)

### Two notes and a stereo pair in every vector

Vital defines `kParallelVoices = poly_float::kSize / 2`, so SSE processes two synth notes per aggregate voice. It assigns masks `[true,true,false,false]` and `[false,false,true,true]`, while the audio convention is `[left,right,left,right]`. [voice count](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/voice_handler.cpp#L24-L30) [aggregate construction](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/voice_handler.cpp#L874-L895) [stereo/voice masks](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_constants.h#L149-L159)

Within each oscillator, Vital stores eight SIMD phase vectors for at most 16 unison oscillators. Each phase vector represents a down/up detune pair for both packed notes. `setPhaseIncMults()` alternates the sharp/flat mask and computes reciprocal ratios, producing the repeated pattern `[down, up, down, up]`. [maximum and array dimensions](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.h#L156-L161) [packed state arrays](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.h#L276-L305) [detune construction](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L610-L634)

This layout avoids a separate pan multiply for every unison oscillator. The down/up members of each pair accumulate directly into left/right lanes; one post-pass equal-power stereo blend crossfeeds the channels to implement the Stereo Spread control. That economy is coupled to Vital's symmetric pair stereo model. [detuned accumulation](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L288-L343) [stereo post-blend](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L866-L888)

Odd unison counts are padded to an even active count, keeping the hot loop pair-shaped instead of taking a scalar tail. The public parameter is limited to 1-16 unison voices. [even padding](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1210-L1217) [parameter limit](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/common/synth_parameters.cpp#L472-L483)

### Block ordering and control-rate work

Vital computes one phase-increment buffer for the audio chunk, vectorized across the two packed notes. MIDI pitch, transpose, tune, and phase modulation can still vary sample by sample. Detune multipliers, unison count, normalization, wavetable-buffer selection, and waveform-specialization dispatch happen outside the innermost sample loop. [phase-increment buffer](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1295-L1335) [block setup and dispatch](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1354-L1438)

For the oscillator core, the loop order is effectively:

```text
for each packed pair of active synth notes:
  prepare one event-free chunk
  for each SIMD pack of unison pairs:
    keep packed phase/multipliers live
    for each sample in the chunk:
      advance packed phase
      sample/interpolate four oscillator lanes
      accumulate into packed stereo output
```

`processChunk()` determines how many SIMD phase packs are needed, processes all detuned packs, and processes the center pack last so normalization can be fused with the accumulated result. [chunk loop](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L1440-L1507)

This is structurally different from KURV. KURV's current `render_pair()` handles one note and only two internal time samples, then the synth loops to the next note. It does share shape dispatch, phase-step setup, gains, and reductions across those two samples, which is why it is already much better than the old single-sample path, but it cannot retain vector phase/gain state over a host-sized chunk. [`src/voice.rs`, pair render](../../src/voice.rs#L1110-L1303) [`src/voice.rs`, synth pair loop](../../src/voice.rs#L2188-L2234)

### Phase and frequency representation

Vital uses a wrapping 32-bit fixed-point phase: one complete cycle is `2^32`. Table index and fractional interpolation position come from bit partitions of that integer. This makes phase wrap an integer overflow and makes four independent lookup indices natural SIMD integers. [phase constants and index interpolation](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L28-L36) [index/fraction extraction](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L205-L224)

KURV stores `f32` phase and wraps with compare/blend. Its procedural BLEP/BLAMP kernels already need normalized floating phase and phase step, so changing to integer phase would add int-to-float conversion to the dominant arithmetic. Integer phase is therefore not an automatic win for KURV even though it is an excellent match for Vital's table lookup. [`src/oscillator.rs`, phase state and scalar wrap](../../src/oscillator.rs#L44-L130) [`src/oscillator.rs`, packed phase advance](../../src/oscillator.rs#L374-L433)

Vital uses a SIMD polynomial approximation to `exp2` for audio-rate pitch conversion. KURV's normal static path already caches per-lane phase steps and computes detune ratios when the unison layout changes, so copying an approximate exponential into the oscillator loop would solve a cost KURV does not normally pay. [Vital fast `exp2`](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/futils.h#L24-L52) [`src/voice.rs`, phase-step cache](../../src/voice.rs#L1388-L1403)

## Antialiasing and why it is separate from SSE

SIMD does not make Vital cleaner; it makes multiple equivalent oscillator evaluations run together. Vital's low-alias claim comes from a different design:

1. Wavetable frames contain 2,048 samples and stored frequency-domain data. [wave-frame dimensions](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/lookups/wave_frame.h#L25-L55)
2. The oscillator derives a highest allowed harmonic from phase increment, zeros higher bins, runs an inverse transform into preallocated buffers, and wraps guard samples. [harmonic cutoff](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L766-L803) [bin truncation and inverse transform](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/spectral_morph.h#L35-L68)
3. Per sample, it uses four-point Catmull interpolation across the selected band-limited buffer. [interpolator](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L218-L243)
4. If spectral/frame spread is disabled, Vital computes one band-limited buffer and shares its pointer across unison pairs. It only builds separate buffers when per-unison spectral differences require them. [spectral-unison sharing](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L821-L863)

That is a wavetable plus runtime Fourier-buffer architecture. It is deliberately outside KURV's procedural core constraint. KURV's comparable conceptual optimization is already present: immutable spectral tables are shared globally, harmonic selection is cached, and the universal Spline path uses direct compact event corrections rather than reconstructing a table. The existing measured mipmapped-wavetable comparator was 1.90-2.38x slower at 32-note workloads despite excellent static alias residue, so Vital's table architecture is not evidence that a table rewrite will beat KURV's current procedural kernel. [KURV generator investigation](generator-dsp-cpu-quality-2026-08-04.md#rejected-or-inconclusive-experiments)

Vital's official statements about a low noise floor and sharp Nyquist cutoff are product claims, not measurements supplied by the source repository. The harmonic truncation explains how the design targets that result, but this audit did not produce a Vital binary-versus-KURV spectral measurement. [Vital feature page](https://vital.audio/)

## Unison level and stereo normalization

Vital distinguishes one center phase pack from the remaining detuned phase packs. The Blend parameter interpolates their relative levels, then `1 / sqrt(center^2 + detuned^2 * (pair_count - 1))` normalizes the energy. Stereo Spread is a later equal-power crossfade between original and swapped stereo lanes. [amplitude normalization](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L716-L734) [stereo crossfade](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp#L866-L888)

KURV precomputes arbitrary per-lane detune, pan, level-curve weight, random/alternate/X geometry, equal-power left/right gains, and an energy normalization when the layout changes. This supports a substantially richer stereo distribution than Vital's symmetric pair plus global spread. Replacing it wholesale with Vital's scheme would reduce work but would also delete audible KURV behavior. [`src/voice.rs`, layout rebuild](../../src/voice.rs#L294-L358) [`src/voice.rs`, lane geometry](../../src/voice.rs#L361-L421)

## Exact 16-note x 64-lane comparison

The two engines do not expose the same workload:

- Published Vital is capped at **16 unison oscillators per oscillator**. [source constant](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.h#L156-L160)
- KURV's target is **16 simultaneous notes x 64 unison lanes = 1,024 procedural oscillator lanes per internal sample**.
- At KURV's exact target, every eight-lane vector is full: eight AVX vectors per note, 128 vector groups across 16 notes. Cross-note packing or padded tails cannot improve SIMD occupancy in that cell.
- Vital at its published maximum performs eight four-lane phase groups per packed pair of notes. Across 16 notes that is 64 SSE groups per sample, but only 256 oscillator lanes total. Extrapolating its CPU percentage to 1,024 lanes is invalid.

KURV's current renderer has already produced a 19.2-29.3% improvement over its frozen pre-fusion baseline across the exact 16 x 64 matrix while remaining bit-identical in a 105-case sweep. That result came from fusing two temporal samples around the existing eight-lane core, not from changing oscillator truth. [measured KURV matrix](generator-dsp-cpu-quality-2026-08-04.md#converged-16-note-x-64-lane-result)

## Ranked opportunities for KURV

| Rank | Experiment | Likely CPU value at 16 x 64 | Sound risk | Why / constraint |
|---:|---|---|---|---|
| 1 | **True block-major packed-phase renderer**: keep each `f32x8` phase, step, and gain vector live for an event-free fixed chunk (for example 8-32 internal samples), writing to fixed preallocated L/R scratch. Segment at MIDI/parameter events. | Medium to potentially large; it attacks repeated phase assembly, gain loads, shape dispatch, and tiny two-sample call overhead without reducing oscillator quality. | Low if operation order per lane and sample is preserved; implementation risk high around envelopes, oversampling, Swarm, and sample-accurate events. | This is the main transferable Vital idea. It is a loop/data-layout experiment, not a naive extension of `render_pair()`. The earlier 3/4-sample pair extensions were neutral/slower, so the experiment only earns its code if the entire hot loop is reordered and benchmarked. |
| 2 | **Packed phase/state bank**: store the 64 phases in eight persistent SIMD packs, or prove from assembly that the present one-field `VaOscillator` array already compiles to equivalent packed loads/stores. Keep a single source of truth. | Medium ceiling in the phase/setup portion; zero if LLVM already coalesces the loads/stores. | Low, with bit-exact wrap required. | Vital stores `phases_[pack]` directly. KURV currently constructs a vector then converts it back to an array on every advance. [`advance8_pair`](../../src/oscillator.rs#L422-L433) |
| 3 | **Block-hoisted exact stereo specialization** for configurations mathematically representable as symmetric hard-panned pairs plus one global spread. Use one packed stereo accumulator and skip arbitrary per-lane L/R gains only under that exact predicate. | Medium inside matching presets; zero elsewhere. | Zero only if the predicate is exact; high if made the default because it would remove KURV's pan-shape/random/X behavior. | Vital gets this saving from its product model. A per-sample/per-lane branch is likely slower; dispatch must be once per block. Prior KURV hot-path branch experiments already regressed packed cells. [rejected branch evidence](generator-dsp-cpu-quality-2026-08-04.md#rejected-or-inconclusive-experiments) |
| 4 | **Pad non-multiple unison counts to a zero-gain SIMD pack** instead of taking four/scalar tails. | Small for counts such as 9-11 or 13-15; **none at 64**. | Low if dormant phase activation is defined. | Vital pads odd counts to an even pair. KURV already has 8-wide, 4-wide, then scalar tails, so the remaining ceiling is small. |
| 5 | **Cross-note SIMD packs** for low-unison presets. | None at 64 lanes; potentially useful at 1-4 unison where KURV's within-note vectors are underfilled. | Low DSP risk but high voice-engine/MPE/legato complexity. | Vital gets two notes per SSE vector. At the requested dense target KURV already has full AVX lane occupancy, so this is not the answer to the current DUNE gap. |
| 6 | **Initialization-time ISA dispatch or separate generic/v3 artifacts**, never an inner-loop feature test. | Preserves the current AVX/FMA win on capable CPUs while retaining compatibility; no new win on the already-v3 build. | None. | KURV already has the wider implementation. The product task is making sure the host loads the intended v3 artifact, not adding SSE intrinsics. |
| 7 | **Fixed-point phase prototype** only if hardware counters show float wrap/index work is material after ranks 1-2. | Uncertain and probably low for procedural Spline. | Medium numeric/sound risk. | Fixed phase is ideal for Vital's table index. KURV must convert it back to float for every BLEP/BLAMP evaluation, so the benefit may disappear. |
| 8 | **Vital-style vector `exp2` in the audio loop**. | Negligible in KURV's static 16 x 64 path. | Medium accuracy risk. | KURV already caches detune/phase steps. Optimize the work that profiling shows, not a function Vital needs for audio-rate pitch modulation. |

## Already present in KURV

- Eight-wide and four-wide oscillator evaluation, vector phase wrap, vector BLEP/BLAMP arithmetic, FMA accumulators, and horizontal reduction. [`src/oscillator.rs`](../../src/oscillator.rs#L137-L433) [`src/voice.rs`](../../src/voice.rs#L939-L1103)
- Parameter/layout-time detune ratios, pan gains, and energy normalization rather than per-sample exponentials and square roots. [`UnisonLayout::rebuild`](../../src/voice.rs#L326-L358)
- A waveform-specialized fast path and a fused two-temporal-sample path. [`render`](../../src/voice.rs#L937-L1103) [`render_pair`](../../src/voice.rs#L1110-L1303)
- Shared immutable spectral banks plus cached harmonic-row selection for the optional Spectral mode. [`src/oscillator.rs`](../../src/oscillator.rs#L686-L852)
- Release thin-LTO and one codegen unit; the tuned artifact additionally enables x86-64-v3 so `f32x8` is AVX and `mul_add` is FMA. [`Cargo.toml`](../../Cargo.toml#L70-L88) [`wide` backend selection](https://github.com/Lokathor/wide/blob/7a18c367fbfbf7980f75d89f1854818022b5d0d9/src/f32x8_.rs#L3-L12)

## Techniques not to transfer

- **Do not copy GPL code.** Reimplement only independently described layout concepts. [Vital licensing](https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/README.md#L6-L21)
- **Do not call SSE an antialiasing technique.** Vital's spectral truncation and tables produce its cutoff; SIMD is throughput only.
- **Do not replace the procedural core with Vital's wavetable/IFFT path.** It violates KURV's stated architecture and the local comparator was slower at dense polyphony.
- **Do not copy `-Ofast/-ffast-math` blindly.** KURV's requirement is truthful, repeatable output. Explicit vector operations and `mul_add` expose the useful contraction without globally permitting reassociation, NaN/Inf assumptions, or other accuracy changes.
- **Do not replace KURV's stereo geometry with symmetric hard-panned pairs by default.** That would benchmark a cheaper, different sound.
- **Do not prioritize cross-note packing for the 64-lane target.** It solves SIMD underfill, and the target has no underfill.

## Recommended next experiment

The one Vital-derived branch worth spending serious time on is a **clean-room block-major `f32x8` renderer**:

1. Keep the existing Spline/Spectral equations and all 64 independent lane phases.
2. Split process blocks only at sample-accurate MIDI or parameter discontinuities.
3. Prepare fixed-size envelope, pitch, Swarm ratio, and left/right gain ramps for the segment in preallocated storage.
4. For each eight-lane unison pack, load phase/step/gain once, render 8-32 time samples, and write into fixed L/R accumulators.
5. Preserve current accumulation order for a bit-exact mode; if a reordered reduction is faster, treat its null/error as an explicit sound result.
6. Compare against the frozen exact 16-note x 64-lane binary for Swarm off/Wander/Jitter and every AA mode before retaining code.

This experiment targets the part Vital genuinely organizes better while retaining KURV's procedural oscillator and richer unison stereo model. Everything else in the advertised SSE story is already present, narrower than KURV, tied to Vital's wavetable design, or irrelevant to the exact 64-lane workload.
