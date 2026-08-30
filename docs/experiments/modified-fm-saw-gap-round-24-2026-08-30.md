# Modified-FM saw gap audit, round 24

Date: 2026-08-30

Baseline: `214b75ef90edbcc420af2d05bdfc6c4bb4d823e7`, KURV `0.8.9`

Status: rejected at the saw-first gate. The ignored probe was removed after
recording the result. Production DSP, package version, dependencies, and
oscillator layout are unchanged.

## Decision before implementation

The one genuinely untested architecture selected by this round is the
modified-FM pulse oscillator of Timoney, Lazzarini, and Lysaght:

\[
p_k(\theta)=e^{k(\cos\theta-1)}\cos\theta.
\]

Its harmonics follow a smooth modified-Bessel envelope. A saw is obtained by
removing the pulse mean and integrating. This is mathematically distinct from
every locally tested family:

- it is not an additive/IDFT/Clenshaw harmonic loop or a finite Fourier table;
- it is not BLIT: the analytic pulse is not a periodic sinc and retains an
  infinite, smoothly decaying spectrum;
- it deposits no BLEP/BLAMP/PTR event residual and keeps no correction ring;
- it is not DPW, ADAA, a spectral mip, coefficient compression, time-packed
  evaluation, or an ISA-specific scheduling change.

The pitch-dependent modulation index `k` continuously trades wanted harmonic
extension against the infinite tail. There is no integer harmonic-cap switch,
table mip boundary, event-density branch, or fractional crossing schedule.
That gives it a plausible transition path. The sample kernel is one cosine,
one exponential approximation, one subtraction, and one integrator update,
all lane-vectorizable. The decisive risks are equally clear: wanted-harmonic
droop, the remaining infinite tail, exponential cost, retained integrator
state, DC control, and reset/pitch-state reconstruction.

The probe therefore stops at saw. The paper derives square and triangle by
further integration, but implementing them before saw clears ideal-projection,
transition, and CPU gates would add code without evidence.

## Exhaustive local report audit

All 68 reports under `docs/experiments` at the baseline were read. The groups
below account for every report once.

### Reference, phase-warp, and accepted production infrastructure

- `va-oscillator-reference-harness-2026-08-30.md`
- `va-static-phase-warp-reference-2026-08-30.md`
- `va-warped-pulse-edge-block-preparation-2026-08-30.md`
- `va-portable-indexed-eval4-round-14-2026-08-30.md`
- `va-derivative-jump-regularized-round-8-2026-08-30.md`

The harness already defines the correct target: exact ideal band-limited
projection, aligned curve RMS/peak, wanted harmonics, artifacts, and actual
shipping CPU. Production is a continuous phase accumulator feeding canonical
raw shapes plus optimized support-two spline BLEP/BLAMP. Scalar paths can exit
outside event support; packed paths use event masks and a runtime-selected
AVX2/FMA constant-saw backend. The retained 256-byte custom curve remains 16
uniform cubics. Version 0.8.9 also prepares invariant warped pulse edges once
per fixed block.

### Exact Fourier, additive, recurrence, projection bank, and mip families

- `canonical-additive-crossover-round-4-2026-08-30.md`
- `canonical-continuous-transition-round-5-2026-08-30.md`
- `clenshaw-canonical-round-7-2026-08-30.md`
- `exact-fourier-high-note-crossover-2026-08-30.md`
- `exact-fourier-production-crossover-round-2-2026-08-30.md`
- `canonical-cap3-additive-transition-round-17-2026-08-30.md`
- `canonical-high-note-crossover-round-16-2026-08-30.md`
- `canonical-x8-reciprocal-hoist-round-15-2026-08-30.md`
- `triangle-additive-ownership-round-18-2026-08-30.md`
- `state-free-polynomial-fourier-round-21-2026-08-30.md`
- `piecewise-quintic-fourier-bank-round-22-2026-08-30.md`
- `tiny-canonical-projection-tables-round-20-2026-08-30.md`
- `spectral-mip-real-render-round-3-2026-08-30.md`
- `va-compact-coefficient-mip-oracle-2026-08-30.md`
- `va-unclamped-coefficient-mip-boundary-2026-08-30.md`
- `va-narrow-coefficient-mip-crossover-2026-08-30.md`
- `va-narrow-bank-integration-boundary-2026-08-30.md`
- `va-single-horner-transition-2026-08-30.md`
- `va-canonical-coefficient-backend-boundary-2026-08-30.md`
- `va-canonical-coefficient-x8-probe-2026-08-30.md`
- `va-canonical-constant-x8-integration-boundary-2026-08-30.md`
- `va-constant-x8-sample-kernel-2026-08-30.md`

