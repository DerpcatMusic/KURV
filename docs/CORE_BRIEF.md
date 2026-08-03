# KURV spectral VA core

KURV applies spectral math inside the oscillator, before filtering and amplitude shaping.

## Signal path

```text
MIDI / audition drone
  -> analytic VA spectrum for saw or pulse
  -> pitch-dependent harmonic limit
  -> Phase Disperse | Harmonic Stretch | Formant | Spectral Fold
  -> inverse FFT to a 2048-sample cycle
  -> phase-continuous interpolated playback
  -> resonant low-pass + filter envelope
  -> amp ADSR
  -> output
```

There is no forward FFT because the VA source spectrum is known mathematically. There is no STFT, incoming audio window, overlap-add path, additive oscillator bank, or delay-based disperser. The inverse transform is simply the efficient bridge from modified harmonic coefficients back to the time-domain oscillator cycle.

The current and next cycles are double-buffered and crossfaded. All transform, inverse-transform, and cycle storage is allocated before audio processing.

## Shared spectral controls

```text
Effect   -> selects one of the four coefficient transforms
Amount   -> transform depth
Focus    -> frequency-derived harmonic pivot or spectral center
Shape    -> curvature, stretch, bandwidth, or fold range
Motion   -> slow transform movement
Keytrack -> makes Focus follow played pitch
Stereo   -> left/right oscillator read-phase separation
```

The cycle view uses the same analytic source and spectral transform as the audio oscillator. It is not an oscilloscope or a separate visual approximation.
