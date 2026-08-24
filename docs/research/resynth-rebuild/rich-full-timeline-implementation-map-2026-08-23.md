# Rich full source-time frame sequence: implementation map (2026-08-23)

**Status:** read-only implementation map. This report inspects the current Rich
compiler, persisted artifact codec, and audio renderer. No Rust source was
edited for this task.

**Checkout:** `7c53670` (`Refresh final RESYNTH documentation references`).
The source references below are line ranges in this checkout.

## Executive recommendation

The current Rich payload is a fixed bank of eight rendered waveform snapshots.
Increasing `RICH_FRAME_COUNT` is not a full-timeline implementation: the
number of source windows, FFT size, per-frame waveform size, harmonic-bin
limits, codec layout, visual cache, and callback indexing all currently depend
on the value `8`.

Use this migration boundary:

1. **Define “full source-time” first.** Under the existing fixed
   `RICH_ZONE_SAMPLES = 32_768` PCM budget, a literal sample-for-sample source
   timeline is impossible for arbitrary sources. A sequence with more than
   eight full-band 4,096-sample waveform frames either exceeds the budget or
   loses the current 17.2 kHz guard band when its frames are made shorter.
   The implementation should therefore call its contract **full source
   coverage at a bounded temporal resolution**: monotonically ordered frame
   intervals cover `[0, source_frames]`, with a measured maximum number of
   intervals. It must not claim that every source sample or transient is
   recoverable when a source interval is longer than the analysis window.
2. **Keep the one immutable publication boundary.** Put all timeline metadata
   and frame data inside `RichZoneArtifact`, hence inside the existing
   `ResynthRtArtifact` node. Do not publish a frame bank, frame count, or
   positions through separate atomics. The publication protocol already keeps
   generation, revision, and the complete artifact together
   (`src/resynth_state/publication.rs:178-298`).
3. **Do not force a larger frame count into the current PCM slab.** First choose
   the representation that meets the spectral and realtime contract:
   - If the existing rendered-PCM representation and 17.2 kHz guard are
     mandatory, retain eight 4,096-sample snapshots and explicitly label Rich
     as a coarse approximation; or
   - For the requested full source-time sequence, replace the per-frame PCM
     snapshots with a bounded coefficient/parameter sequence (or accept a
     separately approved larger artifact budget). Worker-side synthesis may
     remain FFT-based; callback-side evaluation must remain fixed-work,
     allocation-free, lock-free, and FFT-free.

The map below assumes the second option: a bounded sequence with explicit
source boundaries and a worker-built representation whose storage is sized
from a measured frame cap. It calls out where a PCM-only implementation must
make a different product decision.

## Current data path

### 1. Analysis and compiler

