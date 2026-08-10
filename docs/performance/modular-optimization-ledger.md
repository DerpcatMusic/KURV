# KURV Modular Optimization Ledger

Append-only evidence log for the modular oscillator performance campaign. An
optimization is accepted only when the same pinned-core release workload gets
meaningfully faster and its output-equivalence or sound-quality gate passes.
Rejected experiments stay in this file.

## Measurement contract

- Host rate: 48 kHz
- CPU: AMD Ryzen 7 7800X3D, benchmark process pinned to logical CPU 4
- OS: Linux 7.2.0-rc5-1-cachyos-rc x86_64
- Rust: 1.97.1 (`8bab26f4f`, LLVM 22.1.6)
- Cargo: 1.97.1
- Release flags: `RUSTFLAGS='-C target-cpu=x86-64-v3'`
- Build: `cargo build --release --no-default-features --example generator_lab --locked`
- Timing: internal warm-up followed by median/min/max across the recorded repeats
- CPU gate: same source-derived benchmark, arguments, pinning, toolchain, and build flags
- DSP gate: deterministic checksum for exact-path changes; max/RMS residual and continuity or alias metrics for intentional sound changes
- Realtime gate: no process-time allocation, deallocation, locks, I/O, syscalls, logging, or unbounded work

## Frozen baseline

- Branch: `codex/modular-core-shell`
- Commit: `babdb93e6d22dd76ec8535b4aee477e86f8c73c7`
- Remote: `origin/codex/modular-core-shell`
- Installed artifact: `build-20260809T233758-2080510`
- CLAP SHA-256: `5f7a766e470b722ed89dcb0691e540bdee7bd86f905e7e1f3aee0deabc633b97`
- VST3 binary SHA-256: `281e24b88525352907e07678b5dc25cb41d097fdcf66259188e74ed3eb427014`
- Original lab binary SHA-256: `f6d4d50a4190a92083b098fe17239e088352a8a89a8b23708010eb3767b2cc34`

### Legacy compatibility path

All rows use Spline Optimized, 2x oversampling, saw, MIDI 69, no jitter.

| Oscillators | Unison | Polyphony | Frames x repeats | Median ns/frame | Min | Max |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 1 | 500,000 x 9 | 207.437 | 203.327 | 217.881 |
| 1 | 8 | 8 | 250,000 x 9 | 317.806 | 308.510 | 337.386 |
| 3 | 8 | 8 | 200,000 x 9 | 484.094 | 475.843 | 498.992 |
| 3 | 64 | 24 | 50,000 x 7 | 4,344.228 | 4,208.900 | 4,412.447 |

## Measurement infrastructure

### M0001 - Structural oscillator bank workload

- Change: added `bench-bank` and `bench-bank-pool` to `examples/generator_lab.rs`
- Production DSP changed: no
- Lab binary SHA-256: `742a2169c4a31bd53435591d23f683f40f2e673a2a0911278bc62200cb7520f9`
- Decision: accepted as campaign infrastructure

The workload disables the three legacy oscillators, configures 1-32 instances
through the same `OscillatorDspConfig` array used by the plugin, warms transition
state, and renders through `PolySynth::render_block` plus the real oversampler.

| Path | Oscillators | Unison | Polyphony | Frames x repeats | Median ns/frame | Min | Max | Versus comparable legacy |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Bank | 1 | 1 | 1 | 500,000 x 9 | 418.672 | 400.194 | 426.763 | 2.02x slower |
| Bank | 1 | 8 | 8 | 250,000 x 9 | 1,974.131 | 1,925.174 | 2,085.122 | 6.21x slower |
| Bank | 3 | 8 | 8 | 150,000 x 9 | 3,580.663 | 3,378.067 | 3,643.578 | 7.40x slower |
| Bank | 8 | 8 | 8 | 75,000 x 7 | 7,409.754 | 7,125.475 | 7,873.256 | n/a |
| Bank | 32 | 1 | 8 | 75,000 x 7 | 5,496.569 | 4,999.542 | 5,649.816 | n/a |

Finding: the structural path is not a small bookkeeping regression. It bypasses
the legacy block/SIMD oscillator kernels and renders scalar oscillator lanes per
sample. The steady-state block path also copies the large active oscillator set
before rendering each active voice. Both costs now have dedicated workloads.

## Accepted production patches

### P0001 - Borrow settled oscillator banks instead of copying them

- Files: `src/voices/voice.rs`
- Hypothesis: the settled block renderer is copying the full 32-slot bank once
  globally and once per active voice, then scanning transition state once per
  sample even though no transition is active.
- Change: use a shared immutable bank in the steady-state block renderer. Keep
  the prior copy-and-advance renderer only while a module is adding, removing,
  or changing over its 8 ms transition.
- Realtime impact: removes work and memory traffic; adds no allocation, lock,
  I/O, syscall, or unbounded operation.
- Candidate lab SHA-256: `3f0848e1f885d55481c440a9699ca8b1e936e8e2c7809b9c793cc90a1691797a`

| Oscillators | Unison | Polyphony | Before ns/frame | After ns/frame | Time reduction | Speedup | Checksum |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 1 | 418.672 | 141.296 | 66.25% | 2.96x | exact |
| 1 | 8 | 8 | 1,974.131 | 1,088.984 | 44.84% | 1.81x | exact |
| 3 | 8 | 8 | 3,580.663 | 2,517.881 | 29.68% | 1.42x | exact |
| 8 | 8 | 8 | 7,409.754 | 6,278.924 | 15.26% | 1.18x | exact |
| 32 | 1 | 8 | 5,496.569 | 3,713.465 | 32.44% | 1.48x | exact |

Validation:

- All five deterministic benchmark checksums matched M0001 exactly.
- `cargo test --locked --lib voices::voice::tests`: 3 passed, 0 failed.
- The transition path and its arithmetic order remain unchanged.
- Decision: accepted.

