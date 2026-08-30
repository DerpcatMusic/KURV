# Single-Horner narrow-backend transition (2026-08-30)

## Verdict

The transition subproblem passes. Keep the implementation probe-only until the
three-frame offline publication backend exists, but do not reject this design:
interpolating the four active polynomial coefficients over 1,024 samples gives
the same signal as a dual-output linear fade to within `2.38e-7`, clears the
requested approximately `0.0011` step bound, handles rapid reversal without
extra history, and remains cheaper than shipping 2x even when forced active
continuously in the real structural path.

The phase is evaluated once and is identical for both endpoints. At each sample
the evaluator selects the same segment from the raw and projected curves,
linearly interpolates only `a/b/c/d`, then executes one Horner chain. It does
not build or copy a 64-coefficient curve on the audio thread. The temporary
scalar and AVX2 hooks were reverted because this branch does not yet contain a
real published cap-2/3/6 bank to connect them to.

## Equivalence, range, and transitions

The probe uses the cap-5.5 target selected from the cap-3/cap-6 frames and the
range-safe raw endpoint. It exhaustively checks 65,536 phases at 33 blend
positions. Because polynomial evaluation is linear in its coefficients, the
single-Horner result matches `lerp(raw.eval, projected.eval)` within f32
rounding:

| Shape | Peak error vs dual fade | Range over blend | Direct / 1024 step | Rapid-reversal step |
|---|---:|---:|---:|---:|
| saw | 1.79e-7 | [-1.000000, .999969] | .000976384 | .000773489 |
| square | 2.38e-7 | [-1.169309, 1.169309] | .000976394 | .000785232 |
| pulse37 | 2.38e-7 | [-1.166836, 1.094952] | .001048847 | .000821650 |
| triangle | 1.19e-7 | [-1.000000, 1.000000] | .000070653 | .000040054 |
| drawn | 1.79e-7 | [-.876415, .903136] | .000264012 | .000169098 |

All outputs were finite. Rapid reversal flips the target every 137 samples,
well before the 1,024-sample transition can finish. The state simply slews back
from its current coordinate; it neither restarts nor jumps.

The finite/gain contract remains explicit:

- ordinary `WaveCurveRt` stays clamped and unchanged;
- the raw endpoint is eligible only if every cubic extremum is finite and
  inside `[-1, 1]`, so its unclamped evaluation equals current 1x exactly;
- every offline projected frame and cubic extremum must be finite and at most
  `1.5` absolute; the measured global cap-2/3/6 bound is `1.372665`;
- coefficient interpolation is a pointwise output interpolation, so it cannot
  leave the convex range of its two evaluated endpoints and needs no clamp.

## Pitch, warp, and morph eligibility

One scalar selector coordinate spans raw, cap 6, cap 3, and cap 2. Adjacent
frames and raw-to-cap-6 use the same active-coefficient evaluator. A target
coordinate may jump on an abrupt pitch bend, reverse repeatedly, or return to
raw; the current coordinate moves by at most `1/1024` of the normalized
raw/projection transition per sample. The cap-frame differences are smaller
than the raw/projection worst case measured above.

Phase warp remains ineligible because applying a nonlinear phase map to an
already bandlimited curve creates new bandwidth. Partial custom/canonical morph
also remains ineligible until both endpoints have matching projected banks.
Enabling warp or partial morph targets the raw selector coordinate through the
same bounded slew. This bridge adds no phase discontinuity; any phase-warp
parameter smoothing remains the existing warp path's responsibility.

## CPU

Pinned x86-64-v3 release medians:

| Transition evaluator | Scalar ns/sample | x8 ns/sample/lane |
|---|---:|---:|
| dual output evaluation + lerp | 15.785 | 2.399 |
| active coefficients + one Horner | 12.159 | 2.575 |

Scalar saves about 23%. AVX2 is about 7% slower than the dual evaluator in
isolation because it still performs two sets of selector loads; it saves the
second Horner chain but not the gathers.

The candidate was then forced active on every sample in temporary real custom
structural paths, rather than only for its usual 1,024-sample transient:

| Workload | Current 1x instructions / cycles | Continuous bridge instructions / cycles | Shipping 2x instructions / cycles |
|---|---:|---:|---:|
| one lane, 20k x 64 | 2,174,020,918 / 849,152,952 | 2,199,890,212 / 867,892,883 | 2,630,359,788 / 985,998,110 |
| x8 unison, 10k x 64 | 1,081,252,000 / 435,034,000 | 1,106,217,268 / 450,981,568 | 1,279,898,274 / 506,166,621 |

Worst-case continuous transition costs 1.19% instructions and about 2.2%
cycles at one lane, and 2.31% instructions / 3.67% cycles at x8. It still uses
13.6% fewer instructions and 10.9% fewer cycles than shipping 2x in the x8
workload. Normal steady cap-frame evaluation does not pay this transition cost;
the bridge runs only while the selector coordinate is moving.

## Minimal production state

The bounded per-voice state is two `f32`s (8 bytes): current selector coordinate
and target selector coordinate. Endpoint frame indices and interpolation mix
are derived from the coordinate, so reversal needs no stored curve or output
history. The three immutable 256-byte frames remain publication state per
curve, not voice state. Evaluation is allocation-free, lock-free, stateless
apart from the two scalars, and has no signal latency; only representation
settling takes up to 1,024 samples.

Production integration still needs the offline compiler/publication work from
the prior narrow-backend round. Shipping the temporary hooks without that bank
would only benchmark fake endpoints, so no runtime source or version change is
retained here.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example coefficient_mip_lab --example process_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/coefficient_mip_lab
$lab transition
taskset -c 8 $lab bench

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  ./target/release/examples/process_lab 64 20000 1 custom 1 48000 1
```

The release probes built with the checkout's 83 existing warnings. Temporary
runtime bridge methods, call-site substitutions, and x8-unison configuration
were reverted before commit.
