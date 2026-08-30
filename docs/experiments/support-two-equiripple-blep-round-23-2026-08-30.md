# Support-two equiripple BLEP/BLAMP, round 23

Date: 2026-08-30

Status: rejected; production DSP unchanged

Baseline: `f6f53d3` (production DSP unchanged)

## Distinct candidate

The catalog already rejects support-two/support-three least-squares polynomial
BLEPs, 12/24-sample Hann-windowed event kernels, an 11-pole Elliptic BLEP, and
sparse event schedules. This probe instead tests a stateless lookup residual
made from a 63-point sinc at 16x, tapered by a 30 dB Dolph-Chebyshev window.
That window has equal-height frequency sidelobes; integrating its normalized
impulse once produces the BLEP table and integrating the odd BLEP residual
again produces the BLAMP table.

Only nonnegative normalized distances `0..2` are stored at `1/16`-sample
spacing. Linear interpolation and odd/even symmetry reconstruct the residual.
The exact event distance remains `phase / phase_step`, so fractional crossing
time is not quantized to the table grid. Saw, square, and pulse use BLEP edge
corrections; triangle uses the paired BLAMP slope corrections.

This differs from the parallel minimum-phase BLEP work: it has no future-event
ring, retained lane state, warm-up, or reset tail. Its two immutable 33-float
tables total 264 bytes, shared by every oscillator. Runtime work is bounded,
deterministic, allocation-free, lock-free, and I/O-free. The production type,
renderer, version, and latency contract are untouched.

## Gates encoded in the ignored probe

- scalar/x8 parity and finite output over all four canonical shapes;
- ideal bandlimited whole-cycle RMS/peak error, complex wanted-harmonic error,
  off-grid energy, DC, gain, and boundary/global residual steps at coherent
  periods 1745, 109, and 7;
- rapid repeating pitch schedule `440x24 | 7040x32 | 110x24 | 12000x32`;
- real scalar and x8 stereo accumulation at 440 and 7040 Hz in both 24- and
  32-frame blocks, compared with current `SplineOptimized` 1x.

The expected risk is the x8 scalar table gather. The experiment keeps that
cost honest rather than replacing the equiripple residual with another fitted
polynomial before the lookup architecture earns further work.

## Static quality

The candidate improved ideal-bandlimited RMS in all 12 cells. Values below are
current/candidate RMS and ideal-relative error; lower and more-negative are
better.

| Wave | Hz | RMS current / candidate | Ideal dB current / candidate |
|---|---:|---:|---:|
| Saw | 27.507 | .016720 / .016334 | -30.761 / -30.964 |
| Saw | 440.367 | .044112 / .023040 | -22.289 / -27.931 |
| Saw | 6857.143 | .173628 / .084994 | -9.614 / -15.819 |
| Square | 27.507 | .023514 / .022170 | -32.572 / -33.083 |
| Square | 440.367 | .059993 / .023296 | -24.405 / -32.622 |
| Square | 6857.143 | .264299 / .096529 | -11.104 / -19.852 |
| Pulse 31% | 27.507 | .023637 / .022932 | -32.526 / -32.789 |
| Pulse 31% | 440.367 | .061932 / .029425 | -24.130 / -30.594 |
| Pulse 31% | 6857.143 | .230006 / .075073 | -12.202 / -21.927 |
| Triangle | 27.507 | .0007841 / .0007840 | -57.342 / -57.343 |
| Triangle | 440.367 | .001512 / .000355 | -51.640 / -64.216 |
| Triangle | 6857.143 | .094936 / .022071 | -15.670 / -28.342 |

DC stayed effectively unchanged: saw/square/triangle remained within `1.1e-5`
of zero and pulse retained its expected `-0.38` duty-cycle mean. Candidate gain
was closer to unity for every saw/square/pulse cell; triangle high-note gain was
`1.011626` versus current `0.853322`. Mid-note off-grid energy was -67.9 to
-82.4 dB and high-note energy was below -311 dB.

