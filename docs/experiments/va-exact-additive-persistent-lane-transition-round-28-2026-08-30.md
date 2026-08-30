# Exact-additive persistent-lane transition, round 28 (rejected)

Date: 2026-08-30
Baseline: `abdb6dd17b48ef61e0fb4b635e80471d6069eb71`

## Verdict

Reject the test-only exact-additive secondary mode. A block-reseeded finite
recurrence reproduced the legal cap-three saw and square projections accurately
when the backend was already stable, but the bounded one-block handoff did not
remove the active-pitch seam and the candidate lost every 4/16/64-block CPU
gate. No production parameter, UI, selector, renderer, version, or release code
was changed.

The prototype has been removed after preserving these results. The separate
round-27 finite-DSF source harness remains outside this experiment.

## Prototype contract

The ignored release probe entered at the same structural boundary as the real
constant-x8 saw and generic-shape renderers:

- saw used the `accumulate_saw8_block_constant` control path;
- square used `accumulate_shape8_block_constant` at shape 3 and width 0.5;
- one persistent state record was indexed by each of the eight oscillator lanes,
  rather than attached to a temporary packed block;
- `VaOscillator` phase was the sole authoritative phase and the recurrence was
  reseeded from it at every block;
- the exact branch retained at most three legal harmonics and used analytic
  saw/square Fourier coefficients;
- an already-active lane crossing between cap four and cap three rendered both
  families for one 32-frame block and used a smoothstep output crossfade;
- note-on selected its eligible owner directly, note-off did not introduce an
  engine handoff, and reset cleared the oscillator before direct reseeding;
- transition state occupied 24 bytes for eight lanes; the sample kernel had no
  allocation, locks, I/O, logging, or table bank.

This is the smallest honest production-shaped seam available for the retained
experiment. It deliberately did not claim support for warp, PWM, phase
modulation, triangle, pulse, or custom curves.

## Accuracy and artifact gates

Errors were measured against the exact legal finite Fourier projection using
the authoritative sample phases. `delta_error_peak` is the peak difference
between the candidate and exact adjacent-sample deltas. The current column is
the same metric for the shipping spline renderer on the identical schedule.

| wave | schedule | candidate peak | candidate RMS | delta error peak | current delta error peak | ratio |
|---|---|---:|---:|---:|---:|---:|
| saw | cap 3 to cap 2 | 0.000003535 | 0.000000924 | 0.000005452 | 0.750137284 | 0.000007 |
| saw | cap 4 to cap 3 pitch crossing | 0.388231581 | 0.119555034 | 0.744969145 | 0.750137881 | 0.993110 |
| saw | phase reset at cap 3 | 0.000003535 | 0.000000910 | 0.000005452 | 0.754434993 | 0.000007 |
| square | cap 3 to cap 2 | 0.000005524 | 0.000001654 | 0.000006975 | 0.927388004 | 0.000008 |
| square | cap 4 to cap 3 pitch crossing | 0.473908495 | 0.162042751 | 0.920947932 | 0.927388810 | 0.993055 |
| square | phase reset at cap 3 | 0.000004879 | 0.000001442 | 0.000006975 | 0.933882652 | 0.000007 |

Stable cap changes and reset replay confirm that authoritative per-lane phase
ownership fixes the transient-pack lifetime problem. They do not solve the
backend identity problem. At an active cap-four/cap-three pitch crossing, the
one-block smoothstep retained about 99.3% of the shipping path's delta error for
both waves. The handoff therefore fails before considering CPU.

Note-on and note-off were represented in the duty-cycle workloads. A high note
started directly in the additive owner with no stale transition state, and
note-off ended ownership without switching the renderer. This avoids a fake
note-boundary crossfade but did not rescue throughput.

## Real x8 duty-cycle CPU

The release probe measured the real stereo accumulation boundary before the
common oversampler. Values are median nanoseconds per x8 output frame; ranges
are the minimum and maximum of five runs. Each note used 32-frame blocks and
reinitialized persistent oscillator/lane ownership at note-on.

### Direct cap-three note-on

| wave | note blocks | current median | candidate median | ratio | current range | candidate range |
|---|---:|---:|---:|---:|---:|---:|
| saw | 4 | 6.114144 | 52.578433 | 8.599475 | 5.527490-6.513073 | 51.699720-56.496292 |
| saw | 16 | 4.549193 | 48.932586 | 10.756321 | 3.954395-5.600505 | 44.782521-51.280163 |
| saw | 64 | 4.107339 | 46.518868 | 11.325793 | 3.917621-5.137058 | 42.337601-47.545747 |
| square | 4 | 22.315547 | 43.817786 | 1.963554 | 19.772282-24.072123 | 43.371821-46.669685 |
| square | 16 | 20.010151 | 43.629012 | 2.180344 | 18.982862-20.463840 | 42.212590-45.280853 |
| square | 64 | 20.038753 | 42.814665 | 2.136593 | 19.083316-20.261153 | 42.407821-48.820651 |

### Active cap-four to cap-three pitch crossing

| wave | note blocks | current median | candidate median | ratio | current range | candidate range |
|---|---:|---:|---:|---:|---:|---:|
| saw | 4 | 4.072424 | 35.800799 | 8.791030 | 4.028897-4.586578 | 34.911702-38.550758 |
| saw | 16 | 4.018750 | 41.334069 | 10.285304 | 3.877708-4.454797 | 40.060195-41.630352 |
| saw | 64 | 4.236975 | 42.963555 | 10.140149 | 3.960966-4.401695 | 42.336389-45.830485 |
| square | 4 | 19.729346 | 43.093667 | 2.184242 | 19.418605-20.020679 | 42.271749-43.335276 |
| square | 16 | 19.730833 | 43.080459 | 2.183408 | 19.205321-20.168921 | 41.542927-43.710229 |
| square | 64 | 19.721450 | 43.544640 | 2.207984 | 19.118841-20.204026 | 43.089529-47.253489 |

Direct ownership is the most favorable lifecycle policy because it avoids dual
rendering altogether. Even there, saw was 8.60-11.33 times current and square
was 1.96-2.18 times current. Adding the one bounded pitch handoff did not create
a crossover at any tested note length.

## Reproduction

The focused source was formatted and run once from its isolated worktree:

```bash
cargo fmt -- src/oscillators/va/experiment.rs
cargo test --release -q exact_additive_transition_round28::exact_additive_transition_report -- --ignored --nocapture
```

Result: one focused test passed, 409 tests were filtered out, and the report
completed in 3.55 seconds after the release build. The build emitted existing
unused/dead-code warnings; it emitted no prototype compile errors.

## Decision boundary

There is no production seam to expose from this result. Correct ownership is
necessary but insufficient: the finite recurrence is too expensive relative to
the optimized saw path, remains slower than the square path, and a bounded
dual-render crossfade still carries the underlying renderer mismatch through an
active pitch transition. Square and triangle work must not proceed from this
candidate as though saw had cleared the common gate.
