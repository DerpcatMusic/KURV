# Rich full-source-time timeline designs — 2026-08-23

## Decision scope

This is a design-only report. No Rust source was changed. The goal is a Rich
artifact that retains the complete source time axis, while keeping worker and
audio-thread work bounded. “Full-source-time” means that the first, middle,
and last source regions are represented in order and that playback duration is
not silently shortened. It does **not** mean that arbitrary source duration
and full-bandwidth 48 kHz output can both fit in a fixed byte budget.

The fixed production-artifact budget used for the comparison is the current
Rich safety envelope of 6 MiB. The current payload is:

```text
22 zones × 32,768 samples × 4 bytes = 2,883,584 bytes (2.75 MiB)
```

Each zone currently contains eight 4,096-sample waveform frames. The eight
frames are selected as source snapshots and then looped as eight uniformly
spaced timeline phases. They are not a full source-time sequence. The current
persisted Rich payload also has 22 centers, 22 fundamental bins, and (in newer
packs) eight frame gains. `artifact_persisted_bytes`, the codec, and restore
logic all assume this shape.

The source decode and source-audition buffers are separate allocations. The
budget below is for the immutable Rich production artifact, not for the
already-decoded source audition. `MAX_RESYNTH_DECODED_FRAMES` is much larger
than any one Rich output budget, so whole-plugin memory accounting must remain
separate.

## Design 1 — Uniform time-domain sequence with periodic mips (recommended)

### Representation

Build one derived Rich waveform for the complete source duration instead of 22
independent pitch-zone slabs:

```text
RichSequenceArtifact {
    source_sample_rate: f32,
    source_frames: u32,          // original duration authority
    rendered_frames: u32,        // fixed-capacity derived sequence
    rendered_sample_rate: f32,   // rendered_frames / source duration
    root_hz: f32,
    samples: Box<[f32]>,
    periodic_integral: Box<[f64]>,
    periodic_mips: Box<[PeriodicMipLevel]>,
}
```

Use `RICH_MAX_RENDER_FRAMES = 131,072` for the first bounded implementation.
For a 48 kHz source this gives 2.7307 seconds at native rate. If the source is
longer, first create a band-limited projection with exactly 131,072 output
frames and set:

```text
rendered_sample_rate = 131,072 / source_duration_seconds
```

The output still covers the complete duration; only its effective bandwidth is
reduced. Do not truncate the tail or claim full 17.2 kHz bandwidth after this
projection. A UI/visual model should expose the effective rate or a reduced
bandwidth warning.

The base sequence is overlap-added from every bounded analysis frame. A
practical worker setting is a 4,096-sample Hann analysis window and a 512- or
1,024-sample hop in the projected source coordinate. Phase tracking and the
existing Rich controls happen off-thread. Normalize the complete reconstructed
sequence once, after applying a smoothed frame-level loudness curve. Do not
peak-normalize each frame independently, because that destroys the source
amplitude timeline.

Periodic mip samples and their integrals are built before publication. With the
current periodic mip layout, 131,072 base frames require approximately
3,145,840 bytes for the base, base integral, and derived mip/integral levels.
This is about 3.00 MiB, leaving roughly 3 MiB in the 6 MiB envelope for metadata,
future visual summaries, and alignment. The exact total depends on the final
mip stopping rule; the compiler should retain a checked byte calculation.

### FFT resolution and source-time fidelity

At a native 48 kHz effective rate, a 4,096-point FFT has 11.72 Hz bins. This
keeps the current Rich analysis resolution while retaining all source-time
regions. The 512-sample hop gives much finer temporal coverage than the current
eight snapshots. The final waveform is one contiguous sequence, so an impulse
or change in the middle is not lost merely because it fell between eight
windows.

For a source longer than 2.7307 seconds, the output rate falls in proportion to
duration. This is the unavoidable fixed-memory trade-off: a 60-second source
with 131,072 output frames has an effective rate of about 2.18 kHz and cannot
retain 17.2 kHz content. It does retain the complete timeline and makes the
quality reduction explicit. A larger fixed cap can raise this boundary, but it
cannot remove the duration/bandwidth trade-off.

### Worker cost

The worker performs source projection, approximately 256 bounded FFT frames at
full capacity with a 512-sample hop, phase/harmonic processing, overlap-add,
and mip construction. Source projection cost is linear in decoded source size;
all retained analysis and output allocations are bounded by the cap. Compared
with the current eight FFT analysis frames and 22-zone rendering, there are
more analysis frames but only one time-varying output sequence instead of a
separate IFFT bank for every zone/frame. Cancellation should be polled per
analysis frame and per output/mip block.

