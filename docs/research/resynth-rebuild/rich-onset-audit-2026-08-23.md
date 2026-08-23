# Rich mode timeline and Grain onset audit (2026-08-23)

**Status:** read-only audit. The report was created before source inspection. No
Rust source was edited for this audit.

## Scope and validation

This audit covers the current Rich renderer, the Grain onset/pitch gate, and
matching tests and research notes. The relevant implementation is at commit
`64f30e5` (`Integrate bounded spectral pitch frames into grain rendering`). The
working tree also contained unrelated in-progress Rust changes from other
agents; they were not changed here.

Focused validation from the current tree:

```text
cargo test rich_ --lib -- --test-threads=1
7 passed, 0 failed
```

The passing tests cover fixed Rich storage, stationary high-band retention,
root/octave behavior, and Rich zone handover. They do not cover a changing
source timeline or audio around a Grain onset.

## Executive summary

- **Rich timeline fidelity remains a P1 gap.** Rich analyzes exactly eight
  short windows and stores eight reconstructed frames. It does not retain a
  full source-time sequence. Short events can be missed, and long-source
  windows leave uncovered source intervals. The runtime then treats the eight
  frames as uniformly spaced and loops them.
- **The Rich timeline coordinate is not fully aligned with the analyzed
  windows.** Analysis places window starts from `0` through
  `source_len - source_span` using seven intervals, while runtime places frames
  at eight cyclic phases (`0/8` through `7/8`). This is a timing/model mismatch,
  not only a low frame-count problem.
- **Grain onset continuity is partly improved but not proved.** The current
  tree interpolates onset metadata in `frame_at_render` (commit `64f30e5`), so
  the old “nearest onset lookup” criticism in `phase2-review-2026-08-23.md` is
  stale. However, family/confidence data still uses nearest-frame selection,
  and the dry/tuned PCM crossfade has no end-to-end continuity test or bounded
  slew guarantee.
- **One small coordinate bug is still visible:** Grain source reads reflect
  positions at the source boundary, but onset lookup uses the unreflected
  position clamped to `[0, 1]`. A grain crossing an edge can therefore select a
  different onset/tuning state from the sample it reads.

## Rich mode: timeline evidence

### Current data path

1. `rich_source_analysis_with_cancel` (`src/oscillators/resynth/artifact/rich.rs:40-90`)
   sets `source_span` to at most `RICH_FRAME_SAMPLES * stride`. At the normal
   48 kHz rate this is 4,096 source frames, about 85.3 ms.
2. It creates exactly `RICH_FRAME_COUNT` frames (`rich.rs:51-84`). Each frame
   is an FFT of one window, followed by a log envelope.
3. The window starts are
   `last_start * frame / (RICH_FRAME_COUNT - 1)` (`rich.rs:56`). For a 48 kHz,
   48,000-frame source the starts are approximately
   `0, 6,272, 12,544, 18,816, 25,088, 31,360, 37,632, 43,904`.
   Each window is only 4,096 frames, leaving approximately 2,176-frame gaps
   between most windows.
4. `render_rich_zone` reconstructs one slab containing eight 4,096-sample
   frames (`rich.rs:116-171`). Each frame is independently peak-normalized.
5. `eval_at_timeline` chooses only two adjacent slab frames and crossfades
   during the final eighth of each frame interval (`rich.rs:397-439`).
   `eval_morphed_with_dynamic` derives the timeline from source duration
   (`rich.rs:380-392`), while the live voice advances an eight-phase timeline
   using `source_sample_rate / source_frames / host_sample_rate`
   (`src/oscillators/va/mod.rs:97-118`,
   `src/voices/voice/resynth.rs:275-285,291-311`).

### Concrete fidelity failures

- For any source at or below 4,096 retained frames, `last_start == 0`; all
  eight analysis frames use the same complete source window. A 20 ms source
  containing different material at its beginning and end therefore produces a
  static Rich timbre, despite having a nonzero source timeline.
- For a longer source, material inside a gap is never analyzed. A transient at
  5,000 frames in the 48,000-frame example above is outside both the first and
  second windows. The reconstructed timeline cannot reproduce it.
- Analysis and playback use different frame coordinates. The final analysis
  window begins at about 0.915 source seconds, but the runtime starts frame 7
  at phase 7/8 (0.875 seconds). The boundary handover is consequently not a
  faithful source-time boundary.