### P0002 - Skip dormant legacy synthesis in structural-only blocks

- Files: `src/voices/voice.rs`
- Hypothesis: materialized generator stacks disable all three legacy
  oscillators, but every structural sample still enters the legacy scalar
  renderer before rendering the structural bank.
- Change: in settled structural-only blocks, advance the envelope and glide
  directly and render the oscillator bank. Mixed legacy/structural and
  transitioning paths remain unchanged.
- Realtime impact: removes duplicated scalar bookkeeping; adds no realtime
  resource operation.
- Candidate lab SHA-256: `204d7f040d071f0b451a277ff582045585a0c01db17569d164a6f48be5c00f34`

| Oscillators | Unison | Polyphony | Before ns/frame | After ns/frame | Time reduction | Speedup | Checksum |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 1 | 141.296 | 113.840 | 19.43% | 1.24x | exact |
| 1 | 8 | 8 | 1,088.984 | 845.189 | 22.39% | 1.29x | exact |
| 3 | 8 | 8 | 2,517.881 | 2,212.640 | 12.12% | 1.14x | exact |
| 8 | 8 | 8 | 6,278.924 | 5,709.063 | 9.08% | 1.10x | exact |
| 32 | 1 | 8 | 3,713.465 | 3,463.759 | 6.72% | 1.07x | exact |

Validation:

- All five deterministic benchmark checksums matched P0001 exactly.
- `cargo test --locked --lib voices::voice::tests`: 3 passed, 0 failed.
- Frozen-baseline default path: 418.672 to 113.840 ns/frame, a cumulative
  72.81% time reduction and 3.68x speedup.
- Decision: accepted.

### P0003 - Remove callback-time fallback-wave compilation

- Files: `src/lib.rs`, `src/shell.rs`
- Hypothesis: constructing `WaveCurveRt::default()` for every one of the 32
  oscillator slots compiles a dynamic knot curve and allocates twice per slot
  inside every audio callback.
- Change: compile the fallback curve once while constructing `KurvDspState`
  and copy its fixed 64-float realtime representation during processing.
- Realtime impact: removes all heap activity found by Truce's allocation
  checker from the measured process path.

| Metric | Before | After |
|---|---:|---:|
| Realtime violations per callback | 64 | 0 |
| Fallback curve compilations per callback | 32 | 0 |
| Default curve compilations per DSP-state construction | 3 | 1 |

Validation:

- Gate: `RUST_MIN_STACK=16777216 cargo test --locked --lib --features rt-paranoid block_path_respects_pitch_and_mpe_event_boundaries`
- The new feature-gated process audit failed before the production change with
  exactly 64 realtime violations and passed afterward with zero.
- The existing event-boundary comparison still passed across 1x-4x
  oversampling and both block/scalar paths.
- Output arithmetic is unchanged; `WaveCurveRt` is copied from the same
  compiled default coefficients.
- Decision: accepted.

## Oscillator scaling scoreboard

The primary product target is oscillator-level scaling inside one group. Group
routing optimizations are tracked separately and cannot hide oscillator cost.

Current settled Spline Optimized 2x results at 8-note polyphony and 8 unison
lanes per oscillator:

| Oscillators | Median ns/frame | Increment from prior row | Median per oscillator |
|---:|---:|---:|---:|
| 1 | 845.189 | 845.189 | 845.189 |
| 3 | 2,212.640 | 683.726 per added oscillator | 737.547 |
| 8 | 5,709.063 | 699.285 per added oscillator | 713.633 |

The current curve is already mildly sublinear because envelope, voice, and
block overhead are shared, but scalar lane synthesis still dominates. The next
oscillator-level target is to restore packed block rendering so marginal cost
falls further.

## Rejected experiments

### R0001 - Zero-jitter settled-state flags

- Files tested: `src/voices/voice.rs`
- Hypothesis: remember when each oscillator's jitter ratios are exactly unity,
  then skip the per-lane zero-step additions while jitter remains disabled.
- Candidate lab SHA-256: `1fdf74b7dd57ac2cc1dde9d9d9368bc6e284617480280700b129296d47ea675f`
- Output: all deterministic checksums matched the accepted path exactly.

| Oscillators | Unison | Polyphony | Before ns/frame | Candidate ns/frame | Change |
|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 1 | 113.840 | 116.588 | 2.41% slower |
| 1 | 8 | 8 | 845.189 | 917.379 | 8.54% slower |
| 3 | 8 | 8 | 2,212.640 | 2,451.092 | 10.78% slower |
| 8 | 8 | 8 | 5,709.063 | 6,193.818 | 8.49% slower |
| 32 | 1 | 8 | 3,463.759 | 3,913.467 | 12.98% slower |

Finding: two extra per-slot flags and their hot-path branches cost more than
the compiler's existing additions of zero. The dense workloads regressed by
8-13%, so the production experiment was fully removed. Decision: rejected.

### M0002 - Structural oscillator note-trigger workload

- Change: added `bench-trigger-bank` to `examples/generator_lab.rs` and shared
  its structural configuration builder with the steady-state bank benchmark.
- Production DSP changed: no.
- Lab binary SHA-256: `a9a7b6d97b59772e3f41415beb41faa5f8ab4939565641dcb2c0bf8c11ec59ce`
- Workload: 64 unison lanes, 192 kHz internal rate, 301 repetitions, pinned CPU
  4. Setup and configuration are outside the timed interval; the interval is
  the complete sequence of structural `note_on` calls.
- Decision: accepted as campaign infrastructure.

