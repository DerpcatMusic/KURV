# 0.8.13 recurrence, DPW, and rational premise audit (rejected)

Date: 2026-08-30
Baseline: `e91c070` (KURV 0.8.13)

## Verdict

Do not build another runtime harness. The 0.8.13 scalar and fixed-block wins do
not change the rejection premise of the best pruned recurrence, factored DPW2,
or global rational cap-3 candidates. Their decisive failures were waveform or
pitch-transition artifacts, not repeated BLEP setup in the paths that 0.8.13
changed.

No production source, test harness, version, oscillator state, or dependency was
changed. No compile or timing run was performed.

## Source-diff boundary

The strongest recurrence and DPW second shots used baseline `7850f6b`; the
rational probe used `2cd5b85`. From `7850f6b` through 0.8.13, the production VA
changes relevant to generation are fixed warped-pulse edge preparation and the
custom x8, x4, and scalar BLEP invariant hoists. All three candidates explicitly
exclude warp and custom-curve rendering.

`src/oscillators/va/backend.rs`, which owns the production constant x8 canonical
saw backend used by the recurrence comparison, has no diff between `7850f6b`
and `e91c070`:

```text
git diff 7850f6b..e91c070 -- src/oscillators/va/backend.rs
# no output

git rev-parse 7850f6b:src/oscillators/va/backend.rs
git rev-parse e91c070:src/oscillators/va/backend.rs
# both 9548f603f6ffd912070e994a6cf4b3ca743f5af8
```

The 0.8.13 scalar selector is additionally guarded by partial custom mix and
exact saw/square shape. Canonical `custom_mix == 0` rendering therefore never
enters it. The x8/x4 hoists have the same custom-only boundary. None supplies a
new backend-ownership, pitch-smoothing, or representation-handoff mechanism.

## Candidate premises

| Candidate | Best retained result | Decisive prior gate | Changed by 0.8.13? |
|---|---|---|---|
| Pruned real-output recurrence | Approximately `1e-6` RMS exact projection; steady x8 ratio `0.216..0.784` | At the 5.9-to-6.1 kHz family handoff, entry excess delta was `0.3925` saw and `0.3467` square; a full-block fade worsened entry. Four-block saw duty was `1.106..1.521x`. | No. The current canonical family and note/transition ownership are unchanged. |
| Stateless factored DPW2 | Better normal/high-note RMS; saw and triangle often materially faster | Every rapid-pitch transition probe regressed; the 27.5 Hz discontinuity peak also increased for saw, square, pulse, and triangle. | No. Those errors are inherent to the step-dependent DPW waveform; the new helpers are custom-only. |
| Global rational cap-3 saw | RMS `0.0000477..0.0000606`; coarse scalar/x8 CPU appeared faster | Immediate cap-4-to-3 and cap-3-to-2 selector peaks regressed `26.7%` and `51.1%`; rapid-pitch peak regressed `19.2%`. | No. It still requires the same current/rational family switches at 6 and 8 kHz. |

The later finite-residual and phase-aligned recurrence handoffs already tested
cheaper transition work and an optimistic crossing oracle. Both failed because
the two sampled waveform families do not meet robustly, rather than because the
selector was too expensive.

## Decision

Reject all three second shots before compilation. Re-running the retained DPW
harness or rebuilding the removed recurrence/rational probes would duplicate an
unchanged deterministic gate. A future experiment is justified only after a
production change supplies a shared sampled waveform family, bounded harmonic-
space transition, or note-lifetime ownership that covers generic and settled
renderers without freezing live controls.
