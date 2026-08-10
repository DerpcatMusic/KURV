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

### R0008 - Rejected block-level no-BLEP-event proof gate

- Candidate file: `src/oscillators/va/backend.rs`
- Hypothesis: when every AVX2 lane can be proven to remain outside spline-BLEP
  support and below phase wrap for a complete block, skip per-sample event
  masks, reciprocal setup, and residual branches while preserving the exact
  phase additions and output FMAs.
- Unsafe audit: the existing runtime AVX2/FMA feature guard, eight-lane slice
  invariant, intrinsic bounds, phase arithmetic, and fallback kernel were
  unchanged. The scalar proof was conservative under one f32 epsilon of
  accumulated rounding per frame.
- Output: frozen and candidate checksums were exact at MIDI notes 24, 69, and
  120 for one, three, and eight oscillator workloads.

Pinned serial 2x saw rendering at eight unison lanes and eight-note polyphony,
ABBA mean of process medians:

| MIDI note | Oscillators | Before ns/frame | Candidate ns/frame | Regression |
|---:|---:|---:|---:|---:|
| 24 | 1 | 109.663 | 109.909 | 0.22% |
| 24 | 8 | 363.135 | 372.801 | 2.66% |
| 69 | 1 | 119.807 | 126.541 | 5.62% |
| 69 | 3 | 224.959 | 246.518 | 9.58% |
| 69 | 8 | 474.666 | 539.773 | 13.72% |
| 120 | 1 | 145.679 | 148.132 | 1.68% |
| 120 | 8 | 692.768 | 696.673 | 0.56% |

- Finding: the eight scalar-to-f64 lane proofs and code-layout expansion cost
  more than the already-vectorized no-event branch inside the kernel.
- Decision: rejected; candidate remained isolated and was never applied to
  production.

### R0009 - Rejected lazy AVX2 saw reciprocal

- Candidate file: `src/oscillators/va/backend.rs`
- Hypothesis: defer the existing reciprocal vector until the first BLEP event
  in a block, then cache it for later events, avoiding one divide in blocks
  whose event masks stay empty.
- Output and safety: checksums were exact across MIDI notes 24, 69, and 120.
  The runtime AVX2/FMA guard, lane bounds, phase updates, polynomial, and
  output arithmetic were unchanged.

Pinned serial 2x saw rendering at eight unison lanes and eight-note polyphony,
ABBA mean of process medians:

| MIDI note | Oscillators | Before ns/frame | Candidate ns/frame | Change |
|---:|---:|---:|---:|---:|
| 24 | 1 | 104.361 | 103.434 | -0.89% |
| 24 | 8 | 359.745 | 375.502 | +4.38% |
| 69 | 1 | 117.731 | 118.606 | +0.74% |
| 69 | 3 | 218.876 | 228.135 | +4.23% |
| 69 | 8 | 471.218 | 489.825 | +3.95% |
| 120 | 1 | 146.135 | 149.626 | +2.39% |
| 120 | 8 | 684.398 | 723.182 | +5.67% |

- Finding: the mutable readiness dependency and divide inside the correction
  branch cost more than the unconditional block-level reciprocal, especially
  as oscillator count and event density rise.
- Decision: rejected; candidate remained isolated and was never applied to
  production.

### M0008 - Post-pack oscillator scaling and waveform profile

- Production DSP changed: no.
- Workload: pinned serial 2x rendering, eight unison lanes, eight-note
  polyphony, settled structural oscillator banks.

| Waveform | 1 oscillator ns/frame | 3 oscillators | 3x ratio | 8 oscillators | 8x ratio |
|---|---:|---:|---:|---:|---:|
| Triangle | 174.483 | 400.453 | 2.30x | 910.266 | 5.22x |
| Saw | 123.672 | 237.001 | 1.92x | 475.368 | 3.84x |
| Pulse | 153.240 | 337.756 | 2.20x | 825.631 | 5.39x |
| Sine | 139.865 | 274.600 | 1.96x | 625.944 | 4.48x |

- Finding: all four waveforms already scale substantially below one complete
  render cost per added oscillator because oscillator instances share packed
  SIMD kernels. Saw is the strongest current scaling path.
- Pulse profile: 74.49% of cycles were in
  `accumulate_shape8_block_constant`, dominated by shape and spline-BLEP
  arithmetic.
- Sine profile: 69.08% was in the same packed shape kernel, dominated by the
  sine polynomial and phase wrapping.
- Decision: accepted as the next waveform-specific optimization baseline.

### R0010 - Rejected partial morph snapshot copy

- Candidate file: `src/voices/internal_rt_pool.rs`
- Hypothesis: publish only the active `job_samples` prefix of each audio-rate
  shape lane instead of copying all 512 fixed-capacity samples.
- Output and RT behavior: checksums were exact; publication ordering, timeout
  fallback, fixed storage, and voice-order reduction were unchanged.
- A benchmark-only extension to `bench-morph` now selects 128, 256, or 512
  internal-sample jobs so this seam can be measured directly.
- At 256 samples, seven successful independent process medians were
  2,132.254 ns/frame before and 2,132.845 ns/frame after, a neutral 0.03%
  regression.
- The 128-sample pool startup gate produced too few successful baseline
  processes for a trustworthy comparison; the available movement was inside
  pool scheduling noise. Full 512-sample jobs copy the same bytes.
- Decision: rejected and fully reverted; the benchmark extension remains.

### P0016 - Order VA-table payload reads before generation validation

- Files: `src/oscillators/va/table.rs`
- Defect: the second acquire generation load did not order the relaxed table
  payload reads that preceded it. On a weakly ordered target, validation could
  theoretically observe the old generation while accepting payload words from
  overlapping editor and audio-thread snapshots.
- Change: place an acquire fence after every payload word has been read and
  before the final generation validation. A concurrent writer now changes the
  observed generation and the audio thread retries on a later callback.
- Realtime impact: the path still exits before the fence when the generation
  is unchanged. Storage remains fixed and lock-free; there is no allocation,
  lock, I/O, or syscall in process.
- Frozen process-lab SHA-256:
  `a21d86225031e9db0946c873d727dfff4514b7d5dc6efa1f03ed33394be3320b`
- Candidate process-lab SHA-256:
  `a21d86225031e9db0946c873d727dfff4514b7d5dc6efa1f03ed33394be3320b`
- Result: the release binary is byte-identical on this x86-64 build, where
  the acquire fence is a compiler-ordering barrier and emits no instruction.
  The table snapshot contract is now sound on weaker architectures as well.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted as a sound-quality and state-integrity fix with zero
  steady-state DSP cost.

### R0011 - Rejected fat release LTO

- Candidate file: `Cargo.toml`
- Hypothesis: replace ThinLTO with fat LTO to recover cross-module
  optimization opportunities lost during oscillator modularization.
- Build contract: both binaries used Rust 1.97.1, one codegen unit, the same
  locked dependencies, release optimization, and
  `-C target-cpu=x86-64-v3`. Only the LTO mode changed.
- Output: all workload checksums were exact.

Pinned serial 2x rendering at eight unison lanes and eight-note polyphony,
ABBA mean of process medians:

| Waveform | Oscillators | ThinLTO ns/frame | Fat LTO ns/frame | Change |
|---|---:|---:|---:|---:|
| Saw | 1 | 120.191 | 117.738 | -2.04% |
| Saw | 3 | 219.563 | 216.879 | -1.22% |
| Saw | 8 | 474.518 | 467.880 | -1.40% |
| Sine | 1 | 134.225 | 134.966 | +0.55% |
| Sine | 3 | 273.631 | 276.606 | +1.09% |
| Sine | 8 | 625.065 | 610.796 | -2.28% |
| Pulse | 1 | 154.371 | 153.482 | -0.58% |
| Pulse | 8 | 776.451 | 774.357 | -0.27% |

- Finding: code-layout gains were waveform dependent rather than a reliable
  whole-synth improvement, while release linking became materially slower.
- Decision: rejected and fully reverted; retain ThinLTO.

### P0017 - Hoist the exact pure-sine oscillator endpoint

- Files: `src/oscillators/va/render.rs`
- Hypothesis: settled shape `0.0` is always pure sine, but the packed kernel
  still prepares spline/pulse state and enters runtime waveform dispatch for
  every sample.
- Change: gate the exact `shape == 0.0` endpoint once per block and run the
  existing sine polynomial with the same phase add, wrap, and output FMAs.
  Every nonzero shape retains the existing morph and antialiasing path.
- Realtime impact: one block-invariant comparison; no allocation, lock, I/O,
  syscall, approximation, group-level cache, or altered summation.
- Frozen thin-LTO generator-lab SHA-256:
  `b8ad16d50329f21e053b6b93430f306d5ffe69ef38501f3514c9c619f58aa0ae`
- Candidate generator-lab SHA-256:
  `02c301adf91dc452ca471f35e5046083cf01d457dd9c815eb5ea54fac12da4fc`

Pinned serial 2x pure-sine rendering at eight unison lanes and eight-note
polyphony, ABBA mean of process medians:

| Oscillators | Before ns/frame | After ns/frame | Time reduction | Checksum |
|---:|---:|---:|---:|---:|
| 1 | 137.262 | 127.690 | 6.97% | exact |
| 3 | 276.376 | 249.640 | 9.67% | exact |
| 8 | 620.074 | 542.367 | 12.53% | exact |

Validation:

- Eight-oscillator saw and pulse controls moved -0.33% and +0.30%,
  respectively, inside code-layout noise.
- Scalar-versus-block residuals stayed inside the established SIMD bounds:
  worst peak 2.384e-6 and worst RMS -133.125 dB across one, three, and eight
  oscillators.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0018 - Preserve the inverse-warp derivative at zero phase step

- Files: `src/oscillators/va/warp.rs`
- Defect: the inverse pulse-edge solver divided by an epsilon-clamped phase
  step but asked the forward warp for a derivative scaled by the original
  zero or subnormal step. That collapsed Newton's derivative to the emergency
  `0.05` floor and could place a warped pulse edge incorrectly while an
  oscillator was stopped or crossing an extreme modulation boundary.
- Change: use the same epsilon-safe step for the forward warp and derivative
  normalization in the scalar, four-lane, and eight-lane solvers.
- Realtime impact: normal audio-rate steps are greater than epsilon and execute
  the same arithmetic with the same values. The change adds no branch,
  allocation, lock, I/O, syscall, cache, or group-level work.
- Frozen generator-lab SHA-256:
  `02c301adf91dc452ca471f35e5046083cf01d457dd9c815eb5ea54fac12da4fc`
- Candidate generator-lab SHA-256:
  `d102e1aeac5c179753b4f8dfd8eb0d7a0386ad3fe4ca8e656af0bc1a1d711658`
- All twelve normal-rate warp transition diagnostics were text-identical to
  P0017, including zero tail error.
- One-, three-, and eight-oscillator bend/harm block comparisons were
  text-identical to P0017; established scalar-versus-block residuals did not
  move.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted as an oscillator sound-correctness fix with zero normal
  DSP cost.

### P0019 - Hoist pure sine from the four-lane oscillator pack

- Files: `src/oscillators/va/render.rs`
- Profile: in a three-oscillator, one-unison-lane, eight-note workload,
  `sample_shape4_at` consumed 30.83% of sampled cycles after P0017. The exact
  sine endpoint still entered the generic waveform selector in the four-lane
  kernel even though the eight-lane kernel already had a direct path.
- Change: gate settled `shape == 0.0` once per four-lane block and execute the
  existing SIMD sine polynomial with the same phase advance, wrapping,
  multiply, and `add4_to8` accumulation order.
- Realtime impact: one block-invariant comparison; no allocation, lock, I/O,
  syscall, approximation, group cache, or changed reduction order.
- Frozen P0018 generator-lab SHA-256:
  `d102e1aeac5c179753b4f8dfd8eb0d7a0386ad3fe4ca8e656af0bc1a1d711658`
- Candidate generator-lab SHA-256:
  `c8aec6ccceb54b6b83c636822dddc466853b3d10ecc4fa6f59f51d99d948f69a`

Pinned serial 2x pure-sine rendering with one unison lane and eight-note
polyphony, ABBA process-median means:

| Oscillators | Before ns/frame | After ns/frame | Time reduction | Checksum |
|---:|---:|---:|---:|---:|
| 3 | 161.001 | 131.993 | 18.02% | exact |
| 4 | 165.922 | 132.556 | 20.11% | exact |

Validation:

- Three- and four-oscillator scalar-versus-block diagnostics were
  text-identical to P0018. Their RMS residuals remained -138.481 dB and
  -139.590 dB, respectively.
- Adjacent saw controls moved +0.60% and -0.34%; pulse controls moved -2.96%
  and -0.21%. Checksums were exact and no adjacent workload regressed outside
  the established process-to-process noise band.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0012 - Rejected manual pure-pulse saw lifetime split

- Candidate file: `src/oscillators/va/render.rs`
- Hypothesis: move the unused saw calculation behind the non-pulse branch in
  the four- and eight-lane spline segment kernels.
- Output and RT behavior: checksums were exact; BLEP, phase, accumulation, and
  fixed-storage behavior were unchanged.
- Pinned serial 2x pulse rendering, ABBA process-median means versus P0019:

| Oscillators | Unison lanes | Before ns/frame | Candidate ns/frame | Change |
|---:|---:|---:|---:|---:|
| 1 | 8 | 159.381 | 159.586 | +0.13% |
| 3 | 8 | 339.936 | 341.128 | +0.35% |
| 8 | 8 | 781.370 | 784.594 | +0.41% |
| 3 | 1 | 199.113 | 200.684 | +0.79% |
| 4 | 1 | 205.994 | 207.671 | +0.81% |

