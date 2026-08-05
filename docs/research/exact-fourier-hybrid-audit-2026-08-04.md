# Exact-Fourier hybrid oscillator audit

Research report — 2026-08-04

## Decision

Do not replace KURV's current FMA Lagrange oscillator and 2x path globally with direct Fourier
synthesis.

The direct evaluator is mathematically and numerically excellent, and its quality improvement is real:
it removes aliases rather than merely turning up upper harmonics. It does not, however, have one safe
CPU crossover across Saw, Triangle, Pulse, all three morph regions, scalar tails, eight-wide SIMD,
unison, and polyphony. Once pitch automation, harmonic fades, path alignment, fixed latency, and
mixed low/high notes are included, the production change becomes a second synthesis topology rather
than a small oscillator optimization.

If exact Fourier is pursued later, the defensible form is a deliberately adaptive, opt-in quality path:

- use it only when the actual legal harmonic count and current shape family fit a measured work budget;
- retain current Lagrange/Spline plus 2x as the low-note and unsupported fallback;
- fade the top harmonic before it reaches Nyquist;
- crossfade algorithms over roughly one harmonic of pitch range;
- render exact and fallback voices into separate linear buses;
- advance the procedural Fourier evaluation by half a host sample before the existing 33-sample direct
  delay, so it is phase-aligned with the effective 32.5-sample 2x path.

That adaptive design is worth retaining as a quality-mode candidate. It does not earn default-product
complexity after the current FMA optimization.

## Snapshot and isolation

The final comparison used a frozen copy of the live dirty tree after the SIMD Lagrange residuals were
rewritten as FMA Horner chains:

| File | SHA-256 |
|---|---|
| `src/oscillator.rs` | `1183c963251dd057768650595654c3936826b22ead20499b35e9f414b67a9c99` |
| `src/oversampling.rs` | `f16250fcd2cc92be84053e64f29371a5cc6c4b3826efe78b73848cf3ecfd763d` |
| isolated harness `src/main.rs` | `c3811cefd1351e4fefdcd60bba78a803b0cc3d82f641c61e7e14210f0f7ea66d` |
| wrapper-only frozen-source copy | `175b8bbb9fcb8e24a4b56450dbd8c880a6fb9bd45a727fbfc74c167349134199` |

The harness and outputs are under `/tmp/kurv_fourier_audit`. Production DSP was not edited for this
audit. The only frozen-source patch exposes the existing private sine, saw, triangle, and pulse SIMD
evaluators to the harness; it does not change their formulas.

Build command:

```sh
cd /tmp/kurv_fourier_audit
RUSTFLAGS='-C target-cpu=x86-64-v3 -C target-feature=+fma -Awarnings' \
  cargo build --release --quiet
```

Exact source-exposure diff:

```sh
diff -u frozen_fma/oscillator.rs candidate_fma_oscillator.rs
```

## Evaluator architecture

Let `p` be phase, `d` the host-rate phase step, `w` pulse width, and

```text
H = floor((0.5 - epsilon) / d)
```

be the actual number of legal harmonics. `H` is based on the host output band, not the internal 2x
rate. The prototype never executes a fixed eight-partial loop: scalar and SIMD evaluators stop at the
actual maximum `H` in the active lane group.

The evaluator computes one approximate SIMD sine/cosine fundamental using KURV's existing folded
polynomial, then advances harmonics with the complex rotation recurrence:

```text
sin((n+1)t) = sin(nt) cos(t) + cos(nt) sin(t)
cos((n+1)t) = cos(nt) cos(t) - sin(nt) sin(t)
```

Triangle advances by `2t` and visits only odd harmonics. Pulse width uses a scalar sine/cosine
recurrence shared by every SIMD lane. No allocation, lock, logging, I/O, or mutable shared state occurs
in the evaluator.

The coefficient families are:

```text
Saw:
    sin coefficient = -2 / (pi n)

Triangle, odd n only:
    cos coefficient = -8 / (pi^2 n^2)

Pulse:
    DC              = 2w - 1
    sin coefficient = 2(1 - cos(2 pi n w)) / (pi n)
    cos coefficient = 2 sin(2 pi n w) / (pi n)
```

Sine, Triangle, Saw, Pulse, and every existing linear shape morph are evaluated by interpolating these
coefficients before the harmonic is accumulated. Morphing therefore does not evaluate two complete
oscillators.

