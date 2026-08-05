# DUNE and Ableton swarm/unison motion: evidence and KURV target

Research date: 2026-08-04.

## Conclusion

The proposed split, **DUNE = periodic/sine motion** and **Ableton = random/noise
motion**, is not supported.

- DUNE 3's official documentation only promises individual modulation of every
  oscillator and a Rate control. Two independent contemporary technical reviews
  call it random pitch modulation. A controlled capture of the installed DUNE
  3.6.5 produces broadband, non-repeating, band-limited pitch motion rather than
  a single periodic trajectory.
- Ableton does not have one common "drift" algorithm. Wavetable's Shimmer and
  Noise modes use ongoing random-interval pitch jitter; Meld's Swarm oscillator
  exposes random pitch motion; Drift uses preset-seeded per-voice random tuning
  offsets whose static differences produce audible beating.
- Neither DUNE nor the relevant Ableton sources document moving pan or moving
  amplitude weights as part of their swarm pitch algorithm. Their phase-reset,
  stereo-spread, and level-distribution controls are separate.

If KURV needs two genuinely distinct motion modes, the evidence-backed pair is
**Wander** (continuous smooth stochastic motion) and **Jitter** (new targets at
random intervals with click-safe slew). A periodic sine mode could still be a
creative feature, but it should not be presented as DUNE behavior. Ableton
Drift's fixed "voice-card" variation is a useful third concept, not a moving
Swarm mode.

## Evidence scale

- **High:** current first-party manual/reference or a named developer statement.
- **Medium-high:** multiple independent descriptions plus a controlled
  black-box result.
- **Medium:** controlled output behavior whose hidden implementation cannot be
  identified uniquely.
- **Low/unknown:** no source or measurement distinguishes the alternatives.

## Synapse DUNE family

### DUNE 1: static Fat stack, not Swarm

The official DUNE 2 manual describes DUNE 1 retrospectively: its oscillator
section had a Fat mode with seven fixed additional oscillators rather than
DUNE 2's configurable 32-oscillator stacks. Contemporary DUNE 1 reviews likewise
describe Fat as bringing in seven detuned copies. DUNE 1's defining
"Differential Unison Engine" could give complete unison voices different
modulation and starting phase, but that is a layer above the Fat oscillator
copies. No source found describes ongoing per-Fat-oscillator drift.

**Confidence:** high for static seven-oscillator Fat; unknown for its exact
detune and gain curves.

### DUNE 2: several static stack layouts plus per-note Random

DUNE 2 replaced Fat with two stacks of up to 32 oscillators. The official manual
defines:

- Linear: equally spaced pitch;
- Nonlinear: some oscillators closer to the center;
- Gaussian: amplitude follows a Gaussian distribution, shaped by Amount;
- Alternate: Amount lowers alternating oscillators;
- Random: oscillator tuning is randomized whenever a new key is pressed.

Density selects the oscillator count, Detune spreads pitch around the center,
Spread places oscillators around the stereo center, and Amount changes static
level distribution. Reset makes every stack oscillator start at the same
initial phase; otherwise the starts are not phase-locked. DUNE 2 Random is
therefore **per-note static random tuning**, not continuous drift.

**Confidence:** high. These are explicit first-party semantics.

### DUNE 3: ongoing individually modulated Swarm

DUNE 3 retains the DUNE 2 layouts and adds Perfect 5th, Minor, Major, Sub Osc,
and Swarm. The current manual says all oscillators in a Swarm stack are
modulated individually and exposes an additional Rate knob. Synapse's product
page calls Swarm an evolution of the classic Supersaw and says every oscillator
gets its own subtle modulation. The older 3.5 manual adds the practical claim
that eight Swarm oscillators can resemble 32 oscillators in an ordinary stack.

The official wording does not name the destination or generator. Stronger
support for **random pitch** comes from two independent 2019 descriptions:

- Electronic Musician calls Swarm a supersaw with random pitch modulation of every
  oscillator in the stack.
- Computer Music/MusicRadar contrasts a static Linear stack with Swarm, where
  oscillator frequencies are individually modulated; lower Rate gives slow
  sweeps and higher Rate gives a faster, more phasey result.

The parameters remain orthogonal:

