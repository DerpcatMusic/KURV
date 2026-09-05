# Integration checkpoint — 2026-09-05

Base: main 251d8b84e40e6c574e20e59f5ff4f7b1c6a02874 (PR #16 already merged).

## Integrated locally

PRs #18, #19, #17, #10 and #11, plus selected #14 benchmark scenarios and runner improvements. These are local integrations, not completed GitHub merges. Aggregate version: 0.8.158.

Additional fixes: profiler producer/consumer ownership guards with dropped-record accounting; preallocation before callbacks; thread-local test captures; explicit benchmark voice capacity; fail-closed result parsing and Clippy ratchet; paired AB/BA benchmark execution; source-hashed multi-seed oscillator partition checks; current-source adapters replacing stale extracted harnesses. Public component CI supplements existing full-plugin gates.

## Fresh evidence

- Shared VA adapter: 40 passed, 0 failed, 43 ignored. Ignored manual experiments are not passing acceptance tests.
- Three seeds on baseline and AVX2/FMA: 11,784,192 stereo sample comparisons; maximum partition discrepancy 1.19e-7. Generic PM scalar comparison maximum 7.5e-8; canonical shape/backend comparison maximum 7.883e-6.
- Parent routing: two independent holdout runs, each 7,482,240 bit-identical comparisons; 192 graph cases and 19,200 mixed-route cases. Dropped-parent negative control fails as intended. These component proofs do not execute the complete synth.
- Actual profiler source: 10 tests pass, concurrent producer integrity checked; real non-test initialization and first/sustained/full-ring callbacks incur zero allocator operations in the instrumented probe.
- Matrix acceptance tests: 11 pass; quantile tests: 2 pass; Clippy parser regression tests: 3 pass.
- Old phase-modulation probe now compiles against the shared production VA adapter. Its PM/direct-tuning diagnostic still reports approximately 0.694479 maximum difference; this is not an aliasing measurement or a passing quality threshold.

Adapters replace framework SIMD with wide and omit documented persistence/resynthesis plumbing. Read tools/audits/pm_integration/README.md for exact scope. Its results.json contains source hashes and can be checked with run.py --verify-record.

## What cannot yet be claimed

No fresh full-synth CPU improvement percentage. Full Cargo builds stop at missing ../derpcat-access/Cargo.toml even with default features disabled. Tracked checkout also lacks licensing module/backend inputs and a patched vendor dependency needed by the original build. No stubs or bypasses were introduced. The 600-configuration full-voice routing test and real worst-case CPU matrix are therefore pending.

The #19 historical 16-voice, four-unison, 4x result (317 microseconds p50, 5.95% of a 256-frame/48-kHz budget) is an absolute result from its original environment, not a baseline/candidate speedup. Earlier #17 parent-buffer reductions and packed-PM speedups are component-specific historical results, not a combined synth gain.

## Merge and cleanup order

1. Restore legitimate full-build dependencies and run the aggregate branch's full-plugin correctness gates, including the 600-configuration voice test.
2. Build main and this branch on the same machine/compiler/options. Run scripts/audits/run_performance_matrix.py in paired sequential AB/BA mode. Cover single/asymmetric/multiple unison, nested parents, cross-routing/cycles, filters, small and irregular blocks, sample rates and oversampling. Inspect peak and tail latency as well as medians, with complete non-silent finite output validation.
3. Merge the aggregate only once required gates pass. Then close superseded #10/#11/#14/#17/#18/#19 with links to the aggregate and evidence. Do not close them before the replacement is published and merged.
4. Keep #13 and #15 experimental: conflicts and known workload-dependent regressions prevent blanket promotion. Review #8 and the #9/#12 expression-stack chain separately; they are not included here.

GitHub ready/merge/create-branch connector actions returned Unknown tool in this session. This is a missing connector capability, not an approval rejection. No remote merge or PR closure was performed.