| Polyphony | Oscillators | Baseline median trigger ns |
|---:|---:|---:|
| 8 | 1 | 30,401 |
| 8 | 3 | 30,491 |
| 8 | 8 | 30,660 |
| 8 | 32 | 30,511 |
| 32 | 1 | 122,972 |
| 32 | 3 | 123,182 |
| 32 | 8 | 122,612 |
| 32 | 32 | 123,232 |

Finding: trigger cost is effectively flat across active oscillator counts
because every note currently seeds all 32 slots and all 64 lanes. This workload
isolates an oscillator-level deadline spike hidden by the steady-state test.

### P0004 - Seed only active structural oscillators on note trigger

- Files: `src/voices/voice.rs`
- Hypothesis: note-on initializes 2,048 oscillator lanes per voice regardless
  of whether the patch contains 1, 3, 8, or 32 oscillators.
- Change: seed the active/fading slot list already owned by
  `ActiveOscillatorSet`; newly enabled slots retain their dedicated seeding
  path.
- Realtime impact: removes inactive oscillator writes and random-phase work;
  adds no resource operation or unbounded work.
- Candidate lab SHA-256: `913cc4897452b56a5bb2f423cfcb660a54d81d24cbd996eded67f9fc79f90c8f`

| Polyphony | Oscillators | Before ns | After ns | Time reduction |
|---:|---:|---:|---:|---:|
| 8 | 1 | 30,401 | 2,640 | 91.32% |
| 8 | 3 | 30,491 | 4,470 | 85.34% |
| 8 | 8 | 30,660 | 9,030 | 70.55% |
| 8 | 32 | 30,511 | 30,561 | -0.16% |
| 32 | 1 | 122,972 | 10,530 | 91.44% |
| 32 | 3 | 123,182 | 18,041 | 85.35% |
| 32 | 8 | 122,612 | 36,181 | 70.49% |
| 32 | 32 | 123,232 | 123,372 | -0.11% |

Validation:

- Active-slot phase and jitter initialization still calls the identical
  `seed_slot` implementation with the identical slot, seed, and target.
- The 32-oscillator control rows are unchanged within 0.2% measurement noise.
- `cargo test --locked --lib voices::voice::tests`: 3 passed, 0 failed.
- Decision: accepted.

### P0005 - Copy only rendered oscillator rows through the realtime pool

- Files: `src/voices/voice.rs`, `src/voices/internal_rt_pool.rs`
- Hypothesis: pool preparation and commit copy 49,536 bytes of structural
  oscillator state per active polyphonic voice and job, independent of active
  oscillator count.
- Change: copy only the active/fading slot list and each slot's rendered phase
  lanes. Complete jitter rows remain copied so unison shrink/expansion and
  inactive-lane cleanup preserve existing behavior.
- Realtime impact: removes fixed memory traffic; adds no allocation, lock, I/O,
  syscall, or unbounded work.
- Candidate lab SHA-256: `0ab6eee9c3ac17e8e8cebbe836b427b0d47f3681a5f960accf90654a2a487e0e`
- Pool measurement exception: the process was intentionally left unpinned so
  seven helper threads could run concurrently; every helper reported FIFO
  scheduling. Commands and machine state were otherwise identical.

| Oscillators | Unison | Polyphony | Before ns/frame | After ns/frame | Time reduction | Checksum |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 8 | 24 | 908.596 | 824.612 | 9.24% | exact |
| 3 | 8 | 24 | 1,777.738 | 1,641.620 | 7.66% | exact |
| 8 | 8 | 24 | 3,831.486 | 3,752.313 | 2.07% | exact |
| 1 | 64 | 24 | 3,590.735 | 3,496.245 | 2.63% | exact |
| 32 | 8 | 24 | 14,112.448 | 14,337.145 | -1.59% | exact |

Validation:

- Pooled and serial checksums matched exactly for 1, 3, and 8 oscillators.
- The 32-oscillator control retains all rows and stayed within the run's noisy
  deadline-fallback range; it is not counted as a gain.
- `cargo test --locked --lib voices::voice::internal_rt_pool::tests`: 5 passed,
  0 failed, including bit-null, timeout fallback, recovery, and 1x-4x cases.
- Decision: accepted for the repeatable 1-3 oscillator gains.

### M0003 - Structural phase-warp switch continuity sweep

- Change: added `sweep-bank-warp` to `examples/generator_lab.rs`.
- Production DSP changed: no.
- Lab binary SHA-256: `0dedc886f01f6ebe5c1f45ba3acda651f36b0109cf5b06440a816ac914673db7`
- Workload: one structural saw, one unison lane, 2x Spline Optimized,
  96 kHz internal rate, warp amount 0.98, all 12 ordered mode changes.
- Metric: maximum first difference of `changed - old-mode reference`; this
  isolates the automation discontinuity from the saw's natural reset step.
- Decision: accepted as sound-quality campaign infrastructure.

| Start | Target | Baseline maximum residual step |
|---|---|---:|
| None | PWM | 0.146089375 |
| None | Bend | 0.078361131 |
| None | Harmonic | 0.083763503 |
| PWM | None | 0.146089375 |
| PWM | Bend | 0.224450499 |
| PWM | Harmonic | 0.062325865 |
| Bend | None | 0.078361131 |
| Bend | PWM | 0.224450499 |
| Bend | Harmonic | 0.162124634 |
| Harmonic | None | 0.083763503 |
| Harmonic | PWM | 0.062325865 |
| Harmonic | Bend | 0.162124634 |

Finding: structural mode changes currently switch transfer functions in one
sample. The worst normalized residual step is 0.22445 even though settled output
reaches the target exactly. A CPU-neutral transition must reduce every pair,
settle to the exact target, and leave the steady renderer unchanged.

### M0004 - Scalar-versus-block structural output comparison

