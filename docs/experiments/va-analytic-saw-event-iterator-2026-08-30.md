# Analytic saw BLEP event iterator (2026-08-30)

## Verdict

Reject and close sparse canonical BLEP scheduling.  Replacing the duplicate
phase walk and 64-byte schedule with an analytic event iterator preserves exact
output, but is still 90-186% slower for structural x8 and 14-40% slower for
scalar rendering.  The high-note x8 event union is fully dense.

Production DSP and version remain unchanged.

## Iterator

For each lane, the probe computes wrap crossings directly from the block's
initial phase and reciprocal step with `ceil((cycle - phase) / step)`.  It walks
crossings rather than samples and records candidate frames in one `u64` (8
bytes, no array).  A conservative six-frame neighborhood around each crossing
covers cumulative-f32 rounding at strict support boundaries.  At candidate
frames, the existing precomputed residual evaluator remains the final exact
lane mask and computes the unchanged fractional correction.  Ordinary raw-saw
evaluation and the real phase walk remain untouched.

The iterator is rebuilt from current phase and step at every 64-frame block;
there is no persistent state to become stale after a block-boundary pitch
change or phase reset.  No allocation, lock, I/O, or oscillator object growth
is introduced.

## Identity and CPU

Across wrap-adjacent initial phases and eight slightly detuned lanes, final
phase and stereo output peak error were exactly zero.  Release x86-64-v3,
20,000 blocks, best of five, nanoseconds per host frame:

| range | candidate frames | current x8 | analytic x8 | delta | current scalar | analytic scalar | delta |
|---|---:|---:|---:|---:|---:|---:|---:|
| low (`step=.0046`) | 6 / 64 | 2.485 | 4.732 | +90.39% | 1.937 | 2.204 | +13.79% |
| mid (`step=.041`) | 61 / 64 | 4.200 | 9.156 | +118.01% | 1.941 | 2.494 | +28.48% |
| high/cap-6 (`step=.083`) | 64 / 64 | 4.187 | 11.959 | +185.64% | 2.073 | 2.907 | +40.21% |

The low scalar case is the most favorable possible workload and still loses.
For x8, eight reciprocal/ceil crossing calculations dominate at low density;
at mid/high density they are pure overhead because the union of lane support
windows covers nearly every frame.  Narrowing the conservative neighborhood
would risk missing a correction when analytic multiplication and cumulative
f32 addition choose opposite sides of a strict support boundary.  Keeping the
existing residual as the exact final mask avoids that artifact but cannot
recover the iterator cost.

Exact command:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test analytic_saw_event_iterator_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

After one OS link-time SIGKILL, the disposable target was cleaned and the same
command passed 1/1 with 376 tests filtered out and the checkout's existing 25
test-build warnings.  The probe is retained only as failure evidence; runtime
code is unchanged.