- Every frame is peak-normalized (`rich.rs:167-170`) and then optionally
  rescaled by a coarse per-frame gain (`rich.rs:249-256,427-439`). This is
  bounded, but it can reshape the amplitude envelope of short events. No test
  currently checks that event order, event timing, and level survive.
- Note initialization intentionally randomizes the Rich timeline phase
  (`src/voices/voice.rs:1459-1462`). That may be a useful pad behavior, but it
  is incompatible with an exact note-on-at-source-zero or host-synchronized
  timeline unless the product contract says otherwise.

### Existing coverage versus required coverage

Existing Rich tests:

- `src/oscillators/resynth/artifact.rs:59-112` checks fixed storage and high
  spectrum for a stationary broadband source.
- `artifact.rs:1434-1475` compares high-harmonic deltas for short and long
  stationary sources.
- `artifact.rs:1543-1577` checks root/octave behavior at 110, 220, and 440 Hz.
- The voice test checks bounded phase-continuous zone handover, not source-time
  fidelity.

The research notes already identify this limitation:

- `docs/research/open-source-granular-spectral-resynthesis-2026-08-21.md:218-232`
  says the current Rich path samples at most eight windows and does not retain
  sample-time regions.
- `docs/research/resynth-rebuild/phase2-review-2026-08-23.md:152-167`
  requires a first/middle/last-region timeline test and calls the eight-frame
  path a coarse approximation unless it is replaced.
- `docs/research/resynth-rebuild/analysis-artifact-audit-2026-08-23.md:17-18,42`
  confirms that Rich has eight fixed 4,096-sample analysis frames.

### Smallest safe Rich fix

There is no safe one-line DSP fix that turns this artifact into full
sample-long resynthesis. The smallest safe sequence is:

1. Add a worker/compiler regression test using distinct first, middle, and last
   regions, plus a short transient placed in a known analysis gap. Render the
   timeline through `eval_at_timeline` and assert event order, approximate
   event phase, and finite/bounded output. Add a short-source case at 4,096
   frames or less.
2. Make a product decision. If eight snapshots are intentional, label Rich as
   a bounded coarse timeline approximation and make the UI/visualization use
   the same eight-frame artifact. Do not claim full source-time fidelity.
3. If source-time fidelity is required, replace the eight-snapshot model with a
   bounded worker-built frame sequence. Carry an explicit frame count and
   source positions in the immutable artifact; interpolate using those
   positions. Keep callback work fixed and allocation-free. Select a measured
   maximum frame budget before changing `RICH_STORAGE_BYTES` or zone layout.
4. As a low-risk intermediate step, align analysis positions and runtime phase
   semantics (including the final-to-first wrap) and test them. This fixes the
   coordinate mismatch but does **not** recover events that were never
   analyzed. Do not close the fidelity gap by merely increasing
   `RICH_FRAME_COUNT` without revisiting FFT window length, storage, and
   frequency resolution.
5. Separately document whether random note-start timeline phase is intended.
   Do not silently change it while fixing frame alignment.

## Grain onset continuity: current path

### What changed and what is consumed

- Grain compilation extracts up to 128 flux peaks from the retained source
  (`src/oscillators/resynth/artifact/grain.rs:125-148`) and places them in the
  immutable artifact.
- `PreparedPitchFrameBank::from_pitch_track` marks a pitch frame when a
  transient is within one frame span (`src/oscillators/resynth/analysis.rs:200-235`).
  This is a binary, coarse onset map.
- The current `frame_at_render` lookup linearly interpolates only `onset`, while
  keeping the nearest frame's family/confidence (`analysis.rs:248-272`). This
  is the partial continuity improvement added by `64f30e5`.
- `gate_spectral_tune` multiplies Tune by confidence and `(1 - onset)`
  (`src/oscillators/resynth/artifact/grain.rs:438-444`). The audio loop selects
  dry, tuned, or a linear dry/tuned PCM crossfade per sample
  (`grain.rs:682-715`).

### Remaining continuity risks

1. **No audio continuity proof exists.**
   `analysis.rs:438-457` tests onset metadata interpolation only, and
   `grain.rs:1015-1039` tests the scalar gate values. The production-like
   `artifact.rs:267-325` test checks that pitch modes are finite and distinct,
   not that an impulse/onset has a bounded first difference.
