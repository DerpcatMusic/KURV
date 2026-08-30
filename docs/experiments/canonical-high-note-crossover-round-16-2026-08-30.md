# Canonical high-note crossover (round 16, rejected)

Date: 2026-08-30

Verdict: the proposed `>= 8 kHz` production crossover is rejected. No production DSP, state, publication layout, or scalar/x4 fallback changed.

## Actual renderer seam

The round-15 symbol probe used the generic varying-step x8 renderer as its current baseline. This round corrected the probe to call the actual block-stable production seam, `accumulate_saw8_block_constant`, including its runtime-selected AVX2/FMA backend. A small `cfg(test)` helper selects the host-detected backend before timing because unit tests deliberately default the global backend to baseline.

The candidate remains the reciprocal-hoisted support-3/degree-7 Estrin kernel. Selection is outside the sample loop and eligibility is limited to stable x8 blocks. Scalar, x4, varying-step pitch, modulation, warp, and transition blocks remain on current code by construction. The test probe has no audio-thread allocation, lock, I/O, or logging; setup and reporting remain outside the timed kernel.

## Release CPU gate

Host: Ryzen 7 7800X3D, rustc 1.98.0, native AVX2/FMA, pinned CPU 8. Each cell is 400,000 64-frame stereo x8 blocks and five `perf stat` runs.

```text
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native' \
  cargo test canonical_x8_symbol_probe --lib --release --locked --no-run

KURV_ASM_PROBE=current KURV_ASM_BLOCKS=400000 KURV_ASM_HZ=8000 \
taskset -c 8 perf stat -r 5 -e cycles:u,instructions:u,branches:u,branch-misses:u -- \
target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce \
oscillators::va::experiment::canonical_x8_symbol_probe \
--ignored --exact --nocapture --test-threads=1
```

The command was repeated for both kernels and every frequency:

| Hz | shipping constant x8 cycles | support-3 cycles | candidate delta |
|---:|---:|---:|---:|
| 7,000 | 387,716,507 +/-1.34% | 459,898,764 +/-2.13% | +18.6% |
| 8,000 | 386,189,468 +/-1.17% | 458,823,722 +/-4.67% | +18.8% |
| 9,000 | 397,444,147 +/-1.09% | 446,206,403 +/-0.94% | +12.3% |
| 10,000 | 425,915,522 +/-0.85% | 435,441,568 +/-1.06% | +2.2% |
| 12,000 | 521,807,491 +/-2.55% | 446,374,406 +/-3.60% | -14.5% |

Selector overhead is excluded from the candidate numbers. It therefore cannot repair the 8-10 kHz losses. Only the isolated 12 kHz cell wins, which does not satisfy the requested broad `>= 8 kHz` region.

## Ideal projection and artifacts

An offline coherent-cycle script evaluated the exact current and support-3 residual polynomials for saw, square, and pulse (31% duty) against analytic bandlimited Fourier coefficients. `wanted_db` is complex wanted-bin error energy relative to wanted energy; more negative is better. `unwanted` is energy outside DC and the ideal retained harmonic bins.

| Hz | shape | current RMS / wanted dB / unwanted | support-3 RMS / wanted dB / unwanted |
|---:|---|---|---|
| 6,857 | saw | 0.173628 / -9.61 / 0 | 0.051163 / -20.23 / 0 |
| 6,857 | square | 0.264299 / -11.10 / 0 | 0.053014 / -25.06 / 0 |
| 6,857 | pulse31 | 0.230006 / -11.42 / 0 | 0.053987 / -24.04 / 0 |
| 8,000 | saw | 0.156804 / -10.13 / 9.0e-19 | 0.018249 / -28.81 / 6.9e-34 |
| 8,000 | square | 0.169611 / -14.50 / 2.8e-33 | 0.014378 / -35.93 / 5.3e-31 |
| 8,000 | pulse31 | 0.281673 / -9.64 / 1.34e-4 | 0.052671 / -29.12 / 1.86e-3 |
| 9,600 | saw | 0.205042 / -7.80 / 0 | 0.029283 / -24.70 / 0 |
| 9,600 | square | 0.235016 / -11.67 / 0 | 0.034788 / -28.45 / 0 |
| 9,600 | pulse31 | 0.357856 / -7.56 / 0 | 0.033291 / -28.39 / 0 |
| 12,000 | saw | 0.170902 / -8.41 / 0 | 0.050211 / -21.10 / 4.73e-4 |
| 12,000 | square | 0.341804 / -8.41 / 0 | 0.079364 / -21.10 / 0 |
| 12,000 | pulse31 | 0.283428 / -8.43 / 7.65e-4 | 0.110687 / -36.69 / 1.11e-2 |

Support-3 materially improves curve and wanted-bin errors, but violates the non-regression condition for pulse unwanted energy at 8 and 12 kHz and introduces a small unwanted component for 12 kHz saw.

## Pitch and kernel transitions

Switching kernels at identical phase is not numerically continuous: maximum current-versus-support-3 output deltas over 65,536 phases are 0.346 for saw, 0.286-0.554 for square, and 0.606-0.688 for pulse across 8-12 kHz. In an abrupt 7 kHz-to-target step, the candidate crossover increases the worst adjacent-sample jump over an all-current transition by 0.320-0.325 (saw), 0.235-0.394 (square), and 0.446-0.507 (pulse).

A stable-block guard merely delays this discontinuity until eligibility becomes true. Avoiding it requires voice-lifetime selection or a crossfade/state machine, neither of which can rescue the already failing 8-10 kHz CPU cells. Phase reset has the same kernel-delta problem at edge-adjacent reset phases. No transition mechanism was promoted.

## Decision

Fail fast at the actual saw renderer CPU gate, reinforced by pulse unwanted-energy regression and large kernel-switch discontinuities. Square/pulse production integration and selector state were reverted/not introduced because their shared residual cannot satisfy the full gate. The retained changes are test-only probe accuracy improvements and this record.

Limitations: exact coherent frequencies are determined by integer periods (6,857, 8,000, 9,600, and 12,000 Hz), while CPU cells use the requested exact steps. The offline projection does not model host oversampling because the candidate is explicitly a 1x kernel.
