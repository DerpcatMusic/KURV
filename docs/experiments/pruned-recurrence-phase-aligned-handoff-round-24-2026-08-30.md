# Pruned recurrence phase-aligned handoff, round 24 (rejected statically)

Date: 2026-08-30
Baseline: `2b502dbda8fe1225b712f041462ff0e65834f918`

## Verdict

Reject before implementing a Rust probe. Deferring the current-to-recurrence
switch until a phase where their values meet is not robust at actual sample
positions. Value crossings exist in continuous phase, but a 6.1 kHz oscillator
has only about eight reachable samples per cycle. The best reachable value is
often still far apart, and the first-difference mismatch at a good value sample
is large. Accounting for the current-renderer deltas emitted while waiting makes
the optimistic deferred policy worse than switching immediately for most
starting phases. Rapid parameter changes remove even the nominal one-cycle wait
bound, and arbitrary x8 lane phases cannot share one crossing without rendering
mixed families.

No renderer, ignored test, selector state, compile, timing run, production edit,
version bump, or push was made.

## Candidate and feasibility model

The proposed policy kept oscillator phase authoritative and rendered only the
current spline family while pending. At a matched departure sample it would
switch so the next sample came only from the exact pruned recurrence. Unlike
rounds 22 and 23, it needed neither a full-block crossfade nor a time-domain
correction.

The offline evaluator directly mirrored the scalar `SplineOptimized` BLEP and
BLAMP polynomials in `antialias.rs`, including support overlap and pulse-width
clamping. The recurrence side used the analytic f64 Fourier coefficients and
the exact legal harmonic cap. The broad scan covered:

- 4,096 uniformly offset starting phases;
- saw, square, triangle, and pulse widths 0.10, 0.20, 0.31, 0.50, 0.70, 0.90;
- 6.001, 6.1, 7.0, 7.999, 8.001, 10.0, 11.999, 12.001, 16, 20, and 23.999 kHz;
- cap-three, cap-two, and cap-one regions at 48 kHz;
- every sample reachable during one complete cycle.

For a phase `p`, the value error was

```text
e[p] = spline(p, step, width) - exact_cap(p, step, width)
```

and the discrete slope mismatch was `e[p] - e[p-step]`. The first optimistic
screen independently minimized absolute value error and
`max(abs(value error), abs(slope mismatch))` over all reachable samples. This is
strictly more informed than a cheap runtime detector or precomputed phase target.

## Joint-crossing result

At the required 6.1 kHz cap-three entry, the worst result across starting phases
after an oracle chose the best reachable sample in the next cycle was:

| shape | best-value error max | slope error at best-value max | best joint error max |
|---|---:|---:|---:|
| saw | 0.089980 | 0.314006 | 0.168663 |
| square | 0.120834 | 0.481798 | 0.283868 |
| pulse31 | 0.054002 | 0.489889 | 0.130696 |
| triangle | 0.021383 | 0.151719 | 0.043796 |

The value-only result looks promising for triangle and some pulse widths, but
not for a common four-shape backend. More importantly, its large local slope
error means waiting to that point emits an artifact before the nominal switch.
Cap-two and cap-one were less reliable. At 20 kHz, optimistic best-value maxima
were 0.3372 saw, 0.6741 square, 0.6517 narrow pulse, and 0.4293 triangle; best
joint maxima were 0.6517, 1.3034, 1.2590, and 0.8298.

## Whole-wait transition gate

A denser 16,384-phase seam scan included every adjacent-sample error produced
while waiting and the final current-to-exact boundary. For every starting phase,
an oracle selected the departure sample which minimized

```text
max(
    abs(e[n] - e[n-1]) for every current sample emitted while pending,
    abs(e[departure]) at the final boundary
)
```

At 6.1 kHz, the oracle deferred policy compared with immediate switching as
follows:

| shape | immediate worst | optimistic deferred worst | phases improved by waiting | extra search samples after first departure, 95% / max |
|---|---:|---:|---:|---:|
| saw | 0.288665 | 0.460290 | 11.9% | 0 / 0 samples |
| square | 0.362960 | 0.640268 | 14.4% | 0 / 0 samples |
| pulse31 | 0.445307 | 0.510004 | 24.6% | 1 / 1 samples |
| triangle | 0.152813 | 0.151719 | 20.8% | 1 / 1 samples |

The final column describes the oracle's best deferred candidate, excluding the
immediate switch. Zero means it departed on the first current sample. For saw
and square, searching farther into the cycle did not help. Including
the immediate option means the rational policy simply switches immediately for
roughly 75-88% of phases. Triangle's tiny worst-case improvement cannot rescue
the other shapes.

The 5.9 kHz exit used a forced cap-three recurrence against the destination
current renderer. It was similarly non-robust: waiting improved only 13.1% saw,
14.6% square, 25.4% pulse31, and 19.9% triangle phases. Optimistic deferred
worst errors were 0.4184, 0.5967, 0.4825, and 0.1420 respectively. Pulse-width
extremes did not establish a universal exception.

## Runtime policy and dynamic behavior

A value threshold requires evaluating both families at pending samples, which
reintroduces repeated recurrence setup or point evaluation. A precomputed target
phase avoids that work but depends on shape, cap, step, and effective pulse width;
the dense oracle above already gives it more information and still fails.

Pitch or pulse-width movement relocates the error crossings. A pending target
must therefore be discarded and recomputed on every change. With audio-rate or
block-rate movement, the switch can be postponed indefinitely, so the one-cycle
bound exists only for stationary parameters. A phase reset can jump over the
target and restarts the same arbitrary-phase problem.

The x8 structure is a separate rejection. Detuned lanes with arbitrary phase
reach different crossings on different samples. Switching lanes independently
requires both renderer families plus masking, lane repacking, or scalar fallback;
waiting for a common eight-lane crossing has no robust bound. Each option loses
the proposed one-family-only structural advantage.

## Decision

Continuous-phase crossings do not translate into robust sample-time handoffs at
these high frequencies. The dense oracle failed before runtime policy cost was
considered, so a 24/32-frame ignored probe would not add decision value. The
recurrence frontier remains useful only if both sides of a selector implement
the same sampled waveform family, or if the existing renderer itself is replaced
without a live backend seam.
