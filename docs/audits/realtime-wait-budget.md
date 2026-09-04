# Realtime pool: prevent the wait policy from expanding after slow jobs

Audit baseline: `d084681411a95803bb52206647c2bc881c4cbf8b`.

## Confirmed defect

`InternalRtPool::render_block_job` computes a nominal interval equal to 75% of
`job_samples / synth.sample_rate`, capped at 2 ms for exact saw jobs and 5 ms
otherwise. But two subsequent operations defeat the job-relative ceiling:

1. Calibration raises the interval to at least 2 or 5 ms.
2. The adaptive estimate can raise it to 16 ms, even when the job represents
   far less audio time. Observations include elapsed scheduling/wait time, so
   a scheduling stall can permit longer waits in future jobs.

These are source-derived policy values, **not measured stalls**:

| Job | Audio interval | Original nominal | Calibration exact / generic | Maximum adaptive |
|---|---:|---:|---:|---:|
| 64 frames, 48 kHz | 1.333 ms | 1.000 ms | 2 / 5 ms | 16 ms |
| 32 frames, 192 kHz | 0.167 ms | 0.125 ms | 2 / 5 ms | 16 ms |

The sample rate here is the synth's internal rate; host callback size and
oversampling matter. Do not call these values host callback deadlines.

## Change

Use the existing nominal interval for calibration and steady-state jobs. The
cost model still controls how many helpers participate, but cannot authorize
longer intentional waiting. Remove the adaptive wait calculation and its 16 ms
constant. Move the nominal policy into a dependency-free module so the exact
production function can be tested without building the GUI/plugin.

No DSP equations, atomics, claim ownership, cancellation, reduction order, or
voice-state publication are changed. No extra clock reads enter the render
loop. This is a defensive wait-policy correction, **not a claimed throughput
speedup**.

## Proof and validation

Run the tests against the actual production policy:

```sh
rustc --edition=2024 --test scripts/audits/rt_budget_tests.rs -o /tmp/kurv-rt-budget-tests
/tmp/kurv-rt-budget-tests
```

Executed with Rust 1.97.1: **3 passed, 0 failed**. The full plugin test suite
was not run because required private/path dependencies are unavailable in this
checkout. No callback-contention or audio-throughput benchmark was run.

Tests cover the two small-job cases above, all combinations of 0–512 selected
frame sizes and six sample rates, and equal audio intervals at 1x–4x rates.
There is no workload-cost input that can enlarge the production wait budget
any more. `transient_timeout_falls_back_exactly_and_recovers` in
`internal_rt_pool.rs` remains the existing full-engine regression for the
unchanged fallback state protocol.

## Limits and performance tradeoffs

This does **not** guarantee a callback deadline. Setup occurs before the timer;
a claimed voice can run for a whole job before checking the timer; reduction
and state copy-back also take time. On a timeout, serial fallback recomputes
work, and the shortened allowance can increase fallback frequency on loaded
machines. Heavy/offline rendering and small blocks therefore need integration
benchmarks before merging. A render that inherently exceeds the audio interval
cannot be made realtime by changing a wait constant.

Follow-up priorities:

- Measure successful and timed-out callback p50/p95/p99/max under helper
  preemption, including shadow preparation and serial fallback. Report glitches
  and fallback counts alongside average throughput.
- Pass a real remaining host callback budget into the scheduler, accounting for
  other plugin stages and fallback cost. Internal subjobs are not whole host
  callbacks.
- Evaluate cancellation/deadline checks between DSP chunks, with clock-read
  overhead measured. Incomplete rows must never publish `voice_ready`.
- Measure a strategy that finishes unclaimed work and avoids recomputing already
  completed voices while preserving exact serial state/output order. Do not
  mutate live voices from workers as a shortcut.
- Separate render-cost estimation from scheduler wakeup/wait latency; the
  current wall-clock estimate is not pure voice DSP cost.
