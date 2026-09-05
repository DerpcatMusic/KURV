# Whole-synth performance matrix consolidation

This consolidates the useful PR14 runner/scenarios onto PR18 without replacing
PR18's whole-stream finite/peak/energy checks. No synth speedup is established by
these infrastructure changes. Full process_lab compilation in this workspace is
blocked by the missing private sibling derpcat-access/Cargo.toml; no dependency
was stubbed. Python runner acceptance tests pass; full workload execution remains
required on an authorized build machine.

## Measurement contract

Every callback is checked after the timed process call. Report p50/p95/p99/p999,
maximum, deadline misses, whole-stream energy/sum/peak and audibility. Reject
missing fields, wrong workload identity, NaN, silence, impossible counts,
unordered percentiles and unsuccessful process exits. The printed precision can
reject extremely quiet valid outputs; that is a fail-closed limitation, not proof
that the synth was silent. A checksum/energy match is not audio-equivalence proof.

The runner records binary SHA256, explicit source/compiler/build descriptions,
CPU/platform/affinity, an allowlist of non-secret DSP/toolchain environment and raw stdout/stderr. It alternates
AB/BA sequential runs and checkpoints failures rather than reporting a speedup.
A shared POSIX lock prevents competing instances of this runner; coordinate all
other CPU benchmarks externally. Linux/macOS are supported; platforms without
fcntl fail closed. Binaries must contain identical instrumentation and scenarios.

Warmup is explicitly configurable (default 256 callbacks) and identical across
scenario families, replacing the previous scenario-prefix-dependent warmup.
Requested voices/oversampling outside supported ranges now fail rather than clamp.
Quantiles use tested nearest-rank indexing: a 20-sample p95 is sample 19,
not the maximum. A short run does not reliably estimate p99.9; use enough callbacks and inspect
sample counts. Pre/post validation still affects caches although it is not timed.

## Coverage and semantics

`--mode full` expands independent source/carrier unison counts 1/4/8/16/64 and
held-note counts 1/8/32. It covers static sine/triangle/saw/pulse, no-route controls,
PM, transpose routing, AM/ring/pan, mixed PM+level, self/cycle feedback, parent
depths including 1 kHz voice LFOs, forward three-oscillator PM, filter depth,
four-oscillator stress and one/two-group mixed rigs. Source-carrier pairs include
both 1x64 and 64x1. These are named workload families, not exhaustive combinations
of every synth control. `xfm` means OscillatorControl::Transpose (pitch modulation),
not a claim of linear through-zero frequency modulation. `xdepthpm` has a carrier
feeding parent depth; `xnestedpm` is a forward A→B→C phase chain. Feedback topology
semantics belong to production routing and must be separately checked for audio
correctness. Held MIDI note counts are capped by the production MAX_POLYPHONY constant
(currently 32), and voice_mode is explicitly set before state initialization.
These are requested held notes, not independently measured active voices.

Override axes with --voices, --unison, --frames, --sample-rates, --oversampling.
Use --scenarios to select any existing process_lab case directly. The dry-run
prints the actual case list and process count, avoiding fixed test case totals.
The full Cartesian matrix can take hours; inspect the dry-run and stage cases.

```
python3 scripts/audits/run_performance_matrix.py BASE CANDIDATE --dry-run --mode full
python3 scripts/audits/run_performance_matrix.py BASE CANDIDATE \
  --baseline-build 'COMMIT rustc VERSION FLAGS' \
  --candidate-build 'COMMIT rustc VERSION FLAGS' \
  --voices 1 8 32 --frames 32 64 256 --oversampling 1 4 \
  --scenarios xpm-1x64 xpm-64x1 xcyclepm-64x64 xdepthpm-64x64 \
  --callbacks 2048 --repeats 5 --output paired-results.json
python3 -m unittest discover -s scripts/audits -p test_performance_matrix.py
```

Assertions test parser rejection, configurable independent axes, pair ordering,
metadata recording and failure persistence. They deliberately do not hardcode a
matrix size or assert machine-specific timing thresholds. Rust scenario setup
and actual full-synth timings still need full-plugin compilation/validation.