- Finding: every representative packed workload regressed. The manual source
  split worsened generated code or layout even though it appeared to remove
  arithmetic.
- Decision: rejected and fully reverted.

### P0020 - Route settled pooled banks through packed oscillator kernels

- Files: `src/voices/internal_rt_pool.rs`, `src/voices/voice.rs`
- Profile before: pooled three-oscillator saw rendering spent 31.30% of all
  sampled cycles in scalar `sample_shape_normalized` on helpers and 24.66% in
  scalar `VaVoice::render_oscillator_bank`. Main-thread participation added
  another 18.80% in the same scalar functions.
- Defect: every pool participant copied a mutable render-settings cursor and
  used the per-frame scalar bank renderer even when the published bank was
  settled and immutable. The serial path already used packed four- and
  eight-lane oscillator kernels for the same state.
- Change: settled pool jobs borrow the immutable published settings and call
  the existing static-bank block renderer. Transitioning jobs retain a private
  mutable cursor across chunks, including the original inactive transition
  tail behavior. Morph shapes remain fixed-size per-chunk snapshots.
- State contract: a helper or the participating audio thread advances only its
  claimed shadow voice. Successful jobs commit each shadow once, then the audio
  thread alone advances canonical bank transition state. Timeout returns
  before commit/canonical advancement, so serial fallback still advances live
  state exactly once.
- Realtime impact: removes sparse settings copies and scalar waveform work;
  adds no allocation, lock, I/O, syscall, group cache, or unbounded loop.
- Frozen P0019 generator-lab SHA-256:
  `c8aec6ccceb54b6b83c636822dddc466853b3d10ecc4fa6f59f51d99d948f69a`
- Candidate generator-lab SHA-256:
  `491fea014f12f3f12c9da0ee8417c660c7831fdc5e198489deb8125238805966`

Unpinned pooled 2x rendering at eight unison lanes and eight-note polyphony,
ABBA process-median means with zero deadline fallbacks:

| Waveform | Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|---:|
| Saw | 1 | 495.407 | 150.057 | 69.71% |
| Saw | 3 | 1,110.315 | 222.386 | 79.97% |
| Saw | 8 | 2,608.606 | 412.372 | 84.19% |
| Sine | 3 | 1,584.617 | 186.858 | 88.21% |
| Sine | 8 | 3,787.406 | 299.756 | 92.09% |
| Pulse | 3 | 1,145.095 | 209.201 | 81.73% |
| Pulse | 8 | 2,542.748 | 374.978 | 85.25% |

Oscillator-level scaling for pooled saw changed from 1.00x / 2.24x / 5.27x
at one/three/eight oscillators to 1.00x / 1.48x / 2.75x. Three oscillators
therefore add only 48% over the one-oscillator wall time in this representative
group workload.

Validation:

- Candidate pooled checksums converged to the established serial packed
  reference; the largest serial/candidate checksum delta was 1.22e-4 over the
  full benchmark accumulation. The scalar-to-packed waveform residual remains
  bounded by P0007: pulse is worst at 2.918e-5 peak and -121.31 dB RMS.
- Active-jitter controls retained the scalar fallback and were checksum-exact
  before/after at three and eight oscillators.
- The post-profile no longer contains scalar waveform or scalar bank rendering
  among its sampled hot functions. Helpers are dominated by
  `spline_triangle8_precomputed` and `accumulate_shape8_block_constant`.
- Existing realtime-pool success, unsupported/release, timeout/recovery, and
  1x-4x suites: 5 passed, 0 failed.
- Targeted voice suite: 3 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### M0009 - Constant-warp code-generation audit

- Production DSP changed: no.
- Binary: Rust 1.97.1 profiling build, ThinLTO, x86-64-v3.
- Finding: every x4/x8 constant-step specialization still calls
  `warp_phase4` or `warp_phase8` from inside its sample loop. Disassembly of
  those callees shows mode dispatch, amount clamps, epsilon check, depth
  arithmetic, and one `vdivps` executing per internal sample.
- Existing pulse-edge/Newton solving is already outside the frame loop.
- Decision: a bounded constant-step warp specialization experiment is
  justified. Dynamic-step paths and pulse-edge inversion must remain
  unchanged.

### M0010 - Mixed oscillator-bank benchmark

- Change: added `bench-bank-mixed` and `bench-bank-mixed-pool` to
  `examples/generator_lab.rs`.
- Production DSP changed: no.
- Workload: one-unison-lane saw oscillators in stable slot order with one
  harmonic-warp oscillator at the middle split. This forces compatible plain
  runs on both sides of an incompatible oscillator.
- Purpose: prove that one custom/warped oscillator does not force every other
  oscillator in a group back to scalar rendering.
- P0020 pinned serial baseline at eight-note polyphony:

| Oscillators | Baseline ns/frame |
|---:|---:|
| 4 | 668.803 |
| 8 | 1,026.689 |
| 16 | 1,748.151 |

- Decision: accepted as mixed-bank scaling infrastructure.

### P0021 - Pack compatible runs inside mixed oscillator banks

- Files: `src/voices/voice.rs`, `examples/generator_lab.rs`
- Defect: single-lane instance packing was all-or-nothing. One warped, custom,
  differently shaped, or multi-lane oscillator forced every otherwise
  compatible oscillator in the group through the scalar block renderer.
- Change: walk settled entries in stable slot order, pack each contiguous
  compatible run of at least three oscillators, and render incompatible or
  shorter runs scalar in place. The mixed scalar/block diagnostic now exposes
  this path directly.
- Realtime impact: bounded scans over the existing initialized prefix and the
  existing fixed-size gather/scatter buffers only. There is no allocation,
  lock, I/O, syscall, global regrouping, or oscillator reordering.
- Frozen M0010 generator-lab SHA-256:
  `b63dfa84b74ad14ae12d506b1c03090de39b539b89b3ef035d74949d5fbeb804`
- Candidate generator-lab SHA-256:
  `02c2852d6b83dc3de06ecf74c40f8431ba147571ae5fc025ea420a45b57c7a4f`

Pinned serial 2x mixed-saw rendering with one unison lane and eight-note
polyphony, ABBA process-median means:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 4 | 683.322 | 681.146 | 0.32% |
| 8 | 1,028.016 | 631.958 | 38.53% |
| 16 | 1,776.689 | 610.139 | 65.66% |

The four-oscillator workload is an intentional no-pack control: its middle
incompatible oscillator leaves compatible runs of only two and one. Eight and
sixteen oscillators exercise packed runs on both sides of the incompatible
entry.

Unpinned pooled controls with zero deadline fallbacks:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 8 | 494.306 | 327.209 | 33.80% |
| 16 | 747.635 | 331.534 | 55.66% |

Validation:

- Four-oscillator mixed output stayed bit-exact because it remained scalar.
- Eight oscillators peaked at 5.960e-7 scalar/block error with -139.156 dB
  RMS; sixteen peaked at 9.537e-7 with -139.953 dB RMS.
- Fully compatible three/eight/32-oscillator controls moved +0.49%, +0.83%,
  and -1.04%, respectively, with exact checksums.
- Targeted voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0022 - Skip canonical replay for settled pooled oscillator banks

- File: `src/voices/internal_rt_pool.rs`
- Defect: after every successful pooled render, the audio thread replayed one
  `ActiveOscillatorRenderSet::advance` call per internal sample even when the
  published oscillator bank was already settled. Settled jobs cannot change
  canonical bank transition state.
- Change: snapshot the published bank's transition status and replay canonical
  advancement only for transition jobs. Timeout still returns before replay,
  and serial fallback retains its existing once-only advancement.
- Realtime impact: removes a bounded per-sample loop from every successful
  settled pooled job; adds one boolean snapshot and no allocation, lock, I/O,
  syscall, group cache, or oscillator-dependent work.
- Frozen P0021 generator-lab SHA-256:
  `5e63dfdff2db0995ddc7dab459159cc559814e6bf704095cee13c2980a43e055`
- Candidate generator-lab SHA-256:
  `e6925a5c2534061d03eabbc9bb723fd4f719979e0ae805f4e79b1eba4f9c1892`

Unpinned pooled 2x saw rendering at eight unison lanes and eight-note
polyphony, ABBA process-median means with zero deadline fallbacks:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 131.970 | 123.783 | 6.20% |
| 3 | 173.186 | 152.928 | 11.70% |
| 8 | 256.509 | 246.081 | 4.07% |

Validation:

- Accumulated output checksums were bit-identical before/after for all three
  measured oscillator counts.
- Existing voice and realtime-pool success, transition, unsupported/release,
  timeout/recovery, and 1x-4x suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0023 - Use the packed sine polynomial in scalar oscillator endpoints

- Files: `src/oscillators/va/antialias.rs`,
  `src/oscillators/va/render.rs`
- Profile before: one settled scalar sine oscillator spent 40.55% of sampled
  cycles in the `f64` cosine implementation. One- and two-oscillator banks do
  not enter the three-lane instance-packing threshold.
- Change: add a scalar form of the existing folded high-accuracy `f32` sine
  polynomial and use it for the scalar sine waveform arm. Three-or-more packed
  oscillators continue using the coefficient-identical x4/x8 endpoint.
- Realtime impact: removes a transcendental call and `f64` sine arithmetic;
  adds no allocation, lock, I/O, syscall, state, or oscillator grouping.
- Frozen P0022 generator-lab SHA-256:
  `e6925a5c2534061d03eabbc9bb723fd4f719979e0ae805f4e79b1eba4f9c1892`
- Candidate generator-lab SHA-256:
  `f3a07ce37d6a9e6d684ae3121ecbf9cd89b1d8839d62f6a43df9449d14f67c61`

Pinned serial 2x pure-sine rendering with one unison lane and eight-note
polyphony, ABBA process-median means:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 292.166 | 216.845 | 25.78% |
| 2 | 445.408 | 325.550 | 26.91% |

Non-sine pinned controls kept their original endpoint arithmetic and output:
one saw oscillator improved 203.662 to 198.378 ns/frame (2.59%), while one
triangle oscillator measured 222.994 to 224.872 ns/frame (+0.84%, neutral).

Validation:

- A coherent 131,072-sample old/new sine render measured 3.576e-7 peak error
  and -141.012 dB relative RMS error. Maximum non-fundamental spur remained
  below -165.95 dBc.
- Scalar/block comparisons were exact for one and two sine oscillators. Three
  packed sine oscillators measured 2.384e-7 peak and -146.034 dB RMS error.
- Existing oscillator endpoint checks: 2 passed, 0 failed.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### M0011 - All-warp oscillator-bank benchmark

- Change: added `bench-bank-warp` and `bench-bank-warp-pool` to
  `examples/generator_lab.rs`.
- Production DSP changed: no.
- Workload: every active structural oscillator uses harmonic phase warp at
  75%, exposing the constant-step x4/x8 warp kernels without dilution by plain
  oscillators.
- Purpose: measure whether preparing mode, clamp, depth, and division once per
  block removes meaningful oscillator-local work.
- Decision: accepted as warp-specialization infrastructure.

### P0024 - Prepare constant-step SIMD warp kernels once per block

- Files: `src/oscillators/va/warp.rs`, `src/oscillators/va/render.rs`
- Defect: constant-step x4/x8 kernels repeated mode dispatch, amount clamp,
  epsilon handling, safe-step division, and depth calculation for every sample.
- Change: prepare fixed x4/x8 warp state once, dispatch the mode once per
  block, and monomorphize the inner loop by warp mode. Dynamic-step paths and
  pulse-edge/Newton inversion remain unchanged.
- Realtime impact: removes repeated vector divisions and branches from warped
  oscillator sample loops; adds fixed stack/register state only, with no
  allocation, lock, I/O, syscall, cache, or cross-oscillator dependency.
- Frozen M0011 generator-lab SHA-256:
  `f268ccbab8cb396cbf25d9abe623607205690864e2cbd57b38ca820b64758e9b`
- Candidate generator-lab SHA-256:
  `377f5d8e82fd73977fa9628e36249f8356e3e67a2b3660f69bfd92888251f977`

Pinned serial 2x harmonic-warped saw banks at eight unison lanes and
eight-note polyphony, ABBA process-median means:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 376.945 | 342.805 | 9.06% |
| 3 | 891.249 | 768.416 | 13.78% |
| 8 | 2,152.759 | 1,836.559 | 14.69% |

Four-lane controls proved the x4 specialization independently: one oscillator
improved 372.606 to 336.204 ns/frame (9.77%), and three improved 852.607 to
718.261 ns/frame (15.76%). An unpinned pooled three-oscillator control with
four chunks and zero deadline fallbacks improved 441.834 to 389.626 ns/frame
(11.82%).

Validation:

- Harmonic benchmark checksums were bit-identical before/after at every
  measured oscillator and lane count.
- PWM, phase-bend, and harmonic scalar/block diagnostics for saw and pulse were
  text-identical before/after. The existing worst packed residual remained the
  PWM pulse at 1.562e-5 peak and -121.947 dB RMS.
- All twelve live warp-mode transitions were text-identical before/after and
  converged to zero tail target error.
- Profiling disassembly found all seven x4/x8 vector divides before the
  specialized sample-loop backedges; none remained inside a sample loop.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0013 - Rejected per-sample fixed-endpoint bypass

- Experiment: bypass generic shape decoding for exact sine, triangle, saw, and
  pulse values inside `sample_shape_normalized`.
- Fixed endpoints: one-oscillator changes ranged from neutral to 2.48% faster;
  two exact saw oscillators improved 286.249 to 266.993 ns/frame (6.72%).
