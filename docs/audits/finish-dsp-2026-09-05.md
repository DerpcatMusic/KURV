# DSP consolidation evidence — 2026-09-05

Base: `cc6968e5a6b650253805ce7416c980899fbc0ca9`; package 0.8.164.
Consolidates production work from PRs #8, #9, #12, #13 and #15. No tests were authored, added or modified. Existing branch proofs were run against the combined production source in separate worktrees.

## Resulting behavior

- Worker waits use the nominal audio-time budget (75%, capped at 2 ms exact / 5 ms generic), without the old 16 ms extension. A serial calibration has no helper deadline: interrupting its bounded render duplicated work and corrupted the first cost estimate. This is a bound on waiting, not a guarantee that rendering plus recovery meets every callback deadline.
- Compiled expressions accommodate the four source programs needed for a table-transition blend. Curve edits finish the current 4 ms fade before accepting the newest source, preventing recursive expression growth. Postfix validation rejects malformed publication data. Compact byte opcodes and a constant pool reduce `WaveCurveRt` from 816 to 696 bytes despite the larger expression capacity. The initial enlarged-float-buffer implementation was rejected after CPU regressions.
- Negative `fract` uses floor semantics in scalar and SIMD evaluators. Mixed spline/function evaluation clamps the spline at the same point in all lane widths. Exact interpolation endpoints preserve the original program.
- Scalar triangle correction avoids the unused edge only in AVX2/FMA builds. Portable builds retain the existing kernel.
- `experimental-1x-dsp` is off by default. It enables a finite-harmonic high-note saw/triangle crossover for unwarped canonical shapes. Configured host modulation (including Mod Wheel and XY sidecar masks) and generator routes use the established antialiaser, including zero-depth routes; custom and warped paths retain fallback behavior. This does not certify arbitrary host automation or FM/PM sidebands. Existing unused event/correction experiments were not imported. Repeated warp guards reuse one helper.
- CI also runs the existing realtime allocation suite with the optional feature enabled. The warning ceiling is unchanged. Redundant visibility within private modules and trivial const/lossless conversion warnings were cleaned up in touched code.

## Existing verification

- Full library suite with `clap,vst3,licensing,rt-paranoid`: 395 passed, 55 ignored. The same suite with `experimental-1x-dsp`: 395 passed, 55 ignored. Ignored checks are not counted as passing.
- Existing expression-capacity harness from PR #12 (`75888d1`): five checks pass, plus the explicitly executed formerly ignored nested-composition overflow case. Its test modules were retained verbatim while its production evaluator was replaced with the candidate evaluator.
- Existing `run_custom_quality_proofs.py` from that branch: three checks pass in portable and native builds. Against the main baseline, the fract and mixed-curve parity checks fail as expected.
- Existing PR #15 integration harness rebased onto main: both baseline and AVX2 dispatch pass 144 explicit PM route cases bit for bit, the high-note Fourier/crossover oracle, 720 custom/warp cases (maximum scalar/x8 difference 1.12e-5), and 1,120 factor/shape/frequency/width cases (maximum difference 3.00e-5). Its adapter reuses the current PM adapter through `include!`; test bodies are unchanged.
- Existing PR #13 triangle harness: 15,728,640 bit-identical comparisons. Native scalar component timing reports 1.1–7.4% lower time across its eight rows. This is not a whole-plugin CPU result.
- Current-source public DSP integration: all existing library checks and six seeded baseline/AVX2 runs pass, including 56 PM oracle cases, 1,964,032 stereo partition comparisons per seed, and 448 factor/shape cases per dispatch. The recorded source hashes match the final DSP files.
- Clippy: default 6,519 warnings; optional feature 6,523; existing ceiling 6,523. Source formatting and `git diff --check` pass.

Proof output and paired callback records are in [the evidence directory](finish-dsp-2026-09-05/). Separate proof worktrees preserve the original PR test bodies because this task did not authorize test changes.

## Whole-plugin callback measurements

Ryzen 7 7800X3D, Linux, Rust 1.97.1 release/thin-LTO, generic x86-64, identical features `clap,vst3,licensing,process-lab`; optional 1x disabled. Existing process_lab and paired-matrix runner, alternating AB/BA order, three rounds, 256 warmup and 2,048 measured callbacks per run, 64 frames / 48 kHz: **1.333 ms deadline**. The first matrix contains 32 cases, including one-voice rows in the raw record. The follow-up contains six 16-voice cases.

