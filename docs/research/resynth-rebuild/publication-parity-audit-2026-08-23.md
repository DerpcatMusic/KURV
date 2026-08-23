# RESYNTH publication and parity audit — 2026-08-23

## Scope and evidence

This is a read-only audit of immutable RESYNTH publication, restore, stale-build
completion, and block/scalar parity. No Rust source was edited. The checkout is
at `64f30e5` (`Integrate bounded spectral pitch frames into grain rendering`).

Validation run:

```text
cargo test resynth --all-targets -- --test-threads=1
154 passed, 0 failed
```

The passing suite is useful evidence for local helpers. It does not yet prove
the callback publication/reclamation protocol or active-RESYNTH block parity.

## What the implementation currently guarantees

- `AtomicResynthArtifact` publishes one immutable `ResynthArtifactNode` through
  one `AtomicPtr` (`src/resynth_state/publication.rs:178-235`). The node keeps
  generation, revision, and the complete `Arc<ResynthRtArtifact>` together.
  `try_view_after` uses an Acquire load (`publication.rs:250-259`), and stores
  use Release (`publication.rs:232-233`). This is the correct shape for the
  prepared pitch bank: it must remain inside the same artifact node.
- The callback acknowledgement is a bounded seqlock snapshot of seen
  publication plus the two plan-live generations
  (`publication.rs:137-175, 271-283`). Retirement retains a node while either
  the callback has not observed the retirement generation or the playback plan
  still lists that generation (`publication.rs:293-297`).
- Async build completion has a pre-publication revision gate. The worker checks
  the desired revision before and after compilation and again while holding the
  document gate (`src/resynth_state/build.rs:516-589`). A pending completion is
  checked again before `rt.store` (`build.rs:677-706`). Aggregate restore locks
  every document gate, preflights every changed slot, opens an odd epoch, then
  stores pointers and documents before closing the epoch
  (`src/resynth_state.rs:1664-1826`).
- Grain restore reconstructs mips and `PreparedPitchFrameBank` from the
  persisted authoritative PCM (`src/oscillators/resynth/artifact/grain.rs:236-268`).
  The bank is therefore not a second pointer and is not silently lost on v13
  decode (`src/resynth_state/codec.rs:397-469`).
- The helper pool rejects a synth with an active RESYNTH engine
  (`src/voices/internal_rt_pool.rs:831-835`). This is a good safety boundary,
  but it is not a block/scalar parity test.

## Findings and concrete gaps

### 1. High — publication tests do not exercise the two-live-layer reclamation fence

`pointer_publication_is_coherent_under_concurrent_revisions`
(`src/resynth_state.rs:2372-2429`) acknowledges every view immediately as
`[observed, 0]`. It never holds an old and new artifact in a live
`ResynthPlaybackPlan`, fills the two-node retired queue, or verifies that a
retired artifact remains alive until the plan fade ends. It also checks that all
samples in one payload agree, but does not relate the payload sentinel to the
view's revision.

`revision_and_source_transitions_never_admit_more_than_two_layers`
(`src/voices/oscillator_bank.rs:1016-1059`) checks plan fields, but does not
feed its `[old, new]` live pair back to `slot.acknowledge_rt` while producers
attempt further stores. Thus the dangerous sequence is untested:

1. callback accepts A;
2. producer publishes B;
3. callback retargets to `[A, B]` and acknowledges that pair;
4. producer publishes C and then attempts D;
5. D must remain blocked while A/B are live;
6. after the fade, callback acknowledges `[B, 0]`; collection must release A and
   D must then succeed.

This is a lifecycle-proof gap, not a demonstrated UAF in the current protocol.
A `Weak<ResynthRtArtifact>` drop sentinel should make premature reclamation
observable. Without it, a future change can free a node while a callback plan
still contains its raw view and the existing tests can remain green.

### 2. Medium — aggregate restore epoch coverage is deterministic, not an interleaving test

`aggregate_restore_yields_one_even_epoch_and_exact_fixed_rt_set`
(`src/resynth_state.rs:2508-2546`) manually writes epoch `3` and asserts that a
read is rejected. It does not overlap `try_rt_update_after` with the odd epoch
window while two or more slots are being replaced. The callback must never
accept a mixed set such as slot 0 from the incoming restore and slot 7 from the
previous pack. The before/after epoch check appears intended to prevent this
(`resynth_state.rs:1058-1085`), but there is no producer/callback stress test
that proves every accepted update is a complete fixed set and every rejected
update is retried without acknowledging unaccepted raw views.

### 3. Medium — stale completion coverage proves a serial gate, not a worker race

`stale_completion_never_replaces_newer_desired_revision`
(`src/resynth_state.rs:3045-3081`) constructs a pending completion, increments
the desired revision under the document lock, and calls `try_commit_pending`
serially. This proves the pre-store check in `build.rs:677-706`, but it does not
exercise an actual worker completing while a newer rebuild, clear, or aggregate
restore is admitted.

