# Procedural custom-curve antialiasing experiment — 2026-08-06

## Decision

Do not add either tested custom-curve antialiaser. The strongest candidate was
genuinely clean, but it was materially more expensive than KURV's current 2x
custom path on both portable x86-64 and x86-64-v3. No DSP source was retained.

The editable waveform remains procedural: this experiment did not use a
wavetable, FFT, additive synthesis, allocation, locks, or an event queue.

## Correct event model

`WaveCurveData::compile_rt` does not send the user's uneven Hermite segments
directly to the oscillator. It resamples them at eight uniform phases, solves a
periodic cubic B-spline, and stores eight cubic segments in `WaveCurveRt`.
Consequently the realtime curve is periodic and C2 continuous:

- value is continuous at every knot;
- first and second derivatives are continuous;
- the first discontinuity is in the third derivative, where adjacent cubic
  coefficients change.

A BLEP for a value jump or an ordinary BLAMP for a slope jump therefore targets
events this curve does not have. The credible local correction is a
higher-integrated BLAMP/PTR acting on each third-derivative jump. This is an
engineering generalization of the cited BLEP/BLAMP/PTR methods, not a result
claimed verbatim by those papers.

## Baseline and method

Both portable and v3 release binaries came from `192fcd4`. The existing
`generator_lab` was used with `KURV_LAB_CUSTOM=1`; no test files were added or
changed. Quality renders contained 65,536 samples after 16,384 warm-up frames
at coherent FFT bins 89, 601, 4806, and 7000. The default editable curve was
used at 100% mix with no phase warp.

The direct 1x and current Spline/Normal 2x baselines were:

| FFT bin | Direct 1x unwanted energy | Current 2x unwanted energy |
|---:|---:|---:|
| 89 | -152.25 dBc | -142.01 dBc |
| 601 | -123.82 dBc | -141.96 dBc |
| 4806 | -65.07 dBc | -95.78 dBc |
| 7000 | -43.07 dBc | -85.89 dBc |

The apparently lower 1x floor at bin 89 is numeric noise, not a meaningful
advantage over 2x. The high-note rows are the useful stress cases.

## Candidate A: exact cubic interval average

Each cubic segment has an exact quartic antiderivative. The first prototype
averaged the continuous curve over one centered phase-step interval, splitting
only at uniform segment boundaries and cycle wrap. This is the oscillator-side
analogue of first-order antiderivative antialiasing and preserves the procedural
curve exactly inside each integral.

| FFT bin | Exact average 1x | Gain versus direct 1x |
|---:|---:|---:|
| 89 | -112.21 dBc | numeric-floor regression |
| 601 | -126.06 dBc | 2.24 dB |
| 4806 | -71.16 dBc | 6.09 dB |
| 7000 | -47.47 dBc | 4.40 dB |

It missed the 12 dB quality gate and remained about 38 dB behind current 2x at
the highest bin. Even before a dense SIMD implementation, the exact scalar
path increased retired instructions by 52.7% in the 24-note, three-oscillator,
one-lane cell. It was rejected.

## Candidate B: third-derivative knot PTR

The stronger prototype detected each uniform cubic knot, computed the jump in
the global cubic coefficient analytically, and replaced the local truncated
cubic with a C4 polynomial transition. For normalized knot distance `x` in
`[-1, 1]`, the interior polynomial was:

```text
P(x) = -5/256 + 15/64 x^2 + 1/2 x^3 + 45/128 x^4
       -5/64 x^6 + 3/256 x^8
```

It matches the left and right truncated-cubic value and first four derivatives
at the transition boundaries. The correction was scaled by the exact adjacent
cubic-coefficient jump and the cube of transition width. Scalar, SIMD4, and a
fused AVX2/FMA SIMD8 evaluator were prototyped. Common narrow transitions used
one analytically selected knot per lane; wider high-note transitions used the
two adjacent knots without allocation or stored event state.

With a two-phase-step transition, quality easily cleared the requested 12 dB:

