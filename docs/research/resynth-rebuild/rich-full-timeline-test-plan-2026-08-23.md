# Rich full source-time fidelity and fresh/persisted parity test plan — 2026-08-23

**Status:** test design and review only. This report does not edit Rust.

## Recommendation

Add the tests below before calling Rich a full source-time renderer. Keep the
full-timeline tests red until the artifact carries every analyzed frame in
source order and the audio path consumes that sequence. The current checkout
has started a v14 storage expansion (`RICH_FRAME_COUNT = 32`,
`RICH_FRAME_SAMPLES = 4096`), but storage expansion alone is not enough:

- `rich_source_analysis_with_cancel` still analyzes one 4096-sample window at
  each of 32 sparse positions. It does not retain a complete overlapping
  source-time sequence.
- The artifact has no source position/hop metadata. Runtime interpolation still
  assumes uniformly spaced frame phases and wraps frame 31 to frame 0.
- The codec has a `RICH_TIMELINE_PACK_VERSION = 14` writer path, but the reader
  must be tested for both v14 full payloads and v13/v3-v13 legacy payloads.
  Legacy data must not be interpreted as a v14 slab.
- Existing unit tests inspect a Rich slab or a stationary tone. They do not
  render the complete source timeline through the published playback path.

The exit condition is therefore **observable source-time behavior plus
fresh/recall equality**, not merely `RICH_STORAGE_BYTES`, frame count, or a
passing codec size calculation.

## Scope and contract under test

The tests assume this product contract for Rich:

1. Worker-side analysis and reconstruction retain the complete admitted source
   duration in source order. A source longer than the named Rich memory budget
   may be bandwidth-reduced, but it is not truncated or reduced to eight
   snapshots.
2. Each retained frame has a deterministic source position (or an equivalent
   fixed hop and duration). Playback uses those positions, not an unrelated
   eight-phase approximation.
3. The immutable published artifact contains all timeline data, its source
   duration/rate, and any gain curve needed by the renderer. No second pointer
   carries frame metadata.
4. The callback reads only bounded immutable data and advances a preallocated
   phase. It does not analyze, allocate, lock, perform I/O, or publish a frame.
5. The fresh build and a persisted restore from the same source bytes have the
   same timeline, metadata, and audible result. The saved source master remains
   byte-exact.
6. A test render starts at timeline phase 0.0. The normal note-start random
   phase is deliberately disabled for these tests; random note phase is a
   separate pad/playback policy and must not make a fidelity test flaky.

The tests must not require sample-for-sample equality between the input WAV and
Rich output. Rich is a resynthesis algorithm. They must require duration,
ordering, event timing, spectral identity, finite output, and fresh/recall
parity.

## Exact deterministic fixtures

All fixtures are generated in the test, never read from a repository file.
Use `FS = 48_000` Hz and `wav_i16(1, FS, ...)` for the persisted-source path.
The f32 generator and the i16 conversion are part of the fixture definition.
Use `round(sample * 32_767.0)` after clamping to `[-0.98, 0.98]`.

### Fixture A: four ordered harmonic regions plus a silent marker tail

This is the primary full-timeline fixture.

```text
FS                 = 48_000 Hz
N                  = 96_000 frames (2.000000 s)
root               = 220.0 Hz (set root_override_hz explicitly)
base amplitude     = 0.20, sine at 220 Hz
marker amplitude   = 0.55, one marker at a time
fade               = 256 frames (5.333333 ms), raised-cosine edges
regions            = [0, 19_200), [19_200, 38_400),
                     [38_400, 57_600), [57_600, 76_800),
                     [76_800, 96_000)
marker harmonics   = 5, 7, 11, 13, none
marker frequencies = 1_100, 1_540, 2_420, 2_860, none Hz
seed               = 0x0123_4567_89ab_cdef
```

For each active marker `h`, generate
`0.55 * marker_envelope(i, start, end) * sin(TAU * 220*h*i/FS)`.
The base sine is present for all 96,000 frames. The marker envelope is 0 in
its 256-frame edge, 1 in the interior, and uses a raised-cosine ramp at each
edge. Do not add random noise. The last 0.4 s contains the base only; this
negative marker region catches truncation and accidental timeline wrap.

The five region centers are 9,600, 28,800, 48,000, 67,200, and 86,400 source
frames. Each interior probe window must be at least 4,096 frames from a region
edge. The fixture is long enough to expose sparse-window holes while remaining
small enough for a fast end-to-end test.