| Property | DUNE 3 control/behavior | Evidence |
|---|---|---|
| Oscillator count | Density, up to 32 in either of two stacks | Official manual |
| Static pitch spread / motion depth | Detune spreads pitch; installed capture also shows Swarm excursion increasing with Detune | Manual + black box |
| Motion speed | Rate, 0--100% display | Manual + black box |
| Stereo position | Spread distributes the stack around center | Official manual |
| Static level distribution | Amount, meaning depends on tuning mode | Official manual |
| Start phase | Reset makes all stack oscillators start at the same phase | Official manual |
| Motion destination | Frequency/pitch | Two technical reviews + black box |
| Per-oscillator relationship | Each oscillator has its own/individual modulation | Synapse product page + manual |
| Exact PRNG/LFO, distribution, smoothing, update cadence | Not disclosed | Unknown |

### Installed DUNE 3.6.5 black-box capture

This measurement did not automate or interact with Bitwig.

#### Setup

- Installed Windows payload:
  `/home/derpcat/.wine/drive_c/Program Files/Common Files/VST3/Synapse Audio/DUNE3.vst3`
- Reported binary version: `3.6.5.0`
- Payload SHA-256:
  `3eb7f7d2ab31d6642c2688f783c159bb12f2bb84874dab9ff37b7516e8c7f6ad`
- Isolated host: Pedalboard 0.9.24 through yabridge `ba7022d` and Wine 11.14
  Staging; 48 kHz, 512-sample host buffer.
- DUNE's stack tuning selector is not one of its 1,440 exposed VST3
  parameters. The factory `Swarm King KS.fxp` component state supplied the
  hidden Swarm selection. Every exposed parameter was then restored to the
  installed `Init Patch.fxp` value, preserving only that hidden tuning mode.
- Controlled signal: oscillator 1 Ramp Up; oscillator 2/noise/ring modulation,
  modulation matrix, arpeggiator, effects, and limiter disabled; one synth
  voice; oscillator Density 2; phase Reset 100%; Spread 100% to place the pair
  on separate channels; Detune 35%; sustained MIDI C4.
- Pitch trace: sixth-order 210--315 Hz band-pass around the fundamental,
  analytic phase derivative, 1 ms means, then Welch power spectrum. The
  high-Rate measurements are limited by the estimator and are treated only as
  qualitative.

#### Results

| DUNE Rate | Median motion-band frequency (`f50`) | 90% motion-band frequency (`f90`) | Interpretation |
|---:|---:|---:|---|
| 0% | 0.67 Hz | 0.98--1.10 Hz | Rate zero is still moving; slow wander |
| 25% | 5.9--6.2 Hz | 10.1--10.3 Hz | broadband faster modulation |
| 50% | 22.3--23.0 Hz | 32.9--33.8 Hz | broadband, phasey modulation |
| 75--100% | tens of Hz | roughly 47 Hz in this estimator | motion is faster, but exact bandwidth is unresolved |

The trajectories contained many spectral components rather than one stable
line. Re-rendering the same controlled 30-second 25% Rate condition after a
plugin reset produced correlation `-0.040` and maximum sample difference
`0.336`; it did not restart an identical cycle. That does not reveal whether
the implementation is interpolated PRNG, a sum of randomized LFOs, or another
deterministic stochastic process, but it falsifies a simple shared sine-rate
model for observable output.

At 0% and 25% Rate, each hard-panned oscillator's 50 ms RMS varied only about
0.31--0.32%, and the L/R energy ratio varied by only 0.04 dB over 26 seconds.
The two channels' audio correlations were close to zero at low and mid rates.
Combined with the documented separation between Detune, Spread, Amount, and
Reset, this supports **moving pitch with static pan and static weighting**.

**Confidence:** medium-high for non-periodic/band-limited random-looking pitch
motion and static pan/weights; medium for the measured low/mid Rate bands;
unknown for the internal random generator, interpolation kernel, update cadence,
and exact cross-oscillator correlation.

## Why DUNE can appear cheaper than KURV at 64 oscillator lanes

### The clarified workload is 64 lanes, not 64-note polyphony

The compared DUNE patch uses one held note, one DUNE unison voice, and both
VA oscillator stacks at Density 32 in Swarm mode: `2 * 32 = 64` oscillator
lanes. The KURV patch uses one held note and one 64-lane unison oscillator.
Those raw oscillator counts are comparable. They must not be confused with
DUNE's **Polyphony** control or its eight higher-level **Unison Voices**:

- DUNE 3.6 note polyphony is at most 24;
- each of oscillator stacks 1 and 2 has Density `0..32`;
- the eight DUNE unison voices duplicate complete synth voices and parameters;
- Synapse's current product page explicitly advertises two 32-oscillator
  stacks and up to 520 oscillators per note with all eight unison voices.

