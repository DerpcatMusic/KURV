# RAPID sample-resynthesis: observable contract

Date: 2026-08-26
Scope: public behavior KURV may honestly call “RAPID-like”; not a reconstruction of Parawave's proprietary DSP.

## Decision

At the public evidence boundary, “RAPID-like Rich” means **offline import turns a short source's spectral character into a new, keyboard-playable multisample that can produce moving, indefinitely sustaining textures**. Parawave does not publish how literal speech, rhythm, or the source timeline survives that conversion.

For KURV, the behavioral target is therefore:

- import/analysis happens before playback and publishes a playable artifact;
- the artifact produces a perceptually seamless, indefinitely sustaining evolving texture even when the input has no loop;
- every held MIDI note plays that texture at the note's pitch, so chords are independently tuned rather than octave-snapped grains;
- note-on starts deterministically by default; start-position randomization is optional behavior, not mandatory RAPID parity;
- source spectrum and movement inform timbre, while literal words, rhythm, duration, and melodic contour are not preserved;
- normalization is a separate KURV policy because Parawave publishes no resynthesis loudness contract; and
- runtime work should resemble bounded sample-oscillator playback, with analysis/reconstruction kept off the audio thread. That is a KURV engineering interpretation of RAPID's generated-multisample workflow, not a claim about RAPID internals.

## First-party evidence

