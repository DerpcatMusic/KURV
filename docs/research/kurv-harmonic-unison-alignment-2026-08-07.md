# KURV harmonic unison alignment: terminology, precedents, and algorithm

Research date: 2026-08-07  
Repository snapshot: `main @ 62f206a`  
Scope: define what “harmonic alignment” should mean for KURV’s per-oscillator
unison lanes. This note is research only; no Rust code or tests were changed.

## Executive conclusion

The user’s clarified meaning is a **harmonic/partial stack**, not the current
nearest-ratio alignment lattice.

For a played fundamental `f0`:

```text
1st harmonic = 1 × f0 = f0
2nd harmonic = 2 × f0
3rd harmonic = 3 × f0
4th harmonic = 4 × f0
...
```

The sequence `1×, 2×, 4×, 8×` is a valid but narrower family: the
**power-of-two/octave partials**. It is not what “harmonics 1, 2, 3, 4” means;
the full harmonic series is `1×, 2×, 3×, 4×, 5×...`.

KURV currently mixes three different ideas under HARM:

1. reduced ratios such as `8:7`, `5:4`, and `3:2`;
2. octave multiplication of those ratios; and
3. 12-TET semitone candidates.

That is a **hybrid interval quantizer**, not a harmonic-series selector. In
particular, `8:7` means the frequency relationship between the 8th and 7th
partials. It is not the frequency of the 8th partial relative to `f0`, which is
`8:1`.

The implementation decision should therefore be explicit:

- `NOTE`: snap the voice’s relative pitch to a 12-TET semitone grid.
- `HARMONIC` or `PARTIAL`: select integer harmonic multiples of the played
  `f0`, with optional `ALL`, `ODD`, `EVEN`, or `OCTAVE` families.
- If KURV keeps near-note just intervals such as `9:8`, `5:4`, or `3:2`, that
  should be a separately named **RATIO/JUST** mode. It must not be called the
  literal harmonic-series mode.

There is a hard constraint: a literal harmonic series cannot produce several
distinct targets inside a ±2.24-semitone window. The second harmonic is +12
semitones, the third is about +19.02 semitones, and so on. To make near-note
harmonic colors such as `9:8` useful inside a small Range, KURV must explicitly
choose **octave-reduced harmonic intervals** or **just ratios**. That is a
musically useful interpretation, but it is different from putting the actual
9th partial at `9×f0`.

## Sourced terminology

The ASA/ANSI Acoustical and Bioacoustical Terminology Database is the governing
terminology source used here:

- **Fundamental frequency**: for a periodic function, the reciprocal of its
  period; it is also the lowest natural frequency of an oscillating system.
- **Fundamental**: the frequency of a harmonic complex whose period matches the
  period of the whole complex.
- **Partial**: a sinusoidal component of a complex tone.
- **Harmonic**: a partial whose frequency is an integral multiple of the
  fundamental frequency.
- **Inharmonic complex tone**: a complex tone whose partials are not integral
  multiples of the fundamental.
- **Subharmonic**: a sinusoidal quantity at an integral submultiple of the
  fundamental, such as `f0/2` or `f0/3`.

The ASA notes that “overtone” has often been used in place of “harmonic,” with
the `n`th harmonic called the `(n-1)`th overtone, and marks the term as
deprecated because the numbering is ambiguous. In ordinary synth language,
the useful operational distinction is:

| Term | Exact meaning for KURV |
|---|---|
| `f0` / fundamental | The played note’s base oscillator frequency. |
| Harmonic `n` | `n × f0`; the fundamental is harmonic 1. |
| Overtone | A component above the fundamental; the first overtone is harmonic 2. |
| Partial | Any sinusoidal component, whether harmonic or inharmonic. |
| Harmonic series | The set `f0, 2f0, 3f0, 4f0, ...`. |
| Subharmonic | `f0/n`; useful for a symmetric lower-frequency counterpart, but not an upper overtone. |
| Frequency ratio | A relationship such as `3:2` or `5:4` between two frequencies. |
| Just intonation | A tuning language built from simple rational frequency ratios; it is not identical to the harmonic index of one partial. |