- Regression: one oscillator at the continuous saw-to-pulse midpoint regressed
  254.518 to 304.820 ns/frame (19.76%) because all endpoint guards remained in
  the per-sample morph loop.
- Output: checksums stayed bit-identical in every endpoint and morph control.
- Decision: rejected and fully reverted. Fixed endpoints must be selected
  outside the sample loop or recovered through oscillator-instance packing.

### P0025 - Skip warped pulse-edge solving at the exact saw endpoint

- File: `src/oscillators/va/render.rs`
- Defect: scalar, x4, and x8 warped paths solved the inverse pulse edge for
  every `shape >= 2.0`, including exact saw at `2.0`. Shape segmentation maps
  exact saw to `(Saw, 0.0)` and returns before the pulse sampler can consume
  that edge.
- Change: prepare the pulse edge only for `shape > 2.0`. Saw-to-pulse morphs
  and pulse retain the original edge/Newton path.
- Realtime impact: removes fixed Newton/division setup from exact warped saw
  blocks; adds no state, allocation, lock, I/O, syscall, or approximation.
- Frozen P0024 generator-lab SHA-256:
  `377f5d8e82fd73977fa9628e36249f8356e3e67a2b3660f69bfd92888251f977`
- Candidate generator-lab SHA-256:
  `6b705a10027d15c2247a8e26dfe5ac08f8d1fbad0ff348e2bfde98a37b29167c`

Pinned serial 2x harmonic-warped saw banks at eight unison lanes and
eight-note polyphony, ABBA process-median means:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 351.491 | 331.745 | 5.62% |
| 3 | 809.808 | 718.571 | 11.27% |
| 8 | 1,872.467 | 1,656.320 | 11.54% |

Three-process pulse controls were neutral at 980.544 before and 976.612
ns/frame after (-0.40%). Saw-to-pulse midpoint controls measured +1.66%,
inside the observed process variance and with identical output.

Validation:

- PWM, phase-bend, and harmonic scalar/block diagnostics for exact saw,
  saw-to-pulse midpoint, and pulse were text-identical before/after.
- All twelve live warp-mode transition diagnostics were text-identical.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0014 - Rejected generic two-oscillator run threshold

- Experiment: lower the existing compatible-run threshold from three
  oscillators to two so the x4 instance kernel also handles pairs.
- Local result: two exact-saw oscillators improved by roughly 40%.
- Regression: the shared threshold changed LLVM's inlining decision for the
  existing x4 path. A mixed eight-oscillator bank first regressed 5.6%; an
  attempted isolated caller without forced x4 inlining regressed about 13.4%.
  `perf stat` measured roughly 9% more retired instructions, and a symbolized
  profile showed the previously inlined x4 kernel emitted as a separate hot
  call.
- Decision: rejected. Pair rendering must be dispatched without changing the
  established three-or-more run detector, and the x4 kernel must retain its
  proven inlining behavior.

### P0026 - Pack an exact two-oscillator bank without perturbing larger banks

- File: `src/voices/voice.rs`
- Defect: a settled bank containing two compatible one-lane oscillators
  rendered both instances independently even though the existing x4 kernel
  had two spare SIMD lanes.
- Change: dispatch an exact two-entry compatible bank before the established
  settled-bank renderer, fill the inactive x4 lanes with zero steps and gains,
  and commit state only for the two active entries. The three-or-more run
  detector remains unchanged. The x4 kernel is explicitly inlined so its new
  second call site cannot de-inline the existing larger-bank hot path.
- Realtime impact: one bounded two-entry compatibility check and fixed stack
  state only; no allocation, lock, I/O, syscall, regrouping, or oscillator
  reordering.
- Frozen P0025 generator-lab SHA-256:
  `6b705a10027d15c2247a8e26dfe5ac08f8d1fbad0ff348e2bfde98a37b29167c`
- Candidate generator-lab SHA-256:
  `cd03175b5f2d15e1567af1b8c64414eec0fcd2c56d536c8f67eb4c779113fe96`

Pinned serial two-oscillator banks at one unison lane and eight-note
polyphony:

| Shape | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|
| Sine | 303.477 | 181.234 | 40.28% |
| Triangle | 315.500 | 221.956 | 29.65% |
| Saw | 277.737 | 201.707 | 27.37% |
| Pulse | 321.875 | 246.383 | 23.45% |
| Sine/triangle midpoint | 405.456 | 322.332 | 20.50% |
| Triangle/saw midpoint | 402.140 | 276.153 | 31.33% |
| Saw/pulse midpoint | 387.178 | 266.789 | 31.09% |

A longer pinned saw control measured 277.733 before and 193.026 ns/frame
after (30.50%). The existing three-oscillator bank measured +0.78%, inside
observed process variance, and the mixed eight-oscillator bank improved 0.53%.
The unpinned pooled two-saw control, with all three workers participating and
zero deadline fallbacks, improved 162.057 to 121.770 ns/frame (24.86%).

Validation:

- Sine remained bit-exact. Peak scalar/block residuals were 2.384e-7 for
  triangle and saw, 1.311e-5 for pulse, and 6.527e-6 for the worst morph.
  RMS residuals ranged from -118.705 to -159.531 dB relative to signal.
- The three-oscillator and mixed eight-oscillator control checksums remained
  bit-identical before/after.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0027 - Improve the folded-sine polynomial at equal DSP cost

- File: `src/oscillators/va/antialias.rs`
- Defect: the scalar, x4, and x8 folded-sine kernels used an older degree-11
  coefficient set with avoidable approximation error.
- Change: replace only the six `f32` coefficients. Polynomial degree,
  folding, multiplication count, `mul_add` count, evaluation order, and
  scalar/SIMD structure are unchanged.
- Realtime impact: no new operation, state, allocation, branch, lock, I/O, or
  syscall.
- Frozen P0026 generator-lab SHA-256:
  `cd03175b5f2d15e1567af1b8c64414eec0fcd2c56d536c8f67eb4c779113fe96`
- Candidate generator-lab SHA-256:
  `3040931fc5c2cecfbd6741e88e21710ec7e4c9e7f129f0c00223a4f3cc12b21b`

Quality was measured with the production `f32::mul_add` evaluation order
against an `f64` sine reference. RMS used all 2^24 uniformly spaced phases;
peak error scanned every representable folded `f32` value from 1/16 through
1/4:

| Error metric | Before | After | Improvement |
|---|---:|---:|---:|
| Peak absolute | 1.806177e-7 | 1.522243e-7 | 15.72% |
| RMS | 3.910544e-8 | 3.127295e-8 | 20.03% |
| RMS error | -148.155 dBFS | -150.097 dBFS | 1.941 dB |

Pinned long-run controls were neutral: one scalar oscillator measured +1.15%,
three packed oscillators +1.47%, and one eight-lane oscillator -0.07%, all
inside observed process variance. The three-oscillator `perf stat` control
retired 12,408,208,608 instructions before and 12,408,210,046 after, a
0.000012% difference attributable to process-level bookkeeping rather than a
new DSP operation.

Validation:

- The three-oscillator scalar/block sine residual peaked at 2.384e-7 with
  -145.976 dB RMS relative to signal.
- Existing VA render tests: 2 passed, 0 failed.
- Decision: accepted.

### P0028 - Copy only active oscillator jitter lanes into pool jobs

- File: `src/voices/voice.rs`
- Defect: pooled render-state prepare and commit copied both complete 64-lane
  jitter arrays for every active oscillator even when that oscillator rendered
  only one lane. Phase state already copied only the active prefix.
- Change: copy the active `render_voices` prefix of jitter ratios and steps,
  matching the existing oscillator-phase state contract.
- Realtime impact: no new state, allocation, branch inside a sample loop,
  lock, I/O, or syscall. For a one-lane oscillator, each transfer falls from
  512 to 8 jitter bytes. Prepare plus commit therefore avoid 1,008 bytes per
  oscillator, or 31.5 KiB for a 32-oscillator render job.
- Frozen P0027 generator-lab SHA-256:
  `3040931fc5c2cecfbd6741e88e21710ec7e4c9e7f129f0c00223a4f3cc12b21b`
- Candidate generator-lab SHA-256:
  `91513c042fffbd0575b0eb7c427109c9fbfb4f84896dad07726658150a138388`

Unpinned pooled saw controls used one unison lane, eight-note polyphony, all
three workers participating, and zero deadline fallbacks. A long
eight-oscillator run improved 137.351 to 124.428 ns/frame (9.41%). A long
32-oscillator DSP-dominated run was neutral at 210.925 before and 210.829
ns/frame after. Two-process controls independently measured 6.12% faster at
eight oscillators and 4.21% faster at 32; one- and three-oscillator controls
remained inside pooled scheduling variance.

Validation:

- Every pooled checksum was bit-identical before/after, including one, three,
  eight, and 32 oscillators.
- Existing voice and realtime-pool suites, including jitter modes: 8 passed,
  0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### M0012 - Mixed custom/canonical warped-bank benchmark

- Change: `KURV_LAB_CUSTOM=1` now gives structural-bank benchmark
  oscillators a 50% custom-wave mix. Combined with `bench-bank-warp`, this
  exercises the constant x4/x8 custom-wave renderers while their canonical
  pulse component remains audible and computationally active.
- Production DSP changed: no.
- Purpose: measure whether warped pulse-edge inversion can be prepared once
  per constant-step custom block instead of once per sample.
- Frozen M0012 generator-lab SHA-256:
  `42c412f4330604917582137faf78607900aba48f39a5b657c956d53757c995da`
- Decision: accepted as custom-warp specialization infrastructure.

### P0029 - Prepare the warped custom-morph pulse edge once per block

- Files: `src/oscillators/va/render.rs`, `examples/generator_lab.rs`
- Defect: the constant-step x4/x8 custom-wave morph renderers called the
  automatic warped-shape sampler every sample. For saw-to-pulse shapes, that
  recomputed the same inverse warped pulse edge, including its bounded Newton
  solve and divisions, on every sample.
- Change: prepare the optional x4/x8 pulse edge once before the sample loop
  and pass it to the same shape sampler. Full custom mix and shapes at or below
  exact saw still skip preparation. The comparison diagnostic gained a
  `custom-harm` variant for this exact path.
- Realtime impact: moves pure constant-step work out of the sample loop; adds
  no state, approximation, allocation, lock, I/O, or syscall.
- Frozen M0012 generator-lab SHA-256:
  `42c412f4330604917582137faf78607900aba48f39a5b657c956d53757c995da`
- Candidate generator-lab SHA-256:
  `e9c58f1990c3b02f4d28b1b7db9f74be58965d8c9e67f4749c17ba48ef525be6`

Pinned serial 50% custom / 50% saw-to-pulse morph banks with harmonic warp,
eight-note polyphony:

| Oscillators | Unison lanes | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|---:|
| 1 | 8 | 1,176.440 | 489.633 | 58.38% |
| 3 | 8 | 3,280.921 | 1,229.527 | 62.53% |
| 8 | 8 | 8,600.545 | 3,048.362 | 64.56% |
| 1 | 4 | 1,388.345 | 707.272 | 49.06% |

Exact-saw custom controls improved 1.24%, and non-custom saw-to-pulse controls
improved 1.74%, both inside observed process variance. Every before/after
benchmark checksum was bit-identical.

Validation:

- The `custom-harm` scalar/block diagnostic peaked at 2.354e-6 and -125.533
  dB RMS for one eight-lane oscillator; three oscillators peaked at 2.861e-6
  and -125.705 dB RMS.
- Existing VA render tests: 2 passed, 0 failed.
- Decision: accepted.

### R0015 - Add a second settled-bank jitter call site without restoring inlining

- Experiment: add an exact-two small-unison renderer while leaving
  `advance_settled_structural_jitter_block` to the compiler's default inlining
  decision.
- Result: the new call site caused LLVM to outline the previously inlined
  jitter advance helper. A pinned three-oscillator control slowed 3.25%;
  `perf stat` showed 4.34% more retired instructions and 14.1% more branches.
- Correction: explicitly inline this small fixed-loop helper at both settled
  render call sites. The final candidate restored the three-oscillator control
  to 572.218 before and 572.013 ns/frame after, while retired instructions fell
  0.011% from 69,151,708,670 to 69,144,283,363.
- Decision: rejected in its outlined form; compiler behavior was corrected
  before accepting P0030.

### P0030 - Pack two small-unison oscillators into one SIMD render

- File: `src/voices/voice.rs`
- Defect: two compatible oscillators with two to four unison lanes each were
  rendered as two complete oscillator passes even though their combined four
  to eight lanes fit one existing SIMD kernel.
- Change: recognize only an exact two-oscillator, settled, canonical pair with
  matching shape and pulse width; pack both oscillators' active lanes into one
  x4 or x8 constant-step render; then commit each lane back to its owning
  oscillator. Custom waves and phase warps retain their established paths.
- Realtime impact: one bounded two-entry compatibility check and fixed stack
  arrays only; no allocation, lock, I/O, syscall, oscillator reordering, or
  persistent cross-group state.
- Frozen P0029 generator-lab SHA-256:
  `e9c58f1990c3b02f4d28b1b7db9f74be58965d8c9e67f4749c17ba48ef525be6`
- Candidate generator-lab SHA-256:
  `8a1426e445645089aac2ded32d7b6d839b3b519570a49fc234fc9f595bfa4a7a`

Pinned serial exact-two banks at eight-note polyphony:

| Shape | Lanes per oscillator | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|---:|
| Sine | 2 | 505.369 | 205.739 | 59.29% |
| Saw | 2 | 431.789 | 224.631 | 47.98% |
| Pulse | 2 | 529.696 | 284.977 | 46.20% |
| Saw | 3 | 574.693 | 196.631 | 65.78% |
| Saw | 4 | 275.446 | 200.641 | 27.16% |