### Realtime lookup

The callback keeps the existing fixed-work shape of a one-shot/periodic sample
reader:

1. derive source phase from elapsed voice time and `source_frames/source_rate`;
2. multiply by `rendered_frames`;
3. select the appropriate mip from the source step; and
4. cubic/interpolated-read the base or mip level.

Lookup is allocation-free, lock-free, and independent of FFT size. It is
constant bounded work plus the fixed mip-level selection. A wrap seam must be
made periodic in the worker artifact; otherwise a full timeline can still click
at the loop boundary. Ordinary pitch and playback controls remain cheap phase
deltas.

### Persistence compatibility

This needs a new Rich payload variant and a pack-version branch. The existing
Rich wire payload cannot be decoded as a sequence: it has no rendered-frame
count/rate and its 22×32,768 values have different semantics. Keep the legacy
Rich decoder for old packs and write a new version containing at least
`source_frames`, `rendered_frames`, `rendered_sample_rate`, `root_hz`, and the
base sequence. Mips can be regenerated deterministically during restore to
reduce pack size, provided restore is treated as worker work and the generated
mips are covered by parity tests. Existing packs must remain playable through
the legacy eight-frame artifact; do not reinterpret them as a new timeline.

## Design 2 — Fixed time-frequency zone tile bank

### Representation

Keep the current 22 keyboard zones and the exact 2,883,584-byte waveform
payload, but change the tile shape to 32 timeline frames × 1,024 samples per
zone:

```text
22 zones × 32 frames × 1,024 samples × 4 bytes = 2,883,584 bytes
```

Store 32 normalized source positions (or derive uniform centers), 32 frame
gains, and the existing zone center/bin metadata. Every source interval has a
corresponding tile. Worker analysis windows are centered on those positions and
are overlapped so that a transient is not sampled only at an interval edge.
Runtime crossfades adjacent tiles at their actual source positions rather than
assuming the old eight cyclic phases.

A 64×512 variant gives twice the temporal tile count but a 93.75 Hz periodic
waveform-bin spacing. The 32×1,024 variant is the better fixed shape: its
periodic tile resolution is 46.875 Hz at 48 kHz. Both use the same bytes; this
report compares the 32×1,024 form.

### FFT resolution and source-time fidelity

A 1,024-sample periodic reconstructed tile has 46.875 Hz bins. A larger worker
analysis window can improve measurement, but it cannot remove the tile's
periodic-bin and phase-resolution limit. The 32 timeline points cover the whole
source and are a clear improvement over eight points, but a fast event between
centers is still smeared over neighboring tiles. This is “full duration with a
coarse grid,” not sample-time fidelity.

### Worker cost

The worker analyzes 32 source positions and renders 32×22 tiles. Relative to
the current eight-frame/22-zone bank, this is approximately four times the
frame FFT/IFFT work and still includes the zone multiplier. It is easy to bound
and can reuse much of `render_rich_zone`, but the 22-way duplication remains a
large worker cost for data that shares one source timeline.

### Realtime lookup

The callback computes a bounded timeline index, reads two adjacent 1,024-sample
tiles from the selected zone, and crossfades them. It needs no FFT, allocation,
lock, or scan. Zone selection remains the current hysteretic O(1) path. This is
the cheapest runtime representation of the three, with two or three waveform
reads per output sample.

### Persistence compatibility

This is the closest to the current wire shape because the float payload remains
exactly 22×32,768 samples. It still needs a new pack version: old packs contain
eight 4,096-sample frames, and interpreting their bytes as 32×1,024 tiles would
play the old frames in four unrelated pieces. A dual decoder can keep the old
`RichZoneArtifact` for old versions and use the same-size tile payload for the
new version. New metadata must identify the timeline count and coordinate
policy; do not infer it from payload length alone.

## Design 3 — Shared spectral descriptor timeline

### Representation

Store a much denser source-time grid of compact spectral features and synthesize
at runtime. A concrete bounded record is:

```text
K = 512 source-time frames
P = 48 tracked harmonic records per frame
R = 64 residual-envelope bands per frame

harmonic: { ratio: f32, amplitude: f32, phase: f32 }  // 12 bytes
residual: [f32; 64]                                  // 256 bytes
loudness: f32, flags/count: u32                       // 8 bytes
```

The records consume about `512 × (48×12 + 64×4 + 8) = 430,080 bytes`, plus
positions, root, rate, and version metadata. The representation is well below
1 MiB and leaves room for more partials, quantization guards, or a compact
visual summary while staying under the 6 MiB Rich cap. Frequencies are stored
as measured ratios or absolute values, not as coarse periodic waveform bins.
The worker tracks phase across adjacent frames and stores a deterministic
residual model; it does not ask the callback to run an FFT.

