# State-free polynomial Fourier, round 21 (2026-08-30)

## Question

Can an x8, constant-step, cap-at-most-six canonical oscillator beat the current
1x local-event renderer by evaluating a table-free sine/cosine polynomial per
sample, deriving harmonics with multiple-angle recurrences, and applying exact
Fourier coefficients? The candidate retained no oscillator recurrence state and
performed no lookup, allocation, lock, I/O, or logging in the render loop.

## Prototype

The test-only prototype quadrant-reduced each lane to `[-pi/4, pi/4]`, evaluated
odd/even sine/cosine polynomials, and formed subsequent harmonics from the
fundamental. It covered saw and 31% pulse in the real constant-block x8 helper;
square and triangle share the same harmonic construction but were not promoted
after the saw/pulse gate failed. Production code was reverted after measurement.

Approximation error against `f64::sin` over a dense cycle was:

| polynomial | fundamental RMS | fundamental peak |
|---|---:|---:|
| degree 5 | 1.1730e-3 | 4.5249e-3 |
| degree 7 | 3.607e-5 | 1.56899e-4 |
| degree 9 | 7.396e-7 | 3.5426e-6 |
| shipping polynomial | 2.260e-8 | 4.179e-8 |

After propagation through six harmonics, degree 7 (the minimum credible CPU
candidate) had the following exact-projection differences:

| shape | RMS | peak |
|---|---:|---:|
| saw | 3.255e-5 | 9.987e-5 |
| square | 5.097e-5 | 1.997e-4 |
| triangle | 4.411e-5 | 1.950e-4 |

Degree 5 was already visibly too inaccurate (shape RMS 1.16e-3 to 1.73e-3).
Degree 9 reduced shape RMS to 6.25e-7 to 1.02e-6 but necessarily adds work to a
degree-7 candidate that already loses. Polynomial/multiple-angle error is inside
the wanted waveform rather than table interpolation, though its nonlinear error
can also create tiny unwanted components bounded here by the total RMS/peak.

## Release CPU

Commands:

```text
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none RUSTFLAGS='-C target-cpu=native' cargo test phase_polynomial_cpu_report --lib --release --locked --no-run
taskset -c 8 target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce oscillators::va::experiment::phase_polynomial_cpu_report --ignored --exact --nocapture --test-threads=1
```

The detected backend was AVX2. Each cell rendered 20,000 64-frame x8 stereo
blocks, five repeats; numbers are ns per host frame. `ratio` is candidate/current.

| phases | shape | Hz/cap | current | candidate | ratio |
|---|---|---:|---:|---:|---:|
| coherent | saw | 7000/3 | 215.936 | 473.702 | 2.194 |
| coherent | saw | 8000/2 | 233.001 | 345.571 | 1.483 |
| coherent | saw | 10000/2 | 224.986 | 379.884 | 1.689 |
| coherent | saw | 12000/1 | 299.314 | 313.132 | 1.046 |
| coherent | pulse | 7000/3 | 343.793 | 660.462 | 1.921 |
| coherent | pulse | 8000/2 | 400.132 | 547.206 | 1.368 |
| coherent | pulse | 10000/2 | 468.162 | 499.143 | 1.066 |
| coherent | pulse | 12000/1 | 519.495 | 301.523 | 0.580 |
| decorrelated | saw | 7000/3 | 275.490 | 518.833 | 1.883 |
| decorrelated | saw | 8000/2 | 266.611 | 409.402 | 1.536 |
| decorrelated | saw | 10000/2 | 274.002 | 401.723 | 1.466 |
| decorrelated | saw | 12000/1 | 373.203 | 312.105 | 0.836 |
| decorrelated | pulse | 7000/3 | 617.121 | 673.308 | 1.091 |
| decorrelated | pulse | 8000/2 | 606.331 | 518.021 | 0.854 |
| decorrelated | pulse | 10000/2 | 611.655 | 513.730 | 0.840 |
| decorrelated | pulse | 12000/1 | 884.955 | 356.944 | 0.403 |

The isolated wins are not a usable common region: saw loses every coherent cell,
and pulse loses coherent cells through 10 kHz. Selecting by whether voices happen
to have coherent phases would be musically unstable. The earlier block recurrence
was materially cheaper because it paid trigonometry once per block; this method
pays quadrant reduction and sine/cosine evaluation for every sample.

## Transitions and limitations

Within a fixed cap the evaluator is stateless, so phase, pitch, and width sweeps
have no retained-state drift or reset seam. A hard cap change still removes a
legal harmonic instantly. Existing exact-additive measurements bound adjacent
sample peaks at cap 3->2 / 2->1 as: saw 0.21221 / 0.31831, square 0.42441 / 0,
pulse37 0.14376 / 0.46408, triangle 0.09006 / 0. Smoothing the top harmonic was
not adopted: prior table-round measurements showed that it attenuates a legal
wanted bin, directly worsening ideal projection accuracy.

The benchmark covers coherent and deliberately decorrelated lanes and the actual
x8 constant-block helper, but not a full DAW session. Square and triangle were
not separately timed after the required saw/pulse phase-robust CPU gate failed.
Memory growth and retained state are zero; only fixed-size vector locals exist.

## Verdict

Rejected. Degree 7 is not accurate enough to justify a regression and is 1.05x
to 2.19x slower for coherent saw; higher degrees cannot repair CPU. The few cap-1
or decorrelated-pulse wins do not define a stable, shape-common production seam,
and cap smoothing would dull legal harmonics. The test-only runtime prototype was
fully reverted; production code and package version are unchanged.
