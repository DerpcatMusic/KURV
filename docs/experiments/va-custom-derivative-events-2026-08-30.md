# VA custom derivative-event experiment — 2026-08-30

## Verdict

Reject the tested 1x sparse exact-fractional derivative-event path. It lowered
unwanted energy by 4.73–10.23 dB relative to KURV's shipping 2x custom-curve
path, but achieved that by rounding and attenuating the intended curve. It was
also only 6.6% cheaper than 2x at the high-pitch 64-lane workload. No production
DSP changed.

This is not a repeat of the rejected wide-kernel, event-ring, all-wide mask,
half-cycle BLAMP, DPW, or degree-7 BLEP trials. The probe corrects only the two
actual slope events in `WaveCurveRt::default()` and uses KURV's existing compact
optimized four-point BLAMP polynomial.

## Candidate and thread contract

The default custom curve is a continuous piecewise-linear cycle with slope
jumps `-8` at phase `0.25` and `+8` at phase `0.75`. For each event, the probe
computes the signed periodic distance from the current phase, divides by the
current phase step to retain exact fractional timing, and adds:

```text
slope_jump * phase_step * optimized_blamp(distance / phase_step)
```

The candidate is stateless, deterministic, bounded, allocation-free, and
lock-free. Its audio-thread cost is `O(events * lanes)` with no persistent
memory. The retained harness allocates and writes files only outside its timed
DSP loop. Generalizing this implementation to every compiled custom curve
would require publishing its actual nonzero derivative events; that production
work was not justified after the candidate failed the curve-fidelity gate.

## Identical quality workload

All modes use the `WaveCurveRt::default()` evaluator from baseline commit
`427917d`, 48 kHz, 65,536 coherently indexed output samples, and FFT bins 89,
601, 4806, and 7000 (about 65, 440, 3520, and 5127 Hz). `raw2` evaluates the
same curve twice per host sample and uses KURV's real 97-tap 2x decimator plus
shipping spline correction EQ. The renderer warms one complete coherent cycle
before capture.

Unwanted energy contains every non-DC FFT bin outside the triangle's expected
odd harmonics. Harmonic error compares wanted magnitudes through 20 kHz against
the exact `1 / harmonic^2` triangle law. Lower is better for both columns.

| FFT bin | Raw 1x unwanted dBc | Events 1x unwanted dBc | Shipping 2x unwanted dBc | Events 1x wanted RMS error | Shipping 2x wanted RMS error |
|---:|---:|---:|---:|---:|---:|
| 89 | -84.818 | **-109.536** | -99.306 | 5.237 dB | **1.235 dB** |
| 601 | -59.820 | **-84.129** | -74.387 | 5.397 dB | **1.294 dB** |
| 4806 | -31.413 | **-52.526** | -47.798 | 5.341 dB | **1.315 dB** |
| 7000 | -26.375 | **-48.126** | -41.882 | 4.355 dB | **1.163 dB** |

Static artifact checks found zero DC in every coherent render and no NaN or
infinity. The candidate did not increase the largest adjacent sample step over
raw 1x. That is not a fidelity win: peak magnitude fell from shipping 2x
`0.948701780` to `0.788046718` at bin 7000, direct evidence of excessive corner
rounding. Because the stationary curve gate already failed, modulation and
host-audition expansion were intentionally skipped.

## CPU

Release build: `-C target-cpu=x86-64-v3`; one pinned CPU; 64 lanes; 1,000,000
host frames; three `perf stat` repetitions. Retired instructions were stable to
0.01%; cycle variation is shown because other agent builds made wall time noisy.

| FFT bin | Raw 1x instructions | Events 1x instructions | Shipping 2x instructions | Events vs 1x | Events vs 2x |
|---:|---:|---:|---:|---:|---:|
| 601 | 449,157,483 | 806,197,411 | 1,034,546,210 | +79.5% | -22.1% |
| 7000 | 449,155,779 | 966,158,393 | 1,034,545,794 | +115.1% | -6.6% |

At bin 601 the candidate used 320,388,796 cycles versus shipping 2x's
425,102,400 (-24.6%). At bin 7000 it used 426,657,936 versus 420,899,630
(+1.4%). Sparse branches lose their benefit as more lanes remain inside event
support at high pitch.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features --example custom_event_lab --locked

lab=/tmp/kurv-va-events-target/release/examples/custom_event_lab
$lab check
for bin in 89 601 4806 7000; do
  for mode in raw1 events1 raw2; do
    $lab render "$mode" "$bin" 65536 "/tmp/va-event-${mode}-${bin}.f32"
    python3 scripts/analyze-custom-event.py \
      "/tmp/va-event-${mode}-${bin}.f32" "$bin" 65536
  done
done

taskset -c 8 perf stat -r 3 -e instructions:u,cycles:u -- \
  $lab bench events1 601 64 1000000 1
```

`custom_event_lab check` verifies scalar and SIMD event results agree within
`2e-6`. The release build completed with the checkout's 83 existing warnings;
the experiment introduced no build error or new production warning.
