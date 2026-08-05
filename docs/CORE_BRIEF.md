# KURV procedural VA core

KURV starts with the smallest useful virtual-analog instrument: a procedural oscillator, an amp
envelope, MIDI, and output gain.

## Signal path

```text
MIDI
  -> packed continuous phase accumulators
  -> sine | triangle | PolyBLEP saw | PolyBLEP pulse
  -> legacy 2-point or spline 4-point event correction
  -> 1-64 oscillator unison stack
  -> amp ADHSR with per-stage curvature
  -> output
```

Every output sample is calculated from continuous phase. KURV does not use an FFT, a wavetable,
a generated single-cycle buffer, an additive bank, a filter, a spectral transform, or a hidden
post effect.

The Legacy mode preserves the original two-point PolyBLEP saw/pulse path and uncorrected analytic
triangle. Spline mode uses four-point cubic B-spline PolyBLEP on saw/pulse discontinuities and
four-point PolyBLAMP on triangle corners. Lagrange mode uses four-point integrated Lagrange
PolyBLEP/PolyBLAMP for a flatter wanted-harmonic response. All three are table-free, bounded SIMD
paths and can be compared independently at 1x. Sine uses a bounded-error SIMD polynomial. The audio
thread uses fixed voice storage and performs no allocation, locking, logging, or I/O.

## Controls

```text
Waveform    -> Saw, Pulse, Triangle, or Sine
Pulse Width -> pulse duty cycle
Antialiasing -> Legacy 2PT, Spline 4PT, or Lagrange 4PT
Voices      -> oscillators per played note, 1-64
Detune      -> symmetric stack width from 0 to 48 semitones
Distribution-> Vital-style edge/even/center detune-power curve
Stereo      -> equal-power left/right stack spread
Layout      -> pitch-radial X, alternating, or deterministic-random panning
Attack      -> amp-envelope rise time
Hold        -> time retained at full level after attack
Decay       -> amp-envelope decay time
Sustain     -> held amp-envelope level
Release     -> amp-envelope release time
Curves      -> attack, decay, and release temporal curvature
Output      -> final gain
```