| Concern | Current implementation | Consequence for migration |
| --- | --- | --- |
| Shared analysis cache | `ResynthAnalysisModel.rich_analysis` is `Option<Arc<RichSourceAnalysis>>` (`src/oscillators/resynth/mod.rs:210-220`). It is built once in `analyze_wav_with_visual_cache_and_cancel` (`mod.rs:413-417`) and reused by the Rich compiler. | Keep this worker cache as the source of the immutable sequence. Add its source boundaries and representation version here rather than recomputing during every algorithm selection. |
| Analysis frame shape | `RichAnalysisFrame = (Vec<Complex>, Vec<f64>, f32)` and `RichSourceAnalysis` stores a `Vec` plus one global `source_bin_hz` (`src/oscillators/resynth/artifact/rich.rs:18-24`). There is no persisted frame count, source start, source end, or frame position. | Replace the tuple with a named bounded frame type. Carry a source interval (prefer integer source-frame boundaries) and any per-frame frequency-axis metadata needed by the chosen representation. |
| Source projection | `stride = source_sample_rate / 48_000`, then `window_source_frames = 4_096 * stride` (`rich.rs:45-49`). This preserves the declared 17.2 kHz band when the source rate is high. | The stride policy is useful and should remain explicit. It does not by itself make a source-time sequence. |
| Short source | For sources no longer than one analysis window, `source_span = ceil(source_len / 8)`; this recent change makes short frames distinct (`rich.rs:49-59`). | Preserve the short-source behavior in the new planner and add a parity test. Do not treat this local fix as full-timeline coverage. |
| Long source | For a long source, `source_span` is fixed at one 4,096-sample projected window. Eight starts are `last_start * frame / 7` (`rich.rs:60-71`). The windows leave uncovered gaps for most long sources. | Build one bounded frame plan with explicit ordered boundaries. A test must prove the plan's coverage, not only that first and last spectra differ. |
| Spectral extraction | Each retained window is Hann-windowed, FFT'd, converted to a smoothed log envelope, and assigned a peak gain (`rich.rs:72-99`). | Keep cancellation checks at frame boundaries and inside expensive worker loops. Bound the number and size of analysis frames before allocation. |
| Zone compiler | `render_rich_zone` maps each source envelope to harmonics, splits tonal/residual magnitude, assigns phase, inverse FFTs one 4,096-sample frame, and peak-normalizes it (`rich.rs:102-186`). `compile_from_analysis_with_cancel` writes all eight frames into a fixed `22 x 32,768` slab (`rich.rs:244-300`). | This is the main representation seam. A PCM frame whose size is reduced from 4,096 changes FFT-bin resolution and the 17.2 kHz harmonic limit. Do not merely divide the slab into 16/32/64 pieces without deciding how fractional harmonic bins and high-band content are preserved. |
| Artifact fields | `RichZoneArtifact` has one source rate/length, `center_hz[22]`, `fundamental_bins[22]`, `frame_gains[8]`, `dynamic`, and `slabs[22][32,768]` (`rich.rs:7-16`). | Add `frame_count`, source boundaries, and representation-specific frame data. Keep inactive array entries deterministic if using fixed inline arrays. |
| RT size invariant | `RICH_ZONE_COUNT = 22`, `RICH_ZONE_SAMPLES = 32,768`, `RICH_FRAME_COUNT = 8`, `RICH_FRAME_SAMPLES = 4,096`; the slab is 2,883,584 bytes and is const-asserted below 6 MiB (`src/oscillators/resynth/artifact/shared.rs:51-70`). | Preserve the existing budget only if the new representation can meet it. A 64-frame coefficient sequence may fit, but a 64-frame full PCM sequence at 4,096 samples does not. Record the chosen budget and prove it with a const/test bound. |

The Rich compile is selected in `src/oscillators/resynth/mod.rs:514-579`.
It decodes the authoritative Source Master again, builds the source audition
artifact, then calls either `compile_from_analysis_with_cancel` or the fallback
`compile_with_cancel`. The new sequence must remain worker-built at this same
boundary. It must not make audio decode, FFT, or source analysis work.

### 2. Persisted artifact codec

The pack currently uses `PACK_VERSION = 13`
(`src/resynth_state/codec.rs:9-21`). Rich sizing and serialization are fixed:

- `artifact_persisted_bytes` writes the algorithm/base metadata, source rate and
  frame count, 22 centers, 22 fundamental bins, eight frame gains for pack
  versions with continuous controls, and all `22 * 32,768` slab samples
  (`codec.rs:193-234`).
- `write_artifact` writes those fields in the same order
  (`codec.rs:237-333`), with no Rich frame count or source-position array.
- `read_artifact` allocates an inline fixed slab, reads/validates every sample,
  and calls `RichZoneArtifact::from_persisted` with the hardcoded eight-gain
  shape (`codec.rs:471-524`). It rejects non-finite or over-amplitude values
  (`codec.rs:507-513`).
- The whole pack is bounded by 32 MiB
  (`src/resynth_state.rs:29`, `resynth_state.rs:1333-1403`); an aggregate source
  budget also applies (`resynth_state.rs:30,80-84`). The fixed Rich slab is
  already included in worst-entry sizing (`resynth_state.rs:86-95`).

