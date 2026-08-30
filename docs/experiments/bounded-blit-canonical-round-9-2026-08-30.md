# Bounded f64 BLIT round 9 (rejected)

Date: 2026-08-30

Baseline: `350d516` (production DSP unchanged)

Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, CPU 8

Verdict: rejected; no runtime or benchmark code retained

## Candidate

Round 8's raw `f32x8` BLIT arithmetic was fast but numerically unstable. This
second shot used the minimum authoritative bounded scheme:

- two `f64x4` banks for eight lanes;
- f64 phase, numerator/denominator rotations, prior BLIT value, and integrator;
- analytical zero handling for the periodic-sinc quotient;
- exact rephase and exact finite-saw DC/integrator initialization every
  64-sample structural block;
- both Euler and trapezoidal integration, with every rephase included in CPU.

The sample loop remained allocation-, lock-, trig-, lookup-, and I/O-free.
Rephase performs vector trig and the exact cap-sized finite sum at the block
boundary. State is 13 f64 vectors/values per four-lane bank plus cap, roughly
840 bytes per x8 pack before alignment and transition policy. Scalar was
explicitly allowed to remain on shipping.

## Exact command and workload

```text
cargo fmt
taskset -c 8 cargo test bounded_blit_x8_report --lib --release --locked -- --ignored --nocapture --test-threads=1
```

Stability covered 1024 coherent periods at 27.507, 440.367, and 6857.143 Hz.
It reports finite output, peak, global DC, last-cycle minus first-cycle DC, and
the largest correction imposed by authoritative rephase. CPU is the median of
five runs over 12,000 real 64-frame x8 blocks. Current calls
`accumulate_saw8_block`; candidate timing includes exact rephase, x8 generation,
and lane reduction/checksum.

## Stability and transition corrections

| Hz / cap | Integrator | Peak | DC | Cycle DC drift | Max rephase correction |
|---:|---|---:|---:|---:|---:|
| 27.507 / 872 | Euler | 1.536806 | ~0 | 0.029636 | 1.054014 |
| 27.507 / 872 | Trapezoid | 1.266927 | ~0 | 0.009751 | 0.252627 |
| 440.367 / 54 | Euler | 1.177195 | ~0 | -0.010693 | 1.053993 |
| 440.367 / 54 | Trapezoid | 1.164082 | ~0 | -0.013563 | 0.254911 |
| 6857.143 / 3 | Euler | 1.746729 | 2e-9 | 0.493279 | 1.188232 |
| 6857.143 / 3 | Trapezoid | 0.860508 | 2e-9 | 0.465510 | 0.465510 |

The scheme fixes round 8's runaway DC: every run stays finite and total DC is
effectively zero. It does so by imposing discontinuous block corrections.
Even trapezoidal integration requires 0.25 low/mid corrections and 0.47 at the
high-note gate. Euler exceeds 1.0. These are audible click-sized events, not a
small rounding residual.

Changing a settled 440 Hz/cap-6 bank to an exactly reprised 7040 Hz/cap-3 bank
measured a 1.030091 sample jump. Carrying the old bank is invalid because its
rotations, BLIT scale, and integrator correspond to the old pitch/cap. Therefore
pitch and cap transitions still require a dual-kernel fade.

## Actual x8 structural CPU

Nanoseconds per 64-frame x8 block:

| Cap / Hz | Current range | Euler | Trapezoid |
|---:|---:|---:|---:|
| 13 / 1777.778 | 346.988-503.639 | 1382.601 | 1422.606 |
| 6 / 3692.308 | 422.179-422.194 | 1126.677 | 1148.847 |
| 3 / 6857.143 | 557.146-558.037 | 1054.671 | 1040.897 |
| 2 / 9600 | 553.441-558.381 | 994.199 | 1001.923 |

Separate current timings were taken beside each integration variant, hence the
cap-13 spread; either endpoint leaves the verdict unchanged. The bounded
candidate costs 1.8-4.1 times current. Round 8's raw f32 SIMD win is fully erased
by the minimum precision and authoritative initialization needed for bounded
output, before adding a click-removing crossfade or direct-latency path.

## Verdict and limitations

Reject immediately: exact block rephase introduces large corrections and loses
the actual >=8-unison structural CPU gate at every tested cap. A shorter rephase
interval reduces numerical drift but raises exact-sum/trig cost; a longer one
increases corrections. A fade adds both kernels and cannot reverse the CPU
loss. Saw therefore does not advance to ideal wanted/alias analysis, shifted
square/pulse, or integrated triangle. No production state, object size,
publication cost, source, or RT behavior changed.
