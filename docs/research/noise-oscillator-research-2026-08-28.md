# KURV continuous Noise oscillator research (2026-08-28)

## Source truth

- Serum 2's official manual exposes procedural White, Pink, Brown, and Geiger colors. White/Pink/Brown are spectral slopes; Geiger is sparse random clicks. Serum's other Noise entries are mono/stereo WAV playback, not additional procedural engines. Its color modes add a stereo-correlation control and a bipolar low/high-pass control: <https://xferrecords.com/manual/serum-2/docs>
- DaisySP's MIT-licensed Noise modules are useful minimal references, not a dependency. `WhiteNoise` is one integer multiply per sample; `Dust` is a Bernoulli impulse source; `ClockedNoise` is sample-and-hold noise with BLEP correction; `Particle` excites a resonant filter with random impulses: <https://github.com/electro-smith/DaisySP/tree/master/Source/Noise>
- Mutable Instruments' archived Shruthi documentation uses one continuous control around white noise: low-pass below center and high-pass above center. This is a precedent for a musical continuum rather than a mode list: <https://github.com/pichenettes/mutable-instruments-diy-archive/blob/main/docs/shruthi/manual.md>

The user's remembered "Radiation" category appears to be the audible idea represented by Serum's Geiger source, not an official Serum 2 engine name.

## Accepted model

Noise remains an `OscillatorEngineKind` inside the existing ordered oscillator module. It is generated per polyphonic note, before ordered filters, and never per filter section. Common oscillator level, pan, pitch, unison, placement, modulation identity, reset, persistence, and group routing stay shared.

The smallest honest continuous source has three source controls:

1. **Color**: a continuous generated spectral slope. Center is white; left darkens toward Brown; right differentiates toward bright blue/violet noise. This is source coloration state, not an inserted ordered filter module.
2. **Texture**: continuous white noise at zero, progressively clocked/sample-held noise through the middle, then increasingly sparse Geiger-like impulses. The held transitions require bandlimiting or a bounded smoothing seam so audio-rate modulation cannot emit full-scale discontinuities.
3. **Stereo**: zero reuses one random stream for both channels; one uses decorrelated streams. Normalize the interpolation so the middle does not lose level.

Pitch controls the clock/hold rate when Texture leaves continuous noise. This gives transpose/fine a real meaning instead of leaving shared controls inert. Phase controls reseed/start position only if that behavior is deterministic and audible; otherwise the Noise card should label them honestly rather than pretending noise has periodic phase.

## Rejected designs

- A menu of copied Serum sample names: it is sample playback, not a procedural noise engine.
- A new DSP crate: the required PRNG, sparse gate, hold state, and color state are smaller than a dependency boundary.
- Per-unison colored-noise filter banks: they multiply state and CPU. Generate lane samples from one compact stream, sum them through the existing unison spatial gains, then color the resulting stereo pair once per oscillator/note.
- A free-running global noise bus: it breaks per-note/polyphonic modulation and generator ordering.
- Calling filtered white noise "generated without filtering": arbitrary spectral tilt needs state or frequency-domain shaping. The honest efficient implementation keeps that state inside the Noise source rather than adding an ordered filter tile.

## Runtime seam

Own one compact `NoiseState` per oscillator slot in `OscillatorBankVoiceState`: deterministic PRNG state, held values and phase, stereo color state, and click-smoothing state. Do not enlarge every `VaOscillator` lane. One xorshift64* stream avoids 64 persistent PRNG states while preserving deterministic unison decorrelation.

Audio-rate modulation may change Color, Texture, Stereo, level, pan, or clock rate, but it must only update bounded coefficients and counters. No allocation, locks, table rebuilds, logging, or source analysis belongs in `Plugin::process()`.

## Acceptance gates

- Mono at Stereo 0 must be sample-identical left/right; Stereo 1 must be decorrelated without a level jump through the sweep.
- Color endpoints need measured spectral slopes; Texture must move continuously through noise, held/quantized noise, and sparse impulses without NaN/Inf or full-scale clicks.
- One Noise oscillator must not allocate or scale state with maximum unison count.
- The card preview must sample the same deterministic evaluator or show a stable statistical envelope, never a decorative unrelated waveform.
- Benchmark scalar and 8-lane counter hashing against the existing VA oscillator hot path; keep only a measured winner.

## Measured implementation results

Pinned release kernel results on the development machine:

| Rendered lanes | xorshift64* | `wide::u32x8` candidate | xoroshiro128++ candidate |
|---:|---:|---:|---:|
| 1 | 13.84 ns/sample | no material win | 14.16 ns/sample |
| 8 | 66.09 ns/sample | no material win | 69.06 ns/sample |
| 64 | 481.41 ns/sample | no material win | 505.89 ns/sample |

Both alternatives were rejected. The integer SIMD candidate saved less than 1% while enlarging state; the official xoroshiro128++ reference was 2–5% slower. The retained xorshift64* stream is the smallest measured winner.

The first integrated implementation missed KURV's settled block renderer. Routing Noise through that existing block path reduced the 64-frame `Plugin::process()` profile from 3,952 to 2,078 ns/frame at 64 polyphonic voices (47%), and from 264 to 215 ns/frame at one voice (19%). The block path explicitly excludes Noise from VA-only packing predicates, so it does not reinterpret Noise as a wavetable oscillator.