This also falsifies a hard hidden eight-lane Swarm cap as the default
explanation. The older manual's statement that eight Swarm oscillators can
sound like 32 ordinary oscillators is a sound-design recommendation, not a
claim that Density 32 silently evaluates only eight. Density is documented as
the oscillator count, and Synapse's first-party capacity claims count the full
stacks. Amount-dependent weighting can still make some lanes quieter, so a
sound-matched comparison must record DUNE's AMT and KURV's Voice Weight.

### KURV's default quality setting performs 128 source-lane evaluations

KURV's default Spline 4PT / 2x path renders the complete 64-lane source twice
per host sample and then decimates it. DUNE's two 32-lane stacks total 64
source lanes, but no first-party source says that its VA oscillator stacks are
rendered at 2x. The DUNE manual is explicit when oversampling is used elsewhere:
most analog-modelled filters use 3x, and the installed changelog identifies a
Screamer distortion mode using 32x. This is not proof that DUNE's VA oscillator
uses no oversampling, but it makes a blanket full-stack 2x assumption
unsupported.

A frozen local KURV release harness makes the cost of the known multiplier
visible. This was one held MIDI C4 saw, 64 lanes, 48 kHz, 1,000,000 frames,
eleven medians pinned to CPU 12 on the Ryzen 7 7800X3D. The artifact SHA-256
was `ea2538748fc37b24e57b6149f5cf9b25911dde27f1be4e8980a7c4375f95a2de`:

| KURV path | Swarm | Median ns/host sample | Change from Spline1, Swarm off |
|---|---:|---:|---:|
| Legacy 1x | off | 65.544 | -14.2% |
| Spline 1x | off | 76.432 | reference |
| Spline 2x | off | 139.952 | **+83.1%** |
| Legacy 1x | Wander | 80.483 | +5.3% |
| Spline 1x | Wander | 92.562 | +21.1% |
| Spline 2x | Wander | 171.071 | +123.8% |

The user's earlier observation that DUNE used about 65% of KURV is consistent
with this known cost before invoking a proprietary trick: `0.65 * 139.952 =
90.97 ns`, close to KURV's measured 1x Spline-plus-Wander region. That is an
inference, not a DUNE timing measurement. It says that eliminating the second
complete stack evaluation is large enough to close the observed gap if
Spline2-class spectral accuracy can be retained at host rate.

Simply changing the default to Spline1 is not the accuracy solution. KURV's
coherent saw measurements put Spline1 near -45 dBc alias residual while the
optimized Spline2 path reaches roughly -93 dBc. The useful target is therefore
an event-sparse or otherwise host-rate bandlimited source with Spline2-class
alias rejection, not silently spending half the work and accepting the old
alias floor.

### What DUNE actually documents about CPU

First-party evidence supports these mechanisms:

- optimized SSE vector processing;
- an internal multithreaded engine using up to six cores;
- multithreading disabled below 128-sample buffers and recommended at 512
  samples for 44.1/48 kHz;
- selectable Normal, Fast, Very Fast, and Audio Rate modulation processing,
  with Normal recommended for most sounds and Audio Rate explicitly described
  as expensive;
- oscillator Density zero disabling a stack to save processing;
- Swarm moving pitch independently per oscillator while stereo spread and
  weighting remain separate static controls.

The manual does **not** disclose a minBLEP, wavetable mip system, shared phase
accumulator, SIMD width beyond the general SSE statement, lane culling,
Swarm update cadence, PRNG, interpolation kernel, or VA-oscillator oversampling
factor. Those remain hypotheses, not facts.

KURV already batches oscillator lanes eight-wide on this AVX2 machine, so
"DUNE probably has SIMD" is not by itself an explanation. A cheaper source
algorithm, avoiding a whole second pass, less per-sample Swarm state, and years
of data-layout tuning are more plausible contributors.

### Multithreading and Wine can distort the displayed comparison

The live Bitwig process topology was inspected without changing the project:

- DUNE ran in a 24-thread native `BitwigPluginHost-X64-AVX2` process with a
  separate 12-thread `yabridge-host.exe.so` child;
- KURV ran natively in one 21-thread `BitwigPluginHost-X64-AVX2` process;
- all three processes were allowed on logical CPUs 0--15.

