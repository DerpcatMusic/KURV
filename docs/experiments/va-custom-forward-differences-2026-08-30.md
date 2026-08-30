# VA custom cubic forward-difference experiment — 2026-08-30

## Verdict

Reject production integration. Cubic forward differences reproduce the stable
curve to within one float ULP per lane, but every tested scalar, time-packed,
x4, and x8 layout costs more than KURV's existing Horner evaluator. The current
AVX2/FMA evaluator is too small for segment tracking, resynchronization, reload,
and recurrence state to amortize. Production DSP is unchanged.

This is only a CPU experiment. Forward differences do not antialias the curve
and make no quality claim beyond matching the current raw evaluator.

## Candidate

For a stable curve and phase step, each cubic segment is converted at the exact
current phase into sample value and first, second, and third forward
differences. Each subsequent sample uses three additions. The exact Horner path
reloads whenever the lane enters another of the sixteen segments. A 16- or
32-sample countdown also forces Horner resynchronization to bound `f32` drift.
Output retains the current `[-1, 1]` clamp.

The probe includes:

- scalar per-lane recurrence;
- real cross-lane x4 and x8 recurrence state;
- scalar time packs of four and eight future samples;
- the complete lane mix and KURV output-delay path;
- actual shipping 2x evaluation and decimation as a second baseline.

All state is fixed and allocated before timing. Timed processing has no
allocation, locking, blocking, logging, or I/O. The optimization is eligible
only for a constant curve, constant phase step, and unwarped phase. Curve
morph, pitch modulation, sync, FM/PM, or warp would fall back to the unchanged
Horner evaluator.

| Layout | State | Per lane |
|---|---:|---:|
| Scalar recurrence | 20 B | 20 B |
| Time pack 4 | 48 B | 48 B |
| Time pack 8 | 64 B | 64 B |
| x4 recurrence | 80 B | 20 B |
| x8 recurrence | 160 B | 20 B |

## Output bounds

Ten-million-frame one-lane comparisons used the exact same phases and current
`WaveCurveRt::default()` evaluator.

| FFT bin | x8/32 max absolute | RMS | Segment-boundary max | Peak delta | Max-step delta |
|---:|---:|---:|---:|---:|---:|
| 1 | `5.960e-8` | `4.931e-9` | `0` | `0` | not measurable |
| 89 | `1.192e-7` | `2.760e-8` | `1.490e-8` | `0` | `2.980e-8` |
| 601 | `5.960e-8` | `2.506e-8` | `2.980e-8` | `0` | `0` |
| 7000 | `0` | `0` | `0` | `0` | `0` |

At bin 7000 every sample enters a different segment, so every output is an
exact Horner reload and is bit-identical. That is also the worst performance
case. Scalar, time4, time8, x4, and x8 checks all stay below `2e-5` over five
pitches and 262,144 frames; the measured one-lane maximum is one ULP.

The 65,536-sample coherent render comparison found no peak change at bins 89,
601, or 7000. Maximum adjacent-step difference is one half-ULP at bin 89 and
zero at the mid/high cells. There is no boundary click or spectral change large
enough to distinguish from normal `f32` evaluation order.

## CPU

Release build used `-C target-cpu=x86-64-v3`, one pinned CPU, 500,000 host
frames, and three `perf stat` repetitions. The table gives 64-lane full-path
candidate ratios. Raw1 is the optimization target; raw2 is the shipping default.

| Bin | Best recurrence | Versus raw1 instr/cycles | Versus shipping 2x instr/cycles |
|---:|---|---:|---:|
| 89 | scalar/16 | 5.41x / 3.97x | 2.48x / 1.73x |
| 601 | scalar/16 | 5.78x / 4.23x | 2.65x / 1.93x |
| 7000 | scalar/16 | 9.63x / 9.46x | 4.41x / 4.19x |

The best real x8 low-pitch layout is the 32-sample resync mode. At bin 89 it
retires 1,662,030,687 instructions and 423,840,774 cycles, versus raw1's
300,767,892 and 108,914,746. At bin 601 x8/16 retires 2,165,283,753 and
651,323,999 versus raw1's 300,768,074 and 115,691,602. At bin 7000 it retires
3,773,489,813 and 1,277,568,146 versus raw1's 300,767,932 and 114,461,050.

Time packing does not help. At 64 lanes/bin 89, time4 and time8 retire about
2.477B instructions and 0.81–0.88B cycles. At bin 7000 both approach 3.83B
instructions and 1.40B cycles. The output buffers add state while refill still
needs segment checks and exact reloads.

Even at one lane and bin 1, scalar recurrence retires 22% more instructions and
8% more cycles than raw1. The three-add recurrence cannot repay its segment and
countdown branches against four AVX2/FMA Horner operations. Cross-lane phase
dispersion makes partial reloads more frequent and adds extraction/repacking;
time packing shifts the same work into bursts without removing it.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example forward_diff_curve_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/forward_diff_curve_lab
$lab check
$lab drift fd8-32 89 1 10000000
$lab render fd8-32 89 65536 /tmp/fd-89.f32

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench raw1 89 64 500000 1
taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench fd8-32 89 64 500000 1
```

The release build completed with the checkout's 83 existing warnings and no
experiment error. Formatting, long drift checks, scalar/time/x4/x8 checks,
coherent artifact comparisons, and diff checks passed. Production source,
Cargo metadata, and dependencies remain unchanged.
