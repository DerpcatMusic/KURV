# Warped pulse live-safety experiment — rejected

Branch: `experiment/warped-pulse-live-safe`  
Base: `44d15a6ca8f380f5a666633958933e5d849a3b88`  
Experiment: `d0d8240`  
Revert: `ccb89c1`

## Question

Can KURV keep the alias advantage of its raw-edge warped-pulse correction while avoiding a 24-step inverse solve on every smoothed automation sample, making scalar, SIMD4, SIMD8, and remainder lanes agree, and remaining correct when warp depth becomes phase-step dependent?

## Candidate

- Solve once, then update the raw pulse edge from the previous result with bounded Newton refinement.
- Canonicalize the result at the exact floating-point support boundary.
- Use the same raw-edge correction in scalar, SIMD4, SIMD8, paired, and remainder renderers.
- Fall back to the warped-domain correction above the phase-step-independent depth limit.
- Mark both entry and exit from the specialization with KURV's existing 0.5 ms output-continuity ramp. The 5 ms lane fade remained exclusive to adding or removing unison lanes.

The implementation allocated and locked nothing on the audio thread, but added 482 lines and removed 56.

## Results

The generalization materially improved low-count alias rejection at pulse width 0.37, 98% warp, 2x, FFT bin 5003. Results are residual dBc; more negative is better.

| Mode | Main scalar/SIMD4 | Candidate scalar/SIMD4 | Existing SIMD8 |
|---|---:|---:|---:|
| PWM | -70.992 | -86.423 | -86.423 |
| Bend | -60.209 | -86.159 | -86.160 |
| Harm | -78.419 | -92.986 | -92.987 |

The live automation instruction counts improved on x86-64-v3 by about 17%, 10%, 15%, 2%, 19%, 11%, and 5% for 1, 2, 4, 7, 8, 9, and 64 lanes. Portable x86-64 improved by about 10%, 5%, 8%, -0.5%, 9%, 5%, and 1%. The dense portable result did not clear the 5% retention gate.

The strict output gate also failed. Bend and Harm could be made sample-identical to the exact solve, but high-depth PWM is locally non-monotonic. A seeded inverse can select a different valid edge from the global bisection. Even after exact support-boundary canonicalization, PWM reached a maximum sample delta of `1.41144e-4`, above the `1e-5` limit.

The phase-step fallback was not acceptable either. At FFT bin 31001, the generic high-step path measured roughly -5 to -11 dBc residual while the existing SIMD8 raw-edge path measured -73 to -80 dBc. It made lane behavior mathematically consistent by discarding too much alias performance.

## Decision

Reject and revert the candidate. Its scalar/SIMD4 alias improvement is real, but the live PWM mismatch, high-step alias regression, weak dense-portable gain, and 482-line cost do not clear the correctness and value gates together. The branch preserves the implementation for future reference; its tip restores source exactly to the base plus this report.

A future attempt should solve the phase-step-dependent raw discontinuity directly, with a monotonic PWM transfer or a cheap closed-form/table-assisted inverse that is lane-width independent. A generic warped-domain fallback is not a viable high-note solution.
