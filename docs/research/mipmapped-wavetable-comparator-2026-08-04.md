# KURV mipmapped wavetable comparator

Research report — 2026-08-04

## Verdict

Reject this wavetable design as KURV's universal oscillator core.

The strongest production-shaped candidate—a cached-level, 2,048-sample, SIMD linear wavetable—was
spectrally excellent, allocation-free in the audio path, and faster than optimized Spline2 for one
oscillator and one 8-voice note. It stopped winning as oscillator count rose. At 32 notes it was
`1.90x`, `2.24x`, and `2.38x` slower than Spline2 at 1, 8, and 64 unison. Pulse, implemented from two
phase-shifted saw reads, was viable mathematically but `2.41x` slower than Spline2 at 32 notes / 8
unison. Cubic interpolation was dominated outright.

This is a cache/gather-cost rejection, not a sound-quality rejection. The table core is a credible
low-polyphony alternate, but that narrower product is not KURV's current workload. No live DSP source
was changed and no production patch is proposed.

## Frozen comparator

The experiment used a source snapshot at:

```text
/tmp/kurv-wavetable-comparator.i3Go7o
```

Relevant captured production hashes were:

```text
src/oscillator.rs   2a972e326f86276e967b113ecc8db168fa721bd0b52ac519fe229bca1ad430fe
src/voice.rs        1a24f7000466f80a3cb577118d9da327955956a3bdf5010e24f01bfa0649f0db
src/oversampling.rs ccf7a4a5062989f15994ad9ba36009d237b633e244a6b9b49b0637aac1999e98
src/lib.rs          a2ff7e49c9eab3300bec68fe4f59a2f504ed92fb0e4c7e47adec1cee20826648
```

The diagnostic sources are not production tests:

```text
examples/wavetable_lab.rs              sha256 30d6c70a8c55f90d79cc41fabbdc7556acd4ee5f0e4e48d183f4124d6843129e
scripts/analyze-wavetable-grid.py      sha256 6edca3d661c3d0a0679d4b8a8754f4ba2ff57c654125a0ad8c5ae0d5983d9390
```

The comparison ran on an AMD Ryzen 7 7800X3D, Linux x86-64, Rust 1.97.1 / LLVM 22.1.6, in Cargo's
release profile. The snapshot came from repository commit `6fd7240` plus the captured working-tree
state above.

## Candidate design

- Saw and triangle banks, built off the audio thread in `f64` and stored as `f32`.
- 2,048 points per level with one pre-guard and two post-guards, so the lookup wraps without branches.
- 265 harmonic levels: every integer count from 0 through 256, then 320, 384, 448, 512, 640, 768,
  896, and 1,023.
- 4,348,650 bytes for both banks including level metadata; about half that for saw alone. Startup bank
  construction took about 17 ms.
- The harmonic-level selection is cached per oscillator. The audio lookup performs no allocation,
  lock, I/O, or table construction.
- Scalar index gathering into fixed arrays followed by `f32x8` / `f32x4` linear or Catmull-Rom
  interpolation. This models the available SIMD path honestly: interpolation vectorizes, but
  independent table addresses still have gather/cache cost.
- A hard level policy and a continuous policy were measured. The continuous policy fades the newest
  harmonic over the top quarter of its available Nyquist clearance.
- Pulse at width `w` uses `saw(p + 1 - w) - saw(p) + 2w - 1`. Triangle uses its own bank. The
  Triangle-Saw and Saw-Pulse interiors use ordinary endpoint morphs.
- Legacy2 and the live optimized Spline2 both run through the current 2x oversampler. The wavetable
  candidate runs direct at 1x through the existing factor-1 delay path, preserving host latency. That
  gives the candidate the intended architectural advantage; it still loses at representative
  polyphony.

## Coherent spectral result

Each render used 65,536 samples at 48 kHz and coherent, odd/coprime FFT bins near 3, 5, 8, 12, 16,
and 20 kHz. The analyzer fits the analytic Fourier series, removes one global fundamental gain/phase,
and separately reports non-wanted energy, wanted-band residual, harmonic amplitude error, phase
error, and DC error.

Saw non-wanted energy, dBc:

| Fundamental | Legacy2 | Spline2 | table linear, smooth | table cubic, smooth |
|---:|---:|---:|---:|---:|
| 3.002 kHz | -50.29 | -83.12 | -109.39 | -144.52 |
| 5.000 kHz | -47.12 | -78.97 | -115.82 | -145.53 |
| 8.000 kHz | -48.42 | -89.07 | -123.03 | -144.91 |

