# Canonical x8 reciprocal hoist (round 15, 2026-08-30)

## Change under test

Round 14 found one `vdivps` in the support-3/degree-7 Estrin probe. This test-only revision computes `1 / step` once before the block loop and multiplies by it at each event. The probe's eligibility is intentionally a stable step: a future runtime path would have to update and publish `step` and its reciprocal together at every step boundary. Production code is unchanged.

## Build, disassembly, and tolerance

The existing target was rebuilt once, serialized, after memory pressure fell:

```text
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native' \
  cargo test canonical_x8_symbol_probe --lib --release --locked --no-run

llvm-objdump --no-show-raw-insn -d \
  --disassemble-symbols=kurv_probe_support3_x8_blocks \
  target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce

CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native' \
  cargo test support3_reciprocal_matches_division --lib --release --locked -- --nocapture
```

Disassembly now contains one scalar `vdivss` in setup and no vector divide in the hot loop. Event positions use `vmulps`; the prior FMA/mask schedule is otherwise retained. Symbol size grows from 911 to 948 bytes. The tolerance test covers 20,480 phase/frequency cells from 110 Hz through 12 kHz and bounds reciprocal-multiply position error to `2 * f32::EPSILON * max(abs(reference), 1)`. It passes in native release.

## Identical 7.04 kHz counters

The round-14 command was repeated at 200,000 blocks and five runs:

| kernel | cycles | instructions | branches | branch misses | L1 loads | L1 misses |
|---|---:|---:|---:|---:|---:|---:|
| current cubic/support-2 | 241,905,212 +/-0.75% | 600,607,520 | 56,611,298 | 78,210 | 79,010,955 | 256,792 |
| support-3 reciprocal | 221,158,982 +/-1.41% | 581,889,829 | 26,573,818 | 18,245 | 52,483,046 | 79,305 |

This short cell favors support-3 by 8.6%, but the longer cell below shows that 7.04 kHz is too close to call; it is not used as the crossover boundary.

## High-note sweep

Each cell uses 400,000 64-frame x8 blocks, five `perf stat` repetitions, pinned to CPU 8. Values are cycles; percentages are perf run-to-run variation.

| frequency | current | support-3 reciprocal | delta |
|---:|---:|---:|---:|
| 3,520 Hz | 404,006,373 +/-1.05% | 442,275,409 +/-0.64% | +9.5% lose |
| 7,040 Hz | 468,346,764 +/-0.48% | 466,022,093 +/-1.99% | -0.5% parity |
| 8,000 Hz | 491,373,561 +/-0.66% | 446,531,052 +/-2.62% | -9.1% win |
| 9,000 Hz | 514,442,897 +/-1.05% | 463,612,383 +/-3.78% | -9.9% win |
| 10,000 Hz | 570,872,146 +/-4.40% | 446,736,638 +/-2.35% | -21.7% win |
| 11,000 Hz | 589,588,609 +/-2.55% | 456,751,565 +/-3.29% | -22.5% win |
| 12,000 Hz | 666,586,485 +/-1.10% | 444,956,276 +/-1.22% | -33.2% win |

At 7.04 kHz the longer run has the same 1.198 B versus 1.161 B instruction relation as round 14 but cycle parity, confirming that the earlier short-cell lead is not a safe gate. From 8 kHz upward the measured margins exceed run variation in every cell.

## Verdict

The assembly path remains closed: the only code-generation opportunity was solved in ordinary Rust, and rustc emits the intended off-loop scalar divide plus hot-loop vector multiply.

The high-note crossover question is reopened, narrowly. Support-3 is not suitable at 3.52 kHz and is inconclusive at 7.04 kHz, but is reproducibly faster in this isolated structural x8 probe from 8 kHz through 12 kHz. A subsequent round may test conservative `>= 8 kHz`, stable-step crossover gating, transition behavior, actual voice/unison routing, and full-instrument duty cycles. This record does not promote the candidate: no production integration, runtime state, publication bytes, or audio-thread behavior changed.

Limitations: quality here establishes only reciprocal-versus-division numerical equivalence; the support-3 kernel's acoustic quality advantage comes from round 11. This is still a canonical saw x8 probe rather than a full-host workload.
