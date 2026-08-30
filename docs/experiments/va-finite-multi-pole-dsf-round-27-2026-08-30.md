# Finite multi-pole DSF coefficient law, round 27 (rejected)

Date: 2026-08-30
Baseline: `f950be609a9f0dbeb1ece93b0c72cdcc3350c54f`

## Verdict

Reject the finite three-pole DSF saw renderer. The source-independent candidate
was finite, stateless, phase-exact, table-free, and substantially more accurate
than the shipping spline saw at cap 3 and cap 54. It did not reproduce the
long cap-872 harmonic tail closely enough, regressed the 27.5 Hz quality case,
and lost every measured scalar and x8 CPU gate. No production code, parameter,
UI, version, or release path was changed.

The ignored source harness was removed after recording this report. Square and
triangle were not attempted because saw did not clear the common gate.

## Candidate contract

The experiment approximated the saw coefficient law with at most three shared
signed geometric poles:

```text
1 / k ~= sum(weight[j] * radius[j]^k), j = 0..P
```

One offline fit was shared by legal harmonic caps 3, 54, and 872. Runtime did
not select cap-specific weights or a table bank. The closed-form finite series
subtracted each pole's tail exactly at the legal harmonic cap; it did not use an
infinite-series approximation. The three-pole kernel occupied 52 bytes, owned
no persistent oscillator state, added no latency, and performed no allocation,
locking, I/O, or logging.

The focused report covered ideal band-limited error, wanted-bin complex error,
off-grid energy, one-cycle shape/DC/gain/peak at four initial phases, cap
boundaries, a rapid 24/32-frame pitch schedule, phase reset, scalar-x8 parity,
exact phase publication, finite-tail identity, pole-denominator stability, and
real scalar/x8 stereo accumulation CPU before the common oversampler.

## Shared coefficient fit

| poles | shared relative RMS | max relative error | cap 3 wanted error | cap 54 wanted error | cap 872 wanted error |
|---:|---:|---:|---:|---:|---:|
| 1 | 0.244426506 | 1.000000000 | -18.203 dB | -11.341 dB | -10.802 dB |
| 2 | 0.076103998 | 1.000000000 | -30.280 dB | -24.260 dB | -19.269 dB |
| 3 | 0.029672181 | 0.997327871 | -38.270 dB | -33.132 dB | -27.267 dB |

The best legal fit used:

| pole | radius | weight |
|---:|---:|---:|
| 1 | 0.989289074454 | 0.036685912492 |
| 2 | 0.859756797082 | 0.329081816746 |
| 3 | 0.338626976331 | 2.005575665735 |

Allowing signed weights did not produce negative weights in the selected fit.
The cap-872 maximum relative error remained 0.997327871. Meeting that tail with
more than three poles, cap-specific weights, or a coefficient table would
violate this round's acceptance boundary.

## Saw quality and one-cycle shape

| period | frequency | cap | current ideal error | DSF ideal error | DSF wanted complex error | DSF off-grid | current RMS | DSF RMS | DSF DC | DSF gain | DSF raw peak |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1745 | 27.507 Hz | 872 | -30.761 dB | -26.617 dB | -24.254 dB | -35.797 dB | 0.016719785 | 0.026942024 | -0.000008326 | 0.999858011 | 1.199223995 |
| 109 | 440.367 Hz | 54 | -22.289 dB | -33.122 dB | -33.123 dB | -68.120 dB | 0.044111973 | 0.012674082 | 0.000000226 | 0.999218819 | 1.126910090 |
| 7 | 6857.143 Hz | 3 | -9.614 dB | -38.270 dB | -38.270 dB | -312.228 dB | 0.173627877 | 0.006409188 | 0.000000049 | 0.999582693 | 0.902002037 |

The candidate is a strong high-note reconstruction and improves the cap-54
case, but it is not universally Pareto-safe. At cap 872 it changes current
ideal error from -30.761 dB to -26.617 dB and increases RMS error by 61%.

