# KURV generator artifact and seam audit

Research report — 2026-08-04

## Verdict

The live generator captured for this audit is numerically stable and value-continuous across its
shape endpoints. The Saw-Pulse fused path is not a hidden alternate waveform: its correction is the
same algebra as crossfading the separately corrected Saw and Pulse endpoints. No production
oscillator change is justified by the seam, overlap, DC, finite-value, or denormal probes.

Two user-visible qualifications remain:

1. Pulse Width is not the requested duty cycle whenever the internal phase step is wider than the
   request. This is an intentional bandwidth guard, but it becomes a large timbral contract change at
   high notes or low oversampling factors.
2. The current live factor switch is value-continuous at the first switched sample but is not signal-
   continuous: it crossfades one frozen old sample into a reset/cold target path for 128 host frames.
   The measured error is large on a continuous sine. A generic fade-to-zero fix improves sine and
   regresses high-DC pulses, so that fix was rejected. A silence-deferred factor policy does win and
   is supplied as an isolated patch.

Hard AA changes are acceptable in the measured live output. The decimation FIR spreads the kernel
change, and settled samples are exactly the selected steady algorithm.

## Live truth and isolation

The audit used a frozen source copy at:

```text
/tmp/kurv-artifact-audit.hcYMwI
```

Capture hashes were:

```text
src/oscillator.rs  2a972e326f86276e967b113ecc8db168fa721bd0b52ac519fe229bca1ad430fe
src/voice.rs       b190b076decc9c2f3e38f7e7b101c41b16a09d6073d3042242dd9e27b0f646ac
src/oversampling.rs b9726e5a7070b0e257261cdb09e391500ca9be677f0eba620d7be41e92c76d0c
```

During the audit, the live `voice.rs` advanced independently with Swarm scheduler work. The
oscillator and oversampler stayed hash-identical, and the changed Swarm code was outside these probes
because every artifact render used zero Swarm amount. No live production file was edited by this
audit.

The temporary diagnostic sources are `examples/artifact_audit.rs` and
`scripts/audit-motion-reference.py` inside the frozen copy. They are audit tools, not production tests.

## Shape endpoints and scalar/SIMD agreement