| FFT bin | Knot PTR 1x | Gain versus direct 1x | Difference versus current 2x |
|---:|---:|---:|---:|
| 89 | -152.23 dBc | neutral | 10.22 dB cleaner |
| 601 | -145.62 dBc | 21.81 dB | 3.66 dB cleaner |
| 4806 | -90.34 dBc | 25.27 dB | 5.44 dB worse |
| 7000 | -88.26 dBc | 45.20 dB | 2.37 dB cleaner |

This is the best new signal result from the round. It failed decisively on CPU.
Retired instructions below use the identical 24-note x 64-unison x
three-oscillator workload and include identical warm-up work:

| Build | Direct custom 1x | Current custom 2x | Knot PTR 1x | PTR versus 1x | PTR versus 2x |
|---|---:|---:|---:|---:|---:|
| x86-64-v3 | 1.565B | 3.144B | 5.656B | **+261.5%** | **+79.9%** |
| portable x86-64 | 17.136B | 34.309B | 82.981B | **+384.2%** | **+141.9%** |

The existing AVX2 curve evaluator is exceptionally small: four lane permutes
and a cubic Horner chain. Event selection, two coefficient permutes, division,
masking, jump scaling, and an eighth-degree transition therefore multiply its
cost even when fused into the same evaluator. On the portable build the
per-lane selection is worse still.

A one-phase-step transition reduced the active region but retained the vector
polynomial cost. Quality gain collapsed to only 3.22 dB at bin 4806 and 1.72 dB
at bin 7000, while the dense v3 instruction cost remained roughly 3.5x direct
1x. Lower polynomial order saves only a few vector operations and cannot close
that gap.

## Why this is not an optional quality mode

The two-step PTR is not a cheaper substitute for oversampling: it costs 80%
more than current 2x on v3 and 142% more on the portable target while losing
5.4 dB to 2x in one high-note cell. Phase warp and moving curves would add event
trajectory work and were not pursued after the unwarped CPU gate failed.

Changing compilation to a C3/C4 spline could reduce third-derivative events at
essentially no render cost, but it would change the user's cubic shape rather
than antialias it. That is a separate sound-design experiment, not a transparent
DSP optimization.

## Primary-source grounding

- Ambrits and Bank's EPTR paper shows that polynomial transition regions win
  when transition detection and the trivial waveform can share extremely small
  arithmetic. KURV's eighth-degree, coefficient-selected third-derivative
  transition does not have that classical-saw cost structure:
  [Improved Polynomial Transition Regions Algorithm for Alias-Suppressed Signal Synthesis](http://home.mit.bme.hu/~bank/publist/smc13.pdf).
- Esqueda, Välimäki, and Bilbao derive polynomial BLAMP correction from the
  size and fractional location of a derivative discontinuity. They explicitly
  note that further integration can target higher-derivative discontinuities,
  but leave that direction for future work:
  [Rounding Corners with BLAMP](https://dafx.de/paper-archive/2016/dafxpapers/18-DAFx-16_paper_33-PN.pdf).
- Bilbao, Esqueda, Parker, and Välimäki establish antiderivative
  antialiasing's divided-difference method and its higher-order cost/accuracy
  tradeoff for memoryless nonlinearities. Exact cubic interval integration here
  is a KURV-specific application of that principle:
  [Antiderivative Antialiasing for Memoryless Nonlinearities](https://www.research.ed.ac.uk/en/publications/antiderivative-antialiasing-for-memoryless-nonlinearities/).

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-custom-base-v3 \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features --example generator_lab

KURV_LAB_CUSTOM=1 generator_lab render splineopt 1 saw 7000 65536 /tmp/custom.f32 0.5
python3 scripts/analyze-generator.py /tmp/custom.f32 saw 7000 65536

KURV_LAB_CUSTOM=1 taskset -c 8 perf stat -r 3 -e instructions:u -- \
  generator_lab bench splineopt 1 saw 64 65536 1 69 0.5 0 0.7 24 noise 3
```

The PTR and exact-average commands require the rejected substitutions described
above; they are intentionally absent from the branch. Machine-local binaries,
renders, and `perf` outputs remain under `/tmp/kurv-custom-*` for this session.
