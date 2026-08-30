# Centered phase-domain PTR3 experiment (2026-08-30)

## Decision

Reject centered PTR3 as KURV's canonical saw/square/pulse replacement and do
not proceed to production or assembly. It improves aligned ideal-reference RMS
at every measured pitch, but saw, square, and pulse are slower than current in
every x8 row across three runs. The largest candidate/current ratios are
`1.467x`, `1.420x`, and `1.456x`; low-note peak or immediate-transition behavior
also regresses for each family.

Centered EPTR2 triangle is very fast and accurate, but it is mathematically the
phase-aligned DPW2 triangle already recorded by the factored-DPW second shot and
retains that round's pitch/reset transition regression. It is not a new reason
to integrate the wider PTR3 family.

The temporary ignored measurement harness was removed after capture. This
negative record is the only retained change. Production DSP, oscillator state,
dependencies, presets, and the package version are unchanged.

## Why this is a distinct second shot

Baseline commit: `214b75ef90edbcc420af2d05bdfc6c4bb4d823e7` (`Reject support-two
equiripple BLEP`). Baseline algorithm: current VA `SplineOptimized` at 1x
oversampling.

The local archive was audited before choosing the candidate:

| Existing family | Covered rounds | Why this round does not repeat it |
|---|---|---|
| Recursive/bounded BLIT and exact/additive crossover | recursive round 8, bounded round 9, additive round 4, exact-Fourier rounds | Those synthesize or select harmonics; PTR evaluates a bounded phase-domain polynomial only near a corner. |
| Local support polynomials | local BLEP round 11, support3 Estrin/crossover/assembly/reciprocal/high-note rounds 12-16, support-two equiripple round 23 | Those correct a raw discontinuity with fitted/equiripple residuals. This candidate evaluates the analytic finite-difference waveform directly and has a known `sinc^3` transfer. |
| Sparse/event residuals | analytic iterator, canonical schedule, MinBLEP ring, branchless/crossover and custom derivative/windowed-sinc/elliptic event rounds | PTR has no event queue, future ring, residual table, event iterator, or retained tail. |
| DPW | direct DPW2/DPW4 round 6 and factored DPW2/DPW3/DPW23 second shot | This is the DPW-adjacent frontier: the DPW4-equivalent result is reduced to three local cubic regions centered on the phase being published. It avoids polynomial history and low-frequency subtraction cancellation. |
| Triangle paths | additive ownership round 18 and integrated corrected-square round 19 | Triangle uses the centered improved-PTR quadratic corner itself, with no integration state or harmonic loop. |
| Static warp, coefficient banks, seams, selectors, AVX-512 and block preparation | all recorded 2026-08-30 backend rounds | Those may remain useful backend ideas, but none is another oscillator formula and none is folded into this minimal probe. |

## Primary references

