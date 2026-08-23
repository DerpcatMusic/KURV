# RESYNTH Target and Spectral production-path audit (2026-08-23)

## Scope and evidence

This is a read-only audit of the user-facing `Classic`, `Spectral`, and
`Target` modes for the production Grain path. It covers the editor contract,
worker artifact construction, realtime rendering, persistence, and tests. No
Rust source was edited by this audit.

The checkout was at `64f30e5` (`Integrate bounded spectral pitch frames into
grain rendering`) when inspected. The working tree also received concurrent
source changes from another agent while validation was running; those changes
are not part of this audit. Evidence below refers to the current line numbers
in the checkout.

The contract is stated in `src/oscillators/resynth/targeting.rs`:

- `Classic` is ordinary granular resampling with coupled pitch/read speed
  (lines 10-16).
- `Spectral` is a duration-preserving free spectral shift by the played-note
  interval, preserving source polyphonic intervals (lines 15-17).
- `Target(PlayedNote)` corrects each detected family to the exact played MIDI
  note, including octave; `Target(Scale(_))` maps each detected family to a
  selected absolute scale (lines 18-20 and 42-52).

The editor exposes these modes directly in `src/editor_resynth.rs:904-960`.
The newer prepared-bank note also defines the intended integration boundary:
Target must remain active at `Tune = 0`, Spectral uses Tune as its blend, and
harmonic partials must mix with an unretuned residual/onset path
(`docs/research/resynth-rebuild/prepared-pitch-frame-bank-2026-08-23.md:25-38`).

## Production-path map

1. `compile_rt_artifact_with_cancel_ref` allows Grain without a root and calls
   `GrainSourceArtifact::compile_channels_with_cancel` for Grain
   (`src/oscillators/resynth/mod.rs:523-558`).
2. Grain compilation normalizes/decimates PCM, computes a scalar pitch map only
   when `root_hz` is present, computes transient positions, renders the legacy
   `tuned_samples`, and stores a `PreparedPitchFrameBank` in the immutable
   Grain artifact (`src/oscillators/resynth/artifact/grain.rs:91-190`).
3. Restore rebuilds the pitch bank from persisted PCM and transients, without
   re-normalizing the persisted PCM (`grain.rs:236-268`). The bank is therefore
   inside the same immutable artifact node as the PCM, which is the correct
   publication boundary.
4. The audio path selects the mode in `GrainSchedulerState::render_cloud`
   (`grain.rs:574-747`). It creates one `SpectralFrame` from
   `artifact.pitch_frame_at(controls.position)` (`grain.rs:594-608`), renders
   active grains from dry/tuned PCM (`grain.rs:691-715`), then optionally
   replaces the result with `SpectralRenderer::render_sample(0.0, 0.0)`
   (`grain.rs:731-736`).
5. `SpectralRenderer` itself is fixed-size and advances partial phase without
   allocation (`src/oscillators/resynth/spectral.rs:77-162`). Its residual and
   onset inputs are real API inputs, but production passes zero for both.

The publication shape is sound for immutable metadata: one Grain artifact is
published through the existing `AtomicResynthArtifact`; no second frame pointer
was found. This does not make the current synthesis semantics correct.

## Findings

### P0 — Production Spectral/Target output is a synthetic static partial, not a source spectral transform

**Evidence:** `render_cloud` prepares one frame from the global UI
`controls.position` (`grain.rs:594-608`). It does not select a prepared frame
for each active layer position or maintain a source-time frame sequence. The
renderer is then called with `(residual = 0.0, onset = 0.0)` and its harmonic
result is mixed against the already summed grain output (`grain.rs:731-736`).
`SpectralRenderer::render_sample` only sums sine partials from metadata
(`spectral.rs:141-162`).

Consequences:

- Source residual, source dynamics, source timbre, and onset material are not
  present in the Target/Spectral harmonic output.
- `Target(PlayedNote)` can produce a full-scale sine from a one-candidate frame,
  but it does not preserve the source duration/timbre as the mode contract
  implies.
