# Unison swarm reference and KURV control audit

Research date: 2026-08-04.

## What the references actually say

### DUNE 3

The DUNE 3.6 manual describes **Swarm** as a Supersaw-type oscillator in
which “all oscillators in the stack are modulated individually.” It adds a
single **RATE** knob that “controls the rate of modulation.” The manual does
not disclose the modulator waveform, update rate, random process, or any
second Swarm control. It is therefore correct to make KURV movement
per-lane/oscillator and rate-controlled, but incorrect to claim that DUNE uses
noise, sine, a particular maximum Hz value, or a known CPU trick.

The older DUNE 3.5 manual additionally says Swarm is an evolution of the
classic Supersaw oscillator, and that eight Swarm oscillators can have a
similar effect to 32 ordinary oscillators. That is a product-level sound/CPU
claim, not an implementation disclosure.

### Ableton

Ableton's Wavetable manual documents six unison behaviours:

- **Classic:** equal pitch spacing with alternating stereo panning.
- **Shimmer:** oscillator pitch is jittered at random intervals.
- **Noise:** the same pitch jitter at a much faster rate, creating breathy
  noisy textures.
- **Phase Sync:** Classic detune with phases synced at note start.
- **Position Spread:** even wavetable-position spread plus slight detune.
- **Random note:** each oscillator's wavetable position and detune amount are
  randomised at every note start.

That validates two separate concepts: static per-note randomness and
continuous, non-periodic per-oscillator pitch motion. It does **not** imply
that a shared sine LFO is the right model for either one.

Ableton Drift takes the same per-voice approach: its Unison mode independently
detunes four voices, while its Drift control gives every voice different
oscillator/filter randomisation. It is a useful behavioural reference, not a
specification for KURV's oscillator implementation.

## Current KURV JITTER truth

KURV now has one Swarm algorithm: pitch-only **JITTER**. The old Wander branch
and its interpolated pitch/pan noise are removed. Each lane gets a deterministic
hash target at a rate-controlled event boundary, then the cached phase step
ramps toward that target. The audio loop performs only the fixed-size phase-step
update; it does not evaluate noise, sine, or pan/gain motion per sample.

`Unison Jitter Rate` is `0.02..100 Hz`. The visible rate advances the shared
JITTER event clock directly; target preparation uses roughly one control update
per event, clamped to a safe 32--1024 internal-sample interval. Stereo gains stay
on the static unison layout, so JITTER cannot create amplitude modulation by
pumping pan or lane energy.

The per-note seed means the same lane index on two separately played notes does
not follow the same target sequence. The editor calls the same
`unison_lane_position_stereo_jitter_seeded()` helper as the DSP, so the moving
lane lines represent actual pitch targets rather than decorative animation.
The former mode parameter ID is retained hidden and always formats as JITTER so
existing host state does not shift parameter IDs.

## Current parameter/UI matrix

| Parameter IDs | Custom editor status | Audit |
|---|---|---|
| 0 Output | Visible: draggable top-right output meter | Present. |
| 1 Shape, 2 PWM | Visible in oscillator rail; PWM disabled outside Pulse | Present. |
| 3 Attack, 23 Hold, 4 Decay, 5 Sustain, 6 Release | Visible and draggable in envelope view | Present. |
| 24--29 envelope curve/Y and curve-time/X | Visible through the three editable curve handles | Present; deliberately not duplicated as knobs. |
| 11 Voices, 12 Range, 13 Width, 14 Phase, 34 Detune Amount | Visible in unison rail; Range defaults to `1 st`, and Detune Amount defaults to `100%` | Present. Range is the semitone ceiling; Detune Amount is the normalized amount inside it. |
| 15 Pitch Distribution, 32 Voice Weight | Visible in the unison X/Y field and its bottom bipolar slider | Present; deliberately graphical rather than duplicated as a card. |
| 20 Jitter, 21 Jitter Rate | Visible in unison rail | Present; pitch-only, rate-controlled JITTER. |
| 47 Legacy Swarm Mode | Hidden | Compatibility slot only; runtime is always JITTER. |
| 30 Stereo Alternate, 31 Stereo X | Visible through the stereo triangle | Present; deliberately graphical rather than duplicated as knobs. |
| 16 Velocity, 17 Pressure, 18 MPE Timbre, 19 MPE Bend Range | Visible below the envelope | Present. |
| 33 Oscillator Quality | Visible as the oscillator-pane dropdown | Present; host parameter is hidden but custom UI exposes it. |
| 7 Legacy Audition Drone, 8 Drone Frequency | Hidden and absent from custom UI | Neither is read by the current process path; these are dead legacy parameters, not missing user controls. Delete only with an explicit preset/host-compatibility decision. |
| 9 Pitch Bend, 10 Sustain Pedal | Hidden and absent from custom UI | Host/MIDI ingress controls; actual events are handled in `dispatch_events`, not by a panel control. |
| 22 Legacy Stereo Layout | Hidden and absent from custom UI | Not consumed by the current unison DSP. Legacy compatibility only; do not surface it beside the triangle. |
| meters | Not user controls | Left/right output feeds the meter; stereo seed and swarm time feed the visualisation. |

## Consequence

KURV needs no mode selector to match the documented DUNE contract: **JITTER
Amount** and **JITTER Rate**, with independent per-lane pitch targets, are the
defensible surface. A hidden compatibility slot remains only to protect host
state; it is not a second runtime algorithm.

[DUNE 3.6 manual, pp. 28--29](https://www.synapse-audio.com/DUNE-36-Manual.pdf)

[DUNE 3.5 manual, p. 9](https://www.synapse-audio.com/DUNE-35-Manual.pdf)

[Ableton Live Instrument Reference: Drift](https://www.ableton.com/en/manual/live-instrument-reference/#drift)

[Ableton Live Instrument Reference: Wavetable global and unison controls](https://www.ableton.com/en/manual/live-instrument-reference/#wavetable-global-and-unison-controls)