The shape map is Sine-Triangle over 0-1, Triangle-Saw over 1-2, and Saw-Pulse over 2-3
([`src/oscillator.rs`](../../src/oscillator.rs#L341)). The fused Saw-Pulse path computes:

```text
raw = saw + blend * (pulse - saw)
correction = (2*blend - 1)*wrap_blep - blend*width_blep
```

That is exactly the expansion of `(1-blend)*(saw-wrap_blep) +
blend*(pulse+wrap_blep-width_blep)` ([`src/oscillator.rs`](../../src/oscillator.rs#L571)). It reaches
the exact Saw and Pulse correction formulas at blend 0 and 1.

A 16,384-phase sweep at each boundary, every AA kernel, phase steps `0.001, 0.03, 0.1, 0.24,
0.45`, and widths `0.03, 0.1, 0.5, 0.9, 0.97` found no fixed value jump. With a shape probe epsilon
of `1e-5`, the largest one-sided delta was `2.1696e-5`; it is the expected finite morph slope, not a
boundary offset. The slope changes at 1 and 2 because adjacent linear morph segments have different
endpoint vectors. The product's 8 ms Shape smoothing keeps that control-space corner from becoming a
sample step.

Across 79 million scalar/SIMD comparisons, the worst sample difference was `5.6338e-5` near the 3%
pulse edge at phase `0.030029297`, step `0.001`. This comes from f64 scalar versus f32 SIMD edge
placement and is about -85 dBFS absolute. It is a consistency debt, not a shape seam or stability
fault.

## BLEP overlap, clamp, DC, finite values, and tails

The four-point kernels have support of two phase steps on each side. Their support therefore overlaps
around the cycle once the phase step exceeds `0.25`. Superposition remained bounded; overlap itself
did not create an instability.

The grid covered 900 combinations of four AA variants, phase steps from `1e-12` through the production
`0.45` cap, shapes `0, 1, 2, 2.5, 3`, and all five requested widths. Results:

| Check | Result |
|---|---:|
| NaN or Inf | 0 |
| Worst peak | 1.166666746 |
| Source of worst peak | bounded Lagrange pulse overshoot, step 0.25, width 0.5 |
| Worst DC error for steps >= 0.001 | 6.3e-8 |

The nonzero Pulse DC is correct for an asymmetric pulse: `2*effective_width - 1`. The Saw, Triangle,
and Sine means remained at numerical noise. The Saw-Pulse morph inherits the Pulse DC in proportion
to its blend, also as expected.

Impulse-tail probes of every oversampling factor found no subnormals and finite support:

| Factor | Last nonzero host frame | Smallest nonzero | Subnormals |
|---:|---:|---:|---:|
| 1x | 33 | 1.0 | 0 |
| 2x | 57 | 1.6372e-5 | 0 |
| 3x | 57 | 7.5165e-6 | 0 |
| 4x | 57 | 4.2751e-6 | 0 |

The declared 67-frame decimator tail therefore includes ten exact-zero safety frames after the last
measured FIR output. That margin is what makes an idle reset clean.

## Pulse Width truth at high notes

Pulse and Saw-Pulse clamp the effective duty cycle to:

```text
[max(phase_step, 0.03), 1 - max(phase_step, 0.03)]
```

([`src/oscillator.rs`](../../src/oscillator.rs#L625)). The minimum realizable width is therefore:

| Fundamental | 1x | 2x | 4x |
|---:|---:|---:|---:|
| 3 kHz | 0.06250 | 0.03125 | 0.03000 |
| 8 kHz | 0.16667 | 0.08333 | 0.04167 |
| 12 kHz | 0.25000 | 0.12500 | 0.06250 |
| 20 kHz | 0.41667 | 0.20833 | 0.10417 |

Thus 3% and 97% requests collapse first, followed by 10% and 90%. At 8 kHz, a 3% request rendered
means corresponding to about 16.7%, 8.3%, and 4.2% at 1x, 2x, and 4x. This is not a DC bug: the DC
truthfully reports the clamped waveform.

Removing this clamp is not a low-cost cleanliness fix. It would ask a local BLEP kernel to represent
edge spacing narrower than one internal sample. The production choices are to document the effective
range, raise the factor, or route the high-note case to a genuinely bandlimited representation.

## Phase-step clamp and high-note response

The highest static unison ratio is kept below `0.45`, and every dynamic lane is capped again after its
Swarm ratio ([`src/voice.rs`](../../src/voice.rs#L1039)). For one lane, the corresponding fundamental
ceilings are 21.6, 43.2, and 86.4 kHz at 1x, 2x, and 4x. A requested sweep crossing the cap remains
sample-finite and value-continuous, but its pitch derivative becomes zero: it plateaus rather than
following the request.

At the cap, wide four-point corrections overlap and intentionally attenuate the near-Nyquist
waveform. At step `0.45`, measured Saw peaks were `0.3025` Legacy, `0.1513` Spline, and `0.3488`
Lagrange. This is strong wanted-spectrum coloration, especially for Spline, but not a discontinuity,
DC leak, or runaway.

Representative coherent static output confirms the quality/brightness trade rather than hiding it:

| Fundamental | Wave | Path | Alias residual | Fundamental error vs Sine transfer |
|---:|---|---|---:|---:|
| 8 kHz | Saw | Lagrange 1x / 2x / 4x | -17.1 / -62.8 / -90.0 dBc | -0.14 / -0.01 / -0.00 dB |
| 8 kHz | Saw | Spline 1x / 2x / 4x | -23.9 / -89.1 / -93.6 dBc | -1.60 / -0.45 / -0.10 dB |
| 20 kHz | Saw | Lagrange 1x / 2x / 4x | -39.4 / -54.4 / -92.7 dBc | -3.94 / -0.33 / -0.02 dB |
| 20 kHz | Saw | Spline 1x / 2x / 4x | -51.6 / -81.3 / -97.9 dBc | -10.56 / -2.85 / -0.62 dB |

Spline is cleaner by alias residual and less truthful in wanted amplitude near Nyquist. Lagrange is the
truer-amplitude path in this sweep. That is a real algorithm choice, not evidence of a seam failure.

## Moving controls against the independent reference

The reference evaluates the analytic Fourier series for Sine, Triangle, Saw, arbitrary-width Pulse,
and the same sample-domain shape morph. Control trajectories and phase integration run at 8x in f64;
only legal host-band partials are retained, with a smooth top-partial fade. The f64 trapezoidal phase
integral converged between 8x and 16x at -121.6 dB or better for Shape and below -144.9 dB for the
pitch trajectories.

After optimizing only fractional latency and one global gain, current Lagrange 2x produced:

| Motion | Reference-relative residual | Peak error |
|---|---:|---:|
| Shape 0-3 at 1.37 Hz, 4 kHz carrier | -18.37 dB | 0.252 |
| PWM 0.03-0.97 at 7.3 Hz, 3 kHz carrier | -17.40 dB | 0.513 |
| Pitch 0.2-8 kHz | -22.31 dB | 0.787 |
| High pitch 2-20 kHz | -14.43 dB | 0.693 |
| Combined shape/PWM/0.5-6 kHz | -22.23 dB | 0.662 |

These residuals are not all clicks or aliases. They include the designed Lagrange kernel response,
upper-partial phase/amplitude differences, zero-order-held host controls across the two internal
samples, and Pulse Width clamping. They do show that the current path is not an exact moving
bandlimited oracle, particularly in the 2-20 kHz sweep. No small seam patch fixes that; it is the
quality frontier already identified by the exact-Fourier experiment.

## Live AA changes

The AA probe held 2x, 8 kHz Saw and changed Legacy to Spline to Lagrange to Legacy. The oscillator
kernel selection changes immediately, while only the Spline-2x output correction has an explicit
128-frame ramp. The 97-tap decimator nonetheless spreads the source-kernel change.

| Change | Peak derivative relative to steady source | Settled target mismatch |
|---|---:|---:|
| Legacy to Spline | 1.017x | 0 samples |
| Spline to Lagrange | 1.093x | 0 samples |
| Lagrange to Legacy | 1.007x | 0 samples |

The 9.3% worst derivative increase does not justify duplicating oscillator evaluation or adding a new
AA transition state. After settling, the render becomes bit-identical to the selected steady path.

## Factor switching: confirmed artifact and accepted policy

The live switch resets only the newly selected path, captures `last_output`, and fades from that one
frozen value for 128 frames ([`src/oversampling.rs`](../../src/oversampling.rs#L77),
[`src/oversampling.rs`](../../src/oversampling.rs#L122)). Against a continuous 1 kHz sine for
2x-to-1x, 1x-to-4x, and 4x-to-2x changes:

| Metric | Live range |
|---|---:|
| 128-frame RMS error | 0.731-0.747 |
| Peak error | 1.779-1.808 |

The first switched sample repeats the previous value, so a one-sample click detector underrates the
problem. The artifact is a 2.67 ms frozen-waveform/cold-path burst.

A latency-aware output fade was explored in a separate snapshot. It improved sine RMS error by about
35% and preserved every steady sample, but regressed the 3%-Pulse 8 kHz switch by 24-59% RMS because
fading toward zero destroys legitimate Pulse DC. It was rejected.

The accepted low-cost policy is:

- while a voice or decimator tail is active, keep rendering the actual factor unchanged;
- when `!synth.is_active() && decimator_tail == 0`, reset directly to the requested factor;
- derive AA from the actual factor while deferred;
- on the idle reset, apply the Spline-2x correction immediately;
- if the tail reaches zero mid-block, complete the reset and recompute AA before any later sample.

Measured with a request at frame 1000 and note release at frame 4000, the policy switched at frame
4066, the first real idle point. It had zero mismatches versus an unchanged-2x control before the
switch. A reset used path and a fresh path had zero mismatches over 256 startup frames; 1x produced
its first nonzero at the ordinary 33-sample latency, with no 128-frame residue.

The winning isolated candidate is:

```text
/tmp/kurv-deferred-factor.EsbWJC
/tmp/kurv-deferred-factor.EsbWJC/deferred-factor-switch.patch
```

It also removes the now-unreachable frozen transition fields and branch.

## CPU and exact unaffected paths

The benchmark is the expensive Saw-Pulse morph at MIDI 93, width 0.1, Lagrange 2x, 250,000 host
frames, nine repeats:

| Unison | Live median ns/frame | Deferred candidate | Difference |
|---:|---:|---:|---:|
| 1 | 90.679 | 90.954 | +0.3% |
| 8 | 128.103 | 126.132 | -1.5% |
| 64 | 540.839 | 541.943 | +0.2% |

The spread is benchmark noise; the policy is CPU-neutral in normal active rendering. Four
representative 65,536-frame renders—Saw, Triangle, Pulse 3%, and Pulse 50% at Lagrange 2x/8 kHz—were
byte-for-byte identical between live and candidate. Checksums were also identical at 1, 8, and 64
unison.

Verification completed without adding or running tests:

```text
cargo check --lib
cargo build --release --lib
cargo build --release --example generator_lab
cargo run --release --example deferred_factor_audit
```