Complexity is `O(H)` per scalar/SIMD group, `O(ceil(H/2))` for canonical Triangle, fixed stack/register
state, and no audio-thread allocation.

## Fourier-oracle accuracy

The f64 oracle evaluated the same finite series with `f64::sin`/`f64::cos`. Coherent 65,536-sample
renders covered `H = 1, 2, 3, 4, 5, 6, 8, 10, 12, 16`; all canonical families; Sine-Triangle,
Triangle-Saw, and Saw-Pulse morphs; and pulse widths from `0.03` through `0.97`.

| Metric | Worst result through H=16 |
|---|---:|
| Wanted-harmonic amplitude error | `0.000021 dB` |
| Wanted-harmonic phase error | `0.000095 degrees` |
| Full-signal residual relative to oracle | `-129.94 dB` |
| Maximum individual sample error | `1.67e-6` |

Pulse-width extremes at `H=5` remained equally clean:

| Width | Residual vs oracle | Max amplitude error | Max phase error |
|---:|---:|---:|---:|
| 0.03 | -141.54 dB | 0.0000020 dB | 0.0000088 degrees |
| 0.10 | -135.94 dB | 0.0000037 dB | 0.0000088 degrees |
| 0.37 | -137.07 dB | 0.0000052 dB | 0.0000193 degrees |
| 0.50 | -140.78 dB | 0.0000015 dB | 0.0000002 degrees |
| 0.90 | -138.95 dB | 0.0000016 dB | 0.0000033 degrees |
| 0.97 | -140.01 dB | 0.0000025 dB | 0.0000191 degrees |

The f32 recurrence also followed the f64 oracle under continuous controls:

| Motion | Fourier recurrence residual vs f64 oracle |
|---|---:|
| PWM, 11.7 Hz | -133.44 dB relative |
| Morph, 7.3 Hz | -134.91 dB relative |
| 1.2-4.8 kHz pitch sweep | -133.13 dB relative |
| Threshold-crossing vibrato, 5.1 Hz | -134.45 dB relative |

The recurrence is not the limiting error.

## Static aliases and wanted harmonics

The following coherent measurements compare the current FMA Lagrange 2x output with the phase-aligned
Fourier output. Nonwanted energy excludes DC and every legal Fourier harmonic bin. The Fourier path
uses the top-harmonic fade defined below.

| Fundamental | H | Shape | Current 2x nonwanted | Fourier nonwanted | Current max wanted-amplitude error |
|---:|---:|---|---:|---:|---:|
| 3.0 kHz | 7 | Saw | -103.7 dBFS | -165.6 dBFS | 0.83 dB |
| 3.0 kHz | 7 | Pulse 37% | -140.5 dBFS | -147.5 dBFS | 0.82 dB |
| 3.0 kHz | 7 | Triangle-Saw | -109.7 dBFS | -160.9 dBFS | 0.84 dB |
| 4.0 kHz | 6 | Triangle | -89.9 dBFS | -148.1 dBFS | 0.28 dB |
| 4.0 kHz | 6 | Saw | -65.7 dBFS | -145.9 dBFS | 0.31 dB |
| 4.0 kHz | 6 | Pulse 37% | -67.7 dBFS | -140.6 dBFS | 0.31 dB |
| 4.0 kHz | 6 | Triangle-Saw | -71.7 dBFS | -149.2 dBFS | 0.31 dB |
| 4.0 kHz | 6 | Saw-Pulse | -71.7 dBFS | -149.6 dBFS | 0.31 dB |
| 5.5 kHz | 4 | Saw | -65.2 dBFS | -147.6 dBFS | 7.72 dB |
| 5.5 kHz | 4 | Pulse 37% | -63.9 dBFS | -141.4 dBFS | 7.72 dB |
| 8.0 kHz | 2 | Saw | -68.6 dBFS | -150.1 dBFS | 0.31 dB |
| 8.0 kHz | 2 | Pulse 37% | -64.7 dBFS | -144.1 dBFS | 0.31 dB |

This is not only a brightness trick: at 4 and 8 kHz the exact path removes roughly 72-82 dB of
nonwanted saw/pulse energy. However, the 5.5 kHz result also shows why listening alone can overstate
the advance: the exact path restores a legal upper harmonic that the current 2x decimator/correction
chain attenuates by 7.72 dB. Some perceived improvement there is legitimately a brighter wanted
spectrum.

