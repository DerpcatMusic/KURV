# Spectral job-local scheduling experiment — 2026-08-06

## Finding

One incompatible high note kept `VoiceSettings` in true Spectral mode and disabled KURV's
block-major renderer and internal real-time pool for the complete synth. The oscillator was
already voice-local: each low-frequency SIMD pack used the exact SplineOptimized fallback while
high packs retained true Spectral synthesis. The CPU cliff came from global scheduling, not from
the oscillator algorithm.

The retained experiment allows Spectral settings through the existing block scheduler and
preallocated worker pool. Spectral jobs use the generic per-sample voice kernel, so compatible
lanes keep their existing exact fallback and incompatible lanes keep true Spectral output. Worker
shadows copy and commit the existing fixed-capacity Spectral harmonic caches. No parameter IDs,
compatibility hysteresis, oscillator equations, allocation, locks, or collection growth changed.

## Measurement

- Source baseline: `origin/main` at `44d15a6ca8f380f5a666633958933e5d849a3b88`
- Release profile, no default features
- Portable target: `-C target-cpu=x86-64`
- v3 target: `-C target-cpu=x86-64-v3`
- Workload: Spectral, Normal 2x, saw, 3 oscillators, 64 unison lanes, 24 held notes,
  65,536 timed host frames, 7 repeats
- Notes 36–59 are all fallback-compatible; 48–71 are mixed; 72–95 are all true Spectral.

| Target | Chord | Main ns/frame | Candidate ns/frame | Change | Fallbacks |
|---|---|---:|---:|---:|---:|
| x86-64 | compatible | 1,614.693 | 1,512.275 | -6.34% | 0 |
| x86-64 | mixed | 24,303.321 | 5,706.793 | -76.52% | 0 |
| x86-64 | high | 44,526.085 | 9,622.999 | -78.39% | 0 |
| x86-64-v3 | compatible | 900.024 | 898.612 | -0.16% | 0 |
| x86-64-v3 | mixed | 15,819.047 | 4,212.570 | -73.37% | 0 |
| x86-64-v3 | high | 20,638.215 | 4,567.319 | -77.87% | 0 |

All candidate checks reported participation from each selected helper. Serial-versus-pool
comparisons over 131,072 host frames for compatible, mixed, and all-high chords produced zero bit
mismatches, zero maximum absolute error, and zero deadline fallbacks.

## Retention decision

Retain. The mixed workload exceeds the required 5% win on both targets, the established compatible
path remains within the 2% regression budget, and output is bit-exact. Complexity is limited to the
eligibility correction and copying the Spectral cache state that worker shadows now advance.