These are shared-desktop measurements. No agent-owned KURV compilation ran during timing; unrelated BUFFR work and OS scheduling remained active. Four-core affinity deliberately limits the seven-helper pool; eight-core affinity is a separate follow-up, not a replacement for unfavorable rows. Values are baseline time / candidate time: above 1 is faster. Misses sum all three runs per build.

| Affinity | Scenario (16 voices) | Factor | Paired speedup | Deadline misses, main → candidate |
|---|---|---:|---:|---:|
| 4 cores | solo-saw-1 | 1 | 1.009 | 0 → 0 |
| 4 cores | solo-saw-1 | 4 | 0.773 | 11 → 32 |
| 4 cores | solo-saw-64 | 1 | 1.000 | 0 → 0 |
| 4 cores | solo-saw-64 | 4 | 1.038 | 4 → 0 |
| 4 cores | solo-triangle-1 | 1 | 1.014 | 0 → 0 |
| 4 cores | solo-triangle-1 | 4 | 0.967 | 0 → 0 |
| 4 cores | xpm-1x64 | 1 | 1.024 | 0 → 0 |
| 4 cores | xpm-1x64 | 4 | 1.011 | 0 → 0 |
| 4 cores | xpm-64x1 | 1 | 0.979 | 0 → 0 |
| 4 cores | xpm-64x1 | 4 | 0.950 | 0 → 0 |
| 4 cores | xdepthpm-64x64 | 1 | 1.064 | 22 → 6 |
| 4 cores | xdepthpm-64x64 | 4 | 1.007 | 122 → 118 |
| 4 cores | xcyclepm-64x64 | 1 | 1.004 | 0 → 0 |
| 4 cores | xcyclepm-64x64 | 4 | 0.978 | 153 → 171 |
| 4 cores | gfilter-scream-cutoff-1x64-depth | 1 | 0.928 | 0 → 0 |
| 4 cores | gfilter-scream-cutoff-1x64-depth | 4 | 0.969 | 0 → 0 |
| 8 cores | solo-saw-1 | 1 | 1.014 | 0 → 0 |
| 8 cores | solo-saw-1 | 4 | 1.037 | 0 → 0 |
| 8 cores | xdepthpm-64x64 | 1 | 1.005 | 0 → 0 |
| 8 cores | xdepthpm-64x64 | 4 | 1.005 | 14 → 21 |
| 8 cores | xcyclepm-64x64 | 1 | 0.929 | 0 → 0 |
| 8 cores | xcyclepm-64x64 | 4 | 1.010 | 18 → 15 |

No universal speedup follows from these results. The four-core saw tail regression did not repeat with eight cores; cyclic PM and heavy workloads still expose deadline misses. Tightening the wait budget is a realtime policy correction, not proof of faster callback completion. No scheduler model was changed speculatively to improve this table.

Measured binaries (SHA-256):

- Main: `cb46cf6393410a16d7f723f66fb9f8a3991c8d6db7e13a17eccee3f04cabc0c2`.
- Candidate: `7c761564c702a62c930d2ad6018c2339296e8fecb179f4d7922c7d5bb8bd8d6c`.

The candidate timing build predates only a const annotation on an unused field-audit function, lossless cast cleanup and redundant-branch removal in the disabled 1x feature, CI coverage, and documentation. It is not a release artifact or a host acceptance result.

## Quality and research boundary

The [research refresh](../research/dsp-research-refresh-2026-09-05.md) covers primary sources through August 2026 / DAFx26. New additive-sync research uses costly coefficient transforms and specialized hardware; polygon correction still combines with oversampling. Neither establishes a universally faster replacement for this engine. Existing harmonic-mip infrastructure remains the reuse point.

The optional 1x crossover retains the previously measured 2.3–2.4x saw crossover cost and nested-PM limitations documented in PR #15; those historical timings were not remeasured here. Reconstruction error, wanted-spectrum change and alias energy are different claims. This change does not establish world-fastest CPU, universal alias-free modulation, DAW listening/project-reopen acceptance, or a production release.
