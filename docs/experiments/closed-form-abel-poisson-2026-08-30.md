# Closed-form Abel/Poisson oscillator (rejected)

Date: 2026-08-30

Baseline: `3ccd5975d6916aa41b5926bddc95ec8c0412d8c2`

Machine: AMD Ryzen 7 7800X3D, native AVX2/FMA

Verdict: reject; retain only this evidence

## Question and distinct candidate

Could an infinite Fourier series with exponential Abel damping replace KURV's
finite-support 1x oscillator while using less CPU and approaching the ideal
brick-wall projection more closely? The experiment-document audit found no
earlier Abel- or Poisson-summed oscillator. Prior Fourier rounds used finite
harmonic sums, recurrences, or fitted polynomial banks.

For phase angle `theta`, radius `r < 1`, and phase step `d`, the candidate used

```text
saw(theta) = -2 / (pi r) atan2(r sin(theta), 1 - r cos(theta))
r          = exp(-alpha d)
```

The `1/r` normalization makes the fundamental exact. Square used the direct
odd-harmonic identity

```text
square(theta) = 2 / (pi r) atan2(2 r sin(theta), 1 - r^2)
```

Arbitrary pulse used the argument difference between two rotated Poisson
kernels, collapsed to one `atan2`, plus the exact DC term `2 width - 1`. It did
not evaluate two saws. Saw, square, and pulse therefore needed one phase
`sin_cos` and one `atan2` per sample. The x8 path used `wide`'s existing native
vector approximations. Radius and the pulse-width rotation were prepared once
per constant-step block.

The probe selected one `alpha` per shape by minimizing normalized legal-bin
transfer error plus continuous energy above Nyquist over coherent periods 1745,
109, and 7. It summed analytic coefficient energy through harmonic 65,536.
Selected values were 1.580 saw, 1.335 square, and 1.745 pulse. Triangle was not
included: exponential damping of its `1/k^2` odd series requires a
dilogarithm-like evaluator or another finite approximation, so it is not an
equally cheap closed form.

The candidate was stateless, zero-latency, bounded, and allocation-free. It
published the same f32 phase bits as scalar accumulation after alternating real
24- and 32-frame x8 blocks. Scalar/x8 waveform agreement stayed within `5e-5`
over four frequencies and 32,768 phases per frequency.

## Static ideal-projection quality

RMS values compare aligned oscillator plus common oversampler output with the
exact hard-capped Fourier projection. More-negative ideal dB is better.

| wave | Hz | shipping 1x RMS / dB | shipping 2x RMS / dB | Abel RMS / dB |
|---|---:|---:|---:|---:|
| saw | 27.51 | .016720 / -30.76 | .024356 / -27.49 | .018210 / -30.02 |
| saw | 440.37 | .044112 / -22.29 | .017661 / -30.24 | .048555 / -21.46 |
| saw | 6857.14 | .173628 / -9.61 | .008402 / -35.92 | .134451 / -11.84 |
| square | 27.51 | .023514 / -32.57 | .034691 / -29.19 | .023817 / -32.46 |
| square | 440.37 | .059993 / -24.41 | .023545 / -32.53 | .050932 / -25.83 |
| square | 6857.14 | .264299 / -11.10 | .016591 / -35.15 | .122676 / -17.77 |
| pulse 31% | 27.51 | .023637 / -32.53 | .034637 / -29.21 | .026009 / -31.70 |
| pulse 31% | 440.37 | .061932 / -24.13 | .026118 / -31.63 | .067400 / -23.40 |
| pulse 31% | 6857.14 | .230006 / -12.20 | .008292 / -41.06 | .183455 / -14.17 |

Abel improved the current 1x high-note result, especially square, but lost the
low/mid saw and pulse gates and never approached shipping 2x at mid/high pitch.
Its smooth exponential spectrum cannot reproduce a hard Nyquist cutoff. Across
the matrix, theoretical continuous energy above Nyquist was only -45.44 to
-16.82 dB relative to wanted energy. Folded legal-bin alias was -33.60 to
-18.17 dB. At 6.86 kHz, wanted-transfer error was -17.38 dB saw, -19.98 dB
square, and -19.02 dB pulse, while continuous leakage was -16.82, -19.80, and
-18.36 dB respectively.

