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

