# Recursive phase-modulation saw, round 26

Date: 2026-08-30

Baseline: `5a203d7`, KURV `0.8.12`

## Decision

Reject recursive phase modulation (RPM) as KURV's canonical saw generator. Do
not expand it to square or pulse, production DSP, or assembly.

Both exact-sine and KURV-polynomial candidates lose every decisive gate. They
are much less faithful to the ideal band-limited saw, retain large DC, enter
limit cycles, regress rapid pitch and hard reset steps, and cost 3.0-6.9x the
shipping scalar/x8 paths. Exact scalar and x8 sine implementations also diverge
under feedback. No production source, dependency, package version, preset, or
remote changed.

## Candidate and bounded search

The test-only implementation was independently written from the architecture
identified in the round-25 source audit:

```text
y[n] = sin(theta[n] - beta * 0.5 * (y[n-1] + y[n-2]))
```

`theta` used KURV's saw-aligned phase origin. The two measured evaluators were
ordinary `f32::sin`/wide `f32x8::sin` and KURV's existing aligned scalar/x8 sine
polynomial. Each lane retained exactly two `f32` feedback samples (8 bytes) and
had an explicit zero-state reset. The sample kernels were bounded O(1), with no
allocation, lock, I/O, logging, table, or unbounded loop.

For each evaluator, an offline 65-point grid over `0 <= beta <= 4` plus 18
bounded refinements minimized worst fractional-phase normalized shape RMS. The
seven anchors were linearly interpolated in phase step. Every optimum was
interior to the search bound:

| Period | Frequency | Exact beta | Polynomial beta |
| ---: | ---: | ---: | ---: |
| 1745 | 27.51 Hz | 2.9050 | 2.9050 |
| 436 | 110.09 Hz | 2.8134 | 2.8134 |
| 109 | 440.37 Hz | 2.5861 | 2.5861 |
| 55 | 872.73 Hz | 2.3482 | 2.3482 |
| 14 | 3428.57 Hz | 2.2075 | 2.2075 |
| 7 | 6857.14 Hz | 3.0625 | 2.6250 |
| 4 | 12000 Hz | 3.0681 | 3.0681 |

The search oracle already reported poor worst fractional-phase shape RMS:
`0.545-0.967` exact and `0.545-0.967` polynomial. The upper bound did not
truncate a promising optimum.

## Measurement contract

The temporary ignored report measured only saw:

- ideal band-limited projection at coherent periods 1745, 109, and 7;
- aligned RMS/peak, total ideal error, wanted complex error, off-grid energy,
  DC, gain, and raw peak through the same factor-1 output path as shipping;
- the retained one-cycle shape metric at starting phases 0, 0.137, 0.371, and
  0.733, including unaligned error, fitted phase/DC/gain, and residual error;
- repeated-cycle drift after 512 cycles and a 65,536-sample fixed-phase
  limit-cycle check;
- the repeating `440x24 | 7040x32 | 110x24 | 12000x32` pitch schedule, hard
  phase/state reset, and cold-versus-reset replay;
- scalar/x8 recurrence parity, exact x8 phase publication after a real 32-frame
  block, and explicit zero reset;
- paired five-run medians for real scalar and x8 stereo accumulation at 24 and
  32 frames, 440 and 7040 Hz, before the common oversampler.

The first execution accidentally included the common factor-1 oversampler's
fixed delay in the one-cycle metric, causing the shipping control to hit the
metric's half-sample correction bound. The corrected execution measures the
direct oscillator cycle for both control and candidate. That correction changed
only the one-cycle rows; all other rejection evidence was already valid.

## Fidelity and artifacts

Aligned multi-cycle ideal error is in dB; more negative is better.

| Frequency | Shipping | Exact RPM | Polynomial RPM |
| ---: | ---: | ---: | ---: |
| 27.51 Hz | -30.761 | +2.913 | +2.913 |
| 440.37 Hz | -22.289 | +1.451 | +1.451 |
| 6857.14 Hz | -9.614 | +3.255 | +1.796 |

At 27.51/440.37 Hz, exact candidate RMS was `0.8071/0.6785` versus shipping
`0.0167/0.0441`; peak error was `1.7262/1.4774` versus `0.7527/0.2859`. The
polynomial result was numerically the same there. At 6857.14 Hz, exact and
polynomial RMS remained `0.7639` and `0.6459`, both far above shipping `0.1736`.

Candidate DC was `+0.309` to `+0.470`, while shipping stayed within `8e-6` of
zero in the coherent rows. Candidate fitted gain was `1.318-1.556` versus
shipping `0.772-0.999`. Exact off-grid error reached `+1.037 dB` at 6857 Hz;
the polynomial improved that cell to `-8.793 dB`, but its total curve error
still lost by 11.41 dB.