## Required transition behavior

### Harmonic-count changes

Hard `H` changes are invalid for automation. A 4 kHz saw with 18% pitch modulation at 5.1 Hz crossed
between five, six, and seven retained harmonics 203 times in ten seconds. Abruptly inserting/removing
the top partial produced:

| Transition policy | Measured discontinuity |
|---|---:|
| Hard harmonic count | `0.17487` peak, `-15.15 dBFS` |
| Smooth top-harmonic fade | continuous at the count boundary |

The viable fade is a phase-coherent gain on only the top retained harmonic:

```text
clearance = 0.5 / d - H
x = clamp(4 * clearance, 0, 1)
gain = x^2 * (3 - 2x)
```

The partial fades over the final quarter-harmonic of clearance below Nyquist. All lower harmonics stay
at exact amplitude. This is substantially cheaper than recomputing a smooth gain for every partial.

### Direct/2x alignment

The current direct path is delayed by 33 integer host samples. The 2x FIR path is effectively 32.5
host samples at its output sampling instant. Crossfading the unmodified paths therefore creates a
half-sample phase error:

| Frequency/harmonic region | Uncompensated relative phase |
|---|---:|
| 12 kHz fundamental | 45 degrees |
| 8 kHz second harmonic | 60 degrees |
| upper legal harmonics near Nyquist | approximately 90 degrees |

Because the source is procedural, evaluating Fourier at `phase + 0.5 * host_step` before the existing
33-sample direct delay is exact and causal for constant phase step. It reduced the maximum measured
relative phase error to `0.184 degrees` at 3 kHz and below `0.04 degrees` in the higher-note cases.

### Method changes

A hard current-2x/Fourier route switch around `H=5/6` produced a peak `0.040738` transition error, or
`-27.80 dBFS`. A smooth one-harmonic route crossfade reduced the switching discontinuity while the
aligned paths retained `0.99737` correlation in the transition band.

The required route is therefore a stateful crossfade, not an `if H <= threshold` branch.

## Automation and modulation reference

An 8x f64 Fourier signal was low-passed to the host band with a 4097-tap Kaiser FIR and sampled at the
2x path's half-host-sample center. The table reports RMS residual against that reference after edge
transients were removed.

| Motion | Fourier fade + phase lead | Current FMA Lagrange 2x | Fourier improvement |
|---|---:|---:|---:|
| PWM, 11.7 Hz, 3.5 kHz Pulse | -50.84 dBFS | -34.03 dBFS | 16.81 dB |
| Morph, 7.3 Hz, 3.5 kHz | -56.49 dBFS | -40.87 dBFS | 15.62 dB |
| 4 kHz threshold vibrato, 5.1 Hz | -66.81 dBFS | -26.27 dBFS | 40.54 dB |
| 1.2-4.8 kHz pitch sweep | -67.41 dBFS | -28.50 dBFS | 38.91 dB |

The exact path is materially closer under movement, but “exact static Fourier” must not be marketed as
universally alias-free under arbitrary modulation. Direct 1x PWM still left a `-50.84 dBFS` residual
against the oversampled moving reference because modulation creates sidebands outside the stationary
harmonic model.

The final FMA source produced bit-identical motion renders for PWM, pitch, and threshold cases versus
the pre-FMA frozen path; the morph render changed by only `2.23e-8` RMS (`3.58e-7` peak). These motion
figures therefore remain valid for the final FMA baseline.

## x86-64-v3 CPU

Hardware: AMD Ryzen 7 7800X3D. Commands were pinned to logical core 2 and built with
`target-cpu=x86-64-v3,+fma`. Each timing row is the median process-CPU time, so scheduler preemption is
excluded. One benchmark frame is one host output frame including the selected direct delay or 2x
decimator. `H` is the actual retained harmonic count before the small unison detune.

### Threshold cases