At those same three frequencies, linear-smooth saw wanted-band residual was `-102.86`, `-109.96`,
and `-119.57 dB`; peak retained-harmonic amplitude error was at most `0.00033 dB`; retained-harmonic
phase error was effectively zero. Pulse from shifted saws reached `-111.00`, `-117.52`, and
`-124.96 dBc`. The separate triangle bank reached `-123.09`, `-126.09`, and `-129.02 dBc`. Both
morph regions retained the same basic quality.

The hard level switch preserves every legal harmonic in a static tone, but it changes harmonic count
discontinuously under pitch movement and was therefore not considered production-safe. The smooth
policy avoids that switch. Its intentional cost is attenuation near a harmonic's Nyquist entry: the
second harmonic is mostly absent around a 12 kHz fundamental, and the fundamental is `-0.961 dB` at
20.003 kHz. This behavior must be judged as the anti-alias policy, not as interpolation error.

Linear interpolation is already far below the current alias floor. Cubic buys roughly another
20–35 dB where the linear result is already inaudible, so its much higher CPU has no product value.

## CPU result

All values are median nanoseconds per host frame across nine release runs. One-note cases render
250,000 measured frames; 32-note cases render 20,000. Each run has a 4,096-frame warm-up. The 32-note
set spans MIDI note 36 upward in two-semitone steps, so it exercises many table levels rather than a
single hot table.

Saw:

| Algorithm | 1 note / 1x | 1 note / 8x | 1 note / 64x | 32 notes / 1x | 32 notes / 8x | 32 notes / 64x |
|---|---:|---:|---:|---:|---:|---:|
| Legacy2 | 36.692 | 40.889 | 144.163 | 63.246 | 361.957 | 2,561.437 |
| Spline2 | 41.505 | 61.191 | 256.943 | 99.084 | 623.313 | 4,713.709 |
| table linear, hard | 9.585 | 50.113 | 298.656 | 158.495 | 1,217.448 | 9,878.129 |
| table linear, smooth | 9.716 | 51.259 | 300.748 | 187.763 | 1,397.156 | 11,216.117 |
| table cubic, smooth | 13.188 | 88.604 | 636.892 | 351.759 | 3,147.151 | 25,789.385 |

The strongest table result is real but narrow: linear-smooth is `4.27x` faster than Spline2 for one
saw oscillator and about 16% faster for one 8-unison note. By 64 unison it is 17% slower. With 32
notes it is `1.90–2.38x` slower. At 32 notes / 64 unison, table linear-smooth alone consumes about
53.8% of a 48 kHz core deadline, versus 22.6% for Spline2 and 12.3% for Legacy2.

Other waveforms at 8 unison:

| Wave | Spline2, 1 note | table linear smooth, 1 note | Spline2, 32 notes | table linear smooth, 32 notes |
|---|---:|---:|---:|---:|
| Pulse | 87.825 | 84.030 | 1,183.291 | 2,849.896 |
| Triangle | 81.492 | 49.239 | 1,144.072 | 1,474.205 |
| Triangle-Saw | 106.097 | 88.000 | 1,500.767 | 2,953.836 |
| Saw-Pulse | 91.474 | 87.135 | 1,311.850 | 2,833.135 |

The shifted-saw pulse identity is therefore viable for sound quality and low voice counts, but two
independent reads amplify the table/cache problem. The two-bank Triangle-Saw morph has the same
issue. There is no universal endpoint or morph win.

## Stop condition and 1,024-point check

A second frozen variant halved each table to 1,024 points and reduced the two-bank footprint to
2,144,898 bytes. Linear-smooth saw medians were:

```text
1 note:   1x 9.608 ns, 8x 50.406 ns, 64x 306.066 ns
32 notes: 1x 174.423 ns, 8x 1,441.900 ns, 64x 11,453.278 ns
```

That did not improve the representative 8x or 64x workloads. It shows that simply shrinking the
bank is not the escape hatch; independent lookup/gather work is the dominant frontier. Further table
sizes, native-gather variants, or layout polishing were stopped because the candidate was already
dominated at the product workloads and the requested rejection condition had been met.

## Reproduction

From the frozen directory:

```bash
cargo build --release --example wavetable_lab
target/release/examples/wavetable_lab render-grid /tmp/kurv-wavetable-renders
python3 scripts/analyze-wavetable-grid.py
target/release/examples/wavetable_lab bench-suite > /tmp/kurv-wavetable-bench.csv
```

The analyzer writes `/tmp/kurv-wavetable-spectral.csv` and
`/tmp/kurv-wavetable-spectral.txt`. The benchmark writes one CSV-shaped result per case, including
median, min, max, frame count, repeat count, oscillator count, and a black-boxed checksum.

Decision: keep the current frontier. Revisit a table core only if KURV gains a deliberately
low-polyphony mode, or if the target SIMD/runtime acquires a gather path that demonstrates a full
32-note workload win—not merely a scalar lookup win.