The migration must bump the pack version (next version, rather than silently
changing v13 field meanings). The v13 reader should remain available and map
its eight snapshots to the new representation with uniform boundaries. New
writes should contain, at minimum:

```text
algorithm, source_root_hz, audition_gain
source_sample_rate, source_frames
rich_representation_version
frame_count, frame_storage_shape
frame_boundaries[frame_count + 1]   // integer source-frame boundaries
center_hz[RICH_ZONE_COUNT]
fundamental_bins[RICH_ZONE_COUNT]
frame_gains[frame_count]            // or coefficient normalization metadata
representation_payload
```

Use an explicit representation tag because a coefficient sequence and a PCM
slab cannot be safely decoded by the same reader. Validate all of the following
before constructing the immutable artifact:

- frame count is within the compile-time cap and nonzero;
- boundaries are monotonic, begin at zero, and end at `source_frames`;
- every active frame has valid finite metadata and any required storage offset;
- all samples/coefficients are finite and inside the existing artifact bound;
- payload byte arithmetic is checked before allocation;
- source rate, source length, root, and algorithm agree with the outer document;
- no old-v13 payload is interpreted as a new frame sequence.

Update `artifact_persisted_bytes` and `worst_resynth_entry_bytes` together. The
new metadata is small compared with the existing PCM slab, but coefficient
frames can change the payload size. Check both per-entry and 32 MiB whole-pack
limits. Round-trip tests must compare a freshly compiled artifact with the
persisted artifact through the actual reader, not just compare a direct
constructor.

### 3. Audio renderer

The current renderer has three layers of eight-frame assumptions:

1. `VaOscillator::advance_rich_timeline` stores two layer phases, steps, and
   generation identities (`src/oscillators/va/mod.rs:62-137`). It advances by
   `source_sample_rate / source_frames / host_sample_rate`, which is a good
   source-duration clock and can remain unchanged for a boundary-normalized
   sequence.
2. `rich_timeline_for_view` obtains the Rich artifact and advances that clock
   (`src/voices/voice/resynth.rs:290-312`). It is called once for each live
   playback-plan layer (`resynth.rs:97,116`). Keep this two-layer plan and do
   not add a second publication path for timeline state.
3. `evaluate_resynth_layer` calls `rich.eval_at_timeline` with
   `phase_increment * RICH_FRAME_SAMPLES` (`resynth.rs:275-285`). Current
   `eval_at_timeline` multiplies normalized timeline phase by eight, selects
   two adjacent frames, and linearly crossfades for the final 1/8 of an interval
   (`rich.rs:407-449`). `frame()` computes an offset using the fixed 4,096
   samples (`rich.rs:465-468`).

The replacement renderer should expose a single artifact-level timeline API,
for example conceptually:

```text
sample = rich.eval_timeline(
    zone,
    oscillator_phase,
    source_frames_per_output,
    normalized_source_position,
    host_sample_rate,
    dynamic,
)
```

The implementation must derive all frame indexing and read-rate factors from
the artifact's active frame shape. No caller may multiply by the old
`RICH_FRAME_SAMPLES` constant after the migration. If boundaries are not
uniform, use a bounded search over the inline boundary array; never allocate or
retry without a bound on the callback.

For a coefficient sequence, the renderer needs a bounded additive or basis
kernel. It must interpolate only adjacent source frames, use the same reflected
or wrapped source-position contract at the end, and maintain phase continuity
when a frame changes. Worker-side FFTs can produce coefficients, but callback
code must not invoke FFT, `Vec`, `Arc` clone/drop, locks, file I/O, or source
codec work. Benchmark the lowest-frequency Rich zone, the highest harmonic
count, zone handover, and two-live-layer publication fade before selecting the
frame cap.

