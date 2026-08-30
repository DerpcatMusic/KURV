# Narrow coefficient-bank integration boundary (2026-08-30)

## Verdict

Do not integrate the three-frame bank into the current publication contract.
The cap-2/3/6 representation and single-Horner transition remain validated,
but KURV's present curve ownership makes a correct full integration either a
large continuous CPU regression or an unsafe lifetime shortcut. Production is
unchanged at 0.8.7.

This is an architecture rejection, not an algorithm rejection. A later round
should first introduce a stable, audio-owned compiled-frame cache/handle seam;
the narrow backend can then attach without expanding every value-copy path.

## Lifecycle traced

The trace started at current main `506fc9b`, including the shipping
derivative-regularized curve compiler.

- `WaveCurveData::compile_rt` produces one 256-byte `WaveCurveRt`.
- `VaTableData::compile_rt` embeds up to 16 curves by value in `VaTableRt`.
- `AtomicVaTable` publishes 1,024 coefficient words and reconstructs the whole
  fixed table on the audio thread after a generation change.
- `VaTableTransition` owns `previous`, `current`, and `pending` tables and
  clones them for audible edits.
- Legacy rendering calls `VaTableTransition::select` per host sample while
  curves/tables are smoothing. `select` materializes interpolated 64-coefficient
  curves by value.
- Structural configuration also materializes a selected curve by value into
  `OscillatorDspConfig`, then into `OscillatorDspSettings`.
- `OscillatorSettings`, voice configuration, topology fades, PM/AM/RM paths,
  scalar/x4/x8 renderers, and worker jobs all carry or consume that value.
- `VaOscillator::reset` owns only oscillator-local phase/resynthesis state; the
  proven reversible selector would add two `f32`s per oscillator lane and must
  be reset there.
- Presets persist editable `WaveCurveData`/`VaTableData`, so compiled banks
  should remain derived data and require no preset schema change.

There are 80 custom-curve/generator references across the VA and voice trees,
including 49 direct `custom_curve` references in `src/voices`. This is not a
single evaluator substitution.

## Why value integration fails

Three projected frames add 768 bytes to each 256-byte raw frame. For a full
16-frame table:

| Storage/publication site | Current | Raw + narrow bank |
|---|---:|---:|
| curve coefficient bytes per table | 4,096 | 16,384 |
| atomic coefficient words per table | 1,024 | 4,096 |
| three-table `VaTableTransition` payload | 12,288 bytes | 49,152 bytes |
| 32 audio-owned generator tables | 131,072 bytes | 524,288 bytes |

The memory is bounded, but copying is the blocker. Extending today's value
selection would interpolate 192 additional coefficients for each table
selection. The legacy path may select all three legacy oscillators every host
sample: up to 27,648,000 extra coefficient interpolations per second at 48 kHz,
before factor-1 curve evaluation. Atomic refresh would also add 3,072 relaxed
loads per changed table, and audible legacy edits would clone the expanded
table through previous/current/pending storage.

Carrying the bank by value in `OscillatorSettings` or `OscillatorDspSettings`
would add 768 bytes to objects copied through voice configuration and topology
crossfades. That directly conflicts with the measured goal of beating current
1x, despite the evaluator itself being cheap.

## Why a direct pointer is not acceptable

An exact-frame-only implementation could point into `VaTableRt` and avoid
copies, but current ownership has two independently changing sources:
`KurvDspState::va_tables` and the three-table `VaTableTransition`. Those tables
are overwritten after generation publication while their selections are copied
into synth settings and consumed by internal worker jobs. A raw pointer needs a
proved lifetime and handoff protocol across table replacement, topology fades,
legacy per-sample transitions, and worker completion. No such compiled-bank
handle exists today.

Restricting eligibility to an exact custom keyframe avoids interpolation but
also makes positioned WAVE motion and ordinary table morphs ineligible. Exact
positions are measure-zero during modulation and parameter smoothing. Adding a
hidden unsafe pointer for that narrow case would trade broad maintenance risk
for little reachable musical coverage.

Warp and partial custom/canonical morph remain explicit fallbacks, as already
established. Pitch alone is not the ownership problem: the two-float selector
can safely slew/reverse. The unresolved problem is obtaining stable endpoint
banks at every render seam without RT copying or invalidation.

## Required enabling seam

A credible integration should first create a small stable handle to an
audio-owned selected-bank cache:

1. State/editor threads compile cap-2/3/6 banks offline and publish fixed words.
2. The audio thread snapshots them only on generation changes into storage whose
   address never changes while voices/jobs can reference it.
3. Table selection writes at most one selected bank per oscillator at a bounded
   block/config boundary, never per sample.
4. `OscillatorDspSettings` carries a typed handle plus eligibility generation,
   not 768 bytes or an untracked raw pointer.
5. Existing table/topology transitions invalidate or retarget the handle through
   the proven 1,024-sample two-float selector.

That is a deep publication-module change with its own concurrency and transition
gates. Folding it into this oscillator experiment would be unjustified
complexity and would obscure whether failures came from the backend or the new
ownership system.

## Verification

```bash
git rebase main
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
  cargo check --all-targets --no-default-features --locked
```

The all-target check passes with the checkout's existing warnings. The previous
quality, transition, and structural CPU probes remain unchanged and applicable:
the algorithm clears the cap-6 quality and 0.0011 transition gates, but no real
bank is reachable safely under the current ownership contract. No runtime
source, Cargo dependency, test, preset, or version change is retained.
