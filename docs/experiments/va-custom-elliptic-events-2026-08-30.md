# VA custom Elliptic BLEP viability experiment — 2026-08-30

## Verdict

Reject production integration. The Elliptic BLEP is the strongest unwanted-
energy result so far and its scalar implementation is cheaper than shipping 2x
for one lane. It is not Pareto-safe across the eligible custom-curve workload:
at 64 lanes, the best x8 layout costs 3.13–8.82 times shipping retired
instructions and 2.31–8.51 times cycles. At low pitch its approximately linear
phase has worse complex wanted-curve error than shipping, and an abrupt
601-to-7000-bin pitch jump produced a 1.776 peak. Production DSP is unchanged.

## Provenance and implementation

The standalone Rust probe is derived from Signalsmith Audio's owner-maintained
MIT implementation at commit
`77bf9866b705ddffe4870b40020411cf9192cf3b`. The exact 11th-order pole,
derivative-order-two residue, and eight-state allpass coefficients are retained
with attribution and the owner license in
`examples/elliptic_event_lab.LICENSE.txt`. No runtime dependency was added.

The probe separates one immutable kernel from lane state. The kernel contains
128 fractional pole positions and derivative-order-two coefficients. Scalar,
x4, and x8 state updates share it. SIMD fractional events use bounded scalar
table gathers only when any lane crosses an event; the eight complex pole
updates and allpass are cross-lane SIMD. Initialization computes the table
before rendering. Timed processing has fixed work and performs no allocation,
locking, blocking, logging, or I/O.

| Layout | State | Per lane | Shared kernel |
|---|---:|---:|---:|
| Scalar | 96 B | 96 B | 8,256 B |
| x4 | 384 B | 96 B | 8,256 B |
| x8 | 768 B | 96 B | 8,256 B |

The original class stores its fractional table per instance, whereas this probe
shares one table across all lanes. Scalar/x4/x8 differed by at most `4.172e-7`.
Against the owner C++ output at bin 601, Rust differed by `5.960e-7` maximum and
`1.196e-7` RMS.

## Static quality

All renders use KURV's exact `WaveCurveRt::default()` triangle, 48 kHz,
65,536 coherent output samples, and four buffers of warm-up. Raw2 is the actual
shipping 97-tap 2x decimator plus spline equalizer. The Elliptic allpass declares
12 samples of linear delay; fitted group delay varies from 10.443 to 10.988
samples. Shipping's fitted delay is 32 samples inside its 33-sample host delay
contract.

| FFT bin | Elliptic unwanted | Shipping 2x unwanted | Elliptic wanted magnitude RMS | Elliptic complex wanted | Shipping complex wanted | Elliptic ideal error | Shipping ideal error |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 89 | **-135.189 dBc** | -99.304 dBc | **0.145 dB** | -37.654 dBc | **-44.771 dBc** | -37.654 dBc | **-44.771 dBc** |
| 601 | **-111.731 dBc** | -74.387 dBc | **0.149 dB** | -37.522 dBc | **-55.287 dBc** | -37.522 dBc | **-55.233 dBc** |
| 4806 | **-82.344 dBc** | -47.798 dBc | **0.061 dB** | **-55.258 dBc** | -35.602 dBc | **-55.249 dBc** | -35.344 dBc |
| 7000 | **-82.198 dBc** | -41.882 dBc | **0.042 dB** | **-47.490 dBc** | -31.612 dBc | **-47.489 dBc** | -31.212 dBc |

Shipping wanted-magnitude RMS error is 1.163–1.315 dB, so Elliptic preserves
wanted magnitudes much better and removes substantially more unwanted energy.
It nevertheless loses the low-frequency complex/whole-curve gate because the
allpass phase is only approximately linear. A pure fixed delay cannot remove
that residual phase error.

All static outputs were finite and effectively DC-free. Elliptic peak spans
0.899–1.012 and its maximum adjacent step spans 0.00629–0.48949. Matching
KURV's existing 33-sample path would also require path-specific delay handling:
using the current factor-one delay unchanged would add 33 samples after the
Elliptic allpass, giving roughly 44 samples rather than the shared contract.