For a PCM sequence, the renderer can retain the existing cubic/anti-aliased
readers only if each frame still has enough samples to represent the declared
harmonic guard. A 32,768-sample slab split into 64 frames gives 512 samples per
frame; this is not equivalent to the present 4,096-point inverse FFT and can
alias or erase high harmonics. This is why PCM splitting is not the recommended
implementation without a measured spectral relaxation.

The note-start phase is currently randomized (`src/voices/voice.rs:1459-1462`).
Keep that behavior unless the product contract requires source position zero at
note-on. Timeline fidelity tests must call an explicit deterministic phase-zero
setup; otherwise a test can mistake phase randomization for a missing source
region.

## Recommended representation and frame planner

### Product contract

Adopt these explicit terms in the implementation and UI documentation:

- **Frame coverage:** active boundaries partition the entire retained source
  interval. There are no endpoint-only starts or implicit gaps in the frame
  plan.
- **Bounded temporal resolution:** the cap is finite. A long interval may still
  contain events shorter than the selected frame resolution that are not
  separately recoverable. This is a known quality limit, not a hidden claim of
  sample fidelity.
- **Full-band contract:** if 17.2 kHz retention remains required, the chosen
  representation must preserve it at every active frame. A short PCM frame
  cannot satisfy this merely because its sample values are finite.

### Planner shape

Create a worker-side planner that receives source length, source rate, and the
existing stride policy and returns:

```text
frame_count <= RICH_TIMELINE_FRAME_CAP
boundaries[0..=frame_count] = [0, ..., source_frames]
analysis windows/centers for each interval
```

Use a bounded cap selected by benchmark. A first candidate is 64 active frames
with inline capacity 64, but this is a candidate only: the coefficient kernel
and callback budget must prove it. If the representation remains PCM, the
minimum frame size and harmonic guard will likely force a much smaller cap.
The planner should choose a deterministic count from source duration (for
example, the smallest allowed count that meets the target analysis span, then
clamp to the cap), use integer arithmetic with checked conversions, and write
exactly the same result on fresh compile and rebuild.

Each analysis frame should have a named structure rather than the current
unnamed tuple, along these lines:

```text
RichTimelineFrame {
    source_start: u32,
    source_end: u32,
    spectrum/envelope/coefficients: bounded worker data,
    measured_gain: f32,
}
RichSourceAnalysis {
    frames: bounded Vec<RichTimelineFrame>,
    source_bin_hz or per-frame frequency axes,
}
```

The worker should analyze the planned intervals in source order, preserve
cancellation checks, and retain enough context for the chosen spectral
resolution. If an interval is wider than one FFT window, explicitly define the
reduction (overlap, pooled envelope, or resampled whole-interval analysis) and
add a transient-resolution test. Do not silently pick one center window and
call that interval complete.

### Why the current slab cannot simply be resized

The current per-frame waveform is a 4,096-point inverse FFT. Its target bin,
source bin, harmonic limit, antialias read rate, and runtime `phase_increment`
all use that frame length (`rich.rs:112-145,349-356,459-462`; `resynth.rs:276-284`).
The high-band test relies on this shape (`src/oscillators/resynth/artifact.rs:59-112`).
If the fixed 32,768 samples are split among more frames, FFT-bin spacing and
Nyquist capacity change. If each frame remains 4,096 samples, storage grows
linearly with frame count and breaks the fixed slab/pack bounds.

Therefore the implementation work is a representation migration, not a
constant change:

- **Preferred:** store bounded per-frame harmonic/phase coefficients or another
  compact full-band basis. Render adjacent frame coefficients in a fixed-work
  callback kernel. Measure coefficient quantization and worst-case harmonic
  work, especially at low Rich zones.
- **Alternative:** increase the Rich artifact/pack budget and retain full-size
  PCM frames. This must be approved as a memory and persistence change; update
  publication retirement and history budgets with it.
