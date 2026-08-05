# Alternatives to standard audio oversampling

Research date: 2026-08-04

## Executive verdict

Yes—but there is no single replacement for oversampling. The strongest result in
the literature is a set of domain-specific replacements:

| Problem | Best-supported alternative | Verdict for KURV |
|---|---|---|
| Saw/pulse discontinuities | Higher-order polynomial BLEP, DPW, BLIT/BLIT-FDF, or a tabulated minBLEP | Real replacement for oscillator oversampling when the waveform and phase model are known. |
| Triangle corners | PolyBLAMP | Direct replacement for oversampling the triangle corner; especially relevant because KURV's triangle is currently uncorrected. |
| Memoryless waveshaping | First-/higher-order ADAA or continuous-time convolution | Real replacement or oversampling reducer for a known scalar nonlinearity; not a generic oscillator solution. |
| Stateful nonlinear circuits | Stateful ADAA, WDF/state-space reformulation, or trajectory antialiasing | Promising, but requires redesigning the circuit discretization, state update, and sometimes an implicit solve. |
| Static/structured waveforms | Bandlimited interpolation, integrated wavetables, additive/BLIT tables | Real replacement for a fixed waveform family; less suitable for arbitrary per-sample shape processing without a multidimensional table. |
| Resampling cost | Polyphase, half-band, multistage, or carefully designed IIR/FIR decimators | Does not replace oversampling; it makes the unavoidable rate conversion cheaper and can reduce latency. |

The practical conclusion is a hybrid one: keep a modest oversampling path as a
reference and fallback, then replace the expensive parts that have an exact
structure. For KURV, the highest-confidence first targets are higher-order
PolyBLEP or DPW for saw/pulse, PolyBLAMP for triangle, and a 2x-to-2x
half-band/polyphase decimator for the 4x path. ADAA and stateful WDF methods
only become relevant if KURV gains an explicit nonlinear waveshaper or circuit
model.

“Better sounding” must mean lower alias energy, more accurate wanted harmonics,
controlled phase/latency, and stable behavior under modulation. A mathematically
sharper-looking single-cycle plot is not a quality metric: a vertical edge
contains infinite bandwidth and cannot be reproduced at a finite output rate.

## What oversampling is actually buying

Oversampling changes the point at which nonlinear or discontinuous content is
created:

```text
interpolate -> process at L Fs -> low-pass -> decimate to Fs
```

Content above the host Nyquist frequency can be removed while it is still
ultrasonic. Without that intermediate bandwidth, it folds into the audible
band before any final filter can remove it. This is why oversampling changes the
final `Fs`-rate samples even though the host ultimately receives only one sample
per host interval.

The alternatives below do something more selective: they model the continuous
time event, its discontinuity, or its nonlinearity directly, so less unwanted
energy is created in the first place. They are not magic extra bandwidth and
they do not solve arbitrary nonlinear processing for free.

## 1. Oscillator discontinuities

### BLEP, minBLEP, and PolyBLEP

