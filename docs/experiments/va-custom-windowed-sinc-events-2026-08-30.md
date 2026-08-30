# VA custom windowed-sinc event experiment — 2026-08-30

## Verdict

Reject the tested 1x 12- and 24-sample fractional windowed-sinc BLAMP
residuals. The 24-sample candidate was the quality winner, but it still lost
shipping 2x unwanted energy by 4.54 dB near 3.5 kHz and used 4.82–12.03 times
as many retired instructions across the measured 64-lane workloads. No
production DSP changed.

The official Signalsmith Elliptic BLEP reference reached -82.20 to -137.68 dBc
unwanted energy. That establishes a materially better quality frontier, but its
per-lane 11th-order pole state and allpass require a separate CPU/state/latency
experiment. This experiment does not vendor or integrate that external code.

## Candidate

Both FIR candidates use the same exact fractional derivative events established
in round one: slope jump `-8` at phase `0.25` and `+8` at `0.75`. Their
continuous low-pass is a normalized Hann-windowed sinc at Nyquist. Integrating
the impulse once gives BLEP and twice gives the BLAMP residual. Coefficients
were generated offline at 16,384 integration points per sample and compiled as
fixed `f32` tables with eight fractional entries per sample.

The runtime is deterministic and allocation-, lock-, state-, and I/O-free. It
linearly interpolates the fixed residual table and sums the bounded periodic
images needed by the test range. Complexity is `O(events * images * lanes)`;
the 12-sample path stores 49 floats and the 24-sample path 97 floats. The
scalar/SIMD check passed within `2e-6`.

The harness has a deliberate experiment ceiling: three periodic images for
12 samples and five for 24 samples cover fundamentals through the 5.1 kHz
matrix. A production design covering higher fundamentals would need a fixed
event ring rather than more unconditional image probes. The candidates failed
before that expansion was justified.

## Quality

Baseline is commit `427917d`. All Rust modes use KURV's exact
`WaveCurveRt::default()` evaluator at 48 kHz, 65,536 coherently indexed output
samples, one complete warm-up cycle, and KURV's 33-sample output-latency
contract. Shipping `raw2` evaluates the curve at 2x and passes it through the
real 97-tap decimator and spline EQ.

| FFT bin | Sinc12 unwanted dBc | Sinc24 unwanted dBc | Shipping 2x unwanted dBc | Sinc24 wanted magnitude RMS | Shipping wanted magnitude RMS |
|---:|---:|---:|---:|---:|---:|
| 89 | -99.094 | **-101.689** | -99.306 | **0.034 dB** | 1.235 dB |
| 601 | -73.515 | **-75.642** | -74.387 | **0.033 dB** | 1.294 dB |
| 4806 | -41.497 | -43.257 | **-47.798** | **0.043 dB** | 1.315 dB |
| 7000 | -38.503 | **-44.923** | -41.882 | **0.026 dB** | 1.163 dB |

After fitting and removing one common linear delay, sinc24 complex
wanted-harmonic error was -100.470, -76.890, -65.584, and -61.414 dBc.
Whole-signal error against the ideal bandlimited triangle was -98.027,
-73.211, -43.235, and -44.833 dBc. Shipping 2x measured -44.771, -55.233,
-35.344, and -31.212 dBc on that same ideal-reference metric.

The FIR candidates therefore preserve the intended complex curve much better
than shipping 2x, but the 24-sample stopband remains insufficient in one matrix
cell. More importantly, it fails the CPU requirement by a wide margin.

All coherent renders were finite and effectively DC-free. Sinc24's maximum
adjacent step was smaller than shipping 2x in every cell, but its high-bin peak
was `0.905423939` versus shipping `0.948526084`; this is bounded filtering, not
a click, and also shows the remaining peak-shape difference. Modulation and
host audition were skipped after the static and CPU gates failed.

## 64-lane CPU

Release build used `-C target-cpu=x86-64-v3`, one pinned CPU, 500,000 host
frames, and three `perf stat` repetitions. Retired instructions varied by less
than 0.01%.

| FFT bin | Sinc12 instructions | Sinc24 instructions | Shipping 2x instructions | Sinc12 / 2x | Sinc24 / 2x |
|---:|---:|---:|---:|---:|---:|
| 601 | 1,008,792,084 | 2,819,876,100 | 584,584,722 | 1.73x | 4.82x |
| 4806 | 2,684,400,647 | 5,589,667,202 | 584,580,020 | 4.59x | 9.56x |
| 7000 | 3,539,558,690 | 7,034,682,480 | 584,584,531 | 6.06x | 12.03x |

Sinc24 cycle ratios versus shipping 2x were 4.71x, 7.60x, and 8.90x. The
frequency scaling confirms that sparse long support becomes dense across
staggered unison lanes.

## Elliptic reference

The owner-maintained MIT header was fetched without adding a dependency. The
reference uses derivative order 2, exact samples-in-past event timing, and the
owner's approximate-linear-phase allpass. Its fitted delay was approximately
10.44–10.99 samples (the declared integer allpass delay is 12).

| FFT bin | Elliptic unwanted dBc | Wanted magnitude RMS | Complex wanted error dBc | Ideal-reference error dBc |
|---:|---:|---:|---:|---:|
| 89 | -137.682 | 0.145 dB | -37.654 | -37.654 |
| 601 | -111.734 | 0.149 dB | -37.522 | -37.522 |
| 4806 | -82.345 | 0.061 dB | -55.258 | -55.250 |
| 7000 | -82.196 | 0.042 dB | -47.490 | -47.489 |

Its unwanted-energy result is decisive; the lower complex score at low pitch
reflects residual nonlinear phase after the approximate allpass. CPU, state
size, and pitch-transition behavior remain unmeasured here and must be gated
before considering a clean-room/minimal implementation or licensed reuse.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features --example custom_event_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/custom_event_lab
$lab check
for bin in 89 601 4806 7000; do
  for mode in raw1 sinc12 sinc24 raw2; do
    $lab render "$mode" "$bin" 65536 "/tmp/va-event-${mode}-${bin}.f32"
    python3 scripts/analyze-custom-event.py \
      "/tmp/va-event-${mode}-${bin}.f32" "$bin" 65536 --delay 33
  done
done

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench sinc24 4806 64 500000 1

curl -fsSL \
  https://raw.githubusercontent.com/Signalsmith-Audio/elliptic-blep/main/elliptic-blep.h \
  -o /tmp/elliptic-blep.h
g++ -O3 -march=x86-64-v3 -std=c++20 -I/tmp \
  examples/elliptic_custom_reference.cpp -o /tmp/elliptic_custom_reference
/tmp/elliptic_custom_reference 601 65536 /tmp/va-elliptic-601.f32
python3 scripts/analyze-custom-event.py \
  /tmp/va-elliptic-601.f32 601 65536 --delay 12
```

The Rust release build completed with the checkout's 83 existing warnings and
no experiment build error. The Elliptic reference source remains external and
is governed by its owner's MIT license.
