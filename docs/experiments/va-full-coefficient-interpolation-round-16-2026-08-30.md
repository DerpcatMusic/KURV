# VA round 16: full coefficient interpolation code generation

## Verdict

Reject all production changes. LLVM already emits the minimal eight-chunk
AVX2/FMA kernel for the 256-byte `WaveCurveRt`. An explicit `f32x8` loop cannot
improve that code, while its portable implementation changes fused arithmetic
to multiply-then-add. Both endpoint returns can be bit-inexact; the zero
shortcut also misses the normal transition path.

Production remains 0.8.8. The object layout, interpolation, evaluators,
transition state, audio-thread work, and version are unchanged.

## Generated code

The probe compiled the exact shipping operation as a standalone symbol with
Rust 1.98.0/LLVM 22.1.8 at `-O`.

| target | shipping code | generated operation |
|---|---:|---|
| generic x86-64 | 316 bytes | 16-iteration loop, four scalar `fmaf` calls per iteration |
| AVX2 + FMA | 262 bytes | eight unrolled YMM loads, subtracts, FMAs, and stores |

The AVX2 form broadcasts `mix` once and has no coefficient loop or bounds
checks. It is already the explicit eight-`f32` chunking requested by the
experiment. Writing the same structure with `f32x8` adds source code but does
not change the useful machine operations. On the generic target, `wide`'s
`f32x8::mul_add` falls back to `(a * b) + c` without FMA; replacing scalar
`f32::mul_add` with it would violate the existing bit semantics.

## Endpoint semantics and call distribution

Across 64 curve pairs and nine mixes, 36,864 coefficient comparisons found:

- reproduced shipping loop: 0 bit failures;
- `t == 0` return: 0 corpus bit failures;
- `t == 1` return: 1,322 bit failures.

The `t == 1` shortcut is not algebraically interchangeable in floating point:
`current - previous` rounds before the fused multiply-add. It was rejected on
correctness before CPU could matter.

The corpus does not contain signed-zero coefficients. An adversarial valid
curve with 64 `-0.0` coefficients and positive coefficient deltas found 64/64
bit failures at `t == 0`: shipping FMA produces `+0.0`, while directly returning
the previous curve preserves `-0.0`. `WaveCurveRt` has no invariant excluding
signed zero, so the zero shortcut is not a universal bit-preserving replacement.

The remaining `t == 0` shortcut does not match the real transition duty:

- `WaveCurveTransition::value` bypasses interpolation at completion, while
  `fill_wave_curve_fades` advances before requesting a value, so an ordinary
  audible fade does not call interpolation at zero or one;
- `VaTableTransition::select` also bypasses completed transitions and advances
  before selection;
- positioned-table exact coincident anchors already return a frame directly;
- legacy-table selection can reach zero only at the upper terminal frame, where
  both selected frame indices are already the same.

That last terminal-table case is too narrow to justify an unconditional branch
in every interpolation. If it ever measures hot, the smaller fix is to return
the already-selected final frame in `VaTableRt::select_legacy`.

## CPU measurements

Release mode on a Ryzen 7 7800X3D, generic portable build. The table reports
medians from five alternating paired runs after one warm-up, pinned to CPU 5
with no other experiment compiling or timing. Each block advances phase over
64 host frames. Moving mixes stay strictly inside `(0, 1)` to model the
production transition calls; static mix is 0.37. Each table entry is shipping /
zero-endpoint candidate in nanoseconds per block.

| mix | evaluations/frame | scalar | x4 | x8 |
|---|---:|---:|---:|---:|
| static | 1 | 7806.2 / 7999.5 (+2.48%) | 7951.8 / 7984.0 (+0.40%) | 7882.9 / 7873.3 (-0.12%) |
| static | 4 | 8884.1 / 9117.2 (+2.62%) | 9760.1 / 9760.7 (+0.01%) | 10943.3 / 10944.1 (+0.01%) |
| static | 16 | 13207.3 / 13563.3 (+2.70%) | 18468.5 / 18454.1 (-0.08%) | 23954.2 / 23911.5 (-0.18%) |
| moving | 1 | 7821.8 / 8036.4 (+2.74%) | 7950.0 / 7990.3 (+0.51%) | 7866.6 / 7854.4 (-0.16%) |
| moving | 4 | 8879.1 / 9135.3 (+2.89%) | 9772.3 / 9787.6 (+0.16%) | 10967.1 / 10965.9 (-0.01%) |
| moving | 16 | 13173.1 / 13566.5 (+2.99%) | 18594.6 / 18584.5 (-0.05%) | 23985.2 / 23935.7 (-0.21%) |

The branch consistently costs 2.48-2.99% on the scalar consumer. All x4/x8
deltas are within -0.21% to +0.51%, too small to distinguish from ordinary
scheduling and frequency noise, and there is no representative-path win. One
preempted static duty-1 window was visible in an individual run; using the
five-run median prevents it from determining the result.

Isolated best-case cost demonstrates why the branch looked tempting but does
not establish a real-path win: at `t == 0`, full interpolation was 816.405 ns
and the direct return 19.674 ns. At interior `t == 0.37`, the medians were
817.628 / 817.219 ns. The exact-one candidate is invalid and was not retained.

## Code size

Adding only the zero-shortcut branch increased the standalone symbol:

| target | shipping | zero branch | increase |
|---|---:|---:|---:|
| generic x86-64 | 316 bytes | 348 bytes | 32 bytes |
| AVX2 + FMA | 262 bytes | 376 bytes | 114 bytes |

The AVX2 increase includes a second 256-byte copy path. It buys nothing in the
ordinary interior transition distribution and makes the hot symbol larger.

## Commands

```text
rustc -O --edition=2024 --emit=obj --emit=asm interpolate_codegen.rs
rustc -O --edition=2024 -C target-feature=+avx2,+fma \
  --emit=obj --emit=asm interpolate_codegen.rs
nm -S --size-sort interpolate-{generic,avx2}.o
```

```text
CARGO_TARGET_DIR=/tmp/kurv-va-interpolate3-target \
taskset -c 5 cargo test --release --no-default-features --lib --locked \
  wave_curve::compiler_experiment::wave_curve_interpolation_report \
  -- --ignored --nocapture --test-threads=1
```

The manual report passed 1/1 while proving both endpoint shortcuts invalid as
universal bit-preserving replacements.