A long alternating-process single-oscillator/two-lane control improved 1.37%
from 286.822 to 282.895 ns/frame. The unrelated three-oscillator/two-lane
control was neutral at 572.218 before and 572.013 ns/frame after; its checksum
remained bit-identical.

Excluded-path controls also remained neutral: two eight-lane oscillators
improved 2.60% with bit-identical output. An alternating custom-wave control
varied by +1.63% wall time, while `perf stat` measured 0.64% fewer cycles and
only 0.07% more instructions, well inside the run-to-run spread of that path.

Validation:

- Peak scalar/block residuals were 2.384e-7 for sine and saw, 4.768e-7 for the
  four-lane saw case, and 1.335e-5 for pulse. RMS residuals ranged from -121.631
  to -144.093 dB relative to signal.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### M0013 - Current oscillator-level scaling contract

- Production DSP changed: no.
- Workload: pinned serial 2x saw rendering, one unison lane per oscillator,
  eight-note polyphony, and one shared oscillator bank.

| Oscillators | Median ns/frame | Ratio to one oscillator |
|---:|---:|---:|
| 1 | 210.432 | 1.00x |
| 3 | 185.081 | 0.88x |
| 8 | 190.845 | 0.91x |
| 32 | 388.420 | 1.85x |

- Finding: three oscillators cost less than one because three compatible
  instances fill an existing x4 SIMD pack; eight remain under the
  single-oscillator cost. The remaining inversion is the one-oscillator
  scalar fallback, which still performs waveform synthesis per sample.
- Decision: accepted as the next oscillator-local optimization baseline.

### P0031 - Stop rewriting proven-neutral jitter lanes

- File: `src/voices/voice.rs`
- Defect: three settled structural render helpers rewrote active one-lane
  jitter ratio and step state to `1.0` and `0.0` every block, although the
  settled-bank eligibility gate already proves those exact bit patterns.
- Change: remove the six redundant stores while preserving jitter remaining,
  clock advancement, refresh behavior, and all non-settled paths.
- Realtime impact: strictly less bounded oscillator-local state traffic; no
  new state, branch, allocation, lock, I/O, syscall, or group-level cache.
- Frozen P0030 generator-lab SHA-256:
  `8a1426e445645089aac2ded32d7b6d839b3b519570a49fc234fc9f595bfa4a7a`
- Candidate generator-lab SHA-256:
  `bf2fab12f14801a8b1d3ea20c811c82d7cf00486f89cbfa1e5828e2ab81d022a`

A pinned alternating-process 32-oscillator saw control improved from an
average 384.243 to 359.371 ns/frame, a 6.47% time reduction, with bit-identical
output. Hardware counters independently measured 2.80% fewer cycles, 0.93%
fewer retired instructions, and fewer branches. One-, three-, and
eight-oscillator controls remained inside process variance.

Validation:

- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0032 - Share sine and cosine phase reduction in warp kernels

- File: `src/oscillators/va/warp.rs`
- Defect: scalar and SIMD Phase Bend and Harmonic kernels evaluated sine and
  cosine independently at the same phase. Scalar PWM similarly made four
  separate trigonometric calls for two phase positions.
- Change: use the existing paired sine/cosine evaluators so each phase is
  folded and reduced once. Position-only kernels remain unchanged because
  computing an unused partner would add work.
- Realtime impact: removes duplicated oscillator-local phase reduction and
  polynomial setup; adds no approximation, branch, state, allocation, lock,
  I/O, syscall, or group-level cache.
- Frozen P0031 generator-lab SHA-256:
  `bf2fab12f14801a8b1d3ea20c811c82d7cf00486f89cbfa1e5828e2ab81d022a`
- Candidate generator-lab SHA-256:
  `fb06f4bd9efc3792f0d6aa931d225a1faf11b090049dfbac6fcf7c53c60ba241`

Pinned serial warped-saw results at eight-note polyphony:

| Path | Warp | Oscillators x lanes | Before ns/frame | After ns/frame | Time reduction |
|---|---|---:|---:|---:|---:|
| Structural bank | Harmonic | 1 x 1 | 375.814 | 369.226 | 1.75% |
| Structural bank | Harmonic | 1 x 8 | 318.484 | 301.993 | 5.18% |
| Structural bank | Harmonic | 8 x 8 | 1,617.373 | 1,558.198 | 3.66% |
| Legacy control | PWM | 1 x 8 | 376.784 | 363.256 | 3.59% |
| Legacy control | Phase Bend | 1 x 8 | 333.592 | 315.303 | 5.48% |
| Legacy control | Harmonic | 1 x 8 | 319.317 | 313.245 | 1.90% |

Every benchmark checksum was bit-identical. An alternating plain-bank control
was neutral at 283.831 before and 283.676 ns/frame after.

Validation:

- Before/after scalar/SIMD diagnostics were text-identical for PWM, Phase Bend,
  and Harmonic. Peak residuals were 8.345e-7, 8.345e-7, and 9.537e-7;
  corresponding RMS errors were -137.679, -137.545, and -137.858 dB.
- Existing VA render tests: 2 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0016 - Prepared constant shape wrapper

- Experiment: prepare shape clamp, segment, blend gain, and mixed-custom SIMD
  broadcast once per constant block instead of expressing them through the
  established per-sample helpers.
- Frozen P0032 generator-lab SHA-256:
  `fb06f4bd9efc3792f0d6aa931d225a1faf11b090049dfbac6fcf7c53c60ba241`
- Candidate generator-lab SHA-256:
  `db5e6464248a37b99abd681d49cdb5c3c37a192a8294af6678ec44d2896cbbbe`

Pinned triangle/saw midpoint results varied from 1.12% faster for one 8-lane
oscillator to 0.76% slower for three and 3.19% slower for eight. Hardware
counters on the dense 8x8 case showed 0.08% more retired instructions. The
mixed-custom Harmonic target improved only 0.20%, and an exact-saw control
improved 0.89%. Checksums remained bit-identical.

- Finding: LLVM already hoists or folds nearly all of this invariant work;
  explicit preparation added 87 lines and wrapper/code-layout cost without a
  repeatable material gain.
- Decision: rejected and removed in full.

### P0033 - Reuse the AVX2 curve selector for four custom lanes

- File: `src/wave_curve.rs`
- Defect: four-lane custom-wave evaluation selected spline coefficients by
  scanning and blending all 16 segments every sample. The eight-lane AVX2/FMA
  evaluator already selects the same four coefficient planes directly, making
  four custom lanes slower than eight in representative patches.
- Change: on AVX2/FMA builds, widen four phases with four inactive zero lanes,
  reuse the established eight-lane evaluator, and return the active prefix.
  Other targets retain the existing portable four-lane selector unchanged.
- Realtime impact: replaces 16 segment comparisons and 64 coefficient blends
  per x4 evaluation with one established bounded AVX2 plane selection; no
  approximation, state, allocation, lock, I/O, syscall, or group-level cache.
- Frozen P0032 generator-lab SHA-256:
  `fb06f4bd9efc3792f0d6aa931d225a1faf11b090049dfbac6fcf7c53c60ba241`
- Candidate generator-lab SHA-256:
  `521ed2ed67a6d5e8c9fc6df51cbc12668fa1330db778a248b011f21bd71a2e3a`

Pinned serial custom-wave results at four unison lanes and eight-note
polyphony:

| Path | Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|---:|
| Pure custom saw | 1 | 488.866 | 294.246 | 39.81% |
| 50% custom Harmonic morph | 1 | 740.877 | 460.584 | 37.83% |
| 50% custom Harmonic morph | 3 | 1,888.380 | 1,129.938 | 40.16% |
| 50% custom Harmonic morph | 8 | 4,792.454 | 2,826.313 | 41.03% |

The untouched eight-lane custom control improved 0.60%. An alternating plain
four-lane control was neutral at 204.444 before and 204.556 ns/frame after.
All benchmark checksums were bit-identical.

Validation:

- The pure-custom scalar/SIMD diagnostic was text-identical before/after at
  7.153e-7 peak and -141.097 dB RMS. The established mixed custom/Harmonic
  diagnostic retained its printed peak and RMS bounds; its accumulated block
  checksum changed only 1.53e-7.
- Existing VA render tests: 2 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0017 - Zero-pad one oscillator into an x4 waveform pack

- Experiment: route an exact one-lane structural oscillator through the x4
  constant-step kernel with three inactive zero-gain lanes, so one oscillator
  can use the same SIMD synthesis as larger compatible banks.
- Frozen P0033 generator-lab SHA-256:
  `521ed2ed67a6d5e8c9fc6df51cbc12668fa1330db778a248b011f21bd71a2e3a`
- Broad candidate SHA-256:
  `b3b8bb23751ed50e83490351503bea61431afe1cbef9e8677586cca9c00ff2f0`
- Final endpoint-only candidate SHA-256:
  `31c5cedfb778ad406dfcf4b20b7a2e062b4c3756ff5b1a4b3b9841a3deeabd6c`

The broad candidate improved sine 16.22% and saw 3.55%, but regressed pulse
8.34% and a three-oscillator control 13.54%. Restricting SIMD to exact sine and
saw, outlining both endpoint and scalar loops, and isolating helper placement
produced a better performance candidate: sine improved 14.42%, saw 8.73%,
three oscillators 1.04%, and custom scalar 2.70%. Pulse retired 1.30% fewer
instructions and the dense packed control retired 0.31% fewer.

- Correctness failure: sine remained bit-exact and saw's scalar/SIMD residual
  was only 1.192e-7 peak at -160.673 dB RMS, but the existing realtime event
  test requires exact equality between contiguous and MIDI/MPE-sliced blocks.
  The SIMD saw accumulation changed least-significant bits after an event
  boundary, making output partition-dependent.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 0 passed, 1 failed.
- Decision: rejected and removed in full; host event partitioning remains
  bit-exact.

### R0018 - Cache-line-pad per-voice completion epochs

- Experiment: place each realtime-pool `voice_ready` atomic on its own 64-byte
  cache line to prevent helpers completing adjacent voices from false-sharing.
- Frozen P0033 generator-lab SHA-256:
  `521ed2ed67a6d5e8c9fc6df51cbc12668fa1330db778a248b011f21bd71a2e3a`
- Candidate generator-lab SHA-256:
  `f811b6c4421d34483f262f810b78efeea78ec141e89cdd3f8bfc1ad0e2a00fb8`

The first pooled sweep at eight-note polyphony changed 1, 3, 8, and 32
oscillators by +1.92%, +9.57%, -7.83%, and +0.35%, respectively. An
interleaved A/B/B/A check retained a noisy 6.81% apparent gain at eight
oscillators, but the 32-oscillator check regressed about 0.91%. Every measured
run had all three assigned helpers participating and zero deadline fallbacks.

- Finding: isolated completion words can help one scheduler/cache placement,
  but do not improve the oscillator-scaling endpoint and materially regress
  smaller banks. The change also expands 32 completion epochs from 128 bytes
  to 2,048 bytes of fixed pool storage.
- Decision: rejected and removed in full.

### R0019 - Pass precomputed x8 BLAMP distance

- Experiment: make the x8 BLAMP residual helpers consume absolute distance so
  the narrow-support triangle path can reuse its already nonnegative corner
  distance instead of taking one redundant SIMD absolute value per sample.
- Frozen P0033 generator-lab SHA-256:
  `521ed2ed67a6d5e8c9fc6df51cbc12668fa1330db778a248b011f21bd71a2e3a`
- Candidate generator-lab SHA-256:
  `193dc5ce5665dfc2adb42cad1b8dc690030ffb9b15f3b90c8ad3c93471e4d4e3`

At one 8-lane oscillator, interleaved timing was neutral: 230.564 before and
231.148 ns/frame after. At eight 8-lane oscillators, the candidate averaged
948.792 versus 956.714 ns/frame before, an apparent 0.83% reduction within the
observed run spread. Hardware counters on the dense case showed 0.31% fewer
instructions but 0.11% more cycles. Checksums were bit-identical.

- Finding: the removed vector operation is real, but too small to produce a
  repeatable cycle reduction in the complete oscillator path.
- Decision: rejected and removed in full.

### R0020 - Render active-jitter blocks oscillator-major

- Experiment: when structural unison jitter is active, precompute the bounded
  envelope block and render every frame of one oscillator before advancing to
  the next oscillator. The exact scalar oscillator routine and per-frame
  accumulation order were retained.
- Frozen P0033 generator-lab SHA-256:
  `521ed2ed67a6d5e8c9fc6df51cbc12668fa1330db778a248b011f21bd71a2e3a`
- Candidate generator-lab SHA-256:
  `f551b038692a8440b3913c7cc22b800087c48fa206d5b2b5bd36724df8d0119e`

Pinned active-jitter saw results at one unison lane and eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time change |
|---:|---:|---:|---:|
| 1 | 288.747 | 284.256 | -1.56% |
| 3 | 498.176 | 591.123 | +18.66% |
| 8 | 1,075.954 | 1,380.207 | +28.28% |
| 32 | 4,122.168 | 5,245.254 | +27.25% |

All checksums were bit-identical. The draft was also tightened before
measurement to exclude gliding voices, whose pitch must advance per frame.

- Finding: keeping one oscillator's state hot does not offset repeatedly
  walking the frame accumulators and creates substantially worse code/locality
  as the oscillator bank grows.
- Decision: rejected and removed in full.

### P0034 - Prepare warped x8 pulse support once per block