### FFT resolution and source-time fidelity

Use a 4,096-point Hann analysis window (11.72 Hz bins at 48 kHz) and map 512
frame centers across the full duration. This has the best measured frequency
resolution of the three designs and the densest timeline grid. It is still a
model: only 48 harmonic tracks and 64 residual bands survive. Dense noisy or
polyphonic material may lose partials, and residual synthesis must be validated
against the current Rich controls.

### Worker cost

The worker performs up to 512 FFT analyses, peak selection/tracking, phase
unwrapping, and residual-envelope extraction. This is roughly 64 times the
current eight-frame analysis count, although it avoids 22 separate waveform
IFFTs. It is bounded at K and naturally cancellable per frame. Descriptor
compilation is more algorithmically risky than either waveform-bank design.

### Realtime lookup

The callback indexes two adjacent records and advances a fixed set of oscillator
phases. It performs a fixed `P + R` amount of work (about 112 partial/residual
updates per sample in this concrete form), with no allocation or FFT. This is
not as cheap as a waveform read and requires per-voice phase state. Frame
identity and phase interpolation must be fixed-size; a data-dependent peak
loop is not acceptable on the callback.

### Persistence compatibility

This is a new wire representation. The existing zone centers, fundamental bins,
and float slabs cannot carry the descriptor meaning without a lossy migration.
Add a new pack version and retain the old decoder. Persisting descriptors is
smaller than persisting a waveform sequence, but persistence tests must cover
phase continuity, missing/quiet partials, residual-band bounds, and fresh-build
versus restored output.

## Comparison

| Design | Fixed payload example | FFT / frequency resolution | Worker cost | Realtime lookup | Persistence compatibility |
|---|---:|---|---|---|---|
| 1. Sequence + mips | 131,072 base frames + periodic mips ≈ 3.00 MiB | 4,096 analysis bins: 11.72 Hz at 48 kHz; long sources down-project effective rate | Bounded projection + ~256 analysis/reconstruction frames + mips; source-length linear only during worker build | One phase/index and one periodic-mip read; no FFT; O(1) bounded | New sequence variant/version; old Rich packs stay on legacy decoder |
| 2. Zone tiles | 22×32×1,024 f32 = 2,883,584 B | 1,024 periodic tile bins: 46.875 Hz; 32 timeline points | ~4× current 22-zone frame work | Two adjacent tile reads and crossfade; O(1) | Closest byte shape; still requires version and dual decoder |
| 3. Spectral descriptors | 512×48 partials + 64 residual bands ≈ 0.43 MiB | 4,096 analysis bins: 11.72 Hz; dense 512-frame timeline | Up to 512 FFTs + tracking; bounded but algorithmically heavy | ~112 fixed oscillator/residual updates per sample | New descriptor payload/version; old decoder required |

All three avoid callback allocation, locking, source I/O, and FFT. None can
promise native full-bandwidth output for arbitrarily long sources under a fixed
cap. Design 1 makes this limitation easiest to state because its effective
sample rate is explicit.

## Recommendation

Implement **Design 1, uniform time-domain sequence plus periodic mips**, with a
6 MiB Rich production-artifact cap and an initial `RICH_MAX_RENDER_FRAMES` of
131,072. It is the safest implementable boundary for KURV:

- it retains the complete source duration instead of looping eight snapshots;
- it keeps the audio thread on the existing bounded sample-reader model;
- it preserves 4,096-point worker FFT resolution at native rate;
- it avoids 22× duplication and per-voice additive spectral synthesis;
- it makes long-source bandwidth loss measurable rather than hidden; and
- it can be published as one immutable artifact with deterministic restore.

Design 2 is a useful fallback only if retaining the current 22-zone API and
payload shape is more important than timeline fidelity. Design 3 should remain a
later experiment: its memory is attractive, but callback CPU, phase state,
residual modeling, and persistence parity create more failure modes.

The implementation should be staged after this design decision (not in this
report):

1. Add a worker-only sequence compiler and a byte-budget assertion.
2. Add first/middle/last and transient-order tests at short and long source
   durations, including loop seam and effective-rate metadata.
3. Add a new persisted Rich sequence version while retaining the old decoder.
4. Compare fresh-build and restored rendered output before switching the
   publication path.
5. Verify scalar/block parity and bounded audio-thread lookup; never put FFT,
   mip construction, or source projection in the callback.
