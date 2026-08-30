# Support-three Estrin scheduling round 12 (rejected)

Date: 2026-08-30

Baseline: `136679f` (production DSP unchanged)

Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, CPU 8

Verdict: rejected; no runtime or benchmark code retained

## Candidate

Round 11's degree-seven, three-sample residual was the quality winner but its
Horner chain left the x8 high-note block 13.8% slower than current. This round
kept exactly the same coefficients, support, odd sign, endpoint constraints,
event masks, phase normalization, and block plumbing. Only polynomial schedule
changed.

The Estrin form groups coefficient pairs, reuses `d2 = d*d` and `d4 = d2*d2`,
then combines four pair FMAs through two parallel dependency chains. It uses
four coefficient-pair FMAs, two combine FMAs, two power multiplies, and a final
add embodied by the outer FMA, reducing serial dependency depth versus seven
Horner FMAs. Scalar, x4, and x8 forms were evaluated. There is no additional
state, memory, lookup, allocation, lock, I/O, or publication.

## Exact commands and inspection

```text
cargo fmt
taskset -c 8 cargo test support3_estrin_cpu_report --lib --release --locked -- --ignored --nocapture --test-threads=1
taskset -c 8 cargo test support3_estrin_cpu_report --lib --release --locked -- --ignored --nocapture --test-threads=1 | rg 'support3_estrin_(equivalence|cpu|block)'
bin=target/release/deps/pure_va_dispersion_core-dacde528ceb2d1c0
nm -C "$bin" | rg 'support3_estrin|local_blep'
objdump -Cd "$bin" | rg -n -m 4 'support3_estrin_cpu_report|vfmadd|vmulps'
```

Timing used seven medians over one million scalar samples, 500,000 SIMD
vectors, and 16,000 64-frame x8 stereo blocks. The release binary was stripped,
so `nm` exposed no candidate symbol and the inlined loop could not be isolated
by name. `objdump` confirms native `vfmadd*` instructions elsewhere/in the
binary, but this is not claimed as a candidate-specific instruction count.
Source-level operation scheduling and measured cycles remain the evidence.

## Numeric equivalence and retained quality

One million evenly spaced residual positions measured maximum Horner-versus-
Estrin difference `1.4273e-5`. Coherent saw results:

| Hz | Schedule | RMS | Peak | Boundary residual | DC |
|---:|---|---:|---:|---:|---:|
| 27.507 | Horner | 0.004291478 | 0.066186821 | 0.116247896 | 1e-9 |
| 27.507 | Estrin | 0.004291478 | 0.066186821 | 0.116247896 | 2e-9 |
| 440.367 | Horner | 0.017156064 | 0.066270683 | 0.116443753 | 1.7e-8 |
| 440.367 | Estrin | 0.017156054 | 0.066270683 | 0.116443753 | 6e-9 |
| 6857.143 | Horner | 0.051165682 | 0.091395594 | 0.182778866 | -1.69e-6 |
| 6857.143 | Estrin | 0.051162884 | 0.091384091 | 0.182767362 | 3.60e-7 |

Estrin retains the quality win within the requested tight practical tolerance;
rounding is slightly favorable at the high coherent probe. It is stateless, so
round 11's transition behavior and eligibility remain unchanged.

## CPU

First pinned run, ns per scalar sample or SIMD vector:

| Hz | Scalar current / Horner / Estrin | x4 current / Horner / Estrin | x8 current / Horner / Estrin |
|---:|---:|---:|---:|
| 440 | 3.960 / 5.956 / 6.020 | 2.558 / 3.104 / 3.012 | 4.169 / 6.071 / 5.484 |
| 7040 | 6.375 / 14.259 / 14.282 | 4.365 / 5.710 / 5.056 | 9.009 / 10.191 / 9.251 |

Estrin reduces x4/x8 cost versus Horner, especially x8, but scalar is unchanged
or slightly worse and every raw lane still loses current.

Actual 64-frame x8 stereo block timings:

| Run | Hz | Current | Horner | Estrin | Estrin vs current |
|---:|---:|---:|---:|---:|---:|
| 1 | 440 | 299.257 | 400.844 | 355.500 | +18.8% |
| 1 | 7040 | 564.265 | 658.512 | 568.316 | +0.7% |
| 2 | 440 | 302.594 | 402.449 | 371.106 | +22.6% |
| 2 | 7040 | 638.381 | 868.282 | 615.590 | -3.6% |

The high-note block is effectively at the noise boundary: one run narrowly
loses and one wins while the current measurement itself moves 13%. The low-note
block consistently loses by 19-23%, and the scalar/x4 requirements also lose.
This is not the required uniform `<= current` real-CPU result.

## Decision

Reject production promotion. Estrin is the correct compiler-friendly schedule
for this fitted polynomial and closes the interesting high-x8 gap, but it does
not make the support-three backend Pareto-safe across the required lanes and
structural cells. No handwritten assembly or further degree-six variant was
justified. Current optimized cubic remains shipping; all experiment code was
reverted.
