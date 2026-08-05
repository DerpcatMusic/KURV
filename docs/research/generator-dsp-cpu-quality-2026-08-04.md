# KURV generator DSP: lower CPU, lower aliasing, and truer 1x-4x output

Research report — 2026-08-04

## Scope and verdict

This report targets the live dirty working tree, not the initial commit and not the older oscillator
described in the existing research notes. The current engine already has the right basic topology:

- Legacy two-point quadratic PolyBLEP for saw and pulse, with an uncorrected analytic triangle;
- four-point integrated cubic B-spline PolyBLEP/PolyBLAMP;
- four-point integrated cubic Lagrange PolyBLEP/PolyBLAMP;
- independent 1x, 2x, 3x, and 4x synthesis followed by one stereo decimator;
- eight-wide and four-wide oscillator SIMD, then a scalar tail.

The best experiment is therefore a deepening of this core, not a rewrite. The highest-value candidates
are:

| Rank | Candidate | Expected quality effect | Expected CPU effect | Risk |
|---:|---|---|---|---|
| 1 | Collapse the scalar `f64` path to the existing `f32` SIMD math and fuse repeated edge work | Same phase truth, more consistent scalar/SIMD output | Large win at 1-3 unison voices and every scalar tail; moderate win for pulse, triangle, and Saw-Pulse morphs | Low |
| 2 | Replace the standard Spline kernel with an optimized four-segment cubic spline, retaining the current Spline as the A/B baseline | Largest same-support alias-rejection lead found; comparable support and latency | Similar polynomial order; potentially a few more FMAs because the optimized coefficients are denser | Medium |
| 3 | Redesign 2x/4x as linear-phase half-band stages and 3x as a third-band FIR | Same linear-phase contract if the folded transition band and attenuation are matched explicitly | Large decimator win from structural zero coefficients; 4x should benefit most from two stages | Medium |
| 4 | Define and fit the combined oscillator-plus-decimator response, including 1x | Truer wanted-harmonic level and consistent timbre across AA/factor changes | One tiny host-rate EQ per stereo channel; can replace the current unexplained fixed EQ | Low-medium |
| 5 | Interpolate shape and pulse-width trajectories across internal samples, while hoisting host-rate invariants out of the inner loop | Cleaner PWM/morph automation and more truthful high-rate synthesis | Small interpolation cost, with a possible net win if voice setup/envelope targets are computed once per host sample | Medium |
| 6 | EPTR/DPW4 as a saw/triangle comparator, not as the universal core | Known strong alias suppression for classical waves | Potentially very low scalar cost; uncertain under KURV's random-phase SIMD banks | Medium-high |
| 7 | Cascaded allpass half-band IIR as a non-default CPU ceiling | Strong magnitude rejection at extremely low cost | Lowest filter arithmetic and latency | High for KURV because nonlinear phase changes the waveform contract |

The lazy result is: keep all three visible algorithms and all four factors during the experiment. First
remove work that provably computes the same result twice, then challenge only the Spline coefficients
and decimator topology. Do not add a wavetable, correction queue, FFT, additive bank, or dependency.

## Live implementation and its actual math

### The phase and execution paths

