# Constant-x8 returned-sample kernel prerequisite (2026-08-30)

## Verdict

The seam is functionally correct but independently slower, so do not retain it
in production.  A single phase-owning callback can expose ungained x8 samples
from saw, square, custom, and warped fallback generation with bit-identical
phase, gain, and output.  It also gives a coefficient selector one common place
to mix every eligibility exit.  On the measured release workload it regressed
all four existing paths by 0.21% to 5.49%, failing the prerequisite gate before
any coefficient backend cost is added.

Production DSP and version remain unchanged.  The ignored probe is retained so
the deeper-kernel alternative has exact evidence rather than an architectural
assumption.

## Probe and ownership

`probe_sample_kernel` loads the eight oscillator phases into one `f32x8`, owns
the wrap/advance loop, calls a monomorphized generator for the current ungained
sample, and immediately performs the existing left/right `mul_add`.  It writes
the final phases back once per 64-frame block.

There is no heap state, object growth, returned sample block, temporary left or
right buffer, allocation, lock, I/O, or analysis.  Its only explicit live state
is the 32-byte phase vector.  A future selector could consume both current and
projected samples inside this callback before gain accumulation, including on
custom/warp/general-shape exits; therefore the seam solves the stale-state
ownership problem from the previous round.

## Identity and CPU

The release probe used eight unequal phase steps and stereo gains at the cap-6
crossover, 64-frame blocks, 20,000 blocks per timing sample, and the best of
five runs.  Both baseline and candidate use the same current generator math.

| path | current ns/frame | sample-kernel ns/frame | delta | phase peak | stereo output peak |
|---|---:|---:|---:|---:|---:|
| saw | 4.390 | 4.494 | +2.38% | 0 | 0 |
| square | 7.936 | 8.372 | +5.49% | 0 | 0 |
| 50% custom/saw | 6.876 | 6.891 | +0.21% | 0 | 0 |
| phase-bent saw | 7.837 | 8.010 | +2.21% | 0 | 0 |

The result shows the compiler can inline the callback sufficiently for exact
output, but the unified loop loses small specialization advantages of the
dedicated accumulators.  Square is the clearest rejection at +5.49%; treating
the +0.21% custom result as noise would not rescue the required all-exit seam.

Exact command:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test constant_x8_sample_kernel_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

The command passed 1/1 with 374 tests filtered out and the checkout's existing
25 test-build warnings.

## Consequence

Do not refactor the current accumulators merely to host the narrow coefficient
backend.  A future attempt needs a zero-overhead interception mechanism inside
the already-specialized saw/pulse/custom/warp loops (for example a compile-time
sample transform proven to optimize away when inactive), and must first match
each current loop independently.  The generic common kernel is closed.
