# Sparse canonical BLEP block schedule (2026-08-30)

## Verdict

Reject the sparse block schedule.  It is bit-identical and allocation-free,
but schedule construction costs much more than the current per-frame SIMD event
masks—even at a low step where only 4 of 64 frames contain any x8 BLEP event.
Production DSP and version remain unchanged.

## Probe

For each stable 64-frame block, the probe duplicates the exact f32 phase walk
from the current constant-x8 saw renderer and records a 64-byte boolean schedule
for frames where any lane is inside the two-sample spline-BLEP support.  The
render pass advances phase normally, calls the existing precomputed residual
evaluator only on scheduled frames, and otherwise emits the raw saw directly.
Active mask, support, and inverse step remain block-hoisted exactly as in the
current kernel.

The scalar comparison uses the same scheme in f64.  Both schedules are rebuilt
from the current phase and step every block, so a phase reset or block-boundary
pitch change has no persistent/stale schedule state.  There are no allocations,
locks, I/O, or retained oscillator fields.

## Identity and event density

Eight slightly detuned lanes began around the wrap boundary to exercise reset
and support edges.  Across all ranges, final phase and stereo output peak error
were exactly zero in f32.  Event density rises rapidly because the union of
eight independently phased lane supports is not sparse:

| range | normalized base step | x8 frames with any event |
|---|---:|---:|
| low | 0.0046 | 4 / 64 |
| mid | 0.041 | 55 / 64 |
| high / cap-6 region | 0.083 | 64 / 64 |

Thus the target high-note region has no empty x8 frames to skip.

## CPU

Release x86-64-v3, 20,000 blocks, best of five, nanoseconds per host frame:

| range | current x8 | scheduled x8 | delta | current scalar | scheduled scalar | delta |
|---|---:|---:|---:|---:|---:|---:|
| low | 2.489 | 3.711 | +49.08% | 1.960 | 3.723 | +90.00% |
| mid | 4.324 | 7.302 | +68.88% | 1.991 | 3.828 | +92.32% |
| high | 4.219 | 6.819 | +61.63% | 2.098 | 3.911 | +86.42% |

The current evaluator already hoists its divide and returns immediately after
cheap vector comparisons on empty frames.  An exact schedule must reproduce
the phase walk before rendering, then read/branch on schedule state during the
real walk.  Avoiding residual calls cannot recover that duplicate work.  A
formula-derived event index might remove the first walk, but would not be
bit-identical to cumulative f32 phase at support boundaries and cannot help the
dense high-note x8 case.

Exact command:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test sparse_saw_event_schedule_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

The command passed 1/1 with 375 tests filtered out and the checkout's existing
25 test-build warnings.  The ignored probe remains as the self-contained
failure record; no runtime code is retained.
