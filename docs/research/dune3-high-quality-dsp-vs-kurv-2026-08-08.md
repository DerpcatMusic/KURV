# DUNE 3 high-quality DSP and anti-aliasing

Reviewed 2026-08-08. Sources are Synapse Audio first-party material only.

## Confirmed facts

- Synapse documents DUNE 3 as supporting VA, wavetable, and FM synthesis, with two 32-oscillator stacks, up to eight unison voices, and a Swarm mode. The product page describes the oscillator blocks and features, but does not describe their anti-aliasing implementation: [DUNE 3 product page](https://www.synapse-audio.com/dune3.html).
- The current manual documents VA saw/pulse/triangle waveforms, wavetable mode with 1–256 waveforms, and FM based on three sine operators. It does not name BLEP, minBLEP, bandlimited tables, mipmaps, polyBLEP, or another oscillator bandlimiting method: [DUNE 3.6 manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf), sections 4.2–4.2.2.
- The manual explicitly documents **3× oversampling for analog-modeled filters except Alpha**. This is a filter implementation detail, not an oscillator-quality setting: [DUNE 3.6 manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf), section 4.5.
- The manual explicitly documents **4× and 32× oversampling variants for the Screamer distortion effect**, with the 32× variant described as more CPU-intensive and potentially higher quality on high notes: [DUNE 3.6 manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf), section 4.15.1.
- DUNE 3 documents optimized SSE vector processing and support for multiple processor cores. It recommends at least 128-sample buffers for multithreading; below that, multithreaded processing is disabled because synchronization overhead is too high: [DUNE 3.6 manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf), sections 1.4 and 10.
- DUNE 3 exposes modulation-rate choices of Normal, Fast, and Audio-rate. The manual says Audio-rate processes the entire synth engine and modulation sources sample by sample instead of in blocks and needs substantial CPU: [DUNE 3.6 manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf), section 10.
- The manual mentions host operation at 44.1/48 kHz and WAV-file import from 44.1–192 kHz. It does **not** state an internal oscillator/rendering sample rate or a global internal oversampling factor: [DUNE 3.6 manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf), sections 1.4 and 7.3.

## Unknown from first-party documentation

Synapse does not publicly document, in the product page or current manual reviewed here:

- the VA oscillator anti-aliasing algorithm;
- whether VA oscillators use bandlimited waveforms, oversampling, or another method;
- any hidden/fixed oscillator oversampling factor;
- the internal sample rate used by any oscillator, wavetable, FM, or full voice path;
- whether different oscillator modes or notes select different quality paths.

No official DUNE source code or developer statement establishing those details was found in the first-party material reviewed.

## What the missing oversampling control proves

Only this: the documented user interface does not expose a general oscillator oversampling control. It does **not** prove that DUNE runs its oscillators at 1×, does not oversample internally, or does not use bandlimited oscillator generation. Synapse explicitly uses fixed internal oversampling in documented modules (the analog-modeled filters), so a hidden/fixed oscillator strategy remains possible. That is an inference, not a claim about DUNE’s actual oscillator implementation.

## Consequence for KURV

The fair conclusion is not “DUNE has no oversampling, therefore KURV should remove quality modes.” The defensible target is: make KURV Eco’s oscillator output sufficiently bandlimited at the host rate, then validate it against KURV’s higher-quality path with high notes, saw/pulse edges, sync/FM, wavetable movement, and 64-voice unison. Only if the measured alias energy and listening result stay within the chosen threshold should a quality mode be removed. DUNE’s public documentation cannot tell us which algorithm to copy.
