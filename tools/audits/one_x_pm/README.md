# High-note 1x PM and nested-PM counterexample

This harness executes the actual `one_x_high::{saw,triangle}` scalar kernels and the actual shipping `SplineOptimized` baseline. Only the truce-simd import and module doc-comment syntax are adapted for isolated compilation. It does not reproduce those arithmetic functions in Python.

## Reproduce

Requires Rust1.97.1, cached wide0.7.33, Python and NumPy. Run after the high-note integration module is present:

```sh
RUSTUP_TOOLCHAIN=1.97.1 cargo run --release --offline --manifest-path tools/audits/one_x_pm/Cargo.toml --features experimental-1x-dsp > /tmp/kurv-one-x-pm-samples.csv
python tools/audits/one_x_pm/analyze.py /tmp/kurv-one-x-pm-samples.csv
```

`KURV_SOURCE_ROOT` optionally selects another repository checkout. The final proof was rerun from the combined review branch; source SHA256 fingerprints are recorded in `source-sha256.txt`. Samples are reproducible intermediates and are not committed.

4096-sample coherent period,48kHz host, carrier9996.09375Hz or12949.21875Hz. Phase in cycles is

```
p(t) = carrier*t + 0.123
       + depth*sin(2*pi*(2261.71875*t
               + nested_depth*sin(2*pi*4300.78125*t)))
```

The carrier step passed to both production kernels stays constant, as in the audited phase-modulated path. No effective-velocity correction is inserted. Depths are0,.02,.15,.5cycles, with selected nested cases at.5cycles.

## Reference and metric

Python evaluates that continuous phase equation at256x and1024x host rate, then evaluates the raw saw or triangle. Ideal Fourier lowpass removes everything at/above host Nyquist and decimates. Comparing256x/1024x estimates reports reference convergence. This measures the desired result of phase-modulating the underlying waveform and then antialiasing, not phase-modulating an already fundamental-only approximation. Those are different spectral contracts.

Static cases use the exact finite Fourier series instead of oversampling. Very small PM errors below reference-convergence resolution are flagged in CSV; they do not establish the printed candidate error as an accurate absolute value. Convergence agreement is an empirical check, not a rigorous mathematical error bound.

The metric is total reconstruction error relative to reference power, with no fitted gain, phase, or DC removal. It combines wanted-sideband attenuation/distortion and aliasing. It is explicitly not labeled alias power: dense nested-PM spectra do not permit a trustworthy clean wanted/unwanted bin split here.

## Result: do not treat the high-note mode as a PM fix

At12949.21875Hz, depth.5cycles, nested depth.5cycles:

| Shape | Shipping error | High-note error | Change |
| --- | ---: | ---: | ---: |
| Saw | -3.491dB | -2.134dB | **1.357dB worse** |
| Triangle | -4.444dB | -2.729dB | **1.714dB worse** |

The deeper256x→1024x oracle retains that counterexample; convergence error is far below these signal errors. Shallow PM generally improves, and static high notes improve drastically, but neither establishes unrestricted modulation quality. At9996Hz, the deepest nested triangle case also regresses slightly (~.04dB).

A high-note mode that restores fundamental amplitude also amplifies modulation sidebands; several sidebands can cross Nyquist. Removing source harmonics also removes legitimate sidebands that those harmonics would have contributed below Nyquist. Finite stationary harmonic selection alone solves neither issue.

Recommended scope: static/unmodulated high-note feature only until modulation dispatch is explicitly tested. If the feature deliberately supports PM as an opt-in timbral choice, expose the measured limitation. The tests here bypass the renderer selector to identify what happens if that kernel is used; a production fallback can prevent these cases from reaching it.

No claim about scalar/x4/x8 timing, full voice routing, or complete plugin behavior is made by this harness.
