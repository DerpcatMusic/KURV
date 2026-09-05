# Parent-depth routing: remove repeated graph traversal

Baseline: `d084681411a95803bb52206647c2bc881c4cbf8b`.

A parent controlling a modulation amount ultimately needs simple per-sample arithmetic:

`amount = clamp(base + parent_0 * depth_0 + parent_1 * depth_1 + ..., -1, 1)`

`phase_offset += source * amount`

The existing phase and pitch block accumulators nevertheless call `block_amount` for each frame, traversing the parent linked list, looking up route entries and selecting an optional override repeatedly. The dependency chain across the linked-list loads also obstructs straightforward vectorization across samples. This is routing overhead, separate from oscillator generation and antialiasing cost.

The change traverses each parent once per block and applies its multiply/add to a contiguous stack array. Parent order, operation association, final clamping, and every audio-rate parent sample are preserved. A no-parent/no-override specialization hoists the constant clamp out of the sample loop. The existing phase override shortcut intentionally does not clamp supplied amounts; this behavior is preserved, including in the proof's out-of-range overrides. Pitch overrides retain their existing clamp. No signal smoothing, control-rate decimation, approximation, heap allocation, or oversampling is added.

The helper `mixed_gain_amount_block` exposes the same amount evaluation to the mixed phase/gain renderer. It returns `None` for no gain route, `Some(false)` for a constant amount, or `Some(true)` for an audio-rate amount; this lets callers retain constant ring-modulation arithmetic outside the sample loop. It assumes an already validated single-gain-route mixed graph, and does not itself validate graph eligibility.

## Executable proof

```
python3 tools/audits/pm_depth/probe.py
python3 tools/audits/pm_depth/probe.py --native
```

Each reports **2,920,320 bit-identical sample comparisons**. The harness extracts production route structures, defaults and methods (including inline attributes) from the pinned baseline and current checkout, compiling them with Rust 1.97.1. Small enum stand-ins replace unrelated application enums; oscillator rendering and the full plugin are not compiled. No dependency on the unavailable private `derpcat-access` crate is needed for this isolated proof.

Cases cover 1/8/32/128-frame blocks, 0/1/2/4/8/16 parents per route, 1/2/4 incoming routes, phase/transpose/cents accumulation, level/pan/ring gain amount evaluation, matching/unmatched/absent amount overrides, exact zero parent weights, zero base amounts, signed varying sources, clamp saturation, and nonzero initial accumulation. The test checks the mixed helper's constant/dynamic classification too. There is no feedback-policy change: this helper is consumed by eligible feed-forward block rendering; scalar feedback evaluation remains unchanged.

## Measurements and their limits

```
taskset -c 2 python3 tools/audits/pm_depth/probe.py --bench > portable.csv
taskset -c 2 python3 tools/audits/pm_depth/probe.py --native --bench > native.csv
```

Measured on AMD EPYC 9V74 under KVM, Rust 1.97.1, `opt-level=3`, no LTO; portable and `target-cpu=native` measured separately. Each case runs 30,000 iterations in eight alternating baseline/candidate order rounds. Inputs, graph and output use `black_box`. Both implementations run in the same executable. Raw CSV and median summary accompany the change.

**These are route-accumulator timings, not whole-voice or whole-plugin speedups.** They do not include parent waveform generation, oscillator synthesis, filters, unison, envelopes or full modulation topology evaluation. Full-plugin integration still needs the private dependency and a host build.

Example median nanoseconds per 32-frame block, one incoming phase route, no override:

| Parents | Portable baseline | Portable candidate | Native baseline | Native candidate |
| --- | ---: | ---: | ---: | ---: |
| 0 | 41.2 | 7.6 | 38.3 | 10.4 |
| 1 | 58.0 | 13.1 | 65.2 | 10.7 |
| 4 | 142.4 | 22.1 | 143.6 | 14.1 |
| 16 | 718.3 | 47.8 | 710.7 | 38.0 |

Across measured cases with at least one generator parent, route accumulation CPU fell 73–94% portable and 84–97% native. The existing one-route LFO override with no generator parents already had a contiguous fast loop and remains close to its original cost. Pitch and mixed-gain helpers have equivalence proof but no separate throughput claim in this benchmark.

No patch version bump: this is an unshipped review change.
