# Lucid observable note-lock contract

Research date: 2026-08-26. This note separates Lucid's published behavior,
Minimal Audio's undisclosed implementation, and KURV's newly chosen product
contract. It does not claim algorithmic parity.

## Decision

Lucid's observable reference is **per-grain scale correction**, not
"all spectral energy locks to one played MIDI note."

In Lucid's Scale pitch mode, each grain is analyzed and retuned in real time to
the active scale. Hard correction chooses the nearest scale note; Smooth is a
gentler correction; Off disables correction while retaining scale-based
transposition. Detection is explicitly monophonic: vocals and leads are the
intended inputs, while chords and non-tonal material fall back to the scale
root. [Pitch Controls](https://manual.minimal.audio/lucid-manual/granular-view/pitch-controls.md)
[Quick Start](https://manual.minimal.audio/lucid-manual/quick-start.md)

Minimal Audio head developer Ben Wyss supplies the only public implementation
fact: Scale mode runs an actual pitch detector and pitch shifter per grain. The
company does not call this spectral processing or disclose the detector,
shifter, correction law, or latency. Earlier zero-latency paths without pitch
detection were cut because they could not scale-lock.
[Developer interview](https://blog.minimal.audio/behind-lucid-a-new-approach-to-granular/)

## Observable Lucid behavior

| Input/control | Observable result |
|---|---|
| Monophonic pitched grain + Scale | Detect one pitch and move the grain into the enabled scale in real time. |
| Scale + Hard | Snap the grain directly to its nearest enabled scale note. |
| Scale + Smooth | Apply a less pronounced, more natural correction; its curve and timing are undisclosed. |
| Chord or non-tonal input | No single pitch is tracked; use the scale root as fallback. |
| Repitch | Couple pitch and playback speed like tape or a classic sampler. |
| Free | Shift pitch independently of playback speed and preserve source timing. |
| Chord | Stack every enabled interval on each grain. |
| Arp | Assign successive enabled intervals to successive grains. |
| Grain Rate: MIDI | Incoming MIDI controls grain-generation rate, producing an audio-rate pitched tone at high rates. It is not the Scale retune target. |

The pitch-mode, correction, chord, and arp rows are documented in
[Pitch Controls](https://manual.minimal.audio/lucid-manual/granular-view/pitch-controls.md).
The MIDI row is documented in
[Grain Controls](https://manual.minimal.audio/lucid-manual/granular-view/grain-controls.md)
and clarified by Lucid's lead developer: MIDI/scale snapping applies to grain
rate and does not make that rate follow the detected input pitch.
[Developer clarification](https://www.kvraudio.com/forum/viewtopic.php?p=9287726)

Lucid independently controls grain Rate and Size, while Stretch changes time
without changing pitch. These separations are useful reference behavior, but
they do not reveal the pitch-shifting algorithm.
[Grain Controls](https://manual.minimal.audio/lucid-manual/granular-view/grain-controls.md)
[Playback Controls](https://manual.minimal.audio/lucid-manual/granular-view/playback-controls.md)

## KURV's Spectral Note Lock contract

"Spectral Note Lock" is a KURV name and a stronger, different target:

- Every KURV voice targets its own played MIDI note, including octave.
- Tune 0% preserves the source's local melodic contour while globally placing
  its root at the played note.
- Tune 100% removes that contour from voiced harmonic energy and holds it at
  the played note. Attacks, consonants, noise, and other unvoiced residual stay
  natural instead of being fabricated into pitched grains.
- Intermediate Tune values move continuously between those two same-time
  endpoints. Tune must not select scale degrees, jump among detected notes or
  octaves, randomize grain pitch, or change source read speed.
- A held C-E-G therefore produces three independently locked voices at C, E,
  and G from the same source timbre.

This is not Lucid parity. It deliberately borrows Lucid's musical outcome—no
stray grain pitches—and replaces Lucid's scale target and fallback with KURV's
exact per-voice note target. Lucid's own public wording is only that every grain
stays in key, and its Hard mode can legitimately produce several scale degrees.
[What is Lucid?](https://manual.minimal.audio/lucid-manual/lucid-manual/what-is-lucid.md)

## Reference evidence and limits

The official walkthrough directly corroborates Repitch, Free, Scale, Smooth,
and Hard at 5:16–5:43. It does not present a controlled dry/target sweep from
which cents accuracy, transient preservation, or correction timing can be
measured.
[Official walkthrough](https://www.youtube.com/watch?v=2Y8_i3bHCRM&t=316s)

Minimal Audio's four downloadable dry/wet product-demo pairs were captured and
checked. Each pair has matched exported duration (about 13.714 seconds for the
first three and 16 seconds for the fourth), but every wet file is a designed
preset result. None isolates Scale mode, a known input F0, a target note, or a
single control change, so the demos are listening references only—not evidence
for an exact note-lock transfer function.
[Official product demos](https://www.minimal.audio/products/lucid)

No first-party source found discloses:

- an STFT, phase vocoder, PSOLA, WSOLA, spectral-bin mapping, FFT size, or
  formant policy;
- detector type, analysis window/hop, confidence threshold, octave correction,
  or whether analysis updates within a grain;
- Smooth/Hard timing or interpolation, cents tolerance, transient/residual
  routing, latency, or CPU/grain limits;
- a mode that maps all voiced energy to one incoming MIDI note.

Corpus checked: every page in the official Lucid manual sitemap, the official
product payload and eight dry/wet demo files, all four videos on Minimal
Audio's channel at launch (walkthrough, quickstart, product video, preset
demos), the Minimal Audio developer interview, and the lead developer's public
KVR clarifications. No localized or non-English first-party Lucid manual,
product page, interview, or authored video track was found; YouTube's automatic
translations do not add first-party evidence.
[Manual sitemap](https://manual.minimal.audio/lucid-manual/sitemap-pages.xml)
[Official quickstart](https://www.youtube.com/watch?v=_gGQR-GoqsY)
[Official product video](https://www.youtube.com/watch?v=RAvvq7lJNHs)
[Official preset demos](https://www.youtube.com/watch?v=16DU7n6krqc)

The remaining reference gap is a controlled run of the Lucid binary with a
known monophonic pitch sweep and isolated Scale settings. That capture can set
perceptual comparison targets, but it cannot change the product decision above
or reveal proprietary internals.
