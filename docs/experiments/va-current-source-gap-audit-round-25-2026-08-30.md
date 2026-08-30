# VA current-source gap audit, round 25 (2026-08-30)

Status: research only. No production DSP, test harness, package version,
dependency, build, benchmark, commit, or remote changed in this round.

## Decision

The 79 existing experiment reports already cover the standard virtual-analog
oscillator families and most plausible execution variants. A primary-source
refresh through 2026 found one newly implemented architecture and two older but
still genuinely untested execution models worth bounded probes:

| Rank | Candidate | Ideal-BL fidelity chance | Rapid-transition chance | 24/32 scalar + x8 CPU chance | First gate |
|---:|---|---|---|---|---|
| 1 | pitch-derived recursive phase modulation (RPM) | medium | medium | high | saw only |
| 2 | finite multi-pole DSF coefficient-law fit | high if a 2-3-pole fit holds | medium-low | medium | saw only, at most three poles |
| 3 | fractional-delay feedback-loop impulse train | medium-high statically, but not exactly periodic | low | low-medium scalar, low x8 | saw only |

This ranking is by chance of clearing all three KURV gates, not by novelty or
static alias suppression alone. None is a production recommendation. RPM is the
only candidate justified for the next full harness experiment; the other two
should wait until that cheap gate reports.

## Local catalog boundary

Every Markdown report currently under `docs/experiments` was included in the
title/content audit. The catalog already closes or materially tests:

- exact additive, IDFT, Clenshaw, pruned recurrences, Fourier banks, tiny tables,
  spectral mips, cap crossovers, and transition handoffs;
- analytic and recursive BLIT, direct and factored DPW, PTR, BLEP, BLAMP,
  minBLEP, Gaussian and equiripple residuals, windowed-sinc and elliptic event
  filters, ADAA, Abel/Poisson summation, and modified-FM generation;
- coefficient compilers, interpolation and quantization, rational selectors,
  scalar/x4/x8 scheduling, AVX2/AVX-512, and assembly reconnaissance.

Keyword and formula searches found no existing RPM/feedback-FM, finite
multi-pole DSF coefficient fit, Tomisawa recurrence, or fractional-delay
feedback-loop experiment. The catalog does contain a source-level mention of
Gamma's single geometric DSF/Buzz family; that mention is not an executed
multi-pole coefficient-law probe.

## 2024-2026 primary-source refresh

### Current maintained oscillator implementations

The useful new evidence is in Faust's owner-maintained oscillator library:

- On 2026-01-13 it added
  [recursive phase-modulation saw and square oscillators](https://github.com/grame-cncm/faustlibraries/commit/e15ad0798c3387e53c4696e97a749ac47aa549a1).
  A 2026-01-15 correction made the saw and square recurrences consistent
  ([follow-up commit](https://github.com/grame-cncm/faustlibraries/commit/166b2dfb16bba35c0c22d7f59fe13bb075fa1729)).
  The [current pinned source](https://github.com/grame-cncm/faustlibraries/blob/46cdcd41e485f9d542d5ecfd6b0295d8f0267b3b/oscillators.lib#L1997-L2052)
  keeps two feedback samples, averages them as an anti-hunting filter, and
  evaluates one sine per output sample. The source traces the design to
  [Tomisawa's Yamaha feedback-FM patent](https://patents.google.com/patent/US4249447/en).
- On 2025-02-22 Faust added finite and infinite
  [direct-summation-formula oscillators](https://github.com/grame-cncm/faustlibraries/commit/296c1cd2ea451bd26034716c95c6c566c11cbe8d).
  The [current finite source](https://github.com/grame-cncm/faustlibraries/blob/46cdcd41e485f9d542d5ecfd6b0295d8f0267b3b/oscillators.lib#L1864-L1943)
  subtracts the geometric tail at a chosen harmonic count, so the generated
  spectrum can stop at Nyquist instead of relying on an infinite decaying tail.

The other active first-party implementations reinforce already tested families:

- VCV Fundamental's late-2025/early-2026 VCO work fixed event ordering, reverse
  phase and narrow-pulse crossings, pulse-width changes, and inserted minBLAMPs
  for triangle/saw slope changes. The relevant history is visible in the
  [current VCO file](https://github.com/VCVRack/Fundamental/commits/v2/src/VCO.cpp),
  including the [minBLAMP change](https://github.com/VCVRack/Fundamental/commit/81b0db8a37181b567234141d21b5ae398d3250c7)
  and the [event-ordering change](https://github.com/VCVRack/Fundamental/commit/eae28b72c040b9ece5fb85f634f6c826c79f7399).
  KURV's sparse minimum-phase ring and support-two BLAMP reports already test
  that architecture rather than merely its older name.
- Signalsmith's owner implementation last changed its runtime header in 2025 to
  fix state initialization
  ([pinned commit](https://github.com/Signalsmith-Audio/elliptic-blep/commit/77bf9866b705ddffe4870b40020411cf9192cf3b)).
  KURV's elliptic-event report already ports and validates that exact design
  boundary, including state, phase, latency, transitions, and CPU.
- Gamma's current
  [`DSF` and `Buzz` source](https://github.com/LancePutnam/Gamma/blob/master/Gamma/Oscillator.h)
  remains a closed-form geometric-harmonic or equal-amplitude impulse family.
  A single geometric term is not a new candidate below.

### Recent papers and author code that do not create a candidate

- The 2025 DAFx paper
  [Towards Neural Emulation of Voltage-Controlled Oscillators](https://www.dafx.de/paper-archive/2025/DAFx25_paper_33.pdf)
  uses sample-autoregressive RNN/GRU/LSTM/TCN models. Its best comparison model
  has about 26,000 parameters, and the paper explicitly says learned aliasing is
  governed by aliasing in the recordings. The
  [author repository](https://github.com/RiccardoVib/NeuralOSC/tree/643225e036e77f1f48a1ae2227e366f304c0c65b)
  defaults to 64 recurrent units and a 96-sample input. This is a physical-VCO
  black-box model, not an ideal-bandlimited curve generator, and has no plausible
  route to beating KURV's few-operation scalar/x8 kernel.
- The 2025 DAFx paper
  [Stable Limit Cycles as Tunable Signal Sources](https://www.dafx.de/paper-archive/2025/DAFx25_paper_68.pdf)
  is mathematically distinct, but its
  [author implementation](https://github.com/wolframw/stable-limit-cycles/tree/b08130ccf3d03e636c62c5e7d31978b394f8d02e)
  runs 24 ODE substeps, captures 12 oversampled values, and applies an eighth-
  order Butterworth filter for each output sample. It targets nonlinear timbres,
  not exact saw/square/pulse/triangle projections, so it fails the requested 1x
  execution boundary before a KURV probe.
- The August 2026
  [Arbitrary Polygon Oscillator](https://arxiv.org/abs/2608.24726)
  is current and has
  [author source](https://github.com/antonioargentieri1/Arbitrary_Polygon_Oscillator/tree/79c3d032e50b48f6355bdeb6897a2399777eaf25),
  but its antialiasing is a four-point polyBLAMP plus adaptive oversampling.
  General polygon geometry is new; the antialiasing architecture is not. KURV
  already tests the relevant BLAMP/PTR and oversampling boundaries.

## Ranked candidate 1: pitch-derived RPM

The current Faust saw recurrence is:

```text
y[n] = sin(theta[n] - beta * 0.5 * (y[n-1] + y[n-2]))
```

This is genuinely distinct from KURV's modified-FM round. Modified FM evaluated
an analytic `exp(k * (cos(theta) - 1)) * cos(theta)` pulse and then integrated
it. RPM instead closes a two-sample nonlinear phase-feedback loop; it has no
event residual, harmonic loop, projection table, integrator, or cap selector.

Why it deserves the next probe:

- one sine plus a few arithmetic operations per sample is a plausible x8 kernel;
- the two-point feedback average is fixed, tiny state rather than a long event
  or delay ring;
- `beta` continuously controls spectral extension, allowing a pitch-derived
  policy with no integer harmonic-cap switch;
- the existing KURV sine polynomial can test the architecture without a new
  dependency or copied Faust code.

Risks are first-class: feedback PM has an infinite nonlinear spectrum rather
than an exact cutoff, the two retained samples can produce reset/pitch seams,
large `beta` can enter unwanted limit-cycle behavior, and one sine per scalar
sample may still lose the shipping polynomial residual. The source calls the
result saw-like; it does not claim exact ideal-bandlimited coefficients.

The minimum experiment is one saw recurrence and a bounded offline search for
`beta(step)`. Gate exact and KURV-polynomial sine separately against all coherent
ideal-projection, DC/gain/peak, fractional-phase, reset, rapid pitch, and real
24/32 scalar+x8 CPU cells. Do not implement the squared-feedback square variant
unless saw clears every gate.

## Ranked candidate 2: finite multi-pole DSF coefficient-law fit

Faust's finite DSF proves a useful constant-cost primitive for one geometric
coefficient sequence. The genuinely new candidate is not that single sequence;
it is a small signed sum:

```text
target coefficient c[k] ~= sum(j=1..P, weight[j] * radius[j]^k), P <= 3
```

Each pole uses the exact finite DSF tail subtraction at the legal harmonic cap.
Two or three poles can therefore approximate the saw's `1/k` law (and a
separately fitted triangle `1/k^2` law) with work independent of harmonic count.
Square and arbitrary pulse can be derived from two shifted saw evaluations, as
in the existing Fourier reports.

This is not the rejected single-radius Abel/Poisson oscillator: that round kept
the exact `radius^k / k` law of one smoothed infinite Fourier series. It is also
not additive/IDFT, because runtime work does not visit each legal harmonic. A
single DSF pole, an infinite tail, or a per-cap table bank would be renamed old
work and must not be tested.

The chance is narrower than RPM. The representation can be exactly band-limited
after truncation, but coefficient-fit error lives in wanted harmonics; shifted
pulse doubles evaluation; cap changes still remove a harmonic; and recursive
quadrature states complicate rapid pitch/reset behavior. Stop if `P > 3`, if one
shared fit cannot cover the measured caps, or if saw needs cap-specific runtime
weights. Those outcomes collapse back into the already rejected coefficient
bank/additive families.

## Ranked candidate 3: fractional-delay feedback-loop impulse train

Nam, Valimaki, Abel, and Smith's
[DAFx-09 feedback-delay-loop oscillator](https://www.dafx.de/paper-archive/2009/papers/paper_72.pdf)
injects an impulse into a delay loop containing a fractional-delay allpass, then
derives classic waveforms with a leaky integrator. This is not KURV's evaluated
periodic-sinc quotient or harmonic recurrence: its phase/frequency mechanism is
a stateful waveguide-like delay loop.

The paper reports strong efficiency and alias suppression, so it remains a
legitimate static second shot. It also states the reasons for rank 3: the output
is not exactly periodic, time-varying pitch needs extra processing, and pitch
changes attenuate high frequencies. Period-sized storage and fractional-delay
reads are additionally hostile to eight decorrelated lanes and 24/32-sample
blocks.

Only a saw micro-gate is justified: fixed maximum delay storage, exact startup,
one settled pitch, one hard pitch jump, and actual scalar/x8 memory traffic. Do
not proceed to pulse/triangle if exact periodic alignment, transition, or x8 CPU
already loses.

## Renamed or recombined ideas explicitly rejected

- Nam et al.'s
  [low-order fractional-delay BLIT](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf)
  remains BLIT with Lagrange/B-spline/allpass interpolation. KURV's BLIT and
  residual-filter rounds cover the mathematical family; changing the
  interpolator is not a new architecture.
- Pekonen and Holters'
  [nonlinear-phase basis functions](https://www.dafx.de/paper-archive/2012/papers/dafx12_submission_15.pdf)
  are parallel first/second-order IIR event filters with fractional event
  excitation. That maps directly to the already executed elliptic IIR-event
  experiment, not a new gap.
- Faust's 2026
  [`twin_osc`](https://github.com/grame-cncm/faustlibraries/blob/46cdcd41e485f9d542d5ecfd6b0295d8f0267b3b/oscillators.lib#L1945-L1995)
  applies a time-varying comb to an existing saw. It may create PWM/morph/detune,
  but it cannot replace the underlying antialiased saw generator.
- A one-pole finite DSF is a geometric-sum restatement; an infinite DSF openly
  retains an aliasing tail. Only the bounded multi-pole coefficient fit above is
  distinct enough to test.
- Polygon polyBLAMP, longer/minimum-phase/equiripple BLEPs, alternative
  polynomial schedules, neural emulation, ODE limit cycles, and hand-written
  assembly do not reopen their failed family gates without new measured
  evidence.

## Recommended handoff

Run RPM saw first. It is the smallest, newest, and most SIMD-plausible gap. If
it fails ideal projection, transitions, or scalar CPU, record the rejection and
move to the at-most-three-pole finite DSF saw. Keep the feedback-delay loop as a
final low-priority second shot; its own primary paper already predicts the
transition trade that KURV treats as a hard gate.