| Case | Unison | Current 2x ns/frame | Fourier ns/frame | Delta |
|---|---:|---:|---:|---:|
| Saw, H=1 | 1 | 55.760 | 38.267 | -31.4% |
| Saw, H=1 | 8 | 73.366 | 64.429 | -12.2% |
| Saw, H=1 | 64 | 316.854 | 319.753 | +0.9% |
| Saw, H=5 | 1 | 49.858 | 44.216 | -11.3% |
| Saw, H=5 | 8 | 50.148 | 55.359 | +10.4% |
| Saw, H=5 | 64 | 293.828 | 408.267 | +38.9% |
| Pulse, H=4 | 1 | 41.452 | 36.313 | -12.4% |
| Pulse, H=4 | 8 | 68.168 | 59.383 | -12.9% |
| Pulse, H=4 | 64 | 381.773 | 353.389 | -7.4% |
| Pulse, H=5 | 1 | 40.086 | 39.549 | -1.3% |
| Pulse, H=5 | 8 | 62.840 | 62.140 | -1.1% |
| Pulse, H=5 | 64 | 354.042 | 381.709 | +7.8% |
| Triangle, H=6 | 1 | 37.146 | 31.159 | -16.1% |
| Triangle, H=6 | 8 | 64.465 | 48.738 | -24.4% |
| Triangle, H=6 | 64 | 344.810 | 269.187 | -21.9% |
| Triangle, H=10 | 1 | 54.465 | 50.447 | -7.4% |
| Triangle, H=10 | 8 | 82.011 | 76.942 | -6.2% |
| Triangle, H=10 | 64 | 420.263 | 418.459 | -0.4% |
| Sine-Triangle, H=8 | 1 | 78.203 | 72.197 | -7.7% |
| Sine-Triangle, H=8 | 8 | 105.353 | 99.428 | -5.6% |
| Sine-Triangle, H=8 | 64 | 565.006 | 572.504 | +1.3% |
| Sine-Triangle, H=10 | 1 | 61.674 | 64.645 | +4.8% |
| Sine-Triangle, H=10 | 8 | 77.316 | 83.208 | +7.6% |
| Sine-Triangle, H=10 | 64 | 468.058 | 548.056 | +17.1% |
| Triangle-Saw, H=6 | 1 | 40.746 | 41.361 | +1.5% |
| Triangle-Saw, H=6 | 8 | 83.346 | 64.533 | -22.6% |
| Triangle-Saw, H=6 | 64 | 522.942 | 389.499 | -25.5% |
| Triangle-Saw, H=8 | 1 | 42.547 | 47.479 | +11.6% |
| Triangle-Saw, H=8 | 8 | 79.638 | 67.328 | -15.5% |
| Triangle-Saw, H=8 | 64 | 479.843 | 430.773 | -10.2% |
| Saw-Pulse, H=5 | 1 | 42.190 | 39.497 | -6.4% |
| Saw-Pulse, H=5 | 8 | 86.225 | 62.797 | -27.2% |
| Saw-Pulse, H=5 | 64 | 538.876 | 367.185 | -31.9% |
| Saw-Pulse, H=6 | 1 | 40.195 | 41.918 | +4.3% |
| Saw-Pulse, H=6 | 8 | 83.564 | 59.308 | -29.0% |
| Saw-Pulse, H=6 | 64 | 503.859 | 363.547 | -27.8% |

These data rule out one hard threshold. Scalar tails become slower before wide SIMD groups in several
families, while a 64-oscillator Saw bank becomes slower much earlier than Triangle or morphs.

Conservative crossover centers for an adaptive experiment are:

| Region | Maximum H without a demonstrated broad regression |
|---|---:|
| Canonical Saw | 1 |
| Canonical Pulse | 4 |
| Canonical Triangle | 10, approximately break-even at 64-unison |
| Sine-Triangle morph | 8 |
| Triangle-Saw morph | 6 for scalar safety; SIMD remains favorable at 8 |
| Saw-Pulse morph | 5 for scalar safety; SIMD remains favorable at 6 |

These are route-crossfade centers, not legal hard switches.

### Retired-instruction check

`perf stat -r 5` was used on three 64-unison cases to separate timing noise from actual work:

| Case | Current cycles/frame | Fourier cycles/frame | Cycle delta | Current instructions/frame | Fourier instructions/frame | Instruction delta |
|---|---:|---:|---:|---:|---:|---:|
| Saw, H=5 | 1,092 | 1,432 | +31.1% | 3,282 | 4,175 | +27.2% |
| Triangle, H=6 | 1,730 | 1,317 | -23.9% | 4,381 | 3,963 | -9.5% |
| Triangle-Saw, H=5 | 2,618 | 1,758 | -32.9% | 6,699 | 6,099 | -9.0% |

