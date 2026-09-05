# KURV

KURV is a small, fast procedural virtual-analog synthesizer built with Truce 6.3.

```text
MIDI note
  -> continuous phase accumulator
  -> sine | triangle | PolyBLEP saw | PolyBLEP pulse
  -> four-point B-spline PolyBLEP / PolyBLAMP antialiasing
  -> 1-64 oscillator unison stack
  -> selectable 1x-4x synthesis quality (2x default)
  -> amp ADHSR with per-stage curvature
  -> output
```

The oscillator calculates every sample directly from continuous phase and corrects each
discontinuity with a four-point B-spline PolyBLEP/PolyBLAMP residual; the 1x and 2x paths use a
shorter optimized residual with matching passband compensation. Two-point PolyBLEP, integrated
Lagrange, and the exact-partial Spectral bank exist in the tree as `#[cfg(test)]` research
backends and are not selectable at runtime.

A quality control selects 1x, 2x, 3x, or 4x synthesis. The oversampled modes pass through steep
fixed-allocation equiripple decimators (97/145/193 taps, 0.05 dB passband ripple, -84 dB stopband)
with passband compensation. Every mode reports the same 33-sample latency, so quality can change
without forcing the host to rebuild its delay graph. A quality change is applied when no voice is
sounding and the decimator tail has drained, so it never clicks; changing it under a held note
takes effect at the next silence.

## Synth engine

- Mono, last-note-priority Legato with glide, or 2-32-note fixed-allocation polyphony with
  quietest/oldest deterministic voice stealing
- 1-64 oscillators per note with an interactive edge-to-center pitch distribution up to ±48 semitones,
  independent center/even/side voice weighting, a continuous Random–Alternate–X stereo triangle,
  per-note stereo randomization, and pitch-only per-lane JITTER
- Sine, triangle, band-limited saw, and variable-width band-limited pulse
- Velocity depth, channel/poly pressure depth, true lower-zone MPE CC74/pitch bend, native CLAP
  MIDI 2 per-note bend and CC74 timbre, sustain pedal, and a shared 1-96-semitone bend range
  (48 default)
- Spring-loaded pitch wheel, +/-12-semitone transpose, and +/-4-octave shift
- Interactive amp ADHSR with literal two-axis timing/level handles for attack, decay, and release
- Sample-accurate MIDI event handling
- Allocation-free audio processing
- 1x/2x/3x/4x oscillator quality with 97/145/193-tap equiripple decimation, silence-deferred
  switching, and constant 33-sample reported latency
- Cached unison ratios and panning, block-read smoothed controls, and an event-segmented fused AVX
  block-major saw renderer for dense held stacks, with exact fallback and no audio-thread heap work

The freely resizable Truce editor opens at 840 x 540, supports 50-200% UI scaling with continuous
auto-fit below the selected scale, and persists the host's logical window size without global-zoom
drift. It splits its oscillator display between a single-cycle waveform rendered
from the same procedural function as the audio engine and an interactive unison field. Drag the
field horizontally for stack width up to 48 semitones and vertically for Vital-style detune-power
distribution from edge-heavy through even to center-heavy. The adjacent triangle continuously blends
Random, Alternate, and X stereo layouts, while each lane's vertical position shows its actual pan and
JITTER visibly moves the lanes using the same deterministic pitch targets as the DSP.
The envelope is a draggable ADHSR contour with a 500 ms minimum timeline whose hollow curve handles
move freely in both axes and match the DSP. The waveform view follows waveform and pulse width;
neither view uses an FFT or captured audio.

## Run and build

```bash
cargo truce run --target-cpu baseline # local standalone only
./scripts/dev-build.sh
```

`scripts/dev-build.sh` is the development build boundary. It always builds the
current checkout in a pinned glibc 2.17 container with the baseline x86-64 CPU
target, rejects binaries requiring newer glibc, publishes the result through
the managed artifact store, and leaves `~/.clap/KURV.clap` and
`~/.vst3/KURV.vst3` as symlinks to the published bundle. Static publishing is
the default:

```bash
./scripts/dev-build.sh
```

The compatibility flags are accepted for older commands but all managed builds
are static portable bundles:

```bash
./scripts/dev-build.sh --static
```

Raw Cargo and `cargo truce` commands link against the development machine and
are local-only. Never distribute or install their Linux output. All Linux DAW
and release bundles go through `dev-build.sh` or `build-linux-release.sh`.
The Linux package supports x86-64 glibc distributions, including Ubuntu LTS and
Void's glibc edition. Void-musl requires a separate musl package.

## Source layout

```text
src/
  lib.rs           Truce plugin entry, DSP state, and parameter plumbing
  shell/           host process() boundary and block dispatch
  runtime/         per-block configuration, event dispatch, and metering
  oscillators/     VA engine (va/), noise, and sample resynthesis (resynth/)
  voices/          fixed-allocation polyphony, unison, ADHSR, and the helper pool
  filters/         SVF, phaser, scream, and the spectral ratio brickwall
  modulators/      LFOs, envelopes, and modulation routing
  generators/      generator stack model and persistence
  wave_curve/      user curve compilation and band-limited curve tables
  params.rs        host-automatable parameters and legacy state migration
  editor*/         egui editor; no editor module touches DSP state directly
```

## Building

`cargo build` needs the private `derpcat-access` licensing crate as a sibling checkout. Without it,
build the synthesis core and run the tests with the licensing backend stubbed out:

```bash
cargo test -p pure_va_dispersion_core --lib --no-default-features --features clap,vst3
```

`scripts/rust-quality.sh` runs the same gate CI enforces: format, lint with `-D warnings` over all
targets, and the test suite. A build without the `licensing` feature is a development build and
must not be distributed.