- Vesa Välimäki and Jussi Kleimola,
  [Reducing Aliasing from Synthetic Audio Signals Using Polynomial Transition Regions](https://doi.org/10.1109/LSP.2011.2177819),
  IEEE Signal Processing Letters 19(2), 2012. It establishes finite polynomial
  transition regions and their equivalence to differentiated polynomial
  waveforms.
- The authors' archived
  [PTR publication page and reference Python source](https://web.archive.org/web/20220122041130id_/http://research.spa.aalto.fi/publications/papers/spl-ptr/).
  The source was used to check the W=3 segment boundaries, coefficients, and
  pulse construction.
- Dániel Ambrits and Balázs Bank,
  [Improved Polynomial Transition Regions Algorithm for Alias-suppressed Signal Synthesis](https://doi.org/10.5281/zenodo.850287),
  DAFx-17. It shows that the transition can be centered to remove PTR's phase
  offset while retaining the raw linear region and exact DPW-equivalent result.

The measured Rust formulas were an independent minimal implementation. No
source was copied into KURV and no dependency was added.

## Candidate

For phase `p`, positive phase step `h`, and signed wrap distance `d`, define
`u = d / h + 1.5`. The saw returns raw `2p - 1` outside the three-sample-wide
transition. Within it, the three cubic regions are:

```text
0 <= u < 1: ((-u^2 / 3 + 2h)u + 1 - 3h)
1 <= u < 2: (((2u / 3 - 3)u + 3 + 2h)u - 3h)
2 <= u < 3: (((-u / 3 + 3)u + 2h - 9)u + 8 - 3h)
```

Centering makes the value correspond to the current published phase with zero
extra latency. It is algebraically the centered third finite difference of the
periodic fourth-order DPW polynomial, but does not retain earlier polynomial
values or subtract nearly equal large terms.

Square and pulse are differences of two centered PTR saws plus the exact DC
term `2w - 1`. Width is clamped with the same one-edge-separation rule used by
the probe's scalar/x8 paths. Triangle uses the centered EPTR2 quadratic corner:

```text
trough: -1 + h + 4d^2/h,  |d| < h/2
peak:    1 - h - 4d^2/h,  |d| < h/2
```

The W=3 saw is valid while the periodic transition does not overlap itself,
`0 < h < 1/3`. All scheduled pitches are within that range; the probe explicitly
falls back to the raw saw outside it rather than pretending the formula remains
valid.

Both scalar and f32x8 implementations are stateless beyond KURV's existing
phase. They are bounded O(1), allocate zero bytes, and perform no lock, I/O,
logging, resizing, table lookup, or history update. The SIMD reciprocal is
hoisted once per block.

## Measurement contract

The temporary ignored
`centered_ptr_canonical_quality_transition_and_cpu_report` probe checked:

- scalar PTR3 against a direct f64 centered DPW4 finite difference;
- centered triangle against the phase-aligned DPW2 triangle;
- scalar/f32x8 agreement for saw, square, pulse, and triangle;
- bit-exact x8 phase publication after a real 32-frame block;
- ideal band-limited curve RMS/peak at periods 1745, 109, and 7;
- known wanted transfer (`sinc^3` for PTR3, `sinc` for EPTR2), folded numeric
  alias, off-grid artifacts, total legal-bin error, DC, gain, and boundary
  residual;
- immediate pitch, hard phase reset, and PWM changes on the repeating schedule
  `440x24 | 7040x32 | 110x24 | 12000x32`;
- real scalar and x8 stereo accumulation at 24- and 32-frame internal blocks,
  at 440 and 7040 Hz, against production `SplineOptimized` seams before the
  common oversampler.

The quality path sent both baseline and candidate through the same factor-1
`StereoOversampler`. Raw candidate samples are analyzed separately so common
output alignment is not mislabeled as oscillator alias or transfer error.

## Correctness and static gates

The release build with `-C target-cpu=native` completed successfully. All
formula and runtime contracts passed on three executions:

- PTR3 versus direct f64 centered DPW4 at four fractional edge offsets and
  periods 1745, 109, and 7;
- EPTR2 triangle versus phase-aligned DPW2;
- scalar/f32x8 parity and finite output for all four shapes;
- bit-exact x8 phase publication after a real 32-frame accumulation.

Each execution reported `1 passed; 0 failed; 400 filtered out`. Compilation
emitted only the checkout's pre-existing unused/dead-code warnings. `cargo fmt
--all`, `git diff --check`, and the staged diff check also passed.

## Ideal-reference result

Aligned RMS error is in dB; more negative is better.

| Wave | 27.5 Hz current -> PTR | 440.4 Hz | 6857.1 Hz |
|---|---:|---:|---:|
| Saw | -30.761 -> -31.028 | -22.289 -> -23.848 | -9.614 -> -11.235 |
| Square | -32.572 -> -32.950 | -24.405 -> -26.708 | -11.104 -> -13.317 |
| Pulse 31% | -32.526 -> -32.809 | -24.130 -> -25.922 | -12.202 -> -14.487 |
| Triangle | -57.342 -> -57.343 | -51.640 -> -61.445 | -15.670 -> -26.490 |

PTR3's known `sinc^3` response reduces folded alias cleanly, but attenuates
wanted high harmonics. The aligned curve still improves modestly because the
current 1x spline error is larger. This is not a universal quality win: at
27.5 Hz, saw peak error rises from `0.752724` to `0.776956` and square from
`1.055902` to `1.097584`. Pulse peak improves there.

## Immediate-transition result

| Wave / metric | Current | PTR | Outcome |
|---|---:|---:|---|
| Saw global maximum step | 0.928833 | 1.036901 | worse 11.6% |
| Square PWM-event step | 1.105341 | 1.312778 | worse 18.8% |
| Pulse pitch-event step | 1.348644 | 1.545186 | worse 14.6% |
| Triangle pitch-event step | 0.896978 | 1.000000 | worse 11.5% |
| Triangle reset/global step | 1.949844 | 1.960667 | slightly worse |

Hard phase reset is immediate and has no stale history, as expected for a
stateless formula. The regressions come from instantly changing the
step-dependent polynomial shape, not from retained state.

## Coarse 24/32-frame CPU result

The table spans medians from three adjacent pinned executions, with five timing
repetitions per workload. Ratios are candidate/current; below one is faster.

| Wave | Scalar ratio range | x8 ratio range | Decision |
|---|---:|---:|---|
| Saw | 0.811-1.088x | 1.036-1.467x | reject |
| Square | 1.018-1.359x | 1.039-1.420x | reject |
| Pulse 31% | 0.853-1.311x | 1.061-1.456x | reject |
| Triangle | 0.562-0.891x | 0.268-0.595x | duplicate DPW2 frontier |

Measurements used real stereo scalar and x8 accumulation seams before the
common oversampler, at 440 and 7040 Hz for both 24- and 32-frame blocks. They
were pinned to logical CPU 6 on the Ryzen 7 7800X3D while Bitwig and its audio
engine were open at low activity. Near/parity/apparent wins are therefore
provisional. The rejection does not depend on those uncertain rows: every
saw/square/pulse x8 row lost in all three runs, and the high-note losses were
often 20-47%.

## Historical execution

The temporary probe was built and run with:

```bash
env CARGO_TARGET_DIR=/tmp/kurv-va-ptr-target \
  CARGO_BUILD_JOBS=1 RUSTFLAGS='-C target-cpu=native' \
  cargo test --release --no-default-features --lib --locked \
  oscillators::va::experiment::centered_ptr_canonical_quality_transition_and_cpu_report \
  --no-run

taskset -c 6 /tmp/kurv-va-ptr-target/release/deps/pure_va_dispersion_core-4541c32e2a044c33 \
  oscillators::va::experiment::centered_ptr_canonical_quality_transition_and_cpu_report \
  --exact --ignored --nocapture --test-threads=1
```

The fresh single-job build took 7m15s; each report took about 2.32s. No further
pristine timing run is warranted because the universal candidate already fails
large x8, peak, and transition gates. Writing assembly would optimize a rejected
architecture rather than change those signal-quality failures.