The corrected one-cycle metric confirms that these are waveform errors rather
than nuisance alignment:

| Period | Shipping residual RMS | Exact RPM | Polynomial RPM |
| ---: | ---: | ---: | ---: |
| 1745 | 0.0105-0.0110 | 0.9660-0.9669 | 0.9660-0.9669 |
| 109 | 0.0410-0.0438 | 0.7252-0.8000 | 0.7252-0.8000 |
| 7 | 0.1330-0.1472 | 0.3780-0.5844 | 0.4169-0.8790 |

All four fractional starting phases were included. The period-109 candidate
phase fit saturated the allowed `+0.5` sample in every row, while residual peak
remained `1.56-1.68` versus shipping `0.27-0.31`.

## State, transitions, and parity

The exact candidate drifted by `1.1303` RMS / `1.8221` peak between settled
cycles at period 7. The polynomial drifted by `0.1287` / `0.2138`. Although
periods 1745 and 109 repeated exactly under the coherent phase drive, every
tested period retained a fixed-phase tail delta of `1.80-1.83`, demonstrating a
large two-sample feedback limit cycle rather than convergence.

Rapid-pitch and reset results also regress:

| Metric | Shipping | Exact | Polynomial |
| --- | ---: | ---: | ---: |
| schedule RMS | 0.4753 | 0.7511 | 0.7547 |
| schedule DC | -0.00218 | +0.2180 | +0.2523 |
| global step | 1.5876 | 1.9984 | 1.9988 |
| pitch-event step | 1.5876 | 1.9943 | 1.9935 |
| hard-reset step | 0.4330 | 0.8708 | 0.8708 |

Cold-versus-reset replay was bit exact for both candidates, and the x8 phase
publication was bit exact after 32 frames. The KURV-polynomial scalar/x8
recurrence also matched exactly. Exact scalar and wide-x8 sine differed enough
for feedback to amplify their peak difference to `1.9999` over 4096 samples,
which independently disqualifies that evaluator from a shared scalar/SIMD
contract.

## CPU

Corrected-run medians are nanoseconds per sample. Ratios are candidate/current.

| Evaluator | Hz | Frames | Scalar current -> candidate | Ratio | x8 current -> candidate | Ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| exact | 440 | 24 | 1.640 -> 10.047 | 6.126x | 2.982 -> 11.516 | 3.862x |
| exact | 440 | 32 | 1.639 -> 10.053 | 6.132x | 2.904 -> 11.584 | 3.989x |
| exact | 7040 | 24 | 2.287 -> 11.641 | 5.089x | 3.876 -> 11.623 | 2.999x |
| exact | 7040 | 32 | 2.144 -> 11.731 | 5.471x | 3.769 -> 11.575 | 3.071x |
| polynomial | 440 | 24 | 1.632 -> 11.185 | 6.854x | 2.975 -> 17.165 | 5.769x |
| polynomial | 440 | 32 | 1.657 -> 11.163 | 6.736x | 2.918 -> 17.172 | 5.885x |
| polynomial | 7040 | 24 | 2.146 -> 11.383 | 5.305x | 3.930 -> 16.820 | 4.279x |
| polynomial | 7040 | 32 | 2.164 -> 11.448 | 5.290x | 3.769 -> 17.207 | 4.566x |

The run was pinned to logical CPU 6 on the Ryzen 7 7800X3D while Bitwig and the
Windows VM were active. That is coarse rejection evidence, not near-frontier
timing. The margins are too large and coincide with several independent signal
failures, so no pristine repeat is warranted.

## Reproduction and retained scope

```text
env CARGO_TARGET_DIR=/tmp/kurv-va-rpm-target \
  CARGO_BUILD_JOBS=1 RUSTFLAGS='-C target-cpu=native' \
  cargo test --release --no-default-features --lib --locked \
  oscillators::va::experiment::rpm_round26::recursive_phase_modulation_saw_quality_transition_and_cpu_report \
  --no-run

taskset -c 6 /tmp/kurv-va-rpm-target/release/deps/pure_va_dispersion_core-3854f876eac575fd \
  oscillators::va::experiment::rpm_round26::recursive_phase_modulation_saw_quality_transition_and_cpu_report \
  --exact --ignored --nocapture --test-threads=1
```

The cold build completed in 7m28s, the one-cycle correction rebuilt in 2m54s,
and the corrected report passed `1 passed; 0 failed; 408 filtered out` in
12.14s. Only this evidence report is retained. The test harness was removed;
square, pulse, production integration, version changes, and assembly were never
started.
