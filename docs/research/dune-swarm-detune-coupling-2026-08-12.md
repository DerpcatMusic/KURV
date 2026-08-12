# DUNE 3 Swarm: detune coupling and stochastic pitch motion

Research date: 2026-08-12.

## Verdict

DUNE's public contract supports the user's central observation, with one
important limit:

- **High confidence:** Swarm gives every oscillator in an oscillator stack its
  own modulation. Synapse says this in both the current manual and its product
  page. This means the motion is not one shared LFO copied across the stack.
- **High confidence:** `DETUNE` remains the stack's pitch-spread control in
  Swarm, while `RATE` controls motion speed. A Synapse-confirmed 2019 bug in
  which `DETUNE` became stuck specifically in Swarm is further evidence that
  the control is intentionally active in this mode.
- **Medium-high confidence:** the audible modulation is independent,
  band-limited stochastic **pitch/frequency** motion. The prior controlled DUNE
  3.6.5 capture found different non-repeating trajectories for two isolated
  stack lanes, broadband motion rather than a periodic line, and increasing
  excursion as `DETUNE` increased. Contemporary technical descriptions also
  identify random pitch/frequency modulation.
- **Not established:** Perlin noise. No Synapse manual, developer statement,
  source release, or relevant patent found names Perlin, simplex noise, a PRNG,
  an interpolation kernel, or a noise spectrum. Smooth Perlin noise, filtered
  noise, interpolated random targets, and randomized LFO sums can produce
  similar observable motion.
- **Not yet distinguished:** whether all lanes receive the same maximum random
  excursion after the global `DETUNE` scale, or whether each lane's motion
  depth is proportional to that lane's static distance from center. Both models
  make the entire Swarm calm at `DETUNE = 0` and more animated as `DETUNE`
  rises. Stereo output cannot isolate enough simultaneous lanes to prove the
  lane-index depth law.

The precise DUNE-faithful behavioral target is therefore **independent smooth
stochastic pitch paths whose total field is governed by Detune, with Rate only
changing their temporal speed**. Calling the generator "Perlin" would overclaim
the evidence.

## What Synapse actually specifies

The current DUNE 3.6 manual places Swarm inside the oscillator-stack **Tuning**
selector and says every oscillator in the stack is modulated individually. It
adds `RATE` for modulation rate. In the immediately following common controls,
`DETUNE` spreads stack pitches around the center frequency, `SPREAD` controls
stereo position, and `RESET` controls starting phase. These are separate
dimensions; the documentation gives Swarm no separate motion-depth knob.
[DUNE 3.6 manual, pp. 28-30](https://www.synapse-audio.com/DUNE-36-Manual.pdf)

Synapse's product page independently says each Swarm oscillator gets its own
subtle modulation. The older 3.5 manual calls Swarm a Supersaw with built-in
modulation and says eight Swarm oscillators can have an effect similar to 32 in
another stack mode. This describes the perceptual purpose, not a reduced-lane
or constant-cost implementation.
[DUNE 3 product page](https://www.synapse-audio.com/dune3.html)
[DUNE 3.5 manual, pp. 8-9 and 26-28](https://www.synapse-audio.com/DUNE-35-Manual.pdf)

In 2019, users reported that the oscillator `DETUNE` knob became stuck only
when Swarm was selected in the macOS AU. Synapse developer Richard Hoffmann
confirmed the fix in the next update. This does not disclose the math, but it
rules out treating Detune as irrelevant or bypassed in Swarm.
[Richard Hoffmann's first-party response](https://www.kvraudio.com/forum/viewtopic.php?p=7553537#p7553537)

`Density` lanes inside OSC 1/2 are not the same thing as DUNE's upper-level
`Unison Voices`, which duplicate complete synth voices. The Swarm claim applies
directly to oscillator-stack lanes. Public documentation does not explicitly
state whether a given lane index's stochastic path is also independent between
two simultaneously played MIDI notes, so cross-note sharing remains unknown.
[DUNE 3.6 manual, oscillator stacks pp. 28-30; Unison Voices pp. 53-54](https://www.synapse-audio.com/DUNE-36-Manual.pdf)

## What the output establishes

The controlled DUNE 3.6.5 experiment already recorded in this repository used
two hard-panned stack lanes so their pitch paths could be observed separately.
The lanes had different non-repeating trajectories and near-zero low/mid-rate
audio correlation. A repeated 30-second render at Rate 25% had correlation
`-0.040`, and motion energy occupied a band instead of one periodic spectral
line. At low and medium rates, level and left/right energy stayed nearly
constant, supporting pitch-only motion with static pan and lane weights.
[Full local capture method and results](dune-ableton-swarm-algorithms-2026-08-04.md#installed-dune-365-black-box-capture)

Measured temporal character at 48 kHz was:

| DUNE Rate | Median motion-band frequency | 90% motion-band frequency |
|---:|---:|---:|
| 0% | 0.67 Hz | 0.98-1.10 Hz |
| 25% | 5.9-6.2 Hz | 10.1-10.3 Hz |
| 50% | 22.3-23.0 Hz | 32.9-33.8 Hz |

Rate zero therefore means very slow motion, not a frozen process. The high-rate
measurement was estimator-limited and should not be used to infer an internal
update frequency. Two 2019 technical descriptions agree on random pitch or
frequency modulation and describe slower sweeping versus faster, phasey
motion, but they are secondary corroboration rather than implementation proof.
[Electronic Musician, June 2019, DUNE 3 review](https://www.worldradiohistory.com/Archive-All-Music/Electronic-Musician/2019/Electronic-Musician-2019-06.pdf)
[Computer Music tutorial](https://www.musicradar.com/how-to/how-to-master-synapse-audio-dune-3s-swarm-oscillator-and-dual-filter)

## The two detune laws the evidence still permits

Let `D` be the user Detune value, `s_i` the static signed layout position of
lane `i`, and `n_i(t)` an independent smooth zero-mean stochastic path.

1. Global Swarm depth: `pitch_i(t) = D * (s_i + a * n_i(t))`
2. Lane-relative depth: `pitch_i(t) = D * s_i * (1 + a * n_i(t))`

Both satisfy everything currently observed: no spread/motion at zero Detune,
greater overall motion at higher Detune, independent lanes, and Rate changing
only time behavior. The second makes outer lanes move more than inner lanes and
leaves a center lane fixed; the first does not. DUNE's stereo sum does not
provide enough lane isolation to choose between them reliably.

For KURV, the minimum non-speculative correction is to make the selected static
detune range own the Jitter/Swarm excursion instead of adding an unrelated
absolute pitch range that asks users to zero Detune first. If the two existing
KURV modes are retained, they can share that depth law while differing only in
temporal character. The exact lane-relative law should be chosen from KURV's
musical contract or a purpose-built lane-isolation measurement, not attributed
to DUNE without evidence.

## Public-information boundary

No official DUNE source code is public. The searches covered current and older
Synapse manuals, the product page, Synapse developer/forum statements, English
and German web results, and patent indexes. No relevant patent or first-party
statement disclosed the random generator, interpolation, update cadence,
cross-note state ownership, or lane-index depth curve.
