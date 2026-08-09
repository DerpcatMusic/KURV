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

