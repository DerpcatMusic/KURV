# Rich full-timeline implementation — 2026-08-23

## Selected representation

KURV retains its existing Rich pitch-zone semantics and therefore uses a
source-ordered bank of 22 zones × 32 timeline intervals × 4096 samples. The
resulting immutable artifact payload is 11,534,336 bytes. Each interval keeps
the existing 4096-point worker FFT/IFFT resolution, so the 17.2 kHz Rich guard
continues to apply.

A single time-domain sequence was considered, but it cannot preserve the
current independent target-pitch/zone oscillator contract without replacing the
Rich renderer. The larger zone bank is the compatible implementation of full
source coverage for this engine. It is a bounded temporal approximation: long
sources have 32 ordered analysis intervals, not sample-exact recovery of every
short transient.

## Timeline contract

`source_boundaries[0..=32]` is monotonic, starts at zero, ends at
`source_frames`, and is published inside `RichZoneArtifact`. Worker analysis
uses the interval centers in source order. Realtime rendering interpolates
adjacent intervals with fixed work and never performs FFT, allocation, locking,
or source I/O.

## Persistence

Pack version 14 writes the boundaries, 32 gains, and the larger slabs. The
version 13 reader remains supported. Legacy eight-frame slabs are expanded by
repeating each old frame across four new intervals, preserving old behavior
without interpreting its bytes as a new representation. New and migrated
artifacts are tested through actual write/read paths.

## Validation

- Long-source analysis proves ordered first/middle/last coverage and distinct
  endpoint spectral content.
- Fresh version 14 Rich artifacts round-trip byte-identical waveform data and
  boundaries.
- Version 13 Rich artifacts migrate to the new in-memory shape with preserved
  first-frame content and complete derived boundaries.
- Full Cargo target tests remain the final gate.
