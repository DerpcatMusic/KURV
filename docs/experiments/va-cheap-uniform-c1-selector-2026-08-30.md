# Cheap uniform C1 wave-curve selector (2026-08-30)

## Outcome

Ship in 0.8.6. `WaveCurveData::compile_rt` now compiles the legacy curve first and
accepts a constrained shared-slope C1 candidate only when a bounded, deterministic
offline proof shows a material source-shape win without observed range, knot,
discontinuity, derivative, or ideal-bandlimited regression.

The runtime contract is unchanged: `WaveCurveRt` remains 64 `f32` coefficients
(256 bytes), 16 uniform cubic segments, and uses the existing `eval`, `eval4`, and
`eval8` paths. Compilation and selection happen before publication; the audio
callback only reads the published object.

## Selector

The compile-only selector uses 256 uniform source probes and analytic cubic
extrema/derivative checks. It accepts the candidate only when all gates pass:

- source squared error is at most 0.5625 of legacy (at least 25% RMS reduction);
- peak source error is no worse than legacy plus `1e-7`;
- every explicit editor knot is no worse than legacy plus `1e-6`;
- analytic clamp crossings and overshoot are no worse plus `1e-6`;
- artificial derivative jumps are no worse plus `1e-3`;
- an intentional hard-point or wrap jump retains at least 95% of the legacy jump.

The candidate compiler uses fixed stack storage and the existing 16 uniform
segments. It introduces no runtime metadata and no evaluator work. Legacy remains
the deterministic fallback and therefore preserves preset behavior whenever the
candidate cannot prove itself.

## Seeded 512-curve oracle

The corpus covers smooth curves, explicit hard points, clustered/tight knots,
near-duplicate phases, extrema/overshoot, wrap discontinuities, maximum 16-knot
curves, and random musical/adversarial curves. The production bounded selector was
checked against an 8192-sample source oracle and ideal-bandlimited FFT comparison.

- selected: 64/512 (12.5%);
- selection distribution: 57 near-duplicate, 7 random, 0 other categories;
- full-oracle source regressions: 0;
- strict ideal-bandlimited regressions (`> 0.0 dB`): 0;
- selected full-oracle RMS reduction, min/median/max:
  31.159708% / 77.173793% / 96.588389%;
- worst-of-three ideal-bandlimited delta, min/median/max:
  -29.341117 / -12.826456 / -1.801953 dB (negative is better).

Looser threshold combinations selected more curves but produced strict
ideal-bandlimited regressions. The retained threshold was the cheapest tested gate
with meaningful selection and zero observed regression.

Candidate-only compile cost in the same release corpus run was 10.840 us median,
versus 0.927 us for legacy. Selection adds a bounded 256-probe proof; it does not
run the dense or FFT oracle in production. The extra work is compile/editor-side,
never per audio sample.

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::cheap_uniform_least_squares_c1_selector_sweep \
  --locked -- --ignored --nocapture
```

Passed 1/1. Selected 64/512; zero dense-source and strict ideal-bandlimited
regressions. The test also proves the production candidate coefficients and
selector decision equal the experiment implementation across the corpus.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::seeded_uniform_least_squares_c1_property_report \
  --locked -- --ignored --nocapture
```

Passed 1/1 in 19.71 s. Processed all 512 cases. Candidate median 10839.5 ns;
legacy median 927.0 ns.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_1x_va_quality_and_cpu_report \
  --locked -- --ignored --nocapture
```

Passed 1/1 in 4.39 s. The representative drawn curve correctly fell back to
legacy and retained the previous results. At 48 kHz its 1x low/mid/high
alias/error readings were -60.600/-37.189/-8.749 dB at 9.751 ns/sample; 2x was
-45.587/-39.039/-14.218 dB at 37.676 ns/sample. Saw CPU was 7.252 ns/sample at
1x and 31.710 ns/sample at 2x.

```text
cargo test --release --no-default-features --lib --locked
```

350 passed, 6 failed, 9 ignored in 3.31 s. All wave-curve topology tests passed.
The current checkout also reported failures outside this change's wave-curve path:

- `oscillators::resynth::tests::rich_low_note_post_zone_retains_upper_spectrum`
- `oscillators::resynth::tests::all_three_algorithms_compile_to_distinct_production_artifacts`
- `tests::structural_lfo_source_advances_during_process`
- `voices::voice::internal_rt_pool::tests::partitioned_render_waits_for_three_helpers_and_matches_serial_bits`
- `voices::voice::internal_rt_pool::tests::coarse_jobs_null_for_off_wander_and_jitter`
- `voices::voice::internal_rt_pool::tests::unsupported_and_release_states_fall_back_without_stale_jobs`

These failures are recorded as current-checkout evidence only; this experiment did
not establish whether they predate the change.
