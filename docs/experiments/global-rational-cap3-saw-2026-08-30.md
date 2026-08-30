# Global rational cap-3 saw (rejected)

Date: 2026-08-30

Baseline: `2cd5b85`, KURV `0.8.10`

## Verdict

Reject the production selector. A seven-coefficient global rational evaluator
reproduces the exact cap-3 saw extremely closely and showed promising coarse
CPU, but switching immediately between it and the current renderer increases
the adjacent-sample peak at both cap-region boundaries and during rapid pitch
changes. Production DSP, dependencies, oscillator state, and version remain
unchanged; the bulky test probe was removed after measurement.

## Audit and candidate

The local history and experiment records already contain polynomial Fourier,
piecewise quintic, Clenshaw/Chebyshev, coefficient-bank, projection-table, and
additive/recurrence backends. Searches across all local refs found no rational
oscillator evaluator; the only prior occurrence of "rational" described a
transition policy in the rejected phase-aligned recurrence handoff.

The single saw-only candidate targeted the fixed legal-cap-3 region from 6 to
8 kHz at 48 kHz. With `x = 2 * phase - 1`, its offline rational Remez form was

```text
x * (1 - x^2) * P3(x^2) / Q3(x^2)
```

using seven embedded `f32` coefficients, six Horner FMAs, and one reciprocal.
The endpoint factor makes phase 0, 0.5, and 1 exact. Dense fitting measured the
denominator from 1.0 to 6.084155, so there is no pole in the evaluation range.
The scalar and x8 evaluators retained authoritative phase, zero persistent
state, zero table bytes, zero latency, and no allocation, lock, I/O, or logging.

## Quality and one-cycle shape

The ignored native-release probe passed finite-output, denominator, scalar/x8
parity, exact block-phase, and reset-replay gates. Values below compare the real
shipping 1x and 2x paths with the candidate after the common factor-1 output
path.

| Frequency | Shipping 1x RMS / peak | Shipping 2x RMS / peak | Rational RMS / peak | Wanted / off-grid dB |
| ---: | ---: | ---: | ---: | ---: |
| 6,000 Hz | .142814839 / .273593262 | .001590916 / .002560976 | .000060644 / .000093330 | -78.750 / -3067.922 |
| 6,857.143 Hz | .173627877 / .290918389 | .008511386 / .014951593 | .000047699 / .000062006 | -80.836 / -309.752 |

Candidate DC was zero within `2.8e-8`, and fitted gain was
`0.999928708..1.000081540`. The accepted one-cycle metric was applied to the
raw authoritative evaluator rather than the post-path stream, whose existing
whole-sample latency is outside that metric's intentional `+/-0.5` sample fit.

| Frequency | Raw RMS / peak | Phase samples | Residual RMS / peak |
| ---: | ---: | ---: | ---: |
| 6,000 Hz | .000060644 / .000093330 | 0 | .000047708 / .000073237 |
| 6,857.143 Hz | .000047699 / .000062006 | 0 | .000021007 / .000036158 |

This confirms that the quality result is waveform shape, not a favorable phase,
DC, or gain fit.

## Immediate-transition rejection

The candidate used rational evaluation only while the exact legal cap was three
and otherwise used the current renderer, advancing the same oscillator phase in
both cases. Dense 4,096-phase boundary sweeps therefore include the real
representation handoff without stale state or a crossfade.

| Boundary | Current adjacent peak | Rational-selector peak | Regression |
| --- | ---: | ---: | ---: |
| 5,999 to 6,001 Hz, cap 4 to 3 | .889929652 | 1.127296567 | +26.7% |
| 7,999 to 8,001 Hz, cap 3 to 2 | .806635499 | 1.219233990 | +51.1% |

On the repeating `440x24 -> 7040x32 -> 110x24 -> 12000x32` schedule, the
largest pitch-event step rose from `1.587572217` to `1.892448425` (+19.2%).
Candidate RMS also rose from `.475313943` to `.508191271`; reset replay error
was exactly zero. The rational waveform is closer to the exact projection, but
that does not make its immediate boundary with the current waveform artifact
safe.

## Coarse CPU

The Windows test VM remained active, so CPU was used only as a coarse screen.
Across three native-release runs at 7.04 kHz, 24/32-frame scalar candidate/current
ratios were `.847..873`; x8 ratios were `.773..947`. Every observed cell was an
apparent win, but the x8 spread is too wide for a production performance claim.
A pristine repeat was intentionally skipped because the independent transition
gate had already rejected integration.

The final focused run passed one ignored test with 404 filtered tests. The cold
native build and two incremental LTO builds completed with the checkout's 26
existing warnings.
