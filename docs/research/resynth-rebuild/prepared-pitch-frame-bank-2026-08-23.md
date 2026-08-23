# Prepared RESYNTH pitch-frame bank (2026-08-23)

## Decision

RESYNTH now builds a bounded `PreparedPitchFrameBank` from the authoritative,
normalized Grain PCM during worker artifact compilation. The bank is stored
inside the immutable `GrainSourceArtifact`; it is not published through a
second pointer. Persisted restore regenerates the same bank from the
persisted PCM, so fresh build and recall use the same prepared metadata.

This is a public-behavior implementation boundary, not a claim of Lucid's
proprietary algorithm. The current detector is a bounded worker FFT peak analysis. It retains up to
`MAX_PITCH_FAMILIES` ranked candidates per frame. It is intentionally a public
behavior approximation, not a claim of proprietary algorithm equivalence or
full studio-grade source separation.

## Realtime contract

- `MAX_PREPARED_PITCH_FRAMES` is 2048.
- The bank owns initialized `Box<[PitchFrame]>` storage before publication.
- Audio performs only bounded indexed lookup and onset interpolation.
- `SpectralRenderer` is fixed-size state owned by `GrainSchedulerState`.
- No audio allocation, lock, source analysis, or pointer publication occurs in
  the callback.
- Classic mode avoids pitch-frame lookup and keeps the existing dry path.

## Pitch conversion and mode path

`hz_to_midi` rejects non-finite and non-positive values and uses the standard
440 Hz / MIDI 69 mapping. Prepared frames are converted to `TargetedPitchFrame`
using the selected `PitchMode`, then to bounded `SpectralFrame` partials. Grain
rendering consumes the frame through the shared `SpectralRenderer`.

Target mode is an explicit spectral request. `grain_tune == 0` still enables
the target engine; positive Tune values blend the prepared harmonic result
with the granular residual. Spectral mode uses Tune as its blend amount.

Onset metadata is linearly interpolated between adjacent prepared frames. This
prevents a hard dry/tuned switch at a frame boundary. The source residual is
kept unretuned; prepared harmonic partials carry the mode's pitch correction.

## Persistence and publication

The artifact's existing persisted PCM remains authoritative. Restore rebuilds
mips and the pitch bank from that PCM without re-normalizing it. The bank is a
field of the same immutable artifact node used by `AtomicResynthArtifact`, so
artifact identity, pitch metadata, and generation cannot tear apart.

The live control path uses two atomic slots plus one sequence word. A producer
writes only the inactive slot and publishes an even sequence. Audio accepts a
slot only when the sequence is unchanged, otherwise it returns no live
override and retains the plan snapshot.

## Validation

Targeted tests cover:

- deterministic bank construction, onset marking, and the 2048-frame cap;
- Hz-to-MIDI edge cases;
- fresh-build versus persisted-PCM prepared-frame identity;
- onset interpolation;
- Classic/Spectral/Target rendering through `GrainSchedulerState`;
- phase randomization not changing source positions when Grain Spray is zero;
- coherent live controls under concurrent producer updates;
- bounded worker calibration and exact serial/partitioned parity.

Full library and all-target test commands are run from the repository's native
Cargo environment after each integration milestone.