This is the most thoroughly explored alternative family. Very low harmonic
caps produce real steady x8 frontiers, especially triangle, but common-shape
CPU, harmonic/cap transitions, entry/exit ownership, publication lifetime, or
scalar paths prevent a universal backend. Modified FM does not revisit a
partial recurrence or compressed projection.

### BLIT, DPW, direct integration, and ADAA families

- `recursive-blit-canonical-round-8-2026-08-30.md`
- `bounded-blit-canonical-round-9-2026-08-30.md`
- `dpw-canonical-round-6-2026-08-30.md`
- `factored-dpw-canonical-second-shot-2026-08-30.md`
- `integrated-corrected-square-triangle-round-19-2026-08-30.md`
- `canonical-analytic-adaa-round-10-2026-08-30.md`
- `va-custom-exact-adaa-2026-08-30.md`
- `va-custom-forward-differences-2026-08-30.md`

BLIT failed numeric/CPU/transitions; factored DPW2 found a useful but
non-universal frontier; corrected-square integration found a narrow triangle
region but retained state and transition failures; canonical and custom ADAA
failed their quality/CPU contracts. Modified FM does use an integrator, but its
source is an analytic modified-Bessel pulse rather than a BLIT, corrected
shipping square, polynomial antiderivative, or differentiated polynomial.

### BLEP, BLAMP, event, residual, and selector families

- `local-blep-polynomial-round-11-2026-08-30.md`
- `support3-estrin-round-12-2026-08-30.md`
- `support3-coefficient-crossover-round-13-2026-08-30.md`
- `support-two-equiripple-blep-round-23-2026-08-30.md`
- `va-analytic-saw-event-iterator-2026-08-30.md`
- `va-avx512-masked-residual-2026-08-30.md`
- `va-avx512-pulse-selector-short-block-2026-08-30.md`
- `va-avx512-whole-pulse-block-2026-08-30.md`
- `va-branchless-residual-selection-2026-08-30.md`
- `va-custom-derivative-events-2026-08-30.md`
- `va-custom-elliptic-events-2026-08-30.md`
- `va-custom-windowed-sinc-events-2026-08-30.md`
- `va-density-adaptive-event-lanes-2026-08-30.md`
- `va-shared-slope-event-residual-second-shot-2026-08-30.md`
- `va-sparse-canonical-blep-schedule-2026-08-30.md`
- `va-sparse-minblep-ring-2026-08-30.md`
- `va-stateless-residual-crossover-2026-08-30.md`

The catalog covers longer and higher-order polynomials, Estrin scheduling,
coefficient blending, equiripple lookup, analytic and sparse event iteration,
windowed-sinc and minimum-phase rings, Elliptic BLEP, derivative events,
density-adaptive lanes, stateless crossover, and AVX-512 masks/selectors. The
shipping optimized cubic remains the universal event frontier. Modified FM has
no localized event correction at all.

### Curve compiler, representation, publication, and evaluator families

