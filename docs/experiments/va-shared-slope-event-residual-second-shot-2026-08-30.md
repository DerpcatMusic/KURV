# Shared-slope derivative-event residual second shot — 2026-08-30

## Verdict

Reject production integration. The new offline shared-slope compiler and cheap
selector do not select a different representation for the shipping default
triangle-like custom curve, so its two intentional derivative events and the
resulting BLAMP quality are unchanged from the first experiment. The corrected
structural one-unison process path also regresses at high pitch. The temporary
runtime prototype was reverted; production DSP remains unchanged.

## Candidate

The smallest credible representation is two offline-known events:

| Phase | Derivative jump |
|---:|---:|
| 0.25 | -8 |
| 0.75 | +8 |

This is 16 bytes as four `f32` values. The RT prototype evaluated the existing
compact four-point fractional BLAMP polynomial only for those two events after
the selected `WaveCurveRt` evaluator. Work is bounded `O(2)` per sample, with
no allocation, lock, I/O, persistent state, or added latency. Warp and partial
custom morph retained the existing path. The prior fixed 12- and 24-sample
windowed-sinc tables were also rerun unchanged as a quality upper comparison.

The important second-shot result is negative: the selected production compiler
emits exactly the same default runtime curve and event topology used by the
earlier harness. Shared slopes remove artificial joins only on curves accepted
by the offline selector; they cannot remove these intentional corners.

## Exact ideal-bandlimited quality

The identical 65,536-sample coherent 48 kHz matrix used KURV's 33-sample output
latency and the exact triangle harmonic projection. The new-main results match
the earlier commit's recorded values to the displayed precision.

| FFT bin | Raw 1x unwanted dBc | Four-point BLAMP unwanted dBc | 24-sample residual dBc | Shipping 2x dBc |
|---:|---:|---:|---:|---:|
| 89 | -84.818 | -109.536 | -101.689 | -99.306 |
| 601 | -59.820 | -84.129 | -75.643 | -74.387 |
| 4806 | -31.413 | -52.526 | -43.257 | -47.798 |
| 7000 | -26.375 | -48.126 | -44.924 | -41.882 |

The four-point candidate suppresses unwanted bins but fails curve fidelity:
its wanted-harmonic RMS error remains 5.237, 5.397, 5.341, and 4.355 dB,
versus shipping 2x at 1.235, 1.294, 1.315, and 1.163 dB. The longer 24-sample
residual preserves wanted magnitudes much better but still loses shipping 2x
unwanted energy by 4.54 dB at bin 4806.

All coherent renders were finite and effectively DC-free. At bin 7000 the
24-sample peak remains `0.905423939` versus shipping 2x `0.948526084`; the
four-point peak remains about `0.788`. This is audible corner rounding, not an
acceptable artifact trade. The stateless four-point path has no tail to carry
through pitch changes, but its correction changes immediately with phase step;
static high-pitch peak loss already rejects it before a moving-pitch audition.

## Corrected structural one-unison CPU

The candidate was temporarily inserted into the actual structural-bank
`generate_custom_step` path. Release builds used `-C target-cpu=x86-64-v3`,
CPU 8, factor 1, one note, one unison lane, 64-frame callbacks, 20,000
callbacks, and three `perf stat` repetitions. Sample-rate changes exercise low,
mid, and high normalized phase steps while retaining the same full process.

| Host rate | Baseline instructions / cycles | Candidate instructions / cycles | Result |
|---:|---:|---:|---|
| 48 kHz | 2,174,081,939 / 889,624,894 | 2,214,666,562 / 865,365,409 | +1.9% instructions; cycles inside baseline variance |
| 8 kHz | 2,174,020,574 / 874,566,745 | 2,215,380,418 / 873,286,381 | +1.9% instructions; cycles flat |
| 2 kHz | 2,174,142,350 / 845,962,352 | 2,217,734,725 / 894,217,990 | +2.0% instructions; +5.7% cycles |

At 2 kHz the full output peak fell from `0.141914` to `0.132715`, corroborating
the high-pitch shape loss. The candidate is not Pareto-safe even before adding
generic per-curve event publication or a residual ring.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features \
  --example custom_event_lab --example process_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/custom_event_lab
$lab check
for bin in 89 601 4806 7000; do
  for mode in raw1 events1 sinc24 raw2; do
    $lab render "$mode" "$bin" 65536 "/tmp/va2-${mode}-${bin}.f32"
    python3 scripts/analyze-custom-event.py \
      "/tmp/va2-${mode}-${bin}.f32" "$bin" 65536 --delay 33
  done
done

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  ./target/release/examples/process_lab 64 20000 1 custom 1 48000 1
```

The scalar/SIMD residual check, release builds, coherent analysis, formatting,
and diff checks passed with the checkout's existing warnings. No runtime source,
dependency, or Cargo metadata change is retained.