- Change: added `compare-bank-block` to `examples/generator_lab.rs`.
- Production DSP changed: no.
- Candidate lab SHA-256: `4a55003ae3b75d8800e3237117e4de5943f193d23efa55a01e76eaa3ba7bb66b`
- Method: render two phase-aligned structural synths through scalar and block
  APIs, then report maximum and RMS sample residuals across both channels.
- Decision: accepted as output-parity infrastructure.

| Oscillators | Unison | Shape/variant | Frames | Max abs residual | RMS residual |
|---:|---:|---|---:|---:|---:|
| 1 | 1 | Saw | 320,000 | 0 | 0 |
| 1 | 4 | Saw | 320,000 | 2.384e-7 | 1.868e-8 |
| 1 | 5 | Saw | 320,000 | 2.384e-7 | 4.061e-8 |
| 1 | 7 | Saw | 320,000 | 2.980e-7 | 4.117e-8 |
| 1 | 8 | Saw | 320,000 | 3.576e-7 | 3.765e-8 |
| 3 | 8 | Saw | 320,000 | 7.153e-7 | 9.547e-8 |
| 8 | 8 | Saw | 320,000 | 1.907e-6 | 2.380e-7 |
| 32 | 1 | Saw | 320,000 | 0 | 0 |
| 1 | 8 | Triangle | 160,000 | 2.384e-7 | 4.111e-8 |
| 1 | 8 | Pulse | 160,000 | 9.060e-6 | 6.238e-7 |
| 1 | 8 | PWM/Bend/Harmonic | 480,000 | 3.576e-7 worst | 6.285e-8 worst |
| 1 | 8 | Custom curve | 160,000 | 2.980e-7 | 4.389e-8 |

Finding: packed arithmetic preserves the exact one-lane path. Across packed
paths the worst case is the pulse edge at 9.06e-6 maximum and -122.14 dB RMS;
all other measured paths remain below 1.91e-6 maximum and -135.59 dB RMS.

### P0006 - Restore packed block rendering for structural oscillators

- Files: `src/voices/voice.rs`
- Hypothesis: the modular bank sends every unison lane through scalar waveform
  generation, discarding KURV's existing 8-lane and 4-lane VA kernels.
- Change: settled, held, non-gliding, jitter-neutral structural voices render
  complete lane packs through the existing saw, shape, warped, and custom block
  kernels. Scalar tails and the complete prior fallback remain available.
- Realtime impact: fixed stack accumulators and bounded active-lane loops only;
  no allocation, lock, I/O, syscall, or new DSP stage.
- Candidate lab SHA-256: `4a55003ae3b75d8800e3237117e4de5943f193d23efa55a01e76eaa3ba7bb66b`

| Oscillators | Unison | Polyphony | Before ns/frame | After ns/frame | Time reduction | Cumulative vs M0001 |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 1 | 113.234 | 108.881 | 3.84% | 73.99% |
| 1 | 8 | 8 | 851.022 | 285.166 | 66.49% | 85.56% |
| 3 | 8 | 8 | 2,305.476 | 532.433 | 76.91% | 85.13% |
| 8 | 8 | 8 | 6,018.472 | 1,130.798 | 81.21% | 84.74% |
| 32 | 1 | 8 | 3,427.603 | 2,358.650 | 31.19% | 57.09% |

Oscillator-level scaling after this patch, still inside one group:

| Oscillators | Median ns/frame | Increment from prior row | Median per oscillator |
|---:|---:|---:|---:|
| 1 | 285.166 | 285.166 | 285.166 |
| 3 | 532.433 | 123.634 per added oscillator | 177.478 |
| 8 | 1,130.798 | 119.673 per added oscillator | 141.350 |

Validation:

- M0004 scalar-versus-block comparison: exact for one-lane paths; worst packed
  residual 9.06e-6 peak and -122.14 dB RMS across saw, triangle, pulse, all
  warp modes, custom curves, 4/5/7/8 lanes, and 1/3/8 oscillators.
- `cargo test --locked --lib voices::voice`: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0007 - Pack separate single-lane oscillators into shared SIMD blocks

- Files: `src/voices/voice.rs`
- Hypothesis: a bank of several one-lane oscillators still invokes the packed
  waveform kernel once per oscillator, leaving seven SIMD lanes idle on every
  call.
- Change: compatible settled oscillators with one rendered lane, identical
  shape/pulse width, no custom mix, and no phase warp are gathered into 8-wide
  and 4-wide instance packs. Sparse slot identity, phase, pitch, level, and pan
  remain independent; incompatible banks retain the P0006 renderer.
- Realtime impact: bounded stack gathers and scatters only; no allocation,
  lock, I/O, syscall, shared group cache, or new DSP stage.
- Frozen P0006 lab SHA-256:
  `4a55003ae3b75d8800e3237117e4de5943f193d23efa55a01e76eaa3ba7bb66b`
- Candidate lab SHA-256:
  `9913892d55322da48c9d9ce7c2486a897ffa4c4551897ca82d1f6817960257dd`

One-group oscillator scaling, one unison lane per oscillator and eight-note
polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time reduction | After vs one oscillator |
|---:|---:|---:|---:|---:|
| 1 | 234.540 | 138.285 | 41.04% | 1.00x |
| 3 | 357.503 | 139.078 | 61.10% | 1.01x |
| 8 | 716.529 | 130.469 | 81.79% | 0.94x |
| 32 | 2,397.410 | 323.508 | 86.51% | 2.34x |

The instance pack is intentionally disabled below three oscillators. The
one-oscillator executable improvement therefore comes from code generation and
layout in the complete candidate, not from instance packing, and is not used
to justify the algorithm. The 3/8/32 rows directly exercise the new path.

Dense-unison fallback controls did not regress: 1/3/8 oscillators at eight
unison lanes improved from 281.254/522.417/1,154.927 to
196.899/460.201/1,079.838 ns/frame in the same frozen/candidate comparison.
Those incidental code-layout gains are not attributed to instance packing.