Use these Rich controls for the non-default persistence path:

```text
rich_balance          = -0.25
rich_formant_semitones= 3.0
rich_air_db           = 4.0
rich_diffuse          = 0.20
rich_dynamic          = 0.75
grain controls        = defaults (algorithm is Rich)
root_override_hz      = Some(220.0)
```

The explicit root override avoids making the test depend on pitch-detector
confidence while leaving the estimated root available for metadata checks.

### Fixture B: boundary and duration sentinel

Use the same 2-second source shape as Fixture A, but add two low-level markers
that are not used for the primary spectral ratio:

```text
impulse 1: +0.70 at frame 0, then a 64-frame raised-cosine decay
impulse 2: +0.70 at frame 95_999, preceded by a 64-frame raised-cosine attack
```

Keep the base and harmonic levels unchanged, and clamp before i16 conversion.
The test checks that the first and final source regions are represented, that
no frame index underflows or wraps, and that the final source frame is not
silently dropped. It checks finite/bounded behavior rather than exact impulse
waveform equality.

### Fixture C: static control

Generate 48,000 frames (1.0 s) of
`0.20*sin(2*pi*220*t) + 0.55*sin(2*pi*1100*t)` with the same root and seed.
This is a sanity control: every interior timeline probe should have the same
marker ratio within the stated measurement tolerance. A timeline implementation
must not invent changes in a stationary source.

## Measurement definitions and exact tolerances

Use deterministic Hann-windowed complex demodulation, not a peak-bin choice
that changes when a target frequency falls between FFT bins:

```text
probe window       = 8_192 frames
probe hop          = 256 frames
amplitude(f)       = 2 * |sum(x[n] * hann[n] * exp(-j*2*pi*f*n/FS))|
                     / sum(hann)
```

For a probe centered in each fixture region, compute marker amplitudes at
`[1_100, 1_540, 2_420, 2_860]` Hz. The expected marker is the one assigned to
that region; the last region expects none.

### Full-timeline acceptance

- Render exactly `N` output frames for one source-duration pass at target
  `220.0 Hz`, timeline phase 0.0, host rate 48,000 Hz. The output length must
  be exactly `N` and every sample must be finite.
- At each of the first four region centers, the expected marker must be the
  largest marker and must have amplitude at least `0.05` and at least 4.0 times
  the largest non-expected marker amplitude in that probe.
- In the final marker-off region, every marker amplitude must be at most 0.20
  times the median active-marker amplitude. This is a -13.98 dB upper bound.
- For each marker, measure its 50%-of-active-amplitude onset and offset from
  the 256-frame timeline scan. Expected boundaries are 19,200, 38,400,
  57,600, and 76,800. Each crossing must be within **1,024 source frames**
  (21.333 ms) of the fixture boundary. This allows the documented analysis
  window/group-delay budget while rejecting the current sparse eight-snapshot
  behavior. Event order must be exact; no marker may be observed in a later
  region before its assigned region.
- The rendered peak must be `<= 1.0 + 1e-6`; DC magnitude measured over the
  complete pass must be `<= 1e-4`.
- For Fixture C, the four interior marker amplitudes must have a max/min ratio
  `<= 1.10`. A stationary source must not acquire a timeline envelope.
- For Fixture B, the first and last 256-frame windows must be finite. The final
  impulse sentinel must occur in the final 1,024-frame scan window, not in the
  previous region. This is a boundary-presence check, not an input/output
  waveform identity check.

The 1,024-frame timing tolerance is intentionally explicit. A future test may
make it tighter after the chosen STFT window/hop is fixed, but it must not be
replaced by an unbounded “somewhere in the frame” assertion.

### Fresh-versus-persisted tolerances

The fresh and recalled artifacts are compared at three layers:

1. **Wire and metadata:** exact.
   - `state.encode()` after recall must equal the original encoded bytes.
   - Source WAV bytes, sample rate, channels, frame count, root override, seed,
     controls, selected algorithm, sequence count, source positions/hop, and
     source duration must match exactly.
   - Persisted f32 values that are canonical artifact data must compare with
     `to_bits()`, not `==` after a lossy conversion. Centers, gains, and every
     stored rendered sample are included.
2. **Published artifact render on the same target:** exact where the path is
   specified as deterministic. Render fresh and recalled artifacts from phase
   0 with the same target, host rate, note seed, and controls, then compare
   every sample by `to_bits()`. The test must report the first differing index.
