# Custom waveform quality: reproducible defects and remaining limits

Audited base: `d084681411a95803bb52206647c2bc881c4cbf8b` (0.8.145).
This PR fixes two scalar/SIMD sound inconsistencies. It does **not** add a new
antialiasing engine or claim any end-to-end CPU improvement.

## Confirmed and fixed

### Negative `fract()` operands changed the sound with lane width

`src/wave_curve/function.rs` used Rust's signed fractional part in scalar playback
and `v - floor(v)` in x4/x8 playback. `fract(x-0.5)` at phase 0.25 returned -0.25
in scalar and +0.75 in SIMD: an absolute difference of **1.0**. This is not a
floating-point tolerance issue. The scalar and SIMD custom renderers consume
these evaluators (`src/oscillators/va/render.rs` `generate_custom4/8`,
`sample_custom_shape_with_antialiasing_warped`). Scalar also renders unison tails.

Fix: define expression-language `fract(v)` as `v-floor(v)` in every width.
Existing SIMD expressions keep their sound; negative-operand scalar expressions
change to match them. Positive arguments are unchanged. No extra SIMD work,
allocations, tables, state, or increased oversampling is introduced.

### Spline/function transitions clamped in different orders

`WaveCurveRt::eval()` blended a raw spline with a function and then clamped;
`eval4()` and `eval8()` clamped the spline before blending. Cubic interpolation
can overshoot even with bounded interpolation samples. A bounded four-point
fixture `[0,1,1,0]` at local positions `[0,1/3,2/3,1]` produces a midpoint 1.125.
At a 25% transition to zero, scalar emitted 0.84375 and SIMD 0.75. The negative
fixture had the same mismatch. This test uses a synthetic coefficient fixture;
it establishes evaluator correctness, not the prevalence of that curve in presets.

Fix: clamp each source before scalar blending, matching shipping SIMD. The
scalar path still executes one clamp, moved before the blend. Pure spline and
pure function endpoints are unchanged. `WaveCurveRt::interpolate()` is used by
VA frame selection and runtime transitions, so the mixed-source state is real.

## Executable proof and limits

Normal Rust regression tests live beside the changed code. Because the full
plugin requires the unavailable sibling `derpcat-access` dependency, an isolated
runner executes **actual source** (the complete function module and verbatim
selected `WaveCurveRt` method bodies), with `wide=0.7.33` f32x4/f32x8 re-exported
at their usual import path. No fake vector operations or translated DSP is used.
The runner does not exercise plugin routing, GUI, decimation, or scheduling.

```sh
python docs/audits/run_custom_quality_proofs.py
python docs/audits/run_custom_quality_proofs.py --native
# Run new regression assertions against an untouched old checkout:
python docs/audits/run_custom_quality_proofs.py --source-root /path/to/base-checkout
python docs/audits/custom_aliasing_math.py
```

An explicit Cargo binary/toolchain can be supplied with `--cargo` and
`--toolchain`. `--native` exercises the AVX2/FMA coefficient layout on capable
x86 hardware; portable exercises the non-AVX2 layout. These are correctness runs,
not benchmarks. Full-plugin CI and a host test remain required before release.

Measured in this audit using Rust 1.97.1:

- Base source: both parity regressions fail (the values above).
- Patched portable and native: both parity regressions pass.
- Actual expression evaluator: `sin(tau*x*65)` at 64 samples per base cycle
  matches a unit-amplitude first-harmonic alias within 0.00006 absolute error.
  This is an evaluator witness, not a complete oscillator render measurement.

## Unresolved antialiasing boundaries

### Direct custom evaluation can alias before decimation

The full-custom path in `render.rs` directly evaluates `curve.eval*()` at the
warped phase. Unlike the canonical branch, it does not add a discontinuity
correction or select a pitch-dependent harmonic representation. The
`BandlimitedWaveCurve` implementation exists, but its compile/playback references
outside its own module occur in tests; it is not selected by these renderers.

