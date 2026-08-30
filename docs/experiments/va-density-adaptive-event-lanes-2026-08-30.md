# Density-adaptive canonical residual lanes (2026-08-30)

## Verdict

Reject density-adaptive scalar residual evaluation.  Extracting one to three
active lanes and evaluating the same polynomial scalarly is not bit-identical
to the current packed `wide` implementation, despite matching coefficient and
`mul_add` order.  Mask extraction and scalarization are also several times
slower than always-SIMD.  Production DSP and version remain unchanged.

## Probe

The probe leaves the current f32x8 phase traversal untouched.  Inside each saw
wrap or pulse-edge residual, it extracts the exact lane event predicate.  At
popcount thresholds 1, 2, or 3 it evaluates only active lanes with the current
optimized inner/outer coefficients and scalar `f32::mul_add`; otherwise it
calls the existing precomputed SIMD residual.  Saw uses one correction, while
square and 37%-pulse use both wrap and width-edge corrections.  No state is
retained.

Low, mid, and high notes were exercised with coherent and decorrelated phases.
Coherent events have popcount eight and remain on the current SIMD path;
decorrelated phases exercise the intended scalar branch.  The active-lane
polynomial is independent of vector width, so its failed scalar equivalence is
the same prerequisite for x4 and x8.  An x4 structural implementation was not
timed after that shared exactness gate failed.

## Exactness failure

Coherent cases remained exact because they never selected scalar evaluation.
Decorrelated cases did not:

- low saw peak stereo difference after the 0.137 probe gain: `0.009706169`;
- low square: `0.012285367`;
- mid saw/square: approximately `0.01273367`;
- high threshold 2/3 saw: `0.012748748`.

Threshold 1 at high density can also appear exact simply because more than one
lane is active and the branch stays SIMD.  Increasing the threshold exposes
the same mismatch.  This is not a phase error (all final phase peaks were zero)
but a residual value difference between scalar `mul_add`/selection semantics
and the packed implementation.  Approximate equivalence is unacceptable for a
replacement whose purpose is only CPU reduction.

## CPU rejection

Even ignoring the identity failure, converting phase/step masks to scalar
arrays, counting lanes, branching, and rebuilding a vector dominates the
current short SIMD polynomial.  Representative release x86-64-v3 results:

| workload | threshold | current ns/frame | adaptive ns/frame | delta |
|---|---:|---:|---:|---:|
| low decorrelated saw | 1 | 2.635 | 18.506 | +602.37% |
| low decorrelated square | 1 | 5.040 | 33.056 | +555.83% |
| mid decorrelated saw | 1 | 5.193 | 22.371 | +330.75% |
| high decorrelated saw | 1 | 5.945 | 31.454 | +429.05% |
| high decorrelated square | 1 | 13.474 | 41.421 | +207.41% |

Thresholds 2 and 3 did not rescue CPU and selected more non-identical scalar
frames.  Square/pulse pay extraction twice because they have two discontinuity
edges.  Pitch/reset transitions do not add state—the decision is per frame—but
there is no reason to benchmark them after both exactness and static CPU gates
fail.

Exact command:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test adaptive_event_lane_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

The corrected probe passed 1/1 with 377 tests filtered out and the checkout's
existing 25 test-build warnings.  The ignored probe is retained as failure
evidence only; no runtime path or retained state was added.
