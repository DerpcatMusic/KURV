# KURV realtime DSP architecture checkpoint

Date: 2026-08-28

This is the current implementation contract and measured optimization ledger. It distinguishes shipped behavior from proposed work.

## Signal ownership

```text
control/UI thread
  publishes bounded parameters, graph edits, and immutable source data
                  |
                  v
audio callback
  advances smoothers and LFO/control frames without allocation or locks
                  |
                  v
polyphonic note voice (independent modulation phase and DSP state)
  ordered generator group
    oscillator source -> unison lane render -> stereo lane sum
    noise source      -> unison random lanes -> stereo lane sum -> source color/texture
    later oscillator  <- optional earlier same-group oscillator PM
    ordered filter    -> one stateful filter per note, after preceding oscillator modules
                  |
                  v
sum polyphonic note voices -> group gain/pan/output pair -> host output
```

“Per voice” means per active polyphonic note, not per unison lane. Filtering every unison lane would multiply filter state and CPU while changing the sound. KURV instead filters the stereo sum of the preceding oscillator/unison module inside each note voice. This preserves note-specific envelopes and modulation without paying for one filter cascade per unison lane.

Filters cannot be applied once to the oscillator definition before voices exist: an oscillator definition has no audio signal or causal filter state. Each active note has a different phase, pitch, envelope, modulation value, and filter history. Filtering after all polyphonic voices are summed would be cheaper, but it would lose polyphonic filter modulation and make notes interact through one shared state.

## Realtime boundary

`Plugin::process()` and everything it calls must not allocate, lock, block, log, perform I/O, resize storage, compile curves, analyze audio, or render editor geometry. Fixed-capacity voice/filter/noise state is prepared before processing. Runtime controls are finite-clamped before they reach recursive DSP. A non-finite filter output resets that note's filter state and returns silence for the affected sample.

The editor and analysis paths may do heavier work. Filter visualization evaluates the implemented complex transfer function with the same sanitized parameter mapping and coefficient tables as realtime DSP. LFO playback uses the compiled curve evaluator; editor line segments must agree with that evaluator.

## Current filter contract

| Mode | Realtime topology | Continuous control | Honest limitation |
|---|---|---|---|
| SVF Morph | Up to 64 cascaded second-order TPT SVF sections | Adjacent-order output blend; LP/BP/HP morph | A causal high-order IIR necessarily rotates phase and has a longer transient. Intermediate order is a response crossfade, not a mathematical fractional-order Butterworth prototype. |
| Phaser | Up to 128 first-order all-pass sections mixed with dry | Newest section moves continuously from algebraic bypass; Q controls dry/effected depth; spacing controls logarithmic span | All-pass stages are not one-to-one “notches.” Deep cancellation depends on accumulated phase and dry/wet mix. |
| Scream | Saturated TPT low-pass with saturated high-pass feedback branch and bounded gate | Resonance, scream character, and wet mix | Scream-inspired, not a bit-identical port of Cure Audio Scream's ADAA2/gain/keytracking design. |

The detailed source and dependency survey is in [filter-topology-research-2026-08-28.md](research/filter-topology-research-2026-08-28.md). No surveyed crate beat the local fixed-capacity Rust/SIMD seam strongly enough to justify a dependency or FFI boundary.

## Measured filter matrix

Pinned local `iter` profile, 48 kHz, 64-frame callback, 32 active polyphonic notes, median of seven runs. Absolute numbers vary with machine load; accept/reject decisions use paired runs with identical commands.

| Scenario | Median ns/frame | Notes |
|---|---:|---|
| SVF, 24 dB/oct | 1,437 | Current cached complete stage coefficients |
| SVF, 768 dB/oct | 16,450 | 64 recursive second-order sections |
| SVF cutoff modulation | 4,842 | Audio-rate per-note route |
| SVF slope modulation | 5,824 | Continuous adjacent-stage activation |
| Phaser, 128 stages | 12,320 | Recursive all-pass bank dominates |
| Phaser cutoff modulation | 13,343 | Continuous coefficient lookup |
| Phaser spacing modulation | 16,032 | After ratio-surface optimization |
| Scream | 1,831 | Two TPT sections plus nonlinear feedback |
| Scream cutoff modulation | 4,852 | Coefficient/control work dominates the two-stage core |