The async worker has several checks and the document gate is the right
linearization boundary. The missing proof is that the stale path cannot insert
a pending completion after a newer clear/rebuild has taken ownership, and that
an aggregate restore cannot be followed by a stale worker store. A test-only
compile barrier or an injected `CompletedResynthBuild` completion point is
needed; timing sleeps are not sufficient.

The synchronous `replace` and `select_algorithm` paths compile before taking
the document gate (`src/resynth_state/build.rs:105-159, 162-213`). Their
operation ordering is therefore defined by the eventual gate/store, not by
compile start time. If these public methods may be called concurrently with
queued desired updates, the ordering contract must be tested or explicitly
serialized. Otherwise a slow synchronous call can be the later linearized
publication and legitimately supersede the earlier queued intent.

### 4. Medium — restore parity stops at metadata/mip helpers, not codec plus audio

`persisted_pcm_regenerates_identical_prepared_pitch_frames`
(`src/oscillators/resynth/artifact.rs:1102-1121`) compares a fresh Grain
artifact with `from_persisted` directly. The nearby mip test compares selected
PCM reads (`artifact.rs:1124-1180`). Neither decodes a v13 pack through
`read_artifact`, nor renders the restored artifact through the immutable
publication and voice path.

The prepared bank is rebuilt from `samples`, while tuned PCM, side PCM,
transients, controls, source audition, and publication identity are carried by
different code. A codec or restore regression could therefore pass the direct
bank equality test while fresh and recalled audio diverges. This is especially
important for Target/Spectral controls because the callback owns mutable
`SpectralRenderer` phase state (`src/oscillators/resynth/artifact/grain.rs:574-746`).

### 5. High — active RESYNTH block/scalar parity is untested, and block admission is broad

`PolySynth::block_internal_samples` reports a block size whenever there are
active voices with steady unison and no RESYNTH transition
(`src/voices/poly_synth.rs:2457-2470`). It does not exclude
`has_active_resynth()`. Downstream helper admission does exclude RESYNTH, but
the host can still choose coarse/block branches based on this result. Current
safety depends on generic block code eventually taking the scalar RESYNTH path;
that dependency is not pinned by a parity test.

`render_block` delegates generic work through
`VaVoice::render_generic_block` (`src/voices/poly_synth.rs:2619-2681`,
`src/voices/voice/render.rs:2044-2060`), while the scalar path advances the
playback plan, Grain scheduler, source audition, and frame counter one sample
at a time. No test compares those state transitions for active Grain, Sample,
or Rich. `active_resynth_render_never_crosses_helper_boundary`
(`src/voices/internal_rt_pool.rs:1607-1622`) only calls the private
`pool_eligible` predicate; it does not call `render_block_job`, use a published
artifact, or compare the serial fallback.

Missing cases include block sizes 1/2/4/8/16/24/32, oversampling factors
1/2/3/4, a revision crossfade with two live artifacts, Rich zone handover,
Grain spawn/frame boundaries, Source A/B fade, and a host event-split partial
block. These are the cases that can advance a plan or scheduler twice, skip a
frame, or expose helper-owned RESYNTH state.

## Exact tests to add

### A. Publication and reclamation

Add `publication_reclaims_only_after_plan_drops_old_generation` to
`src/resynth_state.rs` (or a dedicated `publication.rs` test module):

1. Publish artifact A with revision 1 and acknowledge plan `[A, 0]`.
2. Publish B with revision 2. Retarget a live `ResynthPlaybackPlan` to
   `[A, B]`; acknowledge `seen = B` and `live_generations = [A, B]`.
3. Publish C. Keep a `Weak<ResynthRtArtifact>` for A and B. Assert both weak
   references still upgrade and the retired queue contains the expected live
   nodes.
4. Attempt D. Assert `store` returns `None`/`can_store` is false while
   `[A, B]` is acknowledged.
5. Finish the plan fade, acknowledge `[B, 0]`, call `collect`, and assert A's
   weak reference no longer upgrades while B remains alive.
6. Publish D and read generation, revision, and a payload sentinel together.
   Assert the sentinel encodes the same revision; do not only assert that all
   samples are mutually equal.

Add `publication_concurrent_load_ack_and_collect_keeps_live_payload_valid`:
run a producer that attempts A..N stores and a callback loop that loads a view,
reads its identity and sentinel, retargets a two-layer plan, acknowledges only
a complete plan, and calls `collect`. Use a bounded barrier/iteration count,
not sleeps. The test must assert no weak sentinel is dropped while its
  generation is in `live_generations`, and that every accepted view's payload
  revision matches its node revision.

### B. Aggregate restore race

Add `aggregate_restore_never_accepts_a_mixed_slot_set` in
`src/resynth_state.rs`:

- Build two complete packs with distinct per-slot payload sentinels.
- In one thread repeatedly submit the latest restore transaction; in the
  callback thread repeatedly call `try_rt_update_after` with the observed
  generations.