Sources: [ASA fundamental and fundamental-frequency entries](https://asastandards.org/working-groups-home/working-groups-portal/asa-standard-term-database/),
[ASA harmonic definition](https://asastandards.org/terms/harmonic-2/),
[ASA partial definition](https://asastandards.org/terms/partial/),
[ASA inharmonic complex tone definition](https://asastandards.org/terms/inharmonic-complex-tone/),
and [ASA subharmonic definition](https://asastandards.org/terms/subharmonic/).

### The user’s `1, 2, 4, 8` example

The `1×, 2×, 4×, 8×` sequence is the octave-doubling sequence. Every member
is also a harmonic, but it omits `3×`, `5×`, `6×`, `7×`, and so on. It is best
named **power-of-two harmonics**, **octave partials**, or **octave family**.

Indiana University’s acoustics text describes both facts directly: octave
doublings are `1f, 2f, 4f, 8f...`, while 12-TET uses `2^(1/12)` per semitone.
The distinction matters because an octave-family selector can only produce
octaves, whereas the full harmonic series also produces the 3:2, 5:4, 7:4,
and other pitch relationships after octave reduction.

Source: [Indiana University, Acoustics Chapter One: Pitch and Tuning](https://cmtext.com/acoustics/chapter1_pitch.php).

## What “harmonic” means in first-party synth documentation

### Serum 2: separate Semitones, Harmonics, and Ratio modes

Xfer’s official Serum 2 manual is the closest direct precedent found. It
separates the pitch-control modes instead of merging them:

- **Semitones**: pitch steps in the 12-tone equal-tempered keyboard system.
- **Harmonics**: multiply the base frequency using whole-number harmonics.
- **Ratio**: set an oscillator frequency relative to another oscillator using a
  ratio, as in FM synthesis.

This is exactly the conceptual separation KURV needs. Serum’s manual also
describes Harmonics as useful for overtone-rich and harmonic-layer sounds; it
does not define Harmonics as “nearest simple ratios plus semitones.”

Source: [Serum 2 manual, Setting the Octave or Semitone Mode](https://xferrecords.com/web-manual/serum-2/setting-the-octave-or-semitone-mode).

### Ableton Operator: harmonic coarse tuning versus fractional tuning

Ableton’s official Operator manual says Coarse sets the oscillator ratio in
whole numbers, creating a harmonic relationship, while Fine introduces
fractional ratios and therefore an inharmonic relationship. Operator also has
a partial editor and an even/odd editing option.

This gives KURV two useful precedents:

1. integer ratio selection belongs to the harmonic mode;
2. partial-family selection belongs to the harmonic spectrum/partial model,
   not to a hidden blend with semitone quantization.

Sources: [Ableton Operator oscillator parameters](https://www.ableton.com/en/manual/live-instrument-reference/#operator-oscillator-section-and-display),
[Ableton Operator harmonic editor](https://www.ableton.com/en/manual/live-instrument-reference/#user-waveforms).

### Apple Logic EFM1: harmonic indices and even/odd behavior

Apple’s first-party EFM1 guide says the carrier is determined by the played key,
the modulator is normally a multiple of the carrier, and both can be tuned to
the first 32 harmonics. It gives `1:1` and `2:1` as concrete examples and says
even versus odd tuning relationships can change the result from more harmonic
to more metallic/inharmonic.

This is an FM carrier/modulator feature rather than unison, so it is an
analogy, not a direct KURV implementation prescription. It does confirm that
“harmonic number,” “ratio between oscillators,” and “global tune” are distinct
controls in a commercial synth.

Source: [Apple Logic Pro, Set the EFM1 tuning ratio](https://support.apple.com/en-euro/guide/logicpro/lgsifb8884c7/mac).

### Image-Line Harmor: true per-note/per-unison partial synthesis

Harmor is the strongest precedent for the literal partial interpretation. Its
official manual documents up to 516 sine partials **per note and per unison
voice**. It defines harmonic partials as exact integer multiples of the note’s
base frequency and distinguishes them from inharmonic decimal multiples.

The manual also gives the canonical spectral families:

- a saw uses all harmonics with decreasing amplitude;
- filtering out even harmonics and retaining odd harmonics produces the square
  recipe when paired with the appropriate amplitude profile;
- octave-spaced partials are `1, 2, 4, 8, 16...` within that amplitude
  discussion.

This is important for KURV’s scope. Harmor’s “partials per unison voice” means
each unison voice contains an additive harmonic bank. It is not merely moving
one VA oscillator lane to a nearby `p/q` interval. If KURV adds literal
harmonic stacks while keeping one waveform generator per lane, it is creating a
lighter harmonic-layer model, not a conventional detuned-unison mode.

Source: [Image-Line Harmor manual, Additive Synthesis and Harmonic Mapping](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/plugins/Harmor.htm).

### DUNE 3: musical chord/unison modes are a separate concept

Synapse Audio’s official DUNE 3 manual documents Linear, Nonlinear, Gaussian,
Alternate, Random, Perfect 5th, Minor, Major, Sub Osc, and Swarm tuning modes.
Perfect 5th, Minor, and Major generate musical chord relationships in the
oscillator stack; Sub Osc places alternating oscillators an octave below.

DUNE therefore supplies a useful product precedent for a **musical unison mode**,
but those modes are not described as harmonic-series partial selection. DUNE’s
Perfect 5th and chord modes are closer to a fixed interval/ratio or chord
layout than to `n × f0` partials.

Source: [Synapse Audio DUNE 3 User’s Manual, official PDF, oscillator tuning section](https://www.synapse-audio.com/DUNE-36-Manual.pdf),
especially PDF page 29.

### Native Instruments: pure and overtone-based tuning is note tuning

Native Instruments’ official Jacob Collier Audience Choir manual exposes Pure,
Overtone 16–32, and Dynamic Pure Tuning. Pure uses harmonic ratios for fifths
and thirds; Dynamic Pure Tuning adjusts note pitches in real time to match the
harmonic series.

This is a useful boundary case: adaptive just tuning is about the relationship
between **played notes in a chord**, not about assigning each internal unison
lane an overtone number. It should not be used as evidence that a unison
detune slider can freely replace each lane with an overtone while preserving a
small cents Range.

Source: [Native Instruments Jacob Collier Audience Choir manual, Tunings](https://www.native-instruments.com/ni-tech-manuals/jacob-collier-audience-choir-manual/en/settings-page).

## Harmonic series versus just-intonation ratios

The two ideas are related but not interchangeable.

### Literal harmonic partials

For a played note `f0`, the nth harmonic is:

```text
f_n = n × f0
```

Examples:

| Partial | Ratio to `f0` | Offset from `f0` |
|---:|---:|---:|
| 1 | `1:1` | 0 cents |
| 2 | `2:1` | 1200 cents |
| 3 | `3:1` | 1901.955 cents |
| 4 | `4:1` | 2400 cents |
| 5 | `5:1` | 2786.314 cents |
| 6 | `6:1` | 3101.955 cents |
| 7 | `7:1` | 3368.826 cents |
| 8 | `8:1` | 3600 cents |
| 9 | `9:1` | 3803.910 cents |
| 16 | `16:1` | 4800 cents |

The cents conversion is:

```text
cents(ratio) = 1200 × log2(ratio)
```

### Ratios between partials or just intervals

If two partials are `p × f0` and `q × f0`, their interval ratio is:

```text
(p × f0) / (q × f0) = p / q
```

That is why `3:2` is associated with a perfect fifth and `5:4` with a just
major third. But an oscillator at `3/2 × f0` is **not** the third harmonic of
`f0`; it is the ratio between the third and second harmonics, or a fifth-like
interval above `f0`.

Richard Feynman’s Caltech lecture gives the same distinction operationally:
the major triad can be represented by `4:5:6`, and the octave is `1:2`; equal
temperament instead divides the octave into twelve equal logarithmic steps,
using `2^(1/12)` per semitone.

Source: [The Feynman Lectures on Physics, Chapter 50: Harmonics](https://www.feynmanlectures.caltech.edu/I_50.html).

**Inference for KURV:** the current candidate `8:7` is a valid simple interval
between two harmonic partials, but it is not a valid literal “8th harmonic
target” relative to the played note. Calling the current `p/q` table HARMONIC
therefore communicates the wrong algorithm.

## Audit of the current KURV implementation

At the research snapshot, KURV’s static lane path is:

```text
raw_cents = lane_position × detune_range_cents × detune_amount
raw_ratio = 2^(raw_cents / 1200)
```

When alignment is enabled, `src/voice.rs` currently:

1. scans a fixed table of reduced ratios derived from numbers 1 through 8;
2. multiplies those ratios by octave factors;
3. scans equal-tempered semitone targets from -48 through +48; and
4. chooses the nearest target on the same side of unison inside the effective
   Range.

The blend is then performed in cents. JITTER is added after the static pitch
calculation. The latter ordering is appropriate and should remain: JITTER is a
motion layer around a selected base pitch, not a second repeated snapping pass.

The problem is step 1–3, not the cents blend. The current table is a mixture of
interval ratios and equal-tempered notes. It does not enumerate `n × f0`.

At a raw positive offset of about `+2.24` semitones, the current table can
prefer `8:7` at approximately `+231.174` cents because that is numerically
nearby. The literal harmonic candidates around the played note are different:

- `1 × f0` is 0 cents;
- `2 × f0` is +1200 cents;
- no upper literal harmonic lies within +224 cents.

That explains why the current behavior can sound like a strange small musical
interval while failing to sound like the user’s intended overtone/harmonic
stack.

Local source: [`src/voice.rs`](../../src/voice.rs), especially the
`HARMONIC_BASE_CANDIDATES`, `HARMONIC_OCTAVES`, and
`nearest_absolute_candidate` definitions.

## Derived per-unison algorithms

Let:

- `f0` be the frequency of the played note after the existing glide/pitch-bend
  path;
- `p_i` be the signed lane position produced by Distribution, in `[-1, +1]`;
- `R` be the current effective static Range in cents;
- `A` be Detune Amount;
- `x_i = p_i × R × A` be the existing raw lane cents;
- `r_i = 2^(x_i / 1200)` be the existing raw ratio.

The alignment mode should select a target ratio relative to **the same `f0`**.
The target is not derived from a fixed 440 Hz reference and does not need MIDI
note-name tracking. Applying the target ratio to the oscillator phase step
automatically follows note pitch, glide, and pitch bend.

### Mode 1: NOTE / 12-TET

Use semitone targets:

```text
target_ratio(k) = 2^(k / 12)
target_cents(k) = 100 × k
```

Choose the nearest same-sign integer `k` whose target is inside the effective
Range. The center lane always selects `k = 0`.

At `R = 224` cents, the available upper targets are `0`, `+100`, and `+200`
cents; the lower targets are `0`, `-100`, and `-200` cents. This is the mode
that preserves the user’s major/minor/diminished chord-like detune behavior.

### Mode 2: HARMONIC / PARTIAL

For literal overtones, enumerate integer partial indices, not arbitrary `p/q`
ratios:

```text
upper target for partial n = n × f0
upper target cents          = 1200 × log2(n)
lower mirror (subharmonic)  = f0 / n
lower target cents          = -1200 × log2(n)
```

The sign-mirrored lower target is a **subharmonic**, not an overtone. It is the
mathematically clean way to give a signed Range a lower counterpart while
preserving a log-frequency mirror around the played note.

Candidate families can be explicit:

```text
ALL:       n = 1, 2, 3, 4, 5, 6, ...
ODD:       n = 1, 3, 5, 7, 9, ...
EVEN:      n = 2, 4, 6, 8, 10, ...
OCTAVE:    n = 1, 2, 4, 8, 16, ...
```

`OCTAVE` is the user’s `1×, 2×, 4×, 8×` example. It is a subset of `ALL`,
not a synonym for `EVEN`; `6×` and `10×` are even harmonics but are not
powers of two.

For each lane, either:

1. assign a deterministic partial index from the lane’s signed ordinal; or
2. search the selected family for the nearest target on the lane’s side, subject
   to the Range.

The first option is the better definition for a **stack** because it ensures
that multiple unison lanes actually represent different partials. The second
option is a valid **quantizer**, but several lanes will legitimately collapse
to the same target when the Range is small.

The hard Range consequence is unavoidable:

```text
R = 224 cents  -> only n = 1 is inside the literal harmonic window
R = 1200 cents -> n = 1, 2 are available
R = 2400 cents -> n = 1, 2, 3, 4 are available
R = 3600 cents -> n = 1 through 8 are available
```

**Inference:** if the existing Range must remain a cents-bounded unison
control, literal HARMONIC mode should visibly allow the stack to leave the
normal unison window or should honestly collapse to the fundamental at small
Ranges. Silently folding octaves is a different mode.

### Mode 3: octave-reduced harmonic intervals

This is the likely explanation for the desired “musical harmonic” behavior
inside a small Range. Take an integer harmonic and remove octaves until its
ratio lies in `[1, 2)`:

```text
reduced_ratio(n) = n / 2^floor(log2(n))
```

Examples:

| Harmonic index | Octave-reduced ratio | Offset |
|---:|---:|---:|
| 1 | `1:1` | 0 cents |
| 3 | `3:2` | +701.955 cents |
| 5 | `5:4` | +386.314 cents |
| 7 | `7:4` | +968.826 cents |
| 9 | `9:8` | +203.910 cents |
| 11 | `11:8` | +551.318 cents |
| 13 | `13:8` | +840.528 cents |
| 15 | `15:8` | +1088.269 cents |

Mirror the positive ratios for the lower side using `1 / reduced_ratio(n)`.
At `R = 224` cents, this mode can choose `9:8` at +203.910 cents and `8:9`
at -203.910 cents. This is musically near the played note, but it is an
**octave-reduced harmonic interval**, not the actual 9th partial at `9 × f0`.

Important limitation: once every harmonic is octave-reduced, even indices
collapse onto earlier odd-index pitch classes (`6` reduces to `3:2`, `10`
reduces to `5:4`, etc.). Therefore an `EVEN` versus `ODD` selector loses its
literal spectral meaning in this mode unless the original partial index is
retained as a separate voice identity or amplitude rule.

### Mode 4: JUST / RATIO

If the goal is the full set of near-note consonant intervals, use an explicit
rational-ratio mode:

```text
target_ratio = p / q
target_cents = 1200 × log2(p / q)
```

Use reduced `p/q` candidates within a documented limit, include reciprocals for
the negative side, preserve the same-sign and Range constraints, and select the
nearest target in cents. This is close to what the current KURV table does.

But it must be labeled **JUST**, **RATIO**, or **HARMONIC INTERVAL**, because
`p/q` represents a relationship between two harmonic numbers, not necessarily
one harmonic partial relative to `f0`.

## Recommended KURV product model

The cleanest model is:

```text
FREE      existing Range + Distribution
NOTE      relative 12-TET semitone targets
HARM      literal partial targets: ALL / ODD / EVEN / OCTAVE
JUST      optional near-note rational interval targets
```

If only one toggle is acceptable, choose between **NOTE** and **HARM** and make
HARM literal. Do not retain the current hybrid candidate scan under that name.
If the desired sound is specifically several musical intervals inside a small
Range, choose **NOTE** or add **JUST**; do not describe the result as actual
overtone placement.

For either NOTE or JUST, the static pitch blend can remain:

```text
aligned_cents = raw_cents + align × (target_cents - raw_cents)
```

For literal HARM/PARTIAL stack mode, a lane-ordinal assignment is preferable to
independent nearest-target selection:

1. preserve the center lane at `1 × f0` for odd voice counts;
2. assign deterministic upper partials to positive lanes and reciprocal
   subharmonics to negative lanes, or use a documented one-sided stack mode;
3. let Distribution/Range control the lane’s selection position without
   changing pan, gain, phase, or voice count;
4. apply JITTER after the static target ratio, so it drifts around the selected
   harmonic/subharmonic instead of being repeatedly snapped;
5. keep all candidate tables fixed-size and build/retarget them outside the
   sample loop.

**Inference:** a single signed symmetric unison layout cannot simultaneously be
the complete upper harmonic series and remain centered around `f0` in the usual
small-detune sense. The lower half is necessarily subharmonic or octave-reduced
interval material. The UI should expose that choice instead of hiding it in a
“nearest harmonic” label.

## Manual acceptance examples

These examples distinguish the modes without requiring a particular oscillator
waveform.

### Played `f0 = 100 Hz`

| Mode/family | Lane targets |
|---|---|
| Literal ALL | `100, 200, 300, 400, 500 Hz...` |
| Literal OCTAVE | `100, 200, 400, 800 Hz...` |
| Literal ODD | `100, 300, 500, 700 Hz...` |
| Literal EVEN | `200, 400, 600, 800 Hz...` |
| Octave-reduced harmonic intervals | `100, 150, 125, 175, 112.5 Hz...` for harmonic indices 1, 3, 5, 7, 9 |
| Just ratio examples | `100 × 9/8 = 112.5 Hz`, `100 × 5/4 = 125 Hz`, `100 × 3/2 = 150 Hz` |

### Range around `±2.24` semitones

- Literal ALL/ODD/EVEN/OCTAVE: only `1:1` is inside the range; `2:1` is
  +12 semitones. A nonzero stack requires either a larger Range or a different
  mode semantics.
- Octave-reduced harmonic intervals: `9:8` at +203.910 cents and `8:9` at
  -203.910 cents are available.
- NOTE: ±2 semitones are available at ±200 cents.
- JUST/RATIO: candidates such as `9:8`, `8:9`, `8:7`, and `7:8` are all
  rational intervals, but they are not interchangeable harmonic partial IDs.

## Sources

All sources below are first-party standards, institutional references, or
manufacturer documentation. No forum posts, reviews, or search-result summaries
are used for the definitions or product behavior above.

1. [ASA/ANSI Standard Acoustical & Bioacoustical Terminology Database](https://asastandards.org/working-groups-home/working-groups-portal/asa-standard-term-database/)
2. [ASA: harmonic](https://asastandards.org/terms/harmonic-2/)
3. [ASA: partial](https://asastandards.org/terms/partial/)
4. [ASA: inharmonic complex tone](https://asastandards.org/terms/inharmonic-complex-tone/)
5. [ASA: subharmonic](https://asastandards.org/terms/subharmonic/)
6. [Xfer Records: Serum 2, Setting the Octave or Semitone Mode](https://xferrecords.com/web-manual/serum-2/setting-the-octave-or-semitone-mode)
7. [Ableton: Live Instrument Reference, Operator](https://www.ableton.com/en/manual/live-instrument-reference/)
8. [Image-Line: Harmor manual](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/plugins/Harmor.htm)
9. [Apple: Logic Pro EFM1 tuning ratio](https://support.apple.com/en-euro/guide/logicpro/lgsifb8884c7/mac)
10. [Synapse Audio: DUNE 3 User’s Manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf)
11. [Native Instruments: Jacob Collier Audience Choir, Tunings](https://www.native-instruments.com/ni-tech-manuals/jacob-collier-audience-choir-manual/en/settings-page)
12. [Caltech: The Feynman Lectures, Chapter 50: Harmonics](https://www.feynmanlectures.caltech.edu/I_50.html)
13. [Indiana University: Acoustics Chapter One, Pitch and Tuning](https://cmtext.com/acoustics/chapter1_pitch.php)
14. [Apple: Logic Pro tuning project settings](https://support.apple.com/en-gb/guide/logicpro/lgcp452f269/mac)
