# Canonical constant-x8 integration boundary (2026-08-30)

## Verdict

Do not ship the dedicated coefficient backend through the current structural
x8 seam.  The isolated probe proves favorable steady-note economics, but its
16-byte selector belonged to one temporary pack.  The real structural renderer
can hold 64 unison lanes (eight x8 packs) per oscillator slot, and eligibility
loss crosses into accumulator APIs that write directly into the destination.
A click-safe reversible bridge is therefore not an isolated one-function
addition.

Production remains unchanged at 0.8.7.  The successful probe and its numbers
remain in `va-canonical-coefficient-x8-probe-2026-08-30.md`; this record rejects
only the proposed minimal integration seam.

## Traced boundary

`Voice::accumulate_structural_oscillator_block` owns the relevant constant-x8
loop.  For every pack it dispatches among four mutually exclusive paths:

1. `accumulate_custom8_block_constant` for custom mix;
2. `accumulate_shape8_block_constant_warped` for phase warp;
3. `accumulate_saw8_block_constant` for exact saw;
4. `accumulate_shape8_block_constant` for all other canonical shapes,
   including exact 50%-square.

All four advance oscillator phase and accumulate stereo gains directly.  None
returns the ungained x8 sample needed to mix current BLEP output with the
projected output.  When shape, pulse width, custom mix, warp, jitter, or lane
topology changes, control can leave this loop or select another accumulator.
Simply decrementing a selector without rendering both endpoints would make the
next sample jump; simply preserving it would reproduce the stale-state failure
the probe was required to exclude.

The state is also larger in the real ownership model.  `MAX_UNISON` is 64, so a
slot needs eight independent persistent pack selectors.  With 32 structural
oscillator slots, a bounded `[u16; 8]` per slot is 512 bytes per voice (16 KiB
for 32 voices), even before any active-pack bookkeeping.  The render-entry list
is rebuilt and cannot own persistent state, so “only active packs” would need a
new keyed store and reset/copy/topology lifecycle.  A single 16-byte field does
not represent the actual maximum state.

## Rejected shortcuts

- Abruptly clear the selector on an ineligible block: violates the measured
  reversible transition contract and can click.
- Leave selectors untouched while another kernel renders: resumes stale state.
- Render current fallback into temporary `[f32x8; 64]` left/right blocks before
  mixing: adds at least 4 KiB of stack traffic per active transition call and
  duplicates gain accumulation; it is not the measured candidate.
- Add the backend only to exact-saw and exact-square calls: entry works, but
  exit through custom/warp/morph/shape automation remains discontinuous.
- Store state in every `VaOscillator`: repeats the previously rejected object
  growth across the full bank rather than structural x8 packs only.

## Minimum prerequisite

A future production attempt first needs one return-valued, phase-owning x8
sample kernel shared by the four structural branches, or an equally small
accumulator callback that exposes the current sample before gain accumulation.
Only then can one structural loop apply the dual transition and update a keyed
pack selector on every eligibility change.  That refactor must independently
show no regression before the coefficient backend is attached; doing it in
this round would exceed the requested isolated seam and conceal its cost.

No runtime source, state layout, publication path, or version was changed.