- At `Tune = 1`, the result is effectively the generated mono harmonic signal;
  stereo side and grain cloud texture are discarded by
  `((left + right) * 0.5)` before the final assignment (`grain.rs:731-736`).
- At intermediate Tune, the code first reads the old tuned PCM and then blends
  it with a separate generated sine. That is not a defined harmonic-plus-
  residual mix and can apply two different pitch policies.

**Smallest safe step:** choose and document one production algorithm before
further wiring. Either (a) make Spectral use the existing worker-rendered
sample-long `tuned_samples` path and keep Target disabled until a real target
artifact exists, or (b) build an immutable worker payload containing source-time
harmonic amplitudes/phases plus residual/onset PCM and consume it in the audio
path. If (b), pass actual residual/onset samples to the renderer and mix them
with an explicit bounded gain policy. Do not claim integration based only on
`SpectralRenderer` metadata tests.

### P0 — Target and Spectral are not polyphonic in the production analyzer

**Evidence:** `build_pitch_map_with_cancel` runs one root estimator for each
window and returns `Vec<f32>` (`grain.rs:901-951`).
`PreparedPitchFrameBank::from_pitch_track` converts each scalar value into at
most one `PitchCandidate` with hard-coded strength/confidence `1.0`
(`src/oscillators/resynth/analysis.rs:198-235`). No production code extracts
multiple simultaneous candidates. `MAX_PITCH_FAMILIES = 8` is only a storage
capacity; it is populated by helper/unit-test inputs, not by the Grain build.

Therefore `Target(Scale(_))` cannot currently act on “each detected family,” and
`Spectral` cannot preserve polyphonic intervals. The helper tests in
`analysis.rs:396-435` prove policy for manually supplied candidates, not the
production detector.

**Smallest safe step:** replace the scalar track with a bounded worker analysis
that preserves candidate families, strength, confidence, voiced state, source
position, and onset. Keep a fixed-capacity bank in the existing immutable Grain
artifact. Add an end-to-end polyphonic source test before wiring any UI claim
back to the mode.

### P0 — Target with no stable root can become silence

**Evidence:** Grain is allowed to compile with `root_hz = None`
(`mod.rs:523-558`). In that case `pitch_map` is absent (`grain.rs:121-125`),
so the prepared bank is empty. Target nevertheless forces `spectral_mix = 1`
when `Tune` is zero (`grain.rs:412-425`). `pitch_frame_at` returns a default
empty frame (`analysis.rs:247-255`), and the production renderer is called with
zero residual/onset, yielding zero. Target at its default Tune can therefore
silence an otherwise valid unpitched Grain source.

The same issue affects Spectral when Tune is positive on a source with no root.
This is especially reachable because the importer explicitly documents that
Grain works without a fundamental (`mod.rs:365-370`).

**Smallest safe step:** make mode availability explicit. The safe runtime
fallback is to bypass spectral mixing and retain the existing Classic/dry Grain
output whenever the artifact has no usable prepared frame/root. Alternatively,
disable the mode in the editor for such an artifact and show why. Add a no-root
render regression at `Target + Tune=0` and `Spectral + Tune=1`; neither may
silently output zero.

### P1 — Onset interpolation is prepared but not consumed by the spectral result

**Evidence:** `frame_at_render` interpolates onset metadata
(`analysis.rs:257-272`), and the per-layer legacy source gate reads onset
(`grain.rs:686-689`, with `gate_spectral_tune` at lines 438-444). However, the
final spectral branch always calls `render_sample(0.0, 0.0)` and then applies
`SpectralFrame` output (`grain.rs:731-736`). Thus the renderer's onset path is
never used. The per-layer gate cannot suppress the final harmonic replacement.

A transient can therefore still switch between unrelated generated-harmonic
and source outputs without a continuity policy. The pure renderer test at
`spectral.rs:188-198` does not exercise the production call edge.

