# PM and nested-depth improvements

Base: `d084681411a95803bb52206647c2bc881c4cbf8b`. Development proposal;
no release version bump or claim of a universally fastest synthesizer.

## Why a parent can cost too much

Modulating a depth is inexpensive arithmetic: form the depth from its base and
parent values, then multiply the modulator output by that depth. Rendering a new
source still has a cost. Routing, repeated metadata traversal, loss of vectorized
blocks and nonlinear oscillator antialiasing are separate costs.

The existing nested-depth block path traversed its linked parent list again for
every sample. The new path traverses each parent once per block and applies its
multiply/add to a contiguous sample buffer. It preserves sample rate, addition
order and clamp position. It does not turn audio-rate modulation into control-rate
modulation. A single voice-LFO override without generator parents already had a
cheap buffer path; the measurements do not claim that path was universally slow.

Mixed PM+gain routes previously rejected generator-depth parents, and a voice LFO
also disqualified their block renderer even when its depth samples had already
been prepared. Feed-forward parent routes can now stay in the existing fast-mode
block renderer. Both PM and gain consume their actual depth buffers. Feedback,
carrier reuse as a source, unsupported controls and other prior restrictions stay
excluded. A depth-only voice LFO is admitted only when its amount buffer exists.
This does not establish support for arbitrary multi-child voice-LFO depth graphs.

## Proof and measured scope

The [runtime AVX2 PM backend](pm-runtime-avx2.md) fills another actual gap:
ordinary saw blocks already had a native backend, but PM blocks did not. The new
narrow-saw paths use the same spline coefficients and modulation equation.
Portable-binary PM stereo/lane blocks measure 42–48% median lower cost for
8/16/64 aggregate lanes on the tested CPU. Dispatch requires AVX2+FMA, at least
16 frames, and the applicable saw path; globally native builds retain their
original kernel. Peak tested sample difference is 2.384e-7, with exact final
phase state. This is a CPU improvement, not a new antialiasing algorithm.

- [Parent-depth proof and raw timings](pm-parent-depth-routing.md): 2,920,320
  bit-identical comparisons each in portable/native builds. Nested route-buffer
  construction costs 73–94% less CPU portable and 84–97% less native in the test
  matrix. Example: 32 frames, one route, four parents, 142 to 22 ns portable.
- `tools/audits/pm_depth/eligibility.py`: 192 actual topology/classification
  fixtures cover feed-forward parents and excluded feedback/mismatched graphs.
- `tools/audits/pm_depth/mixed_gain_proof.py`: 19,200 comparisons compile the
  actual new gain helper and the original gain block. Static output is bit-identical;
  tested changing-depth output also has zero measured sample difference.
- `tools/audits/pm_integration`: compiles complete production VA modules, with
  host serialization adapters removed, and compares rendering paths and PM against
  actual scalar sampling. It does not compile the full voice/host layer.

These timings describe named components, not total synth CPU. The mixed-graph
eligibility change has topology and gain/amount proofs, but no end-to-end host
speedup claim: the private sibling `derpcat-access` still blocks the full plugin
build. Full voice compilation, pool/serial rendering and host tests remain gates
before merging/releasing these changes.

## PM sound quality is a separate problem

[The new quality experiments](../../tools/audits/pm_quality/README.md) test actual
carrier-only spline correction, phase-travel-dependent widths, and exact analytic
integration of a linearly reconstructed saw phase. All render at 1x; only the
offline oracle oversamples. Simply changing correction width regresses by as much
as about 5 dB in tested cases and is not enabled.

The analytic integral improves total reference error in 87/96 cases, median
4.06 dB, but worsens nine cases (up to 3.36 dB). It introduces a half-sample timestamp
shift and box-filter attenuation. The reference compensates that known timestamp
analytically, without fitted alignment/gain. Chaining the method through several
operators changes graph timing; there is no justified automatic production
replacement yet. The implementation, SIMD variant, tests and counterexamples are
retained as an unregistered experimental module. CPU fixes do not claim these
quality gains for the running synth.

Arbitrary audio-rate FM/PM and nonlinear waveforms can generate new sidebands.
Cheap depth arithmetic alone cannot guarantee removal of those aliases. Feedback
and true intersample reconstruction require their own quality/cost decisions.
