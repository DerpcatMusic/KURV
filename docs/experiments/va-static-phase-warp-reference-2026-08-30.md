# Static phase-warp VA reference baseline

Status: **successful test infrastructure; exposes a scalar pulse-edge CPU target**

## Contract

The ignored release report exercises
`VaOscillator::generate_shape_step_warped` followed by the production
`StereoOversampler`. It covers saw, square, 31% pulse, and triangle with PWM,
phase-bend, and harmonic warp at 50% and 100% depth, at 872.727 and 6857.143 Hz,
and at factors 1x and 2x.

The reference is not an unwarped oscillator or a brightness heuristic. For each
case it densely samples the exact continuous phase mapping used by playback,
evaluates the corresponding raw canonical curve, transforms that cycle, keeps
only harmonics strictly below Nyquist, and compares production output after
fractional-cycle alignment. Reported metrics include curve RMS/peak, wanted-bin
amplitude error, total ideal-projection error, DC, gain, boundary/global residual
steps, and paired real-render CPU.

All FFTs, vectors, alignment, and timing storage are test-only. Production DSP
and the audio-thread state are unchanged.

## Reproduce

```text
cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_static_phase_warp_quality_and_cpu_report \
  --locked -- --ignored --nocapture --test-threads=1
```

The retained run passed 1/1 in 6.71 seconds after the release build.

## Representative 48 kHz findings

At 6857.143 Hz, total 1x ideal-projection error remained large:

| Wave | Static warp range at 100% depth |
|---|---:|
| Saw | -9.80 to -12.32 dB |
| Square | -11.08 to -11.80 dB |
| Pulse 31% | -11.34 to -12.52 dB |
| Triangle | -12.90 to -18.37 dB |

The corresponding 2x ranges were -28.63 to -39.52 dB saw, -35.15 to
-36.91 dB square, -38.07 to -41.42 dB pulse, and -27.87 to -35.39 dB
triangle. Static phase warp therefore does not overturn the earlier finding:
the current 1x path is much cheaper, but its high-note result is not close to
the ideal projection.

The aligned saw RMS is effectively unchanged by static warp because playback
evaluates the smooth mapped phase directly and the remaining error is dominated
by the same raw-cycle BLEP residual. Square under phase-bend and harmonic warp
is mathematically unchanged at 50% duty: both mappings preserve its two edge
phases. PWM moves those edges and changes duty/DC as designed. Nonzero DC in
warped pulse and triangle cases can likewise be part of the intended mapped
curve; the reference includes it rather than treating all DC as an artifact.

## CPU discovery

Median warped/unwarped scalar-step ratios at 440 Hz were:

| Wave | 1x ratio range | 2x ratio range |
|---|---:|---:|
| Saw | 2.35-2.51x | 1.55-1.64x |
| Square | 8.97-13.54x | 5.33-5.55x |
| Pulse 31% | 9.23-13.20x | 5.27-5.99x |
| Triangle | 1.94-2.08x | 1.47-1.55x |

The square/pulse outlier comes from the scalar warped path computing the inverse
warped pulse-edge phase on every sample. That calculation performs three Newton
iterations and trigonometric warp evaluations even though phase step, width,
mode, and amount are block-stable in the common one-lane block renderer. SIMD
constant-block paths already prepare their warp outside the sample loop.

This is a measured optimization target, not yet a repair. The next experiment
must move or eliminate only the redundant inverse-edge preparation while
preserving exact pulse-edge timing under pitch, width, shape, and warp changes.

## Limits

This baseline covers stationary periodic warp. It does not yet provide an ideal
reference for audio-rate warp depth, PM/FM, sync, or arbitrary within-block
parameter motion. Those cases need a time-varying high-rate reference rather
than a periodic harmonic projection.