Bitwig documents that plugins run in separate processes and labels its DSP
meter as current CPU usage, but does not document enough meter semantics to
claim whether a particular displayed device percentage includes every Wine
worker's accumulated CPU. DUNE can also shorten its critical-path wall time by
using internal workers at eligible block sizes. Therefore a single process's
`%CPU`, or one device percentage without an OS-wide process sum, is not a
reliable total-work comparison.

An isolated Pedalboard/yabridge timing attempt was deliberately rejected. Its
Wine bridge paced rendering and consumed an almost fixed amount of CPU from
Density 1 through two stacks at Density 32. At both 64- and 512-sample blocks,
only the main yabridge thread and one named worker accumulated measurable CPU
in the one-note render. This falsifies that harness as a density benchmark and
does not show that DUNE's extra oscillators are free. It also means internal
six-core scaling was not demonstrated for that one-note workload; the 2x
source multiplier remains the strongest measured explanation.

### Swarm is not the dominant mystery

The installed DUNE capture supports pitch motion with static pan and static
weights. KURV Wander advances pitch ratio, left gain, and right gain for every
lane every sample. In the frozen benchmark above, enabling Wander added
21--23% at 64 lanes. DUNE can legitimately be cheaper here if it computes only
pitch motion at control rate and keeps the already-distributed gains constant.
KURV Jitter already follows that cheaper static-pan/static-weight contract.

This does not imply shared DUNE modulation. The two hard-panned captured lanes
had different nonrepeating trajectories and near-zero low/mid-rate audio
correlation, falsifying one observable shared sine LFO. DUNE may still share a
clock or calculate several random lanes in one vector batch; neither affects
the required audible independence.

### Experiments that close the gap without guessing DUNE internals

1. In Bitwig, use one sustained note, DUNE Unison Voices `1`, OSC 1/2 Density
   `32`, both Swarm, identical AMT/level, effects/noise/OSC 3/ring off. Compare
   KURV 64 lanes at Spline1 and Spline2. This directly tests whether the known
   2x multiplier explains the user's meter gap.
2. Repeat at buffer 64 and 512, and with DUNE's Multithreading off/on. Record
   maximum callback/DSP load and sum CPU time for Bitwig's DUNE host,
   yabridge-host, wineserver attributable during the interval, and KURV's host.
   This separates deadline advantage from total work.
3. Repeat OSC 1 Density 32 / OSC 2 off against KURV 32, then enable DUNE OSC 2.
   The incremental cost isolates DUNE's second stack without changing note
   polyphony.
4. Capture matched low, middle, and high notes and score wanted harmonics,
   nonharmonic residual, peak, DC, and noise. Do not accept a CPU win that is
   merely a higher alias floor or lower audible lane weight.
5. Prototype an **event-sparse residual** path for saw/pulse: advance and sum
   the raw host-rate SIMD ramps, then schedule a short bandlimited correction
   only when a lane crosses a discontinuity. At C4, 64 saw lanes create only
   about 16,744 wrap events per second, or 0.349 events per host sample; this
   can be much cheaper than evaluating all 64 source lanes a second time on
   every sample. Reject it unless it reaches the Spline2 residual/wanted-band
   bounds under PWM, morphing, pitch motion, and high notes.
6. Keep DUNE-faithful Swarm pitch-only and KURV's spatial Wander as separate
   costs. A control-rate pitch target plus per-sample phase-increment slew and
   static gains is the apples-to-apples mode.

The fifth experiment is the highest-value new DSP direction. Prior universal
wavetable, sinc, DPW, and Fourier replacements lost on dense CPU or transitions;
an event-sparse correction attacks the measured multiplier directly and does
not require a gather for every lane on every sample.

## Ableton behaviors are three different families

### Wavetable: static layouts and two random-interval jitter speeds

Ableton's manual defines six unison modes:

| Mode | Pitch | Phase | Stereo/table behavior | Time behavior |
|---|---|---|---|---|
| Classic | Equal static spacing | Different phases implied | Alternating stereo channels | Static |
| Shimmer | Random pitch jitter | Not specified | Small per-voice table offset | Random intervals, slow |
| Noise | Same pitch jitter | Not specified | Small per-voice table offset | Much faster than Shimmer |
| Phase Sync | Classic detune | Synchronized at note start | Classic layout | Static detune; phasing comes from beating |
| Position Spread | Small static detune | Not specified | Even table-position spread | Static |
| Random Note | Random detune | Not specified | Random table position | Redrawn at note start |

