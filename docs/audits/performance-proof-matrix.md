# Performance evidence before performance claims

Baseline audited: `d084681411a95803bb52206647c2bc881c4cbf8b`.

This change supplies a reproducible comparison harness. It does not make the
synth faster, establish audio equivalence, or establish superiority over another
synth. No actual plugin timing run was performed for this change: the available
checkout cannot build its private/path dependencies.

## Measurement defects corrected

`process_lab` previously checked finiteness and peak only in its final buffer.
An earlier NaN/Inf or transient peak could escape that check. The timed stream
now accumulates a sticky finite flag and its maximum absolute sample value;
nonfinite output fails the run. The existing stream sum/energy remain available.
The shared observer has direct Rust regression tests for an invalid early
buffer followed by valid audio, and an early peak followed by silence.

The harness adds callback p99, p99.9 (`p999_ns`), and `deadline_misses` counted
against `frames / sample_rate`. Validation, percentile bookkeeping, and deadline
comparison occur after `callback_start.elapsed()`. These are process-call timing
measurements, not a guarantee about total host callback scheduling. Observation
between calls still influences cache/thermal behavior; use identical harness
versions for both builds. Percentiles use sorted sample index `floor(N*p)`;
small sample sets cannot support strong tail-latency conclusions.

## Workloads

`--mode full` covers every combination of:

- 1, 4, 8, 16, 64 unison lanes.
- 1 and 8 held MIDI notes, independently of lane count.
- Single sine, triangle, saw, and pulse oscillator.
- Existing `xpm`, `xfm`, and `xdepthpm` two-oscillator route scenarios.
- New `xnestedpm-N`: forward A → B → C phase-position routes. B is both an
  audio-rate target and source. This is different from modulation of route depth.

`xfm` is the existing Transpose destination route; do not equate it with a
linear-Hz FM implementation. Solo/nested oscillators disable random initial
phase. Other engine/default state is shared by both tested revisions. Scenario
construction follows the existing generator graph API; integration rendering
is still required to verify the complete signal path with available dependencies.

Quick mode selects 1/8/64 lanes and sine/saw plus all four modulation scenarios.
Default axes are 64 frames, 48 kHz, 2x oversampling. Override them explicitly for
other block sizes, rates, and factors. Full mode has 80 cases per combination
of frame size, rate, and oversampling; the example below has 2,160 cases and
12,960 separate processes with the default three pairs, so budget runtime.

## Reproduce

Build both comparison revisions with **the same updated process_lab harness**,
toolchain, release flags, backend selection, and dependency versions. Copy the
resulting binaries to separate stable paths. Their SHA-256 hashes are recorded;
compiler/build information must be supplied explicitly (an installed compiler
does not identify the compiler that produced a supplied binary).

```sh
python3 scripts/audits/run_performance_matrix.py /tmp/lab-before /tmp/lab-after \
  --mode quick --cpu 'CPU model; governor; affinity policy' \
  --baseline-build 'commit=... rustc=... RUSTFLAGS=... profile=release' \
  --candidate-build 'commit=... rustc=... RUSTFLAGS=... profile=release' \
  --output /tmp/kurv-quick.json

python3 scripts/audits/run_performance_matrix.py /tmp/lab-before /tmp/lab-after \
  --mode full --frames 32 64 256 --sample-rates 44100 48000 96000 \
  --oversampling 1 2 4 --dry-run
```

The runner launches no competing timing processes: each pair is sequential,
AB then BA on the next round. Each process performs the existing warmup. Raw
metrics and paired baseline/candidate speedup ratios are retained by workload;
values greater than one mean lower candidate median time. Tail timing and
miss counts remain separate, so an average speedup cannot hide a deadline
regression. Binary hashes, platform, affinity when supported, supplied CPU/build
metadata, settings, and start time are recorded. Each completed case is saved
atomically. Invalid output, failed processes, nonfinite/silent workloads,
unordered percentiles, or missing instrumentation fail closed and save an error.
A dry run only prints the matrix and process count; it does not read binaries.

Do not run this alongside other benchmarks. Record cooling, frequency policy,
affinity, power state, and system load. Alternating order reduces some drift;
it does not remove OS noise or establish statistical significance. Three pairs
are a starting point, not a confidence interval. Use more observations for tails.
Raw checksums/energy are diagnostic only: a separate spectral/correctness gate
must establish that a speedup has not bought aliasing or changed sound.

## Validation performed

- Python stdlib unit tests: **9 passed**, including malformed/nonfinite/silent
  output, axes coverage, paired summaries, AB/BA sequencing, and failure saving.
- Standalone production stream observer tests, Rust 1.97.1: **2 passed**.
- Rust parser/format check via rustfmt 1.97.1: passed.
- Full plugin build, graph rendering, and performance measurements: **not run**.

```sh
python3 -m unittest discover -s scripts/audits -p test_performance_matrix.py -v
rustc --edition=2024 --test examples/process_lab_support/stats.rs -o /tmp/kurv-stats-tests
/tmp/kurv-stats-tests
```

Still missing from a comprehensive performance campaign: custom expressions,
warp/shape automation, filters, effects, resynthesis, voice stealing, note-event
bursts, long release tails, denormals, cross-platform SIMD backends, and helper
preemption. Existing process_lab scenarios cover some of these individually;
this matrix deliberately establishes the requested oscillator/unison/audio-rate
baseline first. Claims about competing synths also need matching sound, quality,
polyphony, oversampling, and latency—not a shared preset name.