2. **Family/confidence can still jump.**
   `frame_at_render` interpolates onset but calls `frame_at` for family and
   confidence. At the midpoint between frames, family identity, family count,
   and confidence can switch. `gate_spectral_tune` can then switch between a
   dry read and a tuned read even when onset itself is smoothly interpolated.
3. **The source modes differ.**
   The dry and tuned PCM arrays are independently produced and normalized. A
   linear mix is safer than a hard switch, but its slope is not bounded against
   the difference between those arrays. The current code has no short
   crossfade/slew state per grain.
4. **Boundary coordinate mismatch.**
   Sampling calls `reflected_mip_sample`, which folds positions with
   `reflected_position` (`src/oscillators/resynth/artifact/shared.rs:603-614,668-715`).
   Onset lookup instead computes
   `layer.position / source_max` and clamps it (`grain.rs:685`). Once a grain
   crosses an edge, the sample and onset map refer to different source
   locations. This can create a mode change at the boundary.
5. **Onset duration is source-position based.**
   A frame span maps to different numbers of output samples at different pitch
   ratios. A high-ratio grain can traverse an onset ramp in very few samples.
   This is bounded work, but it is not a fixed audible transition time.

The older statement in `phase2-review-2026-08-23.md:132-150` that onset lookup
is nearest-only should be updated or superseded: it described the pre-
`64f30e5` implementation. The underlying requirement remains valid because the
current tree still lacks an end-to-end onset test and still has nearest family
selection.

### Existing coverage versus required coverage

Current tests cover:

- transient marking and onset interpolation as metadata
  (`analysis.rs:438-457`);
- finite/distinct Classic, Spectral, Target, and Scale output
  (`artifact.rs:267-325`);
- deterministic Grain scheduling, density, pool bounds, and alias bounds
  (`artifact.rs:142-174,405-425,614-693`).

They do not cover:

- a strong impulse over a tonal bed;
- every marked transient in a rendered Grain;
- output first differences before/during/after an onset;
- a grain crossing the reflected source boundary;
- a family/confidence transition while a grain is active.

### Smallest safe Grain fix

1. Add a production-path regression test. Compile a deterministic tonal bed
   with a strong impulse, select Spectral/Target with Tune at 1, and render
   through `GrainSchedulerState::render_cloud` while a grain crosses each
   marked transient. Assert finite output and a bounded first difference
   relative to nearby dry/tuned samples. Include a source-boundary crossing.
2. Keep the current onset interpolation as the first, lowest-risk behavior.
   It is already a better fix than reverting to nearest onset selection.
3. Change onset lookup to use the same reflected source coordinate as the PCM
   reader. This is a small semantic fix and does not add allocation, locking,
   or unbounded work.
4. If the end-to-end test still shows a spike, add a fixed per-grain gate
   slew/crossfade state. Latch family identity at grain birth, then move the
   dry/tuned mix toward its onset/confidence target over a bounded number of
   samples. This is safer than interpolating unrelated pitch families or
   changing spectral phase on the callback.
5. Keep Classic on the exact dry path. Do not broaden the fix into a new
   spectral residual/onset renderer until the artifact and publication contract
   for that renderer is complete.

## Recommended order

1. Land the Rich timeline regression tests and decide whether eight snapshots
   are an explicit approximation or an implementation gap.
2. Land Grain onset audio/boundary regression tests and the reflected-coordinate
   correction.
3. Only if tests demonstrate a remaining spike, add the bounded per-grain
   gate ramp. Then update the phase2 review to record that onset interpolation
   and audio continuity are separate properties.
4. Keep all worker-built timeline/pitch data inside the existing immutable
   artifact publication boundary. No callback allocation, FFT, lock, or file
   access is needed for these fixes.

## Follow-up status (62e3a1d)

Short Rich sources now use distinct bounded timeline segments instead of
analyzing one short window eight times. Grain pitch/onset lookup uses reflected
source coordinates, and dry/tuned Grain transitions use a fixed 32-sample slew.
The eight-frame Rich representation remains an explicit coarse approximation
for long sources; full sample-time Rich reconstruction is not claimed.
