# Low-unison final optimization, 2026-08-06

Baseline: `816913d`, Spline 4PT / Normal 2x, 24 held notes, pooled block renderer.
Measurements used release binaries compiled separately for portable `x86-64` and
`x86-64-v3`.

## Retained change

Exactly one unison lane now renders directly into mono samples before oscillator
level/pan and secondary mixing. It bypasses SIMD accumulator initialization,
stereo distribution loads, lane reduction, and normalization that are identities
for one lane. Jitter remains inactive for one lane, as it cannot move relative to
a stack. The normal fixed-capacity voice storage is unchanged.

Retired instructions over 524,288 frames:

| Target | Oscillators | Baseline | Optimized | Change |
|---|---:|---:|---:|---:|
| x86-64 | 1 | 5,407,524,337 | 4,514,888,657 | -16.51% |
| x86-64 | 3 | 13,773,333,501 | 10,897,910,920 | -20.88% |
| x86-64-v3 | 1 | 3,600,160,699 | 3,257,907,285 | -9.51% |
| x86-64-v3 | 3 | 9,449,318,711 | 8,052,322,728 | -14.78% |

At one lane, enabling Noise or Sine changed the optimized retired-instruction
count by less than 0.01%. The unchanged 4/8/64-lane paths regressed by at most
0.80% on portable x86-64 and 0.51% on x86-64-v3, below the 2% rejection gate.

## Output verification

Baseline and optimized checksums matched exactly for:

- all four waveform endpoints;
- custom and canonical curves;
- NONE, PWM, BEND, and HARM phase-warp modes;
- one and three enabled oscillators;
- static and continuously morphing shapes; and
- jitter off, Noise, and Sine.

Active phase warp remains in the existing generic warped renderer. Debug
assertions prevent it from entering the direct unwarped kernel.

## Rejected experiment

Moving the two/four-lane tail helpers out of cold placement was rejected. It was
inconsistent across CPU targets and regressed representative dense cases by up
to roughly 10%, so no two-lane specialization is included.