The one-cycle alignment probe reached the same conclusion at initial phases
0.000, 0.137, 0.371, and 0.733. Cap-872 DSF residual RMS stayed near 0.0250,
versus roughly 0.0104-0.0110 current, with DSF residual peaks up to 0.5474.
At cap 54 the DSF residual RMS was 0.012658 for every phase, versus
0.04099-0.04380 current. At cap 3 it was 0.006407, versus
0.13303-0.14719 current. Fitted phase offsets remained below 0.00014 samples
for the cap-54 and cap-3 DSF cases, so those improvements are shape accuracy,
not phase alignment artifacts.

## Cap, pitch, and reset transitions

The cap-boundary probe compared candidate and exact output steps both at a
same-phase cap switch and across the actual adjacent sample:

| cap from/to | switch error peak | switch error RMS | adjacent error peak | adjacent error RMS |
|---|---:|---:|---:|---:|
| 2/1 | 0.005813232 | 0.004110521 | 0.007284935 | 0.004240168 |
| 3/2 | 0.006876158 | 0.004862047 | 0.017985374 | 0.008652764 |
| 4/3 | 0.005527991 | 0.003908721 | 0.018666117 | 0.011406107 |
| 5/4 | 0.001093539 | 0.000772965 | 0.019219329 | 0.011887857 |
| 6/5 | 0.002329565 | 0.001646533 | 0.018349362 | 0.010728281 |
| 54/53 | 0.001331705 | 0.000938553 | 0.069444103 | 0.011143981 |

On the `440x24 | 7040x32 | 110x24 | 12000x32` rapid schedule, exact and
candidate RMS were 0.544324765 and 0.544084788. Candidate peak was 1.211634517
versus exact 1.158409238, and its maximum pitch-event step was 1.900528908
versus exact 1.860798597 and current 1.587572217. The DSF therefore tracks the
finite projection closely but does not improve the shipping transition artifact
envelope.

Reset replay was deterministic with zero replay error. At cap 54 the first
post-reset DSF step was -0.517411947 versus exact -0.513009119, an error of
-0.004402828.

## Numeric, parity, and stability gates

The numeric scan covered caps 1, 2, 3, 4, 5, 6, 54, 218, and 872:

- scalar-x8 peak difference: `0.000006109`;
- direct finite-tail identity peak: `0.000000000000`;
- minimum pole denominator: `0.000114724549`;
- maximum scanned signal magnitude: `1.284910798`;
- phase publication: bit-exact;
- every scanned value: finite.

These gates passed. They establish that the rejection is quality and throughput,
not numerical instability or phase ownership.

## Real scalar and x8 CPU

Values are median nanoseconds per output sample/frame. Each path performed real
stereo accumulation before the common oversampler; ranges are the minimum and
maximum of five measurements.

| frequency | cap | frames | scalar current | scalar DSF | scalar ratio | x8 current | x8 DSF | x8 ratio |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 440 Hz | 54 | 24 | 6.211 | 20.002 | 3.220x | 3.471 | 30.400 | 8.759x |
| 440 Hz | 54 | 32 | 6.316 | 20.408 | 3.231x | 2.448 | 30.787 | 12.575x |
| 7040 Hz | 3 | 24 | 9.864 | 20.378 | 2.066x | 3.994 | 29.782 | 7.456x |
| 7040 Hz | 3 | 32 | 10.006 | 20.777 | 2.076x | 3.881 | 30.039 | 7.740x |

The most favorable measured result was still 2.066 times current scalar CPU.
The production-critical x8 structure was 7.456-12.575 times current. No amount
of transition policy can recover that steady-state deficit.

## Reproduction

The retained harness was rebased unchanged onto the current baseline, formatted,
and run with:

```bash
cargo fmt -- src/oscillators/va/experiment.rs
cargo test --release -q dsf_round27::finite_multi_pole_dsf_saw_report -- --ignored --nocapture
```

Result: one focused test passed, 411 tests were filtered out, and the report
completed in 1.36 seconds after compilation. The build emitted existing
unused/dead-code warnings and no DSF compile errors.

## Decision boundary

Do not productionize this candidate. The legal three-pole shared law cannot
match all measured caps, and the closed-form runtime is much slower than both
current scalar and x8 paths. A follow-up would be a materially different
experiment only if it changes the representation or execution model; adding
more poles, per-cap weights, or a cap-indexed table is explicitly outside this
round and would worsen its already failed CPU contract.