- **Rejected as a complete solution:** raise `RICH_FRAME_COUNT` while retaining
  the current fixed slab and FFT formulas. It produces more labels, not a
  faithful full source-time sequence.

## File-by-file change map

### Worker/compiler files

1. `src/oscillators/resynth/artifact/shared.rs`
   - Replace the semantic use of `RICH_FRAME_COUNT = 8` with an explicit
     timeline capacity and representation constants.
   - Keep `RICH_ZONE_COUNT` and the current budget constants until the chosen
     representation has a new measured bound.
   - Add const assertions for array capacity, payload size, and any minimum
     waveform/basis size.
2. `src/oscillators/resynth/artifact/rich.rs`
   - Introduce named bounded analysis/timeline frame types and source
     boundaries.
   - Replace the endpoint-spaced eight-window loop with the deterministic
     planner and complete-coverage boundary validation.
   - Refactor `render_rich_zone` to compile the selected representation and
     active frame count; retain cancellation, finite checks, tonal/residual
     split, and per-frame gain behavior.
   - Add artifact accessors for active count, boundaries, frame storage shape,
     and representation version. Keep callback accessors allocation-free.
   - Replace `eval_at_timeline`/`frame` indexing with boundary-aware adjacent
     interpolation and remove hardcoded eight/4,096 assumptions from callers.
3. `src/oscillators/resynth/mod.rs`
   - Keep the existing `Arc<RichSourceAnalysis>` analysis cache and compile
     dispatch. Update retained-byte accounting and cancellation tests.
   - Ensure fresh compile and root-override rebuild use identical frame plans.
4. `src/oscillators/resynth/visual.rs`
   - This is not audio state, but it is an immediate dependent of the eight
     snapshot shape: `rich_timeline_db` and its accessor are fixed at
     `RICH_FRAME_COUNT` (`visual.rs:336-363,422-441,551-555`). Add active frame
     count/boundaries and render all active frames into a bounded UI cache.
   - Do not make the visual cache an RT artifact or pointer publication input.

### Codec/state files

1. `src/resynth_state/codec.rs`
   - Bump pack version and add a Rich-timeline feature/version predicate.
   - Add the new metadata and representation payload to sizing, writer, and
     reader in exactly the same order.
   - Retain v13 decode as an eight-frame compatibility adapter with uniform
     boundaries. New writes use the new version.
   - Reject malformed counts, boundaries, representation tags, payload sizes,
     NaNs, infinities, and out-of-range values before artifact construction.
2. `src/resynth_state.rs`
   - Update `worst_resynth_entry_bytes` and any source/pack preflight formulas.
   - Keep the 32 MiB pack and aggregate-source checks unless a separate memory
     decision changes them.
   - Add full fresh/persisted parity through `encode -> decode -> artifact
     render`, not only direct `from_persisted` tests.
3. `src/resynth_state/publication.rs` and `build.rs`
   - No new pointer or publication type should be needed. Verify the larger or
     new coefficient payload remains inside one immutable `Arc` artifact and
     that stale worker builds cannot publish a frame sequence after a newer
     revision. Existing two-live-generation acknowledgement remains the fence.

### Renderer files

1. `src/voices/voice/resynth.rs`
   - Replace the `phase_increment * RICH_FRAME_SAMPLES` argument with an
     artifact-derived read-rate/frame-shape API.
   - Keep both playback-plan layers and pass each layer's timeline position to
     the same artifact evaluator.
   - Add scalar tests for frame-boundary, wrap, Rich zone handover, and
     publication crossfade behavior.
2. `src/oscillators/va/mod.rs`
   - The source-duration step is conceptually reusable. Confirm that generation
     reset, two-layer phase handover, and source-rate/sample-count conversion
     remain valid for explicit boundaries.
   - Do not add a dynamic allocation to oscillator state. If positions need
     lookup state, store only bounded scalar indices/phase values.
