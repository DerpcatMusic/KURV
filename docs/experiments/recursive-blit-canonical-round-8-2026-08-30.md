# Recursive BLIT saw round 8 (rejected)

Date: 2026-08-30

Baseline: `f6d9495` (production DSP unchanged)

Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, CPU 8

Verdict: rejected at the saw micro gate; no runtime or benchmark code retained

## Candidate

The test-only candidate used the classic odd periodic-sinc BLIT

`sin((2N + 1) pi phase) / sin(pi phase)`

with `N` equal to the legal harmonic cap. The numerator and denominator each
used a complex rotation prepared once at note setup; no sample-loop trig,
allocation, locking, I/O, or coefficient lookup occurred. Near exact denominator
zeros the analytic `(2N + 1)` limit was substituted.

Two minimum integration variants were measured:

- forward Euler: `saw += -2 step (BLIT - 1)`;
- trapezoidal: `saw += -step (BLIT[n] + BLIT[n-1] - 2)`.

Startup was analytically primed from the exact finite saw sum. Scalar, `f32x4`,
and `f32x8` states carried the two sine/cosine rotations, previous BLIT sample,
integrator, step, and harmonic scale: ten scalar values per lane before any
pitch-transition or DC-management policy.

## Exact command and workload

```text
cargo fmt
taskset -c 8 cargo test recursive_blit_saw_gate_report --lib --release --locked -- --ignored --nocapture --test-threads=1
```

Quality used 1024 coherent periods at 48 kHz for periods 1745, 109, and 7.
It reports shared ideal-projection RMS/peak, total DC, and last-cycle minus
first-cycle DC. CPU is the median of five runs over one million scalar samples
or 500,000 SIMD vectors. Current calls the real canonical scalar/x4/x8 saw
kernels.

## Quality, DC, and drift

| Hz / cap | Euler RMS / peak | Euler DC / cycle drift | Trapezoid RMS / peak | Trapezoid DC / cycle drift |
|---:|---:|---:|---:|---:|
| 27.507 / 872 | 437.576 / 817.932 | 382.889 / 718.667 | 442.538 / 832.410 | 386.079 / 732.457 |
| 440.367 / 54 | 130184.428 / 331130.769 | -96843.109 / -291331.553 | 130185.223 / 331133.783 | -96843.738 / -291330.152 |
| 6857.143 / 3 | 1175.529 / 2316.247 | -1015.833 / -2038.686 | 1176.136 / 2316.883 | -1016.539 / -2038.522 |

The two recursive oscillators slowly lose their shared phase/norm in `f32`.
The quotient is ill-conditioned near every denominator zero, turning small
rotation error into large impulses that the integrator permanently accumulates.
The analytic zero limit only handles exact zeros and therefore does not bound
near-zero error. This fails the requested DC/drift gate by orders of magnitude.

A leaky integrator can bound the accumulated output, but cannot repair the
incorrect impulse amplitude and necessarily attenuates/tilts wanted low
harmonics. Credible alternatives require periodic authoritative rephasing
(including trig for noninteger-period residual phase), frequent normalization,
or wider arithmetic. Those add work while scalar already loses the CPU gate.

## CPU micro gate

Each cell is `current / Euler / trapezoid`, in median ns per scalar sample or
SIMD vector.

| Cap / Hz | Scalar | x4 | x8 |
|---:|---:|---:|---:|
| 13 / 1777.778 | 4.263 / 9.700 / 9.679 | 2.953 / 2.153 / 2.174 | 5.884 / 4.785 / 4.990 |
| 6 / 3692.308 | 4.928 / 11.274 / 9.532 | 3.454 / 2.192 / 2.235 | 7.250 / 5.331 / 4.985 |
| 3 / 6857.143 | 6.726 / 9.630 / 10.363 | 4.568 / 2.330 / 2.269 | 9.059 / 4.866 / 5.916 |
| 2 / 9600 | 7.917 / 9.782 / 9.521 | 4.322 / 2.192 / 2.235 | 9.128 / 4.817 / 4.945 |

The raw vector recurrence is fast, but scalar loses in every row before the
stability repairs above. SIMD speed therefore cannot promote the candidate.

## Startup, pitch, and harmonic-count transitions

Analytical phase-zero startup produced exactly zero. After settling the
trapezoidal cap-6 state, changing from 440 Hz/cap 6 to an analytically reprised
7040 Hz/cap 3 state differed by 0.801828. Carrying old rotations is invalid;
instant reprime is itself a large discontinuity and would require a dual-kernel
transition.

The exact waveform changes at cap crossings remain:

| Cap change | Peak omitted harmonic |
|---:|---:|
| 13 to 12 | 0.048970759 |
| 6 to 5 | 0.106103301 |
| 3 to 2 | 0.212206602 |
| 2 to 1 | 0.318309903 |

Thus statically correct BLIT coefficients do not solve swept cap transitions.

## Verdict and limitations

Saw fails quality, DC/drift, transition, and scalar CPU gates. Structural
block/unison/factor-2 timing cannot rescue it after adding rephasing, DC, and
transition costs, so it was not run. Shifted-BLIT square/pulse and integrated
triangle were not built. No oscillator state, object size, publication cost,
production source, or RT behavior changed.
