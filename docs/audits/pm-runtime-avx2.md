# Runtime AVX2 for phase-modulated saw blocks

## Outcome

Baseline `d084681411a95803bb52206647c2bc881c4cbf8b` has a runtime AVX2/FMA backend for ordinary saw blocks, but the corresponding PM blocks stay in the build's baseline SIMD implementation. This change gives the existing PM saw stereo and lane-accumulator blocks a runtime AVX2/FMA backend using the existing spline correction coefficients. The generic shape renderer also dispatches its narrow saw case to the new backend.

No modulation equation, antialiasing mode, sample rate, latency, or modulation depth changes. This is a rendering-cost fix, not an aliasing-quality improvement. Sine PM, other shapes, arbitrary waveform PM, and per-sample-changing carrier steps are outside this change.

Dispatch requires the existing published AVX2/FMA backend selection, at least 16 frames, and an applicable narrow saw route (carrier step below 0.25). Builds globally compiled for AVX2+FMA retain their previous kernel, because explicit intrinsic dispatch did not consistently improve those builds. Four-lane rendering, unmodulated lane accumulation, other architectures, and unsupported CPUs retain their existing rendering.

## Measured result

AMD EPYC 9V74 KVM guest, CPU 2 affinity, Rust 1.97.1 release, wide 0.7.33, no LTO. Each pair uses the same executable, input, and loop counts; sequential ABBA runs. Median candidate/baseline cost ratios across 18 cases per cell:

| Total lanes | Stereo accumulation | SIMD lane accumulation |
|---|---:|---:|
| 8 | 0.559 (44.1% less) | 0.534 (46.6% less) |
| 16 | 0.558 (44.2% less) | 0.533 (46.7% less) |
| 64 | 0.585 (41.5% less) | 0.520 (48.0% less) |

These are actual extracted PM block renderers with 64 frames, repeated over independent eight-oscillator groups. They are not whole-plugin CPU percentages. Each cell covers carrier steps 0.0001, 0.01, 0.12; PM depths 0, 0.49, 4; both spline coefficient sets. Depth 4 deliberately also characterizes the existing bounded-wrap implementation beyond its usual input range; this change does not repair that contract.

Across the changed portable-build rows, observed cost reductions range approximately 11–66%. Native-build unchanged routes center around a cost ratio of 1.0. Unchanged four-lane controls have noisy individual outliers up to 27%; small isolated gains should not be treated as reliable. The much larger repeated x8 improvements survive that noise. Machine-wide or end-to-end claims require another machine and the full plugin build.

## Correctness

The harness extracts the actual committed baseline and current production functions, including the runtime intrinsic functions and their existing BLEP helper. Only oscillator storage and backend selection are replaced by minimal adapters; baseline and candidate receive identical phase/step/modulation/gain/accumulation inputs.

46,080 block cases cover 1/16/64 frames, x4/x8 stereo and x8 lane accumulation, eight step values including zero and near the narrow support boundary, five modulation depths, 32 initial phase seeds, both spline modes and optional modulation. Final oscillator phases are bit-identical. Peak output difference is `2.3841858e-7` in the portable build, below the `2e-6` acceptance bound. Globally native output is bit-identical. Additional per-lane checks start from nonzero accumulation buffers and check each lane individually to avoid cancellation hiding an error.

Existing fused multiply-add use in the intrinsic polynomial explains tiny output differences from the baseline portable polynomial. No FFT quality improvement is claimed. Source-extraction tests do not compile the complete plugin or independently prove generic-renderer routing; full VA integration and the private authorization dependency remain separate validation requirements.

The combined review branch additionally passes `tools/audits/pm_integration`:
56 generic PM cases versus actual scalar sampling plus 448 canonical route cases
under both baseline and AVX2 backend selection. Complete VA modules compile with
only host serialization adapters removed. The full voice/host build is still
outside that validation boundary.

## Reproduction

From the repository root:

```sh
cargo +1.97.1 run --offline --release --manifest-path tools/audits/pm_kernel/Cargo.toml
RUSTFLAGS='-C target-cpu=native' CARGO_TARGET_DIR=tools/audits/pm_kernel/target/native cargo +1.97.1 run --offline --release --manifest-path tools/audits/pm_kernel/Cargo.toml
# Fourth CLI argument is the number of independent oscillator groups.
taskset -c 2 tools/audits/pm_kernel/target/release/kurv-pm-kernel-proof --bench baseline 8
taskset -c 2 tools/audits/pm_kernel/target/release/kurv-pm-kernel-proof --bench candidate 8
```

Run baseline/candidate/candidate/baseline sequentially for each group count 1, 2, 8. CSV `width=4` means four-lane stereo, `width=8` means eight-lane stereo, and `width=16` is a historical harness selector for **eight-lane SIMD accumulation**, not sixteen oscillators. Multiply actual lanes by the group count in the filename. `ns_per_block` includes all groups. The final measured files start `final-`; earlier files describe exploratory candidates and must not be pooled with the final set.

## Rejected work

`tools/audits/pm_kernel/rejected/reciprocal-hoist.patch` preserves an earlier phase-step reciprocal/support-hoisting candidate. Specialized kernels passed 46,080 bit-exact cases, but sparse cases regressed and native improvement was marginal. The rejected patch additionally contains an unmeasured generic saw extension. The earlier `portable-*` and `native-*` CSV files characterize that rejected candidate. Initial `avx2-*` files characterize stereo-only native dispatch and an explicit SIMD gain multiplication experiment; the latter regressed portable x4 rendering and was reverted. None of these preliminary timing sets establish the final implementation's performance.