Voices selects oscillators per Wavetable oscillator and Amount scales the
selected effect. There is no independent user Rate: Shimmer and Noise are the
two speed characters. Ableton also documents that Shimmer, Noise, Position
Spread, and Random Note recalculate tables per unison voice.

With Wavetable Hi-Quality off, modulation is calculated every 32 samples. This
does not prove the jitter generator's event interval or smoothing, but it is
direct precedent for KURV's 32-sample target cadence. Hi-Quality's exact update
cadence is not documented.

**Confidence:** high for mode semantics and 32-sample non-HQ modulation
evaluation; unknown for jitter distribution, slew kernel, pitch range, and
cross-voice correlation.

### Meld Swarm: random pitch motion, chord spacing, separate phase controls

Meld provides Swarm Sine, Triangle, Saw, and Square oscillators with Motion and
Spacing macros. The Live manual says Motion adds modulation and Spacing fades
through increasingly complex chords. More decisively, Ableton's exported DSP
reference for `abl.dsp.swarm~` defines Motion as the amount of **random pitch
modulation** and Spacing as chord spread/inharmonicity. Both range from 0 to 1.
There is no exposed Swarm Rate.

Meld's Phase Reset and Phase Spread controls are outside the Swarm macros.
Stacked Voices duplicates both complete engines; its Spread value is a general
modulation-matrix offset, not proof that `abl.dsp.swarm~` moves oscillator pan.

**Confidence:** high for random pitch Motion and independent phase controls;
unknown for oscillator count, rates, distribution, smoothing, and weighting.

### Drift: stored voice-card variation, not an ongoing noise LFO

Drift's global Drift control gives every synth voice different randomization
for oscillator 1, oscillator 2, and filter frequency. Creator Marc Resibois
explained that these three elements are randomly detuned from the main
frequency, their differences create beat sounds, and each preset/instance
stores a random "serial number" so the same randomization returns on reload.

The most defensible reading is a set of **fixed seeded offsets** analogous to
analog voice-card tolerances; the audible fluctuation is the beating of
different fixed frequencies. Neither the creator interview nor the manual
identifies an ongoing random process inside the Drift control. This is separate
from Drift's Wander LFO, which the creator describes as sample-and-hold values
joined with an S-shaped interpolation.

Drift's voice modes are also separate from the Drift control: Poly is one voice
per note, Stereo uses two panned voices, and Unison uses four independently
detuned voices. Oscillator Retrigger controls note-start phase.

**Confidence:** high for seeded per-voice oscillator/filter randomization and
preset recall; medium-high that the offsets are static rather than time-varying,
because that follows from the developer's voice-card, beating, gap, and serial
number description rather than an explicit word "static."

### Roar: useful random-modulator vocabulary, not a unison reference

Roar is a distortion effect, not an oscillator stack. Its Noise source is only
useful as corroborating Ableton vocabulary: Simplex and Wander are smoothed
random signals with different interpolation, S&H is stepped random, and Brown
is low-pass-filtered white noise. It should not be used to claim that Wavetable
or Meld shares those exact algorithms.

## Concrete KURV target

The current KURV implementation already has the right foundation in
[`src/voice.rs`](../../src/voice.rs): per-lane seeded motion, separate pitch and
pan seed domains, two cubic-interpolated hash-noise bands, a 0.02--32 Hz Rate
control, 32-sample target updates, and per-sample linear travel to each target.
The two evidence-backed modes should differ in temporal statistics, not in a
fictional vendor split.

### Mode 1: Wander

Desired output:

- Continuous, band-limited, non-periodic pitch motion with no visible steps.
- Independent deterministic seed domains per oscillator lane; avoid a shared
  trajectory. The same complete plugin state and render should remain
  reproducible even though note retriggers need not restart the trajectory.
- Amount scales pitch excursion; Rate scales the random field's bandwidth.
  Low values should breathe slowly, while high values become phasey/noisy.
- Keep note-start phase randomization/reset separate. Swarm must change phase
  *increment* through pitch, never rewrite oscillator phase during a held note.
- Keep static detune around the intended center. Preserve KURV's current pitch
  expectation and its exact pan-center/constant-power energy invariants.
- The current primary/detail blend is a reasonable KURV design, not a claim
  about DUNE internals.

Vendor-faithful DUNE/Meld behavior stops at pitch motion: pan positions and
amplitude weights stay static. KURV's existing moving-pan path is a musically
material extension and may be retained in the production sound, but it should
be described as KURV spatial motion rather than vendor emulation.