3. **Portable floating-point fallback:** if the implementation documents a
   platform-dependent FMA/FFT path, use the following explicit bound instead
   of silently relaxing all checks:
   `abs(a-b) <= 2e-6 + 2e-5 * max(abs(a), abs(b))`, with a whole-pass RMS error
   `<= 2e-7` and max error `<= 2e-5`. Same-platform CI should still require
   bit identity.

Fresh and recalled renders must independently pass all Fixture A timeline
criteria. A parity test that only compares one stationary slab is insufficient.

## End-to-end test matrix

Place tests beside the production artifact/state tests, but keep the names
specific enough to prevent helper-only coverage from being mistaken for an
integration proof.

### 1. Worker build and artifact timeline

`rich_full_timeline_fixture_preserves_order_duration_and_boundaries`

- Generate Fixture A and compile Rich with the explicit root and controls.
- Inspect the immutable artifact's sequence count/positions and assert that
  positions are monotonic, cover source frame 0 through frame `N-1`, and have no
  gap larger than the selected analysis policy permits.
- Render through the artifact's timeline evaluator for one complete pass. Do
  not call only the FFT helper or inspect only frame 0.
- Apply all full-timeline acceptance criteria above.

`rich_timeline_fixture_does_not_collapse_to_eight_snapshots`

- Compare the marker identity at all five region centers.
- Require five distinct expected states (`h=5,7,11,13,none`).
- This test is an explicit regression against the current eight/sparse-window
  model and must remain in the suite after the implementation passes.

`rich_static_fixture_has_no_invented_timeline_modulation`

- Compile Fixture C and compare the four marker probe amplitudes and frame
  gains. Assert the 1.10 max/min ratio and bounded DC.

### 2. Production audio path and publication

`published_rich_full_timeline_matches_direct_artifact_pass`

- Build the fixture off the worker path, publish one immutable
  `ResynthRtArtifact` through `AtomicResynthArtifact`, accept it in a
  `ResynthPlaybackPlan`, set the oscillator timeline to phase 0, and render
  96,000 samples through `generate_resynth_step_modulated` (or the current
  production voice entry point).
- Analyze that captured callback output with the same demodulator and boundary
  scan. Do not substitute `RichZoneArtifact::eval_at_timeline` for this test.
- Assert generation/source-duration identity and compare the direct artifact
  pass to the production-path pass with the portable floating-point bound.
- A pointer swap must publish one node containing the complete Rich artifact;
  no separate frame pointer or metadata atomic is allowed.

`rich_timeline_scalar_and_block_render_are_identical`

- Clone the same accepted plan and oscillator state into scalar and block
  contexts.
- Render block sizes `[1, 2, 4, 8, 16, 32, 64, 128, 256]` and compare against
  repeated scalar calls. Require `to_bits()` equality on the same target.
- Include blocks crossing source region boundaries and the final source frame.

`rich_timeline_publication_holds_old_sequence_until_ack`

- Publish artifact A with Fixture A and artifact B with Fixture B.
- Accept A, publish B, keep A and B live in the two-layer playback plan, and
  attempt another publication. The bounded publisher must reject/block only
  according to its documented nonblocking result; it must never reclaim the
  sequence still referenced by the plan.
- After the fade, acknowledge only B, collect, and verify the pending publish
  succeeds. Use a drop/lifetime sentinel and check that a callback never reads
  a retired sequence.

### 3. Fresh versus persisted state

`rich_full_timeline_fresh_and_recalled_render_are_identical`

- Build a `ResynthAssetPackState` from Fixture A using the explicit controls and
  root override. Snapshot the fresh Rich artifact and encode at `PACK_VERSION`.
- Decode into a new state with `replace_all`, snapshot the recalled artifact,
  and re-encode. Assert byte-exact re-encoding, exact metadata/sequence
  identity, and sample-by-sample fresh/recall parity.
- Render both snapshots through the production accepted-plan path with phase 0
  and the same note seed. Run all timeline and finite-output checks on both
  outputs, not only their difference.
- Repeat once with Fixture B to cover first/final boundary sentinels.

`rich_v13_legacy_payload_is_not_read_as_v14_full_timeline`

- Encode a Rich state using the last pre-timeline version (v13), decode it, and
  verify the explicit legacy migration behavior. Legacy data may be expanded
  deterministically for compatibility, but it must not claim full source-time
  fidelity; the decoded marker scan should either carry the documented legacy
  approximation flag or be excluded from the full-fidelity assertion.