DC stayed near zero for saw/square and near the pulse target at low/mid pitch.
At 6.86 kHz pulse DC moved from the exact `-0.38` to `-0.36068`. Candidate gain
fell with pitch: .997/.976/.881 saw, .998/.988/.970 square, and
.998/.983/.935 pulse. Fundamental normalization cannot preserve total gain once
legal upper harmonics are attenuated and illegal harmonics fold back.

## Rapid changes, reset, and PWM

The 8,192-sample pitch schedule repeated 440 Hz for 24 frames, 7.04 kHz for 32,
110 Hz for 24, and 12 kHz for 32. Candidate ideal-error RMS improved from
.13303 to .10953 saw, .23505 to .11597 square, and .20112 to .16889 pulse.
Those improvements did not eliminate transition artifacts: maximum event
output steps increased from 1.58757 to 1.67191 saw and from 1.88091 to 2.07793
square and 2.03654 pulse. Pulse's maximum error step also worsened from .66958
to .77351.

The separate 31/20/50/40% PWM schedule was nearly tied in RMS
(.06048 current, .05983 Abel), while Abel peak error was slightly worse
(.35256 versus .34841) and its event output step rose from 2.00000 to 2.00170.
Cold versus warmed-then-reset replay was bit exact for both current and Abel on
all three shapes. Statelessness solves history/reset drift, not the spectral or
CPU gates.

## Real 24/32-frame CPU

The release run included phase gather/publication, eight slightly detuned x8
lanes, stereo accumulation/reduction, and the common factor-1 or factor-2
oversampler. Radius and width preparation ran inside each candidate block.
Ratios are Abel divided by shipping; below one wins.

| wave | Hz | frames | scalar / 1x | x8 / 1x | scalar / 2x | x8 / 2x |
|---|---:|---:|---:|---:|---:|---:|
| saw | 440 | 24 | 3.398 | 2.774 | .877 | .871 |
| saw | 440 | 32 | 3.744 | 2.826 | .946 | .916 |
| saw | 7040 | 24 | 3.612 | 2.712 | .930 | .811 |
| saw | 7040 | 32 | 3.464 | 2.841 | 1.014 | .843 |
| square | 440 | 24 | 3.071 | 1.881 | .777 | .723 |
| square | 440 | 32 | 3.033 | 1.991 | .774 | .777 |
| square | 7040 | 24 | 2.695 | 1.413 | .735 | .472 |
| square | 7040 | 32 | 2.436 | 1.532 | .724 | .491 |
| pulse 31% | 440 | 24 | 3.808 | 2.164 | .955 | .831 |
| pulse 31% | 440 | 32 | 3.776 | 2.359 | .958 | .966 |
| pulse 31% | 7040 | 24 | 3.240 | 1.797 | .939 | .588 |
| pulse 31% | 7040 | 32 | 3.038 | 2.025 | .920 | .687 |

Every shipping-1x cell lost decisively: scalar was 2.44-3.81x slower and x8 was
1.41-2.84x slower. Absolute candidate medians were 14.28-18.89 ns per host
frame scalar and 17.89-24.96 ns x8. It sometimes beat the much costlier 2x
path, but 2x was dramatically more accurate at the pitches where that CPU
comparison looked favorable. The one 1.014 ratio against scalar 2x does not
matter because the universal 1x gate already failed by multiples.

This was a coarse rejection run: CPU 4 affinity, 10,000 blocks per cell, five
medians, and no competing Cargo, rustc, or KURV test process. Desktop/DAW/VM
load was not pristine, so near results would require repetition. None of the
shipping-1x losses is near. The ignored report passed 1/1 with 400 tests
filtered in 3.39 seconds after a clean native release build.

## Decision

Reject the architecture. Abel/Poisson summation is elegant, stateless, smooth
under pitch changes, and materially improves some high-note 1x curves. Its
non-brick-wall leakage is intrinsic, and one transcendental pair per sample is
far too expensive versus KURV's current local polynomial residuals. Assembly
cannot remove the spectral compromise and is unlikely to erase a 1.4-3.8x CPU
gap dominated by vector `atan2`/`sin_cos`.

The temporary 868-line `cfg(test)` probe was removed. No production renderer,
selector, oscillator state, latency, dependency, package version, or published
artifact changed.