- File: `src/oscillators/va/render.rs`
- Defect: constant-step x8 warped and custom-wave kernels rebuilt the pulse
  support width inside the per-sample waveform closure. This kept vector
  clamps and related control flow in the hot loop even for a fixed oscillator
  block.
- Change: prepare the identical clamped support width beside the already
  prepared warped pulse edge and pass both into the sample kernel. Dynamic-step
  calls still prepare width per frame; x4 and unwarped kernels are unchanged.
- Realtime impact: removes repeat vector min/max work from each x8 oscillator
  sample; adds no approximation, state, allocation, lock, I/O, syscall, cache,
  or group-level optimization.
- Frozen P0033 generator-lab SHA-256:
  `521ed2ed67a6d5e8c9fc6df51cbc12668fa1330db778a248b011f21bd71a2e3a`
- Candidate generator-lab SHA-256:
  `831f91254d7df0b44cd4ce5662a07997b2ef656cabcc1a93deeee1ff1ed88985`

Pinned serial results at eight unison lanes and eight-note polyphony:

| Path | Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|---:|
| Warped pulse | 1 | 385.558 | 373.026 | 3.25% |
| Warped pulse | 3 | 901.354 | 801.041 | 11.13% |
| Warped pulse | 8 | 2,182.234 | 1,956.841 | 10.33% |
| Warped saw | 8 | 1,574.839 | 1,330.051 | 15.54% |
| 50% custom warped pulse | 1 | 425.951 | 406.784 | 4.50% |
| 50% custom warped pulse | 8 | 2,408.056 | 2,298.882 | 4.53% |

On dense warped pulse, hardware counters showed 7.75% fewer cycles and 14.08%
fewer retired instructions. The unwarped 8x8 saw control was neutral within
run spread at 539.223 before and 543.670 ns/frame after. Every benchmark
checksum was bit-identical.

Validation:

- Warped-pulse and custom/Harmonic scalar-versus-block diagnostics were
  text-identical before and after: 1.335e-5 at -122.913 dB and 7.510e-6 at
  -125.957 dB, respectively.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0035 - Prepare warped x4 pulse support once per block

- File: `src/oscillators/va/render.rs`
- Change: apply P0034's constant-step preparation to four-lane warped and
  custom-wave kernels. The identical support width is now computed beside the
  warped pulse edge once per oscillator block and passed to the sampler.
- Realtime impact: removes repeat vector clamps from every x4 sample; dynamic
  steps and x8 behavior are unchanged, with no approximation or realtime
  resource operation.
- Frozen P0034 generator-lab SHA-256:
  `831f91254d7df0b44cd4ce5662a07997b2ef656cabcc1a93deeee1ff1ed88985`
- Candidate generator-lab SHA-256:
  `c4f7689fd3729db7a7b9d3d35b34e9c630741a505632c9281e4e7cc507172e74`

Pinned serial results at four unison lanes and eight-note polyphony:

| Path | Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|---:|
| Warped pulse | 1 | 364.716 | 357.685 | 1.93% |
| Warped pulse | 3 | 844.118 | 763.163 | 9.59% |
| Warped pulse | 8 | 2,062.423 | 1,818.878 | 11.81% |
| 50% custom warped pulse | 8 | 2,307.191 | 2,163.912 | 6.21% |

Dense warped pulse retired 18.24% fewer instructions and used 9.97% fewer
cycles. All benchmark checksums were bit-identical.

Validation:

- Warped-pulse and custom/Harmonic scalar-versus-block diagnostics were
  text-identical before and after: 1.901e-5 at -120.919 dB and 9.179e-6 at
  -124.210 dB, respectively.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0021 - Specialize active-jitter one-lane oscillators

- Experiment: keep active-jitter one-lane oscillators in the existing scalar
  branch, avoiding the generic one-iteration lane loop and its neutral lane
  pitch and stereo-gain multiplications.
- Frozen P0035 generator-lab SHA-256:
  `c4f7689fd3729db7a7b9d3d35b34e9c630741a505632c9281e4e7cc507172e74`
- Candidate generator-lab SHA-256:
  `a55dc11c3933e8b59136886b6e8e0629c79f33b4abd00aa40280eb8ae3552bd9`

Pinned active-jitter saw results at one unison lane and eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time change |
|---:|---:|---:|---:|
| 1 | 300.793 | 346.041 | +15.04% |
| 3 | 512.278 | 540.159 | +5.44% |
| 8 | 1,065.805 | 1,175.623 | +10.30% |
| 32 | 4,161.050 | 4,226.717 | +1.58% |

All checksums were bit-identical.

- Finding: LLVM's existing constant one-iteration lane loop and surrounding
  branch layout outperform the hand-specialized active-jitter branch across
  the full oscillator scaling matrix.
- Decision: rejected and removed in full.

### P0036 - Reuse the narrow BLEP wrap-side mask

- File: `src/oscillators/va/antialias.rs`
- Defect: narrow x4/x8 BLEP paths first compared phase against support to find
  the lower discontinuity, then compared the same phase against 0.5 to choose
  its signed distance. With support below 0.5, the event mask already proves
  which side of the wrap every active correction occupies.
- Change: reuse the lower-event comparison for the narrow signed-distance
  blend, removing one SIMD comparison from each BLEP evaluation.
- Realtime impact: pure instruction removal; arithmetic, event masks, state,
  and all realtime resource behavior are unchanged.
- Frozen P0035 generator-lab SHA-256:
  `c4f7689fd3729db7a7b9d3d35b34e9c630741a505632c9281e4e7cc507172e74`
- Candidate generator-lab SHA-256:
  `e48c14fdadd3adc09bdabf95333afe041c95ada78b73ea6f5ff6a0fc445d5a4f`

Pinned dense eight-oscillator results at eight-note polyphony:

| Waveform | Unison lanes | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|---:|
| Saw | 4 | 706.808 | 700.670 | 0.87% |
| Pulse | 4 | 1,238.953 | 1,204.643 | 2.77% |
| Pulse | 8 | 861.921 | 837.027 | 2.89% |

The dense x4 pulse counter run used 1.28% fewer cycles and 0.11% fewer
instructions. All benchmark checksums were bit-identical.

Validation:

- x4 and x8 pulse scalar-versus-block diagnostics were text-identical before
  and after at 1.901e-5/-120.919 dB and 1.335e-5/-122.913 dB.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### P0037 - Prepare the constant x4 saw-to-pulse segment

- File: `src/oscillators/va/render.rs`
- Defect: constant four-lane saw-to-pulse rendering still entered the generic
  shape dispatcher every sample. It repeatedly classified the fixed shape,
  rebuilt morph gain and pulse support, and reconstructed BLEP bounds already
  known for the oscillator block.
- Change: reuse the existing precomputed x4 spline sampler with block-prepared
  segment, blend, gain, support width, active mask, and inverse step. Exact sine
  and triangle retain their original renderer because a broader prototype
  exposed least-significant arithmetic differences on triangle.
- Realtime impact: removes invariant dispatch and vector setup from every x4
  oscillator sample; no approximation, state, resource operation, or group
  optimization.
- Frozen P0036 generator-lab SHA-256:
  `e48c14fdadd3adc09bdabf95333afe041c95ada78b73ea6f5ff6a0fc445d5a4f`
- Candidate generator-lab SHA-256:
  `511aa14fcc6197145c134f425b3d1eb119eabf9261617b8b3dff71c27a8dd8e1`

Pinned serial results at four unison lanes and eight-note polyphony:

| Shape | Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|---:|
| Pulse | 1 | 279.691 | 240.837 | 13.89% |
| Pulse | 3 | 546.341 | 432.013 | 20.93% |
| Pulse | 8 | 1,223.309 | 931.319 | 23.87% |
| 50% saw-to-pulse morph | 8 | 1,377.975 | 982.739 | 28.68% |

Dense pulse used 23.25% fewer cycles and retired 28.82% fewer instructions.
The dedicated exact-saw control remained within run spread at 699.921 before
and 708.132 ns/frame after. All accepted-path benchmark checksums were
bit-identical, including the untouched triangle control.

Validation:

- x4 pulse scalar-versus-block diagnostics were text-identical before and
  after at 1.901e-5 peak and -120.919 dB RMS.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0022 - Write AVX2 accumulators through direct SIMD views

- Experiment: replace the explicit x8-to-array conversions around the AVX2
  saw accumulator loads/stores with `f32x8`'s guaranteed array views.
- Frozen P0037 generator-lab SHA-256:
  `511aa14fcc6197145c134f425b3d1eb119eabf9261617b8b3dff71c27a8dd8e1`
- Candidate generator-lab SHA-256:
  `913cfdbc375a3539ab4b6931a7f92c94a85cf02a5bd41fc799c284ff9ab074c7`

Interleaved dense x8 saw timing averaged 562.257 ns/frame before and 566.365
ns/frame after, a 0.73% regression. Checksums were bit-identical.

- Finding: LLVM already removes the staging arrays and conversions; spelling
  the loads and stores through array views does not reduce generated work.
- Decision: rejected and removed in full.

### R0023 - Hoist active-jitter oscillator pitch multiplication

- Experiment: compute `base_step * oscillator.pitch_ratio` once before the
  active-jitter lane loop instead of expressing it once per unison lane.
- Frozen P0037 generator-lab SHA-256:
  `511aa14fcc6197145c134f425b3d1eb119eabf9261617b8b3dff71c27a8dd8e1`
- Candidate generator-lab SHA-256:
  `3098405e182758170a30e936674ae0c18b3a8b872e0315b71ab1475811408de9`

Interleaved 8-oscillator by 8-lane active-jitter timing averaged 5,553.556
ns/frame before and 5,379.600 after, an apparent 3.13% reduction. Hardware
counters did not confirm it: cycles fell only 0.48% while instructions rose
0.40%. Checksums were bit-identical.

- Finding: LLVM already hoists the invariant multiplication; the timing shift
  is code-placement or system noise rather than less realtime work.
- Decision: rejected and removed in full.

### R0024 - Express prepared warp doubling as self-addition

- Experiment: replace exact SIMD multiplication by two with self-addition in
  prepared PWM and Phase Bend phase/derivative calculations.
- Frozen P0037 generator-lab SHA-256:
  `511aa14fcc6197145c134f425b3d1eb119eabf9261617b8b3dff71c27a8dd8e1`
- Candidate generator-lab SHA-256:
  `511aa14fcc6197145c134f425b3d1eb119eabf9261617b8b3dff71c27a8dd8e1`

- Finding: LLVM canonicalizes both spellings to identical generated code; the
  complete optimized release binaries are byte-for-byte identical.
- Decision: rejected and removed in full.

### P0038 - Reuse the exact x8 saw lower-wrap mask

- File: `src/oscillators/va/backend.rs`
- Defect: the exact AVX2 saw accumulator compared phase against the lower BLEP
  support bound to form its event mask, then repeated an equivalent comparison
  to choose the signed narrow-correction distance.
- Change: retain the lower-wrap comparison and reuse it for both decisions.
- Realtime impact: removes one vector comparison from every exact x8 narrow
  saw BLEP evaluation; arithmetic, oscillator state, and resource behavior are
  unchanged.
- Frozen P0037 generator-lab SHA-256:
  `511aa14fcc6197145c134f425b3d1eb119eabf9261617b8b3dff71c27a8dd8e1`
- Candidate generator-lab SHA-256:
  `a699c9c18950b3addefe563ca22a9e354d6456923fc801448758a7ff54fac6e1`

Pinned dense x8 saw results at eight oscillators and eight-note polyphony:

| Metric | Before | After | Reduction |
|---|---:|---:|---:|
| Interleaved median time | 542.138 ns/frame | 540.539 ns/frame | 0.29% |
| Retired instructions | 2,276,160,699 | 2,264,396,008 | 0.52% |
| CPU cycles | 918,621,907 | 917,373,071 | 0.14% |

The checksum remained bit-identical. A longer timing attempt was discarded
because unrelated scheduler interference increased run spread by several
hundred percent; none of those values were used above.

Validation:

- Scalar-versus-block diagnostics were text-identical before and after at
  1.907e-6 peak and -135.678 dB RMS.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted; small but counter-confirmed instruction removal with a
  simpler hot branch.

### P0039 - Prepare fixed four-lane custom morph controls

- File: `src/oscillators/va/render.rs`
- Defect: the constant four-lane custom-wave morph path repeatedly clamped and
  classified its fixed canonical shape, rebuilt blend/gain vectors, and
  broadcast the unchanged custom mix inside the sample loop.
- Change: prepare those scalar and SIMD values once per oscillator block and
  pass them to the established warped canonical sampler.
- Realtime impact: removes invariant setup from each sample of each custom x4
  oscillator pack; interpolation, BLEP arithmetic, oscillator state, and
  realtime resource behavior are unchanged.
- Frozen P0038 generator-lab SHA-256:
  `a699c9c18950b3addefe563ca22a9e354d6456923fc801448758a7ff54fac6e1`
- Candidate generator-lab SHA-256:
  `5818c86787e2c72bd9b89a6f05dbe52b44f5e25f03d5c0c905af0d26d2f27de1`

Pinned 50% custom / 50% warped-pulse results at four unison lanes and
eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 396.507 | 391.432 | 1.28% |
| 3 | 898.022 | 884.932 | 1.46% |
| 8 | 2,189.625 | 2,153.189 | 1.66% |

The dense case used 1.17% fewer CPU cycles and 0.07% fewer retired
instructions. All benchmark checksums were bit-identical. Overlapping runs
from two leaked benchmark children were detected by process inspection,
terminated, and discarded; the table contains only clean sequential runs with
an enforced per-process timeout and a zero-child check.

Validation:

- Pure-custom and custom/Harmonic scalar-versus-block diagnostics were
  text-identical before and after at -136.938 dB and -124.210 dB RMS.
- Existing voice and realtime-pool suites: 8 passed, 0 failed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0025 - Reuse the precomputed x4 BLEP lower-wrap mask

- Experiment: reuse the lower-support event comparison to choose the signed
  narrow BLEP distance in `spline_blep4_precomputed`, eliminating one explicit
  SIMD comparison.
- Frozen P0039 generator-lab SHA-256:
  `5818c86787e2c72bd9b89a6f05dbe52b44f5e25f03d5c0c905af0d26d2f27de1`
- Candidate generator-lab SHA-256:
  `dd8672f5b0ed56696a4c4b568f19028d20d06a6158a3a2cbd5364212346bf962`

Pinned pulse x4 results at eight-note polyphony:

| Oscillators | Before ns/frame | Candidate ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 240.934 | 240.627 | -0.13% |
| 3 | 419.727 | 427.353 | +1.82% |
| 8 | 902.603 | 906.326 | +0.41% |

All checksums were bit-identical.

- Finding: the source-level comparison removal does not produce a scaling win
  in this helper and perturbs SIMD scheduling or code placement adversely at
  three and eight oscillators.
- Decision: rejected and removed in full.

### R0026 - Borrow voice settings through structural helpers

- Experiment: pass the large `VoiceSettings` value by reference through the
  structural oscillator helper chain instead of by value.
- Frozen P0039 generator-lab SHA-256:
  `5818c86787e2c72bd9b89a6f05dbe52b44f5e25f03d5c0c905af0d26d2f27de1`
- Candidate generator-lab SHA-256:
  `7ce5fe4f2f1d0884582135ce7f61094e57b03342e2b6c1d8137d3f4db21af83a`

Pinned x8 saw results at eight-note polyphony:

| Oscillators | Before ns/frame | Candidate ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 189.428 | 197.746 | +4.39% |
| 3 | 290.048 | 280.974 | -3.13% |
| 8 | 542.947 | 540.452 | -0.46% |

All checksums were bit-identical. On the dense case, retired instructions were
effectively identical at 1,489,384,640 before and 1,489,383,237 after. The
candidate's cycle count moved favorably, but the scaling matrix did not.

- Finding: LLVM already eliminates the apparent value copies. Borrowing
  changes ABI/code placement, regresses the most important one-oscillator
  baseline, and provides no instruction-count reduction.
- Decision: rejected and removed in full.

### P0040 - Pack active-jitter oscillator lanes into SIMD generators

- File: `src/voices/voice.rs`
- Defect: any nonzero per-oscillator jitter disabled settled block rendering
  and sampled every unison lane through the scalar waveform generator. CPU
  therefore grew from both oscillator count and lane count without using the
  established x4/x8 generators.
- Change: preserve the sample-major oscillator loop, but render complete lane
  packs through the existing x8 and x4 custom, warped, or canonical generators;
  retain the scalar tail for remaining lanes and accumulate stereo lanes in
  their original order.
- Realtime impact: no new state, allocation, lock, I/O, syscall, or group-level
  cache. Jitter advancement, phase-state order, oscillator order, event
  boundaries, and bounded work are unchanged.
- Frozen P0039 generator-lab SHA-256:
  `5818c86787e2c72bd9b89a6f05dbe52b44f5e25f03d5c0c905af0d26d2f27de1`
- Candidate generator-lab SHA-256:
  `d2677a000291dd91068e5c873d1c8e7cbb83dc04a9d55fc36a6e254ad819d8b6`

Pinned active-jitter x8 saw results at eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 787.172 | 481.243 | 38.86% |
| 3 | 2,085.989 | 1,089.437 | 47.77% |
| 8 | 5,397.569 | 2,949.760 | 45.35% |

One-to-three oscillator scaling improves from 2.65x to 2.26x; one-to-eight
improves from 6.86x to 6.13x. On the dense case, CPU cycles fell 45.84% and
retired instructions fell 52.62%.

Additional dense eight-oscillator coverage:

| Path | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|
| 4 lanes | 3,102.923 | 1,924.738 | 37.97% |
| 5 lanes, x4 plus scalar tail | 3,553.936 | 2,564.344 | 27.85% |
| 7 lanes, x4 plus scalar tail | 5,071.566 | 3,856.989 | 23.95% |
| Pure custom x8 | 14,359.804 | 5,366.447 | 62.63% |
| Custom/Harmonic x8 | 82,050.404 | 12,125.139 | 85.22% |

The output change is limited to the established scalar-versus-SIMD floating
point ordering. For the main 65,536-frame saw case, the accumulated checksum
delta was 0.00177, or 2.70e-8 per frame. The jitter-on scalar/block diagnostic
remained internally bit-exact before and after; its 131,072-frame accumulated
checksum moved only 3.20e-6 between binaries.

Validation:

- Existing voice and realtime-pool suites: 8 passed, 0 failed when run serially.
  The pool-participation timing assertion also passed in isolation; parallel
  execution can starve a helper and is not a DSP correctness failure.
- Existing VA render suite: passed.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0027 - Prepare fixed eight-lane custom morph controls

- Experiment: mirror P0039 in the constant x8 custom renderer by preparing
  fixed shape classification, blend/gain vectors, and custom-mix broadcast once
  per block.
- Frozen P0040 generator-lab SHA-256:
  `d2677a000291dd91068e5c873d1c8e7cbb83dc04a9d55fc36a6e254ad819d8b6`
- Candidate generator-lab SHA-256:
  `df54c040746a6470fe6c5a57ec43bd92a5d1b2e6a73b916eea85fd06654afea9`

Pinned 50% custom / 50% warped-pulse x8 results at eight-note polyphony:

| Oscillators | Before ns/frame | Candidate ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 431.883 | 417.529 | -3.32% |
| 3, first pass | 976.773 | 991.383 | +1.50% |
| 3, reverse-order repeat | 954.824 | 980.489 | +2.69% |
| 8 | 2,349.115 | 2,282.104 | -2.85% |

All checksums were bit-identical.

- Finding: LLVM already hoists most x8 setup. The added helper changes code
  placement enough to produce endpoint wins but a repeatable regression at
  three oscillators, violating the oscillator-scaling acceptance criterion.
- Decision: rejected and removed in full.

### P0041 - Reuse prepared detune positions during spatial setup

- File: `src/voices/unison.rs`
- Defect: unison layout construction first computed every lane's nonlinear
  detune position, then recomputed the same curve mapping while deriving its
  stereo components.
- Change: pass the stored lane detune position to the existing position-based
  stereo helper.
- Realtime impact: reduces oscillator/unison initialization and spatial-control
  reconfiguration cost. Steady per-sample rendering, lane order, pitch, pan,
  state, and realtime resource behavior are unchanged.
- Frozen P0040 generator-lab SHA-256:
  `d2677a000291dd91068e5c873d1c8e7cbb83dc04a9d55fc36a6e254ad819d8b6`
- Candidate generator-lab SHA-256:
  `aea7846dba1712ade8c7786d0932ba0fca3ccbf332fb3de789eb83c6f5467c78`

Pinned spatial-reconfiguration results:

| Unison lanes | Before ns/config | After ns/config | Time reduction |
|---:|---:|---:|---:|
| 8 | 2,018.092 | 1,894.651 | 6.12% |
| 32 | 3,689.818 | 3,465.374 | 6.08% |
| 64 | 5,716.376 | 5,415.685 | 5.26% |

At 64 lanes, CPU cycles fell 9.64% and retired instructions fell 6.97%.
The tuning-only control moved 1.24% inside run spread and does not exercise the
removed spatial recomputation.

Validation:

- The reused position is populated before every full/spatial rebuild and is
  the same argument the removed helper derived from voice count and curve.
- Existing voice and realtime-pool suites: 8 passed, 0 failed when run serially.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### M0013 - VA-table selection benchmark

- File: `examples/generator_lab.rs`
- Change: add `bench-va-table-select` with exact-frame and fractional-frame
  workloads over a full 16-frame compiled VA table.
- Production DSP changed: no.
- Purpose: measure coefficient-selection work independently from oscillator
  rendering and retain fractional interpolation as a control.
- Decision: accepted as VA-table specialization infrastructure.

### R0028 - Bypass interpolation at exact VA-table frames

- Experiment: when table morph position lands exactly on a compiled frame,
  return that frame directly instead of interpolating all 64 coefficients with
  a zero mix.
- Frozen M0013 generator-lab SHA-256:
  `414c2df981377f14af8bb371116b9a26674068fbe24a423331836d288a5379da`
- Candidate generator-lab SHA-256:
  `ddfba8a1512125c864c99e89b59faf1219636c9e2b57d529e0b09d7baad93819`

Exact-frame selection improved from 7.622 to 4.451 ns/select in the first pass
(41.60%) and from 8.170 to 4.536 ns/select in the longer reverse-order pass
(44.49%). The fractional control was unstable in wall time: 1.29% slower in
the first pass and 13.72% faster in the longer pass.

Hardware counters resolved the ambiguity: on fractional morphs, the candidate
retired 581,984,942 instructions versus 525,984,334 before, a 10.65% increase,
despite a favorable cycle movement from code placement/frequency.

- Finding: the exact-frame branch is valuable only for parked positions and
  adds steady work to continuous morphing, the primary VA-table path. A future
  optimization should cache unchanged selection outside the sample loop rather
  than branch inside every selection.
- Decision: rejected and removed in full.

### P0042 - Hoist active-jitter oscillator setup

- File: `src/voices/voice.rs`
- Defect: after P0040 packed jitter lanes into SIMD, each x8/x4 pack and scalar
  tail still reloaded oscillator channel gains and re-expressed the same
  base-step by oscillator-pitch multiplication.
- Change: compute the oscillator phase multiplier and channel gains once per
  oscillator sample, then reuse them across all lane packs and tails.
- Realtime impact: exact arithmetic grouping, output order, state, and bounded
  resource behavior are unchanged.
- Frozen M0013 generator-lab SHA-256:
  `414c2df981377f14af8bb371116b9a26674068fbe24a423331836d288a5379da`
- Candidate generator-lab SHA-256:
  `493e9f2185f9b3f3b125772feefca1e99566e4741bd88708116d42bacda983f6`

Pinned active-jitter x8 saw results at eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time change |
|---:|---:|---:|---:|
| 1, long reverse-order repeat | 467.832 | 469.601 | +0.38% |
| 3 | 1,104.072 | 1,086.291 | -1.61% |
| 8 | 2,923.707 | 2,875.024 | -1.67% |

Wall time on the smallest case remained frequency-sensitive, but counters
confirmed real work removal at both endpoints:

| Oscillators | Cycle reduction | Instruction reduction |
|---:|---:|---:|
| 1 | 8.57% | 0.59% |
| 8 | 2.14% | 1.48% |

All benchmark checksums were bit-identical.

Validation:

- Existing voice and realtime-pool suites: 8 passed, 0 failed when run serially.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted on counter-confirmed work removal and improved
  multi-oscillator scaling; the small one-oscillator wall movement is retained
  explicitly rather than hidden.

### R0029 - Derive the AVX2 wave-curve bank mask from selector bit three

- Experiment: rely on `VPERMD` consuming only the low three selector bits and
  derive its upper coefficient-bank blend mask by shifting segment bit three
  into the sign bit, replacing an explicit index mask and comparison.
- Frozen P0042 generator-lab SHA-256:
  `493e9f2185f9b3f3b125772feefca1e99566e4741bd88708116d42bacda983f6`
- Candidate generator-lab SHA-256:
  `fd1722b07f010f2a5ef8694dc70e807c8434f50d885c124e1577ecdb8c69bd12`

Pinned oscillator results at eight-note polyphony:

| Path | Before ns/frame | Candidate ns/frame | Change |
|---|---:|---:|---:|
| Pure custom x4 | 214.312 | 202.373 | -5.57% |
| Pure custom x8 | 198.394 | 206.845 | +4.26% |
| 50% custom x8, 1 oscillator | 269.556 | 259.557 | -3.71% |
| 50% custom x8, 3 oscillators | 526.705 | 529.267 | +0.49% |
| 50% custom x8, 8 oscillators | 1,207.673 | 1,206.717 | -0.08% |

All checksums were bit-identical. On pure custom x8, cycles moved favorably by
2.44% but retired instructions increased 0.14% and wall time regressed 4.26%.

- Finding: instruction substitution and code placement help the widened x4
  caller but hurt native x8 evaluation and do not improve dense oscillator
  scaling.
- Decision: rejected and removed in full.

### R0030 - Spell the exact x8 upper BLEP boundary outside the loop

- Experiment: compute `1 - support` beside the block-constant x8 BLEP support
  instead of expressing the subtraction inside the sample loop.
- Frozen P0042 generator-lab SHA-256:
  `493e9f2185f9b3f3b125772feefca1e99566e4741bd88708116d42bacda983f6`
- Candidate generator-lab SHA-256:
  `9fe75279aedac25524c434bec6b689eac6d0679a7306b4564b4ed05864e2f697`

Dense x8 saw timing averaged 549.379 ns/frame before and 542.589 after, an
apparent 1.24% reduction. Checksums were bit-identical. Hardware counters did
not confirm instruction removal: cycles fell 3.08%, but retired instructions
increased 0.30% from 1,293,904,697 to 1,297,759,383.

- Finding: LLVM already loop-hoists the invariant subtraction; the source
  spelling changes layout but not the intended hot-path work.
- Decision: rejected and removed in full.

### P0043 - Fuse jitter advancement into SIMD phase-step construction