- `va-adaptive-c1-compiler-2026-08-30.md`
- `va-uniform-shared-slope-compiler-2026-08-30.md`
- `va-seeded-uniform-c1-selector-2026-08-30.md`
- `va-cheap-uniform-c1-selector-2026-08-30.md`
- `va-residual-weighted-c1-round-5-2026-08-30.md`
- `va-range-safe-bezier-round-6-2026-08-30.md`
- `va-fixed-256-byte-layouts-round-7-2026-08-30.md`
- `va-derivative-lambda-portfolio-round-9-2026-08-30.md`
- `va-curvature-event-regularization-round-10-2026-08-30.md`
- `va-higher-order-fixed-layouts-round-11-2026-08-30.md`
- `va-packed-i16-coefficients-round-12-2026-08-30.md`
- `va-wave-curve-pass-by-value-round-13-2026-08-30.md`
- `va-selected-coefficient-transitions-round-15-2026-08-30.md`
- `va-full-coefficient-interpolation-round-16-2026-08-30.md`
- `va-custom-time-packed-eval8-2026-08-30.md`

These reports close adaptive/shared C1 variants, selector portfolios,
range-safe fitting, fixed-size lower/higher polynomial layouts, coefficient
quantization/interpolation, pass-by-value concerns, and consecutive-time SIMD.
They produced the shipped compiler and portable eval4 wins but no new waveform
generation architecture. Modified FM uses no custom-curve representation and
is intentionally a canonical-saw first gate.

### ISA and code-generation reconnaissance

- `canonical-x8-assembly-recon-round-14-2026-08-30.md`

Generated code, AVX2/FMA, reciprocal hoisting, and the three AVX-512 residual
reports above already cover scheduling/ISA-only ideas. This round reuses the
existing sine polynomial and `fast_exp2`; it adds no intrinsics or assembly.

## Primary-source audit