**Smallest safe step:** select an onset/residual value from the same source-time
frame as the grain layer, and use a bounded crossfade/hysteresis between source
and corrected material. Add an impulse-over-tonal-bed render test checking finite
output and bounded first differences around every marked transient.

### P1 — Prepared frame lookup is tied to UI Position, not the rendered grain/source timeline

**Evidence:** the spectral frame is selected once from
`controls.position` (`grain.rs:596-602`). Active grains can be at different
`layer.position` values, and that position is used only for the legacy gate
(`grain.rs:681-689`). The target correction therefore does not follow the source
material each grain reads. Moving Position changes the one generated partial
frame, but concurrent grains do not receive their own source-time correction.

**Smallest safe step:** define a source-time coordinate for every prepared frame
and select/interpolate it from each layer's source position (or from an explicit
sample-long spectral playhead). Keep this lookup bounded and immutable. Test a
source with different pitches in its first and last halves and verify the output
tracks the correct half while grain Position and duration remain independent.

### P1 — Existing mode tests are not contract tests

**Evidence:** `grain_pitch_modes_reach_the_prepared_spectral_renderer` in
`src/oscillators/resynth/artifact.rs:267-325` checks finiteness and pairwise
`assert_ne!` only. It does not measure output pitch, octave, duration, source
residual, stereo, transient continuity, or scale-family behavior. The
`targeting.rs` and `spectral.rs` tests exercise pure helpers. The persisted test
at `artifact.rs:1101-1121` checks frame-bank equality, not rendered output under
all modes.

**Smallest safe test set:**

1. Render through `generate_resynth_step_modulated` (or its direct production
   Grain call) for Classic, Spectral, Target PlayedNote, and Target Scale.
2. For Target PlayedNote, assert exact played-note frequency including octave
   at `Tune=0` and a nonzero Tune, while checking source duration.
3. Use a two-family source and assert independent Target Scale mapping and
   preserved Spectral intervals. This should fail until production analysis
   supplies families.
4. Render no-root sources and assert the chosen fallback is audible and
   documented.
5. Render an onset over a tonal bed and assert bounded sample-to-sample
   transitions; run the same render after persisted restore and compare output
   identity.
6. Exercise stereo side/residual behavior and multiple unison lanes, since the
   current final branch forces both channels to the same harmonic value.

## Smallest safe implementation order

1. **Lock the public contract and fallback.** Preserve wire tags and Classic
   behavior. Decide whether no-root Grain disables Target/Spectral or falls back
   to Classic. Add mode availability/readout if disabled.
2. **Finish worker analysis.** Build bounded, deterministic source-time families
   with confidence/energy/onset data. Do not infer polyphony from the current
   scalar root track.
3. **Finish one immutable production artifact.** Carry the full prepared
   harmonic/residual representation inside `GrainSourceArtifact`, regenerated
   from authoritative persisted PCM or introduced under a new pack version. Keep
   it behind the existing single artifact pointer and revision gate.
4. **Consume it correctly in RT.** Use per-layer source-time lookup, actual
   residual/onset samples, fixed work only, and an explicit gain/stereo policy.
   Avoid a shared mutable renderer advancing once per unison lane; give mutable
   phase state the correct per-voice/lane ownership or render the immutable
   prepared PCM instead.
5. **Add contract tests before UI sign-off.** Keep the existing bounded,
   deterministic, cancellation, publication, and fresh/restore identity tests,
   then add the output tests listed above.

## Validation commands run

All commands were run from the repository root with the checked-in Cargo
configuration, after the concurrent source updates had landed:

```text
cargo check --all-targets
# passed

cargo test resynth --all-targets -- --test-threads=1
# passed: 154 tests

cargo test grain_pitch_modes_reach_the_prepared_spectral_renderer --lib -- --nocapture --test-threads=1
# passed: 1 test

cargo test --lib -- --test-threads=1
# passed: 297 tests
```

These green tests demonstrate bounded compilation, publication/persistence
invariants, and helper wiring. They do not demonstrate the user-facing
frequency, polyphonic, residual, onset, or no-root behavior identified above.
