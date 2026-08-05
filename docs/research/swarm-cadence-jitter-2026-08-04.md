# Swarm cadence and Jitter integration report

Date: 2026-08-04

## Outcome

Use a maximum Wander/Jitter target interval of **1,024 DSP samples**. It is the
Pareto point in the requested `256 / 512 / 1024 / 2048 / formula` sweep: it
captures almost all remaining target-generation savings while keeping the
slow-rate modulation trajectory close to the prior 32-sample reference.

The second Swarm family is a **stateless Jitter** implementation. It does not
add the rejected per-lane countdown array or enlarge `VaVoice`; default Wander
retains its existing hot layout. Jitter is wired as a real host parameter and
the Unison visualization intentionally stops showing Wander-only pan motion
when Jitter is selected.

The live tree now contains the selected ceiling and stateless Jitter integration.
The two preserved integration artifacts are:

- `/tmp/kurv-swarm-ceiling-1024.patch`
  - SHA-256: `3c1c9ba7b0d6a80441168c74b13c8b39763a70f161501675392c24dbf1f10b63`
- `/tmp/kurv-swarm-jitter-optional.patch`
  - SHA-256: `2f06beec0af0a2e2cccdccf6b0ea4045bb446eb1ff5eb21212fdfc2d63e8e0c9`

Jitter applies after the 1,024-ceiling patch. The final live tree additionally
caches `swarm_rate / sample_rate`, an exact CPU refinement developed in the
parallel general-engine investigation.

## Cadence sweep

The interval remains:

`floor(sample_rate / (rate * 3.731 * 1.55 * 8))`, clamped to `32..=1024`.

At 48 kHz the selected intervals are 1,024 samples at Rates 0.02 and 0.7,
259 at Rate 4, 129 at Rate 8, 64 at Rate 16, and 32 at Rate 32. Rate 32 keeps
the explicit 32-sample path and is byte-identical between the 256 and 1,024
ceilings. Both 100,000-sample hashes are
`f0d277e1bd96a071a31778177431a3a062e43309505d46327548ad5c4417db98`.

The 10-second, 64-lane trajectory sweep compared every candidate with the
prior 32-sample target trajectory:

| Rate | Ceiling | Interval | Pitch RMS null | Pitch correlation | Pitch-band energy | Spectral null |
|---:|---:|---:|---:|---:|---:|---:|
| 0.02 | 256 | 256 | 0.0333 cents | 0.999913 | 0.99988 | -34.18 dB |
| 0.02 | 512 | 512 | 0.0521 cents | 0.999788 | 1.00042 | -27.43 dB |
| 0.02 | **1,024** | **1,024** | **0.0831 cents** | **0.999461** | **1.01909** | **-17.55 dB** |
| 0.02 | 2,048 | 2,048 | 0.1381 cents | 0.998513 | 1.08130 | -9.62 dB |
| 0.02 | formula | 51,875 | 2.0612 cents | 0.605266 | 7.08721 | +8.38 dB |
| 0.7 | 256 | 256 | 0.0413 cents | 0.999870 | 0.99990 | -38.95 dB |
| 0.7 | 512 | 512 | 0.0743 cents | 0.999577 | 0.99991 | -32.25 dB |
| 0.7 | **1,024** | **1,024** | **0.1365 cents** | **0.998572** | **0.99966** | **-25.84 dB** |
| 0.7 | 2,048/formula | 1,482 | 0.1913 cents | 0.997192 | 0.99850 | -22.43 dB |

At Rates 4, 8, 16, and 32, all ceilings at or above 512 select the same
formula intervals. Their pitch correlations versus the 32-sample reference
are 0.998424, 0.998867, 0.999506, and 1.0 respectively. Total stereo lane
energy for the selected ceiling stayed within 0.0007% of the reference across
the sweep; the Rate 0.7 pan correlation was 0.999942.

On the generic 64-unison x 32-note stress cell at Rate 0.7, retired
instructions relative to ceiling 256 fell by 9.7% at 512, 14.6% at 1,024, and
16.1% at 2,048. Median wall time was best at 1,024: 3,108 ns/sample versus
3,707 at 256, 3,255 at 512, and 3,219 at 2,048. The extra 2,048 saving was only
1.8% retired instructions and did not improve wall time. On x86-64-v3, 1,024
reduced retired instructions by 18.1% and cycles by about 19% versus 256 at
Rate 0.7.

The final production-shaped x86-64-v3 saw harness, including oscillator,
polyphony, 2x decimation, and the cached clock step, compared with the frozen
pre-adaptive engine as follows:

| Unison x notes | Frozen Wander ns/frame | Live Wander | Reduction | Live Jitter |
|---:|---:|---:|---:|---:|
| 1 x 1 | 51.856 | 47.278 | 8.8% | 46.887 |
| 8 x 8 | 684.369 | 461.377 | 32.6% | 453.611 |
| 64 x 32 | 13,930.836 | 6,975.524 | 49.9% | 6,800.944 |

At 48 kHz, the dense live Wander cell consumes about 33.5% of one core versus
66.9% for the frozen engine. A five-repeat hardware-counter run reduced cycles
from 4.377 to 2.167 billion, instructions from 11.568 to 5.265 billion, and
branches from 1.001 billion to 608 million. This is an isolated engine result,
not a substitute for a restarted-host DUNE/KURV comparison.

The unclamped formula is rejected. At Rate 0.02 it creates a 51,875-sample
slew (1.08 seconds at 48 kHz), badly distorts the short-window modulation
spectrum, and loses trajectory correlation.

## Stateless Jitter behavior

Wander remains parameter value 0 and the preset-compatible default. Jitter is
value 1 on new integer parameter ID 47. Existing parameter IDs are unchanged;
old state without ID 47 resolves to Wander.

For lane `i`, Jitter derives event times and pitch targets from the existing
lane seed and integer event cell:

- event spacing is deterministically bounded to `0.5..1.5 / Rate`;
- target pitch is a deterministic bipolar hash scaled by Amount and the
  existing per-lane depth;
- target changes use the adaptive 32..1,024-sample linear slew;
- pan and gain remain static in Jitter.

This distribution and slew are KURV choices, not claims about Ableton's
undisclosed implementation. At Rate 0.7 the event interval is approximately
0.714..2.143 seconds and the selected maximum slew is 21.33 ms at 48 kHz. At
Rate 32 the event interval is approximately 15.6..46.9 ms and the slew remains
32 samples / 0.667 ms. With the 24-cent stress setup, the largest possible
target jump is 20.16 cents.

No allocation, lock, I/O, syscall, logging, atomic, or unbounded loop was
added to rendering. Oscillator phase, MPE pitch bend, note identity, envelope,
pan normalization, and energy normalization contracts are preserved. The
stateless design adds **0 bytes** to `VaVoice` (3,088 bytes before and after),
replacing the obsolete rejected 128-byte-countdown prototype.

Mode transitions are value-continuous for every active polyphonic voice, not
only voice 0. Wander to Jitter freezes the current ratio while pan glides back
to the static layout, then begins deterministic Jitter targets. Jitter to
Wander retargets from current values. When Swarm Amount is zero, mode changes
reset immediately to the static/no-motion state so no pending transition can
freeze indefinitely. These operations occur only on configuration changes and
add no default-Wander steady-render work.

The host UI uses the existing Rate row for `RATE` and `MODE`. Jitter mode sets
the Unison preview's Wander motion amount to zero and therefore also avoids
the Wander-only 33 ms repaint loop; it does not falsely animate pan that the
audio mode keeps static.

## Live integration evidence

The current live tree received the 1,024 ceiling followed by Jitter, then the
exact cached-clock-step optimization. This preserved the current default Antialiasing value 1
(Spline), the current factor-aware Spline correction, and the policy that
defers oversampling-factor changes while voices or the decimator tail are
active.

Integrated source hashes:

- `src/voice.rs`: `f3b1a35483a26799aa67e13a66f6a75a44746396679deca29eb922b031a7c3e4`
- `src/lib.rs`: `12291ad277ff39671c4258f177fe52d08ac6f2f37efd818d285d6f33c1a4b347`
- `src/editor.rs`: `2cdc652b9be151f8f56b59e948ebb8499aca27fb983b7573fd621b36332d95d6`
- `src/editor_controls.rs`: `c85b84b28110ae905b9fffb589908f1fef1156aebd9005b005054bed8945d35e`
- `src/editor_unison.rs`: `e5b1ce09d4985c633670999f96d4de5d6a77d3d39420af0cd36e2f075335aedd`

Verification on the integrated live tree:

- `git apply --check` against current live source: pass for both patches;
- ceiling then Jitter apply sequence: pass;
- `cargo fmt --all -- --check`: pass;
- `cargo check --all-targets --locked`: pass;
- `cargo test --lib --locked`: 8/8 pass;
- `cargo test --lib --locked --features rt-paranoid`: 8/8 pass;
- 32-active-note Wander-to-Jitter render: boundary deltas remained below the
  local p99 on both channels, with stable before/after RMS and no runaway;
- no permanent tests were added; the existing suite and diagnostic harnesses were used.
