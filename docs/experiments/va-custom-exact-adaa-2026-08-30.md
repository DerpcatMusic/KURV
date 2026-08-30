# VA custom exact polynomial ADAA experiment — 2026-08-30

## Verdict

Reject production integration. Exact centered interval averaging improves the
raw 1x curve by 5.7–7.0 dB, and the stronger centered triangular kernel improves
it by 10.3–12.3 dB. Neither matches shipping 2x quality. The triangular kernel
loses shipping unwanted energy by 2.2–6.1 dB while roughly doubling wanted-band
droop. Exact splitting is also substantially more expensive than shipping at
64 lanes. Production DSP is unchanged.

## Candidates and correctness boundary

`WaveCurveRt` stores sixteen cubic segments. The probe integrates each segment
in closed form, splitting the interval at every segment boundary and periodic
wrap. Two zero-phase virtual downsampling kernels were tested:

- `box`: one centered phase-step box, using the exact quartic first primitive;
- `triangle`: a centered triangular kernel over two phase steps, using exact
  zeroth and first moments (equivalent to a quintic second primitive).

Both kernels add zero samples of delay, zero persistent state, and no runtime
dependency. A near-equal phase step returns the ordinary curve evaluator. The
moments use `f64`: `f32` subtraction caused catastrophic cancellation at low
pitch in the triangular kernel. Processing is bounded to 64 segment chunks and
performs no allocation, locking, blocking, logging, or I/O.

Scalar, packed-x4, and packed-x8 paths are sample-identical in static, moving
pitch, and morph renders. The runnable check covers near-equal phase, ordinary
segment intervals, boundary crossing, cycle wrap, and one shaped five-knot
curve. Its exact box result matches a 262,144-point midpoint reference within
`3e-6`.

The exactness boundary matters: `WaveCurveRt::eval` clamps its cubic to
`[-1, 1]`. The tested default and shaped curves do not clip inside the measured
intervals. A cubic that overshoots and activates the clamp would require fixed
cubic-root splitting at the clamp crossings. The current primitive would then
integrate the unclipped polynomial, so it is not universally exact for every
possible `WaveCurveRt`. The candidates fail quality and CPU before that extra
complexity is justified.

## Static ideal-reference quality

All renders use the exact default triangle, 48 kHz, 65,536 coherent output
samples, four buffers of warm-up, and KURV's 33-sample output delay. Raw2 is the
actual shipping 97-tap 2x decimator plus spline equalizer. The ideal reference
uses the analytic triangle harmonics; one common fitted delay is removed before
complex wanted-error measurement.

| FFT bin | Raw1 unwanted | Box unwanted | Triangle unwanted | Shipping 2x unwanted | Box wanted RMS | Triangle wanted RMS | Shipping wanted RMS |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 89 | -84.818 | -91.786 | -97.130 | **-99.304 dBc** | **1.152 dB** | 2.305 dB | 1.235 dB |
| 601 | -59.820 | -66.670 | -71.927 | **-74.387 dBc** | **1.187 dB** | 2.375 dB | 1.294 dB |
| 4806 | -31.413 | -37.143 | -41.665 | **-47.798 dBc** | **1.175 dB** | 2.350 dB | 1.315 dB |
| 7000 | -26.375 | -32.062 | -36.670 | **-41.882 dBc** | **0.959 dB** | 1.918 dB | 1.163 dB |

Box complex wanted error is -88.123, -63.271, -36.617, and -31.816 dBc.
Triangle is -82.622, -57.762, -31.005, and -26.173 dBc. Shipping is -44.771,
-55.287, -35.602, and -31.612 dBc. Box preserves wanted phase and magnitude
better than shipping in several cells, but its remaining aliases make its
whole ideal-reference error worse at every bin. Triangle suppresses more
aliases but its `sinc²` passband droop worsens complex and whole-curve error.

All static outputs are finite and effectively DC-free. Box peak spans
0.893–0.999; triangle spans 0.858–0.998. Neither introduces ringing or a click.

## Full-path CPU

Release build used `-C target-cpu=x86-64-v3`, one pinned CPU, 200,000 host
frames, and three `perf stat` repetitions. Ratios are candidate divided by the
actual shipping raw2 path, including curve evaluation, lane mix, and output
delay/decimator.

| Bin | Lanes | Box scalar instr/cycles | Box x8 instr/cycles | Triangle scalar instr/cycles | Triangle x8 instr/cycles |
|---:|---:|---:|---:|---:|---:|
| 601 | 1 | **0.64x / 0.73x** | 1.35x / 1.39x | **0.91x / 1.00x** | 1.74x / 1.75x |
| 601 | 8 | 2.44x / 2.66x | **1.91x / 2.31x** | **4.40x / 4.56x** | 4.42x / 4.70x |
| 601 | 64 | 7.32x / 8.44x | **5.48x / 7.28x** | 13.90x / 15.41x | **13.90x / 16.57x** |
| 7000 | 1 | **0.93x / 0.87x** | 1.54x / 1.70x | **1.50x / 1.58x** | 2.34x / 2.56x |
| 7000 | 8 | 4.52x / 7.84x | **3.26x / 7.24x** | **8.55x / 14.38x** | 8.66x / 15.32x |
| 7000 | 64 | 14.29x / 17.06x | **10.01x / 14.98x** | **27.84x / 31.95x** | 28.13x / 33.57x |

At 64 lanes/bin 601, shipping retires 275,517,772 instructions and 98,737,389
cycles. Packed x8 box retires 1,510,996,066 instructions and 719,100,516
cycles. At bin 7000 it retires 2,757,067,184 instructions and 1,514,913,284
cycles versus shipping's 275,517,771 and 101,122,045.

The packed paths batch phase and mixing but retain per-lane `f64` exact
boundary splitting; they are not a claim of an optimal AVX primitive. A
same-segment vector fast path could reduce CPU, but cannot repair the measured
quality loss, so more implementation complexity was stopped at the quality
gate.

A 64-lane 601-to-7000-bin glide costs box x8 2.00x shipping instructions and
2.60x cycles; triangle x8 costs 4.73x and 5.83x. A smooth shaped-curve morph
costs box x8 5.15x/5.98x and triangle x8 13.01x/15.52x. The morph workload
includes identical per-sample 64-coefficient interpolation in every path.

## Moving pitch and morph artifacts

The 65,536-sample glide, hard pitch jump, smooth morph, and hard morph were all
finite. Scalar/x4/x8 candidate renders were bit-identical. Smooth glide peaks
were 0.991 box and 0.987 triangle; hard-pitch peaks were the same, with maximum
adjacent step 0.427246 versus shipping 0.521667. Neither candidate rings on a
pitch transition.

The smooth shaped-curve morph peaks at 0.988 box and 0.985 triangle. Its mean
is 0.0142 because the target curve itself is asymmetric, not because state
accumulates DC. A hard morph produces a 0.7231 adjacent step in both candidates
versus 0.6294 after shipping's decimator. Exact interval integration cannot
remove a discontinuous change of the curve definition itself.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features --example adaa_curve_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/adaa_curve_lab
$lab check
$lab render tri-s static 7000 7000 65536 /tmp/adaa-tri-7000.f32
python3 scripts/analyze-custom-event.py \
  /tmp/adaa-tri-7000.f32 7000 65536 --delay 33

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench box8 static 7000 7000 64 200000 1
taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench box8 morph 601 7000 64 100000 1
```

The release build completed with the checkout's 83 existing warnings and no
experiment error. Formatting, boundary/wrap/near-equal checks, scalar/packed
parity, finite render checks, and diff checks passed. Production source, Cargo
metadata, and dependencies remain unchanged.
