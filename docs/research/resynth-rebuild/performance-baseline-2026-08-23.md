# KURV RESYNTH Rendering Performance Baseline (2026-08-23)

## Scope and repeatability

This is a CPU/test baseline for the current RESYNTH implementation after the
focused behavior tests were green. It is a test-harness baseline, not a claim
of host callback throughput: this checkout has no dedicated RESYNTH benchmark
target. No Rust source was edited for this note.

Observed checkout:

- `HEAD`: `64f30e5e7a3597d989a5b96aafb8ebb375e59de5`.
- The working tree was dirty with the concurrent RESYNTH/RT-pool source changes
  shown by `git status`; those changes were not made by this baseline task.
- Toolchain: `cargo 1.97.1 (c980f4866 2026-06-30)`, `rustc 1.97.1 (8bab26f4f 2026-07-14)`.
- Host: AMD Ryzen 7 7800X3D, 8 cores / 16 logical CPUs, Linux
  `7.2.0-rc7-1-cachyos-rc`, AVX2 and FMA available.
- CPU scaling governor was `performance`. CPU affinity and background load
  were not controlled, so wall-clock values are local comparison points.

## Cargo target and benchmark discovery

Native Cargo metadata command:

```text
cargo metadata --no-deps --format-version 1
```

Measured wall time: 0.026 s. The KURV package exposes `cdylib`, `staticlib`,
and `rlib` library targets, `kurv-standalone` binary, and `process_lab`
example. It exposes no `bench` target. There is no `benches/` directory and no
`[[bench]]` entry in `Cargo.toml`.

Therefore the repository currently has no Criterion/Bencher RESYNTH throughput
number. `cargo bench` is still the native discovery/build command to use when
a benchmark target is added:

```text
cargo bench --workspace
```

## Green focused test baseline

Native test command:

```text
cargo test resynth --all-targets -- --test-threads=1
```

The resulting optimized test binary was also run directly with the same
filter and serial setting to capture the harness timing without another Cargo
build. Result:

```text
154 passed; 0 failed; 143 filtered out
Cargo test-harness reported: finished in 17.19s
Observed process wall time for the direct harness: 40.10s
```

The process wall time includes local build/agent contention. Use the test
harness's 17.19 s value for this test-set comparison, and do not compare the
40.10 s process value across machines.

A focused existing Grain render stress test was run from that same test
binary:

```text
oscillators::resynth::artifact::tests::grain_high_unison_cheap_path_is_at_least_four_times_faster
```

Result: 1 passed, 0 failed, 296 filtered; the test body reported 0.01 s. It
renders 512 frames at 48 kHz (10.67 ms of audio) after preparing a bounded
Grain artifact. The test asserts the bounded active-lane condition; it does
not expose a calibrated samples/second counter, so its time is only a coarse
regression signal.

## Full-workspace check and interpretation

The initial native command below was run before the RT-pool behavior fix was
present:

```text
cargo test --workspace
```

It built in 1m08s and ran 297 tests in 2.25s: 296 passed and one failed in
`voices::voice::internal_rt_pool::tests::partitioned_render_waits_for_three_helpers_and_matches_serial_bits`.
That failure was outside RESYNTH and was reproduced as a scheduling/worker
participation failure. It is retained here as diagnostic history, not as the
green RESYNTH baseline. The focused serial RESYNTH run above passed after the
fix.

`cargo test --workspace -- --nocapture` is the verbose full-workspace command
for a future repeat. It was not used as a second performance sample because it
would duplicate the full test run and adds output overhead.

## Baseline limits and next measurement

The current Cargo manifest has no repeatable audio-render benchmark. This note
therefore establishes:

- a green serial RESYNTH test count and harness time;
- a bounded Grain render smoke-test time; and
- the host/toolchain conditions needed to compare a future result.

A future source change that claims CPU improvement should add a dedicated
benchmark target, then record at least Sample, Grain, and Rich rendering at
48 kHz with fixed artifact, note, block size, and voice/unison counts. Until
then, the numbers above must not be presented as real-time callback CPU load.
