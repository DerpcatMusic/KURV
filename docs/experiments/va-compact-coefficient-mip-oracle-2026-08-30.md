# Compact coefficient-mip oracle — 2026-08-30

## Verdict

Reject production integration. Offline ideal projections can be compressed to
five ordinary 256-byte `WaveCurveRt` coefficient frames, and interpolating
those coefficients makes band motion continuous. However, the existing
clamped 16-cubic evaluator cannot accurately represent high-note square and
pulse projections, while saw error grows quickly beyond three harmonics. The
representation fails its quality gate before publication or full-path changes
are justified. Production DSP is unchanged.

## Representation

The offline probe samples each source at 65,536 phases, transforms it once, and
retains ideal Fourier projections at harmonic caps 2, 3, 6, 13, and 27. Each
projection is fitted back into the existing sixteen uniform cubics by matching
four points per segment. Runtime data is only five normal `WaveCurveRt` values;
there is no sampled wavetable, FFT, analysis, allocation, lock, or I/O in the
audio path. Ordinary `eval` and `eval8` remain the complete sample evaluators.

Adjacent frames are interpolated coefficient by coefficient. Since evaluation
is linear in the coefficients before the existing output clamp, this removes
the hard frame jumps measured in the earlier 2,048-sample spectral-mip bank.

The probe covers analytic saw, square, 37% pulse, triangle, and KURV's
representative six-knot drawn curve.

## Projection error

RMS and peak compare each fitted 16-cubic frame against its exact retained
harmonic projection over 65,536 phases.

| Shape | Cap 2 RMS / peak | Cap 3 | Cap 6 | Cap 13 | Cap 27 |
|---|---:|---:|---:|---:|---:|
| Saw | 0.000027 / 0.000065 | 0.000095 / 0.000266 | 0.005528 / 0.032732 | 0.019311 / 0.106716 | 0.046485 / 0.227364 |
| Square | 0.129376 / 0.273239 | 0.096899 / 0.200422 | 0.080150 / 0.188357 | 0.054288 / 0.180684 | 0.065218 / 0.224670 |
| Pulse 37% | 0.114286 / 0.372606 | 0.110317 / 0.228802 | 0.079625 / 0.223049 | 0.054902 / 0.206591 | 0.066322 / 0.241011 |
| Triangle | 0.000004 / 0.000010 | 0.000039 / 0.000087 | 0.000111 / 0.000254 | 0.000639 / 0.002148 | 0.002414 / 0.011022 |
| Drawn | 0.000021 / 0.000052 | 0.000122 / 0.000300 | 0.000493 / 0.001891 | 0.002399 / 0.008753 | 0.007534 / 0.043206 |

Triangle and the drawn curve compress well at high-note caps. Saw is credible
only at caps two and three. Square and pulse fail even there because ideal
bandlimited discontinuities overshoot `[-1, 1]`; `WaveCurveRt::eval` clamps
that overshoot away. This is a representation-contract failure, not an FFT or
frame-selection error. Removing the clamp would change every existing curve
and was not considered safe.

## Pitch-band interpolation

A 16,384-step coefficient sweep was tested between every adjacent frame. The
largest same-phase change across one sweep increment was:

- saw: `5.2631e-5`;
- square: `5.9962e-5`;
- pulse: `5.9724e-5`;
- triangle: `1.1086e-5`;
- drawn: `3.4288e-5`.

Thus coefficient interpolation fixes the old hard-mip click. It cannot recover
the shape discarded by the cubic fit and clamp. Against a linear interpolation
of the two ideal projections, peak errors reached `0.272662` for square and
`0.372178` for pulse. Saw reached `0.100219` in the cap 6-to-13 band. A smooth
pitch sweep is continuous but continuously wrong.

## CPU and storage

The retained frame uses the exact shipping evaluator, so scalar, x4, x8, and
the structural one-unison inner sample path have identical instructions and
cycles once a frame is selected. On the pinned x86-64-v3 release probe:

- scalar `eval`: `4.892 ns/sample` median;
- `eval8`: `0.788 ns/sample/lane` median;
- coefficient interpolation plus one scalar evaluation on every sample:
  `7.742 ns/sample`.

The intended once-per-block coefficient selection would amortize its fixed 64
coefficient operations, but it cannot make sampling cheaper than current 1x.
The corrected current-main structural one-unison factor-1 reference remains
2,174,081,939 instructions and 889,624,894 cycles for 20,000 64-frame callbacks
at 48 kHz; after selection this proposal executes that same path. No separate
full integration was retained after the square/pulse quality failure.

Five frames cost 1,280 bytes per curve versus 256 bytes today. A full 16-frame
VA table would grow from 4,096 to 20,480 bytes. Existing atomic publication
would expand from 64 to 320 words per curve unless an immutable-bank ownership
seam were added. This is far smaller than the rejected 160 KiB sampled mip,
but still a 5x publication/storage multiplier for a representation that fails
canonical discontinuities.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example coefficient_mip_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/coefficient_mip_lab
$lab report
taskset -c 8 $lab bench

cargo fmt --all -- --check
git diff --check
```

The release build, projection matrix, sweep probe, formatting, and diff checks
passed with the checkout's existing warnings. The probe is offline-only and no
runtime source, dependency, state, or Cargo metadata change is retained.