### Mode 2: Jitter

Desired output:

- Each lane chooses a new bipolar pitch target after a **random interval**,
  unlike Wander's continuously sampled smooth field.
- Use independent lane interval and target seeds. A practical bounded interval
  distribution such as `0.5..1.5 / Rate` avoids both lockstep periodicity and
  arbitrarily dense bursts; this is a KURV design inference, not an Ableton
  disclosure.
- Slew between targets with a cubic/sigmoid transition rather than jumping.
  Amount scales excursion, while Rate moves continuously from a slow
  Shimmer-like character toward a fast Noise-like character.
- Keep pan and weight static for the vendor-faithful mode. Wavetable's small
  table-position offsets have no direct analogue in KURV's VA oscillator and
  should not be faked with amplitude or pan motion.
- Retain the 32-sample target cadence and per-sample interpolation. At 48 kHz
  this is a 0.667 ms control interval, has first-party Ableton precedent, and
  avoids discontinuities without putting random/hash work in every sample.

### Static Voice Card, documented separately

If KURV later wants a Drift-like option, generate a fixed seeded pitch offset
per lane/voice and hold it for the voice or preset lifetime. Disable Rate in
that mode. Do not label fixed detune as Wander or animate it in the UI; its
motion is the physical beating of static frequency differences.

## Implementation guardrails

- No allocation, locking, RNG service, or unbounded work on the audio thread.
- Generate targets at the bounded control seam; interpolate every sample.
- Do not change gain normalization when switching temporal modes.
- Do not continuously randomize oscillator phase.
- Do not infer exact DUNE or Ableton ranges, distributions, or correlations
  from marketing words. The black-box bands above are character anchors for
  one installed DUNE version, not a clone specification.
- Evaluate pitch, pan, and energy independently. A richer stereo image caused
  by beating is not evidence of moving pan.

## Sources

Primary and developer sources:

- [Synapse DUNE 3.6 manual](https://www.synapse-audio.com/DUNE-36-Manual.pdf)
- [Synapse DUNE 3.5 manual](https://www.synapse-audio.com/DUNE-35-Manual.pdf)
- [Synapse DUNE 2.5 manual](https://www.synapse-audio.com/DUNE-2-5-Manual.pdf)
- [Synapse DUNE 3 product page](https://www.synapse-audio.com/dune3.html)
- [Synapse launch post by Richard Hoffmann](https://www.kvraudio.com/forum/viewtopic.php?t=516207)
- [Bitwig: plug-in crash protection and separate processes](https://www.bitwig.com/support/technical_support/what-is-plug-in-crash-protection-26/)
- [Bitwig user guide: DSP meter](https://www.bitwig.com/userguide/latest/the_window_menus_transport_area/)
- [Ableton Live 12 Instrument Reference: Drift, Meld, Wavetable](https://www.ableton.com/en/manual/live-instrument-reference/)
- [Ableton DSP reference: `abl.dsp.swarm~`](https://docs.cycling74.com/reference/abl.dsp.swarm~)
- [Ableton: Managing CPU load when using Wavetable](https://help.ableton.com/hc/en-us/articles/360000036930-Managing-CPU-load-when-using-Wavetable)
- [Interview with Drift creator Marc Resibois](https://cdm.link/inside-ableton-drift/)
- [Ableton Live 12 Audio Effect Reference: Roar](https://www.ableton.com/en/live-manual/12/live-audio-effect-reference/)

Independent technical descriptions:

- [Computer Music/MusicRadar DUNE 3 Swarm walkthrough](https://www.musicradar.com/how-to/how-to-master-synapse-audio-dune-3s-swarm-oscillator-and-dual-filter)
- [Electronic Musician, June 2019 DUNE 3 review](https://www.worldradiohistory.com/Archive-All-Music/Electronic-Musician/2019/Electronic-Musician-2019-06.pdf)
- [MusicRadar DUNE 1 review](https://www.musicradar.com/reviews/tech/synapse-audio-dune-376799)

Searches of Synapse's public forum, the long DUNE 3 KVR launch thread,
English/German/Polish/Japanese/Russian coverage, Google Patents, and technical
paper indexes found no developer disclosure, patent, paper, or trustworthy
reverse engineering that identifies DUNE Swarm's exact generator or Ableton
Wavetable/Meld's random distribution and smoothing kernel. Those details must
remain unknown rather than being filled in from generic supersaw literature.
