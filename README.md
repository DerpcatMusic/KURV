# KURV

KURV is a small, fast procedural virtual-analog synthesizer built with Truce 6.3.

```text
MIDI note
  -> continuous phase accumulator
  -> sine | triangle | PolyBLEP saw | PolyBLEP pulse
  -> selectable legacy, B-spline, Lagrange, or exact-partial Spectral antialiasing
  -> 1-64 oscillator unison stack
  -> selectable 1x-4x synthesis quality (2x default)
  -> amp ADHSR with per-stage curvature
  -> output
```

The default oscillator calculates every sample directly from continuous phase. The live selector
preserves the original two-point PolyBLEP path, four-point B-spline PolyBLEP/PolyBLAMP, or a
four-point integrated Lagrange PolyBLEP/PolyBLAMP that retains more wanted upper harmonics. It also
offers a fixed-1x Spectral engine backed by offline-generated exact Fourier-partial tables for very
low aliasing in the mid and high registers; low fundamentals deliberately fall back to the
procedural path. A separate quality control selects 1x, 2x, 3x, or 4x synthesis for the VA engines;
the oversampled modes pass through steep
fixed-allocation equiripple decimators with mild
passband compensation. Every mode reports the same 33-sample latency, so quality can change
without forcing the host to rebuild its delay graph.

## Synth engine

- Mono, last-note-priority Legato with glide, or 2-32-note fixed-allocation polyphony with
  quietest/oldest deterministic voice stealing
- 1-64 oscillators per note with an interactive edge-to-center pitch distribution up to ±48 semitones,
  independent center/even/side voice weighting, a continuous Random–Alternate–X stereo triangle,
  per-note stereo randomization, and pitch-only per-lane JITTER
- Sine, triangle, band-limited saw, and variable-width band-limited pulse
- Live Legacy 2PT / Spline 4PT / Lagrange 4PT / Spectral 1x comparison
- Velocity depth, channel/poly pressure depth, true lower-zone MPE CC74/pitch bend, native CLAP
  MIDI 2 per-note bend and CC74 timbre, sustain pedal, and a shared 1-96-semitone bend range
  (48 default)
- Spring-loaded pitch wheel, +/-12-semitone transpose, and +/-4-octave shift
- Interactive amp ADHSR with literal two-axis timing/level handles for attack, decay, and release
- Sample-accurate MIDI event handling
- Allocation-free audio processing
- Dynamic 1x/2x/3x/4x oscillator quality with 97/145/193-tap equiripple decimation, a click-free
  live transition, and constant 33-sample reported latency
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
cargo truce run
./scripts/dev-build.sh
```

`scripts/dev-build.sh` is the development build boundary. It always builds the
current checkout, publishes the result through the managed artifact store, and
leaves `~/.clap/KURV.clap` and `~/.vst3/KURV.vst3` as symlinks to the published
bundle. Static publishing is the default:

```bash
./scripts/dev-build.sh
```

Use `--hot` when you want the Truce shell and logic watcher; use `--once` when
you only want to publish one hot shell:

```bash
./scripts/dev-build.sh --hot --once
```

KURV's crate uses an optimized development profile, so the hot DSP runs at production-like speed
while retaining incremental rebuilds. DSP changes become audible without touching the DAW. Truce
6.3 custom egui editor changes require closing and reopening the plugin window after the automatic
rebuild; the plugin instance stays loaded.

Changing `KurvParams` itself is not a live-safe edit: the parameter object is allocated by the
shell, while the logic dylib reads that concrete type. After adding, removing, reordering, or
changing a parameter field, rebuild both halves and restart the plug-in host before loading KURV
again:

```bash
./scripts/dev-build.sh --hot --once
```

When validating an installed hot shell, run CLAP tests serially:

```bash
clap-validator validate --no-parallel ~/.clap/KURV.clap
```

Truce 6.3's hot-loader temp names are unique within one host process but can collide across the
parallel subprocesses used by `clap-validator`. Static release bundles do not use the hot loader and
can be validated normally.

Use the same boundary for a static release-style build when hot reload is not wanted:

```bash
./scripts/dev-build.sh --static
```

Raw `cargo truce build` remains available for artifact-only work, but it does not publish to the
host and is not the KURV DAW-testing workflow.

## Source layout

```text
src/
  lib.rs         Truce runtime, parameters, editor layout, and sample-accurate MIDI
  oscillator.rs  procedural VA waveforms, PolyBLEP correction, and the optional Spectral bank
  oversampling.rs dynamic fixed-allocation 1x-4x synthesis and decimation
  voice.rs       fixed-allocation polyphonic voice engine, unison, and ADHSR
```
