# Why higher-quality DSP can sound sharper at the same output sample rate

Research date: 2026-08-04.

## Short answer

The final sample rate limits the bandwidth of the **output**. It does not fix
the error made while computing that output. Two processors can both emit 48 kHz
samples, while one emits a much better approximation of the intended
band-limited signal.

Higher-quality DSP usually means some combination of:

- fewer nonlinear aliases and images folded into the audible band;
- more accurate interpolation and fractional-time event placement;
- flatter passband, stronger stopband rejection, and better-controlled phase;
- less phase/frequency drift and round-off from finite-precision arithmetic.

The result is not extra bandwidth above 24 kHz. It is cleaner, better-timed
content inside the same 0--24 kHz output band. That can be heard as sharper
transients, more stable pitch, clearer harmonics, and less gritty or smeared
high-frequency material.

## The sample-rate limit is not the processing-accuracy limit

At output rate `F_s`, the Nyquist frequency is `F_s/2`. A properly
band-limited signal can be reconstructed from its samples by sinc-like
interpolation; the interpolation describes values between samples rather than
adding new independent information. [Smith, *Digital Audio Resampling*,
“What is Bandlimited Interpolation?”][smith-bandlimited]

That theorem does not say that every algorithm producing `x[n]` is equally
accurate. It says what can be reconstructed **if the samples are correct and
the signal is band-limited**. A cheap oscillator, waveshaper, interpolator, or
filter can put the wrong values into those same samples. The DAC then faithfully
reconstructs the wrong waveform.

This is why “it is eventually played at 1x” is not a counterargument. The final
rate decides what the DAC can reproduce; the internal algorithm decides which
48 kHz sequence reaches the DAC.

## Nonlinear processing creates new frequencies

For a linear time-invariant process, a band-limited input stays within its
bandwidth. A nonlinearity does not have that property: clipping, saturation,
wavefolding, hard-sync discontinuities, and nonlinear feedback generate new
harmonics and intermodulation products. Products above `F_s/2` fold back to
in-band frequencies, where they are no longer distinguishable from wanted
audio. This is aliasing, not merely a loss of ultrasonic content. Bilbao et al.
describe the bandwidth expansion and the resulting baseband mirroring directly
for memoryless nonlinearities. [Bilbao et al., IEEE SPL 2017][bilbao]

For example, a generated component at `f` is observed after sampling at a
folded frequency equivalent to `|f - kF_s|` for a suitable integer `k`. The
alias may land in the midrange even though the unwanted source
component was ultrasonic. Later output filtering cannot reliably remove it,
because it now overlaps the wanted band.

## What oversampling changes

An oversampled path usually does this:

```text
input -> interpolate to L*Fs -> nonlinear processing -> low-pass -> decimate to Fs
```

The output is still `F_s`. The nonlinear operation, however, is evaluated at
more time points, so more of its generated spectrum remains above the eventual
output Nyquist limit and can be removed before decimation. The wider normalized
transition band also makes the anti-alias filter’s job easier. This changes the
values of the final-rate samples; it does not change their count.

Oversampling is a practical approximation, not magic. Interpolation and
decimation filters add CPU, latency, passband error, and phase choices. Kahles,
Esqueda Flores, and Välimäki measured perceptual aliasing for several filter
choices in an eight-times waveshaping system; their results show that filter
design materially affects the result, and that the best choice is not simply
“more samples.” [Kahles et al., JAES 2019][kahles]

There are also better-than-brute-force approaches. Antiderivative antialiasing
reduces aliasing from memoryless nonlinearities at audio or reduced
oversampling rates, sometimes with fewer operations than oversampling. That is
another demonstration that **processing accuracy** and **sample rate** are
different dimensions. [Bilbao et al., IEEE SPL 2017][bilbao]

## Interpolation and phase/time resolution

The sample interval is `T_s = 1/F_s`, but an event need not be represented as
if it happened only at an integer sample. Fractional-delay filters and
bandlimited interpolation estimate the signal at times between samples. Smith
shows that even simple linear interpolation is a fractional-delay filter with
frequency-dependent error; better interpolation reduces that error, especially
when high-frequency phase matters. [Smith, *Physical Audio Signal Processing*,
“Fractional Delay Filtering by Linear Interpolation”][smith-fractional]