- File: `src/voices/voice.rs`
- Defect: active-jitter rendering first walked every lane to advance its ratio,
  then walked the same arrays again to construct x8/x4/scalar phase steps.
- Change: advance each ratio immediately before its phase-step load inside the
  existing lane packs and scalar tail, eliminating the separate lane pass.
- Realtime impact: update-before-sample order, lane order, oscillator order,
  floating-point arithmetic, state, and bounded resource behavior are exact.
- Frozen P0042 generator-lab SHA-256:
  `493e9f2185f9b3f3b125772feefca1e99566e4741bd88708116d42bacda983f6`
- Candidate generator-lab SHA-256:
  `a61a7349e4299a0d76e0bef09566faa3c707f40cde11710423e5df81dc6232a1`

Pinned active-jitter x8 saw results at eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 466.129 | 405.293 | 13.05% |
| 3 | 1,051.712 | 864.330 | 17.82% |
| 8 | 2,996.821 | 2,223.468 | 25.81% |

One-to-three oscillator scaling improves from 2.26x to 2.13x; one-to-eight
improves from 6.43x to 5.49x.

Additional dense eight-oscillator coverage:

| Path | Before ns/frame | After ns/frame | Time reduction |
|---|---:|---:|---:|
| 4 lanes | 2,086.187 | 1,509.772 | 27.63% |
| 5 lanes, x4 plus scalar tail | 2,496.362 | 2,163.102 | 13.35% |
| 7 lanes, x4 plus scalar tail | 3,593.236 | 3,365.399 | 6.34% |
| Custom/Harmonic x8 | 12,355.263 | 11,414.488 | 7.61% |

Hardware counters confirmed the removal:

| Oscillators | Cycle reduction | Instruction reduction |
|---:|---:|---:|
| 1 | 8.23% | 14.50% |
| 8 | 22.78% | 20.50% |

Every benchmark checksum was bit-identical.

Validation:

- Existing voice and realtime-pool suites: 8 passed, 0 failed when run serially.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0031 - Spell x4 triangle peak-distance reuse in source

- Experiment: name the precomputed x4 triangle peak distance once and reuse it
  for the narrow-support event test, instead of spelling the same vector
  subtraction and absolute value twice.
- Frozen P0043 generator-lab SHA-256:
  `a61a7349e4299a0d76e0bef09566faa3c707f40cde11710423e5df81dc6232a1`
- Candidate generator-lab SHA-256:
  `a61a7349e4299a0d76e0bef09566faa3c707f40cde11710423e5df81dc6232a1`
- Finding: the complete optimized generator-lab binaries were byte-for-byte
  identical. LLVM already eliminates the redundant source expression, so the
  spelling removes no machine work and cannot improve oscillator scaling.
- Decision: rejected and removed in full.

### M0014 - Add locality-sensitive VA-table selection workload

- File: `examples/generator_lab.rs`
- Change: extend `bench-va-table-select` with a `local` mode that continuously
  moves within one adjacent-frame interval. Existing `fractional` mode keeps
  hopping across intervals, providing the adversarial cache-miss control.
- Purpose: distinguish useful slowly automated morph locality from an
  optimization that only wins a synthetic fixed position.
- Generator-lab SHA-256:
  `d8e64a615805468c19e9744b3ba3e1af491c5f4b2017e74bcdc525a2c6cae717`
- Decision: accepted as VA-table locality benchmark infrastructure.

### R0032 - Precompute every VA-table adjacent-frame delta

- Experiment: store all 16 adjacent coefficient deltas beside every compiled
  table, replacing 64 subtraction operations with cached loads during each
  table-frame interpolation.
- Frozen P0043 generator-lab SHA-256:
  `a61a7349e4299a0d76e0bef09566faa3c707f40cde11710423e5df81dc6232a1`
- Candidate generator-lab SHA-256:
  `a4f815ba27ba08c3f0608e3f028a4cd5fb999d65e6c2308a6b802a70c6bc00c1`

Pinned selection results:

| Path | Before ns/select | Candidate ns/select | Time reduction |
|---|---:|---:|---:|
| Exact frames | 8.049 | 6.396 | 20.54% |
| Fractional morphs | 8.354 | 6.405 | 23.33% |

Exact and fractional hardware counters fell by 20.25% and 21.47% in cycles,
respectively, with 22.85% fewer instructions on both paths.

- Cost: `VaTableRt` grows by 4,096 bytes. Across 32 oscillator tables and the
  three legacy transition copies, realtime state grows by 167,936 bytes, and
  publication must rebuild and clone the larger objects.
- Finding: the speedup is real, but structural oscillator tables select only
  when their configuration changes. Doubling all 32 table objects to optimize
  the legacy continuously-morphed path is the wrong ownership boundary.
- Decision: rejected and removed in full.

### R0033 - Cache one VA-table interval in each legacy transition

- Experiment: keep one 64-coefficient adjacent-frame delta beside each legacy
  table transition, invalidating it only when the table or interval changes.
  This adds about 260 bytes per transition and leaves every `VaTableRt`
  unchanged.
- Frozen M0014 generator-lab SHA-256:
  `d8e64a615805468c19e9744b3ba3e1af491c5f4b2017e74bcdc525a2c6cae717`
- Best fractional candidate SHA-256:
  `21543249a1a0a5c789ef15d85bbd682d829871aae2570fbe1d961229ecc15a75`
- Exact-preserving variant SHA-256:
  `421e23da3ff02275273b711b5a13ad063b2b6a6eef4bcad1e6ce866cdea7b52f`

The best fractional layout reduced within-interval morph time from an average
8.500 to 7.250 ns/select (14.71%) and interval-thrashing time from 11.724 to
9.083 ns/select (22.53%). It regressed exact-frame selection from 7.764 to
9.504 ns/select (22.41%). Restoring the original exact interpolation recovered
exact-frame performance, but then regressed the locality path from 8.528 to
10.956 ns/select (28.48%). Hardware counters confirmed these were compiled
layout and dependency effects, not measurement-only noise.

- Finding: neither layout improves all common automation modes, and the patch
  adds mutable cache state plus invalidation obligations to the transition.
- Decision: rejected and removed in full.

### R0034 - Cache the active-jitter clock step per oscillator

- Experiment: cache `jitter_rate_hz / sample_rate` in each oscillator's
  rendered settings, refreshing it on configuration, sample-rate changes, and
  jitter-rate transitions instead of expressing the division in every active
  jitter oscillator, voice, and sample.
- Frozen M0014 generator-lab SHA-256:
  `d8e64a615805468c19e9744b3ba3e1af491c5f4b2017e74bcdc525a2c6cae717`
- Candidate generator-lab SHA-256:
  `97f20f857872111dfde71ebf618e8355e67277905ea72110142e28f22f1e4429`

At the default 0.7 Hz jitter rate, paired 1/3/8-oscillator runs changed by
+0.81%, -3.48%, and +0.65%. At 100 Hz, a longer counter run changed one
oscillator by +2.12% and eight oscillators by -0.90%. Checksums were exact.
Instructions fell only 0.21% at one oscillator and 0.33% at eight; cycles fell
3.40% and 1.44%, respectively, without a stable wall-time reduction.

- Finding: LLVM already makes the expressed division cheap enough that the
  added field load, oscillator-state footprint, and refresh obligations trade
  layout effects instead of producing a repeatable scaling win.
- Decision: rejected and removed in full.

### R0035 - Hoist settled block pitch multiplication per oscillator

- Experiment: compute `base_step * oscillator.pitch_ratio` once before the
  x8/x4/scalar unison lanes in the settled structural block renderer.
- Frozen M0014 generator-lab SHA-256:
  `d8e64a615805468c19e9744b3ba3e1af491c5f4b2017e74bcdc525a2c6cae717`
- Candidate generator-lab SHA-256:
  `752ed5f635147b0dd91e70e16eba65021673ce522542590dc9f9f130ccc85d6d`

Paired 1/3/8-oscillator wall averages changed by +6.68%, -0.31%, and +1.25%.
A longer counter pass changed one oscillator by +4.72% and eight oscillators
by -0.51%; retired instructions changed by only -0.03% and -0.15%, while
cycles moved +1.22% and -1.03%. All checksums were bit-identical.

- Finding: LLVM already reuses the multiplication in the widened lanes; the
  source spelling mainly changes code placement and does not improve scaling.
- Decision: rejected and removed in full.

### P0044 - Share the x8 warped cycle-reset BLEP

- File: `src/oscillators/va/render.rs`
- Defect: the x8 warped saw-to-pulse renderer evaluated the same cycle-reset
  BLEP independently for both endpoints before blending them.
- Change: evaluate that pure SIMD correction once and reuse it in the saw and
  pulse endpoint expressions. The pulse-width edge remains independent.
- Realtime impact: removes duplicated antialias work without changing
  arithmetic, state, branching, allocation, locking, or output order.
- Frozen M0014 generator-lab SHA-256:
  `d8e64a615805468c19e9744b3ba3e1af491c5f4b2017e74bcdc525a2c6cae717`
- Candidate generator-lab SHA-256:
  `11c48232191a11d599f09cef1f40dc23176ba13d7a00b52626ced03b41f35124`

Pinned warped saw-to-pulse midpoint results at eight unison lanes and
eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 417.240 | 385.398 | 7.63% |
| 3 | 957.027 | 888.136 | 7.20% |
| 8 | 2,356.796 | 2,117.632 | 10.15% |

One-to-eight oscillator scaling improves from 5.65x to 5.50x. All benchmark
checksums were bit-identical.

Hardware counters confirmed the duplicated work removal:

| Oscillators | Cycle reduction | Instruction reduction |
|---:|---:|---:|
| 1 | 10.34% | 9.57% |
| 8 | 8.61% | 11.75% |

Pure triangle, triangle-to-saw, saw, pulse, and x4 controls showed no
systematic regression; the candidate only changes the widened endpoint blend.

Validation:

- Existing voice and realtime-pool suites: 8 passed, 0 failed when run serially.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0036 - Blend L2 and interpolating wave-curve fits

- Experiment: blend two-thirds of the L2-optimal periodic spline controls with
  one-third of the previous interpolating controls to reduce L2 overshoot.
- Result: the blend retained RMS and maximum-error improvements, but dense-edit
  overshoot still rose from 0.03447 to 0.06208 (+80.1%) and edit compilation
  increased from 1.390 to 4.600 microseconds (3.31x) because it required two
  linear solves.
- Finding: it gives up global accuracy, does not eliminate the main overshoot
  tradeoff, and costs more offline work than the mathematically simpler fit.
- Decision: rejected and removed in full.

### P0045 - Fit custom waves with an L2-optimal periodic spline

- File: `src/wave_curve.rs`
- Defect: the 16-segment runtime spline was forced through only 16 point
  samples, leaving avoidable integrated and peak deviation from the editable
  monotone source curve between those points.
- Change: integrate the source curve against the periodic cubic B-spline basis
  with exact four-point Gauss-Legendre quadrature per source/basis interval,
  then solve the 16-control normal system in `f64` on the edit/state thread.
- Runtime contract: the evaluator, 16 segments, 64 stored coefficients, state
  format, SIMD code, and audio-thread work are unchanged.
- Generator-lab SHA-256 before:
  `11c48232191a11d599f09cef1f40dc23176ba13d7a00b52626ced03b41f35124`
- Generator-lab SHA-256 after:
  `235842c3bcdd33a3d45cecc2018def82b507f82ad5b6fe7fc94dcd072473cbc6`

Approximation results against the editable source curve at 65,536 phases:

| Curve | RMS reduction | Maximum-error reduction | Overshoot change |
|---|---:|---:|---:|
| Default | 13.39% | 21.43% | +0.00072 absolute |
| Asymmetric | 9.62% | 20.24% | -37.72% |
| Narrow pulse | 19.40% | 31.68% | -32.02% |
| Dense 16-knot edit | 3.51% | 6.98% | 0.03447 to 0.08254 |

Periodic seam error remained at or below `5.96e-8`. Median edit compilation
rose from 1.394 to 2.918 microseconds; this work is outside the audio callback.

Long custom-render controls were neutral: retired instructions changed +0.12%
at three oscillators and +0.05% at eight, with wall medians moving -1.83% and
-1.15%. The differing checksums reflect the intentionally improved compiled
curve coefficients.

- Compatibility: existing knot-based custom curves retain their state but
  recompile to slightly different, globally more accurate coefficients.
- Validation: release generator lab and optimized library test target compile
  successfully; the repository has no pre-existing wave-curve unit tests.
- Decision: accepted.

### P0046 - Share the x4 warped cycle-reset BLEP

- File: `src/oscillators/va/render.rs`
- Defect: the x4 warped saw-to-pulse renderer repeated the same cycle-reset
  BLEP for both blended endpoints, mirroring the x8 defect removed by P0044.
- Change: compute the x4 correction once and reuse it in both endpoint
  expressions; the pulse-width edge remains independently antialiased.
- Runtime contract: exact arithmetic, output order, oscillator state, and
  realtime resource behavior are unchanged.
- Frozen P0045 generator-lab SHA-256:
  `235842c3bcdd33a3d45cecc2018def82b507f82ad5b6fe7fc94dcd072473cbc6`
