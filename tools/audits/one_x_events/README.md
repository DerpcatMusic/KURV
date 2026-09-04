# 1x signed-event saw: correct event timing, expensive modulation

**Experimental; not enabled in the plugin.** This is a new symmetric delayed
BLEP+BLAMP event renderer, not the older causal minimum-phase BLEP experiment.
It solves signed crossing timing for a specific, explicit reconstruction of the
sampled modulation. It is not a general solution to continuous nested PM.

## Contract and derivation

Input is unwrapped phase in cycles, linearly interpolated between audio-rate
samples. A wrapped phase value cannot communicate complete revolutions or signed
motion; do not shortest-arc unwrap the production phase and call it equivalent.
For each interval, enumerate signed integer crossings (at most 16). Forward saw
jumps are −2; reverse jumps are +2. Time within the interval is exactly
`(edge - previous_phase) / delta`. Exactly integral reverse crossings use the
right-hand sample value, tested against forward/reverse symmetry.

Between crossings a saw's slope is twice the phase slope. At every input knot,
insert a BLAMP with amplitude `2 * (new_delta - previous_delta)`. Omitting this
would treat a bent piecewise-linear phase as a continuous straight ramp and leave
an important residual. Sample-aligned BLAMP coefficients are precomputed: removing
fractional interpolation from these events reduced the initial PM implementation
from about 247 to 40–42 ns/sample on this machine. No fast-math approximation.

Corrections are the integrated step/ramp response minus the naive step/ramp.
The kernel is a normalized symmetric Blackman-windowed sinc with 16-sample radius
and 0.45 cycles/sample cutoff. That guard band is deliberate and removes wanted
content near Nyquist. The 4097-entry step and ramp tables are prepared off-thread;
the renderer never renders intermediate audio samples. Lookup-table density is
not audio oversampling. State and operations are bounded and allocation-free.

An event in `[n-1,n]` is known at sample n. A symmetric kernel reaches 16 samples
before it, so the implementation uses **17 samples latency**, aligning its raw
ring and correction timestamps. A nested chain would incur this latency at each
stage unless the whole routing graph is redesigned; simply swapping production
oscillators changes the synthesis. State is 1080-ish bytes/lane; inspect `size_of`
on the target ABI for exact size. Shared kernel storage is about 66 KiB. Output
startup settles after the finite kernel support; tests discard two periods.

Invalid/nonfinite/extreme phase is sanitized. Above 16 crossings in one interval,
the interval is explicitly degraded to an uncorrected fallback, counted by
`capped_intervals`; slope history is invalidated to prevent a huge recovery pulse.
This branch is bounded, **not alias-free**. It is not appropriate as an invisible
production fallback. Inputs must remain below 1e12 cycles; periodically subtract a
common integer from current and stored phase in a future integration.

## Reproduction

```sh
cargo +1.97.1 test --offline --manifest-path tools/audits/one_x_events/Cargo.toml
cargo +1.97.1 build --release --offline --manifest-path tools/audits/one_x_events/Cargo.toml
python tools/audits/one_x_events/probe.py
tools/audits/one_x_events/target/release/kurv-one-x-events --bench
```

The standalone crate imports the production scalar saw/antialias source; the build
script only substitutes `wide` for the identical `truce-simd` type reexports.
No private plugin dependency or production renderer is changed. Four Rust tests
verify table integration/symmetry, integer translation, reverse integer crossings,
and bounded excessive motion. The Python proof checks quality advantages and
same-filter reference agreement. Rust 1.97.1, release optimization, default x86_64
features. This is a **scalar f64 microbenchmark**, not plugin CPU or SIMD-unison
performance. Seven alternating-order rounds, 128 × 8192 samples/round; median and
range are retained. Kernel construction and phase calculation are outside timing.

## Results

All error values are normalized total reconstruction error in dB, **not isolated
alias power**. At 48 kHz the cases are 486.33 Hz, 9978.52 Hz, a 5841.80 Hz PM
modulator with 0.22-cycle depth, then nested modulation at 1236.33 and 310.55 Hz.
The PM cases include negative phase velocity. Exact definitions are in the probe.

| Case | Shipping vs continuous ideal | Events vs continuous ideal | Events vs PL ideal | Events vs PL same filter |
|---|---:|---:|---:|---:|
| Static 486 Hz | −22.12 | −30.03 | −30.03 | −68.29 |
| Static 9979 Hz | −7.59 | −25.46 | −25.46 | −60.12 |
| PM reversals | −9.33 | −16.20 | −22.65 | −58.89 |
| Nested PM | −9.91 | −14.71 | −18.96 | −55.95 |
| Reverse static | −22.12 | −30.03 | −30.03 | −68.29 |

The oracle alone oversamples at 256× and 512×, Fourier low-passes then decimates.
Their agreement is −55.7 to −69.3 dB, sufficient for these much larger errors, not
proof of arbitrarily deep rejection. The same-filter oracle includes the candidate
kernel transfer function; the other columns use the ideal Nyquist cutoff.

Continuous analytic PM and sample-linear phase produce different target signals.
Their error is already **−17.19 dB for reversals and −16.73 dB for nested PM**.
Good correction of linear segments cannot reconstruct missing continuous phase
curvature. This is why oracle agreement near −56 dB does not mean −56 dB nested-PM
accuracy. Production spline uses the original positive carrier phase step exactly
as in the audited path. Negative phase is wrapped before scalar sampling.

| Case | Event ns/sample | Shipping scalar ns/sample | Ratio |
|---|---:|---:|---:|
| Static 486 Hz | 11.53 | 5.22 | 2.21× |
| Static 9979 Hz | 47.68 | 12.53 | 3.80× |
| PM reversals | 38.72 | 5.24 | 7.39× |
| Nested PM | 42.18 | 5.23 | 8.06× |
| Reverse static | 10.89 | 5.23 | 2.08× |

**Rejected as the default 1x optimization:** improved quality costs 7–8× under PM
and adds latency. A possible quality-mode candidate only after routing/latency,
SIMD lane packing, cap policy and integration tests. Follow-up research should
compare higher-order phase reconstruction with bounded root solving, shorter
kernels, and bandlimited harmonic synthesis under smooth modulation. None is
promised faster or more accurate without a paired measurement.
