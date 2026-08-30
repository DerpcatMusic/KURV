# VA scalar-warp optimizer visibility

Status: **ship in 0.8.10**

## Change

The fixed scalar warped-shape renderer called a private step helper whose
concrete phase-warp arguments prevented consistent optimizer visibility from
the block caller. The same phase step, warp mode, and warp amount crossed that
boundary for every sample in 24- and 32-frame fixed blocks.

The existing private step helper now accepts the phase-warp evaluator as a
generic closure, and `warp_phase_scalar` is inline-visible. The public scalar
API, phase-warp equations, pulse-edge preparation, phase advance, and sample
evaluator are unchanged. Fixed legacy and structural blocks can now optimize
through the complete call chain. Dynamic single-step paths still evaluate the
same scalar warp with their current step and depth on every call.

No manual prepared-depth state or branch is shipped. The x8 constant paths
already prepare their fixed warp and are unchanged. The change adds no retained
state, allocation, lock, I/O, table, approximation, latency, or audio-thread
work.

## Rejected forms

The audit tested the smallest forms before changing the helper boundary:

- inlining only `warp_phase_scalar` did not expose the enclosing step helper;
- inlining only the step helper, and ordinary inlining of both boundaries,
  regressed the measured production block;
- explicit scalar depth preparation was bit-exact and often faster, but its
  production form retained small PWM misses and therefore failed the universal
  Pareto gate.

The accepted generic-helper form removes the duplicate test-only step helper
and produced the universal CPU result without shipping manual prepared state.

## Exactness

The ignored identity gate compares the original no-inline scalar body, the
shipping generic-inline body, and the rejected explicit-depth reference. It
covers:

- 24- and 32-frame continuous blocks;
- shapes 2.001, 2.5, and 3.0;
- phase steps 0.00001, 440/48000, 0.083, and 0.44;
- pulse widths 0.03, 0.31, 0.5, and 0.97;
- PWM, Phase Bend, and Harmonic warp;
- amounts 0.0001, 0.5, and 1.0;
- Spline and Spline Optimized antialiasing;
- 64 consecutive blocks per case, including final oscillator phase.

Every output sample and final phase matched bit for bit.

## Fixed-block CPU

Ryzen 7 7800X3D, release mode, CPU 4, 100,000 blocks per repetition, median of
five with three-way execution order rotated per repetition. The 24 measured
cells span both shapes, all three warp modes, both block sizes, and both the
legacy output and structural accumulation kernel.

The shipping generic-inline ratio was 0.896-0.969 versus the original
no-inline body: every cell was faster, by 3.1-10.4%. The explicit-depth
reference was retained only as test evidence and is not in production.

## Fair outer structural CPU

The outer probe uses a test-only const selector around the same copied
single-lane router. Both sides execute identical engine, jitter, voice-count,
custom/warp eligibility, phase-step, jitter-reset, and stereo-accumulation code;
only the old no-inline or shipping generic-inline block helper differs.

The acceptance bound was declared before the run as a paired median ratio no
higher than 1.015. Each of the 12 cells used nine alternating-order repetitions
of 100,000 blocks on CPU 4. All paired medians won, ranging from 0.890 to 0.986,
or 1.4-11.0% faster. The worst individual repetition was 1.002, inside the
declared noise bound.

## Commands and validation

```text
cargo test --release --no-default-features --lib \
  fixed_scalar_warp_preparation_matches_current_bits --locked -- \
  --ignored --nocapture --test-threads=1
```

Passed 1/1.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  fixed_scalar_warp_preparation_cpu_report --locked -- \
  --ignored --nocapture --test-threads=1
```

Passed 1/1; all 24 shipping cells improved.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  fixed_scalar_warp_outer_structural_cpu_report --locked -- \
  --ignored --nocapture --test-threads=1
```

Passed 1/1; all 12 fair outer cells improved and the declared parity assertion
passed.

```text
cargo test --release --no-default-features --lib 'oscillators::va::' \
  --locked -- --test-threads=1
```

Passed 22, failed 0, ignored 14.

```text
taskset -c 4 cargo test --release --no-default-features --lib --locked -- \
  --test-threads=1
```

The full release library run completed with 350 passed, 9 failed, and 37
ignored. The nine failures exactly match the pre-change baseline: two RESYNTH
artifact/vocoder checks, structural LFO advancement, production-pool dispatch,
and five internal-pool timing/eligibility checks. The three additional ignored
tests are this experiment's identity and CPU evidence. No new failure appeared.