| Area | Published behavior | Contract boundary |
|---|---|---|
| Import | A user creates/selects a User Library, adds files with **Add Files**, a drop area, or oscillator drag-and-drop, enters a name/category, chooses an import method, and imports. RAPID 1.1 added RIFF WAV and AIFF import at any input sample rate. [Manual pp. 49–55](https://parawave-audio.com/public-download/Rapid.Synthesizer.User.Manual.pdf), [version history, 1.1.0](https://parawave-audio.com/rapid_version_history), [FAQ](https://parawave-audio.com/faq) | This is asset creation, not a live audio-input effect. Exact accepted bit depths/channel layouts are not published. |
| Analysis result | Parawave says Sample Resynthesis “converts the spectral content of an input sample into a multi-sample”; its launch note calls it creation of synthetic textures by manipulating source spectral content. [Version history, 1.1.0](https://parawave-audio.com/rapid_version_history), [official timeline, 26 Aug 2017](https://parawave-audio.com/timeline) | “Spectral” is observable product language. It does not disclose FFT/STFT, additive partials, phase vocoding, or any other algorithm. |
| Suitable inputs | Best results are documented for **1–4 second** sources. The input need not be looped. Monophonic, fixed-pitch material without sweeps or fast pitch modulation gives less noisy results. Short sources become grainier; long sources become airier/noisier. [Manual p. 55](https://parawave-audio.com/public-download/Rapid.Synthesizer.User.Manual.pdf) | RAPID does not promise faithful results for arbitrary songs, speech, polyphony, or moving melodies. |
| Sustain and timbre | In Parawave's own support post, a hi-hat resynthesis becomes an “infinite metal texture”; an ideal-length source yields smooth textures with interesting spectral movements; fixed vowels make synth choirs and filtered plucks make pads. [Parawave developer guide, 27 Aug 2017](https://www.kvraudio.com/forum/viewtopic.php?p=6860398#p6860398) | The examples support infinite/evolving texture for suitable inputs. Exact generated loop length, loop-point placement, seam algorithm, and whether every possible input is click-free are not disclosed. |
| Keyboard pitch | Resynthesis asks for a Root Note/MIDI unity note that gives **1:1 pitch playback at that note**. RAPID's tuning changes oscillator cycle rate and keyboard tracking can be disabled. A later fix explicitly refers to resynthesized samples at very low root-note resampling ratios. [Manual pp. 38, 55](https://parawave-audio.com/public-download/Rapid.Synthesizer.User.Manual.pdf), [version history, 1.8.5](https://parawave-audio.com/rapid_version_history) | The output is pitched sample-oscillator material. Zone count, interpolation, pitch detector, antialiasing method, and formant behavior are undisclosed. Root pitch is user input, not documented automatic note locking. |
| Playability | RAPID co-developer Mirko Ruta describes assigning one resynth multisample to several oscillators at different semitone offsets and playing a major chord from one key; the manual credits Ruta as a developer. [Developer example, 4 Sep 2017](https://www.kvraudio.com/forum/viewtopic.php?p=6865817#p6865817), [manual title page](https://parawave-audio.com/public-download/Rapid.Synthesizer.User.Manual.pdf) | This demonstrates ordinary pitched/modulated oscillator use, not the synthesis internals or pitch accuracy across the entire keyboard. |
| Retrigger and phase | For a selected multisample, **Phase** chooses the start point in the sample and **Random** varies that phase on every keypress. Since resynthesis generates a multisample, applying those controls to resynth output is a direct interface inference. [Manual pp. 39–40, 55](https://parawave-audio.com/public-download/Rapid.Synthesizer.User.Manual.pdf) | Parawave does not explicitly document resynthesis default start phase, mono/legato continuation, per-voice loop phase, or bit-identical retriggers. Deterministic retrigger is KURV's chosen default, not claimed RAPID fact. |
| Modulation and shaping | Generated content enters RAPID's normal oscillator path: tuning, start position, random start, delay, unison, Bass/Treble, oscillator insert effects, filters, and the modulation matrix are documented around multisample playback. [Manual pp. 38–46, 109–111](https://parawave-audio.com/public-download/Rapid.Synthesizer.User.Manual.pdf), [product feature list](https://parawave-audio.com/rapid_synth) | No first-party source documents a resynthesis-specific spectral editor, partial controls, a source-timeline scan control, or live modulation of analysis data. |
| Normalization | No reviewed Parawave manual, product, changelog, FAQ, support, German announcement, or official video states peak, RMS, LUFS, per-frame, or per-zone normalization for resynthesis. | RAPID provides no normalization reference contract. KURV must specify and validate its own gain policy and must not call that policy RAPID parity. |
| Quality | RAPID removed its multisample pre-render **Quality** option in 1.1 because the High setting showed no real improvement. Parawave recommends source choice/length rather than a quality tier. [Version history, 1.1.0](https://parawave-audio.com/rapid_version_history), [manual p. 55](https://parawave-audio.com/public-download/Rapid.Synthesizer.User.Manual.pdf) | RAPID-like does not require a user-facing Rich quality switch. Audible quality still needs same-input comparison, not a static setting claim. |
| Sample rate and CPU | A Parawave developer states that RAPID uses 44.1 kHz as its import/base rate, resamples for other host rates, and uses multirate/oversampling selectively as a quality/CPU compromise. The product specification likewise lists 44.1 kHz internal and 48/96/192 kHz via resampling. [Parawave developer response, 27 Jul 2020](https://www.kvraudio.com/forum/viewtopic.php?p=7843644#p7843644), [product specification](https://parawave-audio.com/rapid_synth) | There is no resynthesis-specific CPU, memory, latency, voice-count, or import-time benchmark. “RAPID-like CPU” is not a numeric target. Measure KURV with matched sources, notes, voices, host, buffer, and sample rate. |

The German developer announcement, created on 4 October 2016 and marked last edited on 14 September 2017, lists “Resynthese von Samples” alongside WAV/AIFF sample import and the normal multisample oscillator controls. It corroborates that this is a sample-import/playback facility, but adds no hidden algorithm detail. [Parawave's German announcement](https://www.sequencer.de/synthesizer/threads/rapid-synthesizer-ankuendigung.117636/)

## What is not public

The reviewed first-party record does not establish:

- FFT use, FFT size/hop/window, partial tracking, sinusoidal modeling, residual/noise separation, or phase reconstruction;
- generated zone count, sample length, loop points, loop crossfade, stereo-generation method, or spectral-frame layout;
- automatic pitch detection, stable-note correction for melodic sources, formant preservation, or transient preservation;
- normalization target, headroom, clipping/limiting, or input-level invariance;
- deterministic default retrigger, legato phase behavior, or phase alignment between chord voices; or
- quantified import cost, callback CPU, memory footprint, latency, or maximum polyphony.

Parawave's current roadmap still lists additive/FFT-based oscillator sources as possible **future** features, which is additional reason not to retrofit an FFT/additive implementation claim onto the existing resynthesis importer. [Official roadmap](https://parawave-audio.com/rapid_roadmap)

The public [Parawave video inventory](https://www.youtube.com/c/parawaveaudio-com/videos) checked on 2026-08-26 contains general RAPID showcases and tutorials but no public, dedicated resynthesis walkthrough. No behavioral claim above is inferred from an unlabeled showcase sound.

## Honest KURV acceptance contract

Use the same 1–4 second fixed-pitch vowel, filtered pluck, and metallic/noisy source in RAPID and KURV. KURV Rich can be accepted as “RAPID-like” only when direct renders and a host audition show:

1. a held note sustains indefinitely with no audible wrap click;
2. the output is an evolving synthetic texture recognizably derived from the source, not literal source replay;
3. C–E–G produces three stable, independently pitched voices with the same timbral identity;
4. repeated notes restart consistently under KURV's deterministic default;
5. a spoken phrase does not repeat as recognizable speech;
6. normalization follows KURV's separately declared policy without clipping; and
7. matched real-host CPU remains bounded at the intended polyphony.

Items 4–6 are deliberate KURV product decisions. Items 1–3 are the closest defensible observable meaning of RAPID-like Rich. Item 7 is an acceptance gate, not a claim that public sources reveal RAPID's implementation or exact performance.
