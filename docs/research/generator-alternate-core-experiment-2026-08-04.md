# Generator alternate-core experiment — 2026-08-04

## Verdict

The genuinely different winner on **signal fidelity** was a stateless, symmetric,
16-sample Hann-windowed sinc BLEP/BLAMP with a 16x interpolated lookup table.
It is substantially more faithful than Lagrange and usually much cleaner, but it
does **not** meet KURV's CPU objective for SIMD unison. Do not land it as the
production default.

At 439.82 Hz, 48 kHz host rate:

| Path | Rate | Saw alias residual | Wanted magnitude RMS, <=20 kHz | Peak |
|---|---:|---:|---:|---:|
| Spline | 1x | -45.413 dBc | 4.663 dB | 0.969 |
| Lagrange | 1x | -36.078 dBc | 1.415 dB | 1.065 |
| sinc16 | 1x | -41.592 dBc | **0.010 dB** | 1.159 |
| Spline | 2x | -85.346 dBc | 0.833 dB | 1.115 |
| Lagrange | 2x | -70.882 dBc | **0.202 dB** | 1.175 |
| sinc16 | 2x | **-90.710 dBc** | 0.299 dB | 1.181 |

The sinc16 1x saw alias improvement over Lagrange was 5.5 dB at 440 Hz,
16.4 dB at 1.83 kHz, 26.8 dB at 4.39 kHz, and 13.1 dB at 8.79 kHz. At 2x
the gains were about 20 dB through 4.39 kHz, but sinc16 regressed 2.3 dB
against Lagrange at the 8.79 kHz endpoint. The larger peak is the expected
Gibbs overshoot of a sharper bandlimited edge; it needs headroom, not clipping.

The ideal saw reference used here has harmonic magnitude proportional to
`1 / harmonic_number` (-6 dB/octave), not a flat spectrum.

## CPU result

Ryzen 7 7800X3D, `-C target-cpu=x86-64-v3`, one pinned core, release build,
100,000 frames, five inner repeats and five alternating outer runs. Numbers are
median ns per host frame for sinc16 versus Lagrange.

| Note / rate / shape | 1 lane | 8 lanes | 64 lanes |
|---|---:|---:|---:|
| A4 / 1x / saw | 20.406 -> 21.210 (**+3.9%**) | 26.018 -> 37.854 (**+45.5%**) | 75.157 -> 154.831 (**+106.0%**) |
| A4 / 2x / saw | 44.507 -> 46.931 (**+5.4%**) | 50.562 -> 73.576 (**+45.5%**) | 127.883 -> 285.587 (**+123.3%**) |
| note 108 / 1x / saw | +25.0% | +31.0% | +69.1% |
| note 108 / 2x / saw | +18.9% | +33.7% | +75.0% |
| A4 / 1x / pulse | +16.0% | +50.5% | +111.9% |
| A4 / 1x / triangle | +22.4% | +69.4% | +133.9% |

AVX2 gather cut the first scalar-per-lane lookup attempt roughly in half, but
the remaining gap is structural: wider support makes at least one lane active
in most eight-wide vectors, then BLEP/BLAMP needs multiple indexed reads.
The non-AVX2 scalar fallback was much worse.

The candidate itself is RT-safe and deterministic: two immutable 257-float
tables (2,056 bytes total), no allocation, no locks, and no persistent DSP
state. Its AVX2 gather block validates/clamps every index to `0..255` before
reading the 257-element table.

## Narrow-pulse overlap

For pulse width 0.05, the wider sinc correction handled overlapping transition
regions much more accurately than simply allowing the existing polynomial BLEPs
to overlap.

| Path | Rate / frequency | Alias residual | Wanted magnitude RMS, <=20 kHz | DC error |
|---|---:|---:|---:|---:|
| Lagrange | 1x / 440 Hz | -32.65 dBc | 1.394 dB | 0.0000 |
| Spline | 1x / 440 Hz | -41.97 dBc | 4.600 dB | 0.0000 |
| sinc16 | 1x / 440 Hz | -36.80 dBc | **0.010 dB** | 0.0000 |
| Lagrange | 1x / 1.83 kHz | -24.12 dBc | 1.243 dB | 0.0000 |
| sinc16 | 1x / 1.83 kHz | **-40.20 dBc** | **0.008 dB** | 0.0000 |
| Lagrange | 2x / 440 Hz | -64.06 dBc | **0.202 dB** | -0.0053 |
| sinc16 | 2x / 440 Hz | **-85.08 dBc** | 0.298 dB | -0.0053 |

This does not solve KURV's separate high-pitch pulse-width clamp. At 1x the
0.05-duty DC error was still 0.0831 at 4.39 kHz and 0.2662 at 8.79 kHz for all
algorithms. The oscillator is cleaner, but the requested duty is still being
changed.

## Variants tried and rejected

- **sinc4:** cheaper support, but at 2x/440 Hz saw alias was -68.642 dBc,
  worse than Lagrange's -70.882 dBc. It did not earn a new mode.
- **sinc8:** useful middle fidelity point: 1x/440 saw -38.478 dBc and 0.029 dB
  wanted error; 2x/440 saw -81.290 dBc and 0.299 dB. CPU still rose 27.3%
  at 8 lanes and 88.7% at 64 lanes for 2x saw.
- **Sparse scalar lookup below two active lanes:** 7-26% slower than always
  using AVX2 gather due to mask counting, branches, and scalar reconstruction.
- **Compact linear table, 2 points/sample:** saved table size but lost about
  23 dB of 2x alias rejection. Rejected.
- **Compact cubic-Hermite table, 2 points/sample:** recovered sinc8 quality to
  about 0.1 dB, but four coefficient gathers plus cubic evaluation made it
  14-56% slower than the already-too-slow linear sinc8 table. Rejected.
- **DPW/ADAA family:** offline DPW5 at 1x/440 produced -45.41 dBc alias and
  6.829 dB full-band wanted error, numerically matching current Spline to the
  displayed precision. A stateful DPW implementation would evaluate and
  difference the polynomial every sample, while KURV's local polynomial
  correction usually exits away from an edge. No new frontier.
- **Event-scheduled minBLEP:** a best-case 32-sample correction-ring
  microbenchmark tested AoS, SoA, and batched SoA layouts. At 440 Hz its best
  bookkeeping alone cost 6.1 ns/frame for 8 saw lanes and 47.1 ns/frame for
  64; at 4.19 kHz, AoS cost 10.1 and 80.5 ns/frame. Two-edge pulse bookkeeping
  reached 14.6/116.0 ns/frame at 4.19 kHz. These figures exclude raw waveform,
  fractional-table interpolation, gains, and mixing. One 32-sample ring also
  adds 8 KiB per 64-lane voice (256 KiB at 32-voice polyphony). This cannot beat
  the current core CPU target.
- **BLIT/closed-form harmonic sums:** exact in principle, but require trigonometric
  evaluation, division, and integration/DC control per lane. They only become
  attractive when the harmonic count is already small; the separate adaptive
  Fourier experiment covers that high-pitch seam more directly.

## Stateful DPW continuation

The offline DPW result above was followed by a real stateful implementation in
a fresh snapshot of the optimized Spline2 source. It used the fifth-order DPW
polynomial

`x^5 - (10/3)x^3 + (7/3)x`

with four finite differences, exact note-start history priming, a dedicated
`f64` phase, and two AVX2 `f64x4` banks for each eight-lane oscillator group.
Both direct fourth differences and stage-by-stage normalized differences were
measured. This did not expose a production frontier.

The numerical failure is decisive. A raw `f32` fourth difference at 8.423 Hz
produced a peak of **3,200,485.75** and **+13.88 dBc** alias residual. `f64`
history stopped the explosion, and scaled differentiation between stages
improved the lowest note, but could not approach the stateless Spline path:

| 2x saw path | 8.423 Hz alias / wanted RMS <=20 kHz | 439.819 Hz | 4,394.531 Hz |
|---|---:|---:|---:|
| Optimized Spline2 | **-110.31 dBc / 0.091 dB** | **-93.03 / 0.075** | **-80.41 / 0.029** |
| staged `f64` DPW5 + correction EQ | -66.27 / 0.148 | -83.35 / 0.165 | -71.33 / 0.143 |
| direct `f64` DPW5 + correction EQ | -54.33 / 0.187 | -83.35 / 0.165 | -71.33 / 0.143 |

Pinned-core release benchmarks used `-C target-cpu=x86-64-v3`, 1,000,000 host
frames, and nine repeats. These are median ns per host frame for optimized
Spline2 -> staged SIMD DPW5:

| MIDI note / 2x saw | 1 lane | 8 lanes | 64 lanes |
|---|---:|---:|---:|
| 21 | 47.021 -> 46.462 (-1.2%) | 48.041 -> 85.944 (+78.9%) | 100.907 -> 367.894 (+264.6%) |
| 69 | 44.665 -> 46.052 (+3.1%) | 50.025 -> 88.400 (+76.7%) | 128.546 -> 366.957 (+185.5%) |
| 108 | 53.019 -> 47.558 (-10.3%) | 65.997 -> 90.894 (+37.7%) | 197.035 -> 345.375 (+75.3%) |

