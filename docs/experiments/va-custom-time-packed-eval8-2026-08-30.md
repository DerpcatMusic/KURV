# VA custom consecutive-time `eval8` experiment — 2026-08-30

## Verdict

Reject production integration. Packing eight consecutive samples through the
existing AVX2/FMA `WaveCurveRt::eval8` is bit-identical and substantially
faster in isolation, but the real one-unison process path did not reduce CPU
cycles. The temporary DSP integration was reverted. Production DSP is
unchanged.

## Candidate and eligibility

The candidate generated eight phases by the same sequential `f32` additions
and wraps as the scalar renderer, called the existing `eval8`, and copied the
eight results into the fixed 32-sample voice block. It added no persistent
state, allocation, lock, I/O, dependency, or latency. Eligibility was limited
to one unison lane, full custom mix, constant phase step and curve, and no
warp. Modulation, morphing, partial custom mix, warp, and multi-lane rendering
retained their current paths.

## Exact output

The probe compared scalar Horner and packed `eval8` at FFT bins 1, 89, 601,
and 7000 for block sizes 31, 32, and 33. Each cell ran 32,768 blocks: 12,582,912
samples total. Every output bit and every continuation-phase bit matched.
There was therefore zero RMS error, zero peak error, and zero segment/wrap
boundary error. Full one-unison renders also produced identical peak and
checksum values for both factor 1 and shipping factor 2.

## CPU

Release builds used `-C target-cpu=x86-64-v3`, CPU 8, and three `perf stat`
repetitions.

The isolated 32-sample curve loop used 500,000 blocks:

| FFT bin | Scalar instructions / cycles | Packed instructions / cycles |
|---:|---:|---:|
| 89 | 585,066,757 / 250,827,567 | 237,243,718 / 166,180,709 |
| 601 | 585,068,219 / 223,850,253 | 237,242,232 / 161,543,015 |
| 7000 | 585,066,539 / 235,021,135 | 237,239,924 / 159,930,213 |

This is about 59% fewer instructions and 28–34% fewer cycles in isolation.

The representative full process used one note, one unison lane, 64-frame host
callbacks, 20,000 callbacks, and the actual custom structural-bank path:

| Workload | Baseline instructions / cycles | Packed instructions / cycles |
|---|---:|---:|
| factor 1, 48 kHz | 2,155,737,639 / 847,005,838 | 2,059,113,307 / 847,266,820 |
| factor 2, 48 kHz | 2,612,325,375 / 971,520,237 | 2,419,073,374 / 1,102,916,598 |

Factor 1 retired 4.5% fewer instructions but cycles were unchanged within the
2.3–2.7% run variance. Factor 2 retired 7.4% fewer instructions, yet its cycle
measurement regressed 13.5% with 11% variance. This does not establish a
representative CPU win, much less a Pareto improvement over shipping 2x. The
selector/gather work is cheaper in isolation, but is not the limiting cost in
the complete voice and oversampling pipeline.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example time_pack_curve_lab --example process_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/time_pack_curve_lab
$lab check
taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench pack8 601 500000 1

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  ./target/release/examples/process_lab 64 20000 1 custom 1 48000 1
```

`process_lab` gained only a custom-curve scenario and explicit oversampling
argument so the full workload remains reproducible. The experiment probe is
offline-only. The release build and bit-exact checks passed with the checkout's
83 pre-existing warnings.
