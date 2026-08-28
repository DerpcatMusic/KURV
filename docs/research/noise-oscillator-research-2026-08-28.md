# KURV continuous Noise oscillator research (2026-08-28)

## Source truth

- Serum 2's official manual describes its Noise oscillator as a stereo sample player. Its documented color entries expose stereo correlation and low/high filtering, but neither `Radiation` nor a matching procedural algorithm is documented: <https://xferrecords.com/manual/serum-2/docs>
- Faust's maintained noise library provides the relevant procedural building blocks directly: sample-and-hold noise plus sparse random impulses with a controllable average density: <https://github.com/grame-cncm/faustlibraries/blob/master/noises.lib>
- DaisySP's MIT-licensed Noise modules are useful minimal references, not a dependency. `WhiteNoise` is one integer multiply per sample; `Dust` is a Bernoulli impulse source; `ClockedNoise` is sample-and-hold noise with BLEP correction; `Particle` excites a resonant filter with random impulses: <https://github.com/electro-smith/DaisySP/tree/master/Source/Noise>
- Mutable Instruments' archived Shruthi documentation uses one continuous control around white noise: low-pass below center and high-pass above center. This is a precedent for a musical continuum rather than a mode list: <https://github.com/pichenettes/mutable-instruments-diy-archive/blob/main/docs/shruthi/manual.md>

The requested "Radiation" behavior is therefore treated as an audible target—quantized noise becoming sparse random events—not as a Serum algorithm that can be cloned faithfully.

## Accepted model

Noise remains an `OscillatorEngineKind` inside the existing ordered oscillator module. It is generated per polyphonic note, before ordered filters, and never per filter section. Common oscillator level, pan, placement, modulation identity, reset, persistence, and group routing stay shared. Noise is intrinsically single-lane; pitch, unison, and phase controls are omitted because they do not belong to or affect this source.

The smallest honest continuous source has three source controls alongside shared Level and Pan:

1. **Tilt**: a continuous generated spectral slope. Center is white; left darkens toward Brown; right differentiates toward bright blue/violet noise. This is source coloration state, not an inserted ordered filter module.
2. **Gaps**: continuous white noise at zero, progressively amplitude-quantized sample-and-hold noise through the middle, then increasingly sparse random impulses. The held transitions use a bounded smoothing seam so audio-rate modulation cannot emit full-scale discontinuities.
3. **Stereo**: zero reuses one random stream for both channels; one uses decorrelated streams. Normalize the interpolation so the middle does not lose level.

Gaps uses a fixed sample-rate-relative clock, so its character is stable across played notes and no hidden pitch control changes it.

## Rejected designs

- A menu of copied Serum sample names: it is sample playback, not a procedural noise engine.
- A new DSP crate: the required PRNG, sparse gate, hold state, and color state are smaller than a dependency boundary.
- Per-unison noise lanes: they multiply random work without adding a stable source concept. Generate one compact stereo-correlated stream per oscillator/note and shape it once.
- A free-running global noise bus: it breaks per-note/polyphonic modulation and generator ordering.
- Calling filtered white noise "generated without filtering": arbitrary spectral tilt needs state or frequency-domain shaping. The honest efficient implementation keeps that state inside the Noise source rather than adding an ordered filter tile.

## Runtime seam

Own one compact `NoiseState` per oscillator slot in `OscillatorBankVoiceState`: deterministic PRNG state, held values and phase, stereo color state, and click-smoothing state. Do not enlarge every `VaOscillator` lane. One xorshift64* stream supplies the single-lane source and its controllable stereo decorrelation.

Audio-rate modulation may change Tilt, Gaps, Stereo, level, or pan, but it must only update bounded coefficients and counters. No allocation, locks, table rebuilds, logging, or source analysis belongs in `Plugin::process()`.

## Acceptance gates

- Mono at Stereo 0 must be sample-identical left/right; Stereo 1 must be decorrelated without a level jump through the sweep.
- Tilt endpoints need measured spectral slopes; Gaps must move continuously through noise, held/quantized noise, and sparse impulses without NaN/Inf or full-scale clicks.
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

The mono endpoint originally generated a second independent random value per unison lane and discarded it. Specializing the endpoint keeps left/right sample-identical and reduced the 64-lane kernel from 295.53 to 149.96 ns/source-sample. Full stereo retains the two independent streams; intermediate Stereo values retain the normalized continuous crossfade.