3. `src/voices/voice.rs`
   - Decide and document whether randomized initial Rich timeline phase remains
     intended. Do not change it as an incidental part of frame migration.

## Test and acceptance map

### Compiler and analysis

- A deterministic source with distinct first, middle, and last regions yields
  an ordered frame sequence whose boundaries are `[0, source_frames]` with no
  gap or reversal.
- A short source, exactly one analysis-window source, and a source longer than
  the cap all produce deterministic plans and finite output.
- A transient placed in each represented interval appears in the expected
  frame-resolution region. A transient shorter than the declared bounded
  resolution is either retained by the chosen representation or explicitly
  documented as unresolved; it must not be silently claimed as guaranteed.
- Existing upper-band/high-harmonic and 110/220/440 Hz root tests run across
  multiple timeline frames, not only the first slab.
- Cancellation is observed during planner, analysis, coefficient compilation,
  and zone loops.
- Retained worker bytes and artifact bytes remain under their explicit bounds.

### Renderer and realtime safety

- Render from deterministic phase zero for one complete source-duration cycle.
  Assert source-region spectral order and approximate transition positions.
- Exercise every allowed frame count, the first/last boundary, and final-to-
  first wrap. Assert finite output and a bounded transition difference.
- Compare scalar rendering with serial block rendering at frame boundaries,
  zone handovers, and two-live-layer artifact fades.
- Run the highest-work low-frequency zone and highest source-to-output read
  rate through a fixed allocation/lock/FFT instrumentation test. The callback
  must perform no allocation, lock, I/O, decode, or FFT.
- Verify one immutable artifact identity carries source boundaries and payload;
  no test should pass by separately publishing timeline metadata.

### Codec and persistence

- New-version fresh artifact round-trips through the real pack writer/reader
  with equal metadata and equivalent rendered output over all timeline regions.
- A v13 fixture decodes to an eight-frame compatibility sequence and remains
  audible; re-saving it upgrades to the new wire version without fabricating
  source positions.
- Corrupt frame count, unsorted/overflowing boundaries, wrong representation
  tag, truncated payload, non-finite coefficient, and over-budget payload are
  all rejected.
- Persisted and freshly compiled artifacts agree for frame count, boundaries,
  gains, root/zone metadata, and rendered transition locations.

## Migration order and stop conditions

1. **Contract/budget decision:** choose coefficient sequence, larger PCM budget,
   or retain eight-snapshot approximation. Record guard frequency, temporal
   resolution, artifact bytes, and callback CPU targets before changing a
   constant.
2. **Planner/model:** add bounded frame metadata and tests without changing the
   sounding renderer. Prove deterministic full-coverage boundaries and worker
   byte bounds.
3. **Worker representation:** compile one representation for all active frames;
   retain existing root, high-band, cancellation, and gain tests.
4. **Renderer:** consume the immutable sequence through one boundary-aware API;
   remove caller-side eight/4,096 arithmetic and prove scalar/block parity.
5. **Codec:** add the new version, v13 adapter, checked sizing, malformed-input
   tests, and fresh/persisted audio parity.
6. **Visual/editor projection:** update the bounded timeline display to expose
   active frame count and source positions.
7. **Publication/stress review:** verify stale revision gating, two-live-layer
   retirement, and no second timeline pointer.
8. **Only after all gates pass:** remove obsolete fixed-eight aliases and update
   docs/UI language. If the representation cannot meet full-band and callback
   budgets, stop and retain the explicit eight-snapshot approximation rather
   than shipping a larger but lower-fidelity label.

## Current validation gap

The current tests prove fixed storage, upper-band presence, root/octave behavior,
and a short-source distinct-frame regression (`src/oscillators/resynth/artifact.rs:59-112,
1533-1574`; `src/oscillators/resynth/artifact/rich.rs:496-523`). They do not
prove full source coverage, persisted timeline metadata, source-region event
ordering, or callback parity for a variable frame sequence. Those are the
required exit criteria for this migration.
