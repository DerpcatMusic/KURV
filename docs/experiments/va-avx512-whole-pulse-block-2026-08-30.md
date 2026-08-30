# Whole-block AVX-512 pulse kernel (2026-08-30)

## Verdict

Reject the whole-block AVX-512 pulse backend and do not expand it to saw,
square, or triangle.  Moving the target-feature boundary from each residual to
one runtime-guarded call per 64 frames exposes a real mid/high-density win, but
it nearly doubles low-density cost.  A pitch crossover would therefore add a
narrow architecture-specific policy for a maximum measured saving of 16.5%,
while leaving the common low range substantially worse if selected wrongly.

Production DSP, oscillator state, publication, dispatch, version, and object
size remain unchanged.  The retained code is an ignored, test-only probe.

## Probe

The AVX-512F/VL function owns the complete 64-frame x8 pulse loop: phase walk,
raw pulse, both discontinuities, masked optimized cubic BLEPs, stereo gain
accumulation, and phase writeback.  The runtime feature check occurs once per
block in the timed region.  The comparison calls the shipping
`accumulate_shape8_block_constant` path with
`Antialiasing::SplineOptimized`, the same steps, phases, gains, width 0.37, and
zeroed 64-frame outputs.

Command:

```sh
CARGO_TARGET_DIR=/tmp/kurv-va-events-target \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo test avx512_whole_pulse_block_report --lib --release \
  --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

Host: AMD Ryzen 7 7800X3D (Zen 4), Rust 1.98 stable.  Each timing is the best of
seven 50,000-block release runs, or 22.4 million frames per cell.

## Results

| pitch | lanes | shipping AVX2 ns/frame | whole-block AVX-512 ns/frame | delta |
|---|---|---:|---:|---:|
| low, step .0046 | decorrelated | 3.962 | 7.566 | +90.97% |
| low, step .0046 | coherent | 3.959 | 7.623 | +92.55% |
| mid, step .041 | decorrelated | 7.709 | 7.467 | -3.14% |
| mid, step .041 | coherent | 7.819 | 7.677 | -1.82% |
| high, step .083 | decorrelated | 8.822 | 7.600 | -13.85% |
| high, step .083 | coherent | 8.830 | 7.373 | -16.50% |

The low result is not dispatch noise: the feature check is amortized over the
same 64 frames in every cell.  The masked polynomial executes regardless of
event density, whereas the shipping branchy path cheaply avoids most residual
work at low step.

The block starts from reset phases including .997 (a wrap in the first frame)
and seven decorrelated positions, and repeats independently at every tested
step.  Final phase was bit-identical in every cell (`phase_peak = 0`).  Output
rounding differences were bounded by RMS `9.4e-8`, peak `1.885e-6`; coherent
low was bit-identical.  Thus block-boundary reset and pitch selection do not
change event timing, and a hard pitch selector would introduce at most the
measured rounding delta, not a phase discontinuity.  It was not retained:
adding an AVX-512/pitch policy for only the mid/high region is not a universal
Pareto improvement.

Because candidate and shipping output differ only at that rounding bound, the
ideal-reference metrics are unchanged at reported precision.  The shipping
optimized pulse37 reference errors are:

| pitch | ideal RMS | ideal peak |
|---|---:|---:|
| low | .042565167 | .337954858 |
| mid | .132979584 | .378694190 |
| high | .185280128 | .388801281 |

Those values come from the identical 65,536-phase ideal Fourier comparison in
the immediately preceding branchless-residual experiment.  Wanted harmonics,
alias/error energy, DC, and clicks therefore cannot materially separate these
two implementations; this round is a CPU/ISA experiment, not a new AA curve.

## Code and state economics

The probe adds no tables and no retained state: it uses the existing eight
phases and fixed stack/register temporaries.  Production code and binary size
change is exactly zero because the kernel is test-only.  The release test
binary is stripped by the repository profile, so a reliable per-symbol byte
count is unavailable; the source adds one 64-frame kernel plus the inlined
masked cubic evaluator.  That is already more ISA-specific surface than the
zero new production bytes/state justified by this non-universal result.

## Closeout

Do not pursue the remaining shapes through this whole-block architecture.
The pulse is the favorable high-density case and still needs a pitch- and
ISA-specific selector to avoid a ~92% low-range regression.  That selector is
not justified by a maximum 16.5% narrow-region saving, especially when the
shipping branchy kernel already preserves exact sparse-event economics.
