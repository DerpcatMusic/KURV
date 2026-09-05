# DSP audit continuation, 2026-09-05

This change fixes lost parent-depth modulation, a zero-duration envelope transient, and a release-voice eligibility mismatch. It also makes two measurement tools reject invalid evidence. Package version is 0.8.157; the branch starts at main `251d8b8` (merged PR #16). Existing PR #17 was inspected, not merged.

## Reproduced defects and fixes

| Failure on the pre-fix code | Change and proof |
| --- | --- |
| Zero attack, decay and sustain produced gain **-1** at frame zero. | Voice and group envelopes now complete instantaneous transitions before inserting the net gain correction. The regression checks sustain 0/0.25/0.75/1, both envelope paths, and eventual gain independently of the reference. No new per-sample work. |
| Grouped scalar rendering omitted per-voice generator-depth routes. The audio comparison failed with absolute-error sum 29.980837 versus reference absolute-energy sum 296.42996. | Each affected voice evaluates every child depth from that sample's LFO values. Multiple child routes use the existing sample fallback; the representable single-target block buffer is retained. External buffers become the base before parent contributions are added. |
| The combined filter selector accepted extra contributions, retaining only the last entries. | Reject a shortcut configuration unless it contains exactly the two contributions the shortcut represents. The existing full evaluator handles the rest. |
| Auxiliary output used a constant route amount despite a matching dynamic override. | Both consumers forward per-sample overrides; focused checks cover matching and unrelated route IDs plus per-voice parent contributions. |
| Existing `unsupported_and_release_states_fall_back_without_stale_jobs` failed even in isolation: a released voice was accepted by a renderer requiring `held`. | Shared legacy shape eligibility and morph eligibility now enforce that requirement. Generic rendering remains the fallback, and independent grouped release optimizations remain enabled. The existing regression passes unchanged. |
| `process_lab` checked only its final output buffer for finite values and peaks. | Accumulate both over every timed callback, outside the measured callback timer, and fail nonfinite workloads. A retained fault-control script fails if either accumulator is removed. Warm-up callbacks remain outside these measurements. |
| The triangle analyzer could loop forever for bin zero, crash on exact-zero residual, and overlabel residual energy as aliasing. | Reject invalid bins/silence, handle zero residual, and name the result `nonharmonic_residual_dbc`. Actual-script controls inject invalid input and an independent spurious tone. Aliases falling on wanted bins remain outside this metric's ability to distinguish. |

The existing oversampling test's 100 Hz input is coherent and valid. Its helper now rejects noninteger-period probes rather than silently claiming coherent analysis of a rounded period. Existing formatting differences were normalized; no test assertions were relaxed. Four old experiment binaries also called removed `set_spline_correction_immediate`; those six calls were removed because `reset(factor)` and the current per-factor equalizers own that behavior. Their historical audio/CPU results were not refreshed or treated as current.

## Rendered-audio coverage

`grouped_voice_depth_matches_samplewise_route_amounts` exercises scalar and direct block paths, unison **1/4/8/16/64**, two independent parents for each of two child routes, Level/Pan and PhasePosition/Transpose, and a changing external amount buffer. The reference explicitly configures the child depths per sample, independently of the production depth helper. A separate base-depth render proves the routes actually affect audio.

The block fixture configures unison before note-on and explicitly checks block eligibility. An initial fixture reconfigured unison after note-on and incorrectly called the direct block renderer during its 384-sample topology transition. That invalid fixture was corrected; it was not treated as a DSP defect or accommodated by loosening the error tolerance.

These checks cover the specified matrix, not every possible patch. Filter shortcut and auxiliary override checks are focused value/eligibility checks, not complete filter/auxiliary audio fixtures. Generator-audio parent chains into auxiliary depths remain a separate pre-existing gap. No blanket claim of arbitrary nested PM antialiasing is made.

## Runtime cost boundary

No heap allocation, lock, I/O or dynamic storage is added to the audio paths. Routes without parent depth borrow the base frame and avoid new copies. The sample fallback uses a fixed frame and bounded route scans; its worst-case depth work is proportional to generator routes times parent routes. That fallback's CPU cost is **not benchmarked here**, and restoring missing modulation may cost more than the incorrect path. This is a correctness change, not a claimed synth-wide optimization.

## Validation

Baseline library suite, excluding the two newly added failing regressions: **373 passed, 1 failed, 51 ignored**. The existing failure was the release-voice eligibility test above.

Final optimized library suite: **380 passed, 0 failed, 51 ignored**. With `rt-paranoid` and debug assertions enabled for KURV: **381 passed, 0 failed, 51 ignored**. Ignored manual experiments are not counted as correctness evidence. The MinBLEP manual report was run separately before and after its measurement correction; see [the experiment report and raw evidence](dsp-audit-minblep-2026-09-05.md).

Commands used (release LTO disabled and 32 codegen units for practical validation rebuilds; these builds are not timing baselines):

```sh
cargo test --release --lib --no-default-features --features clap,vst3 --offline --config 'profile.release.lto=false' --config 'profile.release.codegen-units=32' -- --test-threads=2
cargo test --release --lib --no-default-features --features clap,vst3,rt-paranoid --offline --config 'profile.release.lto=false' --config 'profile.release.codegen-units=32' --config 'profile.release.package.pure_va_dispersion_core.debug-assertions=true' -- --test-threads=2
python3 scripts/test-measurements.py
cargo fmt --all -- --check
cargo check --release --example process_lab --no-default-features --features clap,vst3,process-lab --offline --config 'profile.release.lto=false' --config 'profile.release.codegen-units=32'
```

All-target Clippy compilation succeeds after removing the obsolete experiment calls, but the warning-count ratchet **fails: 6,747 warnings against the saved 6,523 baseline**. The baseline was not raised. This is not a claim that the repository's complete quality gate is green. The library-only lint count cannot be substituted for the all-target count.

```sh
cargo clippy --release --all-targets --no-default-features --features clap,vst3 --offline --config 'profile.release.lto=false' --config 'profile.release.codegen-units=32' --message-format=json > clippy.json
python3 scripts/clippy-ratchet.py < clippy.json
```

The measurement script's accumulator check compiles the extracted production loop body with fail-closed extraction markers. It does not execute an entire plugin callback with injected audio. The separate plugin library tests cover real routing/rendering; these are distinct scopes of evidence.

### Build prerequisite and publication limits

This branch inherits missing tracked build inputs from main: `src/licensing/mod.rs`, its non-licensing backend, and `vendor/matari-audio-drag-and-drop-0.1.6`. Cargo also resolves the sibling `../derpcat-access` path. Local untracked inputs from the user's checkout supplied those prerequisites for the full-crate runs; they are not included or changed by this PR. Thus these results **do not establish clean-checkout CI or licensed release qualification**. No installed plugin or running DAW was replaced, and no Windows/macOS or host-UI acceptance is claimed.

Independent machine stress workers were active throughout. CPU ratios in the retained experimental log are deliberately excluded from speedup claims.