`VaOscillator` stores one `f32` phase ([`src/oscillator.rs`](../../src/oscillator.rs#L34-L37)). The scalar
tail converts that already-quantized phase and step to `f64`, evaluates the scalar waveform, and casts
the answer back to `f32` ([`src/oscillator.rs`](../../src/oscillator.rs#L62-L82)). The conversion cannot
recover phase bits that were not stored. The comments claiming that an `f64` phase is retained on the
SIMD entry points are therefore stale ([`src/oscillator.rs`](../../src/oscillator.rs#L94-L105)).

The four- and eight-wide sine paths use the same folded polynomial and FMAs
([`src/oscillator.rs`](../../src/oscillator.rs#L418-L443)), while the scalar sine calls `f64::sin`
([`src/oscillator.rs`](../../src/oscillator.rs#L370-L385)). A local dense phase sweep of the checked-in
SIMD polynomial found about `2.1e-8` maximum absolute error against `sin(2*pi*phase)`, already below
one `f32` ULP around unity. That is not a reason to reduce the phase accumulator's precision; it is
evidence that double-precision transcendental work after an `f32` accumulator buys little output truth.

This matters in the normal path, not only an edge case. `Voice::render` processes groups of eight,
then four, then sends every remaining oscillator through `generate_shape_step`
([`src/voice.rs`](../../src/voice.rs#L856-L995)). Unison counts 1-3 are entirely scalar, and every count
not divisible by four has a scalar tail.

### The three antialiasing modes

Let `d` be the phase step and `p` the wrapped phase.

- **Legacy 2PT.** Saw subtracts a quadratic correction with one-sample support on either side of the
  wrap; pulse adds it at the rising edge and subtracts it at the duty-cycle edge
  ([`src/oscillator.rs`](../../src/oscillator.rs#L508-L565),
  [`poly_blep`](../../src/oscillator.rs#L741-L756)). Triangle is the raw piecewise-linear waveform
  ([`src/oscillator.rs`](../../src/oscillator.rs#L446-L461)). This is the cheapest and weakest baseline.
- **Spline 4PT.** Saw and pulse use an integrated cubic B-spline residual with support `|p/d| < 2`;
  triangle integrates that residual once more as a PolyBLAMP at its two slope discontinuities
  ([`src/oscillator.rs`](../../src/oscillator.rs#L684-L739)). The corresponding SIMD functions gate the
  expensive polynomials with an `event.any()` check
  ([`src/oscillator.rs`](../../src/oscillator.rs#L832-L892),
  [`src/oscillator.rs`](../../src/oscillator.rs#L969-L1029)).
- **Lagrange 4PT.** Saw/pulse use the integrated third-order Lagrange residual and triangle uses its
  next integral ([`src/oscillator.rs`](../../src/oscillator.rs#L619-L682)). Its SIMD implementation has
  the same two-phase support and vector-level event gate
  ([`src/oscillator.rs`](../../src/oscillator.rs#L758-L830),
  [`src/oscillator.rs`](../../src/oscillator.rs#L895-L966)).

The primary comparison behind these two four-point modes is Välimäki, Pekonen, and Nam's integrated
polynomial study. Under its 44.1 kHz, 96 dB SPL masking model, the reported highest perceptually
alias-free saw fundamentals were 5.134 kHz for four-point Lagrange and 7.845 kHz for four-point
B-spline. B-spline rejected aliases better, while Lagrange retained more upper-band amplitude; the
paper supplied stronger post-equalization for B-spline for that reason.
([author-hosted JASA paper](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf))

Those figures are not universal audibility limits. They are useful evidence that KURV's two modes are
genuinely different Pareto points, not a Low/High ordering that should be collapsed without measuring
wanted-harmonic error as well as aliases.

### Morph work is duplicated

For any non-endpoint shape, `generate_shape4/8` evaluates both complete endpoint waveforms and blends
their samples ([`src/oscillator.rs`](../../src/oscillator.rs#L106-L129),
[`src/oscillator.rs`](../../src/oscillator.rs#L183-L206)). That preserves the intended sample-domain
morph, but it repeats shared discontinuity work in the Saw-Pulse segment.

If `C0` is the selected BLEP at phase zero, `Cw` is the duty-edge BLEP, `r` is the raw saw, `q` is the
raw pulse, and `b` is the morph amount, the current calculation is

```text
saw   = r - C0
pulse = q + C0 - Cw
mix   = (1-b)*saw + b*pulse
      = (1-b)*r + b*q + (2*b-1)*C0 - b*Cw
```

The last form evaluates `C0` once instead of twice and is algebraically the same morph. Likewise,
pulse's two BLEPs and triangle's two BLAMPs share one `1/d`, support value, and correction family, but
the current code calls two complete correction functions
([`src/oscillator.rs`](../../src/oscillator.rs#L535-L617),
[`src/oscillator.rs`](../../src/oscillator.rs#L446-L505)).

### Oversampling currently repeats the whole synth

The process callback constructs one `VoiceSettings` value at the host sample, then calls the complete
synth `factor` times with that exact shape and pulse width before taking one decimator output
([`src/lib.rs`](../../src/lib.rs#L947-L973)). `Voice::render` advances envelope and swarm and repeats
shape/gain/setup arithmetic on each call ([`src/voice.rs`](../../src/voice.rs#L816-L853)). Thus 2x-4x
raises oscillator, envelope, unison, and modulation execution rates, but shape and pulse-width control
values are zero-order-held across each group of internal samples.

Oversampling a source does not require an input-audio interpolator, but any host-rate control that moves
an edge is still a sampled trajectory. Fractional edge timing must follow that moving trajectory;
variable-fractional-delay oscillator work treats event placement as part of the antialiasing algorithm,
not a display detail. ([Pekonen et al., ICGCS 2010](https://mac.kaist.ac.kr/pubs/Pekonen2010ICGCS.pdf))

### The decimator contract is narrower than the product prose

`StereoDecimator` performs one full dot product only when a host output is requested; it does not
compute and discard the other high-rate FIR outputs ([`src/oversampling.rs`](../../src/oversampling.rs#L192-L260)).
Its lengths are 97, 145, and 193 taps, exactly `48*factor + 1`
([`src/oversampling.rs`](../../src/oversampling.rs#L264-L269)). This gives every FIR 24 host samples of
group delay; a one-sample linear-phase EQ and eight-sample delay bring every mode to 33 samples.

The coefficient comments specify a 20.5 kHz passband and 24 kHz stopband, but the coefficients are
fixed. Those absolute edges are true only at a 48 kHz host rate; at 44.1 kHz they scale to about
18.83/22.05 kHz and at 96 kHz to 41/48 kHz. The existing response check also evaluates only a 48 kHz
host-rate case ([`src/oversampling.rs`](../../src/oversampling.rs#L352-L387)). The honest invariant is
currently a normalized edge: passband to about `0.4271*host_rate` and stopband from
`0.5*host_rate`.

The FIR-only response contract is under 0.052 dB passband error and below -84 dB stopband. After that
FIR, factors 2-4 apply the fixed host-rate equalizer

```text
H(z) = -0.01788 + 1.03576 z^-1 - 0.01788 z^-2
```

([`src/oversampling.rs`](../../src/oversampling.rs#L12-L14),
[`src/oversampling.rs`](../../src/oversampling.rs#L240-L259)). At 48 kHz it is unity at DC but boosts
15 kHz by about 0.42 dB, 20.5 kHz by 0.57 dB, and Nyquist by 0.60 dB. Factor 1 uses only the direct
delay and bypasses that EQ ([`src/oversampling.rs`](../../src/oversampling.rs#L79-L93)). The current
checks therefore do not state or verify the combined response heard by the user, and 1x is not the
same magnitude reference as 2x-4x.

## Ranked candidate techniques

### 1. Make the scalar and SIMD paths one `f32` algorithm

Use the existing folded sine polynomial and `f32` BLEP/BLAMP algebra for `generate_shape_step` instead
of converting the `f32` state to `f64`. This removes scalar libm sine, double polynomial arithmetic,
and scalar/SIMD timbre differences. It leaves the phase storage and final sample format unchanged.

This candidate should be judged separately at unison counts 1, 2, 3, 5, 7, and 9. A 64-oscillator
benchmark alone can hide the largest win because most of its work already travels through eight-wide
SIMD. If phase-accumulator precision later proves audible, the honest experiment is a separate `f64`
or fixed-point accumulator; converting an `f32` phase after the fact is not that experiment.

Then fuse only the repeated concrete work:

- one wrap-edge BLEP for Saw-Pulse morphing;
- one reciprocal/support calculation for the two pulse edges;
- one reciprocal/support calculation for the two triangle corners;
- explicit SIMD `mul_add` Horner chains where the scalar path already uses them.

For the B-spline arm, the same residuals also have compact truncated-power forms:

```text
r_blep(x)  = sign(x) * ((1-|x|)_+^4 / 6 - (2-|x|)_+^4 / 24)
r_blamp(x) =             (2-|x|)_+^5 / 120 - (1-|x|)_+^5 / 30
```

where `a_+ = max(a, 0)`. These identities expose shared squares/fourth powers and remove the explicit
inner/outer polynomial pair. Cardinal B-splines have exactly this compact finite-support,
piecewise-polynomial structure. ([Unser, “Splines: A Perfect Fit for Signal and Image Processing,”
IEEE SPM 1999](https://doi.org/10.1109/79.799930)) Whether this form beats the current masked Horner
form is target-CPU dependent, so it earns a microbenchmark, not an assumption.

### 2. Optimize the four-point spline instead of increasing its support

Pekonen, Nam, Smith, and Välimäki optimized a symmetric four-segment third-order spline under a
perceptually weighted alias objective and a constrained baseband response. With their stated 44.1 kHz
design and post-equalizer, integrating the optimized BLIT basis into a BLEP extended the reported
perceptually alias-free saw fundamental from 7.845 kHz for the standard four-point B-spline to
12.259 kHz. The support and polynomial order stayed four segments/cubic.
([author-hosted IEEE SPL paper](https://mac.kaist.ac.kr/pubs/PekonenNamSmithValimaki-spletter2012.pdf))

That is the strongest quality-per-support result found for KURV's exact table-free oscillator class. It
should be tested as a replacement kernel behind the existing **Spline 4PT** label, with current Spline
retained internally as the reference. It is not a coefficient paste:

- integrate the optimized impulse basis once for BLEP and twice for BLAMP;
- preserve unit step area, odd BLEP symmetry, even BLAMP symmetry, and continuity at segment joins;
- scale by the actual value or derivative jump exactly as the current saw/pulse/triangle paths do;
- fit KURV's response objective and sample rates, rather than silently adopting the paper's 96 dB SPL
  perceptual model and pole-at-`-0.9` equalizer;
- compare wanted harmonics as well as aliases, because the optimized objective permits controlled
  passband deviation.

The optimized coefficients are denser than the rational cardinal B-spline coefficients, so “same
support” does not guarantee identical CPU. It should still be close to the current Lagrange polynomial
cost and far below another complete 2x synth render.

### 3. Use structural-zero decimators, not a generic “polyphase rewrite”

Classical polyphase decomposition avoids calculating FIR outputs that a decimator will throw away.
KURV already requests one dot product only after `factor` pushes, so a conventional polyphase rewrite
does not by itself cut the 97/145/193 multiplications per retained output. The original multirate
literature also distinguishes this output-rate saving from multistage filter design.
([Crochiere and Rabiner, Proceedings of the IEEE 1981, author-hosted PDF](https://web.ece.ucsb.edu/Faculty/Rabiner/ece259/Reprints/179_interpolation_decimation.pdf))

The real opening is the filter coefficients:

- A 97-tap half-band FIR has only 49 nonzero taps including its center; symmetry leaves 25 unique
  multipliers. The Vaidyanathan-Nguyen construction designs the equiripple half-band prototype while
  preserving those structural zeros. ([Caltech author record and paper](https://authors.library.caltech.edu/records/czbzh-df707))
- 2x can use one such linear-phase stage.
- 4x can use two 2:1 stages. The first 192-to-96 kHz stage can have a much wider transition because the
  second stage removes everything that could fold into the declared host passband. Multistage
  decimators reduce work by placing the sharp filter at the lower rate; this is a primary result of
  Crochiere and Rabiner's multistage treatment
  ([1975 paper, author-hosted PDF](https://web.ece.ucsb.edu/Faculty/Rabiner/ece259/Reprints/087_optimum%20fir%20digital%20filters.pdf)).
- 3x can use a third-band/Nth-band FIR. Nth-band filters trade a small response penalty against every
  Nth coefficient being zero except the center and were developed specifically for N:1
  decimation/interpolation. ([Mintzer, IEEE TASSP 1982, IBM primary record](https://research.ibm.com/publications/on-half-band-third-band-and-nth-band-fir-filters-and-their-design))

For KURV's current normalized contract, a half-/third-band transition can be symmetric around the new
Nyquist: pass to `0.4271*Fs_host`, stop from `0.5729*Fs_host`. Frequencies in the transition above old
Nyquist fold only into the output's upper transition band, not below the declared passband. This is not
identical to the current stop-from-`0.5*Fs_host` contract, so both definitions must be rendered and
compared; do not hide the relaxation behind the same quality label.

Keep the FIR version linear phase and fill the remaining delay to 33 host samples. A naive symmetry
fold was already recorded as slower in the existing KURV research because of reverse access and extra
state ([`oversampling-alternatives.md`](oversampling-alternatives.md#5-making-the-remaining-oversampling-cheaper)).
Do not rerun that exact layout. Structural zeros plus a ring layout that makes paired samples contiguous
is a materially different experiment.

### 4. Fit the response of the whole 12-mode matrix

The current fixed EQ should not remain an unexplained bonus attached only to factors 2-4. Determine
whether it is compensating the oscillator kernel, the FIR, or both, then fit the entire audible chain for
each AA/factor pair.

The integrated-polynomial paper demonstrates that a three-tap linear-phase FIR can restore upper
harmonics: in its setup, optimized coefficients brought all harmonics below 15 kHz within 1 dB of the
desired level. It gives different coefficients for four-point Lagrange and four-point B-spline because
their droop is different.
([Välimäki, Pekonen, and Nam 2012](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf))

For KURV, one host-rate stereo EQ after decimation is preferable to an EQ per oscillator. Linear
filtering commutes with summation, so the cost is independent of polyphony. Candidate coefficient sets
may depend on AA mode and factor; mode changes must transition coefficients or reset/crossfade state
without a click. The acceptance response must include the FIR, its EQ, the oscillator kernel, and the
33-sample alignment—not the FIR coefficients alone.

### 5. Give oversampled controls an internal trajectory and hoist invariants

Shape and pulse width are already smoothed host-rate controls, so the smallest truthful internal model
is a linear ramp from one host value to the next across `factor` renders. This reduces the zero-order-hold
staircase that currently drives morph and PWM at 2x-4x. For PWM, the falling edge is moving; evaluating
its location at each internal time point is a closer approximation than repeating one width for the
whole host interval. Bandlimited interpolation remains the ideal reference, but finite interpolation
always trades response, latency, and work.
([Smith and Gossett, ICASSP 1984](https://doi.org/10.1109/ICASSP.1984.1172555))

Do not turn this into a general control-rate subsystem. The current two smoothed edge controls are the
scope. Compute host-interval endpoint values once, pass a tiny per-substep delta, and hoist any
unchanged voice setup that profiling shows is repeated inside `Voice::render`. MIDI events remain at
their host-provided sample offsets; the plugin should not invent sub-sample event timestamps.

### 6. Keep EPTR/DPW as a comparator, not the destination

Fourth-order DPW was reported perceptually alias-free for saw fundamentals to about 4.6 kHz at 44.1
kHz in the authors' evaluation, versus about 600 Hz for second-order DPW. They recommend a hybrid near
500 Hz to avoid large low-frequency scaling.
([Välimäki et al., IEEE TASLP 2010](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf))

PTR observes that DPW differs from the trivial waveform only around discontinuities and evaluates a
polynomial transition there. The authors measured 27% CPU for PTR versus 36% for DPW in their
20-bank, 88-voice experiment and reported state-free frequency changes. ([Kleimola and Välimäki,
IEEE SPL 2012](https://aaltodoc.aalto.fi/bitstreams/0b7c5649-cd2b-476f-ba53-4e33a8103f06/download))
EPTR then removed the offset addition, reported roughly 30% fewer operations than PTR, and generated
the same output in its scalar formulation. ([Ambrits and Bank, SMC 2013, author PDF](https://home.mit.bme.hu/~bank/publist/smc13.pdf))

These are legitimate bleeding-edge-for-their-class CPU references, but KURV already has random-phase
four-/eight-lane event masks and a continuous Sine-Triangle-Saw-Pulse morph. Scalar operation counts
do not predict that SIMD workload. An EPTR arm earns its place only for endpoint saw/triangle and only
if it beats the fused current kernel; it should not expand the public mode count.

### 7. Treat half-band IIR as an explicit phase trade

The 2025 Carson, Välimäki, Wright, and Bilbao study compared recent integer-oversampling filters. Its
cascaded half-band IIR used two allpass branches and 18 operations per stage; under the tested nonlinear
RNN effects, the half-band IIR and FIR cascades approached FFT-resampling quality at 2x, 4x, and 8x,
while the IIR used fewer operations and zero filter latency. It also introduced nonlinear phase, and the
paper did not include a perceptual listening test.
([IEEE/arXiv primary paper](https://arxiv.org/abs/2501.18470),
[authors' source repository](https://github.com/a-carson/resampling_neural_afx))

That makes it a useful CPU floor, not the default KURV replacement. KURV is a generator whose waveform
phase and transient shape are part of the output, whereas the study's target was nonlinear effect-model
resampling. The linear-phase half-band FIR must be the first challenger. Promote IIR only if magnitude,
group delay, waveform, stereo phase, and listening evidence all justify changing the sound contract.

## Measured implementation result: optimized 2x Spline

The optimized four-segment Spline was integrated behind the existing **Spline 4PT** selection only
when the internal rate is 2x. Its BLEP and BLAMP residuals use scalar, four-lane, and eight-lane
Horner/FMA implementations. A fitted unity-DC five-tap symmetric post-filter
`[0.01770059, -0.09979711, 1.16419304, -0.09979711, 0.01770059]` replaces the old three-tap
correction for that one mode. Both filters are represented at the same group delay, so the reported
latency remains exactly 33 host samples. Coefficients ramp over 128 host samples when the mode changes.

The five-frequency frontier sweep was:

| 2x path | Mean alias residual | Worst alias residual | Wanted magnitude RMS to 20 kHz | Peak |
|---|---:|---:|---:|---:|
| Previous Spline | -81.864 dBc | -73.250 dBc | 0.814 dB | 1.243 |
| Optimized Spline plus five-tap fit | **-90.231 dBc** | **-80.791 dBc** | **0.086 dB** | 1.271 |
| Lagrange | -67.522 dBc | -59.189 dBc | 0.173 dB | 1.300 |

This is an accuracy result, not a brightness score. For example, the coherent 440.186 Hz saw moved
from -85.342 to -93.028 dBc alias residual, versus -70.879 dBc for Lagrange. At 3.99976 kHz, pulse
alias residual moved from -74.264 to -81.959 dBc while wanted-harmonic RMS error fell from 1.152 to
0.115 dB; Lagrange measured -60.057 dBc and 0.146 dB respectively.

Release x86-64-v3 hardware-counter runs across one, eight, and 64 unison lanes put the complete path
within roughly -4% to +3% cycles of the previous Spline depending on shape and packing. Retired
instructions increased about 1-3% because the fixed-latency post-filter retains four samples instead of
two. The attempted one-write circular history was byte-identical but executed more instructions and was
rejected. All Legacy and Lagrange renders at 1x-4x, plus Spline at 1x, 3x, and 4x, remained byte-for-byte
identical to the frozen baseline.

The optimized Spline is now the hidden default for new plugin state at the normal 2x factor. This is not
a quality-at-any-cost promotion: in a matched 48 kHz, 2x, x86-64-v3 production-path harness, the live
Spline also beat the untouched Lagrange engine that had been the best subjective starting point. Pinned
CPU 7, Swarm off, 100,000 frames, and five-run medians at 64 unison x 32 notes measured:

| Shape | Untouched Lagrange ns/frame | Live optimized Spline ns/frame | Reduction |
|---|---:|---:|---:|
| Triangle | 4,781.702 | 4,495.936 | 6.0% |
| Saw | 2,822.697 | 2,771.805 | 1.8% |
| Saw-Pulse midpoint | 6,705.250 | 5,218.950 | 22.2% |
| Pulse | 4,979.609 | 4,623.550 | 7.1% |

This comparison is deliberately against the old Lagrange quality path, not the cheaper and much dirtier
Legacy 2PT path. The live Spline is not cheaper than Legacy at every maximum-polyphony cell; claiming
otherwise would hide the actual quality/CPU trade. At 8 unison x 8 notes it was 6.9-18.3% faster than the
old Lagrange path across the same four shapes.

## Measured exact CPU refinements

The production engine also retained a set of smaller changes only where output stayed exact (or, for
explicit FMA contraction, within one final `f32` ULP while improving the scalar-oracle error):

- stop scanning the 32-slot voice array once the known active count has been rendered; this cut the
  one-note traversal by roughly 52% and the four-note case by 13.8%;
- use the ordinary multiply for the phase-step expression whose addend is exactly zero, avoiding a
  counterproductive scalar FMA and saving about 7-23% in the affected micro-path;
- compute the static Swarm lane layout once per target instead of repeating it for pitch and pan;
- evaluate the shared wrap-edge correction once in the Saw-Pulse SIMD morph, saving roughly 7-15%
  in common four-/eight-lane cells;
- express the Lagrange SIMD residuals as explicit fused Horner chains, slightly reducing CPU while
  improving RMS agreement with the `f64` oracle by about 20-25%;
- cache `swarm_rate / sample_rate` when either control changes, removing one `vdivss` from every
  internal render. Across the final 63-cell endpoint/morph matrix this was checksum-exact and reduced
  retired instructions in every cell.

The final cached-step comparison against its immediately preceding x86-64-v3 build was:

| Swarm-off Spline2 path | Previous ns/frame | Live ns/frame | Reduction | Checksum |
|---|---:|---:|---:|---|
| Saw, 1 unison x 1 note | 47.437 | 45.731 | 3.6% | exact |
| Saw, 8 x 8 | 302.064 | 297.438 | 1.5% | exact |
| Saw, 64 x 32 | 5,645.626 | 5,592.174 | 0.9% | exact |
| Saw-Pulse midpoint, 1 x 1 | 58.579 | 55.153 | 5.8% | exact |
| Saw-Pulse midpoint, 8 x 8 | 430.873 | 420.257 | 2.5% | exact |
| Saw-Pulse midpoint, 64 x 32 | 10,002.163 | 9,614.991 | 3.9% | exact |

Swarm's target generation was then treated as a separate control-rate problem rather than weakening
the oscillator. Against the frozen pre-adaptive x86-64-v3 engine, final default Wander at Rate 0.7
measured 51.856 to 47.278 ns/frame at 1 x 1 (8.8%), 684.369 to 461.377 at 8 x 8 (32.6%), and
13,930.836 to 6,975.524 at 64 x 32 (49.9%). Hardware counters on the dense case fell from
4.377 to 2.167 billion cycles and 11.568 to 5.265 billion retired instructions. Rate 32 remains
checksum-exact. The cadence frontier and the second stateless Jitter family are documented in
[`swarm-cadence-jitter-2026-08-04.md`](swarm-cadence-jitter-2026-08-04.md).

### Converged 16-note x 64-lane result

The final production comparison alternated the frozen pre-fusion x86-64-v3 binary with the live
binary on pinned CPU 7. Each cell is the median of three nine-repeat runs over 4,096 host frames.
The fused Spline2 path was bit-identical in a separate 105-case sweep covering seven shape positions,
1/4/8/63/64 unison lanes, and Swarm off/Wander/Jitter.

| Shape | Swarm off | Wander | Jitter |
|---|---:|---:|---:|
| Triangle | 19.2% faster | 25.5% faster | 25.2% faster |
| Saw | 22.4% faster | 28.3% faster | 29.3% faster |
| Pulse | 20.9% faster | 22.3% faster | 26.8% faster |
| Triangle-Saw midpoint | 20.9% faster | 21.6% faster | 24.0% faster |
| Saw-Pulse midpoint | 24.9% faster | 26.1% faster | 27.6% faster |

The complete final CPU grid contains 195 cells: the same five shapes across Legacy, Spline, and
Lagrange at 1x-4x plus Spectral 1x, each with Swarm off, Wander, and Jitter. Static coherent renders
confirm the real trade: at 522.95/4,185.79 Hz, Spectral saw residue measured -97.65/-125.21 dBc and
its harmonic-envelope RMS error rounded to 0.00 dB, versus Spline2 at -92.26/-80.78 dBc and
8.27/0.52 dB. At 65.92 Hz the 128-row Spectral hybrid falls back and reaches only -56.59 dBc,
where Spline2 reaches -101.37 dBc. Spline2 therefore remains the honest universal default;
Spectral 1x is an explicit mid/high precision option, not a disguised replacement.

### Final bundle and host-boundary validation

The exact final source was rebuilt as x86-64-v3 CLAP and VST3 and atomically published through the
stable installed-bundle pointer. The installed hashes match the validated build outputs.
`clap-validator` ran 21 tests: 20 passed, none failed, one conversion test was
inapplicable, and there were no warnings. `pluginval` strictness 5 passed the VST3 at 44.1, 48, and
96 kHz with 64, 128, 256, 512, and 1,024-frame blocks, including audio, automation, state, and bus
checks. Both normal and `rt-paranoid` library configurations passed the existing eight tests.
The paranoid VST3 wrapper completed pluginval without a report. The paranoid CLAP wrapper reported
two allocations with `clap_validator`'s own process wrapper as the first visible frame while still
passing all tests; because the same KURV logic is clean through VST3, this is retained as a Truce/host
wrapper diagnostic rather than attributed to the oscillator.

Raw host-ABI probes queried the built binaries rather than trusting source forwarding: CLAP returned
33 latency samples before and after activation, and VST3 `IAudioProcessor::getLatencySamples` returned
33 directly. `pluginval` nevertheless printed `Reported latency: 0` in its pre-processing information
stage while still passing. This isolates the disagreement to that JUCE-host observation/cache layer;
the bundle's two raw format contracts agree. A restarted DAW should still be used to confirm that its
PDC display and timing consume the 33-sample report before shipping.

### 2026-08-05 Jitter phase-step SIMD follow-up

A later isolated experiment targeted the remaining Jitter-only steady-state overhead. Jitter has
static pan/gain and only slews pitch between deterministic control-rate random targets, so its target
ratio is now converted to an absolute cached phase step once per control update. The fused renderer
advances those steps directly with explicit eight- and four-lane SIMD; Wander retains its ratio and
pan ramps unchanged. This adds no arrays, allocation, locks, or audio-rate random generation.

On a quiet physical core, x86-64-v3, saw, 16 notes x 64 lanes, SplineOptimized, 262,144 frames x nine
repeats, the isolated candidate measured:

| Internal rate | Frozen Jitter ns/frame | Candidate ns/frame | Change |
|---|---:|---:|---:|
| 1x | 1,288.700 | 1,239.735 | 3.80% faster |
| 2x fused | 1,545.237 | 1,393.119 | 9.84% faster |
| 3x | 3,302.405 | 3,199.210 | 3.13% faster |
| 4x | 4,384.623 | 4,065.106 | 7.29% faster |

At 2x, Swarm off moved by +0.34% and Wander by +0.09%, both effectively flat and checksum-exact.
After manual integration with the new performance/MPE controls, a five-round interleaved rerun on the
same binary family measured the frozen median at 2,044.956 ns/frame and live at 1,866.491 ns/frame,
an 8.73% reduction; the live checksum matched the isolated candidate. The lower percentage reflects
normal clock/load variation, so 8.7-9.8% is the supported dense-default Jitter range.

Fused-versus-sequential comparisons remained exactly zero-error over 1,048,576 frames for Swarm off,
Wander, and Jitter. Repeated long Jitter renders were byte-deterministic. Against the previous Jitter
sound, RMS changed by -0.0098 dB, 18-22 kHz energy changed from -29.553 to -29.586 dBc, 22-24 kHz
from -49.552 to -49.519 dBc, and all samples remained finite. The candidate was retained because it
removes work from the default 2x dense path without a material spectrum or alias change.

The separate public-Vital source audit is in
[`vital-unison-simd-2026-08-05.md`](vital-unison-simd-2026-08-05.md). KURV already uses wider AVX/FMA
vectors than public Vital's SSE2 implementation. The remaining transferable idea is block-major
packed state across an event-free sample segment, not an SSE switch or Vital's GPL wavetable/IFFT
antialiasing core.

### 2026-08-05 production block-major fused renderer

That transferable idea was then implemented without copying Vital code. The production path fuses
phase advance, band-limited saw evaluation, and stereo accumulation across AVX eight-lane blocks,
so it no longer creates a temporary sample vector for every oscillator lane. It is selected only for
event-free, fully held, non-gliding saw segments with static smoothed controls and a supported
multiple-of-eight unison count. MIDI and parameter events split the segment exactly; release tails,
glide, morph/PWM, Spectral mode, smoothing, and unsupported lane counts retain the existing exact
sample path.

Pinned x86-64-v3 measurements at the requested 16 notes x 64 lanes produced these reductions against
the preceding fused renderer:

| Internal rate | Swarm off | Wander | Jitter |
|---|---:|---:|---:|
| 1x | 53.6% | 50.0% | 58.5% |
| 2x | 44.0% | 33.1% | 38.6% |
| 3x | 60.5% | 47.0% | 61.7% |
| 4x | 61.2% | 55.6% | 62.3% |

Every factor/mode pair produced a zero-error time-domain null, including awkward Swarm retarget
boundaries. A further 180-case antialiasing/factor/shape/mode sweep produced identical checksums, and
the production event-segmentation comparator covered unaligned MIDI pitch/MPE events, smoothed-control
fallback, held-to-release transitions, and adaptive Wander blocks. A persistent packed-phase design
was separately rejected: it saved only about 1-2% while adding synchronization state. The landed
renderer therefore changes CPU scheduling only, not spectrum, aliasing, noise, or deterministic output.

## Evidence required before any candidate replaces a live mode

The current selector is valuable because it separates oscillator correction from internal rate
([`src/editor_oscillator.rs`](../../src/editor_oscillator.rs#L75-L160)). The fourth Spectral entry uses
a new parameter ID and fixed 1x rate, so old normalized three-mode automation is not reinterpreted.
Preserve that separation in
the measurements.

| Axis | Required cases | Why |
|---|---|---|
| AA/factor | all 12 combinations | Prevent a better kernel from being credited for a higher rate, or vice versa |
| Waveform | four endpoints plus every morph midpoint; pulse width 0.03, 0.1, 0.5, 0.9, 0.97 | Expose one/two-corner overlap and the duplicated Saw-Pulse edge |
| Fundamental | 20 Hz, 440 Hz, 2 kHz, 4.186 kHz, 8 kHz, and near the `0.45` phase-step cap | Cover low-frequency numeric behavior and high-frequency correction overlap |
| Host rate | 44.1, 48, and 96 kHz | Expose the current normalized-versus-absolute filter contract |
| Unison | 1, 2, 3, 4, 7, 8, 9, and 64 | Separate scalar-tail, four-wide, eight-wide, and worst-case costs |
| Modulation | pitch glide, PWM sweep, shape sweep, and swarm on/off | Distinguish static spectral quality from held-control and changing-step errors |
| Switching | every factor-to-factor and AA-to-AA transition | Verify the fixed 33-sample latency and the current reset/frozen-sample transition behavior |

Use a direct harmonic sum below Nyquist for static saw/pulse/triangle references and a much higher-rate,
verified linear-phase render for moving controls. Report at least:

- non-reference alias energy and maximum discrete alias spur;
- wanted-harmonic magnitude and phase error;
- DC, peak, and time-domain event alignment after the declared latency;
- the combined passband/stopband response, not the raw decimator alone;
- median and high-percentile CPU per host sample, plus instructions and cache misses where available;
- identical finite behavior at the phase-step and pulse-width bounds.

The primary oscillator papers use psychoacoustic masking or noise-to-mask ratios because raw SNR can
overweight inaudible components. The 2019 oversampling-filter study likewise found filter rankings
dependent on the complete interpolation/nonlinearity/decimation chain, not stopband attenuation alone.
([Kahles, Esqueda, and Välimäki, JAES 2019](https://aaltodoc.aalto.fi/items/3d3a2f3d-022a-4b48-98a5-a172c79dfb7a))
KURV should report both physical error and a perceptual metric; neither replaces the other.

A replacement wins only if it is Pareto-better under the same reference: less CPU with no material
quality loss, or lower alias/wanted-band error for the same CPU. “Sounds brighter” is not automatically
more truthful, and a steeper filter is not automatically cleaner if its passband, phase, or switching
behavior changes.

## Techniques deliberately not promoted

- **Cheaper fitted three-tap Spline correction:** the best 20 kHz three-tap fit reduced alias
  residual by another roughly 0.4-0.8 dB, but at a 3.99976 kHz fundamental its wanted-harmonic RMS
  error was 0.405 dB versus 0.115 dB for the chosen five-tap fit, and its pulse peak rose from 1.271
  to 1.310. The saved history bookkeeping did not justify making the wanted spectrum less accurate.
- **Sub-sample Shape/PWM ramps:** a causal previous-to-current interpolation prototype was compared
  with the existing held controls against the corresponding 4x moving-control render. PWM residual
  improved by effectively 0 dB at 5-100 Hz and only 0.17-0.25 dB at extreme 500-2000 Hz modulation.
  At a 4 kHz carrier with 137 Hz PWM it changed the 2x output by a -39.65 dBFS RMS null while adding
  arithmetic to every internal render. That is not a sufficient accuracy return, so it was rejected.
- **Nearest-edge SIMD BLEP shortcut:** below a quarter-cycle phase step only one side of the compact
  wrap correction can be nonzero. Specialized Spline prototypes removed the provably-zero second
  polynomial and reduced retired instructions by roughly 3-8%, but repeated wall-clock measurements
  regressed common eight-voice Saw and morph cells and were inconsistent at 64 lanes. Cold overlap
  fallbacks and dedicated optimized functions did not fix the scheduling/code-size cost, so none landed.
- **Swarm-off phase-step branch:** bypassing the unity Swarm ratio and final clamp was checksum-exact,
  but a per-lane branch regressed packed cells by about 9-30%. A vector-block specialization was neutral
  to slower. Both were rejected rather than adding code that merely looked cheaper.
- **Generic polyphase FIR:** current code already computes only the retained output. Structural-zero
  or multistage filters are the real opportunity.
- **Structural-zero 2x half-band FIR:** a true 97-tap, linear-phase half-band implementation reduced
  the decimator to 48 side MACs plus its center and preserved the 33-sample latency. It saved about
  7.3% at one oscillator, but became neutral at the dense cell and necessarily left a 20.5-24 kHz
  alias shelf. Full-band 440 Hz saw and pulse residuals regressed from about -93/-95 dBc to -43.8 dBc;
  65-97-tap variants and latency-budgeted cleanup stages could not remove the shelf without material
  passband damage. It was rejected on accuracy, not implementation polish.
- **Naive symmetry folding:** the existing note records that layout as slower. Revisit only with a
  contiguous paired-sample layout or new hardware evidence.
- **Long minBLEP/BLIT tables:** they add event state, fractional table interpolation, and cache traffic
  to a core whose current four-sample polynomials already match the waveform events. Four-point
  polynomial corrections outperformed equal-length LUT-BLEP in the 2012 study.
- **Mipmapped wavetable core:** a 2,048-point, 265-level bank reached -109 to -129 dBc static saw
  alias residual with linear interpolation, but independent gathers became 1.90-2.38x slower than
  Spline2 at 32-note workloads. See
  [`mipmapped-wavetable-comparator-2026-08-04.md`](mipmapped-wavetable-comparator-2026-08-04.md).
- **AA-IIR/AA-FIR arbitrary-waveform generation:** valuable for user-defined procedural curves, but
  KURV's four current families have explicit value/slope events and cheaper specialized corrections.
- **ADAA:** there is no memoryless waveshaper in the inspected generator path. It solves a different
  aliasing source.
- **Additive/Fourier as the real-time core:** excellent as a static reference, but low-note work scales
  with retained partial count and does not simplify PWM/morph modulation.
- **More than 4x:** no evidence in the live path justifies another full synth-rate multiplier before the
  current 1x-4x matrix is made internally consistent.

## Primary-source bibliography

1. V. Välimäki, J. Pekonen, and J. Nam, “Perceptually Informed Synthesis of Bandlimited Classical
   Waveforms Using Integrated Polynomial Interpolation,” *JASA*, 2012.
   [Author PDF](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf),
   [DOI](https://doi.org/10.1121/1.3651227).
2. J. Pekonen, J. Nam, J. O. Smith, and V. Välimäki, “Optimized Polynomial Spline Basis Function
   Design for Quasi-Bandlimited Classical Waveform Synthesis,” *IEEE SPL*, 2012.
   [Author PDF](https://mac.kaist.ac.kr/pubs/PekonenNamSmithValimaki-spletter2012.pdf).
3. J. Pekonen, J. Nam, J. O. Smith, and V. Välimäki, “Variable Fractional Delay Filters in
   Bandlimited Oscillator Algorithms for Music Synthesis,” *ICGCS*, 2010.
   [Author PDF](https://mac.kaist.ac.kr/pubs/Pekonen2010ICGCS.pdf).
4. V. Välimäki, J. Nam, J. O. Smith, and J. S. Abel, “Alias-Suppressed Oscillators Based on
   Differentiated Polynomial Waveforms,” *IEEE TASLP*, 2010.
   [Author PDF](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf),
   [DOI](https://doi.org/10.1109/TASL.2009.2026507).
5. J. Kleimola and V. Välimäki, “Reducing Aliasing from Synthetic Audio Signals Using Polynomial
   Transition Regions,” *IEEE SPL*, 2012.
   [Aalto reprint](https://aaltodoc.aalto.fi/bitstreams/0b7c5649-cd2b-476f-ba53-4e33a8103f06/download),
   [DOI](https://doi.org/10.1109/LSP.2011.2177819).
6. D. Ambrits and B. Bank, “Improved Polynomial Transition Regions Algorithm for Alias-Suppressed
   Signal Synthesis,” *SMC*, 2013. [Author PDF](https://home.mit.bme.hu/~bank/publist/smc13.pdf).
7. F. Esqueda, V. Välimäki, and S. Bilbao, “Rounding Corners with BLAMP,” *DAFx*, 2016.
   [Official proceedings PDF](https://dafx.de/paper-archive/2016/dafxpapers/18-DAFx-16_paper_33-PN.pdf).
8. H. Hohnerlein, M. Rest, and J. D. Parker, “Efficient Anti-aliasing of a Complex Polygonal
   Oscillator,” *DAFx*, 2017.
   [Official proceedings PDF](https://www.dafx.de/paper-archive/2017/papers/DAFx17_paper_100.pdf).
9. R. E. Crochiere and L. R. Rabiner, “Interpolation and Decimation of Digital Signals—A Tutorial
   Review,” *Proceedings of the IEEE*, 1981.
   [Author-hosted PDF](https://web.ece.ucsb.edu/Faculty/Rabiner/ece259/Reprints/179_interpolation_decimation.pdf),
   [DOI](https://doi.org/10.1109/PROC.1981.11969).
10. R. E. Crochiere and L. R. Rabiner, “Optimum FIR Digital Filter Implementations for Decimation,
    Interpolation, and Narrow-Band Filtering,” *IEEE TASSP*, 1975.
    [Author-hosted PDF](https://web.ece.ucsb.edu/Faculty/Rabiner/ece259/Reprints/087_optimum%20fir%20digital%20filters.pdf).
11. P. P. Vaidyanathan and T. Q. Nguyen, “A ‘Trick’ for the Design of FIR Half-Band Filters,”
    *IEEE Transactions on Circuits and Systems*, 1987.
    [Caltech author repository](https://authors.library.caltech.edu/records/czbzh-df707).
12. F. Mintzer, “On Half-Band, Third-Band, and Nth-Band FIR Filters and Their Design,” *IEEE TASSP*,
    1982. [IBM primary record](https://research.ibm.com/publications/on-half-band-third-band-and-nth-band-fir-filters-and-their-design).
13. A. Carson, V. Välimäki, A. Wright, and S. Bilbao, “Resampling Filter Design for Multirate Neural
    Audio Effect Processing,” *IEEE TASLP*, 2025. [Primary preprint](https://arxiv.org/abs/2501.18470),
    [authors' code](https://github.com/a-carson/resampling_neural_afx),
    [DOI](https://doi.org/10.1109/TASLPRO.2025.3574878).
14. J. Kahles, F. Esqueda, and V. Välimäki, “Oversampling for Nonlinear Waveshaping: Choosing the
    Right Filters,” *JAES*, 2019. [Aalto record and paper](https://aaltodoc.aalto.fi/items/3d3a2f3d-022a-4b48-98a5-a172c79dfb7a),
    [DOI](https://doi.org/10.17743/jaes.2019.0012).
15. J. O. Smith and P. Gossett, “A Flexible Sampling-Rate Conversion Method,” *ICASSP*, 1984.
    [DOI](https://doi.org/10.1109/ICASSP.1984.1172555).
16. M. Unser, “Splines: A Perfect Fit for Signal and Image Processing,” *IEEE Signal Processing
    Magazine*, 1999. [DOI](https://doi.org/10.1109/79.799930).