Validation:

- Saw maximum residual: 2.384e-7 at three oscillators, 4.768e-7 at eight,
  and 1.907e-6 at 32; worst RMS residual -138.78 dB.
- Triangle maximum residual: 3.576e-7 at three oscillators, 4.768e-7 at
  eight, and 1.907e-6 at 32; worst RMS residual -139.54 dB.
- Pulse is the measured worst case from reordered floating-point summation:
  2.918e-5 peak and -121.31 dB worst RMS. The waveform algorithm and phase
  trajectory are unchanged.
- PWM, phase bend, harmonic warp, and custom-curve fallback comparisons were
  bit-exact at eight oscillators.
- `RUST_MIN_STACK=16777216 cargo test --locked --lib voices::voice`: 8 passed,
  0 failed. The larger stack is required by the existing pool test fixture.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### M0005 - Repeated plugin-callback cost measurement

- Change: added `examples/process_lab.rs`.
- Production DSP changed: no.
- Workload: a fully initialized plugin instance processes repeated unchanged,
  event-free callbacks through the real `PluginLogic::process` boundary. Setup,
  reset, buffer construction, and the first 4,096 warmup callbacks are outside
  the timed interval.
- Purpose: expose fixed callback work that oscillator-only benchmarks cannot
  see, especially repeated snapshots and reconstruction of all 32 structural
  oscillator configurations.
- Decision: accepted as callback-overhead campaign infrastructure.

### P0008 - Rebuild structural DSP configuration only after a change

- Files: `src/generators/state.rs`, `src/pan_curve.rs`, `src/lib.rs`,
  `src/shell.rs`
- Hypothesis: every unchanged plugin callback copies the complete generator
  snapshot, reads active pan-shape data, materializes 32 `OscillatorDspConfig`
  values, and submits the bank to every voice.
- Change: retain the last coherent generator and pan-shape generations in DSP
  state. Snapshot decoding now exits before initializing its fixed arrays when
  the generation is unchanged. Structural oscillator configurations rebuild
  only after a materialized-mode, generator, active VA-table, or active
  pan-shape change; contended snapshots are retried on the next callback.
- Realtime impact: replaces fixed copies and 32-slot reconstruction with
  atomic generation reads on the unchanged path. No allocation, lock, I/O,
  syscall, or unbounded retry was added.
- Frozen M0005 lab SHA-256:
  `f1ed02cd7e25f13c0801576aa2c2dedb74edb9364dda5b9b35ea1ffe01fcd0a9`
- Candidate lab SHA-256:
  `2a40e9786d77418773f2556c871cd6500a55962e09a0b32aebf1568a36e68c48`

| Frames/callback | Before ns/callback | After ns/callback | Time reduction | Before ns/frame | After ns/frame |
|---:|---:|---:|---:|---:|---:|
| 1 | 4,395.923 | 1,260.163 | 71.33% | 4,395.923 | 1,260.163 |
| 16 | 5,478.345 | 2,168.789 | 60.41% | 342.397 | 135.549 |
| 64 | 8,983.779 | 5,599.291 | 37.67% | 140.372 | 87.489 |

Validation:

- Release build with the campaign target and flags succeeded.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Targeted voice suite: 8 passed, 0 failed.
- The complete library run has 24 passing checks and two pre-existing failing
  assertions (`partial_event_tail_stays_serial` and
  `audio_rate_modulation_block_paths_survive_voice_and_oversampling_stress`);
  both fail identically at the frozen pre-P0008 commit and are not regressions
  from this patch.
- Decision: accepted.

### P0009 - Switch phase-warp modes through the identity transfer

- Files: `src/voices/voice.rs`
- Defect: changing PWM, phase bend, harmonic, or off modes assigned the new
  transfer function in one sample while warp depth was still audible.
- Change: mode changes now fade the current depth linearly to zero over at
  most 4 ms, switch modes only at the identity transfer, then fade to the
  requested depth over at most 4 ms. Amount-only changes retain their existing
  smoothing, newly started oscillators still begin directly at their target,
  and the settled renderer is unchanged.
- Realtime impact: one extra oscillator bitmask and bounded arithmetic only
  while a warp-mode transition is active. The settled path avoids the added
  division entirely.
- Frozen P0008 lab SHA-256:
  `9913892d55322da48c9d9ce7c2486a897ffa4c4551897ca82d1f6817960257dd`
- Candidate lab SHA-256:
  `88be5d75d9acf6748732df903aa480fa446700aa9042824361fdcdd709f451bf`

| Start | Target | Before max residual step | After max residual step | Reduction |
|---|---|---:|---:|---:|
| None | PWM | 0.146089375 | 0.004059464 | 97.22% |
| None | Bend | 0.078361131 | 0.004059434 | 94.82% |
| None | Harmonic | 0.083763503 | 0.004059583 | 95.15% |
| PWM | None | 0.146089375 | 0.004059464 | 97.22% |
| PWM | Bend | 0.224450499 | 0.006443888 | 97.13% |
| PWM | Harmonic | 0.062325865 | 0.007372558 | 88.17% |
| Bend | None | 0.078361131 | 0.004059434 | 94.82% |
| Bend | PWM | 0.224450499 | 0.006443888 | 97.13% |
| Bend | Harmonic | 0.162124634 | 0.008118916 | 94.99% |
| Harmonic | None | 0.083763503 | 0.004059583 | 95.15% |
| Harmonic | PWM | 0.062325865 | 0.006992340 | 88.78% |
| Harmonic | Bend | 0.162124634 | 0.008118916 | 94.99% |

Validation:

- Worst ordered transition: 0.224450499 to 0.008118916, a 96.38% reduction.
- All 12 transitions reached the exact target within the 8,192-frame sweep;
  every tail target error was zero.
