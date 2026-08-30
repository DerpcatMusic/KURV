# Unclamped 16-cubic coefficient mip boundary (2026-08-30)

## Verdict

Reject the five-frame representation as a general custom-curve antialiaser and
leave production unchanged. The ordinary `WaveCurveRt` output clamp was the
dominant error for discontinuous shapes at caps 2, 3, and 6, but it was not the
only boundary: 16 cubic segments cease to reproduce the ideal projection well
at caps 13 and 27. The unclamped probe is compelling only as a possible
high-note, cap-at-most-6 representation, which is not enough evidence to add a
second publication type or pitch selector.

This round added a probe-only 256-byte `UnclampedCurve` with the same 64 `f32`
coefficients and scalar/AVX2 Horner evaluation as `WaveCurveRt`, omitting only
the final `[-1, 1]` clamp. Fitting and coefficient interpolation remain offline;
the probe performs no allocation, locking, I/O, or analysis in a simulated RT
path.

## Exact ideal-projection quality

The reference is the exact retained complex Fourier series sampled on 65,536
coherent phases. `wanted` is complex coefficient error relative to retained
harmonic energy; `unwanted` is energy above the requested cap relative to that
same energy.

| Shape / cap | Clamped RMS / peak | Unclamped RMS / peak | wanted / unwanted dB | Unclamped range / RMS gain |
|---|---:|---:|---:|---:|
| square / 2 | .129375780 / .273239493 | .000006830 / .000015378 | -108.774 / -103.537 | [-1.273240, 1.273240] / .900320 |
| square / 3 | .096898825 / .200421810 | .000182069 / .000419974 | -81.379 / -75.298 | [-1.200464, 1.200464] / .949045 |
| square / 6 | .080150260 / .188356876 | .000840136 / .001949787 | -69.567 / -61.898 | [-1.188370, 1.188370] / .966034 |
| square / 13 | .054288013 / .180684328 | .015479124 / .053593099 | -48.426 / -36.338 | [-1.163680, 1.163680] / .986873 |
| square / 27 | .065217501 / .224670172 | .069025915 / .224670172 | -23.418 / -35.494 | [-1.171207, 1.171207] / .987002 |
| pulse37 / 2 | .114286347 / .372606158 | .000040122 / .000100255 | -93.536 / -87.976 | [-1.091873, 1.372665] / .926292 |
| pulse37 / 6 | .079624912 / .223049045 | .001054536 / .002709925 | -67.897 / -59.507 | [-1.223926, 1.136792] / .966858 |
| pulse37 / 13 | .054901953 / .206590772 | .014044041 / .052763849 | -49.732 / -36.821 | [-1.144961, 1.217095] / .985887 |
| pulse37 / 27 | .066321968 / .241011262 | .065569215 / .241011262 | -23.852 / -32.513 | [-1.172483, 1.188724] / .986380 |

The same boundary appears on saw: RMS rises from `.000840683` at cap 6 to
`.010026823` at cap 13 and `.047241202` at cap 27. Smooth triangle remains much
easier (`.000638661` at cap 13 and `.002413533` at cap 27), while the
representative drawn curve reaches `.002398671` and `.007534367`. Every sampled
coefficient and output was finite. Gibbs overshoot is therefore necessary data,
not an error to clip: the global sampled bound was `[-1.273240, 1.372665]`.

## Pitch-band transitions

Interpolating all 64 coefficients is continuous. The largest same-phase step
for a `2/16384` mix movement was `5.9962e-5` (square cap 6-to-13); pulse peaked
at `5.9724e-5`, saw at `5.2631e-5`, triangle at `1.1086e-5`, and drawn at
`3.4288e-5`. This removes hard band clicks but does not remove the cap-13/27
representation error or the large, shape-dependent Gibbs gain/range changes.

## Evaluator and corrected full-path CPU

Five pinned runs of the isolated x86-64-v3 release probe gave these medians:

- clamped scalar: `2.768 ns/sample`; unclamped scalar: `3.859 ns/sample`;
- clamped `eval8`: `.652 ns/sample/lane`; unclamped `eval8`: `.591 ns/sample/lane`;
- interpolate 64 coefficients plus one scalar evaluation: `5.187 ns`.

The scalar result includes the separate probe type's by-value boundary and is
not evidence for a production regression. To measure the real seam, the final
clamp was temporarily removed from the actual structural one-unison custom
path, built into separate binaries, and then reverted. With 20,000 64-frame
callbacks, factor 1, one note/lane, and three `perf stat` repetitions:

| Host rate | Clamped instructions / cycles | Unclamped instructions / cycles |
|---:|---:|---:|
| 48 kHz | 2,174,022,085 / 884,719,693 | 2,171,427,954 / 967,607,899 |
| 2 kHz | 2,173,961,621 / 910,585,563 | 2,171,490,338 / 905,125,251 |

Removing the clamp saves only about 0.12% retired instructions. Cycle variance
was 1.7-6.1%; 48 kHz regressed and 2 kHz was effectively flat. Output peak and
checksum were exactly unchanged for the ordinary default curve. Thus evaluator
CPU does not rescue the failed upper frames, and the result is not Pareto-safe.

## Minimum state if the narrow high-note result is revisited

A cap-2/3/6-only immutable bank would require three 256-byte frames, or 768
bytes per curve (192 atomic `f32` words if the current publication model were
copied). Per voice, selection needs only two frame indices and one interpolation
mix; evaluation itself is stateless and has no latency. A full five-frame bank
is 1,280 bytes per curve. Any production design must use a distinct unclamped
bandlimited type so ordinary `WaveCurveRt` retains its safety clamp, and must
define gain handling for overshoot up to `1.372665`. Those publication and gain
contracts are deliberately not added by this rejected experiment.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example coefficient_mip_lab --example process_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/coefficient_mip_lab
$lab report
for run in 1 2 3 4 5; do taskset -c 8 $lab bench; done

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  ./target/release/examples/process_lab 64 20000 1 custom 1 48000 1
```

The release probe and production baseline built with the checkout's 83 existing
warnings. No runtime source, dependency, Cargo metadata, or version change is
retained.
