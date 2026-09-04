# 1x DSP implementation and competing methods

Base: `d084681411a95803bb52206647c2bc881c4cbf8b` (KURV 0.8.145).
This is a development proposal, not a shipped release or a fastest-synth claim.

## Scope and decisions

Four parallel investigations cover direct harmonic synthesis, compact polynomial
correction, signed event correction, and actual production render routing.
The objective is quality per CPU cost at the output sample rate. Only offline
reference signals run at elevated sample rates; none of the proposed renderers
secretly oversamples its output.

| Method | Result | Decision |
| --- | --- | --- |
| Direct high-note harmonic specialization | Avoid general harmonic loops, coefficient caches and drift; preserve authoritative phase | Integrate behind `experimental-1x-dsp` and test production routes |
| Bounded generic Fourier/Clenshaw | Strong stationary quality; some packed high-note gains, expensive scalar/setup work | Retain reproducible candidate, not a blanket replacement |
| Guarded 6/8/12-sample polynomial BLEP | Lower stationary unwanted spectral energy; longer support and overlapping events cost CPU | Retain scalar/SIMD candidates and measured tradeoffs |
| Signed BLEP plus phase-knot BLAMP | Improves tested reversal and nested-PM reconstruction, but 7–8x scalar cost and 17 samples latency | Reject as the default |

## Evidence and reproduction

- [Production routing and direct specialization](../../tools/audits/one_x_integration/README.md)
- [Bounded harmonic candidates and oracle](../../tools/audits/one_x_harmonic/README.md)
- [Guarded compact corrections, coefficient generator and spectra](../../tools/experiments/one_x_correction/README.md)
- [Signed event reconstruction, continuous/linear references and timing](../../tools/audits/one_x_events/README.md)
- [Actual high-note kernel PM counterexamples](../../tools/audits/one_x_pm/README.md)

At 12,949.22 Hz / 48 kHz, the actual direct high-note scalar renderer's static
reconstruction error is approximately -136.6 dB for saw and triangle, versus
about -7.4 dB for the shipping renderer. This is relative total error against
an exact finite Fourier series, not alias power or a complete synth measurement.
Restoring wanted amplitude also changes timbre and level; existing presets can
sound brighter/louder. There is no added latency or retained harmonic state.

Actual constant-block measurements with the AVX2 backend at base step .30
(14.4 kHz at 48 kHz; detuned lanes) give the following candidate/baseline CPU
cost ratios. Lower is cheaper; these are not whole-voice measurements.

| Shape | 1 lane (time SIMD) | 8 lanes | 64 lanes |
| --- | ---: | ---: | ---: |
| Saw | .654 | .466 | .438 |
| Triangle | .491 | .598 | .592 |

At the .225 crossover, saw costs 2.29x/2.40x for 8/64 lanes and triangle
1.93x/1.83x. A mixed pack can enter this region even when its base step is .19.
The first generic saw integration cost 8–9x there; precomputed native AVX2
rendering removed most of that regression, but the remaining dual-render cost
is real. This is a focused high-note win, not a universally cheaper engine.

The same kernel worsens two intense nested-PM cases by 1.36 dB (saw) and 1.71 dB
(triangle). Both are retained as counterexamples. Generator graph entry points
therefore keep sources and targets on the original renderer whenever a generator
route or legacy modulation source is configured, including zero depth. The
explicit packed PM API also preserves the baseline. A 144-case actual-API test
checks output and phase state bit for bit. The graph guards received source review;
they are not covered by the standalone VA executable's compile boundary. Shared
time-SIMD APIs and nongenerator control/LFO modulation are not claimed universally
protected, and remain part of the full-plugin validation gate.

Raw quality and timing data accompany each harness. Timings are measurements on
a shared virtualized AMD EPYC runner, not portable guarantees. Kernel results
exclude work explicitly identified in each report and must not be presented as
whole-plugin, polyphony, unison or host-callback speedups.

## What remains unresolved

Stationary bandlimiting does not guarantee bandlimited audio-rate modulation.
PM, nested modulation, changing width and nonlinear warp can create new sidebands.
Correcting the discontinuities of a linearly reconstructed phase does not recover
the missing continuous phase trajectory. The event proof measures this distinction
against both targets instead of claiming that an accurate linear model solves PM.

Metrics separate wanted-bin complex error, unwanted-bin energy and total
reconstruction error. Unwanted-bin energy is only a lower bound on alias power:
folded components can coincide with wanted harmonics. Guard bands and harmonic
tapers remove some wanted content, which is reported separately.

The complete plugin build remains blocked by its private sibling dependency
`derpcat-access`. The production integration harness compiles actual DSP modules
while removing host serialization adapters; it does not validate those adapters,
the plugin ABI, licensing, GUI, whole voice graph or DAW behavior. Full host and
quality-matched performance matrix runs remain a release gate.

No release version bump is included because this draft does not ship a change.
