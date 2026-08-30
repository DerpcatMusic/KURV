# Exact-Fourier high-note crossover experiment (2026-08-30)

## Verdict

The fixed-note scalar prototype is a strong high-note candidate, but it is not
ready for production. At 48 kHz from MIDI 93 through 123, a 1x additive
evaluator using exact coefficients of the compiled polynomial beat commit
`427917d`'s shipping 2x custom-curve path on both ideal-reference RMS error and
CPU for saw and square probes. It was 2.7x to 11.5x faster and reduced RMS error
by 22x to 133x.

No production DSP was retained. Pitch motion, coefficient morphing, warp/PM,
harmonic entry/exit transitions, and SIMD unison remain unmeasured. Those are
part of KURV's oscillator contract, so this experiment establishes a crossover
frontier, not a shippable backend.

## What was tested

The retained ignored test in `src/wave_curve.rs` does four things:

1. Integrates every cubic `WaveCurveRt` segment analytically against
   `exp(-i 2 pi k phase)`. This is distinct from the existing 2,048-point FFT
   mip compiler in `src/wave_curve/bandlimit.rs`.
2. Checks the exact saw result: coefficient `k` has zero real part and imaginary
   part `1 / (pi k)`, within `2e-12`, for harmonics 1 through 16.
3. Evaluates the retained harmonics with fixed-size `f32` complex recurrences.
   The audio-rate loop allocates nothing and performs no lock, I/O, or table
   lookup. Cost is O(retained harmonics); prototype state is fixed-size.
4. Compares current 1x analytic evaluation, current sampled spectral mips, the
   raw additive kernel, shipping 2x synthesis plus its 97-tap stereo decimator,
   and a 1x additive candidate plus the normal direct latency path and crossover
   branch.

The reference is the exact Fourier projection of the same compiled polynomial,
retaining every harmonic strictly below Nyquist. RMS error therefore includes
aliasing, wanted-band magnitude/phase error, and recurrence drift. Shipping 2x
is aligned to its actual 32.5-sample signal delay; KURV reports the enclosing
integer latency of 33 samples. The candidate direct path is aligned to 33.

Workload: Ryzen 7 7800X3D, Linux, one pinned logical CPU (`taskset -c 8`), 48
kHz, 131,072 samples for error, 2,000,000 host frames per timing repeat, median
of seven release repeats. Saw and square use exact piecewise-linear/constant
`WaveCurveRt` coefficients. Frequencies are equal-tempered MIDI notes.

## Results

RMS error is linear amplitude. CPU is median nanoseconds per host frame and
includes the selected output-latency/decimation path.

| Shape | MIDI | Hz | Exact partials | Candidate 1x RMS | Shipping 2x RMS | Candidate 1x ns | Shipping 2x ns |
|---|---:|---:|---:|---:|---:|---:|---:|
| Saw | 93 | 1,760.000 | 13 | 0.001454 | 0.064217 | 13.153 | 37.158 |
| Saw | 105 | 3,520.000 | 6 | 0.001527 | 0.078779 | 6.679 | 36.154 |
| Saw | 117 | 7,040.000 | 3 | 0.001221 | 0.111781 | 4.187 | 39.262 |
| Saw | 123 | 9,956.063 | 2 | 0.004021 | 0.129416 | 3.462 | 37.966 |
| Square | 93 | 1,760.000 | 13 | 0.002474 | 0.101730 | 13.452 | 36.336 |
| Square | 105 | 3,520.000 | 6 | 0.002798 | 0.110594 | 6.742 | 36.442 |
| Square | 117 | 7,040.000 | 3 | 0.002330 | 0.162817 | 4.268 | 36.161 |
| Square | 123 | 9,956.063 | 2 | 0.004947 | 0.176690 | 3.425 | 39.383 |

The current raw 1x analytic path measured 7.20-8.67 ns/frame but had 0.123-0.435
RMS error. The sampled spectral evaluator measured 8.54-10.32 ns/frame. At MIDI
93 its discrete 12-partial mip omits the legal 13th harmonic, producing RMS
error 0.034723 for saw and 0.069350 for square; exact coefficients avoid that
coarse-cap loss. At the other notes its error was 0.00191-0.00304, sometimes
lower than the prototype recurrence because the latter accumulates `f32` drift.

## Reproduction

```bash
cd /tmp/kurv-va-fourier
cargo test exact_polynomial_coefficients_reconstruct_a_saw --lib --release --locked -- --nocapture
taskset -c 8 cargo test benchmark_exact_fourier_high_note_crossover --lib --release --locked -- --ignored --nocapture --test-threads=1
cargo fmt -- --check
git diff --check
```

An attempted isolated x86-64-v3 build used:

```bash
env CARGO_TARGET_DIR=/tmp/kurv-va-fourier-target-v3 \
  RUSTFLAGS='-C target-cpu=x86-64-v3' \
  cargo test benchmark_exact_fourier_high_note_crossover --lib --release --locked -- \
  --ignored --nocapture --test-threads=1
```

It failed during dependency compilation with `No space left on device`; the
failed temporary target was deleted. The reported results are therefore the
existing host-default release build, not the requested pinned x86-64-v3 build.

## Limits and next gate

- Only one scalar oscillator was measured. KURV's eight-wide and 64-lane
  unison paths may change the crossover materially.
- Coefficients are static. A production backend needs immutable/off-thread
  coefficient publication and click-free harmonic/cap changes without audio
  thread allocation or locks.
- Audio-rate pitch, morph, warp, PM, sync, and pulse-width movement were not
  modeled. Static Fourier truncation is not an exact reference for arbitrary
  modulation sidebands.
- The `f32` recurrence drifts over the 131,072-sample run. A shorter periodic
  correction, `f64` state, or phase-derived batch evaluator must beat it on
  both error and SIMD CPU before integration.
- This isolated kernel does not include the complete voice, modulation,
  panning, filter, or host callback cost.

The smallest useful continuation is an eight-lane fixed-size recurrence at
caps 2, 3, 6, and 13 with bounded phase re-anchoring, followed by pitch-ramp and
curve-morph transition tests. Do not add a production mode before those gates
pass.
