# Audio-rate and nested phase-modulation quality audit

Baseline: `d084681411a95803bb52206647c2bc881c4cbf8b` (2026-09-04 audit).
This PR adds reproducible evidence and proposed acceptance criteria. It does not
change the oscillator sound, add audio-thread work, or claim to solve arbitrary PM.

## Confirmed limitation and its scope

The production PM saw kernels evaluate the **modulated phase** using BLEP support
derived from the **unmodulated carrier step**. This is true for x4, x8, x8 lane
accumulation, and time-vectorized x8 rendering, not just an unused helper.

For a phase ramp, carrier step `1/128` plus PM slope `7/128` has the same sampled
phase trajectory as a directly tuned oscillator at `1/16`. The compiled production
x4, x8, and x8-lane kernels disagree with direct tuning by **0.694478989 peak
sample units**. Binary-exact phase steps eliminate accumulated phase rounding as
the explanation. The Python equation independently gives `0.6944790057`.

This proves a trajectory-dependent quality limitation, not automatically an
incorrect musical definition: “phase-modulate a carrier-frequency-smoothed
waveform” and “antialias the final trajectory” are different signal operations.
The former is what these kernels implement. The latter is the proposed quality
goal and the reason the ignored acceptance test presently fails.

The reference is **not** another commercial synth, and these measurements do not
establish whole-plugin alias levels or audible severity after its output chain.

## Executable evidence

From the repository root (Rust 1.97.1; Python 3 and NumPy):

```sh
cargo +1.97.1 run --release --manifest-path tools/audits/phase_modulation/Cargo.toml
cargo +1.97.1 run --release --manifest-path tools/audits/phase_modulation/Cargo.toml -- --dump-nested > tools/audits/phase_modulation/target/nested.csv
python tools/audits/phase_modulation/probe.py --production-samples tools/audits/phase_modulation/target/nested.csv
```

The small Rust crate compiles the current production `antialias.rs` and extracts
the three actual saw PM accumulator functions at build time. It uses the exact
`wide 0.7.33` SIMD types that `truce-simd 6.3.0` reexports. Only the phase-state
container is reduced to the field these functions access. It is not a rewritten
DSP model. This isolates the DSP evidence from plugin, UI and licensing builds.
The extractor is intentionally narrow; source signatures changing should trigger
review of the harness rather than silent acceptance of changed behavior.

To require the proposed equivalence criterion, append `-- --require-equivalence`.
That command deliberately fails on this baseline. The full plugin also has an
ignored `pm_audit_linear_phase_ramp_matches_direct_tuning` acceptance test and a
normal constant-periodic-offset regression test in `render.rs`. The full plugin
test was not executed here because its sibling `derpcat-access` dependency is
unavailable; the isolated production-kernel harness was executed successfully.

`results.json` records the Python run including the Rust four-stage sample dump.
Its error is total normalized reconstruction error, **not an alias-only metric**:

| Input trajectory | Error relative to reference | 64x versus 128x reference error | Evidence |
|---|---:|---:|---|
| Static saw | -25.19 dB | -96.23 dB | Float64 equation |
| Sine PM into saw | -12.78 dB | -50.78 dB | Float64 equation |
| Sine PM into sine PM into saw | -12.87 dB | -50.77 dB | Float64 equation |
| Three nested sine sources into saw | -12.69 dB | -50.87 dB | **Production x8 kernel** |
| Saturating audio-rate depth into PM | -6.21 dB | -44.30 dB | Float64 equation |

The four-stage path is `sin(53t)` into `sin(211t + …)` into `sin(997t + …)`
into a saw with carrier 83 cycles per FFT record. Phase indices are 1.1 rad,
1.4 rad, and 0.22 cycles respectively. The record has 16384 samples. At 48 kHz
these are approximately 155, 618, 2921 and 243 Hz. This stresses **nested
audio-rate PM**, including 7844 negative phase increments. The source signals are
analytically evaluated sine functions fed to the real final kernel; this is not
an execution of the complete production generator graph or its sine approximation.
The production output agrees with the float64 model within `1.205e-5` peak units.

The reference generates the continuous periodic target at 64x/128x rate, uses an
ideal FFT low-pass and downsamples. All frequencies are coherent; no window,
gain fitting, or phase alignment hides errors. Nyquist is removed consistently.
The agreement of two reference resolutions is a convergence check, not a proof
of exactness. Reconstruction error includes smoothing of wanted harmonics as
well as aliasing, and the static control makes that limitation visible.

An additional, strictly alias-specific **ideal sine PM model** has carrier 1 kHz,
modulator 9 kHz and depth 0.2 cycles at 48 kHz. Its 28 kHz sideband folds to 20 kHz,
measured at **-28.54 dBFS**. The continuous spectrum lies only at
`±(1000 + k*9000)` Hz; 20 kHz is not in that set. This example demonstrates why
fixing saw-edge BLEPs alone cannot solve PM aliasing. It is not a measurement of
KURV's complete sine oscillator or oversampler.

