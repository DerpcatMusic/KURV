# Jitter final optimization experiment — 2026-08-06

## Decision

Reject every candidate from this round. None met the shipping gate of at least 5% lower dense `24 notes × 64 lanes × 3 oscillators` median cost with no more than 2% regression elsewhere. The branch intentionally contains no DSP change.

The retained implementation already has the required structural fast exits: jitter is inactive for one lane, zero amount, or zero range; pan and gain remain static; pitch motion stays deterministic and bounded; and no per-lane sine/cosine voice arrays are stored.

## Method

- Base: `684a3c85e2daf85cc165db65761963a14e33e0da`
- Generator: Spline 4PT optimized, Normal 2×, saw, MIDI 60, 24-note polyphony
- Portable build: `-C target-cpu=x86-64`
- v3 build: `-C target-cpu=x86-64-v3`
- Matrix: 32,768 frames, three repetitions, 100 Hz jitter update stress
- Dense paired gate: 65,536 frames, five repetitions, 0.7 Hz, baseline and candidate interleaved
- `ns/frame` is a wall-time median and short low-lane runs contain scheduler noise. Checksums and retired instructions were used as secondary evidence.

## Baseline matrix

Each cell is median `ns/frame`; columns are Off / Noise / Sine.

### Portable x86-64

| Lanes | Oscillators | Off | Noise | Sine |
|---:|---:|---:|---:|---:|
| 1 | 1 | 752 | 675 | 718 |
| 1 | 3 | 1,774 | 1,554 | 1,686 |
| 2 | 1 | 556 | 629 | 686 |
| 2 | 3 | 2,327 | 2,155 | 2,686 |
| 4 | 1 | 683 | 546 | 693 |
| 4 | 3 | 1,264 | 1,408 | 1,847 |
| 8 | 1 | 688 | 645 | 794 |
| 8 | 3 | 1,695 | 1,636 | 2,220 |
| 64 | 1 | 2,945 | 2,757 | 3,464 |
| 64 | 3 | 8,586 | 7,812 | 9,776 |

### x86-64-v3

| Lanes | Oscillators | Off | Noise | Sine |
|---:|---:|---:|---:|---:|
| 1 | 1 | 444 | 448 | 431 |
| 1 | 3 | 1,124 | 1,124 | 1,108 |
| 2 | 1 | 447 | 459 | 516 |
| 2 | 3 | 1,631 | 1,515 | 1,798 |
| 4 | 1 | 435 | 451 | 611 |
| 4 | 3 | 1,078 | 1,090 | 1,378 |
| 8 | 1 | 489 | 449 | 553 |
| 8 | 3 | 1,029 | 1,053 | 1,387 |
| 64 | 1 | 2,000 | 1,548 | 2,089 |
| 64 | 3 | 4,446 | 4,234 | 5,801 |

At one lane, all three modes produced identical checksums on each architecture. The apparent timing differences are measurement noise, not hidden 64-lane work.

## Rejected candidates

| Candidate | Result | Reason rejected |
|---|---|---|
| Reuse the existing `exp2_block` for Noise | Portable: about 1.5% better at 0.7 Hz and 0.7% worse at 100 Hz. v3: 0.5% worse at 0.7 Hz and 1.0% better at 100 Hz. | Below 5%; inconsistent by architecture and cadence. |
| Fold the narrow-ramp classifier into the render pass | Retired instructions improved about 1.0% for Noise and 0.9% for Sine; wall time improved 0–1.5%. Output was bit-identical. | Below 5%. |
| SIMD Sine target generation plus SIMD centering/scaling | v3 dense Sine improved about 1.4%; portable regressed and its checksum changed. | Below 5%, portable regression, and trajectory was not preserved. |
| Relax unrefined reciprocal drift from `2e-5` to `4e-5` | Retired instructions improved about 1.0% for Noise and 1.68% for Sine. | Below 5%. |
| Relax unrefined reciprocal drift to `6e-5` or `1e-4` | Existing debug comparison tripped: adaptive spline error exceeded `1e-5`. | Violates the signal-error gate. |
| Generate refined reciprocal steps by recurrence, combined with single-pass classification | Existing debug factor sweep stayed below `1e-5`, and dense Noise checksums were bit-identical. Five paired v3 medians were 6,802 ns/frame baseline versus 7,037 candidate. | 3.45% slower; Noise alone conclusively failed the gate, so the candidate was stopped before redundant Sine profiling. |

Normal-rate v3 retired-instruction baselines for 262,144 frames were 15.831 billion Off, 23.721 billion Noise, and 25.956 billion Sine. This confirms that jitter overhead is real, but the tested micro-optimizations do not remove enough of it to justify more hot-path branches or duplicated kernels.

## Useful next seam

The remaining cost is distributed through per-sample moving-step antialiasing rather than target-update generation. A future round should only reopen this path with a materially different kernel: for example, a provably bounded polynomial reciprocal update or a moving-step BLEP formulation that avoids per-frame correction entirely. Small rearrangements of the current kernel have exhausted their measurable value.
