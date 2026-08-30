# Narrow cap-6 coefficient backend and crossover (2026-08-30)

## Verdict

Reject this minimal integration and retain no runtime change. The three
unclamped frames remain an unusually accurate high-note representation, and
their steady factor-1 cost is far below shipping factor 2. The strict gate that
fails is entry/exit safety: an abrupt pitch, warp, or morph eligibility change
can switch between the raw and projected curves with a same-phase difference
near full scale. A short 32-sample fade still permits a 0.0336 per-sample pulse
step. Hiding it below about 0.0011 requires a 1,024-sample transition, duplicated
evaluation during that transition, and per-voice transition state. That is no
longer the small stateless backend tested here.

Production `WaveCurveRt` remains clamped. The probe uses a distinct finite
`UnclampedCurve`, three 256-byte frames at harmonic caps 2, 3, and 6, adjacent
coefficient interpolation, and coefficient interpolation from cap 6 back to an
unclamped raw frame over the cap 6-to-8 crossover. Raw eligibility requires all
cubic extrema to lie in `[-1, 1]`, making removal of the otherwise-active clamp
output-identical. Compiled frames must be finite and bounded to `1.5`; measured
canonical and drawn projections stayed within `[-1.273240, 1.372665]`.

## Static quality win

The coherent 65,536-phase exact Fourier measurements from the boundary round
remain the relevant eligible-region result:

| Shape / cap | RMS / peak vs exact projection | wanted / unwanted dB |
|---|---:|---:|
| square / 2 | .000006830 / .000015378 | -108.774 / -103.537 |
| square / 6 | .000840136 / .001949787 | -69.567 / -61.898 |
| pulse37 / 2 | .000040122 / .000100255 | -93.536 / -87.976 |
| pulse37 / 6 | .001054536 / .002709925 | -67.897 / -59.507 |
| saw / 6 | .000840683 / .002928078 | -65.347 / -56.890 |
| triangle / 6 | .000111427 / .000253797 | -82.519 / -74.993 |
| drawn / 6 | .000492758 / .001891136 | -70.091 / -61.922 |

For context, KURV's real shipping 2x harness previously measured whole-signal
ideal-reference error of only -35.344 and -31.212 dBc at its two high bins and
unwanted energy of -47.798 and -41.882 dBc. The narrow representation therefore
has substantial eligible high-note quality margin, not merely a raw-1x win.

Adjacent projected-frame interpolation remains continuous: the prior maximum
same-phase step was `5.9962e-5`. Slow and fast pitch sweeps were finite. Their
whole-sweep ideal errors are dominated by the intentionally raw cap-above-8
region and by the ideal reference's integer harmonic removal, so they are not
used to claim static quality.

## Crossover and abrupt changes

Coefficient interpolation can bridge raw to cap 6 when the raw cubic is proven
range-safe; no output crossfade is needed for a continuous pitch sweep. It does
not solve an abrupt selector or eligibility jump. Sweeping every phase for a
jump from raw cap 8.5 to projected cap 5.5 produced:

| Shape | Direct same-phase delta | 32-sample linear step bound | 1,024-sample step bound |
|---|---:|---:|---:|
| saw | .999817 | .031244 | .000976 |
| square | .999827 | .031245 | .000976 |
| pulse37 | 1.074020 | .033563 | .001049 |
| triangle | .072348 | .002261 | .000071 |
| drawn | .270348 | .008448 | .000264 |

The 32-sample transition is not click-safe. The long alternative costs 21.3 ms
at 48 kHz and must evaluate both old and new representations while active.
Phase warp is explicitly ineligible because warping a bandlimited curve creates
new bandwidth. Custom/canonical morph is also ineligible in this minimal form;
it would require matching projected banks for both endpoints. Both must fall
back through the same stateful transition, which was not implemented after the
artifact gate failed.

## CPU and state

On quiet pinned x86-64-v3 runs, median isolated results were:

- current clamped scalar `3.270 ns/sample`, unclamped scalar `3.695`;
- current clamped `eval8` `.659 ns/sample/lane`, unclamped `.589`;
- cap selection plus coefficient interpolation once per 64 samples and scalar
  evaluation `2.825 ns/sample` (the loop auto-vectorizes, so it is an upper
  structure check rather than a scalar production claim);
- the equivalent real `eval8` block `.704 ns/sample/lane`, 6.8% above current
  clamped `eval8`.

A temporary 64-coefficient selection was inserted once per callback around the
actual one-lane structural factor-1 process and then reverted. It moved retired
instructions from 2,174,020,918 to 2,174,802,300 (+0.036%); cycles were
849,152,952 versus 841,960,794, inside run noise. Shipping factor 2 used
2,630,359,788 instructions and 985,998,110 cycles. A real x8-unison factor-1
baseline used 1,081,252,000 instructions and 435,034,000 cycles for 10,000
callbacks, versus factor 2's 1,279,898,274 and 506,166,621. The fixed selector
cost would be about half the one-lane 20,000-callback delta at that callback
count. Thus steady CPU is competitive with current 1x and clearly below 2x,
but the required safe transient would temporarily duplicate curve evaluation.

Three frames cost 768 bytes per curve and 12,288 bytes for all 16 VA-table
frames, versus 4,096 bytes for today's curves. Copying current atomic
publication requires 3,072 `f32` words. Stateless steady evaluation needs two
frame indices and a mix; safe entry/exit additionally needs old/new selector
state and a fade counter per voice.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example coefficient_mip_lab --example process_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/coefficient_mip_lab
$lab report
$lab transition
taskset -c 8 $lab bench

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  ./target/release/examples/process_lab 64 20000 1 custom 1 48000 1
taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  ./target/release/examples/process_lab 64 20000 1 custom 1 48000 2
```

The release probes built with the checkout's 83 existing warnings. The
temporary structural selector and x8-unison configuration were reverted. No
runtime source, dependency, Cargo metadata, or version change is retained.