The low coherent cell exposes the first artifact failure. Candidate off-grid
energy was only -31.9 dB saw, -33.9 square, -33.1 pulse, and -57.3 triangle.
Candidate peak error also regressed from `.752724` to `.994799` for saw and
`.808497` to `1.035755` for pulse. Saw/pulse low-note boundary residuals were
`1.184722`/`1.186149`; those are not acceptable trades for the small 0.20/0.26
dB RMS improvements.

## Rapid transitions

The repeating `440x24 | 7040x32 | 110x24 | 12000x32` schedule regressed every
shape. The zero-state architecture has no stale tail, but changing phase step
changes the normalized lookup position abruptly.

| Wave | Peak current / candidate | Global step current / candidate | Pitch-event step current / candidate |
|---|---:|---:|---:|
| Saw | .989539 / 1.099513 | 1.587572 / 1.824414 | 1.587572 / 1.824414 |
| Square | 1.000000 / 1.238855 | 1.880906 / 2.345607 | 1.880906 / 2.345607 |
| Pulse 31% | 1.000000 / 1.234628 | 1.880906 / 2.234713 | 1.880906 / 2.234713 |
| Triangle | .994892 / .997285 | .991178 / 1.123348 | .991178 / 1.065607 |

## Real 24/32-frame CPU

Each row is a five-repeat median from complete oscillator generation and stereo
accumulation before the common oversampler. Ratios are candidate/current; below
one is faster.

| Wave | Hz | Frames | Scalar ratio | x8 ratio |
|---|---:|---:|---:|---:|
| Saw | 440 | 24 | .924 | 1.872 |
| Saw | 440 | 32 | 1.036 | 1.876 |
| Saw | 7040 | 24 | 1.069 | 3.647 |
| Saw | 7040 | 32 | 1.258 | 3.799 |
| Square | 440 | 24 | 1.038 | 1.789 |
| Square | 440 | 32 | 1.036 | 1.809 |
| Square | 7040 | 24 | 1.122 | 2.899 |
| Square | 7040 | 32 | 1.163 | 2.999 |
| Pulse 31% | 440 | 24 | 1.026 | 1.785 |
| Pulse 31% | 440 | 32 | 1.065 | 1.730 |
| Pulse 31% | 7040 | 24 | 1.161 | 3.003 |
| Pulse 31% | 7040 | 32 | 1.168 | 3.589 |
| Triangle | 440 | 24 | 1.126 | 2.339 |
| Triangle | 440 | 32 | 1.283 | 2.417 |
| Triangle | 7040 | 24 | 1.187 | 2.648 |
| Triangle | 7040 | 32 | 1.237 | 2.738 |

Only low-note 24-frame scalar saw won (`-7.59%`). The other 15 scalar cells
lost by 2.56-28.27%, and every x8 cell lost by 73.01-279.89%. Scalarizing the
table gather after an x8 event mask is fatal as lanes become dense. An AVX2
gather cannot rescue the independent transition and low-note artifact failures,
so no native-only follow-up is warranted.

## Verdict

Reject the architecture. It proves that an equiripple short residual can move
the 1x static fidelity frontier substantially, but not while beating current
CPU and artifacts universally. No production source, state, latency, version,
dependency, or selector changed.

## Reproduction

The release build ran pinned on CPU 8 with no competing KURV release process:

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-equiripple-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
taskset -c 8 cargo test --release --no-default-features --lib \
  oscillators::va::experiment::support_two_equiripple_blep_quality_transition_and_cpu_report \
  --locked -- --ignored --nocapture --test-threads=1
```

The command passed 1/1 with 397 tests filtered out. The test build emitted the
checkout's existing 26 warnings and no experiment warning. Scalar/x8 parity,
finite output, endpoint contracts, all static renders, transitions, and timing
cells completed in one invocation.