### Accepted winners

| Change | Result |
|---|---|
| Ordered filter fast paths and coefficient caching | Removed redundant full configuration rebuilds while preserving bounded modulation and exact response checks. |
| Phaser continuous stage insertion | Eliminated abrupt stage spawn/removal and retained state through motion. |
| Phaser 256-step span ratio surface with linear interpolation | Spacing modulation: 18,684 -> 16,032 ns/frame, 14.2% faster; all 19 focused filter checks passed. The table is initialized off the audio thread and interpolation remains continuous. |
| Cache complete SVF stage coefficients in existing scratch storage | 768 dB/oct: 16,668 -> 16,450 ns/frame; slope modulation: 5,939 -> 5,824; bit-identical output and no added state. |
| Noise settled block routing | 64-note integrated process: 3,952 -> 2,078 ns/frame. |
| Noise mono endpoint specialization | 64 unison lanes: 295.53 -> 149.96 ns/source-sample by avoiding a discarded independent random stream. |
| Polyphonic PM block path | Modulated 32-note PM: about 14.91 -> 2.29 microseconds/frame while preserving per-note modulation. |
| Local compiled LFO cells | Neutral editor segments now evaluate as linear in realtime; the old global spline fit and solver were deleted. |

### Rejected trials

| Trial | Why it was removed |
|---|---|
| Manual four-stage filter unroll | SVF maximum order regressed 2.1%; phaser was neutral/slower. |
| AVX2 coefficient-table gather | Only about 0.8% in one phaser case, changed rounding, and added unsafe platform-specific code. |
| Shared 8 MiB phaser coefficient block | Spacing modulation regressed 18,684 -> 18,825 ns/frame; the benchmark uses genuinely polyphonic coefficient routes. |
| Cached Scream stage objects | Static gain was marginal, but audio-rate cutoff/Q/slope regressed; direct inlined coefficient construction compiles better. |
| Wider Noise PRNG state/SIMD hashing | Less than 1% or slower while increasing state. The compact xorshift64* stream remains the measured winner. |

## Modulation contract

LFO destinations that affect oscillator/filter sound are evaluated per note when polyphonic routing is active. Structural fast paths cover the common single-control cases without rebuilding unrelated parameters. Generator-to-generator PM is also per note and audio-rate:

- source and carrier must be oscillators in the same ordered group;
- source must precede carrier, which makes the graph acyclic without a runtime graph solver;
- the already-rendered source sample modulates the later carrier phase;
- source output is reused, not rendered a second time;
- LFO modulation of PM depth stays in the same block renderer.

AM, RM, and pan modulation are not silently encoded into PM. Adding them requires a persisted route mode, a bounded depth mapping, a clear UI target, and separate aliasing/gain/silence benchmarks. Feedback routes remain rejected until they have an explicit one-sample-delay topology, bounded loop gain, state reset semantics, and hearing-safe output limiting.

## Noise oscillator contract

Noise is an oscillator engine variant and retains common oscillator level, pan, unison, placement, reset, persistence, routing, and modulation behavior. Its continuous source controls are Color, Texture, and Stereo. One compact state lives per oscillator slot per polyphonic note, not per unison lane or filter stage. Full details and source research are in [noise-oscillator-research-2026-08-28.md](research/noise-oscillator-research-2026-08-28.md).

## Next evidence gates

1. Cross-note filter SIMD requires a structure-of-arrays state prototype and a paired release benchmark. Consecutive stages of one IIR are recursive and cannot be parallelized honestly.
2. A more faithful Scream mode requires matched input gain, keytracking, feedback calibration, gate behavior, and either ADAA2 or measured fixed oversampling. Do not claim Cure parity before rendered comparisons.
3. AM/RM/pan generator routing requires one explicit route-mode model; add only modes that reuse the ordered source render and remain bounded under audio-rate modulation.
4. Host acceptance still needs listening and automation sweeps in the real DAW for clicks, note release behavior, CPU, and visualization agreement. Static tests and the process lab are not host proof.
5. Preserve each accepted optimization as a small `main` checkpoint. Revert trials that do not beat paired medians or that weaken numerical/sonic guarantees.