The apparent one-lane wins do not survive KURV's unison workload. The stateful
candidate also enlarges each oscillator from 4 to 56 bytes before adding the
second edge required for pulse. The memory layout can be changed to structure
of arrays, but that cannot remove the two `f64x4` polynomial/difference banks or
repair the fidelity loss.

Pitch motion is a harder blocker than fixed-note CPU. Stage-normalized DPW
state assumes one constant phase step. Changing coherent bin 1201 to 1202, only
**0.0833%**, produced a **4.98** transition peak; a 0.75% change produced
**45.64**. Direct unscaled history still peaked at 4.15 for the smaller change.
The optimized Spline path remained within its normal 1.16 peak. Re-priming DPW
history on each changed step would require four historical polynomial
evaluations per lane on every sample under continuous swarm motion. Computing
nonuniform divided differences is more expensive still.

A lower-order DPW4 probe reduced work but was also dominated. At 440 Hz its
1/8/64-lane CPU was 47.187/85.170/330.162 ns versus Lagrange2's
47.559/51.773/141.300 ns. Its alias residual at 8.4/439.8/4,394.5 Hz was
-90.01/-72.72/-61.07 dBc, while wanted-magnitude RMS stayed near 0.55 dB.

Pulse, triangle, and morph do not rescue this design:

- variable-duty pulse needs either two independent DPW saw histories or a
  fractional interpolated comb delay; the primary paper explicitly recommends
  the delay method as cheaper for high-order DPW;
- DPW triangle uses a separate full-wave polynomial family and its own
  differentiator history;
- saw-to-pulse morph needs saw and pulse states running simultaneously, adding
  cost and state-transition problems before it can blend them.

The symbolic, piecewise reduction of DPW's finite difference avoids the
cancellation and state problems. That reduction is the local spline/BLEP form
KURV is already using, and it is faster because it does nearly all extra work
only around an edge. No DPW patch was retained.

## Primary-source grounding

- Välimäki, Nam, Smith, and Abel derive higher-order DPW oscillators, describe
  the finite-difference passband droop/equalization problem, and conclude that
  increasing DPW order generally costs less than oversampling:
  [Alias-Suppressed Oscillators Based on Differentiated Polynomial Waveforms](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf).
- Brandt derives windowed-sinc BLEP/minBLEP synthesis, uses 16 zero crossings
  with an oversampled table in the worked design, and notes that event cost is
  proportional to oscillator frequency:
  [Hard Sync Without Aliasing](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf).
- The existing polynomial-transition family is grounded in the integrated
  polynomial and spline BLEP/BLAMP work:
  [Integrated polynomial methods](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf),
  [spline-based transitions](https://mac.kaist.ac.kr/pubs/PekonenNamSmithValimaki-spletter2012.pdf).

## Reproduction and artifact

The frozen experiment is `/tmp/kurv-altcore.OrINVx`. The non-production patch
against its frozen pre-experiment oscillator is:

`/tmp/kurv-windowed-sinc16-experimental.patch`

SHA-256: `84025e2cc0c5715d9da94872fa2c23b657253db88ec3505a905781b186f9ff1e`

The patch applies cleanly in isolation and includes the table generator,
generated table, harness selector, scalar fallback, and AVX2 gather path. It is
an experiment for future offline/high-quality use, not a recommendation to
apply over the now-evolving live Spline implementation.

Key commands:

```bash
cd /tmp/kurv-altcore.OrINVx
PYTHONPATH=/tmp/kurv-scipy.qarJGM python3 scripts/generate-windowed-sinc-tables.py 16 16 > src/windowed_sinc_tables.rs
CARGO_TARGET_DIR=/tmp/kurv-altcore-target-v3 RUSTFLAGS='-C target-cpu=x86-64-v3' cargo build --release --example generator_lab --locked
/tmp/kurv-altcore-target-v3/release/examples/generator_lab render sinc16 2 saw 1201 131072 /tmp/sinc16.f32
python3 scripts/analyze-generator.py /tmp/sinc16.f32 saw 1201 131072
taskset -c 8 /tmp/kurv-altcore-target-v3/release/examples/generator_lab bench sinc16 2 saw 64 100000 5 69
```

Verification passed in the frozen tree for `cargo fmt -- --check`, all eight
existing library tests in generic release compilation, and all eight with
`-C target-cpu=x86-64-v3`.