- Candidate generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`

Pinned warped saw-to-pulse midpoint results at four unison lanes and
eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 404.039 | 380.354 | 5.86% |
| 3 | 885.522 | 821.905 | 7.18% |
| 8 | 2,189.417 | 1,970.027 | 10.02% |

One-to-eight oscillator scaling improves from 5.42x to 5.18x. Every target
and control checksum was bit-identical.

Hardware counters confirmed the removal:

| Oscillators | Cycle reduction | Instruction reduction |
|---:|---:|---:|
| 1 | 7.20% | 7.71% |
| 8 | 6.75% | 9.83% |

Triangle-to-saw, pure saw, and x8 controls showed no systematic regression.

Validation:

- Aggregate voice suite: 7 passed and the known scheduling-sensitive worker
  participation assertion flaked once; that exact assertion passed in an
  immediate isolated serial rerun.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted.

### R0037 - Select custom-curve coefficients with AVX2 gathers

- Experiment: replace two coherent eight-float loads, two variable permutes,
  and one blend per coefficient plane with one indexed AVX2 gather.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `206612509a0a73370323ad2e07f4829badfc249c78269d8c036970929f065f97`

Dense custom rendering regressed catastrophically despite exact checksums:

| Lane path | 1 oscillator | 3 oscillators | 8 oscillators |
|---|---:|---:|---:|
| x4 adapter | +51.48% | +83.95% | +100.98% |
| x8 native | +45.90% | +72.74% | +79.30% |

- Finding: coherent coefficient-bank loads and in-register permutes are far
  cheaper than gather latency on this Zen 4 CPU; the penalty compounds with
  oscillator count.
- Decision: rejected and removed in full without further counter runs.

### R0038 - Vectorize the active-jitter x4 remainder explicitly

- Experiment: replace the scalar four-lane jitter update and phase-step
  construction after the x8 loop with explicit `f32x4` loads, arithmetic,
  clamp, and state writeback.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `71207781de6210b4fbcb8f2dfd56f789d466e644aea650359d0d222634a6f9c8`
- Output: every target checksum was bit-identical.

Pinned active-noise-jitter results, averaged across the interleaved duplicate
runs:

| Unison voices | Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|---:|
| 5 | 1 | 432.488 | 442.379 | +2.29% |
| 5 | 3 | 916.440 | 912.924 | -0.38% |
| 5 | 8 | 2,219.615 | 2,298.085 | +3.54% |
| 7 | 1 | 581.843 | 575.377 | -1.11% |
| 7 | 3 | 1,352.852 | 1,381.166 | +2.09% |
| 7 | 8 | 3,493.314 | 3,581.562 | +2.53% |

- Finding: materializing arrays around the portable SIMD vectors costs more
  than the four arithmetic lanes save, and the penalty compounds at higher
  oscillator counts.
- Decision: rejected and removed in full without counter runs.

### R0039 - Evaluate one triangle BLAMP residual below half-cycle support

- Experiment: when cubic BLAMP support is below half a cycle, select the only
  possible wrap-side residual instead of evaluating both residual polynomials.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Combined x4/x8 candidate SHA-256:
  `0f26a7e499601577812e7a934d253c6ea24050cc2c0401d5a5dc76f0b80cd80e`
- Trimmed x4 candidate SHA-256:
  `cacbef7617822e99f571251d29fc68ba99ed544821f6fbc8b8f86dd02ace7e07`
- Trimmed x4 plus inline hint SHA-256:
  `a474f8d16459e12d896ed8ff0fd2603bfe4177afd9bf3643ba307aaf50cfaa98`
- Output: every target and control checksum was bit-identical.

The combined candidate improved four-lane MIDI-117 triangle rendering by
4-6%, while the eight-lane confirmation was neutral at roughly +0.3%. The x8
branch was therefore removed. Adding the x4 inline hint strengthened the rare
high-note result:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 391.898 | 355.986 | 9.16% |
| 3 | 863.496 | 800.972 | 7.24% |
| 8 | 2,052.093 | 1,927.618 | 6.07% |

However, the ordinary MIDI-69 four-lane, eight-oscillator control regressed
from 1,121.582 to 1,207.241 ns/frame, or 7.64%, despite taking the existing
low-support return path.

- Finding: enlarging and inlining the rare high-frequency fallback perturbs
  common-path code generation enough to erase more value than it creates.
- Decision: rejected and removed in full without counter runs.

### R0040 - Load and store AVX2 saw lanes through direct array views

- Experiment: remove temporary `[f32; 8]` conversions around the intrinsic
  AVX2 saw block kernel and load/store through `wide::f32x8` array views.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `6bd4147046cf413791ca2b6d4c71f0c4c3cd50ebdbe2c8a7e60197329a760e8d`
- Output: every target checksum was bit-identical.

Pinned static x8 saw results, averaged across interleaved duplicate runs:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 184.075 | 191.208 | +3.87% |
| 3 | 285.714 | 301.731 | +5.61% |
| 8 | 541.546 | 571.300 | +5.49% |

- Finding: the explicit borrowed views constrain code generation; LLVM's
  temporary-array form schedules the register transfers more efficiently.
- Decision: rejected and removed in full without counter runs.

### R0041 - Hoist x4 warped shape preparation out of the sample loop

- Experiment: clamp and classify the fixed morph shape once per 32-sample
  block, then reuse its segment, blend vector, and normalization gain.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `978e9a535cf3ecf437b15f7b5fc6e92926935802aa26b3f76bf9c7afde5c8d00`
- Output: every target and control checksum was bit-identical.

The saw-to-pulse midpoint initially looked useful:

| Oscillators | Before ns/frame | After ns/frame | Time reduction |
|---:|---:|---:|---:|
| 1 | 369.912 | 364.364 | 1.50% |
| 3 | 815.314 | 810.314 | 0.61% |
| 8 | 1,973.071 | 1,930.019 | 2.18% |

The required cross-segment controls reversed that result at eight oscillators:

| Control | Before ns/frame | After ns/frame | Change |
|---|---:|---:|---:|
| Sine-to-triangle midpoint | 2,433.990 | 2,575.592 | +5.82% |
| Triangle-to-saw midpoint | 1,906.468 | 1,999.556 | +4.88% |
| Untouched x8 saw-to-pulse path | 2,111.269 | 2,146.631 | +1.67% |

- Finding: the additional live SIMD preparation values increase register and
  code-layout pressure enough to make most morph segments slower.
- Decision: rejected and removed in full without counter runs.

### R0042 - Clamp sample rate once before oscillator jitter work

- Experiment: normalize sample rate once at each oscillator-bank render entry
  and pass that value through the jitter clock and refresh calculations.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `acceb0a921ae6c9ffaed4e3c25bcb28b63999ec058e11d6cf02ebdd877b04138`
- Output: every target checksum was bit-identical.

Pinned active-jitter x8 saw results, averaged across interleaved duplicate
runs:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 406.296 | 414.007 | +1.90% |
| 3 | 877.743 | 849.950 | -3.17% |
| 8 | 2,269.223 | 2,295.523 | +1.16% |

- Finding: the compiler already treats the repeated clamp cheaply; explicit
  propagation produces unstable code-layout movement and worsens the main
  eight-oscillator scaling target.
- Decision: rejected and removed in full without counter runs.

### R0043 - Accumulate the unison jitter sum during lane generation

- Experiment: remove the first centering pass over the jitter lane array by
  adding each generated value to the ordered sum as it is stored.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `f8ae985a07e89caf4768b45643c43ff0ed0d6062c72cad3a9ff765aac81ca192`
- Output: every target checksum was bit-identical.

Pinned 64-lane sine-jitter refresh results at 100 Hz, averaged across
interleaved duplicate runs:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 1,764.727 | 1,788.163 | +1.33% |
| 3 | 5,284.396 | 5,432.664 | +2.81% |
| 8 | 13,942.783 | 13,946.280 | +0.03% |

- Finding: the loop-carried sum dependency costs at least as much as the
  removed lane-array read pass and is worse at lower oscillator counts.
- Decision: rejected and removed in full without noise-mode or counter runs.

### R0044 - Unroll the AVX2 saw kernel by two frames

- Experiment: expand two sequential frame bodies per loop iteration while
  preserving frame-one-to-frame-two phase and accumulation dependencies.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `c01afd67b06e0d5256325e08de3667ee71ea941957384aa6005b2f999cd87491`
- Output: every target checksum was bit-identical.

The short sweep appeared faster, but a longer six-way interleaved confirmation
removed the warm-up bias:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 188.401 | 188.492 | +0.05% |
| 3 | 286.531 | 291.889 | +1.87% |
| 8 | 541.090 | 553.745 | +2.34% |

- Finding: LLVM's compact original loop schedules better than the expanded
  body; extra code size and live state dominate the removed branch overhead.
- Decision: rejected and removed in full without counter runs.

### R0045 - Specialize homogeneous AVX2 BLEP event lanes

- Experiment: reuse the event bitmask and skip either the inner or outer
  optimized residual polynomial when every active event lane selects the same
  region; retain both for mixed vectors.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `f0e6a175ce58a40e173a6e788620c5804f9bbbc9c4b7aba0cb2ceae1864d6860`
- Output: every target checksum was bit-identical.

Pinned static x8 saw results, averaged across interleaved duplicate runs:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 183.634 | 190.544 | +3.76% |
| 3 | 289.232 | 294.248 | +1.73% |
| 8 | 546.614 | 532.085 | -2.66% |

- Finding: the extra mask extraction and branches only amortize at the largest
  bank; they worsen the default one-oscillator base cost and three-oscillator
  scaling.
- Decision: rejected and removed in full without high-note or counter runs.

### R0046 - Bypass BLEP work for an all-lane-safe 32-sample block

- Experiment: replay 31 ordered phase additions before the AVX2 loop and take
  a simpler block renderer only when all eight lanes provably remain between
  both BLEP support bands.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `54b5c99bb656f57467dddd0f8a00d5534fe273032f3047e1cbc7e382bcdcdaf6`
- Output: every target checksum was bit-identical.

Pinned static x8 saw results, averaged across interleaved duplicate runs:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 183.659 | 190.436 | +3.69% |
| 3 | 281.092 | 316.160 | +12.48% |
| 8 | 536.120 | 592.157 | +10.45% |

- Finding: randomized unison phases make all-eight-lane safe blocks too rare;
  the exact 31-add proof becomes pure overhead in most calls.
- Decision: rejected and removed in full without counter runs.

### P0047 - Specialize the warped x8 Saw-to-Pulse morph

- File: `src/oscillators/va/render.rs`
- Defect: the common Saw-to-Pulse interval paid the generic four-shape
  classifier and morph dispatch for every SIMD frame even though its two
  endpoints and BLEP topology are fixed for the whole oscillator block.
- Change: select the open `(2, 3)` shape interval once per x8 block and run a
  fixed Saw/Pulse renderer with the morph blend, warp preparation, and shared
  cycle-reset BLEP kept outside the generic shape classifier.
- Realtime impact: phase advancement, fractional edge placement, BLEP order,
  morph arithmetic, accumulation order, state writes, and bounded resource
  behavior are unchanged.
- Frozen P0046 generator-lab SHA-256:
  `de6c4eaec1ffef6156cb34fe692c616beafbafc6024096ab1ae3d9a1f2766316`
- Candidate generator-lab SHA-256:
  `e99d6e1cfbc6704958593e93eaf7c703c208fb360a4068341da7299cab1956ac`

Pinned warped Saw-to-Pulse x8 results, averaged across interleaved duplicate
runs at eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 2,079.990 | 1,802.863 | -13.32% |
| 3 | 5,975.477 | 4,979.898 | -16.66% |
| 8 | 15,621.601 | 12,942.189 | -17.15% |

- One-to-three oscillator scaling improved from `2.873x` to `2.762x` and
  one-to-eight scaling improved from `7.510x` to `7.179x`.
- All target and control checksums were bit-identical. Three longer control
  pairs on the neighboring Triangle-to-Saw interval averaged -0.36% at one
  oscillator and +0.57% at eight oscillators, inside measurement noise.
- Existing voice suite: 8 passed, 0 failed when run serially.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted for a double-digit oscillator-local reduction that grows
  with bank size and improves the requested multi-oscillator scaling.

### P0048 - Unswitch the x8 custom-wave mix outside the sample loop

- File: `src/oscillators/va/render.rs`
- Defect: every x8 custom-wave frame retested whether the oscillator was fully
  custom and rebuilt the partial-mix SIMD splat even though both values remain
  fixed for the entire oscillator block.
- Change: select pure-custom versus partial-morph rendering once per block and
  construct the partial-mix vector once before its frame loop.
- Realtime impact: curve evaluation, canonical-wave correction, morph FMA,
  phase advancement, accumulation order, state writes, and bounded resource
  behavior are unchanged.
- Frozen P0047 generator-lab SHA-256:
  `e99d6e1cfbc6704958593e93eaf7c703c208fb360a4068341da7299cab1956ac`
- Candidate generator-lab SHA-256:
  `2660910e303f559c29630428e5eea8ff95091c580346cf1b7db82f5488caa878`

Pinned 50% custom-wave x8 results, averaged across interleaved duplicate runs
at eight-note polyphony:

| Oscillators | Before ns/frame | After ns/frame | Change |
|---:|---:|---:|---:|
| 1 | 1,088.939 | 1,041.345 | -4.37% |
| 3 | 3,119.571 | 2,798.417 | -10.29% |
| 8 | 7,660.414 | 7,232.096 | -5.59% |

- One-to-three oscillator scaling improved from `2.865x` to `2.687x`; the
  one-to-eight ratio improved from `7.035x` to `6.945x`.
- Pure-custom controls were neutral to favorable: -0.29% at one oscillator and
  -2.52% at three oscillators. Every target and control checksum was
  bit-identical.
- Existing voice suite: 7 passed before the known helper-participation
  scheduling assertion flaked; that exact test passed immediately in isolation.
- Realtime-audited event-boundary test: 1 passed, 0 failed, zero violations.
- Decision: accepted for exact custom-oscillator work removal and meaningfully
  better three-oscillator scaling.
