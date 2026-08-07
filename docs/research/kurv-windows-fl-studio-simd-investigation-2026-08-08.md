# KURV Windows/FL Studio load and SIMD investigation — 2026-08-08

## Conclusion

KURV should not ship only the current `x86-64-v3` Windows build. FL Studio's current published Windows requirements list SSE2, not AVX2, as the processor baseline. A v3-only KURV binary can therefore fail on a CPU that FL Studio itself supports. This is a credible explanation for the Fruity Wrapper load error, although it is not proven without the user's CPU and loader log.

The immediate distribution choice is a baseline x86-64 Windows build. It retains KURV's existing `wide`/`truce-simd` fallback behavior and is the safest single binary. Later, measured hot kernels can use Rust's runtime feature detection and per-function `target_feature` dispatch inside that baseline binary.

SIMDeez is technically capable of generating scalar/SSE2/SSE4.1/AVX2/AVX512/Neon/WASM implementations and selecting an implementation at runtime. It is not a good whole-KURV replacement today: KURV is already built around concrete `truce_simd::simd::f32x4`/`f32x8` values, and several algorithms change data layout or algorithm shape with compile-time AVX2/FMA cfgs. Replacing that with SIMDeez would be a large DSP refactor with numerical and audible-risk surface. SIMDeez is also documented as Beta and its current runtime macro checks/selects in the generated invocation path rather than giving KURV a free one-time render-function pointer.

The Alex Heretic and Nick Wilcox articles describe useful runtime-dispatch patterns, but they are explanatory references, not dependencies. The Rust standard library's `std::arch` APIs are the appropriate implementation primitive. The old `packed_simd` RUSTFLAGS page explains compile flags but does not provide runtime dispatch. Nightly `core::simd`/`std::simd` is portable SIMD, not a complete “one binary chooses the best CPU” solution, and should not be introduced into the release path for this fix.

`simdscan` is useful as a post-build audit: it disassembles an x86-64 ELF/PE/Mach-O binary and classifies instruction mnemonics. It cannot choose a CPU path, prove that an instruction is reachable, or benchmark the audio callback. Its own project lists function-level SIMD analysis as future work, so it should remain a developer-side inspection tool rather than a runtime dependency.

The newer `fearless_simd` project is a more relevant future refactor candidate than SIMDeez: it provides a `Level::new()` runtime capability token and a `dispatch!` API for generic kernels, with fallback/SSE4.2/AVX2+FMA/NEON levels. It still requires rewriting KURV's concrete `wide` functions as generic `Simd` kernels, so adopting it wholesale now would have the same large numerical/audibility risk. It is worth prototyping around one isolated kernel after the baseline shipping fix.

## Current KURV evidence

- `Cargo.toml` pins Truce 6.3.0, `truce-simd = 6.3.0`, and `wide = 0.7.33`.
- `truce-simd` uses the stable `wide` backend. KURV's main render paths use concrete `f32x4` and `f32x8` values in `src/voice.rs`, `src/oscillator.rs`, and `src/oversampling.rs`.
- `src/wave_curve.rs` has compile-time AVX2/FMA branches. The AVX2 branch uses a transposed coefficient layout, while the fallback uses a different layout. A runtime AVX2 call cannot be added without making the data representation valid for both paths.
- `src/oscillator.rs` has compile-time AVX2 branches for spectral saw generation, spectral lookup, and BLAMP residual work. Some AVX2 paths are algorithmically different from the fallback, so generic runtime dispatch is not a mechanical crate flag change.
- `cargo truce` documents `v3` as `x86-64-v3` and `baseline` as the rustc default. The existing release script builds both tiers, but the old tester artifact was a v3-oriented package.
- The current diagnostic builds were made from the checkout being investigated, including the Windows lifecycle logger. The v3 build is not a compatibility substitute for the baseline build.

## One-binary architecture if optimization is needed

Build the whole distributed DLL for the baseline x86-64 target. At an outer initialization seam, select a function pointer once using `std::is_x86_feature_detected!`. Put the optimized implementation in a function with `#[target_feature(enable = "avx2", enable = "fma")]`, and keep the baseline implementation callable on every supported CPU. The selected function must never be called on a CPU lacking its declared features. Keep all shared state and coefficient layouts compatible with the baseline build; do not use crate-wide `cfg(target_feature = "avx2")` for state that must be shared by both variants.

The likely KURV ladder is baseline/SSE2, then SSE4.1 or SSE4.2 only where measured, then AVX2+FMA. AVX512 should not be assumed to be faster for real-time audio because wider code can cause frequency throttling. The dispatch table should be added only around a measured bottleneck, not around the entire `VaVoice`/plugin render graph.

`target-cpu=native` is local-machine optimization and is not suitable for a distributed plugin. A portable binary may contain instructions for optimized functions as long as those functions are unreachable on unsupported CPUs; a raw byte scan for AVX2 is not by itself a valid compatibility test.

## Windows diagnostic boundary

The new Windows logger writes flushed lifecycle markers to `%LOCALAPPDATA%\\KURV\\Logs\\KURV-<pid>.log` and also sends them to `OutputDebugStringA`. It records the module path, process/thread IDs, compile-time and runtime CPU features, DSP construction, state migration, editor lifecycle, and internal RT-helper startup/drop boundaries. It does not write from the realtime `process()` path.

Useful interpretations:

- No KURV log: failure is likely before Rust state creation, such as bundle discovery, missing DLL, loader/export failure, or an unsupported instruction reached during module initialization.
- `startup` but no `dsp-default-enter`: failure is in initial Rust/plugin entry setup.
- `dsp-default-enter` but no `rt-pool-new...`: failure during DSP construction before the helper pool.
- `rt-pool-new-enter` with a zero/partial `new-return` mask: Windows helper creation or priority registration is involved.
- Complete editor markers but wrapper still errors: investigate state migration, host state, or a host/editor boundary rather than CPU dispatch.

The real host remains the final gate. FL Studio must be restarted after replacing the complete `.vst3` bundle, then its plugin manager should rescan plugins with errors. The baseline and v3 packages should be tested separately on the affected machine; if baseline opens and v3 fails, the CPU ISA mismatch is confirmed.

## Primary references

- [Image-Line FL Studio download/system requirements](https://www.image-line.com/fl-studio/download/)
- [Image-Line plugin error guidance](https://support.image-line.com/action/knowledgebase?ans=887)
- [Truce processing model](https://truce.audio/docs/guide/processing/)
- [Truce VST3 format](https://truce.audio/docs/formats/vst3/)
- [Rust `std::arch`](https://doc.rust-lang.org/std/arch/index.html)
- [Rust codegen and `target_feature`](https://doc.rust-lang.org/stable/reference/attributes/codegen.html)
- [`core::simd` nightly status](https://doc.rust-lang.org/stable/core/simd/)
- [SIMDeez documentation](https://docs.rs/simdeez/latest/simdeez/)
- [SIMDeez runtime invocation source](https://raw.githubusercontent.com/arduano/simdeez/master/src/invoking.rs)
- [simdscan repository and scope](https://github.com/vimkim/simdscan)
- [fearless_simd documentation](https://docs.rs/fearless_simd/latest/fearless_simd/)
- [Alex Heretic: runtime AVX2 dispatch](https://alexheretic.github.io/posts/auto-avx2/)
- [Nick Wilcox: Rust auto-vectorization](https://www.nickwilcox.com/blog/autovec2/)
- [packed_simd target-feature guide](https://rust-lang.github.io/packed_simd/perf-guide/target-feature/rustflags.html)