- Decode a v14 full payload and assert that all 32-frame (or future named
  sequence) samples are consumed, the reader consumes exactly all bytes, and
  `artifact_persisted_bytes` equals the bytes actually written.
- Corrupt/truncate the v14 timeline count, positions, one frame sample, and
  trailing bytes. Every malformed payload must be rejected without a partial
  publication.

`rich_restore_rebuild_does_not_use_stale_analysis`

- Start a Fixture A build, change the desired source to Fixture B before the
  completion is committed, then accept the worker result through the normal
  revision gate. The published artifact must have B's source frame count and
  marker order, never A's timeline with B's source master (or the reverse).
- This is a publication identity test as well as a persistence test: source
  bytes, sequence, duration, and rendered output must agree.

## Realtime-safety checks

Run these checks around the production callback tests. They are acceptance
criteria, not optional profiling notes.

1. **No allocation:** prebuild the artifact, plan, oscillator, output buffer,
   telemetry buffer, and any phase state. Run the callback under the repository
   allocation counter/guard (or an equivalent test-only global allocator) and
   assert zero allocations and zero deallocations. The callback must not clone
   an `Arc`, create a `Vec`, resize a container, or allocate a temporary FFT
   buffer.
2. **No blocking:** instrument or review the callback call graph and reject
   `Mutex`, `RwLock`, channel waits, `join`, sleeps, file I/O, syscalls,
   logging, and exception/unwind paths. Publication and acknowledgement must be
   atomic/nonblocking; the callback must not wait for a worker.
3. **Bounded work:** render the same number of callback frames with Fixture C
   at 0.25 s and Fixture A at 2.0 s. The callback operation count must not scale
   with source duration or sequence length. A sequence lookup/interpolation is
   bounded by the fixed capacity, with no scan over all source samples.
4. **Immutable publication:** mutate no Rich sequence field after publication.
   Verify the accepted view keeps source rate, source duration, positions, gain,
   and samples coherent. A separate atomic frame pointer plus a separately
   loaded position/count is a test failure even if audio usually sounds right.
5. **Numerical safety:** all callback output is finite; peak is `<= 1.0+1e-6`;
   complete-pass DC is `<= 1e-4`; silence/near-silence probes do not produce
   NaN, Inf, or a growing denormal tail. Apply the project's FTZ/DAZ or
   no-denormal policy in the harness.
6. **Block/scalar and oversampling parity:** repeat the no-allocation check for
   host block sizes 1, 2, 4, 8, 16, 32, 64, 128, and for every supported
   oversampling factor. The active Rich path must not enter an internal helper
   pool that allocates or uses a different timeline state.
7. **No worker work in audio:** add a test-only counter around FFT, source
   analysis, persistence, and visual-cache construction. The counter must stay
   zero while rendering. Run worker compilation before entering the guard.

## Persistence/version and memory gates

- Keep v13 decoding behavior unchanged. v14 is the first version that may
  carry the full Rich timeline. The v14 reader and writer must agree on frame
  count, positions/hop, gain metadata, and all samples.
- Check `artifact_persisted_bytes()` against actual payload length for both v13
  legacy and v14 full Rich artifacts. Include the outer pack hash and exact
  re-encode assertion.
- Treat the current 32 x 4,096 x 22 f32 slabs as a bounded implementation
  choice, not proof of arbitrary source duration. Record the memory budget and
  the effective source rate/bandwidth when a source exceeds it.
- Never silently drop the source tail to fit the budget. A long-source test
  should use `N = 2_000_000` frames (41.666667 s) with one marker in the final
  0.25 s. If the implementation down-projects, the final marker must still be
  present within the 1,024-frame *effective-source* timing tolerance and the
  persisted duration must remain 41.666667 s (within one source frame).

## Suggested execution order and close criteria

1. Land Fixture A/B/C helpers and the direct artifact tests.
2. Make the production-path publication test pass. A direct helper pass is not
   an end-to-end result.
3. Add v14 write/read, v13 migration, byte accounting, and fresh/recall tests.
4. Add scalar/block, allocation, publication-lifetime, and stale-revision
   checks.
5. Close the Rich rebuild only when both fresh and recalled production renders
   pass every timeline criterion, all exact metadata/wire checks pass, and the
   callback guard reports no allocation/blocking/FFT/file work.

A passing stationary high-band test, a larger fixed slab, or a successful
`encode/decode` call alone does not close full Rich source-time fidelity.