## Live production call chain and additional findings

1. `voices/voice/block_render.rs::render_generator_time_grouped_block` walks
   `generator_routes.order()`, generates source taps, then builds the target's
   per-frame phase modulation via `accumulate_phase_block`.
2. It converts the phase position to `shortest_phase_delta(initial_phase, target)`
   before dispatching to `accumulate_spline_saw8_phase_modulated_lanes_block`,
   `accumulate_spline_saw8_phase_modulated_block`,
   `accumulate_shape8_phase_modulated_block`, x4, or time-x8/tail paths.
3. Those functions in `oscillators/va/render.rs` preserve carrier step when
   sampling the offset phase. `generate_shape_time8` does the same for single
   oscillator time-SIMD. Do not fix only the unison-x8 variant.
4. `voices/poly_synth.rs::block_amount` adds per-frame depth-source signals then
   clamps the result to `[-1, 1]`. Audio-rate depth saturation creates extra
   bandwidth even with smooth sine inputs. The clamp is a bounded-control
   contract, not inherently a programming bug; replacing it would change timbre.
5. `enable_fast_muted_sources` excludes `target_mask` and feedback sources.
   Thus the lane-collapse fast path does **not** silently bypass PM on a nested
   target. For eligible muted root sources with unison, however,
   `render_fast_muted_source_tap` renders only lane zero and advances other lane
   phases without rendering them. This is a deliberate **sound-changing quality
   tradeoff**, not mathematically equivalent SIMD acceleration. Benchmark and
   audition it separately from exact-output optimizations.

The block path rejects unsupported settings and falls back to ordered per-frame
rendering. Feedback has separate delayed-source semantics and is not covered by
these feed-forward measurements. Testing a feed-forward nested chain cannot
establish feedback stability or equivalence across the fallback.

## Why an instantaneous-step substitution is not shipped here

`carrier_step + pm[n] - pm[n-1]` on the already wrapped offsets produces false
near-one-cycle jumps. Using shortest wrapped differences also loses true motion
over half a cycle. Signed negative increments disable current positive-step BLEP
logic; merely taking an absolute value still assumes locally linear motion over
the correction support. Reversals, multiple crossings, changing slope, changing
pulse width and modulation sidebands remain unresolved. A one-line change could
appear to improve a static FFT while breaking nested patches or block boundaries.

For background on waveform edge corrections and their limits, see Pekonen et al.,
[Nonlinear-Phase Basis Functions in Quasi-Bandlimited Classical Waveform Synthesis](https://www.dafx12.york.ac.uk/papers/dafx12_submission_15.pdf).
For explicit PM/phase-shaping definitions, see Smart,
[Wave Pulse Phase Modulation](https://dafx.de/paper-archive/2025/DAFx25_paper_48.pdf).
The code-specific conclusions and numerical results above come from this audit.

## Cheapest useful next steps, with acceptance gates

| Strategy | CPU impact | Limitation / required test |
|---|---|---|
| Preserve existing no-PM specialized kernels | No new per-sample work | Zero/constant PM must match current output |
| Compile route topology and specialize complete feed-forward islands | Amortizes dispatch and parameter work | Exact same taps/order; test 1, 4, 8, 16, 32, 64 lanes |
| Prototype trajectory-aware event correction only for PM targets | Pays only for modulated edges | Unwrapped state, signed crossings, reversals, block-boundary tests |
| Oversample the **whole dependent modulation island** | Cost scales with rate and island size | Recompute every source/depth at that rate; don't just interpolate final PM |
| Restrict modulation bandwidth/depth as an optional quality policy | Potentially cheap | Changes timbre; never call it exact optimization |
| Collapse muted unison modulators | Already implemented fast tradeoff | Measure timbral difference separately; preserve full-quality mode |

Immediate workaround: use higher internal oversampling and lower modulation
depth/bandwidth; avoid repeatedly saturating audio-rate depth when cleanliness is
the goal. Sine modulators reduce source bandwidth but **do not guarantee clean
output**. Output filtering cannot remove aliases that have already folded into
the passband. No factor currently available should be advertised as alias-free
for arbitrary nested modulation.

For each future candidate, report both quality and cycles per generated sample
at 44.1/48/96 kHz, 1/2/4x internal rate, single/unison lanes, shallow/deep PM,
reverse motion, transients and changing depth. Include zero-PM regressions and
worst-case callback duration, not only steady-state kernel averages. In
particular, measure equal-quality configurations before calling one faster.
This audit does not establish superiority over any competing synthesizer.