- Settled CPU controls were neutral: one oscillator/eight unison lanes changed
  from 201.818 to 195.533 ns/frame; eight oscillators/one lane changed from
  132.334 to 131.704 ns/frame. Neither gain is attributed to this patch.
- Targeted voice suite: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0002 - Rejected manual 4-lane saw BLEP precomputation

- Candidate file: `src/oscillators/va/render.rs`
- Hypothesis: manually hoisting constant-step spline BLEP support and inverse
  phase-step values out of `accumulate_saw4_block_constant` would remove SIMD
  divisions from each sample.
- Output: candidate and frozen checksums were exact. Scalar-versus-block
  residuals stayed within the existing P0007 bounds.
- First run: three oscillators improved 149.030 to 140.315 ns/frame (5.85%),
  four improved 148.607 to 146.014 (1.74%), and one oscillator with four
  unison lanes improved 201.162 to 198.407 (1.37%).
- Longer confirmation: three oscillators improved only 143.293 to 139.562
  ns/frame (2.60%), while four oscillators regressed from 144.239 to 146.454
  ns/frame (-1.54%).
- Finding: optimized compilation already hoists enough of the constant work;
  the duplicated loop increased code size without a stable material gain.
- Decision: rejected and fully reverted from production.

### M0006 - Disabled-to-active structural jitter continuation comparison

- Change: added the `jitter-on` variant to `compare-bank-block`.
- Production DSP changed: no.
- Workload: phase-aligned scalar and block synths run with structural jitter
  disabled for 4,096 frames, then enable full noise jitter and compare every
  output sample through the configuration transition.
- Purpose: prove that optimized disabled-jitter bookkeeping preserves the
  free-running clock, countdown, and future audible jitter trajectory.
- Decision: accepted as sound-equivalence campaign infrastructure.

### P0010 - Advance disabled structural jitter once per block

- Files: `src/voices/voice.rs`
- Profile: `advance_structural_jitter` consumed 56.88% of samples in the
  settled eight-oscillator/eight-lane workload even though jitter was disabled,
  every rendered ratio was exactly 1.0, and every step was exactly zero.
- Change: the already-qualified settled block path preserves the jitter clock
  and update countdown with the identical per-frame arithmetic, but removes
  all per-lane additions of zero and avoids rebuilding zero-valued jitter
  targets. Tail lanes are normalized at the same refresh boundary.
- Realtime impact: bounded stack-free scalar bookkeeping per oscillator block;
  no allocation, lock, I/O, syscall, approximation, or group-level cache.
- Frozen P0009 lab SHA-256:
  `88be5d75d9acf6748732df903aa480fa446700aa9042824361fdcdd709f451bf`
- Candidate lab SHA-256:
  `f386df61aeea74f468e6ef68bc32791d9cab0d3e47496195ecf1201f05437f5f`

| Oscillators | Unison | Polyphony | Before ns/frame | After ns/frame | Time reduction | Checksum |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 8 | 8 | 211.533 | 143.947 | 31.95% | exact |
| 3 | 8 | 8 | 442.353 | 292.820 | 33.80% | exact |
| 8 | 8 | 8 | 1,076.535 | 678.421 | 36.98% | exact |

Controls:

- Active-jitter 8x8 stayed on the unchanged scalar path and moved from
  5,688.252 to 5,595.168 ns/frame; the 1.64% difference is not attributed.
- The one-lane 32-oscillator path moved from 326.351 to 325.674 ns/frame;
  the 0.21% difference is measurement noise.

Validation:

- M0006 disabled-to-full-jitter comparison was bit-exact for 1, 3, and 8
  oscillators across 640,000 measured frames each.
- Targeted voice suite: 8 passed, 0 failed on confirmation. One preceding run
  exposed the existing worker-participation scheduling flake, then passed
  unchanged on immediate rerun.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0003 - Rejected pre-sliced oversampling FIR windows

- Candidate file: `src/core/oversampling.rs`
- Hypothesis: slicing each decimator history window once would let optimized
  compilation eliminate repeated FIR-load bounds calculations.
- Output: every candidate checksum matched the frozen P0010 executable.
- Result: the candidate regressed 2x/3x/4x one-oscillator workloads by
  1.09%/0.66%/2.10%, and regressed the dense 8x8 workload by 2.32%.
- Finding: the original fixed-array addressing optimized better than the
  borrowed window slices; no unsafe load experiment is justified by a hotspot
  that represented only 1.43% of the dense profile.
- Decision: rejected and fully reverted from production.

### R0004 - Rejected multiplied disabled-jitter clock advance

- Candidate file: `src/voices/voice.rs`
- Hypothesis: advance the free-running disabled-jitter clock once per block
  with `rate * samples` instead of preserving the repeated per-sample floating
  point additions.
- CPU result: not accepted for timing because the sound-equivalence gate
  failed first.
- Output result after disabled jitter became audible: the one-, three-, and
  eight-oscillator comparisons diverged by -29.409, -28.487, and -30.603 dB
  RMS respectively, with peak errors up to 0.587441072.
- Finding: algebraically equivalent clock arithmetic is not trajectory
  equivalent once rounded state feeds the jitter generator.
- Decision: rejected immediately and replaced with exact repeated additions.

### P0011 - Remove redundant wrap checks from disabled jitter blocks

- Files: `src/voices/voice.rs`
- Profile: after P0010, disabled-jitter bookkeeping still consumed 19.62% of
  dense samples and `wrap_swarm_clock` consumed another 14.87%.
- Change: advance the countdown with exact block arithmetic. Preserve the
  original ordered floating-point clock additions, but test the wrap boundary
  once after the block and replay the original wrapped loop only on the rare
  block that crosses 4,096 cycles.
- Realtime impact: bounded stack-free oscillator-local arithmetic; no
  allocation, lock, I/O, syscall, approximation, or group-level cache.
