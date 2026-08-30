# Canonical coefficient constant-x8 probe (2026-08-30)

## Question

Can the already-supported narrow `cap <= 6` coefficient projection pay for its
1024-sample reversible bridge when it is restricted to exact saw and exact
50%-square constant-x8 packs?  This round deliberately does not add another
publication seam or touch the 33 oscillator call sites.

## Probe

The ignored probe in `src/oscillators/va/render.rs` keeps `[u16; 8]` selector
state outside `VaOscillator` (16 bytes per active x8 pack).  It calls the current
`SplineOptimized` canonical BLEP evaluator, evaluates a compile-time-shaped
16-cubic projected curve with an AVX2/FMA two-bank selector, and dual-renders
only while the selector is between 0 and 1024.  Phase advancement is identical
for both representations.  An ineligible pack slews back to current BLEP, so a
stale projected state cannot survive modulation.

The coefficient fits use exact Fourier projections at harmonic caps 6, 3, and
2.  This isolates runtime representation/economics; it is not a production
coefficient compiler and adds no audio-thread analysis, allocation, lock, or
I/O.

Exact command (CachyOS x86-64-v3, Rust release, 48 kHz, 64-frame blocks):

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test canonical_coefficient_x8_transition_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

## Results

The timings are median-of-five nanoseconds per host frame for one eight-lane
pack.  `continuous` reverses every 1024 samples.  The 100 ms and 500 ms cases
hold current BLEP for the existing 384-sample (8 ms) shape smoother, bridge for
1024 samples, then remain projected.

| shape | current BLEP | projected steady | continuous | 100 ms note | 500 ms note |
|---|---:|---:|---:|---:|---:|
| saw | 7.179 | 2.718 (-62.1%) | 10.727 (+49.4%) | 5.223 (-27.2%) | 3.675 (-48.8%) |
| square | 11.835 | 2.665 (-77.5%) | 15.924 (+34.6%) | 6.750 (-43.0%) | 4.104 (-65.3%) |

Projection accuracy against the exact cap-limited Fourier target over 65,536
phases:

| shape | cap | MIDI | RMS | peak | maximum absolute output |
|---|---:|---:|---:|---:|---:|
| saw | 6 | 105 | 0.000840670 | 0.002927840 | 1.033317208 |
| saw | 3 | 117 | 0.000095006 | 0.000265896 | 0.918541670 |
| saw | 2 | 123 | 0.000027395 | 0.000064850 | 0.826993644 |
| square | 6 | 105 | 0.000840124 | 0.001950145 | 1.188369751 |
| square | 3 | 117 | 0.000182054 | 0.000420153 | 1.200463176 |
| square | 2 | 123 | 0.000006821 | 0.000015497 | 1.273239732 |

Across rapid 137-sample transition reversals, the largest representation-only
sample increment was 0.000313103 for saw and 0.000279586 for square.  This is
well below the prior 0.0011 transition gate.  After 131,072 ineligible samples,
all eight selectors reached zero; the next eligible 64-frame block ended at
exactly 64.  Full eligibility latency is 384 + 1024 = 1408 samples (29.33 ms at
48 kHz), but audio remains continuous throughout the bridge.

## Verdict

**Narrow win; retain as a justified future seam, do not ship this probe.**
Steady exact-shape notes and realistic 100/500 ms duty cycles recover enough
CPU to pay for the dual transition.  Constant rapid eligibility churn loses,
so this must never become a general shape path.

The minimum credible production seam is one dedicated constant-x8 saw/square
renderer with 16 bytes of side selector state per active structural pack, three
immutable 256-byte coefficient frames per supported shape, exact canonical
eligibility, and the existing BLEP fallback.  Eligibility must exclude pulse
width other than 0.5, custom mix, warp, morph, and moving shape; loss of
eligibility reverses the same bounded bridge.  State belongs to the structural
pack scheduler, not `VaOscillator`, and should be allocated only for active
x8 packs.

This probe times the complete candidate/current generation and common phase
advance inside the real 64-frame constant-x8 inner seam.  It intentionally does
not claim an end-to-end voice/plugin speedup: gain accumulation, scheduling,
publication, automation, and host execution are common or absent here and must
be measured if that single seam is implemented.  Production DSP and version
remain unchanged.