The original BLEP idea is to represent a saw or pulse as ordinary piecewise
waveforms plus a correction around each step. The correction is the difference
between an ideal discontinuity and a bandlimited step. Stilson and Smith's
foundational paper derives both BLIT and BLEP-style constructions, discusses
windowed-sinc truncation, harmonic limits, control behavior, and the CPU/quality
tradeoff of direct formulas versus tables. [Stilson & Smith, “Alias-Free Digital
Synthesis of Classic Analog Waveforms” (ICMC 1996)](https://quod.lib.umich.edu/i/icmc/bbp2372.1996.101/--alias-free-digital-synthesis-of-classic-analog-waveforms?rgn=main%3Bview%3Dfulltext)

`minBLEP` is best understood as a practical implementation family, not a
separate mathematical theorem: precompute a finite, usually minimum-phase
BLEP residual and overlap it at each event. It offers a longer and more
accurate correction than a tiny polynomial, but costs table memory, lookup/
interpolation, residual-buffer writes, and careful overlap handling. It is
particularly useful for hard sync and arbitrary discontinuity timing. Eli
Brandt's hard-sync paper points to the public-domain MinBLEP generator and
explains the relationship to the Stilson/Smith construction. [Brandt, “Hard sync
without aliasing”](https://www.cs.cmu.edu/~eli/L/icmc01/hardsync.html) [Werner,
MinBLEP generator source](https://www.musicdsp.org/en/latest/Synthesis/211-matlab-octave-code-for-minblep-table-generation.html)

`PolyBLEP` replaces the lookup table with a short polynomial approximation. The
important tradeoff is explicit: a short correction is cheap and SIMD-friendly,
but its residual is not perfectly bandlimited and its rejection falls off at
high oscillator frequencies. It also has a finite event window, so overlapping
events, pulse widths near one sample, rapidly changing frequency, and hard sync
need special treatment.

The strongest directly comparable result is the integrated polynomial/B-spline
study by Välimäki, Pekonen, and Nam. Their integrated third-order B-spline
correction had the best cost/quality tradeoff among the tested methods and was
perceptually alias-free for sawtooth fundamentals up to 7.8 kHz at 44.1 kHz.
That is a perceptual result for their implementation and test protocol, not a
guarantee for every PolyBLEP formula. [Välimäki, Pekonen & Nam, “Perceptually
informed synthesis of bandlimited classical waveforms using integrated
polynomial interpolation” (JASA 2012)](https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf)

**Cost/quality:** tiny polynomial corrections are among the cheapest approaches
for a large oscillator bank; tabulated BLEP can achieve lower residual aliasing
but consumes cache/state and is less convenient for SIMD. Neither is a generic
replacement for antialiasing arbitrary nonlinear processing.

**Failure modes:** event correction must be placed at the correct fractional
phase; a correction table must handle wraparound and multiple events; pulse
width and hard-sync events can overlap; and low-order polynomial formulas may
need a high-frequency fallback or a modest oversampling factor. A BLEP also
does not automatically correct a derivative discontinuity: triangle needs a
BLAMP-type correction instead.

### BLIT and low-order fractional-delay BLIT

BLIT generates a bandlimited impulse train, then integrates it to obtain saw,
pulse, or triangle waveforms. The original paper gives a closed-form sampled
BLIT based on a periodic digital sinc and also discusses windowed-sinc overlap,
harmonic count, DC correction, and parameter changes. At high frequencies the
number of in-band harmonics becomes small enough that direct sine summation can
be cheaper than a BLIT; at lower frequencies a BLIT is efficient because one
event represents many harmonics. [Stilson & Smith, ICMC 1996](https://freeverb3-vst.sourceforge.io/doc/blit.pdf)

The closed form has numerical singularities when the sinc denominator is near
zero; the authors' equivalent limiting form or an explicit limit branch is
required. Integration also introduces state, DC-offset, startup, and
frequency-change problems. A low-order fractional-delay filter can make the
impulse event compact and continuously positionable. Nam et al. report that a
third-order B-spline fractional-delay BLIT uses only a few nonzero samples per
period and is perceptually efficient, but the algorithm still needs DC and
transient management. [Nam, Välimäki, Smith & Abel, “Efficient antialiasing
oscillator algorithms using low-order fractional delay filters”](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf)

**Verdict:** a real oscillator replacement, most attractive for a waveform
family with sparse periodic events and for hard sync/PWM designs that benefit
from explicit impulse timing. It is more stateful and numerically delicate than
the current compact PolyBLEP, so it is not the first KURV change without a
measured reason.

### DPW / differentiated polynomial waveforms

DPW samples a smooth polynomial waveform and differentiates it one or more
times. Differentiation restores the desired saw/triangle spectral slope while
the smoother underlying polynomial suppresses alias energy. Higher polynomial
orders provide stronger suppression with regular arithmetic and little state.

The objective evaluation by Välimäki, Nam, Smith, and Abel found fourth-order
DPW perceptually alias-free over the grand-piano register. At 44.1 kHz their
second-order version could become audible above roughly 600 Hz, while fourth
order reached about 4.6 kHz in the reported analysis. They explicitly conclude
that increasing polynomial order generally costs less than simply doubling the
sample rate. Their practical design combines a low-order method at low
frequencies with fourth-order DPW above a crossover near 500 Hz because higher
orders require large low-frequency scale factors. [Välimäki et al., “Alias-
Suppressed Oscillators Based on Differentiated Polynomial Waveforms” (IEEE
TASLP 2010)](https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf)

**Cost/quality:** excellent for SIMD oscillator banks: polynomial arithmetic,
finite differences, and one history value are branch-light and table-free.
DPW4 can replace 2x–4x oversampling for ordinary saw/triangle generation at a
fraction of the work, but the exact crossover depends on sample rate,
frequency, amplitude, and the permitted alias floor.

**Failure modes:** large normalization at low frequency amplifies numerical
noise; phase/frequency changes can create transients; the method is tied to the
polynomial waveform and does not directly solve arbitrary shape morphing or
hard-sync topology. Use a low-frequency fallback or a hybrid crossover and
measure DC, noise, and long-run stability.

### PolyBLAMP

BLAMP is the derivative-discontinuity analogue of BLEP. A triangle has finite
amplitude but corners where its first derivative jumps; a step correction alone
does not address that spectral event. Esqueda, Välimäki, and Bilbao derive a
polynomial BLAMP correction for triangles, clipping, and rectification.

Their results report up to 50 dB reduction of alias components and about 20 dB
overall SNR improvement, with the polynomial method more efficient than the
oversampling baselines in the studied cases. [Esqueda, Välimäki & Bilbao,
“Rounding corners with BLAMP” (DAFx 2016)](https://dafx.de/paper-archive/2016/dafxpapers/18-DAFx-16_paper_33-PN.pdf)

For a more complex polygonal oscillator, Hohnerlein, Rest, and Parker report
about 20 dB SNR improvement comparable to 2x–4x oversampling at roughly 25x
lower computational complexity, under their stated oscillator and test
conditions. [Hohnerlein, Rest & Parker, “Efficient Anti-aliasing of a Complex
Polygonal Oscillator” (DAFx 2017)](https://www.dafx.de/paper-archive/details/4nfXx-67F78HSIzBoSbRgA)

**Verdict:** the clearest direct win for KURV's current triangle path. It is not
a general replacement for all oscillator antialiasing, but it targets exactly
the uncorrected corner event in [`src/oscillator.rs`](../../src/oscillator.rs#L99-L101).

## 2. Memoryless nonlinearities

### ADAA and higher-order ADAA

For a scalar memoryless nonlinearity `y = f(x)`, ADAA assumes a continuous-time
interpolation between adjacent input samples and averages the nonlinear output
over that interval. With `F' = f`, first-order ADAA is:

```text
y[n] = (F(x[n]) - F(x[n-1])) / (x[n] - x[n-1])
```

When the denominator is small, the expression is ill-conditioned and must use
a limit/series branch. For `f(x)=tanh(x)`, the first antiderivative is
`log(cosh(x))`, but stable implementations must also avoid overflow and
catastrophic cancellation in the chosen antiderivative.

Bilbao et al. generalized the method to second- and third-order antiderivatives.
Their paper reports improved SNR over first-order ADAA and fewer operations than
straightforward oversampling. Higher order is not free: it needs more
antiderivative evaluations, more history, and carefully derived functions (or
tables). [Bilbao, Esqueda, Parker & Välimäki, “Antiderivative antialiasing for
memoryless nonlinearities” (IEEE SPL 2017)](https://www.research.ed.ac.uk/en/publications/antiderivative-antialiasing-for-memoryless-nonlinearities/)
[Accepted author manuscript](https://www.research.ed.ac.uk/files/34115216/bilbao_pdf.pdf)

**Cost/quality:** a known analytic `f`, `F`, `F2`, ... can be evaluated once per
output sample rather than evaluating the whole nonlinear chain at 2x–8x. For
table-defined or expensive nonlinearities, tables and interpolation can move
the cost from arithmetic to memory. Higher order can buy more rejection but
may be slower than modest oversampling for a cheap `tanh` or polynomial.

**Failure modes:** only memoryless scalar nonlinearities are covered directly;
rapidly changing inputs and high-order antiderivative tables need interpolation
care; equal or nearly equal adjacent samples cause 0/0 cancellation; a
half-sample delay and low-pass effect are intrinsic to the basic construction.
ADAA cannot be inserted blindly into a feedback loop because that delay changes
the loop phase and stability.

### Continuous-time convolution / antialiased waveshaping

Parker, Zavalishin, and Le Bivic derive a closely related method: reconstruct a
piecewise-continuous input, apply the nonlinearity in continuous time, then
analytically convolve the result with a continuous-time rectangular, triangular,
or other piecewise-polynomial kernel. For a locally linear input and a
rectangular kernel, the result reduces to the ADAA divided difference. Higher
order kernels provide different low-pass behavior and more delay.

Their hard-clipper sweep is unusually useful as an objective comparison. With
input gain 10, roughly comparable alias levels required:

| Method | Required rate | Relative rate | Reported SNR |
|---|---:|---:|---:|
| No kernel | 529.2 kHz | 12x | 46.7 dB |
| Rectangular kernel | 176.4 kHz | 4x | 46.3 dB |
| Triangular kernel | 132.3 kHz | 3x | 46.6 dB |

This result demonstrates a major oversampling reduction, not universal 1x
alias-free processing. The method introduces delay and passband attenuation;
the paper gives a half-sample delay for the rectangular/first-order case and a
one-sample delay for the triangular/second-order case. It also warns about
ill-conditioning and the extra difficulty of delayless feedback. [Parker,
Zavalishin & Le Bivic, “Reducing the Aliasing of Nonlinear Waveshaping Using
Continuous-Time Convolution” (DAFx 2016)](https://www.dafx.de/paper-archive/2016/dafxpapers/20-DAFx-16_paper_41-PN.pdf)

**Verdict:** a strong replacement/reducer for a known waveshaper, especially
when the required antiderivatives are cheap. It is not an oscillator method and
does not replace KURV's current oscillator oversampling by itself.

## 3. Stateful nonlinear circuits

### Stateful ADAA

Holters extends ADAA to systems with state, including one-port nonlinearities
inside virtual-analog circuit models. The important detail is that output-only
antialiasing is insufficient: the state update must be transformed too, or the
feedback state continues to inject aliasing. The method may require changing
the discretized integrator and compensating the introduced delay.

In the diode-clipper and tube-screamer-like examples, state-update mitigation at
88.2 kHz (2x) produced low-frequency alias levels comparable to an unmodified
220.5 kHz (5x) model. Output-only mitigation had limited benefit, and one
configuration attenuated higher harmonics by up to 10 dB. The authors describe
the extra compute as modest but note that the main cost can be antiderivative
tables; one example used 1024 table points with cubic interpolation. [Holters,
“Antiderivative Antialiasing for Stateful Systems” (DAFx 2019)](https://www.dafx.de/paper-archive/2019/DAFx2019_paper_4.pdf)

**Verdict:** a real oversampling reducer for eligible stateful models, not a
drop-in post-process. It is useful only when the model can be expressed with a
scalar nonlinearity input and the state/discretization can be redesigned.

### Wave Digital Filters and state-space methods

WDFs and state-space virtual-analog models make the circuit topology explicit,
which gives ADAA a place to act at a nonlinear junction. Albertini, Bernardini,
and Sarti evaluate first- and higher-order ADAA integrated into stateful WDFs
with one-port and multiport nonlinearities, including diode and BJT circuits.
Their source-backed result is significant alias reduction at low oversampling
factors while preserving WDF modularity. [Albertini, Bernardini & Sarti,
“Antiderivative Antialiasing Techniques in Nonlinear Wave Digital Structures”
(AES 2021)](https://hdl.handle.net/11311/1208018)

The costs are model-specific: solving a delay-free loop can require Newton
iterations, an antiderivative may only exist numerically, lookup tables add
memory, and the transformed structure can change the small-signal response or
stability margin. These are not plausible replacements for a generic VA
oscillator. They become relevant to KURV only if it moves toward nonlinear
circuit modeling rather than phase/shape synthesis.

## 4. Bandlimited interpolation and integrated wavetable methods

Bandlimited interpolation is not “extra samples” in the output. It is a better
continuous-time reconstruction between samples. Sinc or windowed-sinc
interpolation gives the ideal bandlimited result in principle; finite kernels
trade stopband rejection and passband error against CPU, memory, and latency.
Julius Smith's resampling text documents the relationship between bandlimited
interpolation, fractional delay, table size, interpolation order, and word
length. [Smith, *Digital Audio Resampling*](https://ccrma.stanford.edu/~jos/resample/)

Integrated wavetables precompute the waveform or its antiderivative so that a
phase lookup plus interpolation approximates the bandlimited waveform at the
requested phase. They can be extremely cheap for static shapes and make
multi-octave alias control straightforward: each table or band contains only
the harmonics that fit its range. The integrated polynomial/B-spline paper
above is a principled version of this idea rather than a generic sampled table.

**Cost/quality:** very low per-sample arithmetic after table construction;
excellent cache behavior if the table is small and the phase path is coherent;
quality depends on table bandwidth, phase interpolation, table transitions, and
shape dimensions. A table can outperform 4x brute-force oscillator rendering
for static shapes, but a continuously morphing waveform needs multiple tables,
crossfades, or an analytic correction.

**Failure modes:** table switching can produce pitch-dependent timbral steps;
linear interpolation has frequency-dependent amplitude/phase error; fast
frequency modulation and hard sync expose table-band transitions; and a
shape-morph dimension multiplies memory. This is a possible replacement for a
fixed oscillator bank, not a general replacement for oversampling after
arbitrary phase/shape processing.

## 5. Making the remaining oversampling cheaper

These methods improve rate conversion; they do not remove the need to create
and discard intermediate samples.

### Polyphase FIR

A decimator need not evaluate every FIR phase at every high-rate sample.
Polyphase decomposition partitions the prototype filter into phase subfilters
and evaluates only the phase that contributes to the retained output. This is
the standard exact FIR optimization for rational sample-rate conversion. It
reduces arithmetic and memory traffic without changing the designed response.
Smith's resampling material provides the signal-processing derivation; the
official multirate implementation reference from MathWorks shows the same
polyphase structure in practical FIR decimators. [Smith, *Digital Audio
Resampling*](https://ccrma.stanford.edu/~jos/resample/) [MathWorks,
“IIR Halfband Stages in Multistage Filter Design”](https://www.mathworks.com/help/dsp/ug/iir-polyphase-filter-design.html)

### Half-band and multistage filters

For 2x conversion, a half-band low-pass filter has approximately every other
FIR coefficient equal to zero, so only about half the taps need multiplication.
For 4x, cascading two 2x stages lets each stage run at the cheapest available
rate and preserves the wider transition band of the earlier stage. For a 3x
path there is no half-band shortcut; a 3:1 polyphase FIR or a carefully
designed IIR/FIR cascade is more natural.

The tradeoff is not merely CPU. A linear-phase FIR gives predictable delay and
usually good transient behavior but can require many taps. Minimum-phase or
IIR designs reduce latency and operations but introduce nonlinear phase,
feedback state, coefficient sensitivity, and possible ringing/denormal issues.
The filter must be designed at the actual factor and host sample rate, not
copied between 2x, 3x, and 4x.

### IIR/FIR hybrids

Carson, Välimäki, Wright, and Bilbao's 2025 resampling study found that a
two-stage half-band IIR followed by a Kaiser-window FIR gave similar or better
results than a competing sample-rate-independent method, with many fewer
operations per sample and under 1 ms latency at typical audio rates. This is
strong evidence for a cheaper decimator, but it is from neural distortion-model
benchmarks and does not prove that the same coefficients are optimal for KURV.
[Carson et al., “Resampling Filter Design for Multirate Neural Audio Effect
Processing” (2025)](https://arxiv.org/abs/2501.18470)

**Failure modes:** IIR phase and feedback state can alter oscillator transients;
minimum-phase filters can shift events; multistage passband errors accumulate;
and poor coefficient quantization can make a narrow transition unstable. FIR
polyphase is the conservative KURV option; an IIR or hybrid should win only if
measured latency/CPU gains survive spectral and transient comparison.

## 6. Mapping to current KURV

This mapping is against the live working-tree source inspected on 2026-08-04,
not a claim about the historical initial commit.

### Current path

- [`src/oscillator.rs`](../../src/oscillator.rs#L325-L426) generates saw and
  pulse with a compact two-sample PolyBLEP correction. The scalar and SIMD
  implementations use the same low-order correction family.
- [`src/oscillator.rs`](../../src/oscillator.rs#L99-L101) and
  [`src/oscillator.rs`](../../src/oscillator.rs#L278-L293) generate triangle as
  a piecewise linear absolute-value waveform with no BLAMP correction.
- [`src/lib.rs`](../../src/lib.rs#L691-L695) calls the synth `factor` times for
  each host sample, so 2x/3x/4x changes the oscillator's internal evaluation
  rate before decimation.
- [`src/oversampling.rs`](../../src/oversampling.rs) sets a 33-sample host delay
  and fixed storage for x2/x3/x4 decimators. The selected decimators use 97,
  145, and 193-tap linear-phase equiripple kernels, with explicit delay
  compensation preserving the previous host-latency contract.
- Factor 1 is a delayed direct path, not an antialiased oscillator path; the
  current saw/pulse PolyBLEP still supplies its local correction.

### Recommended KURV experiments, in order

1. **Add PolyBLAMP to triangle first.** It directly addresses the current
   derivative discontinuity and has the clearest literature match. Keep the
   existing PolyBLEP saw/pulse path as the comparison reference.
2. **Compare higher-order integrated polynomial BLEP and DPW4 for saw/pulse.**
   Both are table-free or small-state and SIMD-compatible. Use a low-frequency
   DPW fallback/crossover if scale-factor noise appears. Do not assume DPW is a
   drop-in replacement for shape morphing or hard sync.
3. **Only benchmark minBLEP/BLIT-FDF if high-frequency alias rejection remains
   audible.** Their extra event state and cache traffic may lose against
   polynomial arithmetic at KURV's large unison counts.
4. **Optimize decimation separately.** The first measured iteration replaced
   the longer Kaiser kernels with shorter equiripple kernels. Symmetry-folding
   variants were rejected because reverse access and extra state made them
   slower on the measured CPU. A future multistage/half-band experiment still
   has to beat the retained one-stage filters under the same response contract.
5. **Keep 2x or 3x as a reference mode.** A specialized oscillator algorithm
   should replace it only after measuring aliased energy, wanted-harmonic
   error, passband tilt, transient timing, DC, NaN/Inf behavior, and CPU across
   low, mid, and near-Nyquist fundamentals.
6. **Do not add ADAA yet.** There is no current KURV memoryless saturation or
   circuit nonlinearity in the inspected oscillator/voice path. If a future
   waveshaper is introduced, ADAA or continuous-time convolution should be
   evaluated at that nonlinear seam rather than wrapped around the whole synth.

## Final answer

Standard oversampling can be replaced, but only where the signal structure is
known. For KURV, oscillator-specific antialiasing is the strongest path to
better quality per CPU: PolyBLAMP for triangle, higher-order PolyBLEP or DPW4
for saw/pulse, and improved polyphase/half-band decimation for the remaining
rate conversion. ADAA and continuous-time convolution are stronger for
memoryless nonlinearities; stateful ADAA/WDF is stronger for circuit models.
None is a universal, strictly better replacement for arbitrary nonlinear DSP.
