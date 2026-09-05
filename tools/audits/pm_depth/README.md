# Parent-depth routing proof

These programs compile production routing methods and compare the optimized
route-buffer calculation against the pinned pre-optimization implementation.
They do not compile a complete voice or measure aliasing. The enum declarations
are deliberately minimal fixtures; route storage, graph ordering, eligibility,
and arithmetic come from production source.

Run using the compiler selected for your build, for example:

```sh
python3 tools/audits/pm_depth/probe.py --rustc /root/.cargo/bin/rustc --toolchain 1.97.1 --seed 1299709
python3 tools/audits/pm_depth/probe.py --rustc /root/.cargo/bin/rustc --toolchain 1.97.1 --native --seed 37
python3 tools/audits/pm_depth/probe.py --rustc /root/.cargo/bin/rustc --toolchain 1.97.1 --negative-control
python3 tools/audits/pm_depth/eligibility.py --rustc /root/.cargo/bin/rustc --toolchain 1.97.1
python3 tools/audits/pm_depth/mixed_gain_proof.py --rustc /root/.cargo/bin/rustc --toolchain 1.97.1
```

The absolute compiler path above is an example, not a required installation.
`--rustc` defaults to `RUSTC`, then PATH; omit `--toolchain` for a directly selected
compiler. Capacity declarations are extracted from production, including dependent
expressions. The parity runner refuses capacity drift relative to the baseline
instead of silently testing invented storage sizes. Named fixture slots remain
controlled scenarios with explicit capacity assertions.

Each seed selects 40 deterministic source streams. The corpus spans blocks
1/3/7/8/15/16/31/32/63/64/65/128, parents 0/1/2/4/8/16, incoming routes 1/2/4,
matching and unrelated overrides, signed values, zero contributions, and clamping.
A negative control deliberately removes parent contributions and must fail the
parity assertion. It never changes checked-out production code.

## Combined PR18 + PR17 check, 2026-09-05

Rust 1.97.1, local merge `2219bf2f8003957d85d25017282057f54a46ae85`:

- Portable holdout seed 1299709: 7,482,240 bit-identical comparisons passed.
- Native holdout seed 37: 7,482,240 bit-identical comparisons passed.
- Production graph topology/eligibility: 192 fixtures passed.
- Actual mixed gain helper: 19,200 comparisons passed, static bit-identical,
  dynamic maximum error zero in this corpus.
- Deliberately removed parent contribution: rejected by the parity assertion.

These counts include repeated arithmetic comparisons; they are not independent
synth patches or spectral quality cases. Old CSV timings in this directory remain
historical PR17 component measurements and must not be presented as fresh combined
synth measurements. No concurrent CPU benchmark was run for this check.

## Full voice regression coverage

`grouped_voice_depth_matches_samplewise_route_amounts` in production PolySynth tests
now compares explicit per-sample route amounts to nested voice-LFO routes with:

- One child (eligible for amount buffering) and two children (sample-route fallback).
- Level/Pan, PM/Transpose, PM/Level, PM/Pan and PM/Ring pairs.
- Fast audio-rate modulation enabled and disabled; scalar and block rendering.
- Block lengths 1/7/16/31, unison 1/4/8/16/64, and a varying external override.
- A nonzero-output check and a zero-depth comparison so silent or disconnected
  fixtures cannot accidentally pass.

**This expanded full-voice test was added and formatted, but not executed here:**
the private `derpcat-access` dependency is absent. It is not covered by the standalone
VA harness. It is still one note and a controlled two-oscillator graph, not maximum
polyphony, arbitrary cross-routing, or host/thread-pool validation.

## Semantic merge review

PR18's `voice_requires_sample_routes` gate still surrounds PR17's time-block
selection. Consequently multiple child depths and an incompatible external
one-child buffer retain sample evaluation. PR17 mixed graph eligibility still
rejects feedback, carriers used as sources, and filter/auxiliary graphs. Parent
sources are included in production topology ordering. No production arithmetic or
routing eligibility was changed as part of this validation cleanup.