## CPU and event density

Release build used `-C target-cpu=x86-64-v3`, one pinned CPU, 500,000 host
frames, and three `perf stat` repetitions. Bin 601 creates approximately 0.0183
events per lane/sample (1.17 across 64 lanes); bin 7000 creates 0.2136 per
lane/sample (13.67 across 64 lanes). Ratios below are candidate divided by the
actual shipping raw2 workload, including lane evaluation, mixing, and the
shared output/decimator path.

| Static bin | Lanes | Scalar instr/cycles | x4 instr/cycles | x8 instr/cycles |
|---:|---:|---:|---:|---:|
| 601 | 1 | **0.71x / 0.89x** | 1.16x / 1.16x | 1.12x / 1.23x |
| 601 | 8 | 1.94x / 2.20x | 1.88x / 1.61x | **1.22x / 1.20x** |
| 601 | 64 | 5.40x / 5.21x | 5.49x / 3.70x | **3.13x / 2.31x** |
| 7000 | 1 | **0.72x / 0.75x** | 1.30x / 1.23x | 1.41x / 1.24x |
| 7000 | 8 | **2.08x / 2.74x** | 2.80x / 3.29x | 2.77x / 2.82x |
| 7000 | 64 | **5.87x / 7.53x** | 8.79x / 8.57x | 8.82x / 8.51x |

At the representative low-density 64-lane point, shipping retired 685,117,601
instructions and 299,862,152 cycles; x8 retired 2,143,061,490 instructions and
693,781,266 cycles. At high density shipping retired 685,119,449 instructions
and 254,498,740 cycles; x8 retired 6,040,892,886 instructions and 2,164,956,422
cycles. Event gathers dominate as density rises.

For a continuous 601-to-7000-bin glide over 300,000 frames, scalar/x4/x8 ratios
versus shipping were respectively 0.70/1.18/1.22x instructions at one lane,
1.19/1.35/1.22x at eight lanes, and 1.59/1.92/1.72x at 64 lanes. Corresponding
64-lane cycle ratios were 1.88/1.78/1.65x. These ratios include per-sample pitch
calculation in both paths; they do not overturn the static oscillator cost.

## Pitch transitions and artifacts

A 65,536-sample glide and a hard pitch jump from bin 601 to 7000 were rendered
after four buffers of static warm-up. Scalar/x4/x8 transition outputs agreed.

| Transition | Path | Peak | DC | Max adjacent step | Step near transition |
|---|---|---:|---:|---:|---:|
| Glide | Raw1 | 0.999985 | -0.0000830 | 0.427043 | 0.232345 |
| Glide | Shipping 2x | 0.996449 | -0.0000839 | 0.520144 | 0.283366 |
| Glide | Elliptic | 1.003515 | 0.0001700 | 0.489038 | 0.262024 |
| Jump | Raw1 | 1.000000 | 0.0001005 | 0.427246 | 0.427246 |
| Jump | Shipping 2x | 0.997372 | 0.0001013 | 0.521667 | 0.520933 |
| Jump | Elliptic | **1.775970** | 0.0003672 | 0.504092 | 0.504092 |

The hard-jump sample step is bounded, but the 1.776 peak is a transient residue/
allpass overshoot not present in either baseline. Smoothing pitch may hide it,
but that would change the experiment and does not satisfy universal transition
safety. This independently rejects production integration.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example elliptic_event_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/elliptic_event_lab
$lab check
$lab render scalar static 601 601 65536 /tmp/elliptic-601.f32
python3 scripts/analyze-custom-event.py \
  /tmp/elliptic-601.f32 601 65536 --delay 12

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench x8 static 7000 7000 64 500000 1
taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench x8 glide 601 7000 64 300000 1

$lab render scalar jump 601 7000 65536 /tmp/elliptic-jump.f32
```

The release build completed with the checkout's 83 existing warnings and no
experiment error. `cargo fmt`, scalar/SIMD parity, owner-output parity, finite
render checks, C++ `-Wall -Wextra -Werror`, and diff checks passed. Production
source, Cargo metadata, and dependencies remain unchanged.