- If the update is `Some`, assert every changed slot's identity matches the
  corresponding document and that all slots belong to one pack sentinel set.
  Retarget/acknowledge the entire changed set before advancing `observed`.
- If the update is `None`, retry next iteration. Assert no partial update is
  acknowledged. Run enough iterations to hit the odd epoch window and include
  a second slot; the current manual odd-epoch test is not enough.

### C. Stale worker and restore completion

Add `stale_worker_completion_is_dropped_before_pointer_store` in
`src/resynth_state.rs` using a test-only build completion barrier or direct
worker completion injection:

1. Publish A and queue a build at revision R.
2. Pause completion immediately before its document-gated store.
3. Admit a newer clear/rebuild at R+1, then release the stale completion.
4. Assert the stale completion is dropped, `published_rt_generation` and
   pointer payload remain A, and `pending_commit` cannot contain R.
5. Repeat with an aggregate restore replacing two slots. Assert neither slot
   can be republished by the old worker and the restore's complete set remains
   installed.

If concurrent synchronous `replace`/`select_algorithm` calls are supported,
add `newer_desired_intent_cannot_be_clobbered_by_slow_sync_compile` with a
barrier around compilation. If they are intentionally serialized to the UI
thread, document that precondition and test the call boundary instead.

### D. Codec/restore Grain audio identity

Add `grain_v13_codec_restore_matches_fresh_published_render`:

- Compile a deterministic stereo source containing a tonal bed, a second
  harmonic, and a transient. Use non-default `PitchMode::Spectral` and
  `Target(PlayedNote)` controls, including `grain_tune = 0` and `grain_tune = 1`
  in separate runs.
- Encode the complete v13 state, decode it with
  `ResynthAssetPackState::decode`, and install it through `replace_all`.
- Compare fresh/restored `pitch_frames`, authoritative PCM, side/tuned PCM,
  transient list, and selected mip reads at positions/rates used by playback.
- Publish each through `AtomicResynthArtifact`, accept each through a
  `ResynthPlaybackPlan`, and render the same finite sequence through
  `generate_resynth_step_modulated`. Compare `to_bits()` for left/right output
  and scheduler/frame state. This must use the decoded artifact, not a direct
  `from_persisted` call only.

### E. Active RESYNTH scalar/block parity

Add `active_resynth_block_matches_scalar_for_all_legal_sizes` in
`src/voices/voice.rs` or `src/voices/poly_synth.rs`:

- Construct two identical synths with one published Grain artifact, an active
  RESYNTH oscillator, identical plan/generation state, and identical note/
  envelope state.
- For `N = 1, 2, 4, 8, 16, 24, 32`, render one clone with N scalar
  `PolySynth::render` calls and the other with `render_block::<N>`.
- Compare every left/right sample with exact `to_bits()` and compare all
  mutable RESYNTH state: plan remaining/gains, `resynth_frame`, both Grain
  scheduler states, source-audition state, and oscillator phase state.
- Repeat with active Sample and Rich artifacts. For Rich, cross a zone boundary.
  For Grain, force a spawn boundary and a prepared-frame/onset boundary.
- Assert the intended `block_internal_samples` contract for active RESYNTH.
  If active RESYNTH is serial-only, it should return `None`; if coarse blocks
  remain allowed, this parity test must be the explicit proof of that contract.

### F. Pool and host fallback

Add `active_resynth_pool_job_returns_none_and_serial_fallback_is_exact`:
create a real published active Grain artifact and call
`InternalRtPool::render_block_job::<16>`, `::<24>`, and `::<32>`. Assert each
returns `None`, then compare the serial `render_block` fallback to scalar
`render` bit-for-bit. Run factors 1, 2, 3, and 4 where the host path supports
them. This must replace the current predicate-only coverage at
`internal_rt_pool.rs:1607-1622`.

Add a host-block event-split test that renders an active RESYNTH voice across a
partial event-free tail and a full coarse window. Compare against a sample-by-
sample reference and assert no RESYNTH frame, plan transition, or source fade
advances twice. Include an artifact handover during the tested window.

## Exit criteria

Do not call immutable publication/reclamation complete until a two-live-layer
queue-saturation test observes artifact lifetime through callback acknowledgement
and release afterward. Do not call restore parity complete until a v13 decoded
Grain artifact renders identically to the fresh artifact through the published
voice path. Do not call stale-build handling complete until a real completion /
newer-intent interleaving is covered. Do not call block rendering complete until
active RESYNTH scalar/block/host fallback parity is exact across steady and
transition cases.

## Follow-up status (62e3a1d)

A two-live-layer publication queue saturation test now verifies that a fourth
store is blocked while both retired generations remain live, and that storage
resumes after the fade acknowledgement. Active RESYNTH coarse admission is
also rejected and the helper pool has deterministic reserved rows. Full
worker-race and codec-through-voice parity stress tests remain future coverage.
