# Canonical immutable coefficient-backend boundary (2026-08-30)

## Verdict

Do not integrate the cap-2/3/6 coefficient backend into canonical rendering in
this round. Immutable saw/square banks remove the custom-table publication
problem and exact shape values are reachable, but the previously validated
single-Horner transition does not transfer to the canonical endpoint. Current
1x canonical output is phase-step-dependent BLEP/spline DSP, not evaluation of
a fixed raw polynomial. A coefficient interpolation therefore cannot land
output-identically on current1. Correct fallback requires both kernels during
the 1,024-sample bridge and must be wired through every optimized canonical
render seam.

Production remains unchanged at 0.8.7. This rejection preserves the accepted
static cap-6 coefficient quality result; it rejects an unsafe assumption about
transition equivalence and the resulting broad integration, not the baked
representation itself.

## Exact-shape reachability

KURV maps Shape `2.0` to exact saw and `3.0` to pulse/square. Square additionally
requires Pulse Width `0.5`. The host parameters use an 8 ms linear smoother.
Once a steady target reaches its endpoint, its f32 value is exact and remains
eligible; equality is not intrinsically measure-zero for a static preset.

The practical limits are:

- at 48 kHz, the shape/PW smoother consumes up to 384 samples, followed by the
  proven 1,024-sample backend slew: 1,408 samples or 29.33 ms before full use;
- notes shorter than that never reach the full projected backend after a shape
  change;
- continuous Shape automation/modulation is ineligible except at isolated
  endpoint dwell periods;
- square loses eligibility during any pulse-width automation away from 0.5;
- custom mix, phase warp, and canonical morph remain fallbacks;
- the cap-at-most-6 pitch region begins around a normalized step of `1/12`
  (approximately 4 kHz fundamental at 48 kHz), so the optimization is already
  restricted to high notes.

Thus exact eligibility is narrow but musically reachable. It is not, by itself,
the rejection gate.

## Why the custom transition proof does not transfer

For `WaveCurveRt`, both endpoints are cubic coefficient sets. Linearity gives:

```text
Horner(lerp(coefficients)) == lerp(Horner(A), Horner(B))
```

within f32 rounding. Canonical current1 instead evaluates sine/triangle/saw/
pulse kernels with PolyBLEP or spline corrections selected from phase step,
antialiasing mode, pulse width, warp, and morph state. There is no fixed set of
four current1 segment coefficients to interpolate with the projected bank.

Using an unclamped linear saw or piecewise-constant square as the raw endpoint
would not be a fallback to current1: at the exact high-note pitches where the
backend is eligible, it would reintroduce the aliasing and edge difference the
current BLEP/spline path exists to prevent. Calling that output-identical would
invalidate the artifact gate.

A correct transition therefore needs:

1. the current canonical kernel;
2. the selected projected polynomial kernel;
3. a reversible 1,024-sample output mix.

That is a dual-kernel transient. The single-Horner experiment cannot be used as
its CPU evidence.

## Render and state surface

Canonical rendering is deliberately specialized. `src/oscillators/va/render.rs`
exposes 33 generator/accumulator entry points covering scalar, x4, x8, paired
oscillators, constant blocks, step blocks, dynamic gains, morphing, warp, and
dedicated saw/pulse fast paths. Exact saw selects those dedicated paths, so
patching only `sample_shape_normalized` or `generate_shape8` would silently miss
the representative structural workload.

Eligibility can also change inside shape/width automation blocks. If those
fallback paths do not advance the same selector and render the dual-kernel
bridge, returning to an exact shape resumes stale state and clicks. Reset,
phase-position changes, PM source paths, topology fades, and internal worker
jobs all need the same contract.

KURV allocates up to 2,240 `VaOscillator` objects per polyphonic voice: 192 in
the legacy bank plus 2,048 in the 32-slot structural bank. Across 32 voices this
is 71,680 oscillator objects. Two `f32`s represent 573,440 bytes of additional
field storage before layout effects; one 11-bit fixed-point selector represented
as `u16` is still 143,360 bytes. A separate packed selector store makes that
cost explicit and avoids relying on unspecified Rust field padding, but requires
another parallel legacy/structural lane array and changes every block-render
signature.

The minimal logical state is one 11-bit current selector coordinate; the target
is derived each sample and reversal needs no history. The current object/layout
has no stable `repr(C)` padding contract that can be safely treated as free
storage.

## Existing CPU evidence

The earlier exact-additive canonical harness established current x8 block costs
near 19-28 ns per host frame for high-note saw/square and shipping 2x costs near
61-88 ns. The cubic `eval8` probe is much cheaper in isolation, so a steady
baked backend remains plausible. However the earlier continuous canonical
transition experiment also showed why transitions must be measured in the real
seam: dual-kernel transition bands cost several times current1 for analytic
kernels.

The cubic backend may make that transient affordable, but proving it requires a
new canonical dual-kernel structural prototype across the dedicated saw and
pulse block functions. The custom single-Horner structural numbers cannot be
substituted. Shipping a partial scalar/general integration would leave the hot
x4/x8 paths unchanged and add state without representative benefit.

## Smallest justified follow-up

Before production integration, add one test-only canonical structural backend
at the dedicated constant x8 saw and 50% square seams, with selector state held
outside `VaOscillator` so object size is unchanged. It must measure:

- steady cap-2/3/6 cubic quality and CPU against current1 and shipping2;
- continuous dual-kernel 1,024-sample entry, exit, and reversal CPU;
- exact Shape/PW smoothing and abrupt host automation;
- whether a packed side-state array harms cache behavior more than embedding
  eight padded bytes.

Only after that gate should the remaining scalar/x4/dynamic/PM paths and baked
coefficient source be added. Doing all 33 seams now would be a broad production
rewrite based on transition evidence for a different endpoint type.

## Verification

```bash
git rebase main
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
  cargo check --all-targets --no-default-features --locked
```

The all-target check passes with current warnings. No baked constants, runtime
state, test, dependency, Cargo version, or production source change is retained.
