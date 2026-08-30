# Short-block AVX-512 pulse selector (2026-08-30)

## Verdict

Reject the AVX-512 pulse selector and leave production unchanged.  Re-running
the earlier whole-block idea at KURV's actual 24- and 32-frame internal chunks
finds a much larger dense-event win than the 64-frame probe, but the required
per-pack pitch/ISA selector repeatedly slows the sparse fallback.  The result
is useful, not Pareto-safe: selected threshold/mid/high packs save roughly
13--32%, while 32-frame low packs regress 5.5--14.6%.

The implementation remains an ignored, test-only experiment.  There is no
production dispatch, version bump, oscillator state, table, or shipped binary
growth.

## What was measured

The probe owns the complete x8 pulse path for `SAMPLES = 24` and `32`: phase
walk, raw pulse, both optimized cubic BLEPs, stereo accumulation, and phase
writeback.  Those are the production chunk sizes
(`FACTOR3_BLOCK_INTERNAL_SAMPLES` and `BLOCK_INTERNAL_SAMPLES`) and the
comparison calls the shipping `accumulate_shape8_block_constant` seam with
shape `3.0` and `Antialiasing::SplineOptimized`.

The selector requires AVX-512F/VL, AVX2, and FMA, and chooses the candidate
only when all eight lane steps exceed `0.04`.  A partially eligible x8 pack
therefore takes the exact shipping path.  The candidate is fixed-work and
branchless inside the sample loop; the shipping path retains its sparse-event
advantage.

Each cell includes phase advancement, two-channel writes, per-block output
clearing, and output/state observation.  The timed modes were shipping,
candidate, and selected real-path calls, interleaved in three rotating orders.
Reported values are the median of nine 50,000-block samples.  Two complete
uncontended passes were pinned to CPU 6.

Host: AMD Ryzen 7 7800X3D (Zen 4), with AVX-512F/BW/VL/VBMI available.  Build:
release, thin LTO, one codegen unit, no default features, locked dependencies,
and `RUSTFLAGS='-C target-cpu=x86-64-v3'` in the isolated
`/tmp/kurv-va-events3-target` target directory.

Command:

```sh
taskset -c 6 \
  /tmp/kurv-va-events3-target/release/deps/pure_va_dispersion_core-d357f6de8033f494 \
  oscillators::va::render::tests::avx512_whole_pulse_block_report \
  --ignored --exact --nocapture --test-threads=1
```

## CPU results

The table gives the full range across both confirmation passes, three pulse
widths (`0.03`, `0.37`, `0.97`), and coherent/decorrelated phases.  Negative
is faster than shipping.

| frames | lane step | selector delta | result |
|---:|---|---:|---|
| 24 | low `.0046` | -2.7% to +5.1% | fallback regression in 11/12 cells |
| 24 | threshold `.0401` | -16.9% to -27.5% | win |
| 24 | mid `.041` | -19.5% to -26.6% | win |
| 24 | high `.083` | -9.3% to -32.7% | win, one noisy weak cell |
| 32 | low `.0046` | +5.5% to +14.6% | consistent fallback regression |
| 32 | threshold `.0401` | -13.5% to -24.3% | win |
| 32 | mid `.041` | -11.4% to -20.9% | win |
| 32 | high `.083` | -18.0% to -29.2% | win |

Calling the candidate unconditionally at low step is worse by 40.7--82.5%,
so removing the selector is not an option.  Raising the threshold also cannot
repair the measured fallback overhead: low packs already take the shipping
branch, and merely evaluating the per-pack policy costs enough to fail the
gate.  Hoisting a CPU capability decision would remove part of that work, but
the pitch policy, mixed-pack handling, architecture-specific backend, and
their real call-site economics would still need a different experiment.

## Exactness and transitions

Final phase was bit-identical in every static and transition case.  Across the
static cells, AVX-512 versus shipping output had RMS error at most `3.90e-7`
and peak error at most `2.328e-6`.  Those are floating-point ordering deltas;
the evaluator, BLEP curve, alias behavior, and discontinuity timing are
unchanged.

Four-block sequences exercised low-to-dense pitch crossings, width changes
`.37 -> .03 -> .97 -> .37`, and coherent/decorrelated phase resets.  Both
24- and 32-frame chunks and both stereo channels were checked.

| transition property | worst case |
|---|---:|
| phase delta | exactly `0` |
| output residual | `-133.47 dB` |
| output residual peak | `4.92e-7` |
| added block-boundary step | `1.94e-7` |

The test asserts phase equality and a `4e-6` bound for output and boundary
regressions.  Both complete runs passed.

## Code and real-time economics

The two candidate monomorphs are 810 bytes each in the release test binary,
1,620 bytes total.  Disassembly shows no stack spills in the hot loop.  The
probe allocates no heap memory, performs no I/O or synchronization, uses only
bounded fixed arrays/register state, and leaves the oscillator's existing
phase contract intact.

Those properties make the kernel real-time safe in isolation, but do not make
the selector free.  Keeping an x86-only backend, feature publication, a
dense-pack threshold, mixed-lane fallback, and another correctness surface is
not earned while the actual short-block fallback loses.  Preserve this probe
as evidence; do not integrate it into production or generalize it to other
shapes without first finding a selector-free call-site architecture.