- Timoney, Lazzarini, and Lysaght, [A Modified FM Synthesis Approach to
  Bandlimited Signal Generation](https://www.dafx.de/paper-archive/2008/papers/dafx08_05.pdf),
  DAFx-08. Equation 1 defines the pulse above; the paper derives its
  modified-Bessel spectrum, chooses `k` from an alias threshold, integrates it
  into saw/square/triangle, and reports favorable non-harmonic-energy and PEAQ
  comparisons against the BLIT, BLEP, and DPW implementations it tested. It
  explicitly leaves a fast exponential approximation as future work.
- Lazzarini et al., [New Perspectives on Distortion Synthesis for Virtual
  Analog Oscillators](https://mural.maynoothuniversity.ie/id/eprint/4104/1/VL_New_perspectives.pdf),
  surveys the same modified-FM oscillator family and records the bipolar-pulse
  extension.
- Gamma's current owner-maintained
  [`DSF`/`Buzz` oscillator source](https://github.com/LancePutnam/Gamma/blob/master/Gamma/Oscillator.h)
  was checked as the nearest open closed-form family. It implements finite
  geometric harmonic sums and closed-form equal-amplitude impulse sums, not
  the modified-Bessel pulse selected here.
- Faust's current owner-maintained
  [oscillator library](https://github.com/grame-cncm/faustlibraries/blob/master/oscillators.lib)
  was checked for an existing implementation to reuse. Its documented classic
  antialiasing families are DPW, BLEP, filter/waveguide, and table-based; no
  matching modified-FM primitive was found. This absence is only a source
  audit result, not proof that no implementation exists elsewhere.

No external code or coefficients are copied into KURV.

## Executed probe contract

The temporary ignored `modified_fm_saw_report` probe was isolated under
`cfg(test)` and removed after rejection.

- Quality: exact `f64` cosine/exponential; coherent periods 436, 55, and 7;
  ideal Fourier projection; aligned RMS/peak, total ideal error, complex wanted
  error, maximum wanted-amplitude deviation, raw DC, and first-to-last-cycle
  drift. Stationary curve error oracle-removes the integrator's arbitrary
  constant; DC and drift remain independent rejection gates.
- Search: the fixed modulation-index set `0.25..4096`; every point is reported,
  and CPU uses the best stationary index for its nearest coherent period.
- Runtime approximation: KURV's existing sine polynomial and `fast_exp2`, with
  no table or dependency; its stationary output is measured separately from
  the exact-math architecture result.
- Artifact gate: an abrupt 440 to 7,040 Hz pitch transition reports peak, DC,
  global step, and local transition step beside shipping 1x.
- CPU: paired alternating seven-run medians for real shipping scalar and
  detected x8 constant-saw paths at 440, 3,520, and 7,040 Hz, in both 24- and
  32-frame blocks.
- RT shape: fixed state only, O(1) work/sample, no allocation, lock, logging,
  I/O, or unbounded work in the candidate kernel. Reporting/allocation stays
  outside timing.

The modulation-index search and scaled-Bessel preparation are excluded from
timing. That is deliberately optimistic. A candidate that loses this gate is
rejected immediately; one that wins must next include bounded block preparation,
reset-state reconstruction, square/pulse/triangle, warp/morph eligibility, and
the complete transition matrix before production can be considered.

## Reproduction used

```text
CARGO_BUILD_JOBS=1 \
CARGO_PROFILE_RELEASE_DEBUG=1 \
RUSTFLAGS='-C target-cpu=native' \
  cargo test modified_fm_saw_report --lib --release --locked -- --ignored --nocapture
```

The cold release build completed in 8m01s. A corrected metric rerun rebuilt in
4m03s and passed its one ignored report in 2.58s. The correction only stopped
the integrator's arbitrary constant from being misclassified as stationary
curve error; it did not alter the candidate kernel. Bitwig and the Windows VM
were active, so CPU is coarse rejection evidence, not a near-frontier timing.

## Result

Rejected. Oracle centering gives the candidate every reasonable stationary
benefit, yet no modulation-index policy clears all fidelity and state gates:

| coherent frequency | shipping 1x RMS / peak / ideal error | best exact modified-FM | raw cycle drift | decision |
| --- | --- | --- | --- | --- |
| 110.09 Hz | 0.02210 / 0.30044 / -28.33 dB | `k=4096`: 0.03616 / 0.51716 / -24.05 dB | 0.000000011 | shipping wins curve fidelity |
| 872.73 Hz | 0.06210 / 0.28441 / -19.27 dB | `k=128`: 0.06882 / 0.33508 / -18.38 dB | 0.001135440 | shipping wins curve fidelity and drift |
| 6,857.14 Hz | 0.17363 / 0.29092 / -9.61 dB | `k=2`: 0.11751 / 0.21317 / -13.01 dB | 0.063451390 | curve win requires intolerable integrator drift |

At 6,857.14 Hz, `k=1` reduces drift to 0.002478015 but loses to shipping at
0.19724 RMS and 0.36562 peak. The practical runtime approximation also turns
the exact low-frequency `k=4096` drift of 1.1e-8 into 0.02538 per cycle. The
paper's required post-integrator DC blocker would add state, spectral droop,
transition behavior, and CPU; excluding it is favorable to the candidate.

After centering only from the settled low segment, the abrupt 440 to 7,040 Hz
transition has competitive peak and maximum step (0.96209 and 1.06930 versus
shipping's 0.96638 and 1.11967), but leaves +0.07050 DC versus shipping's
-0.00317. The pitch/index change therefore fails the state gate even before a
DC blocker is costed.

The optimistic sample kernel, excluding modulation-index search, scaled-Bessel
preparation, and the necessary DC blocker, is already 4.57-19.58x slower than
shipping scalar and 4.99-11.99x slower than the real detected x8 constant-saw
path across 24/32-frame blocks at 440, 3,520, and 7,040 Hz. These margins are
large enough for coarse rejection under the active DAW/VM load; no pristine
rerun is warranted.

Square, pulse, and triangle were not expanded. They inherit the rejected
exponential and integration costs, while adding more state and transitions.
No production integration or assembly follow-up is justified for this family.