- Frozen P0010 lab SHA-256:
  `f386df61aeea74f468e6ef68bc32791d9cab0d3e47496195ecf1201f05437f5f`
- Candidate lab SHA-256:
  `ecbaa961315d31b2ea51bcb62b706f9300d11c2ce235c55e813bb93f3c3d49c5`

| Oscillators | Unison | Polyphony | Before ns/frame | After ns/frame | Time reduction | Checksum |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 8 | 8 | 145.058 | 122.450 | 15.59% | exact |
| 3 | 8 | 8 | 296.078 | 221.735 | 25.11% | exact |
| 8 | 8 | 8 | 682.078 | 480.631 | 29.53% | exact |

Controls and validation:

- The 32-oscillator, one-unison-lane scaling path stayed neutral at
  326.259 to 327.946 ns/frame; the 0.52% difference is measurement noise.
- The unchanged active-jitter path stayed neutral at 5,645.241 to 5,651.116
  ns/frame on the confirmation run.
- M0006 disabled-to-full-jitter comparison was bit-exact for one, three, and
  eight oscillators across 640,000 measured frames each.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0005 - Rejected const-specialized AVX2 spline mode

- Candidate file: `src/oscillators/va/backend.rs`
- Hypothesis: monomorphize the runtime-guarded AVX2 saw kernel and its BLEP
  helpers on optimized versus exact spline mode, removing repeated mode
  branches from event processing.
- Unsafe audit: the existing safe runtime AVX2/FMA feature guard, slice-length
  invariant, intrinsic load/store bounds, and arithmetic sequence were
  unchanged. No unsafe operation or caller obligation was added.
- Output: candidate and frozen benchmark checksums were exact; the existing
  scalar-versus-block residual bounds were unchanged.

| Oscillators | Unison | Polyphony | Before ns/frame | Candidate ns/frame | Regression |
|---:|---:|---:|---:|---:|---:|
| 1 | 8 | 8 | 124.017 | 126.748 | 2.20% |
| 3 | 8 | 8 | 223.234 | 238.816 | 6.98% |
| 8 | 8 | 8 | 485.575 | 508.148 | 4.65% |

- Finding: removing the invariant branch did not offset the instruction-cache
  and code-layout cost of duplicating the large AVX2 polynomial kernel.
- Decision: rejected and fully reverted from production.

### P0012 - Keep rare active-jitter target refreshes off the hot path

- Files: `src/voices/voice.rs`
- Hypothesis: full-noise structural jitter calls the small state-advance path
  every sample, while target generation runs only at the bounded control-rate
  refresh. Keeping that refresh body out of line improves hot instruction
  layout without changing arithmetic.
- Change: mark `prepare_structural_jitter_target` cold and never-inline. The
  function body, call condition, floating-point order, deterministic hash, and
  state writes are unchanged.
- Realtime impact: no new work or state; no allocation, lock, I/O, syscall,
  approximation, or group-level cache.
- Frozen P0011 lab SHA-256:
  `ecbaa961315d31b2ea51bcb62b706f9300d11c2ce235c55e813bb93f3c3d49c5`
- Candidate lab SHA-256:
  `ab7453123a36b2ee2258629c3338b20642a77a40787809d0a287bfe16c09428a`

Active-noise-jitter confirmation at eight unison lanes and eight-note
polyphony:

| Oscillators | Order | Before ns/frame | After ns/frame | Time reduction | Checksum |
|---:|---|---:|---:|---:|---:|
| 8 | frozen then candidate | 5,647.685 | 5,543.656 | 1.84% | exact |
| 8 | candidate then frozen | 5,648.406 | 5,552.226 | 1.70% | exact |

Controls and validation:

- The one-oscillator active-jitter result changed direction across run order
  (4.67% faster, then 0.55% slower) and is treated as neutral.
- Disabled jitter stayed neutral at 476.026 to 474.548 ns/frame.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted as a small repeatable dense active-jitter improvement.

### R0006 - Rejected sparse producer copy for pooled oscillator jobs

- Candidate file: `src/voices/internal_rt_pool.rs`
- Hypothesis: copy only active oscillator settings into each realtime-pool job
  instead of copying all 32 current, target, and configuration slots.
- Output: all pooled benchmark checksums matched exactly.
- Initial active-copy result: one oscillator improved 8.86% and then 6.00%
  in reverse order; three oscillators ranged from neutral to 3.26% faster;
  eight oscillators regressed 1.20% and 1.90%.
- Hybrid experiment: active-copy through three oscillators and original bulk
  copy above that crossover. A confirmation moved one oscillator 1.30% slower,
  three 5.49% faster, and the unchanged eight-oscillator bulk path 3.18%
  faster, exposing pool scheduling and code-layout noise larger than the
  producer-copy effect.
- Finding: workers subsequently copy the full fixed render-settings structure
  into local state again. Optimizing only the producer snapshot adds branching
  and stale-field invariants without removing the dominant fixed copies.
- Decision: rejected and fully reverted; pursue a compact active render-state
  representation rather than a partial copy helper.

### P0013 - Skip zero-weight saw work for pure triangle oscillators

- Files: `src/oscillators/va/render.rs`
- Hypothesis: the packed constant-shape triangle branch computes a complete
  saw waveform and spline BLEP correction even when the triangle-to-saw blend
  is exactly zero.
- Change: when the exact blend scalar is `0.0`, return the already-computed
  spline triangle directly. Nonzero morph positions retain the original saw,
  BLEP, gain, and fused blend arithmetic unchanged.
- Realtime impact: one invariant branch outside any approximation boundary;
  no allocation, lock, I/O, syscall, new state, or group-level cache.
- Frozen P0012 lab SHA-256:
  `ab7453123a36b2ee2258629c3338b20642a77a40787809d0a287bfe16c09428a`