The same principle applies to synthesis. A high-quality oscillator can maintain
a high-resolution phase accumulator and distribute a discontinuity correction
across neighboring output samples. BLEP-style oscillators do this to suppress
the aliases caused by naively sampling a saw or pulse edge; the correction’s
sub-sample positioning is part of the sound even though the final waveform is
still emitted at `F_s`. [Välimäki, Pekonen, and Nam, JASA 2012][blep]

So “time resolution” has two meanings:

1. the output grid, which is fixed by `F_s`;
2. the accuracy with which a model places phase, delay, and transitions on that
   grid, which can be fractional and substantially better than one sample.

Higher quality improves the second. It cannot recover arbitrary information
that was never present in the input, and it cannot make the final output carry
frequencies above Nyquist.

## Filter design and perceived sharpness

An anti-alias or resampling filter has several independent quality dimensions:

- passband ripple and tilt: wanted harmonics should keep their level;
- transition width: the filter must move from passband to stopband where needed;
- stopband attenuation: images and alias-producing energy must be rejected;
- phase/group delay: frequency components should not arrive with unwanted
  relative timing changes;
- latency and ringing: a sharper finite filter generally costs taps, delay, or
  time-domain ringing.

Finite-order filters cannot realize an ideal brick wall exactly. Smith notes
that additional poles/zeros can approach the ideal amplitude response, while
phase dispersion near a cutoff can produce ringing; nonlinear phase changes
relative component timing and can smear transients. [Smith, *Introduction to
Digital Filters with Audio Applications*][smith-filters] [Smith, “Group
Delay”][smith-phase]

Thus a filter can make audio sound sharper by preserving the wanted attack and
harmonic relationships, or less sharp by smearing them. “Steeper” is not
automatically “better”: the target is the best amplitude/phase/latency tradeoff
for the signal.

## Numerical precision is another accuracy budget

Finite precision affects phase increments, filter coefficients, interpolation
fractions, feedback states, and accumulations. The resulting error can appear
as tuning drift, low-level noise, unstable resonances, or small spectral
changes. More precision or a better numerical formulation reduces those errors;
it does not increase Nyquist or create bandwidth.

Smith’s resampling analysis explicitly separates coefficient quantization,
interpolation resolution, and filter-design error. With sufficiently dense
tables and enough interpolation bits, the resampler becomes limited primarily
by the low-pass design rather than quantization effects. [Smith, *Physical Audio
Signal Processing*, “Choice of Table Size and Word Lengths”][smith-word]

## Practical conclusion for KURV

KURV’s 1x/2x/3x/4x quality control should be understood as an internal
anti-aliasing and approximation-quality control, not as a claim that the plugin
outputs above the host sample rate. At the same host rate, a higher-quality
mode can sound more accurate because it computes oscillator edges, nonlinear
products, interpolation, filtering, phase, and arithmetic with less error
before emitting the final-rate samples.

The honest acceptance criteria are therefore wanted-harmonic accuracy, alias
energy, passband/phase error, transient timing, tuning stability, and latency—not
whether a single-cycle display looks more vertically sharp.

[smith-bandlimited]: https://ccrma.stanford.edu/~jos/resample/What_Bandlimited_Interpolation.html
[smith-fractional]: https://ccrma.stanford.edu/~jos/pasp/Fractional_Delay_Filtering_Linear.html
[smith-word]: https://ccrma.stanford.edu/~jos/pasp/Choice_Table_Size_Word.html
[smith-filters]: https://ccrma.stanford.edu/~jos/filters/Lowpass_Filter_Design.html
[smith-phase]: https://ccrma.stanford.edu/~jos/filters/Group_Delay.html
[bilbao]: https://aaltodoc.aalto.fi/items/470aab15-1702-4ccf-a148-24e6173079fb
[kahles]: https://aaltodoc.aalto.fi/items/3d3a2f3d-022a-4b48-98a5-a172c79dfb7a
[blep]: https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf
