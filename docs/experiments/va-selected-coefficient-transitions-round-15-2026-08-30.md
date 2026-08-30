# VA round 15: selected-coefficient curve transitions

## Verdict

Rejected. Interpolating only the coefficients selected by an evaluation can be
bit-identical to first synthesizing the full `WaveCurveRt`, but it loses badly
once the same host-frame curve is evaluated by x4/x8 render lanes or more than
a very small scalar duty. KURV currently pays full interpolation once per host
frame and reuses that curve across voices/unison lanes. Moving interpolation
inside every evaluation destroys that amortization and would require a new
curve-pair representation through settings and render state.

Production remains 0.8.8 from round 14. No state, lifetime, publication, clamp,
transition, or evaluator changes survive this experiment.

## Semantics

The candidate stores references to previous/current curves plus one mix. For
each evaluated segment it applies the exact existing coefficient operation:

```text
(current - previous).mul_add(mix, previous)
```

It then uses the shipping scalar, portable x4, or portable x8 Horner and clamp
ordering. Across 64 curve pairs, five transition positions, every uniform
boundary, and random phase vectors:

- total checked outputs: 251,200
- scalar failures: 0
- x4 failures: 0
- x8 failures: 0

Evaluating the two endpoint curves separately and blending their already
clamped outputs is not equivalent: 142 boundary checks differed. That shortcut
changes clamp/rounding semantics and was rejected without further benchmarking.

## Single-use cost

Release-mode nanoseconds per output sample, including selector and interpolation:

| path | full 64-coefficient interpolation | selected coefficients |
|---|---:|---:|
| scalar | 125.248 | 27.997 |
| x4 | 32.582 | 25.386 |
| x8 | 16.296 | 23.190 |

Selected interpolation helps a single scalar/x4 use, but already loses for one
portable x8 use because it repeats coefficient interpolation across eight lanes.

## Real 64-frame amortization

The block probe advances phase on every evaluation and tests 1, 4, and 16
evaluations per frame. Each pair is full/selected, in nanoseconds per 64-frame
block.

| lanes | duty 1 | duty 4 | duty 16 |
|---|---:|---:|---:|
| scalar | 8004.7 / 812.1 | 8844.7 / 3206.7 | 13167.4 / 12860.1 |
| x4 | 8179.6 / 7383.4 | 9971.7 / 29816.4 | 18767.1 / 117978.9 |
| x8 moving | 8046.6 / 13716.2 | 11282.3 / 54736.7 | 24325.6 / 219172.4 |
| x8 static | 8578.7 / 14024.7 | 11209.0 / 56143.0 | 24904.4 / 219918.5 |

The isolated scalar case remains attractive only while very few samples consume
the morph. The actual VA route publishes one selected curve into settings and
may consume it across multiple voices and unison groups, predominantly via x8
and x4 block kernels. Full interpolation therefore wins at the structural duty
that matters.

There are also up to three distinct full operations before rendering: custom
curve fade, interpolation between VA table frames, and transition between old
and new tables. Deferring these would require carrying a composite/nested morph
through `OscillatorSettings`/`OscillatorDspSettings`, not a minimal evaluator
change, and could multiply selected interpolation work further.

## Commands

```text
cargo fmt --all
cargo test --release \
  wave_curve::compiler_experiment::selected_coefficient_transition_report \
  -- --ignored --nocapture
```

Passed 1/1 with zero full-versus-selected coefficient failures.

```text
cargo test --release wave_curve::topology_tests --locked
```

Passed 3/3. Existing production topology remains unchanged.