At 96 kHz internal rate, base 1500 Hz and expression `sin(tau*x*65)`, the intended
97.5 kHz component samples into 1500 Hz. The mathematical witness differs from a
unit 1500 Hz sinusoid by at most **5.77e-14** in float64. That alias is already
inside the output passband. An output decimation low-pass cannot selectively
remove it. More oversampling moves this specific folding point; it does not
provide a bound for arbitrary expressions.

This does not prove all custom waves sound poor. It disproves a blanket alias
safety guarantee. The expression's clamp is another nonlinear stage: clipping
`2*sin(tau*x)` produces a third harmonic **12.91 dB below its fundamental** in
the continuous waveform witness. Accounting only for the expression before its
output clamp would miss these harmonics.

### Ratio filtering followed by warp is an existing documented contract

Correction to the initial review: `docs/research/oscillator-domain-filtering-2026-08-31.md`
already states that the current ordering is `warp(filtered_cycle)` and that an
exact post-warp brickwall needs another representation. This is a known limitation,
not an undocumented implementation defect discovered by this PR.

With only the canonical fundamental retained and maximum Harmonic warp, the
second harmonic is **6.82 dB below the first** in the mathematical witness.
That is spectral expansion from warp, not automatically aliasing; it becomes
an aliasing concern when new components exceed Nyquist. Ratio Brickwall currently
only substitutes the canonical VA source when custom mix is zero. Its bus filter
case returns input unchanged. It must not be represented as a general filter for
arbitrary custom sources, noise, or already-mixed/nonlinear downstream signals.

Integer ratio edges intentionally select whole harmonics using floor(). Crossing
2x on a saw adds a second partial with peak amplitude **1/pi = 0.31831**,
**-9.94 dBFS** in the canonical Fourier normalization. Actual instantaneous
jumps depend on phase. Smooth automation of the cutoff number does not smooth
that coefficient switch. Whether stepped harmonic selection is desirable is a
product choice; a continuous mode would need fractional edge weighting and a
stated changed spectral contract.

## CPU-conscious workarounds and next experiments

1. **Avoid unnecessary bandwidth at the source.** For a known unwarped harmonic
   expression choose its highest harmonic below internal Nyquist. This bound is
   insufficient after clipping, warp, FM/PM, or audio-rate waveform modulation.
   Lower drive/warp and greater oversampling can reduce particular artifacts;
   none gives a universal bound for arbitrary custom functions.
2. **Profile the expression interpreter separately.** Constant folding and
   specialization of common compiled expressions could remove per-sample opcode
   work. Preserve evaluation order and nonfinite/clamp semantics. Measure single,
   four, eight, and tail lanes; SIMD currently sanitizes via array conversion,
   and the AVX2 four-lane spline path runs an eight-lane kernel with padding.
   These are candidates, not proven speedups.
3. **Treat static and time-varying custom sources differently.** A precompiled
   harmonic/mipmap representation can amortize static custom processing, with
   an explicit interpolation error bound and background compilation. Measure
   its memory/cache cost against the 16-segment evaluator before adopting it.
   It does not by itself solve audio-rate phase or waveform modulation.
4. **Prototype event-aware polynomial correction for procedural curves.**
   Segment/wrap discontinuity corrections or an antiderivative approach require
   crossing-time handling, derivative discontinuities, clamp breakpoints,
   modulation trajectories, and a high-event-count fallback. Do not assume a
   cubic spline is bandlimited or that ADAA alone guarantees clean nested FM.
5. **Compare cost at matched quality.** Record wanted-harmonic attenuation,
   in-band aliases against a high-rate reference, total residual, modulation
   sidebands, and callback worst-case separately. Benchmark one voice/unison
   lane, four/eight/tails, dense polyphony, and nested audio-rate modulation.
   A faster method that silently drops intended modulation bandwidth is not
   evidence of a better optimized equivalent synth.

No antialiasing replacement or stepped-ratio behavior change is shipped in this
PR: both would alter timbre and require measured quality/CPU acceptance gates.
