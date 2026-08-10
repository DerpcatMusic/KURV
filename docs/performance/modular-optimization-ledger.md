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