- Candidate lab SHA-256:
  `9a2bfc9c4183ed2742ce4056c6ccbed7afef17e77494634c5e0b98eccb9eea85`

Pure-triangle confirmation at eight unison lanes and eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time reduction | Checksum |
|---:|---:|---:|---:|---:|
| 1 | 195.683 | 173.870 | 11.15% | exact |
| 3 | 443.927 | 389.327 | 12.30% | exact |
| 8 | 1,074.507 | 893.761 | 16.82% | exact |

Validation:

- The long endpoint confirmation ran candidate before frozen and reproduced
  both gains.
- Scalar-versus-block comparison remained within the existing packed SIMD
  bounds: worst peak 1.907e-6 and worst RMS -135.465 dB across one, three,
  and eight oscillators.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### M0007 - Post-jitter oscillator-kernel profile refresh

- Production DSP changed: no.
- Workload: serial, CPU-4-pinned, eight structural oscillators, eight unison
  lanes, eight-note polyphony, Spline Optimized 2x, full settled blocks.
- Dense saw: 56.28% of sampled cycles are now inside the AVX2/FMA saw and
  spline-BLEP kernel. Disabled-jitter bookkeeping no longer appears as a top
  hotspot.
- Pure triangle: 43.63% of sampled cycles are inside
  `spline_triangle8_precomputed`; the removed zero-weight saw work no longer
  appears.
- Decision: accepted as the next oscillator-local profiling baseline.

### R0007 - Rejected exact-zero shortcut in per-sample morph helpers

- Candidate file: `src/oscillators/va/render.rs`
- Hypothesis: extend P0013's pure-triangle shortcut to the f32x8 and f32x4
  segment-precomputed helpers used by dynamic shape blocks.
- Output: glide quality metrics and all deterministic checksums were exact.
- Dynamic-glide control: task time moved 335.08 to 327.78 ms, but retired
  instructions were unchanged; the cycle movement was not attributable.
- Continuous morph, candidate then frozen: 10,892.678 versus 10,802.219
  ns/frame, a 0.84% regression.
- Continuous morph, frozen then candidate: 10,785.016 versus 10,866.945
  ns/frame, a 0.76% regression.
- Finding: the per-sample branch is paid throughout the triangle-to-saw morph
  segment, while exact-zero occurs too rarely to recover that cost.
- Decision: rejected and fully reverted; retain only P0013's block-invariant
  settled shortcut.

### P0014 - Queue VA-table edits across active crossfades

- Files: `src/lib.rs`
- Defect: retargeting a VA table during its 4 ms crossfade replaced the source
  endpoint with the old target and reset progress, creating a discontinuous
  jump to that target on the next sample.
- Change: keep an active fade immutable, retain only the newest pending table,
  and begin the pending fade from the exact completed endpoint. Inaudible
  changes still apply immediately and clear pending state.
- Realtime impact: fixed-capacity state only; no allocation, lock, I/O, or
  syscall. Settled `advance` now exits before its previous division, add, and
  clamp.
- Frozen process lab SHA-256:
  `c3197009864c8c165c24c3812198c91288bbc867f770f6e2d4aeeb763bbbac11`
- Candidate process lab SHA-256:
  `d96b31dc3b6a6a6ab9b2618098880251c138c24308fd0d11b45f6a5706ba4f40`

Validation:

- A full-scale three-endpoint retarget probe at 48 kHz reduced the worst
  retarget step from 1.979166667 to the normal fade increment 0.010416667,
  a 99.47% reduction.
- The 64-frame unchanged-callback control improved from 85.281 to 84.912
  ns/frame; no CPU cost is attributed to the sound fix.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0015 - Pack active oscillator settings for render snapshots

- Files: `src/voices/voice.rs`, `src/voices/internal_rt_pool.rs`
- Hypothesis: every pooled job and worker voice still copies settings for all
  32 structural slots, so fixed snapshot overhead remains even when a preset
  contains only one or three oscillators.
- Change: keep the 32-slot configuration history on the owning synth, but pack
  render entries into a sorted initialized prefix containing only active and
  fading oscillators. Pool publication and per-voice transition copies now
  copy that prefix while oscillator state continues to use stable slot IDs.
- Realtime impact: fixed-capacity storage only; no allocation, lock, I/O, or
  syscall. Copy traffic scales with active render entries instead of the
  maximum bank size.
- Frozen P0014 generator-lab SHA-256:
  `9a2bfc9c4183ed2742ce4056c6ccbed7afef17e77494634c5e0b98eccb9eea85`
- Candidate generator-lab SHA-256:
  `10ddc297afb118c079ded518cfc4fd62da4f3d23f143fc272aa2ebd00a72b8d4`

Pooled 2x saw rendering, eight-note polyphony, ABBA mean of process medians:

| Oscillators | Unison lanes | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|---:|
| 1 | 1 | 239.108 | 203.760 | 14.78% |
| 1 | 8 | 412.668 | 385.391 | 6.61% |
| 3 | 1 | 312.922 | 275.095 | 12.09% |
| 3 | 8 | 874.701 | 810.957 | 7.29% |
| 8 | 1 | 505.980 | 443.271 | 12.39% |
| 8 | 8 | 1,975.585 | 1,923.715 | 2.63% |

Validation:

- Scalar-versus-block checks were unchanged for one, three, and eight
  oscillators across plain saw, active jitter, custom curves, and harmonic
  warp. Candidate checksums and error bounds matched the frozen binary.
- Structural trigger medians were neutral from one through 32 oscillators;
  the largest measured movement was 0.77%.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted. The biggest gains land at sparse oscillator counts, as
  required by the oscillator-level scaling contract; dense 8x8 rendering is
  dominated by waveform math and therefore moves less.
