# Event-local parabolic quadrature (rejected)

Date: 2026-08-30

Baseline: `09e8126` (`0.8.9`; production DSP unchanged)

Host: AMD Ryzen 7 7800X3D, Linux x86-64, native release build

## Verdict

Reject. A one-sample-radius parabolic aperture improved ideal-projection error
over shipping 1x in every coherent cell and exposed large scalar and selected
x8 CPU wins. It is not a universal improvement: off-grid event phase creates
DC error, saw/square/pulse pitch and reset steps regress by about 20-28%, and
saw x8 loses most 24/32-frame cells. Shipping 2x remains much closer to the
ideal projection for every discontinuous-wave cell and high-note triangle.

No production code, state, dependency, version, or retained probe changed.

## Audit and candidate

Exact centered cell-area integration was not rebuilt. For canonical piecewise
constant or linear waves, raw evaluation is already the exact box average in
every cell that does not cross an edge. Selectively integrating only crossing
cells is therefore mathematically identical to the rejected centered box ADAA
candidate in `canonical-analytic-adaa-round-10-2026-08-30.md`; only its runtime
organization differs.

The distinct probe instead convolved only cells within one phase step of an
edge with the normalized compact aperture

```text
k(u) = 3/4 (1 - u^2), |u| < 1
```

Its exact step integral is cubic. Its exact ramp residual for a slope break is

```text
(3 - 8|u| + 6u^2 - u^4) / 16, |u| < 1.
```

Saw uses one value event, square/pulse use wrap and width events, and triangle
uses its two slope events. Outside those supports, evaluation is the raw
canonical expression. Constant-block reciprocal phase step is hoisted. The
candidate retains only authoritative phase: zero future-ring bytes, no added
latency, allocation, lock, I/O, logging, publication, or stale transition
state. Scalar and x8 outputs matched exactly through pitch, PWM, and reset
changes (`max_abs_error=0`).

## Coherent ideal-projection quality

Values are total ideal-reference error in dB; more negative is better. The
three cells are 110.09 / 872.73 / 6857.14 Hz at 48 kHz.

| Wave | Shipping 1x | Parabolic 1x | Shipping 2x |
|---|---:|---:|---:|
| Saw | -28.330 / -19.270 / -9.614 | **-30.967 / -21.939 / -12.410** | -35.563 / -27.214 / -35.919 |
| Square | -30.076 / -21.224 / -11.104 | **-32.697 / -24.535 / -14.263** | -36.992 / -28.205 / -35.148 |
| Pulse 31% | -30.165 / -21.050 / -12.202 | **-32.928 / -23.726 / -16.054** | -37.136 / -29.177 / -41.064 |
| Triangle | -65.232 / -42.715 / -15.670 | **-66.348 / -47.204 / -20.053** | -45.088 / -44.936 / -39.508 |

The candidate improves shipping 1x by 1.12-4.49 dB, but its compact aperture
cannot approach the 2x discontinuous-wave high-note result. Triangle is the
exception at low/mid pitch: the candidate beats both shipping paths there.

## Off-grid phase and transitions

Sixteen event phases at period 7 exposed the non-cardinal aperture's main
failure. Worst curve RMS was `0.145643` saw, `0.217769` square, `0.181644`
pulse, and `0.065532` triangle, compared with coherent values `0.125833`,
`0.183714`, `0.147617`, and `0.057319`. Worst absolute DC error from the
waveform's analytic mean was `0.006801` saw, `0.013393` square, about `0.00904`
pulse, and `0.001276` triangle. A production DC repair would need
fraction-dependent normalization or compensation and would spend the CPU this
minimal architecture was intended to save.

Rapid 24/32-frame transition results were block-size consistent:

| Wave / event | Shipping local step | Parabolic local step | Change |
|---|---:|---:|---:|
| Saw pitch | 0.84626 | 1.08104 | +27.7% |
| Saw reset | 1.11949 | 1.35356 | +20.9% |
| Square pitch | 1.13959 | 1.37437 | +20.6% |
| Square reset | 1.13782 | 1.37190 | +20.6% |
| Square PWM | 1.08551 | 1.29749 | +19.5% |
| Pulse pitch | 1.13559 | 1.37437 | +21.0% |
| Pulse reset | 1.13782 | 1.37190 | +20.6% |
| Pulse PWM | 2.00000 | 2.00000 | unchanged |
| Triangle pitch | about 0.576 | 0.58667 | about +1.8% |
| Triangle reset | matched | matched | unchanged |

The worse discontinuous-wave event steps alone fail the artifact gate.

## Paired 24/32-frame CPU

Each cell is the median of five paired, alternating-order repetitions over
30,000 real stereo accumulation blocks. Ratios are candidate/current; below
one is faster.

- Scalar won every cell: `0.600-0.921x` for saw/square/pulse and
  `0.629-0.867x` for triangle in the preserved second run.
- x8 was not universal. Saw measured `0.938-1.274x`, losing five of six cells;
  the first run lost all six and reached `1.335x`. Square measured
  `0.777-1.022x`, pulse `0.771-1.038x`, and triangle `0.458-1.042x`.
- The apparent large high-note triangle win is plausible because the current
  BLAMP path is expensive there, but it cannot rescue the cross-wave gate.

Bitwig, its audio/plugin processes, and an eight-vCPU Windows VM remained
active. No competing Cargo or rustc process ran. These CPU values are a coarse
rejection matrix, not promotion-grade timing. The artifact and off-grid
failures are deterministic and make an idle rerun unnecessary.

## Reproduction

The discarded cfg(test) probe used the existing exact Fourier ideal reference,
fractional alignment, wanted complex/magnitude error, DC/peak/step metrics,
shipping 1x/2x renders, 16 off-grid phases, rapid 24/32-frame transitions, and
real scalar/x8 accumulation seams. The exact command was:

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-event-quadrature-target \
CARGO_BUILD_JOBS=1 RUSTFLAGS='-C target-cpu=native' \
cargo test --release --no-default-features --lib \
  oscillators::va::event_quadrature_experiment::event_local_parabolic_quadrature_report \
  --locked -- --ignored --exact --nocapture --test-threads=1
```

The release build and two complete reports passed. The test-only harness was
removed after recording this result because 656 lines of rejected machinery
did not earn a permanent maintenance cost.
