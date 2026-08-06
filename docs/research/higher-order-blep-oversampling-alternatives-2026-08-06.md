# Higher-order BLEP versus Normal 2x — 2026-08-06

## Decision

Do not replace KURV's Spline 4PT / Normal 2x generator with the tested 1x
higher-order BLEP. The new candidate failed both the fidelity and CPU gates, so
no DSP source was retained.

The current default remains the Pareto choice for the complete procedural
sine/triangle/saw/pulse morph family. DPW, event-scheduled minBLEP, and a
generic polyphase rewrite also remain rejected for this seam: existing local
experiments already establish that DPW and minBLEP lose on dense SIMD unison,
while the current decimator already evaluates only one retained-output dot
product after each group of internal samples.

## Fresh candidate

The experiment replaced only the hidden Lagrange selector's saw/pulse edge
correction with a centered cardinal B-spline of degree 7. It was procedural and
table-free, used fixed arithmetic and no persistent state, and had scalar,
SIMD4, and SIMD8 kernels. Its integrated residual had support of four phase
steps on either side of an edge:

```text
u = 4 - abs(x)
R(x) = sign(-x) / 8! * [u_+^8 - 8(u-1)_+^8
                        + 28(u-2)_+^8 - 56(u-3)_+^8]
```

This was deliberately a stronger smoothing candidate than the current cubic
spline, not another restatement of the two-point PolyBLEP. The implementation
used three squarings per eighth power and vector max/mask operations rather
than branches or a lookup table.

## Render result

Both release binaries came from commit `58d2677`, built with
`-C target-cpu=x86-64-v3`. The existing `generator_lab` rendered 65,536 samples
after 16,384 warm-up frames at coherent FFT bins 89, 601, 4806, and 7000. The
matrix covered saw, 50% pulse, and the static triangle-to-saw midpoint. No test
files were added or changed.

| Path | Worst unwanted energy | Median unwanted energy |
|---|---:|---:|
| Spline Optimized, 2x | **-82.35 dBc** | **-90.51 dBc** |
| Degree-7 B-spline BLEP, 1x | -38.61 dBc | -56.82 dBc |

Candidate worst cases by shape were:

| Shape | Worst unwanted energy |
|---|---:|
| Saw | -49.44 dBc |
| Pulse | -49.07 dBc |
| Triangle-to-saw midpoint | -38.61 dBc |

The longer spline reduced the raw 1x discontinuity error, but it did so by
strongly rounding the edge and attenuating wanted upper harmonics. More compact
support cannot supply the missing stop-band rejection, while still longer
support moves toward a minBLEP/event-convolution design and increases the
chance that at least one lane is inside a correction region on every SIMD
sample.

## Dense CPU result

Retired instructions were measured with `perf stat` on one pinned core for the
24-note, 64-unison, three-oscillator workload. Values below are the candidate's
change relative to Spline Optimized / Normal 2x.

| Shape | Candidate instruction change |
|---|---:|
| Saw | **+5.35%** |
| Pulse | **+5.65%** |
| Triangle-to-saw midpoint | **+24.72%** |

Although the candidate runs at 1x, its wider active region and four eighth-power
terms outweighed the saved internal sample. A dedicated block shortcut could
remove some dispatch overhead, but cannot close a 43.7 dB worst-case quality
gap; optimizing the failed kernel would not be useful work.

## Why the other credible alternatives are not repeated

- **DPW4/DPW5:** the prior stateful experiment already measured severe dense
  SIMD regressions, numerical sensitivity at low pitch, and large transition
  peaks under tiny pitch changes. It also needs separate histories for a
  variable-duty pulse and different mathematics for triangle. The primary DPW
  paper's perceptual result is for classical endpoint waves, not KURV's
  continuously moving whole-family contract.
- **Windowed-sinc/minBLEP:** the prior 16-sample candidate improved wanted-band
  truth but doubled dense 64-lane saw cost at 1x; its event-ring microbenchmark
  was already too expensive before interpolation and mixing. Brandt's primary
  construction also makes correction work depend on event frequency and uses
  an oversampled windowed-sinc table.
- **BLIT/FDF:** it adds an integration/DC state and fractional-delay impulse
  work per lane. It is credible for classical endpoints, but not a cheaper
  universal morph core under 24 x 64 x 3 execution.
- **Generic polyphase decimation:** `StereoOversampler` already pushes internal
  samples and computes one FIR result only at the retained host sample. A
  rearrangement into generic phases does not remove its retained-output MACs.
  The previously tested structural-zero half-band reduced isolated decimator
  work but created a 20.5-24 kHz alias shelf and lost the declared full-band
  accuracy.
- **Fusing edge correction into the decimator:** KURV applies discontinuity
  correction before per-lane pan/gain and voice mixing, while decimation occurs
  after the full stereo synth sum. Moving correction across that boundary needs
  an event stream carrying fractional time, jump size, and stereo gain for each
  active lane. That is the event-scheduled minBLEP architecture already rejected
  on bookkeeping, state size, and dense CPU.

## Primary-source check

- Välimäki, Pekonen, and Nam's integrated-polynomial study establishes that
  polynomial BLEPs avoid lookup-table oversampling and that higher order trades
  more correction work for better perceptual alias suppression. It does not
  promise exact full-band rejection for arbitrary morphs:
  [JASA 2012 author PDF](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf).
- Välimäki, Nam, Smith, and Abel show why higher-order DPW can beat oversampling
  for fixed classical waves, while also documenting frequency-dependent
  scaling and differencing:
  [IEEE TASLP 2010 author PDF](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf).
- Brandt's hard-sync work describes the practical oversampled windowed-sinc
  BLEP table and event overlap cost:
  [ICMC 2001 paper](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf).

The directly relevant earlier KURV measurements are in
[`generator-alternate-core-experiment-2026-08-04.md`](generator-alternate-core-experiment-2026-08-04.md),
and the default quality gate is in
[`generator-algorithm-audit-2026-08-06.md`](generator-algorithm-audit-2026-08-06.md).

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-hoble-base-v3 \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features --example generator_lab

generator_lab render splineopt 2 saw 7000 65536 /tmp/base.f32 0.5
generator_lab render lagrange 1 saw 7000 65536 /tmp/candidate.f32 0.5
python3 scripts/analyze-generator.py /tmp/base.f32 saw 7000 65536
python3 scripts/analyze-generator.py /tmp/candidate.f32 saw 7000 65536

taskset -c 8 perf stat -r 5 -e instructions:u -- \
  generator_lab bench splineopt 2 saw 64 131072 1 69 0.5 0 0.7 24 noise 3
taskset -c 8 perf stat -r 5 -e instructions:u -- \
  generator_lab bench lagrange 1 saw 64 131072 1 69 0.5 0 0.7 24 noise 3
```

The second command requires the rejected experimental substitution described
above; it is intentionally not present in the branch. The measured release
binaries and renders remain under `/tmp/kurv-hoble-*` for this machine-local
session.