The Fourier path is a real win in Triangle and expensive morph regions. It is a real regression for a
moderate-partial Saw bank after the FMA work. Averaging those cases into one headline would be
misleading.

### Notes, unison, and polyphony

The following table starts at `H=5`; successive notes are one semitone higher. It shows the strongest
morph win and the Saw counterexample. Times are median process ns per host frame.

| Shape | Notes | Unison | Current 2x | Fourier | Delta |
|---|---:|---:|---:|---:|---:|
| Saw | 1 | 1 | 56 | 46 | -18% |
| Saw | 1 | 8 | 65 | 55 | -16% |
| Saw | 1 | 64 | 232 | 317 | +37% |
| Saw | 4 | 1 | 92 | 81 | -12% |
| Saw | 4 | 8 | 141 | 153 | +8% |
| Saw | 4 | 64 | 812 | 1,062 | +31% |
| Saw | 8 | 1 | 163 | 136 | -17% |
| Saw | 8 | 8 | 230 | 289 | +26% |
| Saw | 8 | 64 | 1,639 | 2,101 | +28% |
| Triangle-Saw | 1 | 1 | 42 | 41 | -1% |
| Triangle-Saw | 1 | 8 | 86 | 61 | -29% |
| Triangle-Saw | 1 | 64 | 509 | 346 | -32% |
| Triangle-Saw | 4 | 1 | 133 | 110 | -17% |
| Triangle-Saw | 4 | 8 | 256 | 179 | -30% |
| Triangle-Saw | 4 | 64 | 1,927 | 1,314 | -32% |
| Triangle-Saw | 8 | 1 | 312 | 200 | -36% |
| Triangle-Saw | 8 | 8 | 515 | 336 | -35% |
| Triangle-Saw | 8 | 64 | 3,833 | 2,541 | -34% |

Dynamic per-note routing does not make the cost disappear. A mixed low/high chord keeps the shared 2x
decimator alive for fallback notes and also executes the direct bus for exact notes. Any note in the
route transition executes both oscillator algorithms. At 64-unison Saw, the exact arm itself is already
slower at useful multi-partial pitches.

## Smallest viable production architecture

The smallest technically correct adaptive implementation would still need four changes:

1. **Oscillator module:** scalar, four-wide, and eight-wide Fourier evaluators plus cached per-bank
   harmonic plans. Plans contain actual lane `H`, top gains, and maximum loop count; they update only
   when phase steps change.
2. **Voice state:** a route target and smoothed route gain per note. Shape-dependent work budgets must
   transition continuously when shape or pitch moves. A transitioning note renders both algorithms.
3. **Two linear synthesis buses:** current voices are summed at 2x and decimated once; exact voices are
   summed at host rate, evaluated with the half-sample phase lead, and passed through the existing
   33-sample direct delay. The two delayed stereo buses are then added.
4. **No global oversampler resets:** per-note route changes cannot call the current global
   `switch_factor()`. Both bus histories must remain live whenever either class of note is active.

In compact form:

```text
for each host frame:
    for each active note:
        route = smooth(shape_work_budget, legal_harmonics)
        exact_bus += route * fourier(note.phase + 0.5 * host_step)
        for each 2x substep:
            fallback_bus[substep] += (1 - route) * lagrange(note.phase)

    output = delay33(exact_bus) + decimate2x(fallback_bus)
```

This is RT-safe and preserves the declared fixed 33-sample latency, but it is not a small patch to the
present one-bus synth/oversampler boundary.

## Final recommendation

- **Reject global exact Fourier.** Low notes are unbounded work, Saw regresses at moderate `H` and
  high unison, automation cannot use hard harmonic or method switches, and the existing one-bus
  oversampler cannot route notes independently.
- **Keep the adaptive prototype.** It is a strong oracle and a credible optional quality mode for high
  Triangle/morph notes. If product scope explicitly values the measured 15-80 dB alias improvement
  over implementation simplicity, implement only the two-bus adaptive design above.
- **Do not call it universally alias-free.** Static wanted harmonics are exact to f32 precision, but
  arbitrary PWM/morph/pitch modulation creates sidebands outside the stationary model; the measured
  PWM residual was still -50.84 dBFS.
- **Prefer the current FMA Lagrange 2x path for the default engine.** It has bounded work, one coherent
  synthesis bus, and no shape-dependent routing state. The Fourier quality gain is real, but the
  default-product CPU/complexity trade does not close.
