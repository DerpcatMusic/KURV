# Fractional-delay feedback-loop saw, round 29

Date: 2026-08-30

Baseline: `f950be6`

## Decision

Reject the fractional-delay feedback-loop impulse-train saw as KURV's
canonical oscillator. Do not expand it to square, pulse, triangle, a dual-read
pitch-crossfade implementation, production DSP, or assembly.

The favorable static case improved fitted one-cycle shape and high-note total
error, and its common-pitch x8 loop was faster than the shipping spline path.
It nevertheless failed the universal gates: static output retained
`+0.071` to `+0.082` DC, low/mid total ideal error regressed, mid/high
off-grid energy regressed, every settled coherent cycle drifted, and the
one-pointer loop became unusable under rapid pitch changes (`298.3` peak,
`248.9` RMS, and `239.6` DC). No production source, dependency, package
version, preset, or remote changed.

## Primary source and bounded candidate

The test-only candidate was independently written from the architecture and
equations in Nam, Valimaki, Abel, and Smith,
[Alias-Free Virtual Analog Oscillators Using a Feedback Delay Loop](https://www.dafx.de/paper-archive/2009/papers/paper_72.pdf),
DAFx-09. It used:

- one impulse circulating in a fixed-size feedback ring;
- the paper's first-order Thiran allpass,
  `y = a*x + x1 - a*y1`;
- the high-frequency coefficient
  `a = sin((1-D)*w0/2) / sin((1+D)*w0/2)`;
- an integer/fractional delay split over a fixed maximum period;
- subtraction of the impulse train's `step` DC before integration;
- a leaky integrator whose leak was scaled to preserve `0.005` loss per
  period, rather than applying the paper example's fixed loss every sample.

The period-scaled leak is deliberately candidate-favorable at low notes. The
coherent quality periods are favorable too: their exact integer delay reduces
the fractional part to `D = 1` and the allpass coefficient to zero. The real
440/7040 Hz CPU cells exercise fractional delay.

Scalar and x8 candidates used bounded, fixed-capacity rings with explicit zero
reset. Allocation occurred only when constructing the test object; the sample
loop had no allocation, resize, lock, I/O, logging, or unbounded work. The
scalar ring payload was 8,192 bytes and the x8 ring payload was 65,536 bytes.
The inline state around those heap allocations was 56 and 288 bytes,
respectively.

The minimum one-pointer architecture changes the read delay and allpass
coefficient directly on pitch changes. This is the exact low-cost seam that
needed to clear before considering the paper's two-pointer crossfade. The
paper itself says the loop is not exactly periodic and describes both pitch
change clicks and high-frequency attenuation from the required crossfade.

## Measurement contract

The temporary ignored release report measured only saw:

- ideal band-limited projection at coherent periods 1745, 109, and 7;
- aligned RMS/peak, total ideal error, wanted-bin complex error, off-grid
  energy, DC, fitted gain, and raw peak;
- the retained one-cycle shape fit at starting phases 0, 0.137, 0.371, and
  0.733, including fitted phase/DC/gain and residual RMS/peak;
- cycle-to-cycle drift after 512 periods;
- the repeating `440x24 | 7040x32 | 110x24 | 12000x32` pitch schedule,
  hard reset, and cold-versus-reset replay;
- bit-exact scalar/x8 output and logical-phase parity at a common phase step;
- paired real scalar and x8 stereo accumulation at 24 and 32 frames, 440 and
  7040 Hz, before the common oversampler.

## Static fidelity and artifacts

Total, wanted, and off-grid errors are dB; more negative is better.

| Period | Hz | Metric | Shipping | Feedback loop |
|---:|---:|---|---:|---:|
| 1745 | 27.51 | RMS error | 0.016722 | 0.071952 |
|  |  | peak error | 0.750752 | 0.170676 |
|  |  | total error | -30.760 | -18.085 |
|  |  | wanted error | -33.058 | -42.325 |
|  |  | off-grid | -34.622 | -44.826 |
|  |  | DC | -0.000008 | +0.071740 |
|  |  | fitted gain | 0.99913 | 1.00788 |
|  |  | raw peak | 0.9977 | 1.0758 |
| 109 | 440.37 | RMS error | 0.044109 | 0.073446 |
|  |  | peak error | 0.285860 | 0.171910 |
|  |  | total error | -22.289 | -17.861 |
|  |  | wanted error | -22.290 | -30.264 |
|  |  | off-grid | -71.396 | -44.642 |
|  |  | DC | approximately 0 | +0.071224 |
|  |  | fitted gain | 0.98628 | 1.01307 |
|  |  | raw peak | 0.9634 | 1.0707 |
| 7 | 6857.14 | RMS error | 0.173628 | 0.106324 |
|  |  | peak error | 0.290918 | 0.185881 |
|  |  | total error | -9.614 | -13.874 |
|  |  | wanted error | -9.614 | -17.753 |
|  |  | off-grid | -314.475 | -42.879 |
|  |  | DC | approximately 0 | +0.081627 |
|  |  | fitted gain | 0.77190 | 1.09929 |
|  |  | raw peak | 0.6092 | 0.9447 |

The candidate's fitted residual shape was genuinely promising. Across the four
fractional starting phases, residual RMS/peak was approximately
`0.00441/0.0950` at period 1745, `0.0173/0.089` at period 109, and
`0.0479/0.0823` at period 7. These residuals are lower than the retained
shipping-control ranges from round 26. That fit removes phase, DC, and gain,
however; it cannot excuse the actual output's large DC, its low/mid total
error, or its off-grid regressions. The high-note total improvement is only
4.26 dB, short of the 10 dB candidate gate.

## Periodicity, pitch changes, and reset

The candidate never became exactly periodic. Between settled cycles after 512
periods, RMS/peak drift was:

| Period | drift RMS | drift peak |
|---:|---:|---:|
| 1745 | 0.000385965 | 0.000386834 |
| 109 | 0.000391847 | 0.000392973 |
| 7 | 0.000439825 | 0.000441134 |

The rapid schedule was a hard failure:

| Metric | Shipping | Feedback loop |
|---|---:|---:|
| schedule peak | 0.9895 | 298.3226 |
| schedule RMS | 0.4753 | 248.9377 |
| schedule DC | -0.00218 | 239.5683 |
| global adjacent step | 1.5876 | 1.2121 |
| pitch-event step | 1.5876 | 0.7963 |

The smaller isolated adjacent-step numbers do not rescue a signal whose stale
loop and integrator energy runs hundreds of times outside the waveform range.
Cold-versus-reset replay was bit exact, and the common-step scalar/x8 output
and logical phases were bit exact. Reset correctness therefore does not explain
or remove the pitch-transition failure.

## CPU

The focused report ran pinned to logical CPU 8. Values are median nanoseconds
per sample; ratios are candidate/current.

| Hz | Frames | Scalar current -> candidate | Ratio | x8 current -> candidate | Ratio |
|---:|---:|---:|---:|---:|---:|
| 440 | 24 | 1.816 -> 2.031 | 1.118x | 3.334 -> 2.097 | 0.629x |
| 440 | 32 | 1.602 -> 1.646 | 1.028x | 2.929 -> 2.080 | 0.710x |
| 7040 | 24 | 2.063 -> 1.624 | 0.787x | 3.929 -> 2.144 | 0.546x |
| 7040 | 32 | 2.077 -> 1.619 | 0.780x | 3.808 -> 2.094 | 0.550x |

Scalar lost both 440 Hz cells and won both 7040 Hz cells. The common-step x8
probe won by 29-45%. This is an optimistic lower bound: a production unison
renderer has decorrelated lane pitches, while the micro-gate splatted one step
to match the existing fair x8 harness. Independent fractional delays would
require lane-wise scattered reads or eight separately addressed rings. The
candidate is already rejected on signal gates, so expanding that memory-access
model or repeating near-frontier timing would not change the decision.

## Reproduction and retained scope

```text
env CARGO_TARGET_DIR=/tmp/kurv-va-fdl-target \
  CARGO_BUILD_JOBS=1 RUSTFLAGS='-C target-cpu=native' \
  cargo test --release --no-default-features --lib --locked \
  oscillators::va::experiment::feedback_delay_round29::fractional_delay_feedback_saw_quality_transition_and_cpu_report \
  --no-run

taskset -c 8 /tmp/kurv-va-fdl-target/release/deps/pure_va_dispersion_core-e9180371540f3547 \
  oscillators::va::experiment::feedback_delay_round29::fractional_delay_feedback_saw_quality_transition_and_cpu_report \
  --exact --ignored --nocapture --test-threads=1
```

The cold focused release build completed in 7m51s. The report passed `1 passed;
0 failed; 411 filtered out` in 0.72s. Existing warnings were unchanged. Only
this evidence report is retained: the test implementation was removed, and no
other waveform, dual-loop transition architecture, production integration,
version change, or push was started.
